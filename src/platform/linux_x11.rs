//! Linux X11 graphical windowing backend via zero-dependency dynamic `dlopen`.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::ptr;

use crate::browser::{Browser, NavKey};
use crate::css::Color;
use crate::paint::Canvas;

const RTLD_LAZY: c_int = 1;
const Z_PIXMAP: c_int = 2;
const LSB_FIRST: c_int = 0;

// Event masks
const KEY_PRESS_MASK: c_long = 1 << 0;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const EXPOSURE_MASK: c_long = 1 << 15;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

// Event types
const KEY_PRESS: c_int = 2;
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
const XK_F5: KeySym = 0xFFC2;
const XK_L_UPPER: KeySym = 0x004C;
const XK_L_LOWER: KeySym = 0x006C;
const XK_R_UPPER: KeySym = 0x0052;
const XK_R_LOWER: KeySym = 0x0072;
const XK_T_UPPER: KeySym = 0x0054;
const XK_T_LOWER: KeySym = 0x0074;
const XK_W_UPPER: KeySym = 0x0057;
const XK_W_LOWER: KeySym = 0x0077;

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

#[link(name = "c")]
extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

#[allow(non_snake_case)]
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
    XLookupString: unsafe extern "C" fn(
        *mut XKeyEvent,
        *mut c_char,
        c_int,
        *mut KeySym,
        *mut c_void,
    ) -> c_int,
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
                XDefaultVisual: load_sym(handle, c"XDefaultVisual")?,
                XDefaultDepth: load_sym(handle, c"XDefaultDepth")?,
                XDefaultGC: load_sym(handle, c"XDefaultGC")?,
                XCreateImage: load_sym(handle, c"XCreateImage")?,
                XPutImage: load_sym(handle, c"XPutImage")?,
                XDestroyImage: load_sym(handle, c"XDestroyImage")?,
                XFlush: load_sym(handle, c"XFlush")?,
                XLookupString: load_sym(handle, c"XLookupString")?,
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
    let clean: String = title
        .chars()
        .map(|c| if c == '\0' { ' ' } else { c })
        .collect();
    let label = if clean.is_empty() {
        "Droplet".to_string()
    } else {
        format!("{clean} - Droplet")
    };
    if let Ok(c_title) = CString::new(label) {
        unsafe {
            (x11.XStoreName)(display, window, c_title.as_ptr());
        }
    }
}

fn present_canvas(
    x11: &X11Api,
    display: *mut Display,
    window: Window,
    visual: *mut Visual,
    depth: c_int,
    gc: GC,
    canvas: &mut Canvas,
) {
    if canvas.width == 0 || canvas.height == 0 {
        return;
    }
    if canvas.pixels.len() < canvas.width.saturating_mul(canvas.height) {
        return;
    }

    unsafe {
        let img = (x11.XCreateImage)(
            display,
            visual,
            depth as c_uint,
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
                (x11.XPutImage)(
                    display,
                    window,
                    gc,
                    img,
                    0,
                    0,
                    0,
                    0,
                    canvas.width as c_uint,
                    canvas.height as c_uint,
                );
            }
            // Clear data pointer before XDestroyImage so Xlib does not free Rust's Vec<u32>.
            (*img).data = ptr::null_mut();
            (x11.XDestroyImage)(img);
        }
        (x11.XFlush)(display);
    }
}

fn handle_key_event(x11: &X11Api, browser: &mut Browser, key_ev: &mut XKeyEvent) {
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
        browser.handle_key_navigation(NavKey::FocusUrl);
        return;
    }
    if ctrl && !alt && (keysym == XK_T_LOWER || keysym == XK_T_UPPER) {
        browser.handle_key_navigation(NavKey::NewTab);
        return;
    }
    if ctrl && !alt && (keysym == XK_W_LOWER || keysym == XK_W_UPPER) {
        browser.handle_key_navigation(NavKey::CloseTab);
        return;
    }
    if ctrl && !alt && (keysym == XK_R_LOWER || keysym == XK_R_UPPER) {
        browser.handle_key_navigation(NavKey::Reload);
        return;
    }
    if ctrl && !alt && keysym == XK_ISO_LEFT_TAB {
        browser.handle_key_navigation(NavKey::PrevTab);
        return;
    }
    if ctrl && !alt && keysym == XK_TAB {
        if shift {
            browser.handle_key_navigation(NavKey::PrevTab);
        } else {
            browser.handle_key_navigation(NavKey::NextTab);
        }
        return;
    }
    if alt && !ctrl && keysym == XK_LEFT {
        browser.handle_key_navigation(NavKey::Back);
        return;
    }
    if alt && !ctrl && keysym == XK_RIGHT {
        browser.handle_key_navigation(NavKey::Forward);
        return;
    }
    if alt && !ctrl && keysym == XK_HOME {
        browser.handle_key_navigation(NavKey::GoHome);
        return;
    }

    match keysym {
        XK_F5 => browser.handle_key_navigation(NavKey::Reload),
        XK_RETURN | XK_KP_ENTER => {
            if browser.url_focused {
                browser.handle_submit_url();
            }
        }
        XK_BACKSPACE => browser.handle_backspace(),
        XK_ESCAPE => browser.handle_key_navigation(NavKey::Escape),
        XK_UP => browser.handle_key_navigation(NavKey::Up),
        XK_DOWN => browser.handle_key_navigation(NavKey::Down),
        XK_PAGE_UP => browser.handle_key_navigation(NavKey::PageUp),
        XK_PAGE_DOWN => browser.handle_key_navigation(NavKey::PageDown),
        XK_HOME => browser.handle_key_navigation(NavKey::Home),
        XK_END => browser.handle_key_navigation(NavKey::End),
        _ => {
            if !ctrl && !alt && len > 0 {
                let slice = &buf[..(len as usize).min(buf.len())];
                let text = String::from_utf8_lossy(slice);
                for ch in text.chars() {
                    if ch == ' ' && !browser.url_focused {
                        browser.handle_key_navigation(NavKey::PageDown);
                    } else if !ch.is_control() {
                        browser.handle_text_input(ch);
                    }
                }
            }
        }
    }
}

pub fn run(mut browser: Browser) -> Result<(), String> {
    let x11 = X11Api::load()?;
    let display = unsafe { (x11.XOpenDisplay)(ptr::null()) };
    if display.is_null() {
        return Err("Unable to open X11 display (check $DISPLAY)".into());
    }
    let _display_guard = DisplayGuard {
        x11: &x11,
        display,
    };

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

    let win_w = browser.viewport_width.max(1) as c_uint;
    let win_h = browser.viewport_height.max(1) as c_uint;

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
                | BUTTON_PRESS_MASK
                | BUTTON_RELEASE_MASK
                | POINTER_MOTION_MASK
                | STRUCTURE_NOTIFY_MASK,
        );
    }

    let mut last_title = browser.active_title().to_string();
    set_window_title(&x11, display, window, &last_title);

    unsafe {
        (x11.XMapWindow)(display, window);
        (x11.XFlush)(display);
    }

    let mut canvas = Canvas::new(
        browser.viewport_width,
        browser.viewport_height,
        Color::rgb(255, 255, 255),
    );
    browser.render_into(&mut canvas);
    present_canvas(&x11, display, window, visual, depth, gc, &mut canvas);
    browser.needs_repaint = false;

    let mut event = XEvent { pad: [0; 24] };
    let mut running = true;

    while running {
        unsafe {
            (x11.XNextEvent)(display, &mut event);
        }

        let mut expose_needed = false;
        let mut pending_motion: Option<(f32, f32)> = None;
        let mut pending_resize: Option<(usize, usize)> = None;

        loop {
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
                            browser.resize(w, h);
                        }
                        if let Some((mx, my)) = pending_motion.take() {
                            browser.handle_mouse_move(mx, my);
                        }
                        let btn = event.xbutton.button;
                        let bx = event.xbutton.x as f32;
                        let by = event.xbutton.y as f32;
                        match btn {
                            1 => browser.handle_mouse_down(bx, by),
                            4 => browser.scroll(-48.0),
                            5 => browser.scroll(48.0),
                            8 => browser.handle_key_navigation(NavKey::Back),
                            9 => browser.handle_key_navigation(NavKey::Forward),
                            _ => {}
                        }
                    }
                    BUTTON_RELEASE => {
                        if let Some((w, h)) = pending_resize.take() {
                            browser.resize(w, h);
                        }
                        if let Some((mx, my)) = pending_motion.take() {
                            browser.handle_mouse_move(mx, my);
                        }
                        if event.xbutton.button == 1 {
                            let bx = event.xbutton.x as f32;
                            let by = event.xbutton.y as f32;
                            browser.handle_mouse_up(bx, by);
                        }
                    }
                    KEY_PRESS => {
                        if let Some((w, h)) = pending_resize.take() {
                            browser.resize(w, h);
                        }
                        let mut key_ev = event.xkey;
                        handle_key_event(&x11, &mut browser, &mut key_ev);
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
            browser.resize(w, h);
        }
        if let Some((mx, my)) = pending_motion {
            browser.handle_mouse_move(mx, my);
        }

        let current_title = browser.active_title();
        if current_title != last_title {
            last_title = current_title.to_string();
            set_window_title(&x11, display, window, &last_title);
        }

        if browser.needs_repaint || expose_needed {
            if canvas.width != browser.viewport_width || canvas.height != browser.viewport_height {
                canvas.resize(
                    browser.viewport_width,
                    browser.viewport_height,
                    Color::rgb(255, 255, 255),
                );
            }
            browser.render_into(&mut canvas);
            present_canvas(&x11, display, window, visual, depth, gc, &mut canvas);
            browser.needs_repaint = false;
        }
    }

    Ok(())
}
