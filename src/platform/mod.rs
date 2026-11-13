//! Cross-platform native graphical windowing backends.

use crate::browser::Browser;

#[cfg(target_os = "linux")]
mod linux_x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

/// Opens a native graphical window for the active operating system and runs its
/// blocking event loop until the window is closed.
pub fn run(browser: Browser) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux_x11::run(browser)
    }

    #[cfg(target_os = "windows")]
    {
        windows::run(browser)
    }

    #[cfg(target_os = "macos")]
    {
        macos::run(browser)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = browser;
        Err(format!(
            "Graphical windowing is not supported on target OS '{}'; Droplet currently builds natively on Linux (X11) and Windows (Win32).",
            std::env::consts::OS
        ))
    }
}
