//! Cross-platform native graphical windowing backends.

pub mod clipboard;

pub use droplet_gfx::{Canvas, Color, DamageRect};

/// Thread-safe "wake the event loop" callback handed to the app by a backend that can be
/// woken from other threads. After it fires, the backend calls [`PlatformApp::poll_background`]
/// on the UI thread.
pub type Waker = std::sync::Arc<dyn Fn() + Send + Sync>;
pub use clipboard::{get_text as clipboard_get_text, set_text as clipboard_set_text};

#[cfg(target_os = "linux")]
mod linux_wake;

#[cfg(target_os = "linux")]
mod linux_wayland;

#[cfg(target_os = "linux")]
mod linux_x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

/// High-level navigation and tab/window actions dispatched by platform backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    PageUp,
    PageDown,
    Up,
    Down,
    Home,
    End,
    Back,
    Forward,
    Reload,
    FocusUrl,
    Escape,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    /// Switch to the zero-based tab index (Ctrl+1..9); out-of-range is ignored.
    SwitchTab(usize),
    GoHome,
    ToggleBookmark,
    Find,
    FindNext,
    FindPrev,
    AcceptCompletion,
    ToggleInspector,
}

/// A foreign native window (an out-of-process web view, DROP-170) that the backend shows
/// over part of the browser window. On X11 `handle` is the client's XEmbed window id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedView {
    pub handle: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Caret movement requested from a native keyboard backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretMove {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// Application interface driven by native OS windowing backends.
pub trait PlatformApp {
    fn viewport_size(&self) -> (usize, usize);
    fn url_focused(&self) -> bool;
    fn needs_repaint(&self) -> bool;
    fn set_needs_repaint(&mut self, dirty: bool);
    fn active_title(&self) -> &str;
    fn resize(&mut self, width: usize, height: usize);
    fn scroll(&mut self, delta_y: f32);
    fn handle_mouse_move(&mut self, x: f32, y: f32);
    fn handle_mouse_down(&mut self, x: f32, y: f32);
    fn handle_mouse_up(&mut self, x: f32, y: f32);
    fn handle_text_input(&mut self, ch: char);
    fn handle_backspace(&mut self);
    fn handle_submit_url(&mut self);
    /// Whether printable keys should be treated as text input (address bar or
    /// find bar) instead of page-level shortcuts such as space-to-page-down.
    fn wants_text_input(&self) -> bool {
        self.url_focused()
    }
    /// Enter/Shift+Enter dispatch. Default submits the address bar when focused.
    fn handle_enter(&mut self, _shift: bool) {
        if self.url_focused() {
            self.handle_submit_url();
        }
    }
    fn handle_key_navigation(&mut self, key: NavKey);
    fn render_into(&mut self, canvas: &mut Canvas);

    /// Moves keyboard focus to the next/previous form control (Tab/Shift+Tab).
    fn handle_tab_focus(&mut self, _backwards: bool) {}
    /// Moves the text caret; `extend` grows the selection (Shift+Arrow).
    fn handle_caret(&mut self, _movement: CaretMove, _extend: bool) {}
    /// Deletes the character after the caret (Delete key).
    fn handle_delete(&mut self) {}
    /// Activates the focused control (Enter/Space).
    fn handle_activate(&mut self) {}
    fn handle_select_all(&mut self) {}
    fn handle_copy(&mut self) {}
    fn handle_cut(&mut self) {}
    fn handle_paste(&mut self) {}
    /// Returns the dirty region since the last render, or `None` when a full repaint is needed.
    fn take_damage(&mut self) -> Option<DamageRect> {
        None
    }
    /// Informs the app which presentation backend (`"shm"`, `"dmabuf"`, `"jpeg"`, etc.) is active.
    fn set_present_backend(&mut self, _backend: &'static str) {}

    /// Called once by backends that can be woken from other threads (X11, Wayland, Win32).
    /// Apps that never receive a waker must do all work synchronously.
    fn set_waker(&mut self, _waker: Waker) {}

    /// Called once, before [`PlatformApp::set_waker`], by backends that can host foreign
    /// windows: the native handle of the browser window (X11 `Window`), for `--embed`.
    fn set_embed_parent(&mut self, _handle: u64) {}
    /// The foreign view to show right now (active tab), or `None` to hide any.
    /// Polled after every event batch; the backend maps, moves and focuses it.
    fn embedded_view(&self) -> Option<EmbeddedView> {
        None
    }

    /// Called on the UI thread after the waker fired: apply finished background work.
    /// Returns `true` if anything changed (the backend then repaints if `needs_repaint`).
    fn poll_background(&mut self) -> bool {
        false
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn env_var_nonempty(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

#[cfg(target_os = "linux")]
pub(crate) fn wayland_env_present() -> bool {
    env_var_nonempty("WAYLAND_DISPLAY") && env_var_nonempty("XDG_RUNTIME_DIR")
}

/// Pure fallback decision: Wayland is attempted only when both Wayland
/// environment variables are present and the client library loaded.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn wayland_candidate(
    wayland_display_present: bool,
    xdg_runtime_dir_present: bool,
    library_available: bool,
) -> bool {
    wayland_display_present && xdg_runtime_dir_present && library_available
}

/// Backend choice on Linux. X11 (also through XWayland) is the maintained backend and the
/// only one that can embed WebKit views (DROP-170), so it wins whenever `$DISPLAY` is set;
/// the frozen Wayland backend is used without X or when `DROPLET_BACKEND=wayland`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn prefer_wayland(backend_env: Option<&str>, display_present: bool) -> bool {
    match backend_env {
        Some("wayland") => true,
        Some("x11") => false,
        _ => !display_present,
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn backend_error_message(wayland_error: Option<&str>, x11_error: &str) -> String {
    match wayland_error {
        Some(wayland_error) => format!(
            "Wayland backend unavailable ({wayland_error}); X11 fallback also failed ({x11_error})"
        ),
        None => format!("X11 backend failed ({x11_error})"),
    }
}

/// Opens a native graphical window for the active operating system and runs its
/// blocking event loop until the window is closed.
#[allow(unused_mut)]
pub fn run<A: PlatformApp>(mut app: A) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let candidate = prefer_wayland(
            std::env::var("DROPLET_BACKEND").ok().as_deref(),
            env_var_nonempty("DISPLAY"),
        ) && wayland_candidate(
            env_var_nonempty("WAYLAND_DISPLAY"),
            env_var_nonempty("XDG_RUNTIME_DIR"),
            linux_wayland::library_available(),
        );
        let mut wayland_error = None;
        if candidate {
            match linux_wayland::run_with(&mut app) {
                Ok(()) => return Ok(()),
                Err(linux_wayland::RunError::Runtime(message)) => {
                    return Err(format!(
                        "Wayland backend failed after initialization: {message}"
                    ));
                }
                Err(linux_wayland::RunError::Init(message)) => {
                    wayland_error = Some(message);
                }
            }
        }

        match linux_x11::run(&mut app) {
            Ok(()) => Ok(()),
            Err(x11_error) => Err(backend_error_message(wayland_error.as_deref(), &x11_error)),
        }
    }

    #[cfg(target_os = "windows")]
    {
        windows::run(app)
    }

    #[cfg(target_os = "macos")]
    {
        macos::run(app)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = app;
        Err(format!(
            "Graphical windowing is not supported on target OS '{}'; Droplet currently builds natively on Linux (Wayland/X11) and Windows (Win32).",
            std::env::consts::OS
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos", test))]
pub(crate) fn format_window_title(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| if c == '\0' { ' ' } else { c })
        .collect();
    if clean.is_empty() {
        "Droplet".to_string()
    } else {
        format!("{clean} - Droplet")
    }
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn to_wide_null(s: &str) -> Vec<u16> {
    let mut wide = Vec::with_capacity(s.len() + 1);
    let mut buf = [0u16; 2];
    for ch in s.chars() {
        let ch = if ch == '\0' { ' ' } else { ch };
        wide.extend_from_slice(ch.encode_utf16(&mut buf));
    }
    wide.push(0);
    wide
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn decode_utf16_unit(pending_surrogate: &mut Option<u16>, unit: u16) -> Option<char> {
    if (0xD800..=0xDBFF).contains(&unit) {
        *pending_surrogate = Some(unit);
        return None;
    }
    if (0xDC00..=0xDFFF).contains(&unit) {
        if let Some(hi) = pending_surrogate.take() {
            let cp = 0x10000 + (((u32::from(hi) - 0xD800) << 10) | (u32::from(unit) - 0xDC00));
            return char::from_u32(cp);
        }
        return None;
    }
    *pending_surrogate = None;
    char::from_u32(u32::from(unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_preferred_when_display_present() {
        assert!(!prefer_wayland(None, true)); // GNOME/KDE Wayland session with XWayland
        assert!(prefer_wayland(None, false)); // pure Wayland, no X server
        assert!(prefer_wayland(Some("wayland"), true));
        assert!(!prefer_wayland(Some("x11"), false));
        assert!(!prefer_wayland(Some("bogus"), true));
    }

    #[test]
    fn format_window_title_handles_empty_unicode_and_nul() {
        assert_eq!(format_window_title(""), "Droplet");
        assert_eq!(format_window_title("Example"), "Example - Droplet");
        assert_eq!(format_window_title("Привет"), "Привет - Droplet");
        assert_eq!(format_window_title("A\0B"), "A B - Droplet");
    }

    #[test]
    fn to_wide_null_encodes_ascii_nul_cyrillic_and_surrogates() {
        assert_eq!(to_wide_null(""), vec![0]);
        assert_eq!(
            to_wide_null("Hi"),
            vec![u16::from(b'H'), u16::from(b'i'), 0]
        );
        assert_eq!(
            to_wide_null("a\0b"),
            vec![u16::from(b'a'), u16::from(b' '), u16::from(b'b'), 0]
        );
        assert_eq!(to_wide_null("Ж"), vec![0x0416, 0]);
        assert_eq!(to_wide_null("😀"), vec![0xD83D, 0xDE00, 0]);
    }

    #[test]
    fn decode_utf16_unit_handles_bmp_surrogate_pairs_and_edge_cases() {
        let mut pending = None;
        assert_eq!(decode_utf16_unit(&mut pending, u16::from(b'A')), Some('A'));
        assert_eq!(pending, None);

        assert_eq!(decode_utf16_unit(&mut pending, 0x0416), Some('Ж'));
        assert_eq!(pending, None);

        // Lone low surrogate without preceding high surrogate is ignored.
        assert_eq!(decode_utf16_unit(&mut pending, 0xDC00), None);
        assert_eq!(pending, None);

        // Valid surrogate pair -> U+1F600 ('😀').
        assert_eq!(decode_utf16_unit(&mut pending, 0xD83D), None);
        assert_eq!(pending, Some(0xD83D));
        assert_eq!(decode_utf16_unit(&mut pending, 0xDE00), Some('😀'));
        assert_eq!(pending, None);

        // Consecutive high surrogates overwrite pending high surrogate.
        assert_eq!(decode_utf16_unit(&mut pending, 0xD83D), None);
        assert_eq!(decode_utf16_unit(&mut pending, 0xD83E), None);
        assert_eq!(pending, Some(0xD83E));
        assert_eq!(decode_utf16_unit(&mut pending, 0xDD14), Some('🤔'));
        assert_eq!(pending, None);

        // High surrogate followed by BMP char clears stale pending surrogate.
        assert_eq!(decode_utf16_unit(&mut pending, 0xD83D), None);
        assert_eq!(pending, Some(0xD83D));
        assert_eq!(decode_utf16_unit(&mut pending, u16::from(b'x')), Some('x'));
        assert_eq!(pending, None);
    }

    struct MockApp {
        width: usize,
        height: usize,
        focused: bool,
        dirty: bool,
        title: String,
        scroll_sum: f32,
        last_move: (f32, f32),
        last_down: (f32, f32),
        last_up: (f32, f32),
        typed: String,
        backspaces: usize,
        submits: usize,
        keys: Vec<NavKey>,
        renders: usize,
    }

    impl PlatformApp for MockApp {
        fn viewport_size(&self) -> (usize, usize) {
            (self.width, self.height)
        }
        fn url_focused(&self) -> bool {
            self.focused
        }
        fn needs_repaint(&self) -> bool {
            self.dirty
        }
        fn set_needs_repaint(&mut self, dirty: bool) {
            self.dirty = dirty;
        }
        fn active_title(&self) -> &str {
            &self.title
        }
        fn resize(&mut self, width: usize, height: usize) {
            self.width = width;
            self.height = height;
            self.dirty = true;
        }
        fn scroll(&mut self, delta_y: f32) {
            self.scroll_sum += delta_y;
            self.dirty = true;
        }
        fn handle_mouse_move(&mut self, x: f32, y: f32) {
            self.last_move = (x, y);
        }
        fn handle_mouse_down(&mut self, x: f32, y: f32) {
            self.last_down = (x, y);
        }
        fn handle_mouse_up(&mut self, x: f32, y: f32) {
            self.last_up = (x, y);
        }
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
            self.keys.push(key);
        }
        fn render_into(&mut self, canvas: &mut Canvas) {
            assert_eq!(canvas.width, self.width);
            assert_eq!(canvas.height, self.height);
            self.renders += 1;
        }
    }

    #[test]
    fn platform_app_trait_surface_works_with_canvas() {
        let mut app = MockApp {
            width: 800,
            height: 600,
            focused: true,
            dirty: false,
            title: "Home".to_string(),
            scroll_sum: 0.0,
            last_move: (0.0, 0.0),
            last_down: (0.0, 0.0),
            last_up: (0.0, 0.0),
            typed: String::new(),
            backspaces: 0,
            submits: 0,
            keys: Vec::new(),
            renders: 0,
        };

        assert_eq!(app.viewport_size(), (800, 600));
        assert!(app.url_focused());
        assert!(!app.needs_repaint());
        assert_eq!(app.active_title(), "Home");

        app.resize(1024, 768);
        assert!(app.needs_repaint());
        app.scroll(48.0);
        app.handle_mouse_move(10.0, 20.0);
        app.handle_mouse_down(12.0, 22.0);
        app.handle_mouse_up(14.0, 24.0);
        app.handle_text_input('a');
        app.handle_backspace();
        app.handle_submit_url();
        app.handle_key_navigation(NavKey::NewTab);

        let mut canvas = Canvas::new(1024, 768, Color::rgb(255, 255, 255));
        app.render_into(&mut canvas);
        app.set_needs_repaint(false);

        assert!(!app.needs_repaint());
        assert!((app.scroll_sum - 48.0).abs() < f32::EPSILON);
        assert!((app.last_move.0 - 10.0).abs() < f32::EPSILON);
        assert!((app.last_move.1 - 20.0).abs() < f32::EPSILON);
        assert!((app.last_down.0 - 12.0).abs() < f32::EPSILON);
        assert!((app.last_down.1 - 22.0).abs() < f32::EPSILON);
        assert!((app.last_up.0 - 14.0).abs() < f32::EPSILON);
        assert!((app.last_up.1 - 24.0).abs() < f32::EPSILON);
        assert_eq!(app.typed, "a");
        assert_eq!(app.backspaces, 1);
        assert_eq!(app.submits, 1);
        assert_eq!(app.keys, vec![NavKey::NewTab]);
        assert_eq!(app.renders, 1);
        assert_eq!(app.take_damage(), None);
        app.set_present_backend("mock");
    }

    #[test]
    fn platform_app_damage_hook_supports_partial_damage() {
        struct DamagedApp(Option<DamageRect>);
        impl PlatformApp for DamagedApp {
            fn viewport_size(&self) -> (usize, usize) {
                (320, 240)
            }
            fn url_focused(&self) -> bool {
                false
            }
            fn needs_repaint(&self) -> bool {
                self.0.is_some()
            }
            fn set_needs_repaint(&mut self, dirty: bool) {
                if !dirty {
                    self.0 = None;
                }
            }
            fn active_title(&self) -> &str {
                "Damaged"
            }
            fn resize(&mut self, _width: usize, _height: usize) {}
            fn scroll(&mut self, _delta_y: f32) {}
            fn handle_mouse_move(&mut self, _x: f32, _y: f32) {}
            fn handle_mouse_down(&mut self, _x: f32, _y: f32) {}
            fn handle_mouse_up(&mut self, _x: f32, _y: f32) {}
            fn handle_text_input(&mut self, _ch: char) {}
            fn handle_backspace(&mut self) {}
            fn handle_submit_url(&mut self) {}
            fn handle_key_navigation(&mut self, _key: NavKey) {}
            fn render_into(&mut self, _canvas: &mut Canvas) {}
            fn take_damage(&mut self) -> Option<DamageRect> {
                self.0.take()
            }
        }

        let rect = DamageRect::new(10, 20, 100, 50);
        let mut app = DamagedApp(Some(rect));
        assert_eq!(app.take_damage(), Some(rect));
        assert_eq!(app.take_damage(), None);
    }

    #[test]
    fn wayland_candidate_requires_display_runtime_dir_and_library() {
        for display in [false, true] {
            for runtime in [false, true] {
                for library in [false, true] {
                    let expected = display && runtime && library;
                    assert_eq!(
                        wayland_candidate(display, runtime, library),
                        expected,
                        "mismatch for ({display}, {runtime}, {library})"
                    );
                }
            }
        }
    }

    #[test]
    fn backend_error_message_formats_with_and_without_wayland_reason() {
        assert_eq!(
            backend_error_message(None, "no $DISPLAY"),
            "X11 backend failed (no $DISPLAY)"
        );
        assert_eq!(
            backend_error_message(Some("missing compositor"), "no $DISPLAY"),
            "Wayland backend unavailable (missing compositor); X11 fallback also failed (no $DISPLAY)"
        );
    }

    #[test]
    fn env_var_nonempty_rejects_unset_variables() {
        assert!(!env_var_nonempty("DROPLET_NONEXISTENT_WAYLAND_VAR_98765"));
    }
}
