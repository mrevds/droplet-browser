//! `droplet-webview`: out-of-process WebKitGTK host for script-heavy sites (DROP-170).
//!
//! Droplet renders simple documents with its own engine. Sites that need a full web
//! platform (JS apps, MSE video, WebSocket, IndexedDB) are handed to this helper, which
//! hosts one WebKitGTK view. GTK 3, WebKitGTK 4.1 and GLib are loaded with `dlopen` at
//! runtime, so the workspace keeps zero crate dependencies and Droplet still starts on a
//! machine without WebKit (it then renders everything with its own engine).
//!
//! Usage: `droplet-webview [--embed XID] [--data-dir DIR] URL`
//!
//! * `--embed XID`: the view lives in a `GtkPlug` inside the given X11 window (XEmbed);
//!   GTK is forced onto its X11 backend. Without it a plain toplevel window opens.
//! * stdin, one command per line: `load URL`, `back`, `forward`, `reload`, `stop`,
//!   `resize W H`, `quit`. EOF on stdin (the browser went away) quits.
//! * stdout, one event per line: `uri URL`, `title TEXT`, `load started|committed|finished`,
//!   `progress 0..1`, `ready XID`.

#[cfg(target_os = "linux")]
mod gtk;

#[cfg(target_os = "linux")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match Options::parse(&args) {
        Ok(opts) => opts,
        Err(message) => {
            eprintln!("droplet-webview: {message}");
            eprintln!("usage: droplet-webview [--embed XID] [--data-dir DIR] URL");
            std::process::exit(2);
        }
    };
    if let Err(message) = gtk::run(opts) {
        eprintln!("droplet-webview: {message}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("droplet-webview: WebKitGTK host is only available on Linux");
    std::process::exit(1);
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub embed: Option<u64>,
    pub data_dir: Option<String>,
    pub url: String,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut embed = None;
        let mut data_dir = None;
        let mut url = None;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--embed" => {
                    let xid = it.next().ok_or("--embed needs an X11 window id")?;
                    embed = Some(parse_xid(xid).ok_or_else(|| format!("bad window id {xid}"))?);
                }
                "--data-dir" => {
                    data_dir = Some(it.next().ok_or("--data-dir needs a path")?.clone());
                }
                other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
                other => url = Some(other.to_string()),
            }
        }
        Ok(Self {
            embed,
            data_dir,
            url: url.unwrap_or_else(|| "about:blank".to_string()),
        })
    }
}

fn parse_xid(text: &str) -> Option<u64> {
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// A command read from stdin.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Load(String),
    Back,
    Forward,
    Reload,
    Stop,
    Resize(i32, i32),
    Quit,
}

impl Command {
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        Some(match word {
            "load" if !rest.is_empty() => Command::Load(rest.to_string()),
            "back" => Command::Back,
            "forward" => Command::Forward,
            "reload" => Command::Reload,
            "stop" => Command::Stop,
            "quit" => Command::Quit,
            "resize" => {
                let (w, h) = rest.split_once(' ')?;
                Command::Resize(w.trim().parse().ok()?, h.trim().parse().ok()?)
            }
            _ => return None,
        })
    }
}

/// Splits complete lines off `buf`, leaving a trailing partial line in place.
pub fn take_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
        return Vec::new();
    };
    let rest = buf.split_off(last_nl + 1);
    let complete = std::mem::replace(buf, rest);
    String::from_utf8_lossy(&complete)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// Event text is one line: newlines inside titles/URLs become spaces.
pub fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_options() {
        let opts = Options::parse(&args(&["--embed", "0x4a00007", "https://ютуб.рф/"])).unwrap();
        assert_eq!(opts.embed, Some(0x4a00007));
        assert_eq!(opts.url, "https://ютуб.рф/");
        assert_eq!(opts.data_dir, None);
        let opts = Options::parse(&args(&["--data-dir", "/x", "--embed", "123"])).unwrap();
        assert_eq!(opts.embed, Some(123));
        assert_eq!(opts.data_dir.as_deref(), Some("/x"));
        assert_eq!(opts.url, "about:blank");
        assert!(Options::parse(&args(&["--embed"])).is_err());
        assert!(Options::parse(&args(&["--embed", "zz"])).is_err());
        assert!(Options::parse(&args(&["--bogus"])).is_err());
    }

    #[test]
    fn parses_commands() {
        assert_eq!(
            Command::parse("load https://пример.рф/?q=тест"),
            Some(Command::Load("https://пример.рф/?q=тест".into()))
        );
        assert_eq!(Command::parse("  back "), Some(Command::Back));
        assert_eq!(
            Command::parse("resize 800 600"),
            Some(Command::Resize(800, 600))
        );
        assert_eq!(Command::parse("resize 800"), None);
        assert_eq!(Command::parse("load"), None);
        assert_eq!(Command::parse("dance"), None);
        assert_eq!(Command::parse("quit"), Some(Command::Quit));
    }

    #[test]
    fn take_lines_keeps_partial_tail() {
        let mut buf = b"back\nload https://\xd1\x8f".to_vec();
        assert_eq!(take_lines(&mut buf), vec!["back".to_string()]);
        assert_eq!(buf, b"load https://\xd1\x8f");
        buf.extend_from_slice(b".ru\n\nquit\n");
        assert_eq!(
            take_lines(&mut buf),
            vec!["load https://я.ru".to_string(), "quit".to_string()]
        );
        assert!(buf.is_empty());
        assert!(take_lines(&mut buf).is_empty());
    }

    #[test]
    fn one_line_strips_newlines() {
        assert_eq!(one_line("Привет\nмир\r"), "Привет мир ");
    }
}
