//! Cross-platform native graphical windowing backends.

pub use droplet_gfx::{Canvas, Color};

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
    GoHome,
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
    fn handle_key_navigation(&mut self, key: NavKey);
    fn render_into(&mut self, canvas: &mut Canvas);
}

/// Opens a native graphical window for the active operating system and runs its
/// blocking event loop until the window is closed.
pub fn run<A: PlatformApp>(app: A) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux_x11::run(app)
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
            "Graphical windowing is not supported on target OS '{}'; Droplet currently builds natively on Linux (X11) and Windows (Win32).",
            std::env::consts::OS
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
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
    }
}
