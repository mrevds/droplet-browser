//! Windows native Win32 graphical windowing backend with zero external crate dependencies.

use std::ffi::c_void;
use std::mem;
use std::ptr;

use droplet_gfx::{Canvas, Color, DamageRect};

use crate::{
    decode_utf16_unit, format_window_title, to_wide_null, CaretMove, NavKey, PlatformApp,
};

#[allow(clippy::upper_case_acronyms)]
type HWND = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HINSTANCE = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HMODULE = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HICON = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HCURSOR = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HBRUSH = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HMENU = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HDC = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HBITMAP = *mut c_void;
#[allow(clippy::upper_case_acronyms)]
type HGDIOBJ = *mut c_void;
const HGDI_ERROR: HGDIOBJ = usize::MAX as HGDIOBJ;
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
const VK_DELETE: usize = 0x2E;
const VK_DIGIT_1: usize = 0x31;
const VK_DIGIT_9: usize = 0x39;
const VK_A: usize = 0x41;
const VK_C: usize = 0x43;
const VK_D: usize = 0x44;
const VK_F: usize = 0x46;
const VK_I: usize = 0x49;
const VK_L: usize = 0x4C;
const VK_R: usize = 0x52;
const VK_T: usize = 0x54;
const VK_V: usize = 0x56;
const VK_W: usize = 0x57;
const VK_X: usize = 0x58;
const VK_F3: usize = 0x72;
const VK_F5: usize = 0x74;

#[repr(C)]
#[derive(Copy, Clone, Default)]
#[allow(dead_code)]
#[allow(clippy::upper_case_acronyms)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
#[allow(clippy::upper_case_acronyms)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[allow(dead_code)]
#[allow(clippy::upper_case_acronyms)]
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
#[allow(clippy::upper_case_acronyms)]
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
#[allow(clippy::upper_case_acronyms)]
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
#[allow(clippy::upper_case_acronyms)]
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
#[allow(clippy::upper_case_acronyms)]
struct RGBQUAD {
    rgb_blue: u8,
    rgb_green: u8,
    rgb_red: u8,
    rgb_reserved: u8,
}

#[repr(C)]
#[allow(dead_code)]
#[allow(clippy::upper_case_acronyms)]
struct BITMAPINFO {
    bmi_header: BITMAPINFOHEADER,
    bmi_colors: [RGBQUAD; 1],
}

#[repr(C)]
#[allow(dead_code)]
#[allow(clippy::upper_case_acronyms)]
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
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> i32;
    fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(ho: HGDIOBJ) -> i32;
    fn CreateDIBSection(
        hdc: HDC,
        pbmi: *const BITMAPINFO,
        usage: u32,
        ppv_bits: *mut *mut c_void,
        h_section: *mut c_void,
        offset: u32,
    ) -> HBITMAP;
    fn BitBlt(
        hdc: HDC,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdc_src: HDC,
        x1: i32,
        y1: i32,
        rop: u32,
    ) -> i32;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlitRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

fn top_down_bitmap_info(width: usize, height: usize) -> Option<BITMAPINFO> {
    if width == 0 || height == 0 {
        return None;
    }
    if width > i32::MAX as usize || height > i32::MAX as usize {
        return None;
    }
    let byte_len = width.checked_mul(height)?.checked_mul(4)?;
    if byte_len > u32::MAX as usize {
        return None;
    }
    Some(BITMAPINFO {
        bmi_header: BITMAPINFOHEADER {
            bi_size: mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: width as i32,
            // Negative height selects a top-down 32-bit BI_RGB DIB matching 0x00RRGGBB pixels.
            bi_height: -(height as i32),
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
    })
}

fn clamp_rect_to_viewport(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    vp_width: usize,
    vp_height: usize,
) -> Option<BlitRect> {
    if width == 0 || height == 0 || vp_width == 0 || vp_height == 0 {
        return None;
    }
    let x1 = x.min(vp_width);
    let y1 = y.min(vp_height);
    let x2 = x.saturating_add(width).min(vp_width);
    let y2 = y.saturating_add(height).min(vp_height);
    if x2 <= x1 || y2 <= y1 || x2 > i32::MAX as usize || y2 > i32::MAX as usize {
        return None;
    }
    Some(BlitRect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

fn rect_from_paint_rect(rc: &RECT, vp_width: usize, vp_height: usize) -> Option<BlitRect> {
    if rc.right <= rc.left || rc.bottom <= rc.top {
        return None;
    }
    let left = rc.left.max(0) as usize;
    let top = rc.top.max(0) as usize;
    let right = rc.right.max(0) as usize;
    let bottom = rc.bottom.max(0) as usize;
    clamp_rect_to_viewport(
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
        vp_width,
        vp_height,
    )
}

fn union_blit_rects(a: Option<BlitRect>, b: Option<BlitRect>) -> Option<BlitRect> {
    match (a, b) {
        (Some(ra), Some(rb)) => {
            let x1 = ra.x.min(rb.x);
            let y1 = ra.y.min(rb.y);
            let x2 = ra
                .x
                .saturating_add(ra.width)
                .max(rb.x.saturating_add(rb.width));
            let y2 = ra
                .y
                .saturating_add(ra.height)
                .max(rb.y.saturating_add(rb.height));
            Some(BlitRect {
                x: x1,
                y: y1,
                width: x2.saturating_sub(x1),
                height: y2.saturating_sub(y1),
            })
        }
        (Some(r), None) | (None, Some(r)) => Some(r),
        (None, None) => None,
    }
}

/// Persistent top-down 32bpp GDI DIBSection paired with a compatible memory DC
/// so software rendering can write straight into GDI-backed memory and present
/// only the damaged sub-rect via `BitBlt`.
struct DibSurface {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u32,
    width: usize,
    height: usize,
    len: usize,
}

impl DibSurface {
    unsafe fn create(screen_dc: HDC, width: usize, height: usize) -> Option<Self> {
        let bmi = top_down_bitmap_info(width, height)?;
        let len = width.checked_mul(height)?;
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            return None;
        }
        let mut raw_bits: *mut c_void = ptr::null_mut();
        let bitmap = CreateDIBSection(
            mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut raw_bits,
            ptr::null_mut(),
            0,
        );
        if bitmap.is_null()
            || raw_bits.is_null()
            || !(raw_bits as usize).is_multiple_of(mem::align_of::<u32>())
        {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(mem_dc);
            return None;
        }
        let old_bitmap = SelectObject(mem_dc, bitmap);
        if old_bitmap.is_null() || old_bitmap == HGDI_ERROR {
            DeleteObject(bitmap);
            DeleteDC(mem_dc);
            return None;
        }
        Some(Self {
            mem_dc,
            bitmap,
            old_bitmap,
            bits: raw_bits.cast::<u32>(),
            width,
            height,
            len,
        })
    }

    fn matches(&self, width: usize, height: usize) -> bool {
        self.width == width && self.height == height && !self.bits.is_null()
    }

    fn pixels_mut(&mut self) -> Option<&mut [u32]> {
        if self.bits.is_null() || self.len == 0 {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts_mut(self.bits, self.len) })
    }

    unsafe fn blit_rect(&self, dest_dc: HDC, rect: BlitRect) -> bool {
        let Some(clamped) = clamp_rect_to_viewport(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            self.width,
            self.height,
        ) else {
            return false;
        };
        BitBlt(
            dest_dc,
            clamped.x as i32,
            clamped.y as i32,
            clamped.width as i32,
            clamped.height as i32,
            self.mem_dc,
            clamped.x as i32,
            clamped.y as i32,
            SRCCOPY,
        ) != 0
    }

    unsafe fn destroy(&mut self) {
        if !self.mem_dc.is_null() {
            if !self.old_bitmap.is_null() && self.old_bitmap != HGDI_ERROR {
                SelectObject(self.mem_dc, self.old_bitmap);
                self.old_bitmap = ptr::null_mut();
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
                self.bitmap = ptr::null_mut();
            }
            DeleteDC(self.mem_dc);
            self.mem_dc = ptr::null_mut();
        } else if !self.bitmap.is_null() {
            DeleteObject(self.bitmap);
            self.bitmap = ptr::null_mut();
        }
        self.bits = ptr::null_mut();
        self.width = 0;
        self.height = 0;
        self.len = 0;
    }
}

impl Drop for DibSurface {
    fn drop(&mut self) {
        unsafe {
            self.destroy();
        }
    }
}

struct WinState<A: PlatformApp> {
    app: A,
    canvas: Canvas,
    dib_surface: Option<DibSurface>,
    pending_damage: Option<DamageRect>,
    pending_full_repaint: bool,
    present_backend: Option<&'static str>,
    last_title: String,
    hwnd: HWND,
    pending_surrogate: Option<u16>,
}

fn report_present_backend<A: PlatformApp>(
    app: &mut A,
    active_backend: &mut Option<&'static str>,
    backend: &'static str,
) {
    if *active_backend != Some(backend) {
        *active_backend = Some(backend);
        app.set_present_backend(backend);
    }
}

fn merge_damage(a: Option<DamageRect>, b: Option<DamageRect>) -> Option<DamageRect> {
    match (a, b) {
        (Some(da), Some(db)) => Some(da.union(db)),
        (Some(d), None) | (None, Some(d)) => Some(d),
        (None, None) => None,
    }
}

fn damage_to_blit_rect(damage: DamageRect, vp_width: usize, vp_height: usize) -> Option<BlitRect> {
    let clamped = damage.clamp(vp_width, vp_height);
    if clamped.is_empty() {
        return None;
    }
    clamp_rect_to_viewport(
        clamped.x,
        clamped.y,
        clamped.width,
        clamped.height,
        vp_width,
        vp_height,
    )
}

unsafe fn present_canvas(hdc: HDC, canvas: &Canvas) {
    let Some(bmi) = top_down_bitmap_info(canvas.width, canvas.height) else {
        return;
    };
    let Some(pixel_count) = canvas.width.checked_mul(canvas.height) else {
        return;
    };
    if canvas.pixels.len() < pixel_count {
        return;
    }

    let w = canvas.width as i32;
    let h = canvas.height as i32;
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
            VK_A => {
                app.handle_select_all();
                return true;
            }
            VK_C => {
                app.handle_copy();
                return true;
            }
            VK_X => {
                app.handle_cut();
                return true;
            }
            VK_V => {
                app.handle_paste();
                return true;
            }
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
            VK_I if shift => {
                app.handle_key_navigation(NavKey::ToggleInspector);
                return true;
            }
            VK_D => {
                app.handle_key_navigation(NavKey::ToggleBookmark);
                return true;
            }
            VK_F => {
                app.handle_key_navigation(NavKey::Find);
                return true;
            }
            VK_DIGIT_1..=VK_DIGIT_9 => {
                app.handle_key_navigation(NavKey::SwitchTab(vk - VK_DIGIT_1));
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
        VK_F3 => {
            if shift {
                app.handle_key_navigation(NavKey::FindPrev);
            } else {
                app.handle_key_navigation(NavKey::FindNext);
            }
            true
        }
        VK_F5 => {
            app.handle_key_navigation(NavKey::Reload);
            true
        }
        VK_RETURN => {
            app.handle_enter(shift);
            true
        }
        VK_BACK => {
            app.handle_backspace();
            true
        }
        VK_DELETE => {
            app.handle_delete();
            true
        }
        VK_ESCAPE => {
            app.handle_key_navigation(NavKey::Escape);
            true
        }
        VK_TAB => {
            if !shift && app.url_focused() {
                app.handle_key_navigation(NavKey::AcceptCompletion);
            } else {
                app.handle_tab_focus(shift);
            }
            true
        }
        VK_LEFT => {
            app.handle_caret(CaretMove::Left, shift);
            true
        }
        VK_RIGHT => {
            if app.url_focused() {
                app.handle_key_navigation(NavKey::AcceptCompletion);
            } else {
                app.handle_caret(CaretMove::Right, shift);
            }
            true
        }
        VK_UP => {
            if app.url_focused() {
                app.handle_key_navigation(NavKey::Up);
            } else {
                app.handle_caret(CaretMove::Up, shift);
            }
            true
        }
        VK_DOWN => {
            if app.url_focused() {
                app.handle_key_navigation(NavKey::Down);
            } else {
                app.handle_caret(CaretMove::Down, shift);
            }
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
            app.handle_caret(CaretMove::Home, shift);
            true
        }
        VK_END => {
            app.handle_caret(CaretMove::End, shift);
            true
        }
        _ => false,
    }
}

unsafe fn sync_title_and_invalidate<A: PlatformApp>(state_ptr: *mut WinState<A>) {
    enum InvalidateRegion {
        None,
        Full,
        Partial(BlitRect),
    }

    let (hwnd, new_title, invalidate) = {
        let state = &mut *state_ptr;
        let active = state.app.active_title();
        let title_change = if active != state.last_title {
            state.last_title = active.to_string();
            Some(format_window_title(&state.last_title))
        } else {
            None
        };
        let invalidate = if state.app.needs_repaint() {
            let (vw, vh) = state.app.viewport_size();
            match state.app.take_damage() {
                Some(dmg) => {
                    let clamped = dmg.clamp(vw, vh);
                    if clamped.is_empty() {
                        InvalidateRegion::None
                    } else {
                        state.pending_damage = merge_damage(state.pending_damage, Some(clamped));
                        match damage_to_blit_rect(clamped, vw, vh) {
                            Some(rect) => InvalidateRegion::Partial(rect),
                            None => InvalidateRegion::Full,
                        }
                    }
                }
                None => {
                    state.pending_full_repaint = true;
                    InvalidateRegion::Full
                }
            }
        } else {
            InvalidateRegion::None
        };
        (state.hwnd, title_change, invalidate)
    };

    if hwnd.is_null() {
        return;
    }

    if let Some(title) = new_title {
        let wide = to_wide_null(&title);
        SetWindowTextW(hwnd, wide.as_ptr());
    }
    match invalidate {
        InvalidateRegion::None => {}
        InvalidateRegion::Full => {
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        InvalidateRegion::Partial(rect) => {
            let rc = RECT {
                left: rect.x as i32,
                top: rect.y as i32,
                right: (rect.x + rect.width) as i32,
                bottom: (rect.y + rect.height) as i32,
            };
            InvalidateRect(hwnd, &rc, 0);
        }
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
            if let Some(mut surface) = (*state_ptr).dib_surface.take() {
                surface.destroy();
            }
            (*state_ptr).hwnd = ptr::null_mut();
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    if msg == WM_ERASEBKGND {
        return 1;
    }

    if msg == WM_DESTROY {
        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WinState<A>;
        if !state_ptr.is_null() {
            if let Some(mut surface) = (*state_ptr).dib_surface.take() {
                surface.destroy();
            }
        }
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
                    let paint_rect = rect_from_paint_rect(&ps.rc_paint, vw, vh);
                    let extra_damage = state.app.take_damage().map(|d| d.clamp(vw, vh));
                    let combined_damage =
                        merge_damage(state.pending_damage.take(), extra_damage);
                    let pending_full = mem::take(&mut state.pending_full_repaint);

                    let mut size_changed = false;
                    if !state
                        .dib_surface
                        .as_ref()
                        .is_some_and(|s| s.matches(vw, vh))
                    {
                        if let Some(mut old) = state.dib_surface.take() {
                            old.destroy();
                        }
                        state.dib_surface = DibSurface::create(hdc, vw, vh);
                        size_changed = true;
                    }

                    let app_dirty = state.app.needs_repaint();
                    let needs_render =
                        app_dirty || size_changed || pending_full || combined_damage.is_some();

                    let mut presented_dib = false;
                    if let Some(surface) = state.dib_surface.as_mut() {
                        if let Some(dst) = surface.pixels_mut() {
                            report_present_backend(
                                &mut state.app,
                                &mut state.present_backend,
                                "win32-dibsection",
                            );
                            if needs_render {
                                let preserve_existing =
                                    !size_changed && !pending_full && combined_damage.is_some();
                                let app = &mut state.app;
                                if preserve_existing {
                                    Canvas::with_external_slice_unfilled(vw, vh, dst, |canvas| {
                                        app.render_into(canvas);
                                    });
                                } else {
                                    Canvas::with_external_slice(
                                        vw,
                                        vh,
                                        dst,
                                        Color::rgb(255, 255, 255),
                                        |canvas| {
                                            app.render_into(canvas);
                                        },
                                    );
                                }
                                app.set_needs_repaint(false);
                            }

                            let damage_blit =
                                combined_damage.and_then(|d| damage_to_blit_rect(d, vw, vh));
                            let blit_target = if size_changed || pending_full {
                                clamp_rect_to_viewport(0, 0, vw, vh, vw, vh)
                            } else {
                                union_blit_rects(paint_rect, damage_blit)
                                    .or_else(|| clamp_rect_to_viewport(0, 0, vw, vh, vw, vh))
                            };
                            if let Some(rect) = blit_target {
                                presented_dib = surface.blit_rect(hdc, rect);
                            }
                        }
                    }

                    if !presented_dib {
                        report_present_backend(
                            &mut state.app,
                            &mut state.present_backend,
                            "win32-stretchdibits",
                        );
                        if needs_render
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
                    if !ch.is_control() {
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

    app.set_present_backend("win32-dibsection");
    let state = Box::new(WinState {
        app,
        canvas,
        dib_surface: None,
        pending_damage: None,
        pending_full_repaint: false,
        present_backend: Some("win32-dibsection"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_info_uses_top_down_32bpp_rgb_layout() {
        assert_eq!(mem::size_of::<BITMAPINFOHEADER>(), 40);
        assert_eq!(mem::size_of::<RGBQUAD>(), 4);
        assert_eq!(mem::size_of::<BITMAPINFO>(), 44);

        let bmi = top_down_bitmap_info(1280, 720).expect("valid 1280x720 bitmap info");
        assert_eq!(bmi.bmi_header.bi_size, 40);
        assert_eq!(bmi.bmi_header.bi_width, 1280);
        assert_eq!(bmi.bmi_header.bi_height, -720);
        assert_eq!(bmi.bmi_header.bi_planes, 1);
        assert_eq!(bmi.bmi_header.bi_bit_count, 32);
        assert_eq!(bmi.bmi_header.bi_compression, BI_RGB);
    }

    #[test]
    fn bitmap_info_rejects_zero_and_overflow_dimensions() {
        assert!(top_down_bitmap_info(0, 100).is_none());
        assert!(top_down_bitmap_info(100, 0).is_none());
        assert!(top_down_bitmap_info(i32::MAX as usize + 1, 1).is_none());
        assert!(top_down_bitmap_info(1, i32::MAX as usize + 1).is_none());
        assert!(top_down_bitmap_info(40_000, 40_000).is_none());
    }

    #[test]
    fn blit_rect_clamps_and_unions_damage_and_paint_rects() {
        let clamped = clamp_rect_to_viewport(120, 80, 400, 300, 320, 240).expect("visible slice");
        assert_eq!(
            clamped,
            BlitRect {
                x: 120,
                y: 80,
                width: 200,
                height: 160,
            }
        );
        assert!(clamp_rect_to_viewport(320, 0, 10, 10, 320, 240).is_none());
        assert!(clamp_rect_to_viewport(0, 0, 0, 10, 320, 240).is_none());

        let paint_rc = RECT {
            left: -10,
            top: 20,
            right: 50,
            bottom: 60,
        };
        let from_rc = rect_from_paint_rect(&paint_rc, 320, 240).expect("clamped paint rect");
        assert_eq!(
            from_rc,
            BlitRect {
                x: 0,
                y: 20,
                width: 50,
                height: 40,
            }
        );

        let unioned = union_blit_rects(Some(from_rc), Some(clamped)).expect("unioned rect");
        assert_eq!(
            unioned,
            BlitRect {
                x: 0,
                y: 20,
                width: 320,
                height: 220,
            }
        );
    }

    // Live CreateDIBSection / BitBlt execution requires a Windows GDI host and
    // is not exercised in Linux cross-compilation CI.
    #[test]
    fn empty_dib_surface_destroy_is_idempotent() {
        let mut surface = DibSurface {
            mem_dc: ptr::null_mut(),
            bitmap: ptr::null_mut(),
            old_bitmap: ptr::null_mut(),
            bits: ptr::null_mut(),
            width: 0,
            height: 0,
            len: 0,
        };
        assert!(surface.pixels_mut().is_none());
        unsafe {
            surface.destroy();
            surface.destroy();
        }
    }
}
