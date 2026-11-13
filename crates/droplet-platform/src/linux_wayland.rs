//! Linux Wayland graphical windowing backend via runtime `dlopen`.
//!
//! This backend is intentionally dependency-free: `libwayland-client.so.0` is
//! resolved at runtime and all protocol interface descriptors, opcodes and
//! wire-level constants are declared by hand in this file. Shared-memory
//! surfaces use `memfd_create` + `mmap` and a double-buffered `wl_shm` pool.
//!
//! The backend only starts when `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` are
//! present; `droplet-platform` falls back to X11 when loading, connecting or
//! initializing the Wayland client fails.
//!
//! Live compositor testing is manual (`cargo run -- about:droplet` under a
//! Wayland session); unit tests deterministically verify wire encoding/decoding,
//! protocol descriptor tables, registry state, fallback gating, input mapping,
//! and double-buffered `wl_shm` pool state.

use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::ptr;
use std::slice;
use std::sync::OnceLock;

use droplet_gfx::{Canvas, Color, DamageRect};

use crate::{format_window_title, NavKey, PlatformApp};

const RTLD_LAZY: c_int = 1;

const MFD_CLOEXEC: c_uint = 1;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;

const WL_MARSHAL_FLAG_DESTROY: u32 = 1;

const WL_SHM_FORMAT_ARGB8888: u32 = 0;
const WL_SHM_FORMAT_XRGB8888: u32 = 1;

const WL_SEAT_CAPABILITY_POINTER: u32 = 1;
const WL_SEAT_CAPABILITY_KEYBOARD: u32 = 2;

const WL_KEYBOARD_KEY_STATE_PRESSED: u32 = 1;
const WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1: u32 = 1;

const WL_POINTER_BUTTON_STATE_PRESSED: u32 = 1;
const WL_POINTER_AXIS_VERTICAL_SCROLL: u32 = 0;

const BTN_LEFT: u32 = 0x110;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

// Linux evdev keycodes used by `wl_keyboard.key`.
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_TAB: u32 = 15;
const KEY_ENTER: u32 = 28;
const KEY_SPACE: u32 = 57;
const KEY_F5: u32 = 63;
const KEY_KPENTER: u32 = 96;
const KEY_HOME: u32 = 102;
const KEY_UP: u32 = 103;
const KEY_PAGEUP: u32 = 104;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_END: u32 = 107;
const KEY_DOWN: u32 = 108;
const KEY_PAGEDOWN: u32 = 109;

// Letter keycodes (evdev).
const KEY_L: u32 = 38;
const KEY_R: u32 = 19;
const KEY_T: u32 = 20;
const KEY_W: u32 = 17;

const MAX_PROTOCOL_VERSION: u32 = 1;
const MAX_COMPOSITOR_VERSION: u32 = 4;
const MAX_SHM_BYTES: usize = 512 * 1024 * 1024;

// wl_display
const WL_DISPLAY_GET_REGISTRY: u32 = 1;

// wl_registry
const WL_REGISTRY_BIND: u32 = 0;

// wl_compositor
const WL_COMPOSITOR_CREATE_SURFACE: u32 = 0;

// wl_shm
const WL_SHM_CREATE_POOL: u32 = 0;

// wl_shm_pool
const WL_SHM_POOL_CREATE_BUFFER: u32 = 0;
const WL_SHM_POOL_DESTROY: u32 = 1;

// wl_buffer
const WL_BUFFER_DESTROY: u32 = 0;

// wl_surface
const WL_SURFACE_DESTROY: u32 = 0;
const WL_SURFACE_ATTACH: u32 = 1;
const WL_SURFACE_DAMAGE: u32 = 2;
const WL_SURFACE_COMMIT: u32 = 6;
const WL_SURFACE_DAMAGE_BUFFER: u32 = 9;

// wl_seat
const WL_SEAT_GET_POINTER: u32 = 0;
const WL_SEAT_GET_KEYBOARD: u32 = 1;

// xdg_wm_base
const XDG_WM_BASE_DESTROY: u32 = 0;
const XDG_WM_BASE_GET_XDG_SURFACE: u32 = 2;
const XDG_WM_BASE_PONG: u32 = 3;

// xdg_surface
const XDG_SURFACE_DESTROY: u32 = 0;
const XDG_SURFACE_GET_TOPLEVEL: u32 = 1;
const XDG_SURFACE_SET_WINDOW_GEOMETRY: u32 = 3;
const XDG_SURFACE_ACK_CONFIGURE: u32 = 4;

// xdg_toplevel
const XDG_TOPLEVEL_DESTROY: u32 = 0;
const XDG_TOPLEVEL_SET_TITLE: u32 = 2;
const XDG_TOPLEVEL_SET_APP_ID: u32 = 3;

#[repr(C)]
struct WlDisplay {
    _private: [u8; 0],
}

#[repr(C)]
struct WlProxy {
    _private: [u8; 0],
}

#[repr(C)]
struct WlArray {
    _private: [u8; 0],
}

#[repr(C)]
struct WlMessage {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const WlInterface,
}

#[repr(C)]
struct WlInterface {
    name: *const c_char,
    version: c_int,
    method_count: c_int,
    methods: *const WlMessage,
    event_count: c_int,
    events: *const WlMessage,
}

// The protocol descriptors below are immutable process-wide tables. Raw
// pointers are only ever compared/dereferenced by libwayland.
unsafe impl Sync for WlMessage {}
unsafe impl Sync for WlInterface {}

struct TypeList<const N: usize>([*const WlInterface; N]);

unsafe impl<const N: usize> Sync for TypeList<N> {}

static NO_TYPES: TypeList<8> = TypeList([ptr::null(); 8]);

static WL_REGISTRY_REQUESTS: [WlMessage; 1] = [WlMessage {
    name: c"bind".as_ptr(),
    signature: c"usun".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static WL_REGISTRY_GLOBAL_TYPES: TypeList<3> = TypeList([ptr::null(); 3]);
static WL_REGISTRY_EVENTS: [WlMessage; 2] = [
    WlMessage {
        name: c"global".as_ptr(),
        signature: c"usu".as_ptr(),
        types: WL_REGISTRY_GLOBAL_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"global_remove".as_ptr(),
        signature: c"u".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static WL_REGISTRY_INTERFACE: WlInterface = WlInterface {
    name: c"wl_registry".as_ptr(),
    version: 1,
    method_count: 1,
    methods: WL_REGISTRY_REQUESTS.as_ptr(),
    event_count: 2,
    events: WL_REGISTRY_EVENTS.as_ptr(),
};

static WL_CALLBACK_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"done".as_ptr(),
    signature: c"u".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static WL_CALLBACK_INTERFACE: WlInterface = WlInterface {
    name: c"wl_callback".as_ptr(),
    version: 1,
    method_count: 0,
    methods: ptr::null(),
    event_count: 1,
    events: WL_CALLBACK_EVENTS.as_ptr(),
};

static WL_COMPOSITOR_CREATE_SURFACE_TYPES: TypeList<1> =
    TypeList([&WL_SURFACE_INTERFACE as *const WlInterface]);
static WL_COMPOSITOR_CREATE_REGION_TYPES: TypeList<1> =
    TypeList([&WL_REGION_INTERFACE as *const WlInterface]);
static WL_COMPOSITOR_REQUESTS: [WlMessage; 2] = [
    WlMessage {
        name: c"create_surface".as_ptr(),
        signature: c"n".as_ptr(),
        types: WL_COMPOSITOR_CREATE_SURFACE_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"create_region".as_ptr(),
        signature: c"n".as_ptr(),
        types: WL_COMPOSITOR_CREATE_REGION_TYPES.0.as_ptr(),
    },
];
static WL_COMPOSITOR_INTERFACE: WlInterface = WlInterface {
    name: c"wl_compositor".as_ptr(),
    version: 4,
    method_count: 2,
    methods: WL_COMPOSITOR_REQUESTS.as_ptr(),
    event_count: 0,
    events: ptr::null(),
};

static WL_REGION_REQUESTS: [WlMessage; 3] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"add".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"subtract".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static WL_REGION_INTERFACE: WlInterface = WlInterface {
    name: c"wl_region".as_ptr(),
    version: 1,
    method_count: 3,
    methods: WL_REGION_REQUESTS.as_ptr(),
    event_count: 0,
    events: ptr::null(),
};

static WL_SHM_CREATE_POOL_TYPES: TypeList<3> = TypeList([
    &WL_SHM_POOL_INTERFACE as *const WlInterface,
    ptr::null(),
    ptr::null(),
]);
static WL_SHM_REQUESTS: [WlMessage; 1] = [WlMessage {
    name: c"create_pool".as_ptr(),
    signature: c"nhi".as_ptr(),
    types: WL_SHM_CREATE_POOL_TYPES.0.as_ptr(),
}];
static WL_SHM_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"format".as_ptr(),
    signature: c"u".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static WL_SHM_INTERFACE: WlInterface = WlInterface {
    name: c"wl_shm".as_ptr(),
    version: 1,
    method_count: 1,
    methods: WL_SHM_REQUESTS.as_ptr(),
    event_count: 1,
    events: WL_SHM_EVENTS.as_ptr(),
};

static WL_SHM_POOL_CREATE_BUFFER_TYPES: TypeList<6> = TypeList([
    &WL_BUFFER_INTERFACE as *const WlInterface,
    ptr::null(),
    ptr::null(),
    ptr::null(),
    ptr::null(),
    ptr::null(),
]);
static WL_SHM_POOL_REQUESTS: [WlMessage; 3] = [
    WlMessage {
        name: c"create_buffer".as_ptr(),
        signature: c"niiiiu".as_ptr(),
        types: WL_SHM_POOL_CREATE_BUFFER_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"resize".as_ptr(),
        signature: c"i".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static WL_SHM_POOL_INTERFACE: WlInterface = WlInterface {
    name: c"wl_shm_pool".as_ptr(),
    version: 1,
    method_count: 3,
    methods: WL_SHM_POOL_REQUESTS.as_ptr(),
    event_count: 0,
    events: ptr::null(),
};

static WL_BUFFER_REQUESTS: [WlMessage; 1] = [WlMessage {
    name: c"destroy".as_ptr(),
    signature: c"".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static WL_BUFFER_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"release".as_ptr(),
    signature: c"".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static WL_BUFFER_INTERFACE: WlInterface = WlInterface {
    name: c"wl_buffer".as_ptr(),
    version: 1,
    method_count: 1,
    methods: WL_BUFFER_REQUESTS.as_ptr(),
    event_count: 1,
    events: WL_BUFFER_EVENTS.as_ptr(),
};

static WL_SURFACE_ATTACH_TYPES: TypeList<3> = TypeList([
    &WL_BUFFER_INTERFACE as *const WlInterface,
    ptr::null(),
    ptr::null(),
]);
static WL_SURFACE_FRAME_TYPES: TypeList<1> =
    TypeList([&WL_CALLBACK_INTERFACE as *const WlInterface]);
static WL_SURFACE_REGION_TYPES: TypeList<1> =
    TypeList([&WL_REGION_INTERFACE as *const WlInterface]);
static WL_SURFACE_OUTPUT_TYPES: TypeList<1> =
    TypeList([&WL_OUTPUT_INTERFACE as *const WlInterface]);
static WL_SURFACE_REQUESTS: [WlMessage; 10] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"attach".as_ptr(),
        signature: c"?oii".as_ptr(),
        types: WL_SURFACE_ATTACH_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"damage".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"frame".as_ptr(),
        signature: c"n".as_ptr(),
        types: WL_SURFACE_FRAME_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_opaque_region".as_ptr(),
        signature: c"?o".as_ptr(),
        types: WL_SURFACE_REGION_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_input_region".as_ptr(),
        signature: c"?o".as_ptr(),
        types: WL_SURFACE_REGION_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"commit".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_buffer_transform".as_ptr(),
        signature: c"i".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_buffer_scale".as_ptr(),
        signature: c"i".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"damage_buffer".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static WL_SURFACE_EVENTS: [WlMessage; 2] = [
    WlMessage {
        name: c"enter".as_ptr(),
        signature: c"o".as_ptr(),
        types: WL_SURFACE_OUTPUT_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"leave".as_ptr(),
        signature: c"o".as_ptr(),
        types: WL_SURFACE_OUTPUT_TYPES.0.as_ptr(),
    },
];
static WL_SURFACE_INTERFACE: WlInterface = WlInterface {
    name: c"wl_surface".as_ptr(),
    version: 4,
    method_count: 10,
    methods: WL_SURFACE_REQUESTS.as_ptr(),
    event_count: 2,
    events: WL_SURFACE_EVENTS.as_ptr(),
};

static WL_SEAT_GET_POINTER_TYPES: TypeList<1> =
    TypeList([&WL_POINTER_INTERFACE as *const WlInterface]);
static WL_SEAT_GET_KEYBOARD_TYPES: TypeList<1> =
    TypeList([&WL_KEYBOARD_INTERFACE as *const WlInterface]);
static WL_SEAT_GET_TOUCH_TYPES: TypeList<1> =
    TypeList([&WL_TOUCH_INTERFACE as *const WlInterface]);
static WL_SEAT_REQUESTS: [WlMessage; 3] = [
    WlMessage {
        name: c"get_pointer".as_ptr(),
        signature: c"n".as_ptr(),
        types: WL_SEAT_GET_POINTER_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"get_keyboard".as_ptr(),
        signature: c"n".as_ptr(),
        types: WL_SEAT_GET_KEYBOARD_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"get_touch".as_ptr(),
        signature: c"n".as_ptr(),
        types: WL_SEAT_GET_TOUCH_TYPES.0.as_ptr(),
    },
];
static WL_SEAT_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"capabilities".as_ptr(),
    signature: c"u".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static WL_SEAT_INTERFACE: WlInterface = WlInterface {
    name: c"wl_seat".as_ptr(),
    version: 1,
    method_count: 3,
    methods: WL_SEAT_REQUESTS.as_ptr(),
    event_count: 1,
    events: WL_SEAT_EVENTS.as_ptr(),
};

static WL_KEYBOARD_ENTER_TYPES: TypeList<3> = TypeList([
    ptr::null(),
    &WL_SURFACE_INTERFACE as *const WlInterface,
    ptr::null(),
]);
static WL_KEYBOARD_LEAVE_TYPES: TypeList<2> = TypeList([
    ptr::null(),
    &WL_SURFACE_INTERFACE as *const WlInterface,
]);
static WL_KEYBOARD_EVENTS: [WlMessage; 5] = [
    WlMessage {
        name: c"keymap".as_ptr(),
        signature: c"uhu".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"enter".as_ptr(),
        signature: c"uoa".as_ptr(),
        types: WL_KEYBOARD_ENTER_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"leave".as_ptr(),
        signature: c"uo".as_ptr(),
        types: WL_KEYBOARD_LEAVE_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"key".as_ptr(),
        signature: c"uuuu".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"modifiers".as_ptr(),
        signature: c"uuuuu".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static WL_KEYBOARD_INTERFACE: WlInterface = WlInterface {
    name: c"wl_keyboard".as_ptr(),
    version: 1,
    method_count: 0,
    methods: ptr::null(),
    event_count: 5,
    events: WL_KEYBOARD_EVENTS.as_ptr(),
};

static WL_POINTER_SET_CURSOR_TYPES: TypeList<4> = TypeList([
    ptr::null(),
    &WL_SURFACE_INTERFACE as *const WlInterface,
    ptr::null(),
    ptr::null(),
]);
static WL_POINTER_ENTER_TYPES: TypeList<4> = TypeList([
    ptr::null(),
    &WL_SURFACE_INTERFACE as *const WlInterface,
    ptr::null(),
    ptr::null(),
]);
static WL_POINTER_LEAVE_TYPES: TypeList<2> = TypeList([
    ptr::null(),
    &WL_SURFACE_INTERFACE as *const WlInterface,
]);
static WL_POINTER_REQUESTS: [WlMessage; 1] = [WlMessage {
    name: c"set_cursor".as_ptr(),
    signature: c"u?oii".as_ptr(),
    types: WL_POINTER_SET_CURSOR_TYPES.0.as_ptr(),
}];
static WL_POINTER_EVENTS: [WlMessage; 5] = [
    WlMessage {
        name: c"enter".as_ptr(),
        signature: c"uoff".as_ptr(),
        types: WL_POINTER_ENTER_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"leave".as_ptr(),
        signature: c"uo".as_ptr(),
        types: WL_POINTER_LEAVE_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"motion".as_ptr(),
        signature: c"uff".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"button".as_ptr(),
        signature: c"uuuu".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"axis".as_ptr(),
        signature: c"uuf".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static WL_POINTER_INTERFACE: WlInterface = WlInterface {
    name: c"wl_pointer".as_ptr(),
    version: 1,
    method_count: 1,
    methods: WL_POINTER_REQUESTS.as_ptr(),
    event_count: 5,
    events: WL_POINTER_EVENTS.as_ptr(),
};

static WL_TOUCH_INTERFACE: WlInterface = WlInterface {
    name: c"wl_touch".as_ptr(),
    version: 1,
    method_count: 0,
    methods: ptr::null(),
    event_count: 0,
    events: ptr::null(),
};

static WL_OUTPUT_INTERFACE: WlInterface = WlInterface {
    name: c"wl_output".as_ptr(),
    version: 1,
    method_count: 0,
    methods: ptr::null(),
    event_count: 0,
    events: ptr::null(),
};

static XDG_WM_BASE_CREATE_POSITIONER_TYPES: TypeList<1> =
    TypeList([&XDG_POSITIONER_INTERFACE as *const WlInterface]);
static XDG_WM_BASE_GET_XDG_SURFACE_TYPES: TypeList<2> = TypeList([
    &XDG_SURFACE_INTERFACE as *const WlInterface,
    &WL_SURFACE_INTERFACE as *const WlInterface,
]);
static XDG_WM_BASE_REQUESTS: [WlMessage; 4] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"create_positioner".as_ptr(),
        signature: c"n".as_ptr(),
        types: XDG_WM_BASE_CREATE_POSITIONER_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"get_xdg_surface".as_ptr(),
        signature: c"no".as_ptr(),
        types: XDG_WM_BASE_GET_XDG_SURFACE_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"pong".as_ptr(),
        signature: c"u".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static XDG_WM_BASE_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"ping".as_ptr(),
    signature: c"u".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static XDG_WM_BASE_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_wm_base".as_ptr(),
    version: 1,
    method_count: 4,
    methods: XDG_WM_BASE_REQUESTS.as_ptr(),
    event_count: 1,
    events: XDG_WM_BASE_EVENTS.as_ptr(),
};

static XDG_SURFACE_GET_TOPLEVEL_TYPES: TypeList<1> =
    TypeList([&XDG_TOPLEVEL_INTERFACE as *const WlInterface]);
static XDG_SURFACE_GET_POPUP_TYPES: TypeList<3> = TypeList([
    &XDG_POPUP_INTERFACE as *const WlInterface,
    &XDG_SURFACE_INTERFACE as *const WlInterface,
    &XDG_POSITIONER_INTERFACE as *const WlInterface,
]);
static XDG_SURFACE_REQUESTS: [WlMessage; 5] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"get_toplevel".as_ptr(),
        signature: c"n".as_ptr(),
        types: XDG_SURFACE_GET_TOPLEVEL_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"get_popup".as_ptr(),
        signature: c"n?oo".as_ptr(),
        types: XDG_SURFACE_GET_POPUP_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_window_geometry".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"ack_configure".as_ptr(),
        signature: c"u".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static XDG_SURFACE_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"configure".as_ptr(),
    signature: c"u".as_ptr(),
    types: NO_TYPES.0.as_ptr(),
}];
static XDG_SURFACE_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_surface".as_ptr(),
    version: 1,
    method_count: 5,
    methods: XDG_SURFACE_REQUESTS.as_ptr(),
    event_count: 1,
    events: XDG_SURFACE_EVENTS.as_ptr(),
};

static XDG_TOPLEVEL_SET_PARENT_TYPES: TypeList<1> =
    TypeList([&XDG_TOPLEVEL_INTERFACE as *const WlInterface]);
static XDG_TOPLEVEL_REQUESTS: [WlMessage; 4] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_parent".as_ptr(),
        signature: c"?o".as_ptr(),
        types: XDG_TOPLEVEL_SET_PARENT_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_title".as_ptr(),
        signature: c"s".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"set_app_id".as_ptr(),
        signature: c"s".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static XDG_TOPLEVEL_EVENTS: [WlMessage; 2] = [
    WlMessage {
        name: c"configure".as_ptr(),
        signature: c"iia".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
    WlMessage {
        name: c"close".as_ptr(),
        signature: c"".as_ptr(),
        types: NO_TYPES.0.as_ptr(),
    },
];
static XDG_TOPLEVEL_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_toplevel".as_ptr(),
    version: 1,
    method_count: 4,
    methods: XDG_TOPLEVEL_REQUESTS.as_ptr(),
    event_count: 2,
    events: XDG_TOPLEVEL_EVENTS.as_ptr(),
};

static XDG_POSITIONER_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_positioner".as_ptr(),
    version: 1,
    method_count: 0,
    methods: ptr::null(),
    event_count: 0,
    events: ptr::null(),
};

static XDG_POPUP_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_popup".as_ptr(),
    version: 1,
    method_count: 0,
    methods: ptr::null(),
    event_count: 0,
    events: ptr::null(),
};

#[link(name = "c")]
extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;

    fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    fn ftruncate(fd: c_int, length: i64) -> c_int;
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
    fn close(fd: c_int) -> c_int;
}

type MarshalFlagsFn = unsafe extern "C" fn(
    proxy: *mut WlProxy,
    opcode: u32,
    interface: *const WlInterface,
    version: u32,
    flags: u32,
    ...
) -> *mut WlProxy;

#[derive(Clone, Copy)]
struct WaylandFns {
    marshal_flags: MarshalFlagsFn,
}

#[allow(clippy::type_complexity)]
struct WaylandApi {
    handle: *mut c_void,
    wl_display_connect: unsafe extern "C" fn(*const c_char) -> *mut WlDisplay,
    wl_display_disconnect: unsafe extern "C" fn(*mut WlDisplay),
    wl_display_dispatch: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    wl_display_roundtrip: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    wl_display_flush: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    wl_proxy_marshal_flags: MarshalFlagsFn,
    wl_proxy_add_listener:
        unsafe extern "C" fn(*mut WlProxy, *const *const c_void, *mut c_void) -> c_int,
    wl_proxy_destroy: unsafe extern "C" fn(*mut WlProxy),
}

impl Drop for WaylandApi {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                dlclose(self.handle);
            }
        }
    }
}

unsafe fn load_sym<T: Copy>(handle: *mut c_void, name: &CStr) -> Result<T, String> {
    let ptr = dlsym(handle, name.as_ptr());
    if ptr.is_null() {
        return Err(format!(
            "Failed to resolve Wayland symbol '{}'",
            name.to_string_lossy()
        ));
    }
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
    Ok(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
}

impl WaylandApi {
    fn load() -> Result<Self, String> {
        let handle = unsafe {
            let h = dlopen(c"libwayland-client.so.0".as_ptr(), RTLD_LAZY);
            if !h.is_null() {
                h
            } else {
                dlopen(c"libwayland-client.so".as_ptr(), RTLD_LAZY)
            }
        };
        if handle.is_null() {
            return Err("Could not load libwayland-client.so.0".into());
        }

        let res = unsafe {
            Ok(Self {
                handle,
                wl_display_connect: load_sym(handle, c"wl_display_connect")?,
                wl_display_disconnect: load_sym(handle, c"wl_display_disconnect")?,
                wl_display_dispatch: load_sym(handle, c"wl_display_dispatch")?,
                wl_display_roundtrip: load_sym(handle, c"wl_display_roundtrip")?,
                wl_display_flush: load_sym(handle, c"wl_display_flush")?,
                wl_proxy_marshal_flags: load_sym(handle, c"wl_proxy_marshal_flags")?,
                wl_proxy_add_listener: load_sym(handle, c"wl_proxy_add_listener")?,
                wl_proxy_destroy: load_sym(handle, c"wl_proxy_destroy")?,
            })
        };
        if res.is_err() {
            unsafe {
                dlclose(handle);
            }
        }
        res
    }

    fn fns(&self) -> WaylandFns {
        WaylandFns {
            marshal_flags: self.wl_proxy_marshal_flags,
        }
    }
}

/// Returns `true` when `libwayland-client.so.0` exposes every symbol this
/// backend needs. Used by `platform::run` to decide whether Wayland is worth
/// attempting before falling back to X11.
pub(crate) fn library_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| WaylandApi::load().is_ok())
}

/// Backend initialization failure. `Init` means no window was shown yet and the
/// caller may safely retry with X11; `Runtime` means the event loop had already
/// started.
pub enum RunError {
    Init(String),
    Runtime(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalKind {
    Compositor,
    Shm,
    Seat,
    XdgWmBase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GlobalBinding {
    kind: GlobalKind,
    name: u32,
    version: u32,
}

/// Pure registry state machine: records the first advertised instance of each
/// required global and clamps the version we bind to the protocol subset this
/// backend implements.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RegistryState {
    compositor: bool,
    shm: bool,
    seat: bool,
    xdg_wm_base: bool,
}

impl RegistryState {
    fn accept(&mut self, interface: &str, advertised_version: u32) -> Option<GlobalBinding> {
        if advertised_version == 0 {
            return None;
        }
        let (kind, slot, max_ver) = match interface {
            "wl_compositor" => (
                GlobalKind::Compositor,
                &mut self.compositor,
                MAX_COMPOSITOR_VERSION,
            ),
            "wl_shm" => (GlobalKind::Shm, &mut self.shm, MAX_PROTOCOL_VERSION),
            "wl_seat" => (GlobalKind::Seat, &mut self.seat, MAX_PROTOCOL_VERSION),
            "xdg_wm_base" => (
                GlobalKind::XdgWmBase,
                &mut self.xdg_wm_base,
                MAX_PROTOCOL_VERSION,
            ),
            _ => return None,
        };
        if *slot {
            return None;
        }
        *slot = true;
        Some(GlobalBinding {
            kind,
            name: 0,
            version: advertised_version.min(max_ver),
        })
    }

    fn complete(&self) -> bool {
        self.compositor && self.shm && self.seat && self.xdg_wm_base
    }

    fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.compositor {
            missing.push("wl_compositor");
        }
        if !self.shm {
            missing.push("wl_shm");
        }
        if !self.seat {
            missing.push("wl_seat");
        }
        if !self.xdg_wm_base {
            missing.push("xdg_wm_base");
        }
        missing
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KeyModifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
    caps: bool,
}

impl KeyModifiers {
    fn from_wayland(depressed: u32, locked: u32) -> Self {
        Self {
            shift: depressed & (1 << 0) != 0,
            caps: locked & (1 << 1) != 0,
            ctrl: depressed & (1 << 2) != 0,
            alt: depressed & (1 << 3) != 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    Nav(NavKey),
    Text(char),
    Backspace,
    Submit,
    None,
}

fn map_key(key: u32, mods: KeyModifiers, url_focused: bool) -> KeyAction {
    if mods.ctrl && !mods.alt {
        match key {
            KEY_L => return KeyAction::Nav(NavKey::FocusUrl),
            KEY_T => return KeyAction::Nav(NavKey::NewTab),
            KEY_W => return KeyAction::Nav(NavKey::CloseTab),
            KEY_R => return KeyAction::Nav(NavKey::Reload),
            KEY_TAB => {
                return KeyAction::Nav(if mods.shift {
                    NavKey::PrevTab
                } else {
                    NavKey::NextTab
                })
            }
            _ => {}
        }
    }
    if mods.alt && !mods.ctrl {
        match key {
            KEY_LEFT => return KeyAction::Nav(NavKey::Back),
            KEY_RIGHT => return KeyAction::Nav(NavKey::Forward),
            KEY_HOME => return KeyAction::Nav(NavKey::GoHome),
            _ => {}
        }
    }

    match key {
        KEY_F5 => KeyAction::Nav(NavKey::Reload),
        KEY_ENTER | KEY_KPENTER => {
            if url_focused {
                KeyAction::Submit
            } else {
                KeyAction::None
            }
        }
        KEY_BACKSPACE => KeyAction::Backspace,
        KEY_ESC => KeyAction::Nav(NavKey::Escape),
        KEY_UP => KeyAction::Nav(NavKey::Up),
        KEY_DOWN => KeyAction::Nav(NavKey::Down),
        KEY_PAGEUP => KeyAction::Nav(NavKey::PageUp),
        KEY_PAGEDOWN => KeyAction::Nav(NavKey::PageDown),
        KEY_HOME => KeyAction::Nav(NavKey::Home),
        KEY_END => KeyAction::Nav(NavKey::End),
        KEY_SPACE if !url_focused => KeyAction::Nav(NavKey::PageDown),
        _ => {
            if mods.ctrl || mods.alt {
                return KeyAction::None;
            }
            match us_layout_char(key, mods.shift, mods.caps) {
                Some(ch) => KeyAction::Text(ch),
                None => KeyAction::None,
            }
        }
    }
}

/// Minimal US/QWERTY character mapping. Wayland compositors can advertise an
/// arbitrary XKB layout; parsing XKB is out of scope for this zero-dependency
/// backend, so non-US layouts fall back to this table.
fn us_layout_char(key: u32, shift: bool, caps: bool) -> Option<char> {
    let (lower, upper) = match key {
        16 => ('q', 'Q'),
        17 => ('w', 'W'),
        18 => ('e', 'E'),
        19 => ('r', 'R'),
        20 => ('t', 'T'),
        21 => ('y', 'Y'),
        22 => ('u', 'U'),
        23 => ('i', 'I'),
        24 => ('o', 'O'),
        25 => ('p', 'P'),
        30 => ('a', 'A'),
        31 => ('s', 'S'),
        32 => ('d', 'D'),
        33 => ('f', 'F'),
        34 => ('g', 'G'),
        35 => ('h', 'H'),
        36 => ('j', 'J'),
        37 => ('k', 'K'),
        38 => ('l', 'L'),
        44 => ('z', 'Z'),
        45 => ('x', 'X'),
        46 => ('c', 'C'),
        47 => ('v', 'V'),
        48 => ('b', 'B'),
        49 => ('n', 'N'),
        50 => ('m', 'M'),
        _ => {
            return match key {
                2 => Some(if shift { '!' } else { '1' }),
                3 => Some(if shift { '@' } else { '2' }),
                4 => Some(if shift { '#' } else { '3' }),
                5 => Some(if shift { '$' } else { '4' }),
                6 => Some(if shift { '%' } else { '5' }),
                7 => Some(if shift { '^' } else { '6' }),
                8 => Some(if shift { '&' } else { '7' }),
                9 => Some(if shift { '*' } else { '8' }),
                10 => Some(if shift { '(' } else { '9' }),
                11 => Some(if shift { ')' } else { '0' }),
                12 => Some(if shift { '_' } else { '-' }),
                13 => Some(if shift { '+' } else { '=' }),
                26 => Some(if shift { '{' } else { '[' }),
                27 => Some(if shift { '}' } else { ']' }),
                39 => Some(if shift { ':' } else { ';' }),
                40 => Some(if shift { '"' } else { '\'' }),
                41 => Some(if shift { '~' } else { '`' }),
                43 => Some(if shift { '|' } else { '\\' }),
                51 => Some(if shift { '<' } else { ',' }),
                52 => Some(if shift { '>' } else { '.' }),
                53 => Some(if shift { '?' } else { '/' }),
                57 => Some(' '),
                71 => Some('7'),
                72 => Some('8'),
                73 => Some('9'),
                74 => Some('-'),
                75 => Some('4'),
                76 => Some('5'),
                77 => Some('6'),
                78 => Some('+'),
                79 => Some('1'),
                80 => Some('2'),
                81 => Some('3'),
                82 => Some('0'),
                83 => Some('.'),
                86 => Some('<'),
                _ => None,
            };
        }
    };
    let use_upper = shift ^ caps;
    Some(if use_upper { upper } else { lower })
}

fn fixed_to_f32(value: i32) -> f32 {
    value as f32 / 256.0
}

const AXIS_SCROLL_SCALE: f32 = 3.0;
const MAX_AXIS_SCROLL: f32 = 240.0;

fn axis_scroll_delta(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    (value * AXIS_SCROLL_SCALE).clamp(-MAX_AXIS_SCROLL, MAX_AXIS_SCROLL)
}

fn apply_key_action<A: PlatformApp>(app: &mut A, action: KeyAction) {
    match action {
        KeyAction::Nav(key) => app.handle_key_navigation(key),
        KeyAction::Text(ch) => app.handle_text_input(ch),
        KeyAction::Backspace => app.handle_backspace(),
        KeyAction::Submit => app.handle_submit_url(),
        KeyAction::None => {}
    }
}

type RegistryGlobalFn =
    unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, *const c_char, u32);
type RegistryRemoveFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32);
type ShmFormatFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32);
type SeatCapabilitiesFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32);
type KeyboardKeymapFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, c_int, u32);
type KeyboardEnterFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, *mut WlProxy, *mut WlArray);
type KeyboardLeaveFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, *mut WlProxy);
type KeyboardKeyFn =
    unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, u32, u32, u32);
type KeyboardModifiersFn =
    unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, u32, u32, u32, u32);
type PointerEnterFn =
    unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, *mut WlProxy, i32, i32);
type PointerLeaveFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, *mut WlProxy);
type PointerMotionFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, i32, i32);
type PointerButtonFn =
    unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, u32, u32, u32);
type PointerAxisFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32, u32, i32);
type BufferReleaseFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy);
type WmBasePingFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32);
type XdgSurfaceConfigureFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy, u32);
type XdgToplevelConfigureFn =
    unsafe extern "C" fn(*mut c_void, *mut WlProxy, i32, i32, *mut WlArray);
type XdgToplevelCloseFn = unsafe extern "C" fn(*mut c_void, *mut WlProxy);

#[repr(C)]
struct WlRegistryListener {
    global: RegistryGlobalFn,
    global_remove: RegistryRemoveFn,
}

#[repr(C)]
struct WlShmListener {
    format: ShmFormatFn,
}

#[repr(C)]
struct WlSeatListener {
    capabilities: SeatCapabilitiesFn,
}

#[repr(C)]
struct WlKeyboardListener {
    keymap: KeyboardKeymapFn,
    enter: KeyboardEnterFn,
    leave: KeyboardLeaveFn,
    key: KeyboardKeyFn,
    modifiers: KeyboardModifiersFn,
}

#[repr(C)]
struct WlPointerListener {
    enter: PointerEnterFn,
    leave: PointerLeaveFn,
    motion: PointerMotionFn,
    button: PointerButtonFn,
    axis: PointerAxisFn,
}

#[repr(C)]
struct WlBufferListener {
    release: BufferReleaseFn,
}

#[repr(C)]
struct WlXdgWmBaseListener {
    ping: WmBasePingFn,
}

#[repr(C)]
struct WlXdgSurfaceListener {
    configure: XdgSurfaceConfigureFn,
}

#[repr(C)]
struct WlXdgToplevelListener {
    configure: XdgToplevelConfigureFn,
    close: XdgToplevelCloseFn,
}

/// Double-buffer availability state. A slot is free when its release flag is
/// set; the compositor sets the flag from `wl_buffer.release`.
struct BufferPoolState {
    released: [Box<Cell<bool>>; 2],
    next: usize,
}

impl BufferPoolState {
    fn new() -> Self {
        Self {
            released: [Box::new(Cell::new(true)), Box::new(Cell::new(true))],
            next: 0,
        }
    }

    fn acquire(&mut self) -> Option<usize> {
        for offset in 0..2 {
            let index = (self.next + offset) % 2;
            if self.released[index].get() {
                self.released[index].set(false);
                self.next = (index + 1) % 2;
                return Some(index);
            }
        }
        None
    }

    fn release(&mut self, index: usize) {
        if let Some(flag) = self.released.get(index) {
            flag.set(true);
        }
    }

    fn is_idle(&self) -> bool {
        self.released.iter().all(|flag| flag.get())
    }

    fn release_ptr(&self, index: usize) -> *mut c_void {
        self.released[index].as_ptr() as *mut c_void
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShmFormat {
    Xrgb8888,
    Argb8888,
}

impl ShmFormat {
    fn to_wire(self) -> u32 {
        match self {
            Self::Xrgb8888 => WL_SHM_FORMAT_XRGB8888,
            Self::Argb8888 => WL_SHM_FORMAT_ARGB8888,
        }
    }
}

fn wayland_damage_coords(
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    surface_w: usize,
    surface_h: usize,
) -> Option<(i32, i32, i32, i32)> {
    let clamped = DamageRect::new(x, y, w, h).clamp(surface_w, surface_h);
    if clamped.is_empty() {
        return None;
    }
    Some((
        i32::try_from(clamped.x).unwrap_or(i32::MAX),
        i32::try_from(clamped.y).unwrap_or(i32::MAX),
        i32::try_from(clamped.width).unwrap_or(i32::MAX),
        i32::try_from(clamped.height).unwrap_or(i32::MAX),
    ))
}

fn wayland_damage_rect(
    damage: Option<DamageRect>,
    force_full: bool,
    surface_w: usize,
    surface_h: usize,
) -> Option<(i32, i32, i32, i32)> {
    if force_full {
        return wayland_damage_coords(0, 0, surface_w, surface_h, surface_w, surface_h);
    }
    match damage {
        Some(rect) => wayland_damage_coords(rect.x, rect.y, rect.width, rect.height, surface_w, surface_h),
        None => wayland_damage_coords(0, 0, surface_w, surface_h, surface_w, surface_h),
    }
}

fn select_shm_format(has_xrgb8888: bool, has_argb8888: bool) -> Option<ShmFormat> {
    if has_xrgb8888 {
        Some(ShmFormat::Xrgb8888)
    } else if has_argb8888 {
        Some(ShmFormat::Argb8888)
    } else {
        None
    }
}

fn finalize_shm_pixels(pixels: &mut [u32], format: ShmFormat) {
    match format {
        ShmFormat::Xrgb8888 => {
            #[cfg(target_endian = "big")]
            for pixel in pixels.iter_mut() {
                *pixel = u32::from_ne_bytes(pixel.to_le_bytes());
            }
            #[cfg(target_endian = "little")]
            let _ = pixels;
        }
        ShmFormat::Argb8888 => {
            #[cfg(target_endian = "little")]
            for pixel in pixels.iter_mut() {
                *pixel |= 0xFF00_0000;
            }
            #[cfg(target_endian = "big")]
            for pixel in pixels.iter_mut() {
                *pixel = u32::from_ne_bytes((*pixel | 0xFF00_0000).to_le_bytes());
            }
        }
    }
}

struct ShmPool {
    fns: WaylandFns,
    pool: *mut WlProxy,
    fd: c_int,
    map: *mut u8,
    map_len: usize,
    width: usize,
    height: usize,
    format: ShmFormat,
    buffers: [*mut WlProxy; 2],
    state: BufferPoolState,
}

impl ShmPool {
    fn new(
        api: &WaylandApi,
        shm: *mut WlProxy,
        width: usize,
        height: usize,
        format: ShmFormat,
        buffer_listener: *const WlBufferListener,
    ) -> Result<Self, String> {
        let stride = width
            .checked_mul(4)
            .ok_or_else(|| "Wayland buffer stride overflow".to_string())?;
        let one_buffer = stride
            .checked_mul(height)
            .ok_or_else(|| "Wayland buffer size overflow".to_string())?;
        let total = one_buffer
            .checked_mul(2)
            .ok_or_else(|| "Wayland pool size overflow".to_string())?;
        if total == 0 || total > MAX_SHM_BYTES {
            return Err(format!("Refusing to allocate {total} byte Wayland shm pool"));
        }

        let fd = unsafe { memfd_create(c"droplet-wayland-shm".as_ptr(), MFD_CLOEXEC) };
        if fd < 0 {
            return Err(format!(
                "memfd_create failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        if unsafe { ftruncate(fd, total as i64) } != 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                close(fd);
            }
            return Err(format!("ftruncate failed: {err}"));
        }
        let map = unsafe {
            mmap(
                ptr::null_mut(),
                total,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd,
                0,
            )
        };
        if map == usize::MAX as *mut c_void || map.is_null() {
            let err = std::io::Error::last_os_error();
            unsafe {
                close(fd);
            }
            return Err(format!("mmap failed: {err}"));
        }

        let pool = unsafe {
            (api.wl_proxy_marshal_flags)(
                shm,
                WL_SHM_CREATE_POOL,
                &WL_SHM_POOL_INTERFACE as *const WlInterface,
                1,
                0,
                ptr::null_mut::<c_void>(),
                fd,
                total as c_int,
            )
        };
        if pool.is_null() {
            unsafe {
                munmap(map, total);
                close(fd);
            }
            return Err("wl_shm.create_pool failed".into());
        }

        let mut this = Self {
            fns: api.fns(),
            pool,
            fd,
            map: map as *mut u8,
            map_len: total,
            width,
            height,
            format,
            buffers: [ptr::null_mut(); 2],
            state: BufferPoolState::new(),
        };

        for (index, offset) in [0usize, one_buffer].into_iter().enumerate() {
            let buffer = unsafe {
                (api.wl_proxy_marshal_flags)(
                    pool,
                    WL_SHM_POOL_CREATE_BUFFER,
                    &WL_BUFFER_INTERFACE as *const WlInterface,
                    1,
                    0,
                    ptr::null_mut::<c_void>(),
                    offset as c_int,
                    width as c_int,
                    height as c_int,
                    stride as c_int,
                    format.to_wire(),
                )
            };
            if buffer.is_null() {
                return Err("wl_shm_pool.create_buffer failed".into());
            }
            this.buffers[index] = buffer;
            let data = this.state.release_ptr(index);
            let rc = unsafe {
                (api.wl_proxy_add_listener)(
                    buffer,
                    buffer_listener as *const *const c_void,
                    data,
                )
            };
            if rc != 0 {
                return Err("wl_proxy_add_listener(wl_buffer) failed".into());
            }
        }

        Ok(this)
    }

    fn acquire(&mut self) -> Option<usize> {
        self.state.acquire()
    }

    fn release(&mut self, index: usize) {
        self.state.release(index);
    }

    fn is_idle(&self) -> bool {
        self.state.is_idle()
    }

    fn buffer_ptr(&self, index: usize) -> Option<*mut WlProxy> {
        let buffer = *self.buffers.get(index)?;
        if buffer.is_null() {
            None
        } else {
            Some(buffer)
        }
    }

    fn buffer_slice_mut(&mut self, index: usize) -> Option<(&mut [u32], *mut WlProxy)> {
        let buffer = self.buffer_ptr(index)?;
        let pixel_count = self.width.checked_mul(self.height)?;
        let buffer_bytes = pixel_count.checked_mul(4)?;
        let offset = index.checked_mul(buffer_bytes)?;
        if offset.checked_add(buffer_bytes)? > self.map_len {
            return None;
        }
        let ptr = unsafe { self.map.add(offset) as *mut u32 };
        let slice = unsafe { slice::from_raw_parts_mut(ptr, pixel_count) };
        Some((slice, buffer))
    }
}

impl Drop for ShmPool {
    fn drop(&mut self) {
        for buffer in self.buffers {
            if !buffer.is_null() {
                unsafe {
                    (self.fns.marshal_flags)(
                        buffer,
                        WL_BUFFER_DESTROY,
                        ptr::null(),
                        1,
                        WL_MARSHAL_FLAG_DESTROY,
                    );
                }
            }
        }
        if !self.pool.is_null() {
            unsafe {
                (self.fns.marshal_flags)(
                    self.pool,
                    WL_SHM_POOL_DESTROY,
                    ptr::null(),
                    1,
                    WL_MARSHAL_FLAG_DESTROY,
                );
            }
        }
        if !self.map.is_null() {
            unsafe {
                munmap(self.map as *mut c_void, self.map_len);
            }
        }
        if self.fd >= 0 {
            unsafe {
                close(self.fd);
            }
        }
    }
}

struct WaylandState<A: PlatformApp> {
    app: *mut A,
    display: *mut WlDisplay,
    registry: *mut WlProxy,
    compositor: *mut WlProxy,
    shm: *mut WlProxy,
    seat: *mut WlProxy,
    keyboard: *mut WlProxy,
    pointer: *mut WlProxy,
    wm_base: *mut WlProxy,
    surface: *mut WlProxy,
    xdg_surface: *mut WlProxy,
    xdg_toplevel: *mut WlProxy,
    pool: Option<ShmPool>,
    retiring_pools: Vec<ShmPool>,

    registry_state: RegistryState,
    compositor_version: u32,
    shm_xrgb8888: bool,
    shm_argb8888: bool,
    shm_format: ShmFormat,
    configured: bool,
    running: bool,
    width: usize,
    height: usize,
    pending_toplevel_size: Option<(usize, usize)>,
    pending_resize: Option<(usize, usize)>,
    force_repaint: bool,
    mods: KeyModifiers,
    pointer_x: f32,
    pointer_y: f32,
    last_title: String,
    error: Option<String>,

    registry_listener: WlRegistryListener,
    shm_listener: WlShmListener,
    seat_listener: WlSeatListener,
    keyboard_listener: WlKeyboardListener,
    pointer_listener: WlPointerListener,
    buffer_listener: WlBufferListener,
    wm_base_listener: WlXdgWmBaseListener,
    xdg_surface_listener: WlXdgSurfaceListener,
    xdg_toplevel_listener: WlXdgToplevelListener,

    api: WaylandApi,
}

impl<A: PlatformApp> WaylandState<A> {
    fn new(
        api: WaylandApi,
        display: *mut WlDisplay,
        app: *mut A,
        width: usize,
        height: usize,
    ) -> Self {
        Self {
            app,
            display,
            registry: ptr::null_mut(),
            compositor: ptr::null_mut(),
            shm: ptr::null_mut(),
            seat: ptr::null_mut(),
            keyboard: ptr::null_mut(),
            pointer: ptr::null_mut(),
            wm_base: ptr::null_mut(),
            surface: ptr::null_mut(),
            xdg_surface: ptr::null_mut(),
            xdg_toplevel: ptr::null_mut(),
            pool: None,
            retiring_pools: Vec::new(),
            registry_state: RegistryState::default(),
            compositor_version: 1,
            shm_xrgb8888: false,
            shm_argb8888: false,
            shm_format: ShmFormat::Xrgb8888,
            configured: false,
            running: true,
            width: width.max(1),
            height: height.max(1),
            pending_toplevel_size: None,
            pending_resize: None,
            force_repaint: true,
            mods: KeyModifiers::default(),
            pointer_x: 0.0,
            pointer_y: 0.0,
            last_title: String::new(),
            error: None,
            registry_listener: WlRegistryListener {
                global: registry_global::<A>,
                global_remove: registry_global_remove,
            },
            shm_listener: WlShmListener {
                format: shm_format::<A>,
            },
            seat_listener: WlSeatListener {
                capabilities: seat_capabilities::<A>,
            },
            keyboard_listener: WlKeyboardListener {
                keymap: keyboard_keymap::<A>,
                enter: keyboard_enter::<A>,
                leave: keyboard_leave::<A>,
                key: keyboard_key::<A>,
                modifiers: keyboard_modifiers::<A>,
            },
            pointer_listener: WlPointerListener {
                enter: pointer_enter::<A>,
                leave: pointer_leave::<A>,
                motion: pointer_motion::<A>,
                button: pointer_button::<A>,
                axis: pointer_axis::<A>,
            },
            buffer_listener: WlBufferListener {
                release: buffer_release,
            },
            wm_base_listener: WlXdgWmBaseListener {
                ping: wm_base_ping::<A>,
            },
            xdg_surface_listener: WlXdgSurfaceListener {
                configure: xdg_surface_configure::<A>,
            },
            xdg_toplevel_listener: WlXdgToplevelListener {
                configure: xdg_toplevel_configure::<A>,
                close: xdg_toplevel_close::<A>,
            },
            api,
        }
    }

    fn app(&mut self) -> &mut A {
        unsafe { &mut *self.app }
    }

    fn add_proxy_listener<T>(&mut self, proxy: *mut WlProxy, listener: *const T) -> bool {
        let data = self as *mut Self as *mut c_void;
        let rc = unsafe {
            (self.api.wl_proxy_add_listener)(proxy, listener as *const *const c_void, data)
        };
        rc == 0
    }

    fn add_registry_listener(&mut self) {
        let proxy = self.registry;
        let listener = &self.registry_listener as *const WlRegistryListener;
        if !self.add_proxy_listener(proxy, listener) {
            self.error = Some("wl_proxy_add_listener(wl_registry) failed".into());
        }
    }

    fn roundtrip(&mut self, phase: &str) -> Result<(), String> {
        let rc = unsafe { (self.api.wl_display_roundtrip)(self.display) };
        if rc < 0 {
            return Err(format!(
                "Wayland {phase} roundtrip failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn registry_bind(
        &self,
        name: u32,
        interface: *const WlInterface,
        version: u32,
    ) -> *mut WlProxy {
        let interface_name = unsafe { CStr::from_ptr((*interface).name) };
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.registry,
                WL_REGISTRY_BIND,
                interface,
                version,
                0,
                name,
                interface_name.as_ptr(),
                version,
                ptr::null_mut::<c_void>(),
            )
        }
    }

    fn create_surface(&self) -> *mut WlProxy {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.compositor,
                WL_COMPOSITOR_CREATE_SURFACE,
                &WL_SURFACE_INTERFACE as *const WlInterface,
                self.compositor_version.max(1),
                0,
                ptr::null_mut::<c_void>(),
            )
        }
    }

    fn get_xdg_surface(&self, surface: *mut WlProxy) -> *mut WlProxy {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.wm_base,
                XDG_WM_BASE_GET_XDG_SURFACE,
                &XDG_SURFACE_INTERFACE as *const WlInterface,
                1,
                0,
                ptr::null_mut::<c_void>(),
                surface,
            )
        }
    }

    fn get_toplevel(&self) -> *mut WlProxy {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.xdg_surface,
                XDG_SURFACE_GET_TOPLEVEL,
                &XDG_TOPLEVEL_INTERFACE as *const WlInterface,
                1,
                0,
                ptr::null_mut::<c_void>(),
            )
        }
    }

    fn get_keyboard(&self) -> *mut WlProxy {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.seat,
                WL_SEAT_GET_KEYBOARD,
                &WL_KEYBOARD_INTERFACE as *const WlInterface,
                1,
                0,
                ptr::null_mut::<c_void>(),
            )
        }
    }

    fn get_pointer(&self) -> *mut WlProxy {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.seat,
                WL_SEAT_GET_POINTER,
                &WL_POINTER_INTERFACE as *const WlInterface,
                1,
                0,
                ptr::null_mut::<c_void>(),
            )
        }
    }

    fn sync_title(&mut self, force: bool) {
        let title = format_window_title(self.app().active_title());
        if !force && title == self.last_title {
            return;
        }
        if let Ok(c_title) = CString::new(title.clone()) {
            unsafe {
                (self.api.wl_proxy_marshal_flags)(
                    self.xdg_toplevel,
                    XDG_TOPLEVEL_SET_TITLE,
                    ptr::null(),
                    1,
                    0,
                    c_title.as_ptr(),
                );
            }
            self.last_title = title;
        }
    }

    fn set_app_id(&self) {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.xdg_toplevel,
                XDG_TOPLEVEL_SET_APP_ID,
                ptr::null(),
                1,
                0,
                c"droplet".as_ptr(),
            );
        }
    }

    fn set_window_geometry(&self, x: i32, y: i32, width: i32, height: i32) {
        if self.xdg_surface.is_null() {
            return;
        }
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.xdg_surface,
                XDG_SURFACE_SET_WINDOW_GEOMETRY,
                ptr::null(),
                1,
                0,
                x,
                y,
                width,
                height,
            );
        }
    }

    fn surface_attach(&self, buffer: *mut WlProxy) {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.surface,
                WL_SURFACE_ATTACH,
                ptr::null(),
                1,
                0,
                buffer,
                0 as c_int,
                0 as c_int,
            );
        }
    }

    fn surface_damage_region(&self, x: i32, y: i32, width: i32, height: i32) {
        let opcode = if self.compositor_version >= 4 {
            WL_SURFACE_DAMAGE_BUFFER
        } else {
            WL_SURFACE_DAMAGE
        };
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.surface,
                opcode,
                ptr::null(),
                1,
                0,
                x,
                y,
                width,
                height,
            );
        }
    }

    fn surface_commit(&self) {
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.surface,
                WL_SURFACE_COMMIT,
                ptr::null(),
                1,
                0,
            );
        }
    }

    fn destroy_request(&self, proxy: *mut WlProxy, opcode: u32) {
        if proxy.is_null() {
            return;
        }
        unsafe {
            (self.api.wl_proxy_marshal_flags)(
                proxy,
                opcode,
                ptr::null(),
                1,
                WL_MARSHAL_FLAG_DESTROY,
            );
        }
    }

    fn sync_seat_devices(&mut self, capabilities: u32) {
        if capabilities & WL_SEAT_CAPABILITY_KEYBOARD != 0 {
            if self.keyboard.is_null() {
                let keyboard = self.get_keyboard();
                if keyboard.is_null() {
                    self.error = Some("wl_seat.get_keyboard failed".into());
                } else {
                    self.keyboard = keyboard;
                    let listener = &self.keyboard_listener as *const WlKeyboardListener;
                    if !self.add_proxy_listener(keyboard, listener) {
                        self.error = Some("wl_proxy_add_listener(wl_keyboard) failed".into());
                    }
                }
            }
        } else if !self.keyboard.is_null() {
            let keyboard = self.keyboard;
            self.keyboard = ptr::null_mut();
            unsafe {
                (self.api.wl_proxy_destroy)(keyboard);
            }
        }

        if capabilities & WL_SEAT_CAPABILITY_POINTER != 0 {
            if self.pointer.is_null() {
                let pointer = self.get_pointer();
                if pointer.is_null() {
                    self.error = Some("wl_seat.get_pointer failed".into());
                } else {
                    self.pointer = pointer;
                    let listener = &self.pointer_listener as *const WlPointerListener;
                    if !self.add_proxy_listener(pointer, listener) {
                        self.error = Some("wl_proxy_add_listener(wl_pointer) failed".into());
                    }
                }
            }
        } else if !self.pointer.is_null() {
            let pointer = self.pointer;
            self.pointer = ptr::null_mut();
            unsafe {
                (self.api.wl_proxy_destroy)(pointer);
            }
        }
    }

    fn init(&mut self) -> Result<(), String> {
        let registry = unsafe {
            (self.api.wl_proxy_marshal_flags)(
                self.display as *mut WlProxy,
                WL_DISPLAY_GET_REGISTRY,
                &WL_REGISTRY_INTERFACE as *const WlInterface,
                1,
                0,
                ptr::null_mut::<c_void>(),
            )
        };
        if registry.is_null() {
            return Err("wl_display.get_registry failed".into());
        }
        self.registry = registry;
        self.add_registry_listener();
        if let Some(error) = self.error.take() {
            return Err(error);
        }

        self.roundtrip("registry")?;
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if !self.registry_state.complete() {
            return Err(format!(
                "Wayland compositor is missing required globals: {}",
                self.registry_state.missing().join(", ")
            ));
        }

        self.roundtrip("globals")?;
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let Some(shm_format) = select_shm_format(self.shm_xrgb8888, self.shm_argb8888) else {
            return Err("Wayland compositor does not advertise wl_shm XRGB8888 or ARGB8888".into());
        };
        self.shm_format = shm_format;
        if self.keyboard.is_null() && self.pointer.is_null() {
            self.roundtrip("seat capabilities")?;
            if let Some(error) = self.error.take() {
                return Err(error);
            }
        }

        let surface = self.create_surface();
        if surface.is_null() {
            return Err("wl_compositor.create_surface failed".into());
        }
        self.surface = surface;

        let xdg_surface = self.get_xdg_surface(surface);
        if xdg_surface.is_null() {
            return Err("xdg_wm_base.get_xdg_surface failed".into());
        }
        self.xdg_surface = xdg_surface;

        let xdg_toplevel = self.get_toplevel();
        if xdg_toplevel.is_null() {
            return Err("xdg_surface.get_toplevel failed".into());
        }
        self.xdg_toplevel = xdg_toplevel;

        let listener = &self.xdg_surface_listener as *const WlXdgSurfaceListener;
        if !self.add_proxy_listener(xdg_surface, listener) {
            return Err("wl_proxy_add_listener(xdg_surface) failed".into());
        }
        let listener = &self.xdg_toplevel_listener as *const WlXdgToplevelListener;
        if !self.add_proxy_listener(xdg_toplevel, listener) {
            return Err("wl_proxy_add_listener(xdg_toplevel) failed".into());
        }

        self.set_app_id();
        self.sync_title(true);
        self.surface_commit();

        for _ in 0..8 {
            if self.configured {
                break;
            }
            self.roundtrip("configure")?;
            if let Some(error) = self.error.take() {
                return Err(error);
            }
        }
        if !self.configured {
            return Err("xdg_surface did not send an initial configure event".into());
        }

        self.apply_pending_size();
        self.set_window_geometry(0, 0, self.width as i32, self.height as i32);

        let listener = &self.buffer_listener as *const WlBufferListener;
        let pool = ShmPool::new(
            &self.api,
            self.shm,
            self.width,
            self.height,
            self.shm_format,
            listener,
        )?;
        self.pool = Some(pool);
        self.force_repaint = true;
        self.app().set_present_backend("wayland-shm");
        Ok(())
    }

    fn apply_pending_size(&mut self) {
        let Some((width, height)) = self.pending_resize.take() else {
            return;
        };
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.app().resize(width, height);
        self.force_repaint = true;
        self.set_window_geometry(0, 0, width as i32, height as i32);
    }

    fn present(&mut self, force: bool) -> Result<(), String> {
        self.retiring_pools.retain(|pool| !pool.is_idle());

        let needs_pool = self
            .pool
            .as_ref()
            .is_none_or(|pool| pool.width != self.width || pool.height != self.height);
        if needs_pool {
            if let Some(old_pool) = self.pool.take() {
                if !old_pool.is_idle() {
                    if self.retiring_pools.len() >= 4 {
                        self.retiring_pools.remove(0);
                    }
                    self.retiring_pools.push(old_pool);
                }
            }
            let listener = &self.buffer_listener as *const WlBufferListener;
            let pool = ShmPool::new(
                &self.api,
                self.shm,
                self.width,
                self.height,
                self.shm_format,
                listener,
            )?;
            self.pool = Some(pool);
        }

        let index = {
            let Some(pool) = self.pool.as_mut() else {
                return Ok(());
            };
            pool.acquire()
        };
        let Some(index) = index else {
            self.force_repaint = true;
            return Ok(());
        };

        let damage = self.app().take_damage();
        let force_full = force || needs_pool;
        let width = self.width;
        let height = self.height;
        let app_ptr = self.app;

        let buffer = {
            let Some(pool) = self.pool.as_mut() else {
                return Ok(());
            };
            let pool_format = pool.format;
            match pool.buffer_slice_mut(index) {
                Some((slice, buffer)) => {
                    Canvas::with_external_slice(
                        width,
                        height,
                        slice,
                        Color::rgb(255, 255, 255),
                        |canvas| {
                            let app = unsafe { &mut *app_ptr };
                            app.render_into(canvas);
                            app.set_needs_repaint(false);
                        },
                    );
                    finalize_shm_pixels(slice, pool_format);
                    buffer
                }
                None => {
                    pool.release(index);
                    ptr::null_mut()
                }
            }
        };
        if buffer.is_null() {
            self.force_repaint = true;
            return Ok(());
        }

        self.surface_attach(buffer);
        if let Some((dx, dy, dw, dh)) = wayland_damage_rect(damage, force_full, width, height) {
            self.surface_damage_region(dx, dy, dw, dh);
        }
        self.surface_commit();
        self.force_repaint = false;
        Ok(())
    }

    fn run_loop(&mut self) -> Result<(), String> {
        let mut first_frame = true;

        while self.running {
            self.retiring_pools.retain(|pool| !pool.is_idle());
            self.apply_pending_size();
            self.sync_title(false);

            if first_frame || self.force_repaint || self.app().needs_repaint() {
                self.present(first_frame)?;
                first_frame = false;
            }
            if !self.running {
                break;
            }

            unsafe {
                (self.api.wl_display_flush)(self.display);
            }
            let rc = unsafe { (self.api.wl_display_dispatch)(self.display) };
            if rc < 0 {
                return Err(format!(
                    "wl_display_dispatch failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if let Some(error) = self.error.take() {
                return Err(error);
            }
        }
        Ok(())
    }
}

impl<A: PlatformApp> Drop for WaylandState<A> {
    fn drop(&mut self) {
        if self.display.is_null() {
            return;
        }

        self.destroy_request(self.xdg_toplevel, XDG_TOPLEVEL_DESTROY);
        self.xdg_toplevel = ptr::null_mut();
        self.destroy_request(self.xdg_surface, XDG_SURFACE_DESTROY);
        self.xdg_surface = ptr::null_mut();
        self.destroy_request(self.surface, WL_SURFACE_DESTROY);
        self.surface = ptr::null_mut();

        self.pool = None;
        self.retiring_pools.clear();
        self.destroy_request(self.wm_base, XDG_WM_BASE_DESTROY);
        self.wm_base = ptr::null_mut();

        for proxy in [
            self.keyboard,
            self.pointer,
            self.seat,
            self.shm,
            self.compositor,
            self.registry,
        ] {
            if !proxy.is_null() {
                unsafe {
                    (self.api.wl_proxy_destroy)(proxy);
                }
            }
        }
        self.keyboard = ptr::null_mut();
        self.pointer = ptr::null_mut();
        self.seat = ptr::null_mut();
        self.shm = ptr::null_mut();
        self.compositor = ptr::null_mut();
        self.registry = ptr::null_mut();

        unsafe {
            (self.api.wl_display_flush)(self.display);
            (self.api.wl_display_disconnect)(self.display);
        }
        self.display = ptr::null_mut();
    }
}

unsafe extern "C" fn registry_global<A: PlatformApp>(
    data: *mut c_void,
    _registry: *mut WlProxy,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    let interface_name = match CStr::from_ptr(interface).to_str() {
        Ok(name) => name,
        Err(_) => return,
    };
    let Some(mut binding) = state.registry_state.accept(interface_name, version) else {
        return;
    };
    binding.name = name;

    let interface_ptr = match binding.kind {
        GlobalKind::Compositor => &WL_COMPOSITOR_INTERFACE as *const WlInterface,
        GlobalKind::Shm => &WL_SHM_INTERFACE as *const WlInterface,
        GlobalKind::Seat => &WL_SEAT_INTERFACE as *const WlInterface,
        GlobalKind::XdgWmBase => &XDG_WM_BASE_INTERFACE as *const WlInterface,
    };
    let proxy = state.registry_bind(binding.name, interface_ptr, binding.version);
    if proxy.is_null() {
        state.error = Some(format!("wl_registry.bind({interface_name}) failed"));
        return;
    }

    match binding.kind {
        GlobalKind::Compositor => {
            state.compositor = proxy;
            state.compositor_version = binding.version;
        }
        GlobalKind::Shm => {
            state.shm = proxy;
            let listener = &state.shm_listener as *const WlShmListener;
            if !state.add_proxy_listener(proxy, listener) {
                state.error = Some("wl_proxy_add_listener(wl_shm) failed".into());
            }
        }
        GlobalKind::Seat => {
            state.seat = proxy;
            let listener = &state.seat_listener as *const WlSeatListener;
            if !state.add_proxy_listener(proxy, listener) {
                state.error = Some("wl_proxy_add_listener(wl_seat) failed".into());
            }
        }
        GlobalKind::XdgWmBase => {
            state.wm_base = proxy;
            let listener = &state.wm_base_listener as *const WlXdgWmBaseListener;
            if !state.add_proxy_listener(proxy, listener) {
                state.error = Some("wl_proxy_add_listener(xdg_wm_base) failed".into());
            }
        }
    }
}

unsafe extern "C" fn registry_global_remove(
    _data: *mut c_void,
    _registry: *mut WlProxy,
    _name: u32,
) {
}

unsafe extern "C" fn shm_format<A: PlatformApp>(
    data: *mut c_void,
    _shm: *mut WlProxy,
    format: u32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    match format {
        WL_SHM_FORMAT_XRGB8888 => state.shm_xrgb8888 = true,
        WL_SHM_FORMAT_ARGB8888 => state.shm_argb8888 = true,
        _ => {}
    }
}

unsafe extern "C" fn seat_capabilities<A: PlatformApp>(
    data: *mut c_void,
    _seat: *mut WlProxy,
    capabilities: u32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    state.sync_seat_devices(capabilities);
}

unsafe extern "C" fn keyboard_keymap<A: PlatformApp>(
    data: *mut c_void,
    _keyboard: *mut WlProxy,
    format: u32,
    fd: c_int,
    _size: u32,
) {
    let _ = data as *mut WaylandState<A>;
    // The compositor keymap is delivered as an XKB text description. This
    // zero-dependency backend uses a compact US layout table instead of
    // linking/parsing XKB, but the descriptor must always be closed.
    let _supported = format == WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1;
    unsafe {
        close(fd);
    }
}

unsafe extern "C" fn keyboard_enter<A: PlatformApp>(
    _data: *mut c_void,
    _keyboard: *mut WlProxy,
    _serial: u32,
    _surface: *mut WlProxy,
    _keys: *mut WlArray,
) {
}

unsafe extern "C" fn keyboard_leave<A: PlatformApp>(
    _data: *mut c_void,
    _keyboard: *mut WlProxy,
    _serial: u32,
    _surface: *mut WlProxy,
) {
}

unsafe extern "C" fn keyboard_key<A: PlatformApp>(
    data: *mut c_void,
    _keyboard: *mut WlProxy,
    _serial: u32,
    _time: u32,
    key: u32,
    key_state: u32,
) {
    if key_state != WL_KEYBOARD_KEY_STATE_PRESSED {
        return;
    }
    let state = &mut *(data as *mut WaylandState<A>);
    let focused = state.app().url_focused();
    let action = map_key(key, state.mods, focused);
    apply_key_action(state.app(), action);
}

unsafe extern "C" fn keyboard_modifiers<A: PlatformApp>(
    data: *mut c_void,
    _keyboard: *mut WlProxy,
    _serial: u32,
    depressed: u32,
    _latched: u32,
    locked: u32,
    _group: u32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    state.mods = KeyModifiers::from_wayland(depressed, locked);
}

unsafe extern "C" fn pointer_enter<A: PlatformApp>(
    data: *mut c_void,
    _pointer: *mut WlProxy,
    _serial: u32,
    _surface: *mut WlProxy,
    surface_x: i32,
    surface_y: i32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    let x = fixed_to_f32(surface_x);
    let y = fixed_to_f32(surface_y);
    state.pointer_x = x;
    state.pointer_y = y;
    state.app().handle_mouse_move(x, y);
}

unsafe extern "C" fn pointer_leave<A: PlatformApp>(
    _data: *mut c_void,
    _pointer: *mut WlProxy,
    _serial: u32,
    _surface: *mut WlProxy,
) {
}

unsafe extern "C" fn pointer_motion<A: PlatformApp>(
    data: *mut c_void,
    _pointer: *mut WlProxy,
    _time: u32,
    surface_x: i32,
    surface_y: i32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    let x = fixed_to_f32(surface_x);
    let y = fixed_to_f32(surface_y);
    state.pointer_x = x;
    state.pointer_y = y;
    state.app().handle_mouse_move(x, y);
}

unsafe extern "C" fn pointer_button<A: PlatformApp>(
    data: *mut c_void,
    _pointer: *mut WlProxy,
    _serial: u32,
    _time: u32,
    button: u32,
    button_state: u32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    let (x, y) = (state.pointer_x, state.pointer_y);
    if button == BTN_LEFT {
        if button_state == WL_POINTER_BUTTON_STATE_PRESSED {
            state.app().handle_mouse_down(x, y);
        } else {
            state.app().handle_mouse_up(x, y);
        }
    } else if button_state == WL_POINTER_BUTTON_STATE_PRESSED {
        match button {
            BTN_SIDE => state.app().handle_key_navigation(NavKey::Back),
            BTN_EXTRA => state.app().handle_key_navigation(NavKey::Forward),
            _ => {}
        }
    }
}

unsafe extern "C" fn pointer_axis<A: PlatformApp>(
    data: *mut c_void,
    _pointer: *mut WlProxy,
    _time: u32,
    axis: u32,
    value: i32,
) {
    if axis != WL_POINTER_AXIS_VERTICAL_SCROLL {
        return;
    }
    let state = &mut *(data as *mut WaylandState<A>);
    let delta = axis_scroll_delta(fixed_to_f32(value));
    if delta != 0.0 {
        state.app().scroll(delta);
    }
}

unsafe extern "C" fn buffer_release(data: *mut c_void, _buffer: *mut WlProxy) {
    let released = &*(data as *const Cell<bool>);
    released.set(true);
}

unsafe extern "C" fn wm_base_ping<A: PlatformApp>(
    data: *mut c_void,
    _wm_base: *mut WlProxy,
    serial: u32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    if !state.wm_base.is_null() {
        unsafe {
            (state.api.wl_proxy_marshal_flags)(
                state.wm_base,
                XDG_WM_BASE_PONG,
                ptr::null(),
                1,
                0,
                serial,
            );
        }
    }
}

unsafe extern "C" fn xdg_surface_configure<A: PlatformApp>(
    data: *mut c_void,
    _xdg_surface: *mut WlProxy,
    serial: u32,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    if !state.xdg_surface.is_null() {
        unsafe {
            (state.api.wl_proxy_marshal_flags)(
                state.xdg_surface,
                XDG_SURFACE_ACK_CONFIGURE,
                ptr::null(),
                1,
                0,
                serial,
            );
        }
    }
    if let Some(size) = state.pending_toplevel_size.take() {
        state.pending_resize = Some(size);
    }
    state.configured = true;
}

unsafe extern "C" fn xdg_toplevel_configure<A: PlatformApp>(
    data: *mut c_void,
    _xdg_toplevel: *mut WlProxy,
    width: i32,
    height: i32,
    _states: *mut WlArray,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    if width > 0 && height > 0 {
        state.pending_toplevel_size = Some((width as usize, height as usize));
    }
}

unsafe extern "C" fn xdg_toplevel_close<A: PlatformApp>(
    data: *mut c_void,
    _xdg_toplevel: *mut WlProxy,
) {
    let state = &mut *(data as *mut WaylandState<A>);
    state.running = false;
}

/// Runs the Wayland backend against an already-borrowed application. On
/// `RunError::Init` the caller may retry with X11; `RunError::Runtime` means the
/// window was already live.
pub fn run_with<A: PlatformApp>(app: &mut A) -> Result<(), RunError> {
    if !crate::wayland_env_present() {
        return Err(RunError::Init(
            "WAYLAND_DISPLAY and XDG_RUNTIME_DIR must both be set".into(),
        ));
    }

    let api = WaylandApi::load().map_err(RunError::Init)?;
    let display = unsafe { (api.wl_display_connect)(ptr::null()) };
    if display.is_null() {
        return Err(RunError::Init(format!(
            "wl_display_connect failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    let (width, height) = app.viewport_size();
    let mut state = Box::new(WaylandState::new(
        api,
        display,
        app as *mut A,
        width,
        height,
    ));
    state.init().map_err(RunError::Init)?;
    state.run_loop().map_err(RunError::Runtime)
}

/// Pure-Rust Wayland wire-format encoder/decoder used to validate protocol
/// framing, padding, fixed-point conversion, and message layouts in tests.
#[cfg(test)]
mod wire {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) enum WireError {
        Truncated,
        InvalidHeaderSize(u16),
        UnalignedHeaderSize(u16),
        SizeMismatch { declared: usize, actual: usize },
        EmbeddedNul,
        MissingNul,
        InvalidUtf8,
        TrailingBytes(usize),
        LengthOverflow,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct WireHeader {
        pub(super) object_id: u32,
        pub(super) opcode: u16,
        pub(super) size: u16,
    }

    impl WireHeader {
        pub(super) fn encode(self) -> [u8; 8] {
            let mut out = [0u8; 8];
            out[0..4].copy_from_slice(&self.object_id.to_ne_bytes());
            let word2 = ((self.size as u32) << 16) | (self.opcode as u32);
            out[4..8].copy_from_slice(&word2.to_ne_bytes());
            out
        }

        pub(super) fn decode(bytes: &[u8]) -> Result<Self, WireError> {
            if bytes.len() < 8 {
                return Err(WireError::Truncated);
            }
            let object_id = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let word2 = u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let size = (word2 >> 16) as u16;
            let opcode = (word2 & 0xFFFF) as u16;
            if size < 8 {
                return Err(WireError::InvalidHeaderSize(size));
            }
            if !size.is_multiple_of(4) {
                return Err(WireError::UnalignedHeaderSize(size));
            }
            Ok(Self {
                object_id,
                opcode,
                size,
            })
        }
    }

    pub(super) struct WireWriter {
        buf: Vec<u8>,
    }

    impl WireWriter {
        pub(super) fn begin(object_id: u32, opcode: u16) -> Self {
            let mut buf = Vec::with_capacity(32);
            buf.extend_from_slice(
                &WireHeader {
                    object_id,
                    opcode,
                    size: 8,
                }
                .encode(),
            );
            Self { buf }
        }

        pub(super) fn write_u32(&mut self, value: u32) {
            self.buf.extend_from_slice(&value.to_ne_bytes());
        }

        pub(super) fn write_i32(&mut self, value: i32) {
            self.buf.extend_from_slice(&value.to_ne_bytes());
        }

        pub(super) fn write_fixed(&mut self, value: f32) {
            let fixed = (value * 256.0).round() as i32;
            self.write_i32(fixed);
        }

        pub(super) fn write_string(&mut self, value: &str) -> Result<(), WireError> {
            if value.as_bytes().contains(&0) {
                return Err(WireError::EmbeddedNul);
            }
            let len_with_nul = value
                .len()
                .checked_add(1)
                .ok_or(WireError::LengthOverflow)?;
            let wire_len = u32::try_from(len_with_nul).map_err(|_| WireError::LengthOverflow)?;
            self.write_u32(wire_len);
            self.buf.extend_from_slice(value.as_bytes());
            self.buf.push(0);
            let pad = (4 - (len_with_nul % 4)) % 4;
            self.buf.resize(self.buf.len() + pad, 0);
            Ok(())
        }

        pub(super) fn write_array(&mut self, bytes: &[u8]) -> Result<(), WireError> {
            let wire_len = u32::try_from(bytes.len()).map_err(|_| WireError::LengthOverflow)?;
            self.write_u32(wire_len);
            self.buf.extend_from_slice(bytes);
            let pad = (4 - (bytes.len() % 4)) % 4;
            self.buf.resize(self.buf.len() + pad, 0);
            Ok(())
        }

        pub(super) fn finish(mut self) -> Result<Vec<u8>, WireError> {
            let size = u16::try_from(self.buf.len()).map_err(|_| WireError::LengthOverflow)?;
            let header = WireHeader {
                object_id: u32::from_ne_bytes([
                    self.buf[0],
                    self.buf[1],
                    self.buf[2],
                    self.buf[3],
                ]),
                opcode: (u32::from_ne_bytes([
                    self.buf[4],
                    self.buf[5],
                    self.buf[6],
                    self.buf[7],
                ]) & 0xFFFF) as u16,
                size,
            };
            self.buf[0..8].copy_from_slice(&header.encode());
            Ok(self.buf)
        }
    }

    pub(super) struct WireReader<'a> {
        payload: &'a [u8],
        pos: usize,
        header: WireHeader,
    }

    impl<'a> WireReader<'a> {
        pub(super) fn open(packet: &'a [u8]) -> Result<Self, WireError> {
            let header = WireHeader::decode(packet)?;
            let declared = header.size as usize;
            if packet.len() < declared {
                return Err(WireError::Truncated);
            }
            if packet.len() != declared {
                return Err(WireError::SizeMismatch {
                    declared,
                    actual: packet.len(),
                });
            }
            Ok(Self {
                payload: &packet[8..declared],
                pos: 0,
                header,
            })
        }

        pub(super) fn header(&self) -> WireHeader {
            self.header
        }

        pub(super) fn read_u32(&mut self) -> Result<u32, WireError> {
            let end = self.pos.checked_add(4).ok_or(WireError::LengthOverflow)?;
            if end > self.payload.len() {
                return Err(WireError::Truncated);
            }
            let val = u32::from_ne_bytes([
                self.payload[self.pos],
                self.payload[self.pos + 1],
                self.payload[self.pos + 2],
                self.payload[self.pos + 3],
            ]);
            self.pos = end;
            Ok(val)
        }

        pub(super) fn read_i32(&mut self) -> Result<i32, WireError> {
            Ok(self.read_u32()? as i32)
        }

        pub(super) fn read_fixed(&mut self) -> Result<f32, WireError> {
            Ok(super::fixed_to_f32(self.read_i32()?))
        }

        pub(super) fn read_string(&mut self) -> Result<&'a str, WireError> {
            let len_with_nul = self.read_u32()? as usize;
            if len_with_nul == 0 {
                return Err(WireError::MissingNul);
            }
            let padded = len_with_nul
                .checked_add((4 - (len_with_nul % 4)) % 4)
                .ok_or(WireError::LengthOverflow)?;
            let end = self.pos.checked_add(padded).ok_or(WireError::LengthOverflow)?;
            if end > self.payload.len() {
                return Err(WireError::Truncated);
            }
            let raw = &self.payload[self.pos..self.pos + len_with_nul];
            if raw[len_with_nul - 1] != 0 {
                return Err(WireError::MissingNul);
            }
            let text_bytes = &raw[..len_with_nul - 1];
            if text_bytes.contains(&0) {
                return Err(WireError::EmbeddedNul);
            }
            let text = std::str::from_utf8(text_bytes).map_err(|_| WireError::InvalidUtf8)?;
            self.pos = end;
            Ok(text)
        }

        pub(super) fn read_array(&mut self) -> Result<&'a [u8], WireError> {
            let len = self.read_u32()? as usize;
            let padded = len
                .checked_add((4 - (len % 4)) % 4)
                .ok_or(WireError::LengthOverflow)?;
            let end = self.pos.checked_add(padded).ok_or(WireError::LengthOverflow)?;
            if end > self.payload.len() {
                return Err(WireError::Truncated);
            }
            let slice = &self.payload[self.pos..self.pos + len];
            self.pos = end;
            Ok(slice)
        }

        pub(super) fn finish(self) -> Result<(), WireError> {
            let rem = self.payload.len().saturating_sub(self.pos);
            if rem != 0 {
                return Err(WireError::TrailingBytes(rem));
            }
            Ok(())
        }
    }

    pub(super) fn encode_get_registry(display_id: u32, new_registry_id: u32) -> Result<Vec<u8>, WireError> {
        let mut w = WireWriter::begin(display_id, super::WL_DISPLAY_GET_REGISTRY as u16);
        w.write_u32(new_registry_id);
        w.finish()
    }

    pub(super) fn encode_registry_bind(
        registry_id: u32,
        name: u32,
        interface: &str,
        version: u32,
        new_id: u32,
    ) -> Result<Vec<u8>, WireError> {
        let mut w = WireWriter::begin(registry_id, super::WL_REGISTRY_BIND as u16);
        w.write_u32(name);
        w.write_string(interface)?;
        w.write_u32(version);
        w.write_u32(new_id);
        w.finish()
    }

    pub(super) fn encode_surface_attach(
        surface_id: u32,
        buffer_id: u32,
        x: i32,
        y: i32,
    ) -> Result<Vec<u8>, WireError> {
        let mut w = WireWriter::begin(surface_id, super::WL_SURFACE_ATTACH as u16);
        w.write_u32(buffer_id);
        w.write_i32(x);
        w.write_i32(y);
        w.finish()
    }

    pub(super) fn encode_toplevel_set_title(
        toplevel_id: u32,
        title: &str,
    ) -> Result<Vec<u8>, WireError> {
        let mut w = WireWriter::begin(toplevel_id, super::XDG_TOPLEVEL_SET_TITLE as u16);
        w.write_string(title)?;
        w.finish()
    }

    pub(super) fn decode_registry_global(packet: &[u8]) -> Result<(u32, &str, u32), WireError> {
        let mut r = WireReader::open(packet)?;
        let header = r.header();
        if header.opcode != 0 {
            return Err(WireError::InvalidHeaderSize(header.opcode));
        }
        let name = r.read_u32()?;
        let interface = r.read_string()?;
        let version = r.read_u32()?;
        r.finish()?;
        Ok((name, interface, version))
    }
}

#[cfg(test)]
mod tests {
    use super::wire::*;
    use super::*;

    struct MockApp {
        url_focused: bool,
        typed: String,
        backspaces: usize,
        submits: usize,
        navs: Vec<NavKey>,
    }

    impl MockApp {
        fn new(url_focused: bool) -> Self {
            Self {
                url_focused,
                typed: String::new(),
                backspaces: 0,
                submits: 0,
                navs: Vec::new(),
            }
        }
    }

    impl PlatformApp for MockApp {
        fn viewport_size(&self) -> (usize, usize) {
            (800, 600)
        }
        fn url_focused(&self) -> bool {
            self.url_focused
        }
        fn needs_repaint(&self) -> bool {
            false
        }
        fn set_needs_repaint(&mut self, _dirty: bool) {}
        fn active_title(&self) -> &str {
            "Test"
        }
        fn resize(&mut self, _width: usize, _height: usize) {}
        fn scroll(&mut self, _delta_y: f32) {}
        fn handle_mouse_move(&mut self, _x: f32, _y: f32) {}
        fn handle_mouse_down(&mut self, _x: f32, _y: f32) {}
        fn handle_mouse_up(&mut self, _x: f32, _y: f32) {}
        fn handle_text_input(&mut self, ch: char) {
            self.typed.push(ch);
        }
        fn handle_backspace(&mut self) {
            self.backspaces += 1;
        }
        fn handle_submit_url(&mut self) {
            self.submits += 1;
        }
        fn handle_key_navigation(&mut self, key: NavKey) {
            self.navs.push(key);
        }
        fn render_into(&mut self, _canvas: &mut Canvas) {}
    }

    #[test]
    fn wire_header_roundtrip_and_validation() {
        let header = WireHeader {
            object_id: 42,
            opcode: 3,
            size: 24,
        };
        let encoded = header.encode();
        assert_eq!(WireHeader::decode(&encoded), Ok(header));
        assert_eq!(WireHeader::decode(&encoded[..7]), Err(WireError::Truncated));

        let too_small = WireHeader {
            object_id: 1,
            opcode: 0,
            size: 4,
        }
        .encode();
        assert_eq!(
            WireHeader::decode(&too_small),
            Err(WireError::InvalidHeaderSize(4))
        );

        let unaligned = WireHeader {
            object_id: 1,
            opcode: 0,
            size: 10,
        }
        .encode();
        assert_eq!(
            WireHeader::decode(&unaligned),
            Err(WireError::UnalignedHeaderSize(10))
        );
    }

    #[test]
    fn wire_strings_and_arrays_pad_to_word_boundaries() {
        for sample in ["", "a", "ab", "abc", "abcd", "wl_shm", "Привет - Droplet"] {
            let mut w = WireWriter::begin(7, 2);
            w.write_string(sample).unwrap();
            let packet = w.finish().unwrap();
            assert_eq!(packet.len() % 4, 0);
            let mut r = WireReader::open(&packet).unwrap();
            assert_eq!(r.read_string(), Ok(sample));
            assert_eq!(r.finish(), Ok(()));
        }

        let mut w = WireWriter::begin(9, 1);
        assert_eq!(w.write_string("bad\0str"), Err(WireError::EmbeddedNul));

        let mut w_arr = WireWriter::begin(11, 0);
        w_arr.write_array(&[1, 2, 3, 4, 5]).unwrap();
        w_arr.write_fixed(12.25);
        w_arr.write_i32(-99);
        let arr_pkt = w_arr.finish().unwrap();
        assert_eq!(arr_pkt.len() % 4, 0);
        let mut r = WireReader::open(&arr_pkt).unwrap();
        assert_eq!(r.read_array(), Ok(&[1u8, 2, 3, 4, 5][..]));
        assert!((r.read_fixed().unwrap() - 12.25).abs() < 0.01);
        assert_eq!(r.read_i32(), Ok(-99));
        assert_eq!(r.finish(), Ok(()));
    }

    #[test]
    fn wire_protocol_messages_encode_and_decode_deterministically() {
        let reg_pkt = encode_get_registry(1, 2).unwrap();
        assert_eq!(reg_pkt.len(), 12);
        let mut r = WireReader::open(&reg_pkt).unwrap();
        assert_eq!(r.header().object_id, 1);
        assert_eq!(r.header().opcode, WL_DISPLAY_GET_REGISTRY as u16);
        assert_eq!(r.read_u32(), Ok(2));
        assert_eq!(r.finish(), Ok(()));

        let bind_pkt = encode_registry_bind(2, 10, "xdg_wm_base", 1, 5).unwrap();
        let mut r = WireReader::open(&bind_pkt).unwrap();
        assert_eq!(r.header().opcode, WL_REGISTRY_BIND as u16);
        assert_eq!(r.read_u32(), Ok(10));
        assert_eq!(r.read_string(), Ok("xdg_wm_base"));
        assert_eq!(r.read_u32(), Ok(1));
        assert_eq!(r.read_u32(), Ok(5));
        assert_eq!(r.finish(), Ok(()));

        let attach_pkt = encode_surface_attach(3, 8, 0, -4).unwrap();
        let mut r = WireReader::open(&attach_pkt).unwrap();
        assert_eq!(r.header().opcode, WL_SURFACE_ATTACH as u16);
        assert_eq!(r.read_u32(), Ok(8));
        assert_eq!(r.read_i32(), Ok(0));
        assert_eq!(r.read_i32(), Ok(-4));
        assert_eq!(r.finish(), Ok(()));

        let title_pkt = encode_toplevel_set_title(6, "Droplet Browser").unwrap();
        let mut r = WireReader::open(&title_pkt).unwrap();
        assert_eq!(r.header().opcode, XDG_TOPLEVEL_SET_TITLE as u16);
        assert_eq!(r.read_string(), Ok("Droplet Browser"));
        assert_eq!(r.finish(), Ok(()));

        let mut global_w = WireWriter::begin(2, 0);
        global_w.write_u32(4);
        global_w.write_string("wl_compositor").unwrap();
        global_w.write_u32(5);
        let global_pkt = global_w.finish().unwrap();
        assert_eq!(
            decode_registry_global(&global_pkt),
            Ok((4, "wl_compositor", 5))
        );
        assert_eq!(
            decode_registry_global(&global_pkt[..global_pkt.len() - 4]),
            Err(WireError::Truncated)
        );
    }

    #[test]
    fn protocol_interface_descriptors_match_wayland_signatures() {
        unsafe {
            assert_eq!(
                CStr::from_ptr(WL_REGISTRY_INTERFACE.name).to_str(),
                Ok("wl_registry")
            );
            assert_eq!(WL_REGISTRY_INTERFACE.method_count, 1);
            assert_eq!(WL_REGISTRY_INTERFACE.event_count, 2);
            assert_eq!(
                CStr::from_ptr(WL_REGISTRY_REQUESTS[0].signature).to_str(),
                Ok("usun")
            );

            assert_eq!(
                CStr::from_ptr(WL_COMPOSITOR_INTERFACE.name).to_str(),
                Ok("wl_compositor")
            );
            assert_eq!(WL_COMPOSITOR_INTERFACE.method_count, 2);

            assert_eq!(CStr::from_ptr(WL_SHM_INTERFACE.name).to_str(), Ok("wl_shm"));
            assert_eq!(
                CStr::from_ptr(WL_SHM_REQUESTS[0].signature).to_str(),
                Ok("nhi")
            );
            assert_eq!(
                CStr::from_ptr(WL_SHM_POOL_REQUESTS[0].signature).to_str(),
                Ok("niiiiu")
            );
            assert_eq!(
                CStr::from_ptr(WL_BUFFER_EVENTS[0].name).to_str(),
                Ok("release")
            );

            assert_eq!(
                CStr::from_ptr(WL_SURFACE_REQUESTS[WL_SURFACE_ATTACH as usize].name).to_str(),
                Ok("attach")
            );
            assert_eq!(
                CStr::from_ptr(WL_SURFACE_REQUESTS[WL_SURFACE_DAMAGE as usize].name).to_str(),
                Ok("damage")
            );
            assert_eq!(
                CStr::from_ptr(WL_SURFACE_REQUESTS[WL_SURFACE_COMMIT as usize].name).to_str(),
                Ok("commit")
            );
            assert_eq!(
                CStr::from_ptr(WL_SURFACE_REQUESTS[WL_SURFACE_DAMAGE_BUFFER as usize].name)
                    .to_str(),
                Ok("damage_buffer")
            );
            assert_eq!(
                CStr::from_ptr(WL_SURFACE_REQUESTS[WL_SURFACE_DAMAGE_BUFFER as usize].signature)
                    .to_str(),
                Ok("iiii")
            );
            assert_eq!(WL_SURFACE_INTERFACE.version, 4);

            assert_eq!(WL_SEAT_INTERFACE.method_count, 3);
            assert_eq!(WL_KEYBOARD_INTERFACE.event_count, 5);
            assert_eq!(WL_POINTER_INTERFACE.event_count, 5);

            assert_eq!(
                CStr::from_ptr(XDG_WM_BASE_INTERFACE.name).to_str(),
                Ok("xdg_wm_base")
            );
            assert_eq!(
                CStr::from_ptr(XDG_WM_BASE_REQUESTS[XDG_WM_BASE_GET_XDG_SURFACE as usize].signature)
                    .to_str(),
                Ok("no")
            );
            assert_eq!(
                CStr::from_ptr(XDG_WM_BASE_REQUESTS[XDG_WM_BASE_PONG as usize].signature).to_str(),
                Ok("u")
            );
            assert_eq!(
                CStr::from_ptr(XDG_SURFACE_REQUESTS[XDG_SURFACE_ACK_CONFIGURE as usize].name)
                    .to_str(),
                Ok("ack_configure")
            );
            assert_eq!(
                CStr::from_ptr(XDG_TOPLEVEL_REQUESTS[XDG_TOPLEVEL_SET_TITLE as usize].name)
                    .to_str(),
                Ok("set_title")
            );
            assert_eq!(
                CStr::from_ptr(XDG_TOPLEVEL_REQUESTS[XDG_TOPLEVEL_SET_APP_ID as usize].name)
                    .to_str(),
                Ok("set_app_id")
            );
            assert_eq!(XDG_TOPLEVEL_INTERFACE.event_count, 2);
        }
    }

    #[test]
    fn registry_state_tracks_required_globals_versions_and_duplicates() {
        let mut reg = RegistryState::default();
        assert!(!reg.complete());
        assert_eq!(
            reg.missing(),
            vec!["wl_compositor", "wl_shm", "wl_seat", "xdg_wm_base"]
        );

        // Unknown global and zero version are ignored.
        assert_eq!(reg.accept("wl_output", 4), None);
        assert_eq!(reg.accept("wl_compositor", 0), None);
        assert!(!reg.complete());

        // Compositor version is clamped to MAX_COMPOSITOR_VERSION (4); other globals to 1.
        assert_eq!(
            reg.accept("wl_compositor", 6),
            Some(GlobalBinding {
                kind: GlobalKind::Compositor,
                name: 0,
                version: 4,
            })
        );
        // Duplicate advertisement of an already-bound global is ignored.
        assert_eq!(reg.accept("wl_compositor", 4), None);

        assert_eq!(
            reg.accept("wl_shm", 3),
            Some(GlobalBinding {
                kind: GlobalKind::Shm,
                name: 0,
                version: 1,
            })
        );
        assert!(reg.accept("wl_seat", 8).is_some());
        assert_eq!(reg.missing(), vec!["xdg_wm_base"]);
        assert!(!reg.complete());

        assert!(reg.accept("xdg_wm_base", 5).is_some());
        assert!(reg.complete());
        assert!(reg.missing().is_empty());
    }

    #[test]
    fn key_mapping_handles_navigation_shortcuts_and_text_input() {
        let none = KeyModifiers::default();
        let ctrl = KeyModifiers {
            ctrl: true,
            ..none
        };
        let ctrl_shift = KeyModifiers {
            ctrl: true,
            shift: true,
            ..none
        };
        let alt = KeyModifiers {
            alt: true,
            ..none
        };
        let shift = KeyModifiers {
            shift: true,
            ..none
        };
        let caps = KeyModifiers {
            caps: true,
            ..none
        };
        let shift_caps = KeyModifiers {
            shift: true,
            caps: true,
            ..none
        };

        assert_eq!(map_key(KEY_L, ctrl, false), KeyAction::Nav(NavKey::FocusUrl));
        assert_eq!(map_key(KEY_T, ctrl, false), KeyAction::Nav(NavKey::NewTab));
        assert_eq!(map_key(KEY_W, ctrl, false), KeyAction::Nav(NavKey::CloseTab));
        assert_eq!(map_key(KEY_R, ctrl, false), KeyAction::Nav(NavKey::Reload));
        assert_eq!(map_key(KEY_TAB, ctrl, false), KeyAction::Nav(NavKey::NextTab));
        assert_eq!(
            map_key(KEY_TAB, ctrl_shift, false),
            KeyAction::Nav(NavKey::PrevTab)
        );
        assert_eq!(map_key(KEY_LEFT, alt, false), KeyAction::Nav(NavKey::Back));
        assert_eq!(
            map_key(KEY_RIGHT, alt, false),
            KeyAction::Nav(NavKey::Forward)
        );
        assert_eq!(map_key(KEY_HOME, alt, false), KeyAction::Nav(NavKey::GoHome));
        assert_eq!(map_key(KEY_F5, none, false), KeyAction::Nav(NavKey::Reload));

        assert_eq!(map_key(KEY_ENTER, none, true), KeyAction::Submit);
        assert_eq!(map_key(KEY_KPENTER, none, true), KeyAction::Submit);
        assert_eq!(map_key(KEY_ENTER, none, false), KeyAction::None);
        assert_eq!(map_key(KEY_BACKSPACE, none, true), KeyAction::Backspace);
        assert_eq!(map_key(KEY_ESC, none, true), KeyAction::Nav(NavKey::Escape));

        assert_eq!(map_key(KEY_UP, none, false), KeyAction::Nav(NavKey::Up));
        assert_eq!(map_key(KEY_DOWN, none, false), KeyAction::Nav(NavKey::Down));
        assert_eq!(
            map_key(KEY_PAGEUP, none, false),
            KeyAction::Nav(NavKey::PageUp)
        );
        assert_eq!(
            map_key(KEY_PAGEDOWN, none, false),
            KeyAction::Nav(NavKey::PageDown)
        );
        assert_eq!(map_key(KEY_HOME, none, false), KeyAction::Nav(NavKey::Home));
        assert_eq!(map_key(KEY_END, none, false), KeyAction::Nav(NavKey::End));
        assert_eq!(
            map_key(KEY_SPACE, none, false),
            KeyAction::Nav(NavKey::PageDown)
        );
        assert_eq!(map_key(KEY_SPACE, none, true), KeyAction::Text(' '));

        // Letter casing and punctuation.
        assert_eq!(map_key(30, none, true), KeyAction::Text('a'));
        assert_eq!(map_key(30, shift, true), KeyAction::Text('A'));
        assert_eq!(map_key(30, caps, true), KeyAction::Text('A'));
        assert_eq!(map_key(30, shift_caps, true), KeyAction::Text('a'));
        assert_eq!(map_key(53, none, true), KeyAction::Text('/'));
        assert_eq!(map_key(53, shift, true), KeyAction::Text('?'));
        assert_eq!(map_key(30, ctrl, true), KeyAction::None);
        assert_eq!(map_key(999, none, true), KeyAction::None);

        let parsed = KeyModifiers::from_wayland((1 << 0) | (1 << 2), 1 << 1);
        assert_eq!(
            parsed,
            KeyModifiers {
                shift: true,
                ctrl: true,
                alt: false,
                caps: true,
            }
        );

        let mut app = MockApp::new(true);
        apply_key_action(&mut app, KeyAction::Text('h'));
        apply_key_action(&mut app, KeyAction::Text('i'));
        apply_key_action(&mut app, KeyAction::Backspace);
        apply_key_action(&mut app, KeyAction::Submit);
        apply_key_action(&mut app, KeyAction::Nav(NavKey::Reload));
        apply_key_action(&mut app, KeyAction::None);
        assert_eq!(app.typed, "hi");
        assert_eq!(app.backspaces, 1);
        assert_eq!(app.submits, 1);
        assert_eq!(app.navs, vec![NavKey::Reload]);
    }

    #[test]
    fn pointer_fixed_and_scroll_helpers_clamp_and_reject_non_finite() {
        assert!((fixed_to_f32(256) - 1.0).abs() < f32::EPSILON);
        assert!((fixed_to_f32(-384) - (-1.5)).abs() < f32::EPSILON);
        assert!((axis_scroll_delta(15.0) - 45.0).abs() < f32::EPSILON);
        assert!((axis_scroll_delta(500.0) - MAX_AXIS_SCROLL).abs() < f32::EPSILON);
        assert!((axis_scroll_delta(-500.0) - (-MAX_AXIS_SCROLL)).abs() < f32::EPSILON);
        assert_eq!(axis_scroll_delta(f32::NAN), 0.0);
        assert_eq!(axis_scroll_delta(f32::INFINITY), 0.0);
    }

    #[test]
    fn buffer_pool_state_alternates_and_recycles_on_release() {
        let mut pool = BufferPoolState::new();
        assert!(pool.is_idle());

        assert_eq!(pool.acquire(), Some(0));
        assert!(!pool.is_idle());
        assert_eq!(pool.acquire(), Some(1));
        assert_eq!(pool.acquire(), None);

        // Out-of-bounds release is a safe no-op.
        pool.release(2);
        assert_eq!(pool.acquire(), None);

        // Simulate compositor invoking `wl_buffer.release` via user_data pointer.
        let ptr0 = pool.release_ptr(0);
        unsafe {
            buffer_release(ptr0, ptr::null_mut());
        }
        assert_eq!(pool.acquire(), Some(0));
        assert_eq!(pool.acquire(), None);

        pool.release(0);
        pool.release(1);
        assert!(pool.is_idle());
    }

    #[test]
    fn shm_format_selection_and_pixel_packing_support_xrgb_and_argb() {
        assert_eq!(select_shm_format(true, true), Some(ShmFormat::Xrgb8888));
        assert_eq!(select_shm_format(false, true), Some(ShmFormat::Argb8888));
        assert_eq!(select_shm_format(false, false), None);

        let mut xrgb_buf = [0u32; 4];
        Canvas::with_external_slice(2, 2, &mut xrgb_buf, Color::rgb(0x11, 0x22, 0x33), |_| {});
        finalize_shm_pixels(&mut xrgb_buf, ShmFormat::Xrgb8888);
        assert_eq!(xrgb_buf, [0x0011_2233_u32; 4]);

        let mut argb_buf = [0u32; 4];
        Canvas::with_external_slice(2, 2, &mut argb_buf, Color::rgb(0xAA, 0xBB, 0xCC), |_| {});
        finalize_shm_pixels(&mut argb_buf, ShmFormat::Argb8888);
        assert_eq!(argb_buf, [0xFFAA_BBCC_u32; 4]);
    }

    #[test]
    fn run_error_distinguishes_init_and_runtime_variants() {
        let init = RunError::Init("no compositor".into());
        assert!(matches!(init, RunError::Init(ref msg) if msg == "no compositor"));
        let rt = RunError::Runtime("disconnected".into());
        assert!(matches!(rt, RunError::Runtime(ref msg) if msg == "disconnected"));
    }

    #[test]
    fn wayland_damage_coords_clamps_and_rejects_empty() {
        assert_eq!(wayland_damage_coords(0, 0, 0, 10, 800, 600), None);
        assert_eq!(wayland_damage_coords(800, 0, 10, 10, 800, 600), None);
        assert_eq!(wayland_damage_coords(0, 600, 10, 10, 800, 600), None);
        assert_eq!(
            wayland_damage_coords(750, 580, 100, 100, 800, 600),
            Some((750, 580, 50, 20))
        );
        assert_eq!(
            wayland_damage_coords(16, 32, 200, 100, 800, 600),
            Some((16, 32, 200, 100))
        );
    }

    #[test]
    fn wayland_damage_rect_handles_partial_full_and_none() {
        assert_eq!(
            wayland_damage_rect(Some(DamageRect::new(20, 40, 900, 700)), false, 800, 600),
            Some((20, 40, 780, 560))
        );
        assert_eq!(
            wayland_damage_rect(Some(DamageRect::new(20, 40, 100, 100)), true, 800, 600),
            Some((0, 0, 800, 600))
        );
        assert_eq!(
            wayland_damage_rect(Some(DamageRect::new(0, 0, 0, 0)), false, 800, 600),
            None
        );
        assert_eq!(
            wayland_damage_rect(None, false, 800, 600),
            Some((0, 0, 800, 600))
        );
    }
}
