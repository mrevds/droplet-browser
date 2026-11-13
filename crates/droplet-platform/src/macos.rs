//! macOS native Cocoa/AppKit graphical windowing backend.
//!
//! The backend talks to Objective-C and CoreGraphics directly through the
//! runtime FFI, without `objc`, `cocoa`, or `windows-sys`-style crates. It
//! creates an `NSApplication`, an `NSWindow`, and a custom `NSView` subclass
//! whose `drawRect:` implementation blits the `Canvas` pixels through a
//! `CGBitmapContext`.
//!
//! Manual build requirements:
//! - build on a macOS host with the Xcode command-line tools installed (for the
//!   Cocoa, CoreGraphics, and Objective-C runtime frameworks and linker);
//! - run the binary on the macOS main thread (AppKit is not thread-safe);
//! - for normal activation and a menu bar, launch the binary from a signed or
//!   ad-hoc app bundle (a bare `cargo run` binary still opens a window, but
//!   macOS may keep it behind the terminal until clicked).
//!
//! This file is compiled only for `target_os = "macos"`; it is not part of the
//! Linux gate and is verified with `cargo check --target aarch64-apple-darwin`.

use std::ffi::{c_void, CStr, CString};
use std::mem;
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use droplet_gfx::{Canvas, Color};

use crate::{format_window_title, NavKey, PlatformApp};

// ---------------------------------------------------------------------------
// CoreGraphics / Foundation types
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

const fn rect(x: f64, y: f64, width: f64, height: f64) -> CGRect {
    CGRect {
        origin: CGPoint { x, y },
        size: CGSize { width, height },
    }
}

// NSView coordinate flag: our view uses a top-left origin, matching the Canvas.
const NS_VIEW_HEIGHT_SIZABLE: u64 = 16;
const NS_VIEW_WIDTH_SIZABLE: u64 = 2;

// NSWindow style mask: titled | closable | miniaturizable | resizable.
const NS_WINDOW_STYLE: u64 = 1 | 2 | 4 | 8;
const NS_BACKING_STORE_BUFFERED: u64 = 2;
const NS_APPLICATION_ACTIVATION_POLICY_REGULAR: i64 = 0;

// NSEventModifierFlags (device-independent subset).
const NS_EVENT_MODIFIER_SHIFT: u64 = 1 << 17;
const NS_EVENT_MODIFIER_CONTROL: u64 = 1 << 18;
const NS_EVENT_MODIFIER_OPTION: u64 = 1 << 19;
const NS_EVENT_MODIFIER_COMMAND: u64 = 1 << 20;

// Virtual key codes (ANSI layout).
const KEY_H: u16 = 4;
const KEY_R: u16 = 15;
const KEY_T: u16 = 17;
const KEY_W: u16 = 13;
const KEY_L: u16 = 37;
const KEY_LEFT_BRACKET: u16 = 33;
const KEY_RIGHT_BRACKET: u16 = 30;
const KEY_RETURN: u16 = 36;
const KEY_TAB: u16 = 48;
const KEY_SPACE: u16 = 49;
const KEY_DELETE: u16 = 51;
const KEY_ESCAPE: u16 = 53;
const KEY_KEYPAD_ENTER: u16 = 76;
const KEY_F5: u16 = 96;
const KEY_HOME: u16 = 115;
const KEY_PAGE_UP: u16 = 116;
const KEY_END: u16 = 119;
const KEY_PAGE_DOWN: u16 = 121;
const KEY_LEFT: u16 = 123;
const KEY_RIGHT: u16 = 124;
const KEY_DOWN: u16 = 125;
const KEY_UP: u16 = 126;

// kCGImageAlphaNoneSkipFirst (6) | kCGBitmapByteOrder32Little (2 << 12).
// This format consumes 0x00RRGGBB pixels stored as B,G,R,X bytes.
const CG_BITMAP_INFO_XRGB_LITTLE: u32 = 6 | (2 << 12);

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_allocateClassPair(
        superclass: *mut c_void,
        name: *const c_char,
        extra_bytes: usize,
    ) -> *mut c_void;
    fn objc_registerClassPair(cls: *mut c_void);
    fn class_addMethod(
        cls: *mut c_void,
        name: *mut c_void,
        imp: *const c_void,
        types: *const c_char,
    ) -> i8;
    fn objc_msgSend();
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(color_space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        color_space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGBitmapContextCreateImage(context: *mut c_void) -> *mut c_void;
    fn CGContextRelease(context: *mut c_void);
    fn CGImageRelease(image: *mut c_void);
    fn CGContextSaveGState(context: *mut c_void);
    fn CGContextRestoreGState(context: *mut c_void);
    fn CGContextTranslateCTM(context: *mut c_void, tx: f64, ty: f64);
    fn CGContextScaleCTM(context: *mut c_void, sx: f64, sy: f64);
    fn CGContextClipToRect(context: *mut c_void, rect: CGRect);
    fn CGContextSetInterpolationQuality(context: *mut c_void, quality: i32);
    fn CGContextSetRGBFillColor(context: *mut c_void, red: f64, green: f64, blue: f64, alpha: f64);
    fn CGContextFillRect(context: *mut c_void, rect: CGRect);
    fn CGContextDrawImage(context: *mut c_void, rect: CGRect, image: *mut c_void);
}

// ---------------------------------------------------------------------------
// Objective-C message helpers
// ---------------------------------------------------------------------------

unsafe fn class(name: &CStr) -> *mut c_void {
    objc_getClass(name.as_ptr())
}

unsafe fn selector(name: &CStr) -> *mut c_void {
    sel_registerName(name.as_ptr())
}

unsafe fn msg0_ptr(obj: *mut c_void, name: &CStr) -> *mut c_void {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name))
}

unsafe fn msg1_ptr(obj: *mut c_void, name: &CStr, arg: *mut c_void) -> *mut c_void {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), arg)
}

unsafe fn msg3_ptr(
    obj: *mut c_void,
    name: &CStr,
    arg1: *mut c_void,
    arg2: *mut c_void,
    arg3: *mut c_void,
) -> *mut c_void {
    let f: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> *mut c_void = mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), arg1, arg2, arg3)
}

unsafe fn msg1_void_ptr(obj: *mut c_void, name: &CStr, arg: *mut c_void) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), arg);
}

unsafe fn msg1_void_i8(obj: *mut c_void, name: &CStr, arg: i8) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, i8) =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), arg);
}

unsafe fn msg1_void_rect(obj: *mut c_void, name: &CStr, value: CGRect) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, CGRect) =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), value);
}

unsafe fn msg1_void_i64(obj: *mut c_void, name: &CStr, arg: i64) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, i64) =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), arg);
}

unsafe fn msg1_void_u64(obj: *mut c_void, name: &CStr, arg: u64) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, u64) =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), arg);
}

unsafe fn msg1_bool_ptr(obj: *mut c_void, name: &CStr, arg: *mut c_void) -> i8 {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i8 =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), arg)
}

unsafe fn msg0_bool(obj: *mut c_void, name: &CStr) -> i8 {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i8 =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name))
}

unsafe fn msg0_u64(obj: *mut c_void, name: &CStr) -> u64 {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> u64 =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name))
}

unsafe fn msg0_u16(obj: *mut c_void, name: &CStr) -> u16 {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> u16 =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name))
}

unsafe fn msg0_f64(obj: *mut c_void, name: &CStr) -> f64 {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> f64 =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name))
}

unsafe fn msg0_rect(obj: *mut c_void, name: &CStr) -> CGRect {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CGRect =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name))
}

unsafe fn msg1_rect_ptr(obj: *mut c_void, name: &CStr, value: CGRect) -> *mut c_void {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, CGRect) -> *mut c_void =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), value)
}

unsafe fn msg0_point(obj: *mut c_void, name: &CStr) -> CGPoint {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CGPoint =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name))
}

unsafe fn msg_point_ptr(obj: *mut c_void, name: &CStr, point: CGPoint, arg: *mut c_void) -> CGPoint {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, CGPoint, *mut c_void) -> CGPoint =
        mem::transmute(objc_msgSend as *const ());
    f(obj, selector(name), point, arg)
}

#[allow(clippy::too_many_arguments)]
unsafe fn msg_init_window(
    obj: *mut c_void,
    name: &CStr,
    content_rect: CGRect,
    style_mask: u64,
    backing: u64,
    defer: i8,
) -> *mut c_void {
    let f: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        CGRect,
        u64,
        u64,
        i8,
    ) -> *mut c_void = mem::transmute(objc_msgSend as *const ());
    f(
        obj,
        selector(name),
        content_rect,
        style_mask,
        backing,
        defer,
    )
}

unsafe fn ns_string(value: &str) -> *mut c_void {
    let sanitized: String = value
        .chars()
        .map(|ch| if ch == '\0' { ' ' } else { ch })
        .collect();
    let c_value = CString::new(sanitized).unwrap_or_default();
    msg1_ptr(
        class(c"NSString"),
        c"stringWithUTF8String:",
        c_value.as_ptr().cast_mut().cast::<c_void>(),
    )
}

// ---------------------------------------------------------------------------
// Backend state
// ---------------------------------------------------------------------------

static STATE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static CLASS_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct MacState<A: PlatformApp> {
    app: A,
    canvas: Canvas,
    window: *mut c_void,
    view: *mut c_void,
    delegate: *mut c_void,
    last_title: String,
}

impl<A: PlatformApp> Drop for MacState<A> {
    fn drop(&mut self) {
        unsafe {
            if !self.window.is_null() {
                msg1_void_ptr(self.window, c"setDelegate:", ptr::null_mut());
            }
            if !self.delegate.is_null() {
                let f: unsafe extern "C" fn(*mut c_void, *mut c_void) =
                    mem::transmute(objc_msgSend as *const ());
                f(self.delegate, selector(c"release"));
                self.delegate = ptr::null_mut();
            }
        }
    }
}

unsafe fn state_ptr<A: PlatformApp>() -> *mut MacState<A> {
    STATE.load(Ordering::Acquire).cast::<MacState<A>>()
}

unsafe fn with_state<A: PlatformApp, R>(f: impl FnOnce(&mut MacState<A>) -> R) -> Option<R> {
    let ptr = state_ptr::<A>();
    if ptr.is_null() {
        None
    } else {
        Some(f(&mut *ptr))
    }
}

unsafe fn sync_title_and_repaint<A: PlatformApp>() {
    with_state::<A, ()>(|state| {
        let active = state.app.active_title();
        if active != state.last_title {
            state.last_title = active.to_string();
            let title = format_window_title(&state.last_title);
            let ns_title = ns_string(&title);
            if !ns_title.is_null() && !state.window.is_null() {
                msg1_void_ptr(state.window, c"setTitle:", ns_title);
            }
        }
        if state.app.needs_repaint() && !state.view.is_null() {
            if let Some(damage) = state.app.take_damage() {
                let (vw, vh) = state.app.viewport_size();
                let clamped = damage.clamp(vw, vh);
                if !clamped.is_empty() {
                    msg1_void_rect(
                        state.view,
                        c"setNeedsDisplayInRect:",
                        rect(
                            clamped.x as f64,
                            clamped.y as f64,
                            clamped.width as f64,
                            clamped.height as f64,
                        ),
                    );
                }
            } else {
                msg1_void_i8(state.view, c"setNeedsDisplay:", 1);
            }
        }
    });
}

unsafe fn event_view_point<A: PlatformApp>(event: *mut c_void) -> (f32, f32) {
    let window_point = msg0_point(event, c"locationInWindow");
    let view = state_ptr::<A>()
        .as_ref()
        .map_or(ptr::null_mut(), |state| state.view);
    if view.is_null() {
        return (window_point.x as f32, window_point.y as f32);
    }
    let local = msg_point_ptr(view, c"convertPoint:fromView:", window_point, ptr::null_mut());
    (local.x as f32, local.y as f32)
}

unsafe fn event_characters(event: *mut c_void) -> String {
    let characters = msg0_ptr(event, c"characters");
    if characters.is_null() {
        return String::new();
    }
    let utf8 = msg0_ptr(characters, c"UTF8String").cast::<c_char>();
    if utf8.is_null() {
        return String::new();
    }
    CStr::from_ptr(utf8).to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn clamp_cg_dirty_rect(dirty: CGRect, vp_width: f64, vp_height: f64) -> Option<CGRect> {
    if !dirty.origin.x.is_finite()
        || !dirty.origin.y.is_finite()
        || !dirty.size.width.is_finite()
        || !dirty.size.height.is_finite()
        || dirty.size.width <= 0.0
        || dirty.size.height <= 0.0
        || vp_width <= 0.0
        || vp_height <= 0.0
    {
        return None;
    }
    let x1 = dirty.origin.x.clamp(0.0, vp_width);
    let y1 = dirty.origin.y.clamp(0.0, vp_height);
    let x2 = (dirty.origin.x + dirty.size.width).clamp(0.0, vp_width);
    let y2 = (dirty.origin.y + dirty.size.height).clamp(0.0, vp_height);
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some(rect(x1, y1, x2 - x1, y2 - y1))
}

unsafe fn present_canvas(
    view: *mut c_void,
    canvas: &Canvas,
    width: usize,
    height: usize,
    dirty_rect: CGRect,
) {
    let _ = view;
    if width == 0 || height == 0 || canvas.pixels.len() < width.saturating_mul(height) {
        return;
    }

    let graphics_context = msg0_ptr(class(c"NSGraphicsContext"), c"currentContext");
    if graphics_context.is_null() {
        return;
    }
    let cg_context = msg0_ptr(graphics_context, c"CGContext");
    if cg_context.is_null() {
        return;
    }

    let width_f = width as f64;
    let height_f = height as f64;
    let clip =
        clamp_cg_dirty_rect(dirty_rect, width_f, height_f).unwrap_or_else(|| rect(0.0, 0.0, width_f, height_f));

    let color_space = CGColorSpaceCreateDeviceRGB();
    if color_space.is_null() {
        return;
    }
    let bitmap = CGBitmapContextCreate(
        canvas.pixels.as_ptr().cast_mut().cast::<c_void>(),
        width,
        height,
        8,
        width.saturating_mul(4),
        color_space,
        CG_BITMAP_INFO_XRGB_LITTLE,
    );
    CGColorSpaceRelease(color_space);
    if bitmap.is_null() {
        return;
    }

    let image = CGBitmapContextCreateImage(bitmap);
    if !image.is_null() {
        CGContextSaveGState(cg_context);
        CGContextClipToRect(cg_context, clip);
        // Clear only the damaged region before blitting the browser canvas.
        CGContextSetRGBFillColor(cg_context, 1.0, 1.0, 1.0, 1.0);
        CGContextFillRect(cg_context, clip);
        // The NSView is flipped (top-left origin); flip back to CoreGraphics'
        // bottom-left origin so the bitmap is drawn the right way up.
        CGContextTranslateCTM(cg_context, 0.0, height_f);
        CGContextScaleCTM(cg_context, 1.0, -1.0);
        CGContextSetInterpolationQuality(cg_context, 0);
        CGContextDrawImage(cg_context, rect(0.0, 0.0, width_f, height_f), image);
        CGContextRestoreGState(cg_context);
        CGImageRelease(image);
    }
    CGContextRelease(bitmap);
}

// ---------------------------------------------------------------------------
// Objective-C callbacks
// ---------------------------------------------------------------------------

unsafe extern "C" fn accepts_first_responder(_this: *mut c_void, _cmd: *mut c_void) -> i8 {
    1
}

unsafe extern "C" fn is_flipped(_this: *mut c_void, _cmd: *mut c_void) -> i8 {
    1
}

unsafe extern "C" fn draw_rect<A: PlatformApp>(
    this: *mut c_void,
    _cmd: *mut c_void,
    dirty_rect: CGRect,
) {
    with_state::<A, ()>(|state| {
        let (width, height) = state.app.viewport_size();
        if width == 0 || height == 0 {
            return;
        }
        let mut needs_render = state.app.needs_repaint();
        if state.canvas.width != width || state.canvas.height != height {
            state
                .canvas
                .resize(width, height, Color::rgb(255, 255, 255));
            needs_render = true;
        }
        if needs_render {
            let _ = state.app.take_damage();
            state.app.render_into(&mut state.canvas);
            state.app.set_needs_repaint(false);
        }
        present_canvas(this, &state.canvas, width, height, dirty_rect);
    });
}

unsafe extern "C" fn key_down<A: PlatformApp>(
    _this: *mut c_void,
    _cmd: *mut c_void,
    event: *mut c_void,
) {
    if event.is_null() {
        return;
    }
    let flags = msg0_u64(event, c"modifierFlags");
    let modifiers = Modifiers {
        command: flags & NS_EVENT_MODIFIER_COMMAND != 0,
        control: flags & NS_EVENT_MODIFIER_CONTROL != 0,
        option: flags & NS_EVENT_MODIFIER_OPTION != 0,
        shift: flags & NS_EVENT_MODIFIER_SHIFT != 0,
    };
    let key_code = msg0_u16(event, c"keyCode");
    let characters = event_characters(event);
    let handled = with_state::<A, bool>(|state| {
        handle_key(&mut state.app, key_code, &characters, modifiers)
    })
    .unwrap_or(false);
    if handled {
        sync_title_and_repaint::<A>();
    }
}

unsafe extern "C" fn mouse_moved<A: PlatformApp>(
    _this: *mut c_void,
    _cmd: *mut c_void,
    event: *mut c_void,
) {
    if event.is_null() {
        return;
    }
    let (x, y) = event_view_point::<A>(event);
    with_state::<A, ()>(|state| state.app.handle_mouse_move(x, y));
    sync_title_and_repaint::<A>();
}

unsafe extern "C" fn mouse_dragged<A: PlatformApp>(
    this: *mut c_void,
    cmd: *mut c_void,
    event: *mut c_void,
) {
    mouse_moved::<A>(this, cmd, event);
}

unsafe extern "C" fn mouse_down<A: PlatformApp>(
    _this: *mut c_void,
    _cmd: *mut c_void,
    event: *mut c_void,
) {
    if event.is_null() {
        return;
    }
    let (x, y) = event_view_point::<A>(event);
    with_state::<A, ()>(|state| {
        if !state.window.is_null() && !state.view.is_null() {
            msg1_bool_ptr(state.window, c"makeFirstResponder:", state.view);
        }
        state.app.handle_mouse_down(x, y);
    });
    sync_title_and_repaint::<A>();
}

unsafe extern "C" fn mouse_up<A: PlatformApp>(
    _this: *mut c_void,
    _cmd: *mut c_void,
    event: *mut c_void,
) {
    if event.is_null() {
        return;
    }
    let (x, y) = event_view_point::<A>(event);
    with_state::<A, ()>(|state| state.app.handle_mouse_up(x, y));
    sync_title_and_repaint::<A>();
}

unsafe extern "C" fn scroll_wheel<A: PlatformApp>(
    _this: *mut c_void,
    _cmd: *mut c_void,
    event: *mut c_void,
) {
    if event.is_null() {
        return;
    }
    let precise = msg0_bool(event, c"hasPreciseScrollingDeltas") != 0;
    let delta_y = msg0_f64(event, c"scrollingDeltaY") as f32;
    let delta = if precise { -delta_y } else { -delta_y * 48.0 };
    with_state::<A, ()>(|state| state.app.scroll(delta));
    sync_title_and_repaint::<A>();
}

unsafe extern "C" fn window_will_close<A: PlatformApp>(
    _this: *mut c_void,
    _cmd: *mut c_void,
    _notification: *mut c_void,
) {
    let application = msg0_ptr(class(c"NSApplication"), c"sharedApplication");
    if !application.is_null() {
        msg1_void_ptr(application, c"terminate:", ptr::null_mut());
    }
    let _ = std::marker::PhantomData::<A>;
}

unsafe extern "C" fn window_did_resize<A: PlatformApp>(
    _this: *mut c_void,
    _cmd: *mut c_void,
    _notification: *mut c_void,
) {
    with_state::<A, ()>(|state| {
        if state.view.is_null() {
            return;
        }
        let bounds = msg0_rect(state.view, c"bounds");
        let width = bounds.size.width.round().max(1.0) as usize;
        let height = bounds.size.height.round().max(1.0) as usize;
        state.app.resize(width, height);
        state.app.set_needs_repaint(true);
    });
    sync_title_and_repaint::<A>();
}

// ---------------------------------------------------------------------------
// Input mapping
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Modifiers {
    command: bool,
    control: bool,
    option: bool,
    shift: bool,
}

fn handle_key<A: PlatformApp>(
    app: &mut A,
    key_code: u16,
    characters: &str,
    modifiers: Modifiers,
) -> bool {
    if modifiers.command {
        match key_code {
            KEY_L => {
                app.handle_key_navigation(NavKey::FocusUrl);
                return true;
            }
            KEY_T => {
                app.handle_key_navigation(NavKey::NewTab);
                return true;
            }
            KEY_W => {
                app.handle_key_navigation(NavKey::CloseTab);
                return true;
            }
            KEY_R => {
                app.handle_key_navigation(NavKey::Reload);
                return true;
            }
            KEY_H if modifiers.shift => {
                app.handle_key_navigation(NavKey::GoHome);
                return true;
            }
            KEY_LEFT_BRACKET if modifiers.shift => {
                app.handle_key_navigation(NavKey::PrevTab);
                return true;
            }
            KEY_RIGHT_BRACKET if modifiers.shift => {
                app.handle_key_navigation(NavKey::NextTab);
                return true;
            }
            KEY_LEFT_BRACKET => {
                app.handle_key_navigation(NavKey::Back);
                return true;
            }
            KEY_RIGHT_BRACKET => {
                app.handle_key_navigation(NavKey::Forward);
                return true;
            }
            KEY_LEFT => {
                app.handle_key_navigation(NavKey::Back);
                return true;
            }
            KEY_RIGHT => {
                app.handle_key_navigation(NavKey::Forward);
                return true;
            }
            _ => {}
        }
    }

    if modifiers.control && key_code == KEY_TAB {
        if modifiers.shift {
            app.handle_key_navigation(NavKey::PrevTab);
        } else {
            app.handle_key_navigation(NavKey::NextTab);
        }
        return true;
    }

    if modifiers.option && !modifiers.command {
        match key_code {
            KEY_LEFT => {
                app.handle_key_navigation(NavKey::Back);
                return true;
            }
            KEY_RIGHT => {
                app.handle_key_navigation(NavKey::Forward);
                return true;
            }
            KEY_HOME => {
                app.handle_key_navigation(NavKey::GoHome);
                return true;
            }
            _ => {}
        }
    }

    let handled = match key_code {
        KEY_F5 => {
            app.handle_key_navigation(NavKey::Reload);
            true
        }
        KEY_RETURN | KEY_KEYPAD_ENTER => {
            if app.url_focused() {
                app.handle_submit_url();
            }
            true
        }
        KEY_DELETE => {
            app.handle_backspace();
            true
        }
        KEY_ESCAPE => {
            app.handle_key_navigation(NavKey::Escape);
            true
        }
        KEY_UP => {
            app.handle_key_navigation(NavKey::Up);
            true
        }
        KEY_DOWN => {
            app.handle_key_navigation(NavKey::Down);
            true
        }
        KEY_PAGE_UP => {
            app.handle_key_navigation(NavKey::PageUp);
            true
        }
        KEY_PAGE_DOWN => {
            app.handle_key_navigation(NavKey::PageDown);
            true
        }
        KEY_HOME => {
            app.handle_key_navigation(NavKey::Home);
            true
        }
        KEY_END => {
            app.handle_key_navigation(NavKey::End);
            true
        }
        KEY_SPACE if !app.url_focused() => {
            app.handle_key_navigation(NavKey::PageDown);
            true
        }
        _ => false,
    };
    if handled {
        return true;
    }

    if !modifiers.command && !modifiers.control {
        let mut inserted = false;
        for ch in characters.chars() {
            if ch.is_control() {
                continue;
            }
            app.handle_text_input(ch);
            inserted = true;
        }
        if inserted {
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Class registration and startup
// ---------------------------------------------------------------------------

unsafe fn register_view_class<A: PlatformApp>() -> Result<*mut c_void, String> {
    let base = class(c"NSView");
    if base.is_null() {
        return Err("Cocoa NSView class is unavailable".into());
    }
    let serial = CLASS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = CString::new(format!("DropletContentView{}_{}", std::process::id(), serial))
        .map_err(|_| "Invalid Cocoa class name".to_string())?;
    let cls = objc_allocateClassPair(base, name.as_ptr(), 0);
    if cls.is_null() {
        return Err("Failed to allocate the Cocoa content view class".into());
    }

    let methods: [(&CStr, *const c_void, &CStr); 8] = [
        (c"drawRect:", draw_rect::<A> as *const c_void, c"v@:{CGRect={CGPoint=dd}{CGSize=dd}}"),
        (c"acceptsFirstResponder", accepts_first_responder as *const c_void, c"c@:"),
        (c"isFlipped", is_flipped as *const c_void, c"c@:"),
        (c"keyDown:", key_down::<A> as *const c_void, c"v@:@"),
        (c"mouseDown:", mouse_down::<A> as *const c_void, c"v@:@"),
        (c"mouseUp:", mouse_up::<A> as *const c_void, c"v@:@"),
        (c"mouseMoved:", mouse_moved::<A> as *const c_void, c"v@:@"),
        (c"mouseDragged:", mouse_dragged::<A> as *const c_void, c"v@:@"),
    ];
    for (selector_name, imp, types) in methods {
        if class_addMethod(cls, selector(selector_name), imp, types.as_ptr()) == 0 {
            return Err(format!(
                "Failed to install Cocoa method '{}'",
                selector_name.to_string_lossy()
            ));
        }
    }
    if class_addMethod(
        cls,
        selector(c"scrollWheel:"),
        scroll_wheel::<A> as *const c_void,
        c"v@:@".as_ptr(),
    ) == 0
    {
        return Err("Failed to install Cocoa method 'scrollWheel:'".into());
    }

    objc_registerClassPair(cls);
    Ok(cls)
}

unsafe fn register_window_delegate_class<A: PlatformApp>() -> Result<*mut c_void, String> {
    let base = class(c"NSObject");
    if base.is_null() {
        return Err("Cocoa NSObject class is unavailable".into());
    }
    let serial = CLASS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = CString::new(format!("DropletWindowDelegate{}_{}", std::process::id(), serial))
        .map_err(|_| "Invalid Cocoa class name".to_string())?;
    let cls = objc_allocateClassPair(base, name.as_ptr(), 0);
    if cls.is_null() {
        return Err("Failed to allocate the Cocoa window delegate class".into());
    }
    for (selector_name, imp) in [
        (c"windowWillClose:", window_will_close::<A> as *const c_void),
        (c"windowDidResize:", window_did_resize::<A> as *const c_void),
    ] {
        if class_addMethod(cls, selector(selector_name), imp, c"v@:@".as_ptr()) == 0 {
            return Err(format!(
                "Failed to install Cocoa delegate method '{}'",
                selector_name.to_string_lossy()
            ));
        }
    }
    objc_registerClassPair(cls);
    Ok(cls)
}

unsafe fn install_main_menu(application: *mut c_void) {
    if application.is_null() {
        return;
    }
    let menu_class = class(c"NSMenu");
    let item_class = class(c"NSMenuItem");
    if menu_class.is_null() || item_class.is_null() {
        return;
    }

    let menu_bar = msg1_ptr(
        msg0_ptr(menu_class, c"alloc"),
        c"initWithTitle:",
        ns_string(""),
    );
    let app_item = msg1_ptr(
        msg0_ptr(item_class, c"alloc"),
        c"initWithTitle:",
        ns_string(""),
    );
    let app_menu = msg1_ptr(
        msg0_ptr(menu_class, c"alloc"),
        c"initWithTitle:",
        ns_string("Droplet"),
    );
    let quit_item = msg3_ptr(
        msg0_ptr(item_class, c"alloc"),
        c"initWithTitle:action:keyEquivalent:",
        ns_string("Quit Droplet"),
        selector(c"terminate:"),
        ns_string("q"),
    );

    msg1_void_ptr(app_menu, c"addItem:", quit_item);
    msg1_void_ptr(app_item, c"setSubmenu:", app_menu);
    msg1_void_ptr(menu_bar, c"addItem:", app_item);
    msg1_void_ptr(application, c"setMainMenu:", menu_bar);
}

fn canvas_size(app: &impl PlatformApp) -> (usize, usize) {
    let (width, height) = app.viewport_size();
    (width.max(1), height.max(1))
}

pub fn run<A: PlatformApp>(mut app: A) -> Result<(), String> {
    unsafe {
        let application = msg0_ptr(class(c"NSApplication"), c"sharedApplication");
        if application.is_null() {
            return Err("Failed to initialize the Cocoa NSApplication".into());
        }
        msg1_void_i64(
            application,
            c"setActivationPolicy:",
            NS_APPLICATION_ACTIVATION_POLICY_REGULAR,
        );
        install_main_menu(application);

        let view_class = register_view_class::<A>()?;
        let delegate_class = register_window_delegate_class::<A>()?;

        let (width, height) = canvas_size(&app);
        let content_rect = rect(0.0, 0.0, width as f64, height as f64);

        let window = msg_init_window(
            msg0_ptr(class(c"NSWindow"), c"alloc"),
            c"initWithContentRect:styleMask:backing:defer:",
            content_rect,
            NS_WINDOW_STYLE,
            NS_BACKING_STORE_BUFFERED,
            0,
        );
        if window.is_null() {
            return Err("Failed to create the Cocoa NSWindow".into());
        }
        msg1_void_i8(window, c"setReleasedWhenClosed:", 0);
        msg1_void_i8(window, c"setAcceptsMouseMovedEvents:", 1);

        let view = msg1_rect_ptr(
            msg0_ptr(view_class, c"alloc"),
            c"initWithFrame:",
            content_rect,
        );
        if view.is_null() {
            return Err("Failed to create the Cocoa content view".into());
        }
        msg1_void_u64(
            view,
            c"setAutoresizingMask:",
            NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE,
        );

        let delegate = msg0_ptr(delegate_class, c"new");
        if delegate.is_null() {
            return Err("Failed to create the Cocoa window delegate".into());
        }

        msg1_void_ptr(window, c"setContentView:", view);
        msg1_void_ptr(window, c"setDelegate:", delegate);

        app.set_needs_repaint(true);
        app.set_present_backend("cocoa");
        let last_title = app.active_title().to_string();
        let title = ns_string(&format_window_title(&last_title));
        msg1_void_ptr(window, c"setTitle:", title);

        let canvas = Canvas::new(width, height, Color::rgb(255, 255, 255));
        let state = Box::new(MacState {
            app,
            canvas,
            window,
            view,
            delegate,
            last_title,
        });
        let state_raw = Box::into_raw(state);
        STATE.store(state_raw.cast::<c_void>(), Ordering::Release);

        msg1_void_i8(view, c"setNeedsDisplay:", 1);
        msg1_void_ptr(window, c"makeKeyAndOrderFront:", ptr::null_mut());
        msg1_void_i8(application, c"activateIgnoringOtherApps:", 1);

        msg0_ptr(application, c"run");

        STATE.store(ptr::null_mut(), Ordering::Release);
        drop(Box::from_raw(state_raw));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_rect_clamps_to_view_bounds() {
        let clamped = clamp_cg_dirty_rect(rect(-10.0, 20.0, 150.0, 300.0), 120.0, 200.0)
            .expect("clamped dirty rect");
        assert_eq!(clamped.origin.x, 0.0);
        assert_eq!(clamped.origin.y, 20.0);
        assert_eq!(clamped.size.width, 120.0);
        assert_eq!(clamped.size.height, 180.0);

        assert!(clamp_cg_dirty_rect(rect(0.0, 0.0, 0.0, 10.0), 120.0, 200.0).is_none());
        assert!(clamp_cg_dirty_rect(rect(120.0, 0.0, 10.0, 10.0), 120.0, 200.0).is_none());
    }
}
