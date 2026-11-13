//! Windows native Win32 graphical windowing backend with zero external crate dependencies.

use std::ffi::c_void;
use std::mem;
use std::ptr;

use droplet_gfx::{Canvas, Color};

use crate::{decode_utf16_unit, format_window_title, to_wide_null, NavKey, PlatformApp};

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMODULE = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HMENU = *mut c_void;
type HDC = *mut c_void;
type WndProc = unsafe extern "system" fn(HWND, u32, usize, isize) -> isize;

// Class and window styles
const CS_VREDRAW: u32 = 0x0001;
const CS_HREDRAW: u32 = 0x0002;

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
const WS_VISIBLE: u32 = 0x1000_0000;

const CW_USEDEFAULT: i32 = 0x8000_0000_u32 as i32;
const SW_SHOW: i32 = 5;
const GWLP_USERDATA: i32 = -21;
const IDC_ARROW: *const u16 = 32512 as *const u16;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const SIZE_MINIMIZED: usize = 1;

// Window messages
const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCDESTROY: u32 = 0x0082;
const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_XBUTTONDOWN: u32 = 0x020B;

// Mouse X-buttons
const XBUTTON1: u16 = 0x0001;
const XBUTTON2: u16 = 0x0002;

// GDI DIB constants
const DIB_RGB_COLORS: u32 = 0;
const BI_RGB: u32 = 0;
const SRCCOPY: u32 = 0x00CC_0020;

// Virtual key codes
const VK_BACK: usize = 0x08;
const VK_TAB: usize = 0x09;
const VK_RETURN: usize = 0x0D;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_ESCAPE: usize = 0x1B;
const VK_PRIOR: usize = 0x21;
const VK_NEXT: usize = 0x22;
const VK_END: usize = 0x23;
const VK_HOME: usize = 0x24;
const VK_LEFT: usize = 0x25;
const VK_UP: usize = 0x26;
const VK_RIGHT: usize = 0x27;
const VK_DOWN: usize = 0x28;
const VK_L: usize = 0x4C;
const VK_R: usize = 0x52;
const VK_T: usize = 0x54;
const VK_W: usize = 0x57;
const VK_F5: usize = 0x74;

#[repr(C)]
#[derive(Copy, Clone, Default)]
#[allow(dead_code)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[allow(dead_code)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: WndProc,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: HINSTANCE,
    h_icon: HICON,
    h_cursor: HCURSOR,
    hbr_background: HBRUSH,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: HICON,
}

#[repr(C)]
#[allow(dead_code)]
struct MSG {
    hwnd: HWND,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
#[allow(dead_code)]
struct PAINTSTRUCT {
    hdc: HDC,
    f_erase: i32,
    rc_paint: RECT,
    f_restore: i32,
    f_inc_update: i32,
    rgb_reserved: [u8; 32],
}

#[repr(C)]
#[allow(dead_code)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
#[allow(dead_code)]
struct RGBQUAD {
    rgb_blue: u8,
    rgb_green: u8,
    rgb_red: u8,
    rgb_reserved: u8,
}

#[repr(C)]
#[allow(dead_code)]
struct BITMAPINFO {
    bmi_header: BITMAPINFOHEADER,
    bmi_colors: [RGBQUAD; 1],
}

#[repr(C)]
#[allow(dead_code)]
struct CREATESTRUCTW {
    lp_create_params: *mut c_void,
    h_instance: HINSTANCE,
    h_menu: HMENU,
    hwnd_parent: HWND,
    cy: i32,
    cx: i32,
    y: i32,
    x: i32,
    style: i32,
    lpsz_name: *const u16,
    lpsz_class: *const u16,
    dw_ex_style: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(lp_module_name: *const u16) -> HMODULE;
    fn GetLastError() -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    fn UnregisterClassW(lp_class_name: *const u16, h_instance: HINSTANCE) -> i32;
    fn AdjustWindowRectEx(
        lp_rect: *mut RECT,
        dw_style: u32,
        b_menu: i32,
        dw_ex_style: u32,
    ) -> i32;
    fn CreateWindowExW(
        dw_ex_style: u32,
        lp_class_name: *const u16,
        lp_window_name: *const u16,
        dw_style: u32,
        x: i32,
        y: i32,
        n_width: i32,
        n_height: i32,
        hwnd_parent: HWND,
        h_menu: HMENU,
        h_instance: HINSTANCE,
        lp_param: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hwnd: HWND) -> i32;
    fn ShowWindow(hwnd: HWND, n_cmd_show: i32) -> i32;
    fn UpdateWindow(hwnd: HWND) -> i32;
    fn GetMessageW(
        lp_msg: *mut MSG,
        hwnd: HWND,
        w_msg_filter_min: u32,
        w_msg_filter_max: u32,
    ) -> i32;
    fn TranslateMessage(lp_msg: *const MSG) -> i32;
    fn DispatchMessageW(lp_msg: *const MSG) -> isize;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w_param: usize, l_param: isize) -> isize;
    fn PostQuitMessage(n_exit_code: i32);
    fn BeginPaint(hwnd: HWND, lp_paint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hwnd: HWND, lp_paint: *const PAINTSTRUCT) -> i32;
    fn InvalidateRect(hwnd: HWND, lp_rect: *const RECT, b_erase: i32) -> i32;
    fn SetWindowTextW(hwnd: HWND, lp_string: *const u16) -> i32;
    fn LoadCursorW(h_instance: HINSTANCE, lp_cursor_name: *const u16) -> HCURSOR;
    fn GetKeyState(n_virt_key: i32) -> i16;
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
}

#[cfg(target_pointer_width = "64")]
#[link(name = "user32")]
extern "system" {
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, new_long: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
}

#[cfg(target_pointer_width = "32")]
#[link(name = "user32")]
extern "system" {
    #[link_name = "SetWindowLongW"]
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, new_long: isize) -> isize;
    #[link_name = "GetWindowLongW"]
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
}

#[link(name = "gdi32")]
extern "system" {
    fn StretchDIBits(
        hdc: HDC,
        x_dest: i32,
        y_dest: i32,
        dest_width: i32,
        dest_height: i32,
        x_src: i32,
        y_src: i32,
        src_width: i32,
        src_height: i32,
        lp_bits: *const c_void,
        lpbmi: *const BITMAPINFO,
        i_usage: u32,
        rop: u32,
    ) -> i32;
}

struct WinState<A: PlatformApp> {
    app: A,
    canvas: Canvas,
    last_title: String,
    hwnd: HWND,
    pending_surrogate: Option<u16>,
}

unsafe fn present_canvas(hdc: HDC, canvas: &Canvas) {
    if canvas.width == 0 || canvas.height == 0 {
        return;
    }
    if canvas.width > i32::MAX as usize || canvas.height > i32::MAX as usize {
        return;
    }
    if canvas.pixels.len() < canvas.width.saturating_mul(canvas.height) {
        return;
    }

    let w = canvas.width as i32;
    let h = canvas.height as i32;
    let bmi = BITMAPINFO {
        bmi_header: BITMAPINFOHEADER {
            bi_size: mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: w,
            // Negative height selects a top-down 32-bit BI_RGB DIB matching 0x00RRGGBB pixels.
            bi_height: -h,
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: BI_RGB,
            bi_size_image: 0,
            bi_x_pels_per_meter: 0,
            bi_y_pels_per_meter: 0,
            bi_clr_used: 0,
            bi_clr_important: 0,
        },
        bmi_colors: [RGBQUAD {
            rgb_blue: 0,
            rgb_green: 0,
            rgb_red: 0,
            rgb_reserved: 0,
        }],
    };

    StretchDIBits(
        hdc,
        0,
        0,
        w,
        h,
        0,
        0,
        w,
        h,
        canvas.pixels.as_ptr() as *const c_void,
        &bmi,
        DIB_RGB_COLORS,
        SRCCOPY,
    );
}

fn handle_key_down<A: PlatformApp>(
    app: &mut A,
    vk: usize,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> bool {
    if ctrl && !alt {
        match vk {
            VK_L => {
                app.handle_key_navigation(NavKey::FocusUrl);
                return true;
            }
            VK_T => {
                app.handle_key_navigation(NavKey::NewTab);
                return true;
            }
            VK_W => {
                app.handle_key_navigation(NavKey::CloseTab);
                return true;
            }
            VK_R => {
                app.handle_key_navigation(NavKey::Reload);
                return true;
            }
            VK_TAB => {
                if shift {
                    app.handle_key_navigation(NavKey::PrevTab);
                } else {
                    app.handle_key_navigation(NavKey::NextTab);
                }
                return true;
            }
            _ => {}
        }
    }

    if alt && !ctrl {
        match vk {
            VK_LEFT => {
                app.handle_key_navigation(NavKey::Back);
                return true;
            }
            VK_RIGHT => {
                app.handle_key_navigation(NavKey::Forward);
                return true;
            }
            VK_HOME => {
                app.handle_key_navigation(NavKey::GoHome);
                return true;
            }
            _ => {}
        }
    }

    match vk {
        VK_F5 => {
            app.handle_key_navigation(NavKey::Reload);
            true
        }
        VK_RETURN => {
            if app.url_focused() {
                app.handle_submit_url();
            }
            true
        }
        VK_BACK => {
            app.handle_backspace();
            true
        }
        VK_ESCAPE => {
            app.handle_key_navigation(NavKey::Escape);
            true
        }
        VK_UP => {
            app.handle_key_navigation(NavKey::Up);
            true
        }
        VK_DOWN => {
            app.handle_key_navigation(NavKey::Down);
            true
        }
        VK_PRIOR => {
            app.handle_key_navigation(NavKey::PageUp);
            true
        }
        VK_NEXT => {
            app.handle_key_navigation(NavKey::PageDown);
            true
        }
        VK_HOME => {
            app.handle_key_navigation(NavKey::Home);
            true
        }
        VK_END => {
            app.handle_key_navigation(NavKey::End);
            true
        }
        _ => false,
    }
}

unsafe fn sync_title_and_invalidate<A: PlatformApp>(state_ptr: *mut WinState<A>) {
    let (hwnd, new_title, needs_repaint) = {
        let state = &mut *state_ptr;
        let active = state.app.active_title();
        let title_change = if active != state.last_title {
            state.last_title = active.to_string();
            Some(format_window_title(&state.last_title))
        } else {
            None
        };
        (state.hwnd, title_change, state.app.needs_repaint())
    };

    if hwnd.is_null() {
        return;
    }

    if let Some(title) = new_title {
        let wide = to_wide_null(&title);
        SetWindowTextW(hwnd, wide.as_ptr());
    }
    if needs_repaint {
        InvalidateRect(hwnd, ptr::null(), 0);
    }
}

unsafe extern "system" fn wnd_proc<A: PlatformApp>(
    hwnd: HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if msg == WM_NCCREATE {
        let cs = lparam as *const CREATESTRUCTW;
        if !cs.is_null() {
            let state_ptr = (*cs).lp_create_params as *mut WinState<A>;
            if !state_ptr.is_null() {
                (*state_ptr).hwnd = hwnd;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    if msg == WM_NCDESTROY {
        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WinState<A>;
        if !state_ptr.is_null() {
            (*state_ptr).hwnd = ptr::null_mut();
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    if msg == WM_ERASEBKGND {
        return 1;
    }

    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WinState<A>;
    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    match msg {
        WM_SIZE => {
            let w = (lparam & 0xFFFF) as u16 as usize;
            let h = ((lparam >> 16) & 0xFFFF) as u16 as usize;
            if wparam != SIZE_MINIMIZED && w > 0 && h > 0 {
                {
                    let state = &mut *state_ptr;
                    state.app.resize(w, h);
                }
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.is_null() {
                {
                    let state = &mut *state_ptr;
                    let (vw, vh) = state.app.viewport_size();
                    if state.app.needs_repaint()
                        || state.canvas.width != vw
                        || state.canvas.height != vh
                    {
                        if state.canvas.width != vw || state.canvas.height != vh {
                            state.canvas.resize(vw, vh, Color::rgb(255, 255, 255));
                        }
                        state.app.render_into(&mut state.canvas);
                        state.app.set_needs_repaint(false);
                    }
                    present_canvas(hdc, &state.canvas);
                }
                EndPaint(hwnd, &ps);
            }
            0
        }
        WM_MOUSEMOVE => {
            let x = (lparam & 0xFFFF) as i16 as f32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
            {
                let state = &mut *state_ptr;
                state.app.handle_mouse_move(x, y);
            }
            sync_title_and_invalidate(state_ptr);
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam & 0xFFFF) as i16 as f32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
            SetCapture(hwnd);
            {
                let state = &mut *state_ptr;
                state.app.handle_mouse_down(x, y);
            }
            sync_title_and_invalidate(state_ptr);
            0
        }
        WM_LBUTTONUP => {
            let x = (lparam & 0xFFFF) as i16 as f32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
            ReleaseCapture();
            {
                let state = &mut *state_ptr;
                state.app.handle_mouse_up(x, y);
            }
            sync_title_and_invalidate(state_ptr);
            0
        }
        WM_XBUTTONDOWN => {
            let xbutton = ((wparam >> 16) & 0xFFFF) as u16;
            let handled = {
                let state = &mut *state_ptr;
                match xbutton {
                    XBUTTON1 => {
                        state.app.handle_key_navigation(NavKey::Back);
                        true
                    }
                    XBUTTON2 => {
                        state.app.handle_key_navigation(NavKey::Forward);
                        true
                    }
                    _ => false,
                }
            };
            if handled {
                sync_title_and_invalidate(state_ptr);
                1
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_MOUSEWHEEL => {
            let wheel_delta = ((wparam >> 16) & 0xFFFF) as i16;
            let delta_y = -(wheel_delta as f32 / 120.0) * 48.0;
            {
                let state = &mut *state_ptr;
                state.app.scroll(delta_y);
            }
            sync_title_and_invalidate(state_ptr);
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let ctrl = GetKeyState(VK_CONTROL) < 0;
            let alt = GetKeyState(VK_MENU) < 0 || (lparam & (1 << 29)) != 0;
            let shift = GetKeyState(VK_SHIFT) < 0;
            let handled = {
                let state = &mut *state_ptr;
                handle_key_down(&mut state.app, wparam, ctrl, alt, shift)
            };
            if handled {
                sync_title_and_invalidate(state_ptr);
                0
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_CHAR => {
            let unit = (wparam & 0xFFFF) as u16;
            if unit < 0x20 {
                let state = &mut *state_ptr;
                state.pending_surrogate = None;
                return 0;
            }
            {
                let state = &mut *state_ptr;
                if let Some(ch) = decode_utf16_unit(&mut state.pending_surrogate, unit) {
                    if ch == ' ' && !state.app.url_focused() {
                        state.app.handle_key_navigation(NavKey::PageDown);
                    } else if !ch.is_control() {
                        state.app.handle_text_input(ch);
                    }
                }
            }
            sync_title_and_invalidate(state_ptr);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn run<A: PlatformApp>(mut app: A) -> Result<(), String> {
    let h_instance = unsafe { GetModuleHandleW(ptr::null()) };
    let class_name = to_wide_null("DropletBrowserWindow");

    let wc = WNDCLASSEXW {
        cb_size: mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfn_wnd_proc: wnd_proc::<A>,
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance,
        h_icon: ptr::null_mut(),
        h_cursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbr_background: ptr::null_mut(),
        lpsz_menu_name: ptr::null(),
        lpsz_class_name: class_name.as_ptr(),
        h_icon_sm: ptr::null_mut(),
    };

    if unsafe { RegisterClassExW(&wc) } == 0 {
        let err = unsafe { GetLastError() };
        if err != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!("RegisterClassExW failed (Win32 error {err})"));
        }
    }

    let (vw, vh) = app.viewport_size();
    let style = WS_OVERLAPPEDWINDOW | WS_VISIBLE;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: vw.max(1) as i32,
        bottom: vh.max(1) as i32,
    };
    let (win_w, win_h) = if unsafe { AdjustWindowRectEx(&mut rect, style, 0, 0) } != 0 {
        (rect.right - rect.left, rect.bottom - rect.top)
    } else {
        (vw.max(1) as i32, vh.max(1) as i32)
    };

    app.set_needs_repaint(true);
    let last_title = app.active_title().to_string();
    let title_wide = to_wide_null(&format_window_title(&last_title));
    let canvas = Canvas::new(vw, vh, Color::rgb(255, 255, 255));

    let state = Box::new(WinState {
        app,
        canvas,
        last_title,
        hwnd: ptr::null_mut(),
        pending_surrogate: None,
    });
    let state_ptr = Box::into_raw(state);

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title_wide.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            win_w,
            win_h,
            ptr::null_mut(),
            ptr::null_mut(),
            h_instance,
            state_ptr as *mut c_void,
        )
    };
    if hwnd.is_null() {
        let err = unsafe { GetLastError() };
        unsafe {
            UnregisterClassW(class_name.as_ptr(), h_instance);
            drop(Box::from_raw(state_ptr));
        }
        return Err(format!("CreateWindowExW failed (Win32 error {err})"));
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }

    let mut msg: MSG = unsafe { mem::zeroed() };
    loop {
        let ret = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
        if ret == 0 {
            break;
        }
        if ret < 0 {
            let err = unsafe { GetLastError() };
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DestroyWindow(hwnd);
                UnregisterClassW(class_name.as_ptr(), h_instance);
                drop(Box::from_raw(state_ptr));
            }
            return Err(format!("GetMessageW failed (Win32 error {err})"));
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        UnregisterClassW(class_name.as_ptr(), h_instance);
        drop(Box::from_raw(state_ptr));
    }
    Ok(())
}
