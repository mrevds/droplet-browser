//! Out-of-process web views for script-heavy sites (DROP-170).
//!
//! Droplet renders documents with its own engine. Sites that need the full web platform
//! (YouTube, mail, messengers, ...) open in a `droplet-webview` helper process that hosts
//! WebKitGTK and embeds itself into the browser window (see `apps/droplet-desktop/src/bin/droplet-webview`).
//! The tab keeps a [`WebView`]; the platform backend shows the helper's window over the
//! content area ([`droplet_platform::EmbeddedView`]). Dropping the [`WebView`] ends the
//! process, which is how hibernation frees a heavy tab's memory.
//!
//! Only `std::process` is used here; the helper protocol is plain text lines.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::OnceLock;
use std::time::Duration;

use droplet_platform::Waker;
use droplet_url::{Scheme, Url};

/// Sites that need a full web platform. A host matches itself and its subdomains.
const BUILTIN_SITES: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "google.com",
    "gmail.com",
    "telegram.org",
    "whatsapp.com",
    "vk.com",
    "ok.ru",
    "yandex.ru",
    "ya.ru",
    "x.com",
    "twitter.com",
    "instagram.com",
    "facebook.com",
    "discord.com",
    "spotify.com",
    "twitch.tv",
    "reddit.com",
    "github.com",
    "notion.so",
    "figma.com",
];

/// Event reported by the helper on its stdout.
#[derive(Debug, Clone, PartialEq)]
pub enum WebViewEvent {
    /// The helper's XEmbed window id: the view can now be shown.
    Ready(u64),
    Uri(String),
    Title(String),
    Started,
    Committed,
    Finished,
    Progress(f32),
    /// The helper process ended (crash, killed, or quit).
    Exited,
}

pub fn parse_event(line: &str) -> Option<WebViewEvent> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    Some(match word {
        "ready" => {
            let hex = rest.trim().trim_start_matches("0x");
            WebViewEvent::Ready(u64::from_str_radix(hex, 16).ok()?)
        }
        "uri" => WebViewEvent::Uri(rest.to_string()),
        "title" => WebViewEvent::Title(rest.to_string()),
        "load" => match rest.trim() {
            "started" => WebViewEvent::Started,
            "committed" => WebViewEvent::Committed,
            "finished" => WebViewEvent::Finished,
            _ => return None,
        },
        "progress" => WebViewEvent::Progress(rest.trim().parse().ok()?),
        _ => return None,
    })
}

fn host_matches(host: &str, site: &str) -> bool {
    let host = host.trim_end_matches('.');
    host.eq_ignore_ascii_case(site)
        || (host.len() > site.len()
            && host.as_bytes()[host.len() - site.len() - 1] == b'.'
            && host[host.len() - site.len()..].eq_ignore_ascii_case(site))
}

/// User list: one host per line in `$XDG_CONFIG_HOME/droplet/webkit-sites`
/// (`~/.config/...`); `#` starts a comment; a line `*` routes every http(s) site.
fn user_sites() -> &'static [String] {
    static SITES: OnceLock<Vec<String>> = OnceLock::new();
    SITES.get_or_init(|| {
        let dir = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        let Some(dir) = dir else {
            return Vec::new();
        };
        std::fs::read_to_string(dir.join("droplet").join("webkit-sites"))
            .map(|text| parse_site_list(&text))
            .unwrap_or_default()
    })
}

pub fn parse_site_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| {
            l.split('#')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// Whether `url` should open in a WebKit view rather than Droplet's own engine.
pub fn wants_webview(url: &Url) -> bool {
    wants_webview_with(
        url,
        user_sites(),
        std::env::var("DROPLET_WEBKIT").ok().as_deref(),
    )
}

pub fn wants_webview_with(url: &Url, user: &[String], env: Option<&str>) -> bool {
    if !matches!(url.scheme, Scheme::Http | Scheme::Https) {
        return false;
    }
    match env {
        Some("all") => return true,
        Some("off") => return false,
        _ => {}
    }
    user.iter().any(|s| s == "*" || host_matches(&url.host, s))
        || BUILTIN_SITES.iter().any(|s| host_matches(&url.host, s))
}

/// Path of the helper binary: `$DROPLET_WEBVIEW`, else next to the running executable.
fn helper_path() -> PathBuf {
    if let Some(path) = std::env::var_os("DROPLET_WEBVIEW").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let name = if cfg!(windows) {
        "droplet-webview.exe"
    } else {
        "droplet-webview"
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// One running helper process.
pub struct WebView {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    events: Receiver<WebViewEvent>,
    /// Native window to embed, once the helper reported it.
    pub handle: Option<u64>,
    pub loading: bool,
}

impl std::fmt::Debug for WebView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebView")
            .field("handle", &self.handle)
            .field("loading", &self.loading)
            .finish_non_exhaustive()
    }
}

impl WebView {
    /// Starts a helper embedded into the native window `parent`, loading `url`.
    pub fn spawn(parent: u64, url: &str, waker: Option<Waker>) -> std::io::Result<Self> {
        let mut child = Command::new(helper_path())
            .arg("--embed")
            .arg(parent.to_string())
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stdin = child.stdin.take();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("droplet-webview-events".to_string())
            .spawn(move || {
                let wake = || {
                    if let Some(w) = &waker {
                        w();
                    }
                };
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Some(event) = parse_event(&line) {
                        if tx.send(event).is_err() {
                            return;
                        }
                        wake();
                    }
                }
                let _ = tx.send(WebViewEvent::Exited);
                wake();
            })?;
        Ok(Self {
            child: Some(child),
            stdin,
            events: rx,
            handle: None,
            loading: true,
        })
    }

    fn send(&mut self, command: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let ok = stdin
                .write_all(command.as_bytes())
                .and_then(|_| stdin.write_all(b"\n"))
                .and_then(|_| stdin.flush());
            if ok.is_err() {
                self.stdin = None;
            }
        }
    }

    pub fn load(&mut self, url: &str) {
        // A URL never contains a raw newline after parsing, but never let one split a command.
        let url = url.replace(['\n', '\r'], "");
        self.send(&format!("load {url}"));
    }

    pub fn back(&mut self) {
        self.send("back");
    }

    pub fn forward(&mut self) {
        self.send("forward");
    }

    pub fn reload(&mut self) {
        self.send("reload");
    }

    pub fn stop(&mut self) {
        self.send("stop");
    }

    /// Takes every event received so far, updating `handle`/`loading` on the way.
    pub fn drain(&mut self) -> Vec<WebViewEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            match event {
                WebViewEvent::Ready(handle) => self.handle = Some(handle),
                WebViewEvent::Started => self.loading = true,
                WebViewEvent::Finished => self.loading = false,
                WebViewEvent::Exited => {
                    self.handle = None;
                    self.loading = false;
                }
                _ => {}
            }
            out.push(event);
        }
        out
    }
}

impl Drop for WebView {
    fn drop(&mut self) {
        self.send("quit");
        self.stdin = None; // EOF also makes the helper quit
        if let Some(mut child) = self.child.take() {
            // Give WebKit a moment to flush cookies/storage, then make sure it is gone.
            let _ = std::thread::Builder::new()
                .name("droplet-webview-reaper".to_string())
                .spawn(move || {
                    for _ in 0..30 {
                        if matches!(child.try_wait(), Ok(Some(_))) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn routes_heavy_sites_and_subdomains() {
        let none: &[String] = &[];
        assert!(wants_webview_with(
            &url("https://www.youtube.com/watch?v=x"),
            none,
            None
        ));
        assert!(wants_webview_with(
            &url("https://mail.google.com/"),
            none,
            None
        ));
        assert!(wants_webview_with(
            &url("https://web.telegram.org/k/"),
            none,
            None
        ));
        assert!(wants_webview_with(&url("https://YouTube.com/"), none, None));
        assert!(!wants_webview_with(
            &url("https://notyoutube.com/"),
            none,
            None
        ));
        assert!(!wants_webview_with(
            &url("https://ru.wikipedia.org/wiki/Тест"),
            none,
            None
        ));
        assert!(!wants_webview_with(&url("about:droplet"), none, None));
    }

    #[test]
    fn env_and_user_list_override() {
        let none: &[String] = &[];
        let wiki = url("https://ru.wikipedia.org/wiki/Привет");
        assert!(wants_webview_with(&wiki, none, Some("all")));
        assert!(!wants_webview_with(
            &url("https://youtube.com/"),
            none,
            Some("off")
        ));
        let user = parse_site_list("# мои сайты\nwikipedia.org  # вики\n\n");
        assert_eq!(user, vec!["wikipedia.org".to_string()]);
        assert!(wants_webview_with(&wiki, &user, None));
        let star = parse_site_list("*");
        assert!(wants_webview_with(&url("http://example.com/"), &star, None));
        assert!(!wants_webview_with(&url("about:blank"), &star, None));
    }

    #[test]
    fn parses_helper_events() {
        assert_eq!(
            parse_event("ready 0x4a00007"),
            Some(WebViewEvent::Ready(0x4a00007))
        );
        assert_eq!(
            parse_event("title Видео — YouTube"),
            Some(WebViewEvent::Title("Видео — YouTube".into()))
        );
        assert_eq!(
            parse_event("title "),
            Some(WebViewEvent::Title(String::new()))
        );
        assert_eq!(
            parse_event("uri https://www.youtube.com/"),
            Some(WebViewEvent::Uri("https://www.youtube.com/".into()))
        );
        assert_eq!(parse_event("load finished"), Some(WebViewEvent::Finished));
        assert_eq!(
            parse_event("progress 0.50"),
            Some(WebViewEvent::Progress(0.5))
        );
        assert_eq!(parse_event("load redirected"), None);
        assert_eq!(parse_event("ready zz"), None);
        assert_eq!(parse_event("garbage"), None);
    }
}
