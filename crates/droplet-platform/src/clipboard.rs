//! Zero-dependency system clipboard access.
//!
//! Linux uses the X11 cut buffer (`XStoreBytes`/`XFetchBytes`) through the same
//! lazy `dlopen` approach as the window backend. Windows uses the Win32
//! clipboard APIs. All text is sanitized so stray NULs can never truncate an
//! OS clipboard payload.

/// Removes NUL bytes and normalizes line endings to `\n`.
pub fn sanitize_text(text: &str) -> String {
    text.replace('\0', "").replace("\r\n", "\n").replace('\r', "\n")
}

/// Normalizes text for a single-line input field.
pub fn sanitize_single_line(text: &str) -> String {
    sanitize_text(text)
        .chars()
        .map(|ch| if ch == '\n' { ' ' } else { ch })
        .collect()
}

/// Reads UTF-8 text from the system clipboard.
pub fn get_text() -> Option<String> {
    let text = platform_get_text()?;
    Some(sanitize_text(&text))
}

/// Stores UTF-8 text on the system clipboard. Returns `false` on failure.
pub fn set_text(text: &str) -> bool {
    platform_set_text(&sanitize_text(text))
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::ptr;

    const RTLD_LAZY: c_int = 1;

    #[link(name = "c")]
    extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }

    #[repr(C)]
    struct Display {
        _private: [u8; 0],
    }

    type XStoreBytes = unsafe extern "C" fn(*mut Display, *const c_char, c_int) -> c_int;
    type XFetchBytes = unsafe extern "C" fn(*mut Display, *mut c_int) -> *mut c_char;
    type XFree = unsafe extern "C" fn(*mut c_void) -> c_int;

    struct XApi {
        handle: *mut c_void,
        open_display: unsafe extern "C" fn(*const c_char) -> *mut Display,
        close_display: unsafe extern "C" fn(*mut Display) -> c_int,
        store_bytes: XStoreBytes,
        fetch_bytes: XFetchBytes,
        free: XFree,
    }

    impl Drop for XApi {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    dlclose(self.handle);
                }
            }
        }
    }

    unsafe fn load_sym<T: Copy>(handle: *mut c_void, name: &CStr) -> Option<T> {
        let ptr = dlsym(handle, name.as_ptr());
        if ptr.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
        }
    }

    impl XApi {
        fn load() -> Option<Self> {
            let handle = unsafe {
                let h = dlopen(c"libX11.so.6".as_ptr(), RTLD_LAZY);
                if h.is_null() {
                    dlopen(c"libX11.so".as_ptr(), RTLD_LAZY)
                } else {
                    h
                }
            };
            if handle.is_null() {
                return None;
            }
            let api = unsafe {
                Some(Self {
                    handle,
                    open_display: load_sym(handle, c"XOpenDisplay")?,
                    close_display: load_sym(handle, c"XCloseDisplay")?,
                    store_bytes: load_sym(handle, c"XStoreBytes")?,
                    fetch_bytes: load_sym(handle, c"XFetchBytes")?,
                    free: load_sym(handle, c"XFree")?,
                })
            };
            if api.is_none() {
                unsafe {
                    dlclose(handle);
                }
            }
            api
        }
    }

    struct DisplayGuard<'a> {
        api: &'a XApi,
        display: *mut Display,
    }

    impl Drop for DisplayGuard<'_> {
        fn drop(&mut self) {
            if !self.display.is_null() {
                unsafe {
                    (self.api.close_display)(self.display);
                }
            }
        }
    }

    pub(super) fn get_text() -> Option<String> {
        let api = XApi::load()?;
        let display = unsafe { (api.open_display)(ptr::null()) };
        if display.is_null() {
            return None;
        }
        let _guard = DisplayGuard {
            api: &api,
            display,
        };
        let mut len: c_int = 0;
        let data = unsafe { (api.fetch_bytes)(display, &mut len) };
        if data.is_null() || len <= 0 {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len as usize) };
        let text = String::from_utf8_lossy(bytes).into_owned();
        unsafe {
            (api.free)(data as *mut c_void);
        }
        Some(text)
    }

    pub(super) fn set_text(text: &str) -> bool {
        let Some(api) = XApi::load() else {
            return false;
        };
        let display = unsafe { (api.open_display)(ptr::null()) };
        if display.is_null() {
            return false;
        }
        let _guard = DisplayGuard {
            api: &api,
            display,
        };
        let Ok(c_text) = CString::new(text) else {
            return false;
        };
        unsafe {
            (api.store_bytes)(display, c_text.as_ptr(), text.len() as c_int) >= 0
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ptr;

    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: *mut std::ffi::c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> *mut std::ffi::c_void;
        fn SetClipboardData(format: u32, data: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut std::ffi::c_void;
        fn GlobalLock(handle: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn GlobalUnlock(handle: *mut std::ffi::c_void) -> i32;
        fn GlobalFree(handle: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    pub(super) fn get_text() -> Option<String> {
        unsafe {
            if OpenClipboard(ptr::null_mut()) == 0 {
                return None;
            }
            let handle = GetClipboardData(CF_UNICODETEXT);
            if handle.is_null() {
                CloseClipboard();
                return None;
            }
            let ptr = GlobalLock(handle) as *const u16;
            if ptr.is_null() {
                CloseClipboard();
                return None;
            }
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
                if len > 1_048_576 {
                    break;
                }
            }
            let units = std::slice::from_raw_parts(ptr, len);
            GlobalUnlock(handle);
            CloseClipboard();
            Some(String::from_utf16_lossy(units))
        }
    }

    pub(super) fn set_text(text: &str) -> bool {
        let wide = crate::to_wide_null(text);
        let bytes = wide.len() * std::mem::size_of::<u16>();
        unsafe {
            if OpenClipboard(ptr::null_mut()) == 0 {
                return false;
            }
            let _ = EmptyClipboard();
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if handle.is_null() {
                CloseClipboard();
                return false;
            }
            let dst = GlobalLock(handle) as *mut u16;
            if dst.is_null() {
                GlobalFree(handle);
                CloseClipboard();
                return false;
            }
            ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            GlobalUnlock(handle);
            if SetClipboardData(CF_UNICODETEXT, handle).is_null() {
                GlobalFree(handle);
                CloseClipboard();
                return false;
            }
            CloseClipboard();
            true
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    pub(super) fn get_text() -> Option<String> {
        None
    }

    pub(super) fn set_text(_text: &str) -> bool {
        false
    }
}

fn platform_get_text() -> Option<String> {
    imp::get_text()
}

fn platform_set_text(text: &str) -> bool {
    imp::set_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_text_removes_nul_and_normalizes_newlines() {
        assert_eq!(sanitize_text("a\0b"), "ab");
        assert_eq!(sanitize_text("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(sanitize_text("Привет\0 мир"), "Привет мир");
    }

    #[test]
    fn sanitize_single_line_flattens_newlines() {
        assert_eq!(sanitize_single_line("a\nb\r\nc"), "a b c");
        assert_eq!(sanitize_single_line("Привет\nмир"), "Привет мир");
    }

    #[test]
    fn clipboard_round_trip_helpers_preserve_utf8_and_never_panic_headless() {
        let mut backing = String::new();
        let payload = "Строка 1\r\nСтрока 2\0!";
        let stored = sanitize_text(payload);
        backing.push_str(&stored);
        assert_eq!(backing, "Строка 1\nСтрока 2!");
        assert_eq!(sanitize_single_line(&backing), "Строка 1 Строка 2!");

        // Live OS clipboard calls must remain safe even when headless.
        let _ = set_text("test");
        let _ = get_text();
    }
}
