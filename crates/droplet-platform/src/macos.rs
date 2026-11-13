//! macOS graphical windowing backend stub.
//!
//! Planned future implementation using native Cocoa/AppKit (`NSApplication`,
//! `NSWindow`, and a backing `CGImage`/`NSView` bitmap surface) via Objective-C
//! runtime FFI without external crate dependencies.

use crate::PlatformApp;

/// Runs the macOS window backend.
///
/// Currently unimplemented; returns an informative error directing callers to
/// supported platforms or headless CLI modes.
pub fn run<A: PlatformApp>(_app: A) -> Result<(), String> {
    Err("macOS Cocoa/AppKit backend is planned for a future ticket; Droplet currently builds natively on Linux (X11) and Windows (Win32).".into())
}
