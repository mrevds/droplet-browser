//! Linux X11 graphical windowing backend via zero-dependency dynamic `dlopen`.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use droplet_gfx::{Canvas, Color, DamageRect};

use crate::linux_wake::{wait_readable, WakePipe};
use crate::{format_window_title, CaretMove, EmbeddedView, NavKey, PlatformApp};

const RTLD_LAZY: c_int = 1;
const Z_PIXMAP: c_int = 2;
const LSB_FIRST: c_int = 0;

// SysV shared memory constants (Linux)
const IPC_PRIVATE: c_int = 0;
const IPC_CREAT: c_int = 0o1000;
const IPC_RMID: c_int = 0;
const SHM_PERMS: c_int = 0o600;
const MAX_X11_SHM_BYTES: usize = 256 * 1024 * 1024;

// Event masks
const KEY_PRESS_MASK: c_long = 1 << 0;
const KEY_RELEASE_MASK: c_long = 1 << 1;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const EXPOSURE_MASK: c_long = 1 << 15;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

// Event types
const KEY_PRESS: c_int = 2;
const KEY_RELEASE: c_int = 3;
const BUTTON_PRESS: c_int = 4;
const BUTTON_RELEASE: c_int = 5;
const MOTION_NOTIFY: c_int = 6;
const EXPOSE: c_int = 12;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;

// Modifier masks
const SHIFT_MASK: c_uint = 1 << 0;
const CONTROL_MASK: c_uint = 1 << 2;
const MOD1_MASK: c_uint = 1 << 3;

// KeySyms
const XK_BACKSPACE: KeySym = 0xFF08;
const XK_TAB: KeySym = 0xFF09;
const XK_RETURN: KeySym = 0xFF0D;
const XK_ESCAPE: KeySym = 0xFF1B;
const XK_DELETE: KeySym = 0xFFFF;
const XK_ISO_LEFT_TAB: KeySym = 0xFE20;
const XK_HOME: KeySym = 0xFF50;
const XK_LEFT: KeySym = 0xFF51;
const XK_UP: KeySym = 0xFF52;
const XK_RIGHT: KeySym = 0xFF53;
const XK_DOWN: KeySym = 0xFF54;
const XK_PAGE_UP: KeySym = 0xFF55;
const XK_PAGE_DOWN: KeySym = 0xFF56;
const XK_END: KeySym = 0xFF57;
const XK_KP_ENTER: KeySym = 0xFF8D;
const XK_F3: KeySym = 0xFFC0;
const XK_F5: KeySym = 0xFFC2;
const XK_A_UPPER: KeySym = 0x0041;
const XK_A_LOWER: KeySym = 0x0061;
const XK_C_UPPER: KeySym = 0x0043;
const XK_C_LOWER: KeySym = 0x0063;
const XK_D_UPPER: KeySym = 0x0044;
const XK_D_LOWER: KeySym = 0x0064;
const XK_F_UPPER: KeySym = 0x0046;
const XK_F_LOWER: KeySym = 0x0066;
const XK_I_UPPER: KeySym = 0x0049;
const XK_I_LOWER: KeySym = 0x0069;
const XK_L_UPPER: KeySym = 0x004C;
const XK_L_LOWER: KeySym = 0x006C;
const XK_R_UPPER: KeySym = 0x0052;
const XK_R_LOWER: KeySym = 0x0072;
const XK_T_UPPER: KeySym = 0x0054;
const XK_T_LOWER: KeySym = 0x0074;
const XK_V_UPPER: KeySym = 0x0056;
const XK_V_LOWER: KeySym = 0x0076;
const XK_W_UPPER: KeySym = 0x0057;
const XK_W_LOWER: KeySym = 0x0077;
const XK_X_UPPER: KeySym = 0x0058;
const XK_X_LOWER: KeySym = 0x0078;
const XK_DIGIT_1: KeySym = 0x0031;
const XK_DIGIT_9: KeySym = 0x0039;
const XK_KP_1: KeySym = 0xFFB1;
const XK_KP_9: KeySym = 0xFFB9;

#[repr(C)]
struct Display {
    _private: [u8; 0],
}

#[repr(C)]
struct Visual {
    _private: [u8; 0],
}

type Window = c_ulong;
type Drawable = c_ulong;
type Atom = c_ulong;
type KeySym = c_ulong;
type Time = c_ulong;
type GC = *mut c_void;

#[repr(C)]
#[allow(dead_code)]
struct XImageFuncs {
    create_image: *mut c_void,
    destroy_image: *mut c_void,
    get_pixel: *mut c_void,
    put_pixel: *mut c_void,
    sub_image: *mut c_void,
    add_pixel: *mut c_void,
}

#[repr(C)]
#[allow(dead_code)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut u8,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    obdata: *mut c_void,
    f: XImageFuncs,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(dead_code)]
struct XKeyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: Time,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    keycode: c_uint,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(dead_code)]
struct XButtonEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: Time,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    button: c_uint,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(dead_code)]
struct XMotionEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: Time,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    is_hint: c_char,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(dead_code)]
struct XConfigureEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    event: Window,
    window: Window,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: Window,
    override_redirect: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(dead_code)]
struct XClientMessageEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    message_type: Atom,
    format: c_int,
    data_l: [c_long; 5],
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(dead_code)]
struct XExposeEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    count: c_int,
}

#[repr(C)]
#[allow(dead_code)]
union XEvent {
    type_: c_int,
    xkey: XKeyEvent,
    xbutton: XButtonEvent,
    xmotion: XMotionEvent,
    xexpose: XExposeEvent,
    xconfigure: XConfigureEvent,
    xclient: XClientMessageEvent,
    pad: [c_long; 24],
}

#[repr(C)]
#[allow(dead_code)]
struct XErrorEvent {
    type_: c_int,
    display: *mut Display,
    resourceid: c_ulong,
    serial: c_ulong,
    error_code: u8,
    request_code: u8,
    minor_code: u8,
}

type XErrorHandler = unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int;

#[repr(C)]
struct XShmSegmentInfo {
    shmseg: c_ulong,
    shmid: c_int,
    shmaddr: *mut c_char,
    read_only: c_int,
}

#[link(name = "c")]
extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;

    fn shmget(key: c_int, size: usize, shmflg: c_int) -> c_int;
    fn shmat(shmid: c_int, shmaddr: *const c_void, shmflg: c_int) -> *mut c_void;
    fn shmdt(shmaddr: *const c_void) -> c_int;
    fn shmctl(shmid: c_int, cmd: c_int, buf: *mut c_void) -> c_int;
}

static XSHM_ERROR_FLAG: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn shm_error_handler(_display: *mut Display, _err: *mut XErrorEvent) -> c_int {
    XSHM_ERROR_FLAG.store(true, Ordering::SeqCst);
    0
}

#[allow(non_snake_case, clippy::type_complexity)]
struct X11Api {
    handle: *mut c_void,
    XOpenDisplay: unsafe extern "C" fn(*const c_char) -> *mut Display,
    XCloseDisplay: unsafe extern "C" fn(*mut Display) -> c_int,
    XDefaultScreen: unsafe extern "C" fn(*mut Display) -> c_int,
    XRootWindow: unsafe extern "C" fn(*mut Display, c_int) -> Window,
    XBlackPixel: unsafe extern "C" fn(*mut Display, c_int) -> c_ulong,
    XWhitePixel: unsafe extern "C" fn(*mut Display, c_int) -> c_ulong,
    XCreateSimpleWindow: unsafe extern "C" fn(
        *mut Display,
        Window,
        c_int,
        c_int,
        c_uint,
        c_uint,
        c_uint,
        c_ulong,
        c_ulong,
    ) -> Window,
    XSelectInput: unsafe extern "C" fn(*mut Display, Window, c_long) -> c_int,
    XMapWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
    XStoreName: unsafe extern "C" fn(*mut Display, Window, *const c_char) -> c_int,
    XInternAtom: unsafe extern "C" fn(*mut Display, *const c_char, c_int) -> Atom,
    XSetWMProtocols: unsafe extern "C" fn(*mut Display, Window, *mut Atom, c_int) -> c_int,
    XNextEvent: unsafe extern "C" fn(*mut Display, *mut XEvent) -> c_int,
    XPending: unsafe extern "C" fn(*mut Display) -> c_int,
    XConnectionNumber: unsafe extern "C" fn(*mut Display) -> c_int,
    XDefaultVisual: unsafe extern "C" fn(*mut Display, c_int) -> *mut Visual,
    XDefaultDepth: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    XDefaultGC: unsafe extern "C" fn(*mut Display, c_int) -> GC,
    XCreateImage: unsafe extern "C" fn(
        *mut Display,
        *mut Visual,
        c_uint,
        c_int,
        c_int,
        *mut u8,
        c_uint,
        c_uint,
        c_int,
        c_int,
    ) -> *mut XImage,
    XPutImage: unsafe extern "C" fn(
        *mut Display,
        Drawable,
        GC,
        *mut XImage,
        c_int,
        c_int,
        c_int,
        c_int,
        c_uint,
        c_uint,
    ) -> c_int,
    XDestroyImage: unsafe extern "C" fn(*mut XImage) -> c_int,
    XFlush: unsafe extern "C" fn(*mut Display) -> c_int,
    XSync: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    XSetErrorHandler: unsafe extern "C" fn(Option<XErrorHandler>) -> Option<XErrorHandler>,
    XLookupString: unsafe extern "C" fn(
        *mut XKeyEvent,
        *mut c_char,
        c_int,
        *mut KeySym,
        *mut c_void,
    ) -> c_int,
    XSendEvent: unsafe extern "C" fn(*mut Display, Window, c_int, c_long, *mut XEvent) -> c_int,
    XMoveResizeWindow:
        unsafe extern "C" fn(*mut Display, Window, c_int, c_int, c_uint, c_uint) -> c_int,
    XMapRaised: unsafe extern "C" fn(*mut Display, Window) -> c_int,
    XUnmapWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
}

impl Drop for X11Api {
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
            "Failed to resolve X11 symbol '{}'",
            name.to_string_lossy()
        ));
    }
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
    Ok(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
}

impl X11Api {
    fn load() -> Result<Self, String> {
        let handle = unsafe {
            let h = dlopen(c"libX11.so.6".as_ptr(), RTLD_LAZY);
            if !h.is_null() {
                h
            } else {
                dlopen(c"libX11.so".as_ptr(), RTLD_LAZY)
            }
        };
        if handle.is_null() {
            return Err("Could not load libX11.so.6 or libX11.so".into());
        }

        let res = unsafe {
            Ok(Self {
                handle,
                XOpenDisplay: load_sym(handle, c"XOpenDisplay")?,
                XCloseDisplay: load_sym(handle, c"XCloseDisplay")?,
                XDefaultScreen: load_sym(handle, c"XDefaultScreen")?,
                XRootWindow: load_sym(handle, c"XRootWindow")?,
                XBlackPixel: load_sym(handle, c"XBlackPixel")?,
                XWhitePixel: load_sym(handle, c"XWhitePixel")?,
                XCreateSimpleWindow: load_sym(handle, c"XCreateSimpleWindow")?,
                XSelectInput: load_sym(handle, c"XSelectInput")?,
                XMapWindow: load_sym(handle, c"XMapWindow")?,
                XStoreName: load_sym(handle, c"XStoreName")?,
                XInternAtom: load_sym(handle, c"XInternAtom")?,
                XSetWMProtocols: load_sym(handle, c"XSetWMProtocols")?,
                XNextEvent: load_sym(handle, c"XNextEvent")?,
                XPending: load_sym(handle, c"XPending")?,
                XConnectionNumber: load_sym(handle, c"XConnectionNumber")?,
                XDefaultVisual: load_sym(handle, c"XDefaultVisual")?,
                XDefaultDepth: load_sym(handle, c"XDefaultDepth")?,
                XDefaultGC: load_sym(handle, c"XDefaultGC")?,
                XCreateImage: load_sym(handle, c"XCreateImage")?,
                XPutImage: load_sym(handle, c"XPutImage")?,
                XDestroyImage: load_sym(handle, c"XDestroyImage")?,
                XFlush: load_sym(handle, c"XFlush")?,
                XSync: load_sym(handle, c"XSync")?,
                XSetErrorHandler: load_sym(handle, c"XSetErrorHandler")?,
                XLookupString: load_sym(handle, c"XLookupString")?,
                XSendEvent: load_sym(handle, c"XSendEvent")?,
                XMoveResizeWindow: load_sym(handle, c"XMoveResizeWindow")?,
                XMapRaised: load_sym(handle, c"XMapRaised")?,
                XUnmapWindow: load_sym(handle, c"XUnmapWindow")?,
            })
        };
        if res.is_err() {
            unsafe {
                dlclose(handle);
            }
        }
        res
    }
}

#[allow(non_snake_case, clippy::type_complexity)]
struct XExtShmApi {
    handle: *mut c_void,
    XShmQueryExtension: unsafe extern "C" fn(*mut Display) -> c_int,
    XShmCreateImage: unsafe extern "C" fn(
        *mut Display,
        *mut Visual,
        c_uint,
        c_int,
        *mut c_char,
        *mut XShmSegmentInfo,
        c_uint,
        c_uint,
    ) -> *mut XImage,
    XShmAttach: unsafe extern "C" fn(*mut Display, *mut XShmSegmentInfo) -> c_int,
    XShmDetach: unsafe extern "C" fn(*mut Display, *mut XShmSegmentInfo) -> c_int,
    XShmPutImage: unsafe extern "C" fn(
        *mut Display,
        Drawable,
        GC,
        *mut XImage,
        c_int,
        c_int,
        c_int,
        c_int,
        c_uint,
        c_uint,
        c_int,
    ) -> c_int,
}

impl Drop for XExtShmApi {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                dlclose(self.handle);
            }
        }
    }
}

impl XExtShmApi {
    fn load() -> Option<Self> {
        let handle = unsafe {
            let h = dlopen(c"libXext.so.6".as_ptr(), RTLD_LAZY);
            if !h.is_null() {
                h
            } else {
                dlopen(c"libXext.so".as_ptr(), RTLD_LAZY)
            }
        };
        if handle.is_null() {
            return None;
        }
        let res = unsafe {
            let query = load_sym(handle, c"XShmQueryExtension").ok();
            let create = load_sym(handle, c"XShmCreateImage").ok();
            let attach = load_sym(handle, c"XShmAttach").ok();
            let detach = load_sym(handle, c"XShmDetach").ok();
            let put = load_sym(handle, c"XShmPutImage").ok();
            match (query, create, attach, detach, put) {
                (Some(q), Some(c), Some(a), Some(d), Some(p)) => Some(Self {
                    handle,
                    XShmQueryExtension: q,
                    XShmCreateImage: c,
                    XShmAttach: a,
                    XShmDetach: d,
                    XShmPutImage: p,
                }),
                _ => None,
            }
        };
        if res.is_none() {
            unsafe {
                dlclose(handle);
            }
        }
        res
    }
}

fn shm_byte_size(width: usize, height: usize) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    let bytes = width.checked_mul(height)?.checked_mul(4)?;
    if bytes > MAX_X11_SHM_BYTES {
        return None;
    }
    Some(bytes)
}

fn shm_probe_supported(library_loaded: bool, query_extension_ok: bool, depth: c_int) -> bool {
    library_loaded && query_extension_ok && (depth == 24 || depth == 32)
}

fn clamp_damage_coords(
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    max_w: usize,
    max_h: usize,
) -> Option<(usize, usize, usize, usize)> {
    let clamped = DamageRect::new(x, y, w, h).clamp(max_w, max_h);
    if clamped.is_empty() {
        None
    } else {
        Some((clamped.x, clamped.y, clamped.width, clamped.height))
    }
}

fn resolve_present_rect(
    damage: Option<DamageRect>,
    expose_needed: bool,
    width: usize,
    height: usize,
) -> Option<(usize, usize, usize, usize)> {
    if expose_needed {
        return clamp_damage_coords(0, 0, width, height, width, height);
    }
    match damage {
        Some(rect) => clamp_damage_coords(rect.x, rect.y, rect.width, rect.height, width, height),
        None => clamp_damage_coords(0, 0, width, height, width, height),
    }
}

struct ShmSurface<'a> {
    x11: &'a X11Api,
    xext: &'a XExtShmApi,
    display: *mut Display,
    img: *mut XImage,
    /// Boxed: `XShmCreateImage` stores a pointer to this struct in `XImage::obdata` and
    /// `XShmPutImage` reads the segment id through it, so its address must never move.
    shminfo: Box<XShmSegmentInfo>,
    width: usize,
    height: usize,
    pixel_len: usize,
    attached: bool,
}

impl<'a> ShmSurface<'a> {
    fn new(
        x11: &'a X11Api,
        xext: &'a XExtShmApi,
        display: *mut Display,
        visual: *mut Visual,
        depth: c_int,
        width: usize,
        height: usize,
    ) -> Option<Self> {
        let byte_len = shm_byte_size(width, height)?;
        let pixel_len = width.checked_mul(height)?;
        let mut shminfo = Box::new(XShmSegmentInfo {
            shmseg: 0,
            shmid: -1,
            shmaddr: ptr::null_mut(),
            read_only: 0,
        });

        let img = unsafe {
            (xext.XShmCreateImage)(
                display,
                visual,
                depth as c_uint,
                Z_PIXMAP,
                ptr::null_mut(),
                &mut *shminfo,
                width as c_uint,
                height as c_uint,
            )
        };
        if img.is_null() {
            return None;
        }
        let valid_layout = unsafe {
            (*img).bits_per_pixel == 32 && (*img).bytes_per_line == (width * 4) as c_int
        };
        if !valid_layout {
            unsafe {
                (*img).data = ptr::null_mut();
                (x11.XDestroyImage)(img);
            }
            return None;
        }

        let shmid = unsafe { shmget(IPC_PRIVATE, byte_len, IPC_CREAT | SHM_PERMS) };
        if shmid < 0 {
            unsafe {
                (*img).data = ptr::null_mut();
                (x11.XDestroyImage)(img);
            }
            return None;
        }
        let shmaddr = unsafe { shmat(shmid, ptr::null(), 0) };
        if shmaddr == usize::MAX as *mut c_void || shmaddr.is_null() {
            unsafe {
                shmctl(shmid, IPC_RMID, ptr::null_mut());
                (*img).data = ptr::null_mut();
                (x11.XDestroyImage)(img);
            }
            return None;
        }

        shminfo.shmid = shmid;
        shminfo.shmaddr = shmaddr as *mut c_char;
        shminfo.read_only = 0;
        unsafe {
            (*img).byte_order = LSB_FIRST;
            (*img).data = shmaddr as *mut u8;
        }

        XSHM_ERROR_FLAG.store(false, Ordering::SeqCst);
        let prev_handler = unsafe { (x11.XSetErrorHandler)(Some(shm_error_handler)) };
        let attach_ok = unsafe { (xext.XShmAttach)(display, &mut *shminfo) };
        unsafe {
            (x11.XSync)(display, 0);
            (x11.XSetErrorHandler)(prev_handler);
            // Mark segment for removal right after attach so crashed processes never leak SysV SHM.
            shmctl(shmid, IPC_RMID, ptr::null_mut());
        }

        if attach_ok == 0 || XSHM_ERROR_FLAG.load(Ordering::SeqCst) {
            unsafe {
                shmdt(shmaddr);
                (*img).data = ptr::null_mut();
                (x11.XDestroyImage)(img);
            }
            return None;
        }

        Some(Self {
            x11,
            xext,
            display,
            img,
            shminfo,
            width,
            height,
            pixel_len,
            attached: true,
        })
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.shminfo.shmaddr as *mut u32, self.pixel_len) }
    }

    fn put_rect(&self, window: Window, gc: GC, x: usize, y: usize, w: usize, h: usize) {
        let Some((cx, cy, cw, ch)) = clamp_damage_coords(x, y, w, h, self.width, self.height)
        else {
            return;
        };
        unsafe {
            (self.xext.XShmPutImage)(
                self.display,
                window,
                gc,
                self.img,
                cx as c_int,
                cy as c_int,
                cx as c_int,
                cy as c_int,
                cw as c_uint,
                ch as c_uint,
                0,
            );
            (self.x11.XSync)(self.display, 0);
        }
    }
}

impl Drop for ShmSurface<'_> {
    fn drop(&mut self) {
        if self.attached {
            unsafe {
                (self.xext.XShmDetach)(self.display, &mut *self.shminfo);
                (self.x11.XSync)(self.display, 0);
            }
            self.attached = false;
        }
        if !self.img.is_null() {
            unsafe {
                (*self.img).data = ptr::null_mut();
                (self.x11.XDestroyImage)(self.img);
            }
            self.img = ptr::null_mut();
        }
        if !self.shminfo.shmaddr.is_null() {
            unsafe {
                shmdt(self.shminfo.shmaddr as *const c_void);
            }
            self.shminfo.shmaddr = ptr::null_mut();
        }
    }
}

struct DisplayGuard<'a> {
    x11: &'a X11Api,
    display: *mut Display,
}

impl<'a> Drop for DisplayGuard<'a> {
    fn drop(&mut self) {
        if !self.display.is_null() {
            unsafe {
                (self.x11.XCloseDisplay)(self.display);
            }
        }
    }
}

fn set_window_title(x11: &X11Api, display: *mut Display, window: Window, title: &str) {
    let label = format_window_title(title);
    if let Ok(c_title) = CString::new(label) {
        unsafe {
            (x11.XStoreName)(display, window, c_title.as_ptr());
        }
    }
}

struct X11BlitTarget<'a> {
    x11: &'a X11Api,
    display: *mut Display,
    window: Window,
    visual: *mut Visual,
    depth: c_int,
    gc: GC,
}

fn present_canvas_rect(
    target: &X11BlitTarget<'_>,
    canvas: &mut Canvas,
    rect: (usize, usize, usize, usize),
) {
    if canvas.width == 0 || canvas.height == 0 {
        return;
    }
    if canvas.pixels.len() < canvas.width.saturating_mul(canvas.height) {
        return;
    }
    let Some((cx, cy, cw, ch)) =
        clamp_damage_coords(rect.0, rect.1, rect.2, rect.3, canvas.width, canvas.height)
    else {
        return;
    };

    unsafe {
        let img = (target.x11.XCreateImage)(
            target.display,
            target.visual,
            target.depth as c_uint,
            Z_PIXMAP,
            0,
            canvas.pixels.as_mut_ptr() as *mut u8,
            canvas.width as c_uint,
            canvas.height as c_uint,
            32,
            (canvas.width * 4) as c_int,
        );
        if !img.is_null() {
            if (*img).bits_per_pixel == 32 {
                (*img).byte_order = LSB_FIRST;
                (target.x11.XPutImage)(
                    target.display,
                    target.window,
                    target.gc,
                    img,
                    cx as c_int,
                    cy as c_int,
                    cx as c_int,
                    cy as c_int,
                    cw as c_uint,
                    ch as c_uint,
                );
            }
            // Clear data pointer before XDestroyImage so Xlib does not free Rust's Vec<u32>.
            (*img).data = ptr::null_mut();
            (target.x11.XDestroyImage)(img);
        }
        (target.x11.XFlush)(target.display);
    }
}

/// Process-wide X error handler. Xlib's default handler exits the process; with foreign
/// (embedded) windows that may vanish at any moment, a `BadWindow` must not kill the browser.
unsafe extern "C" fn lenient_error_handler(_display: *mut Display, err: *mut XErrorEvent) -> c_int {
    if !err.is_null() {
        eprintln!(
            "droplet: ignored X error {} (request {}) on resource {:#x}",
            (*err).error_code,
            (*err).request_code,
            (*err).resourceid
        );
    }
    0
}

// XEmbed protocol messages (freedesktop XEmbed spec 0.5).
const XEMBED_EMBEDDED_NOTIFY: c_long = 0;
const XEMBED_WINDOW_ACTIVATE: c_long = 1;
const XEMBED_FOCUS_IN: c_long = 4;
const XEMBED_FOCUS_CURRENT: c_long = 0;
const XEMBED_VERSION: c_long = 0;

/// What the backend must do to go from the shown foreign view to the wanted one.
#[derive(Debug, PartialEq, Eq)]
struct EmbedActions {
    unmap: Option<u64>,
    place: Option<EmbeddedView>,
    /// A newly shown view gets the XEmbed handshake + focus.
    activate: bool,
}

fn embed_actions(shown: Option<EmbeddedView>, wanted: Option<EmbeddedView>) -> EmbedActions {
    match (shown, wanted) {
        (Some(a), Some(b)) if a.handle == b.handle => EmbedActions {
            unmap: None,
            place: (a != b).then_some(b),
            activate: false,
        },
        (shown, wanted) => EmbedActions {
            unmap: shown.map(|v| v.handle),
            place: wanted,
            activate: wanted.is_some(),
        },
    }
}

fn send_xembed(
    x11: &X11Api,
    display: *mut Display,
    target: Window,
    xembed: Atom,
    msg: [c_long; 4],
) {
    let mut event = XEvent { pad: [0; 24] };
    event.xclient = XClientMessageEvent {
        type_: CLIENT_MESSAGE,
        serial: 0,
        send_event: 1,
        display,
        window: target,
        message_type: xembed,
        format: 32,
        data_l: [0, msg[0], msg[1], msg[2], msg[3]],
    };
    unsafe { (x11.XSendEvent)(display, target, 0, 0, &mut event) };
}

fn sync_embedded(
    x11: &X11Api,
    display: *mut Display,
    parent: Window,
    xembed: Atom,
    shown: &mut Option<EmbeddedView>,
    wanted: Option<EmbeddedView>,
) {
    let actions = embed_actions(*shown, wanted);
    if actions.unmap.is_none() && actions.place.is_none() {
        return;
    }
    unsafe {
        if let Some(old) = actions.unmap {
            (x11.XUnmapWindow)(display, old as Window);
        }
        if let Some(v) = actions.place {
            let win = v.handle as Window;
            (x11.XMoveResizeWindow)(display, win, v.x, v.y, v.width.max(1), v.height.max(1));
            if actions.activate {
                (x11.XMapRaised)(display, win);
                if xembed != 0 {
                    let p = parent as c_long;
                    send_xembed(
                        x11,
                        display,
                        win,
                        xembed,
                        [XEMBED_EMBEDDED_NOTIFY, 0, p, XEMBED_VERSION],
                    );
                    send_xembed(x11, display, win, xembed, [XEMBED_WINDOW_ACTIVATE, 0, 0, 0]);
                    send_xembed(
                        x11,
                        display,
                        win,
                        xembed,
                        [XEMBED_FOCUS_IN, XEMBED_FOCUS_CURRENT, 0, 0],
                    );
                }
            }
        }
        (x11.XFlush)(display);
    }
    *shown = wanted;
}

/// Browser shortcuts that Droplet keeps while an embedded web view has the keyboard;
/// every other key goes to the view.
fn is_shell_shortcut(keysym: KeySym, ctrl: bool, alt: bool, shift: bool) -> bool {
    let lower = |k: KeySym| {
        if (0x41..=0x5a).contains(&k) {
            k + 0x20
        } else {
            k
        }
    };
    let k = lower(keysym);
    if ctrl && !alt {
        return matches!(
            k,
            0x6c /* l */ | 0x74 /* t */ | 0x77 /* w */ | 0x72 /* r */ | 0x64 /* d */
        ) || (shift && k == 0x69/* Ctrl+Shift+I */)
            || (XK_DIGIT_1..=XK_DIGIT_9).contains(&keysym)
            || (XK_KP_1..=XK_KP_9).contains(&keysym)
            || keysym == XK_TAB
            || keysym == XK_ISO_LEFT_TAB;
    }
    if alt && !ctrl {
        return matches!(keysym, XK_LEFT | XK_RIGHT | XK_HOME);
    }
    keysym == XK_F5
}

/// Re-sends a key event to the embedded view (XEmbed: the embedder keeps the X focus and
/// forwards keys). Returns `false` when the key is a browser shortcut and must be handled here.
fn forward_key(x11: &X11Api, display: *mut Display, view: Window, event: &XEvent) -> bool {
    let mut key_ev = unsafe { event.xkey };
    let mut keysym: KeySym = 0;
    let mut buf = [0u8; 8];
    unsafe {
        (x11.XLookupString)(
            &mut key_ev,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as c_int,
            &mut keysym,
            ptr::null_mut(),
        )
    };
    let ctrl = key_ev.state & CONTROL_MASK != 0;
    let alt = key_ev.state & MOD1_MASK != 0;
    let shift = key_ev.state & SHIFT_MASK != 0;
    if is_shell_shortcut(keysym, ctrl, alt, shift) {
        return false;
    }
    let mut fwd = XEvent { pad: [0; 24] };
    key_ev.window = view;
    key_ev.subwindow = 0;
    fwd.xkey = key_ev;
    let mask = if unsafe { event.type_ } == KEY_PRESS {
        KEY_PRESS_MASK
    } else {
        KEY_RELEASE_MASK
    };
    unsafe {
        (x11.XSendEvent)(display, view, 0, mask, &mut fwd);
        (x11.XFlush)(display);
    }
    true
}

fn handle_key_event<A: PlatformApp>(x11: &X11Api, app: &mut A, key_ev: &mut XKeyEvent) {
    let mut buf = [0u8; 32];
    let mut keysym: KeySym = 0;
    let len = unsafe {
        (x11.XLookupString)(
            key_ev as *mut XKeyEvent,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as c_int,
            &mut keysym,
            ptr::null_mut(),
        )
    };

    let shift = (key_ev.state & SHIFT_MASK) != 0;
    let ctrl = (key_ev.state & CONTROL_MASK) != 0;
    let alt = (key_ev.state & MOD1_MASK) != 0;

    if ctrl && !alt && (keysym == XK_L_LOWER || keysym == XK_L_UPPER) {
        app.handle_key_navigation(NavKey::FocusUrl);
        return;
    }
    if ctrl && !alt && (keysym == XK_T_LOWER || keysym == XK_T_UPPER) {
        app.handle_key_navigation(NavKey::NewTab);
        return;
    }
    if ctrl && !alt && (keysym == XK_W_LOWER || keysym == XK_W_UPPER) {
        app.handle_key_navigation(NavKey::CloseTab);
        return;
    }
    if ctrl && !alt && (keysym == XK_R_LOWER || keysym == XK_R_UPPER) {
        app.handle_key_navigation(NavKey::Reload);
        return;
    }
    if ctrl && !alt && shift && (keysym == XK_I_LOWER || keysym == XK_I_UPPER) {
        app.handle_key_navigation(NavKey::ToggleInspector);
        return;
    }
    if ctrl && !alt && (keysym == XK_D_LOWER || keysym == XK_D_UPPER) {
        app.handle_key_navigation(NavKey::ToggleBookmark);
        return;
    }
    if ctrl && !alt && (keysym == XK_F_LOWER || keysym == XK_F_UPPER) {
        app.handle_key_navigation(NavKey::Find);
        return;
    }
    if ctrl && !alt && (XK_DIGIT_1..=XK_DIGIT_9).contains(&keysym) {
        app.handle_key_navigation(NavKey::SwitchTab((keysym - XK_DIGIT_1) as usize));
        return;
    }
    if ctrl && !alt && (XK_KP_1..=XK_KP_9).contains(&keysym) {
        app.handle_key_navigation(NavKey::SwitchTab((keysym - XK_KP_1) as usize));
        return;
    }
    if ctrl && !alt && (keysym == XK_A_LOWER || keysym == XK_A_UPPER) {
        app.handle_select_all();
        return;
    }
    if ctrl && !alt && (keysym == XK_C_LOWER || keysym == XK_C_UPPER) {
        app.handle_copy();
        return;
    }
    if ctrl && !alt && (keysym == XK_X_LOWER || keysym == XK_X_UPPER) {
        app.handle_cut();
        return;
    }
    if ctrl && !alt && (keysym == XK_V_LOWER || keysym == XK_V_UPPER) {
        app.handle_paste();
        return;
    }
    if ctrl && !alt && keysym == XK_ISO_LEFT_TAB {
        app.handle_key_navigation(NavKey::PrevTab);
        return;
    }
    if ctrl && !alt && keysym == XK_TAB {
        if shift {
            app.handle_key_navigation(NavKey::PrevTab);
        } else {
            app.handle_key_navigation(NavKey::NextTab);
        }
        return;
    }
    if alt && !ctrl && keysym == XK_LEFT {
        app.handle_key_navigation(NavKey::Back);
        return;
    }
    if alt && !ctrl && keysym == XK_RIGHT {
        app.handle_key_navigation(NavKey::Forward);
        return;
    }
    if alt && !ctrl && keysym == XK_HOME {
        app.handle_key_navigation(NavKey::GoHome);
        return;
    }

    match keysym {
        XK_F3 => {
            if shift {
                app.handle_key_navigation(NavKey::FindPrev);
            } else {
                app.handle_key_navigation(NavKey::FindNext);
            }
        }
        XK_F5 => app.handle_key_navigation(NavKey::Reload),
        XK_RETURN | XK_KP_ENTER => app.handle_enter(shift),
        XK_BACKSPACE => app.handle_backspace(),
        XK_DELETE => app.handle_delete(),
        XK_ESCAPE => app.handle_key_navigation(NavKey::Escape),
        XK_TAB | XK_ISO_LEFT_TAB => {
            if !shift && app.url_focused() {
                app.handle_key_navigation(NavKey::AcceptCompletion);
            } else {
                app.handle_tab_focus(shift);
            }
        }
        XK_LEFT => app.handle_caret(CaretMove::Left, shift),
        XK_RIGHT => {
            if app.url_focused() {
                app.handle_key_navigation(NavKey::AcceptCompletion);
            } else {
                app.handle_caret(CaretMove::Right, shift);
            }
        }
        XK_UP => {
            if app.url_focused() {
                app.handle_key_navigation(NavKey::Up);
            } else {
                app.handle_caret(CaretMove::Up, shift);
            }
        }
        XK_DOWN => {
            if app.url_focused() {
                app.handle_key_navigation(NavKey::Down);
            } else {
                app.handle_caret(CaretMove::Down, shift);
            }
        }
        XK_PAGE_UP => app.handle_key_navigation(NavKey::PageUp),
        XK_PAGE_DOWN => app.handle_key_navigation(NavKey::PageDown),
        XK_HOME => app.handle_caret(CaretMove::Home, shift),
        XK_END => app.handle_caret(CaretMove::End, shift),
        _ => {
            if !ctrl && !alt && len > 0 {
                let slice = &buf[..(len as usize).min(buf.len())];
                let text = String::from_utf8_lossy(slice);
                for ch in text.chars() {
                    if !ch.is_control() {
                        app.handle_text_input(ch);
                    }
                }
            }
        }
    }
}

pub(crate) fn run<A: PlatformApp>(app: &mut A) -> Result<(), String> {
    let x11 = X11Api::load()?;
    let display = unsafe { (x11.XOpenDisplay)(ptr::null()) };
    if display.is_null() {
        return Err("Unable to open X11 display (check $DISPLAY)".into());
    }
    let _display_guard = DisplayGuard {
        x11: &x11,
        display,
    };
    unsafe { (x11.XSetErrorHandler)(Some(lenient_error_handler)) };

    let screen = unsafe { (x11.XDefaultScreen)(display) };
    let depth = unsafe { (x11.XDefaultDepth)(display, screen) };
    if depth != 24 && depth != 32 {
        return Err(format!(
            "Unsupported X11 display depth {depth} (expected 24 or 32 bit TrueColor)"
        ));
    }

    let visual = unsafe { (x11.XDefaultVisual)(display, screen) };
    let gc = unsafe { (x11.XDefaultGC)(display, screen) };
    let root = unsafe { (x11.XRootWindow)(display, screen) };
    let black = unsafe { (x11.XBlackPixel)(display, screen) };
    let white = unsafe { (x11.XWhitePixel)(display, screen) };

    let (init_w, init_h) = app.viewport_size();
    let win_w = init_w.max(1) as c_uint;
    let win_h = init_h.max(1) as c_uint;

    let window = unsafe {
        (x11.XCreateSimpleWindow)(display, root, 0, 0, win_w, win_h, 0, black, white)
    };
    if window == 0 {
        return Err("XCreateSimpleWindow failed".into());
    }

    let mut wm_delete =
        unsafe { (x11.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0) };
    if wm_delete != 0 {
        unsafe {
            (x11.XSetWMProtocols)(display, window, &mut wm_delete, 1);
        }
    }

    unsafe {
        (x11.XSelectInput)(
            display,
            window,
            EXPOSURE_MASK
                | KEY_PRESS_MASK
                | KEY_RELEASE_MASK
                | BUTTON_PRESS_MASK
                | BUTTON_RELEASE_MASK
                | POINTER_MOTION_MASK
                | STRUCTURE_NOTIFY_MASK,
        );
    }

    let mut last_title = app.active_title().to_string();
    set_window_title(&x11, display, window, &last_title);

    unsafe {
        (x11.XMapWindow)(display, window);
        (x11.XFlush)(display);
    }

    let xext = XExtShmApi::load();
    let shm_query_ok = xext
        .as_ref()
        .is_some_and(|api| unsafe { (api.XShmQueryExtension)(display) != 0 });
    let mut shm_enabled = shm_probe_supported(xext.is_some(), shm_query_ok, depth);
    let mut shm_surface = if shm_enabled {
        xext.as_ref().and_then(|api| {
            ShmSurface::new(
                &x11,
                api,
                display,
                visual,
                depth,
                init_w.max(1),
                init_h.max(1),
            )
        })
    } else {
        None
    };
    if shm_surface.is_none() {
        shm_enabled = false;
    }
    app.set_present_backend(if shm_surface.is_some() {
        "x11-shm"
    } else {
        "x11-putimage"
    });

    let blit_target = X11BlitTarget {
        x11: &x11,
        display,
        window,
        visual,
        depth,
        gc,
    };

    // Background work (page loads) wakes the loop through this self-pipe (DROP-110).
    // Installed before the first frame so a deferred start page loads in the background.
    // Foreign web views (DROP-170) are XEmbed clients of this window.
    app.set_embed_parent(window as u64);
    let xembed_atom = unsafe { (x11.XInternAtom)(display, c"_XEMBED".as_ptr(), 0) };
    let mut shown_embed: Option<EmbeddedView> = None;

    let wake = WakePipe::new().ok();
    if let Some(ref pipe) = wake {
        app.set_waker(pipe.waker());
    }
    let conn_fd = unsafe { (x11.XConnectionNumber)(display) };

    let mut fallback_canvas: Option<Canvas> = None;
    if let Some(ref mut shm) = shm_surface {
        Canvas::with_external_slice(
            init_w,
            init_h,
            shm.pixels_mut(),
            Color::rgb(255, 255, 255),
            |canvas| app.render_into(canvas),
        );
        let _ = app.take_damage();
        shm.put_rect(window, gc, 0, 0, init_w, init_h);
    } else {
        let fb = fallback_canvas
            .get_or_insert_with(|| Canvas::new(init_w, init_h, Color::rgb(255, 255, 255)));
        app.render_into(fb);
        let _ = app.take_damage();
        present_canvas_rect(&blit_target, fb, (0, 0, init_w, init_h));
    }
    app.set_needs_repaint(false);

    let mut event = XEvent { pad: [0; 24] };
    let mut running = true;

    while running {
        // Block until an X event arrives or another thread wakes us.
        let mut have_event = false;
        loop {
            // XPending also flushes queued requests before we sleep.
            if unsafe { (x11.XPending)(display) } > 0 {
                unsafe { (x11.XNextEvent)(display, &mut event) };
                have_event = true;
                break;
            }
            let Some(ref pipe) = wake else {
                unsafe { (x11.XNextEvent)(display, &mut event) };
                have_event = true;
                break;
            };
            let [_, woke] = wait_readable([conn_fd, pipe.read_fd()]);
            if woke && pipe.drain() {
                app.poll_background();
                break;
            }
        }

        let mut expose_needed = false;
        let mut pending_motion: Option<(f32, f32)> = None;
        let mut pending_resize: Option<(usize, usize)> = None;

        loop {
            if !have_event {
                break; // woken by background work only: skip straight to repaint
            }
            unsafe {
                match event.type_ {
                    EXPOSE => {
                        if event.xexpose.count == 0 {
                            expose_needed = true;
                        }
                    }
                    CONFIGURE_NOTIFY => {
                        let w = event.xconfigure.width.max(1) as usize;
                        let h = event.xconfigure.height.max(1) as usize;
                        pending_resize = Some((w, h));
                    }
                    MOTION_NOTIFY => {
                        pending_motion = Some((event.xmotion.x as f32, event.xmotion.y as f32));
                    }
                    BUTTON_PRESS => {
                        if let Some((w, h)) = pending_resize.take() {
                            app.resize(w, h);
                        }
                        if let Some((mx, my)) = pending_motion.take() {
                            app.handle_mouse_move(mx, my);
                        }
                        let btn = event.xbutton.button;
                        let bx = event.xbutton.x as f32;
                        let by = event.xbutton.y as f32;
                        match btn {
                            1 => app.handle_mouse_down(bx, by),
                            4 => app.scroll(-48.0),
                            5 => app.scroll(48.0),
                            8 => app.handle_key_navigation(NavKey::Back),
                            9 => app.handle_key_navigation(NavKey::Forward),
                            _ => {}
                        }
                    }
                    BUTTON_RELEASE => {
                        if let Some((w, h)) = pending_resize.take() {
                            app.resize(w, h);
                        }
                        if let Some((mx, my)) = pending_motion.take() {
                            app.handle_mouse_move(mx, my);
                        }
                        if event.xbutton.button == 1 {
                            let bx = event.xbutton.x as f32;
                            let by = event.xbutton.y as f32;
                            app.handle_mouse_up(bx, by);
                        }
                    }
                    KEY_PRESS => {
                        if let Some((w, h)) = pending_resize.take() {
                            app.resize(w, h);
                        }
                        let forwarded = match shown_embed {
                            Some(view) if !app.wants_text_input() => {
                                forward_key(&x11, display, view.handle as Window, &event)
                            }
                            _ => false,
                        };
                        if !forwarded {
                            let mut key_ev = event.xkey;
                            handle_key_event(&x11, app, &mut key_ev);
                        }
                    }
                    KEY_RELEASE => {
                        if let Some(view) = shown_embed {
                            if !app.wants_text_input() {
                                forward_key(&x11, display, view.handle as Window, &event);
                            }
                        }
                    }
                    CLIENT_MESSAGE
                        if wm_delete != 0 && (event.xclient.data_l[0] as Atom) == wm_delete =>
                    {
                        running = false;
                        break;
                    }
                    _ => {}
                }

                if (x11.XPending)(display) > 0 {
                    (x11.XNextEvent)(display, &mut event);
                } else {
                    break;
                }
            }
        }

        if !running {
            break;
        }

        if let Some((w, h)) = pending_resize {
            app.resize(w, h);
        }
        if let Some((mx, my)) = pending_motion {
            app.handle_mouse_move(mx, my);
        }

        sync_embedded(
            &x11,
            display,
            window,
            xembed_atom,
            &mut shown_embed,
            app.embedded_view(),
        );

        let current_title = app.active_title();
        if current_title != last_title {
            last_title = current_title.to_string();
            set_window_title(&x11, display, window, &last_title);
        }

        if app.needs_repaint() || expose_needed {
            let (vw, vh) = app.viewport_size();
            let vw = vw.max(1);
            let vh = vh.max(1);
            let mut shm_recreated = false;
            if shm_enabled
                && shm_surface
                    .as_ref()
                    .is_none_or(|s| s.width != vw || s.height != vh)
            {
                drop(shm_surface.take());
                shm_surface = xext.as_ref().and_then(|api| {
                    ShmSurface::new(&x11, api, display, visual, depth, vw, vh)
                });
                if shm_surface.is_some() {
                    shm_recreated = true;
                } else {
                    shm_enabled = false;
                    app.set_present_backend("x11-putimage");
                }
            }

            let needs_repaint = app.needs_repaint();
            let damage = if needs_repaint {
                app.take_damage()
            } else {
                None
            };
            let rect = if shm_recreated {
                clamp_damage_coords(0, 0, vw, vh, vw, vh)
            } else {
                resolve_present_rect(damage, expose_needed, vw, vh)
            };

            if let Some(ref mut shm) = shm_surface {
                if needs_repaint || shm_recreated {
                    Canvas::with_external_slice(
                        vw,
                        vh,
                        shm.pixels_mut(),
                        Color::rgb(255, 255, 255),
                        |canvas| app.render_into(canvas),
                    );
                }
                if let Some((rx, ry, rw, rh)) = rect {
                    shm.put_rect(window, gc, rx, ry, rw, rh);
                }
            } else {
                let fb = fallback_canvas
                    .get_or_insert_with(|| Canvas::new(vw, vh, Color::rgb(255, 255, 255)));
                let resized = fb.width != vw || fb.height != vh;
                if resized {
                    fb.resize(vw, vh, Color::rgb(255, 255, 255));
                }
                if needs_repaint || resized {
                    app.render_into(fb);
                }
                let fallback_rect = if resized {
                    clamp_damage_coords(0, 0, vw, vh, vw, vh)
                } else {
                    rect
                };
                if let Some((rx, ry, rw, rh)) = fallback_rect {
                    present_canvas_rect(&blit_target, fb, (rx, ry, rw, rh));
                }
            }
            if needs_repaint {
                app.set_needs_repaint(false);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    fn view(handle: u64, width: u32) -> EmbeddedView {
        EmbeddedView {
            handle,
            x: 0,
            y: 74,
            width,
            height: 500,
        }
    }

    #[test]
    fn embed_actions_show_move_switch_hide() {
        let none = EmbedActions {
            unmap: None,
            place: None,
            activate: false,
        };
        assert_eq!(embed_actions(None, None), none);
        assert_eq!(
            embed_actions(None, Some(view(7, 800))),
            EmbedActions {
                unmap: None,
                place: Some(view(7, 800)),
                activate: true
            }
        );
        assert_eq!(embed_actions(Some(view(7, 800)), Some(view(7, 800))), none);
        // Window resize: move only, no second XEmbed handshake.
        assert_eq!(
            embed_actions(Some(view(7, 800)), Some(view(7, 900))),
            EmbedActions {
                unmap: None,
                place: Some(view(7, 900)),
                activate: false
            }
        );
        // Tab switch to another web view.
        assert_eq!(
            embed_actions(Some(view(7, 800)), Some(view(9, 800))),
            EmbedActions {
                unmap: Some(7),
                place: Some(view(9, 800)),
                activate: true
            }
        );
        // Tab switch to a Droplet-rendered page.
        assert_eq!(
            embed_actions(Some(view(7, 800)), None),
            EmbedActions {
                unmap: Some(7),
                place: None,
                activate: false
            }
        );
    }

    #[test]
    fn shell_shortcuts_stay_with_droplet() {
        assert!(is_shell_shortcut(0x6c, true, false, false)); // Ctrl+L
        assert!(is_shell_shortcut(0x4c, true, false, true)); // Ctrl+Shift+L (uppercase keysym)
        assert!(is_shell_shortcut(0x74, true, false, false)); // Ctrl+T
        assert!(is_shell_shortcut(XK_DIGIT_1 + 2, true, false, false)); // Ctrl+3
        assert!(is_shell_shortcut(XK_TAB, true, false, false)); // Ctrl+Tab
        assert!(is_shell_shortcut(XK_LEFT, false, true, false)); // Alt+Left
        assert!(is_shell_shortcut(XK_F5, false, false, false));
        // Page keys go to the web view: copy/paste, typing, Cyrillic, arrows, Escape.
        assert!(!is_shell_shortcut(0x63, true, false, false)); // Ctrl+C
        assert!(!is_shell_shortcut(0x76, true, false, false)); // Ctrl+V
        assert!(!is_shell_shortcut(0x61, false, false, false)); // a
        assert!(!is_shell_shortcut(0x6d4, false, false, false)); // Cyrillic_ze
        assert!(!is_shell_shortcut(XK_LEFT, false, false, false));
        assert!(!is_shell_shortcut(XK_ESCAPE, false, false, false));
        assert!(!is_shell_shortcut(0x69, true, false, false)); // Ctrl+I without Shift
    }
    use super::*;

    #[test]
    fn shm_byte_size_validates_bounds_and_overflow() {
        assert_eq!(shm_byte_size(0, 100), None);
        assert_eq!(shm_byte_size(100, 0), None);
        assert_eq!(shm_byte_size(1024, 768), Some(1024 * 768 * 4));
        assert_eq!(shm_byte_size(usize::MAX, 2), None);
        // Exceeds MAX_X11_SHM_BYTES (256 MiB -> 64M pixels)
        assert_eq!(shm_byte_size(16_384, 16_384), None);
    }

    #[test]
    fn shm_probe_supported_requires_lib_extension_and_truecolor_depth() {
        assert!(shm_probe_supported(true, true, 24));
        assert!(shm_probe_supported(true, true, 32));
        assert!(!shm_probe_supported(false, true, 24));
        assert!(!shm_probe_supported(true, false, 24));
        assert!(!shm_probe_supported(true, true, 16));
    }

    #[test]
    fn clamp_damage_coords_clamps_partial_and_rejects_empty() {
        assert_eq!(clamp_damage_coords(0, 0, 0, 10, 800, 600), None);
        assert_eq!(clamp_damage_coords(800, 10, 20, 20, 800, 600), None);
        assert_eq!(clamp_damage_coords(10, 600, 20, 20, 800, 600), None);
        assert_eq!(
            clamp_damage_coords(750, 580, 100, 100, 800, 600),
            Some((750, 580, 50, 20))
        );
        assert_eq!(
            clamp_damage_coords(10, 20, 300, 200, 800, 600),
            Some((10, 20, 300, 200))
        );
    }

    #[test]
    fn shm_segment_info_layout_matches_c_abi() {
        // ShmSeg (c_ulong) + c_int (padded) + *mut c_char + c_int (padded)
        assert!(std::mem::size_of::<XShmSegmentInfo>() >= 16);
    }

    #[test]
    fn resolve_present_rect_respects_damage_and_expose_override() {
        // Partial damage clamps to viewport bounds
        assert_eq!(
            resolve_present_rect(Some(DamageRect::new(700, 500, 200, 200)), false, 800, 600),
            Some((700, 500, 100, 100))
        );
        // Empty damage with no expose skips blitting
        assert_eq!(
            resolve_present_rect(Some(DamageRect::new(0, 0, 0, 0)), false, 800, 600),
            None
        );
        // Expose overrides partial or empty damage with full viewport rect
        assert_eq!(
            resolve_present_rect(Some(DamageRect::new(10, 10, 50, 50)), true, 800, 600),
            Some((0, 0, 800, 600))
        );
        // None damage falls back to full viewport rect
        assert_eq!(
            resolve_present_rect(None, false, 800, 600),
            Some((0, 0, 800, 600))
        );
    }
}
