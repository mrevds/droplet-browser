//! Generated `about:history` and `about:bookmarks` pages.
//!
//! Kept in a dedicated module so the core `about.rs` table only needs two
//! extra match arms. All user-controlled text is HTML-escaped.

use std::fmt::Write as _;

use droplet_storage::{Bookmark, HistoryEntry};

use crate::about::{about_nav_html, ABOUT_BASE_CSS};

/// Stable generated page for `about:history`.
pub fn history_page(entries: &[HistoryEntry]) -> String {
    let mut rows = String::new();
    if entries.is_empty() {
        rows.push_str(
            "<div class=\"card\"><p>No history yet. Visit a page and it will appear here.</p></div>",
        );
    } else {
        for entry in entries {
            push_row(
                &mut rows,
                &entry.url,
                &entry.title,
                entry.timestamp,
                "history",
                "Remove",
            );
        }
    }
    let clear = if entries.is_empty() {
        String::new()
    } else {
        "<p><a class=\"btn\" href=\"about:history?clear=1\">Clear history</a></p>".to_string()
    };
    page_shell(
        "Droplet &mdash; History",
        "History",
        &format!(
            "<p class=\"meta\">{} stored navigation{} (most recent first).</p>{clear}",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        ),
        &rows,
        "history",
    )
}

/// Stable generated page for `about:bookmarks`.
pub fn bookmarks_page(bookmarks: &[Bookmark]) -> String {
    let mut rows = String::new();
    if bookmarks.is_empty() {
        rows.push_str(
            "<div class=\"card\"><p>No bookmarks yet. Press <code>Ctrl+D</code> on any page.</p></div>",
        );
    } else {
        for bookmark in bookmarks {
            push_row(
                &mut rows,
                &bookmark.url,
                &bookmark.title,
                bookmark.timestamp,
                "bookmarks",
                "Remove",
            );
        }
    }
    page_shell(
        "Droplet &mdash; Bookmarks",
        "Bookmarks",
        &format!(
            "<p class=\"meta\">{} saved page{}.</p>",
            bookmarks.len(),
            if bookmarks.len() == 1 { "" } else { "s" }
        ),
        &rows,
        "bookmarks",
    )
}

fn page_shell(title: &str, heading: &str, summary: &str, rows: &str, active: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>{title}</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>{heading}</h1>
    {summary}
    {rows}
    <p><a href="about:droplet">&larr; Back to about:droplet</a> | <a href="about:{other}">{other_label}</a></p>
  </div>
</body>
</html>"#,
        nav = about_nav_html(active),
        other = if active == "history" { "bookmarks" } else { "history" },
        other_label = if active == "history" {
            "Bookmarks"
        } else {
            "History"
        },
    )
}

fn push_row(
    out: &mut String,
    url: &str,
    title: &str,
    timestamp: u64,
    page: &str,
    action: &str,
) {
    let display_title = if title.trim().is_empty() { url } else { title };
    let remove_href = format!("about:{page}?remove={}", percent_encode(url));
    let _ = write!(
        out,
        "<div class=\"card\">\
         <p><a href=\"{url}\">{title}</a></p>\
         <p class=\"meta\">{time} &middot; {url_text}</p>\
         <p><a href=\"{remove}\">{action}</a></p>\
         </div>",
        url = escape_html(url),
        title = escape_html(display_title),
        time = format_timestamp(timestamp),
        url_text = escape_html(url),
        remove = escape_html(&remove_href),
    );
}

/// Formats a Unix timestamp as `YYYY-MM-DD HH:MM UTC` without dependencies.
pub fn format_timestamp(timestamp: u64) -> String {
    let days = (timestamp / 86_400) as i64;
    let seconds = timestamp % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3600,
        (seconds % 3600) / 60
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    // Howard Hinnant's civil-from-days algorithm.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// Escapes text for safe interpolation into HTML text or attribute values.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Percent-encodes arbitrary UTF-8 bytes for an `about:` query value.
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Decodes a percent-encoded UTF-8 query value. Invalid escapes are kept
/// literally; invalid UTF-8 returns `None`.
pub fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' && idx + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[idx + 1]), hex_value(bytes[idx + 2])) {
                out.push((high << 4) | low);
                idx += 3;
                continue;
            }
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, title: &str, timestamp: u64) -> HistoryEntry {
        HistoryEntry {
            url: url.to_string(),
            title: title.to_string(),
            timestamp,
        }
    }

    #[test]
    fn timestamp_formatting_matches_known_dates() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(format_timestamp(951_782_400), "2000-02-29 00:00 UTC");
    }

    #[test]
    fn escaping_and_percent_round_trip_unicode() {
        let raw = "https://example.com/путь?q=\"<&>\"";
        assert_eq!(percent_decode(&percent_encode(raw)).as_deref(), Some(raw));
        let escaped = escape_html(raw);
        assert!(escaped.contains("&quot;"));
        assert!(escaped.contains("&amp;"));
        assert!(escaped.contains("&lt;"));
        assert!(!escaped.contains('<'));
    }

    #[test]
    fn history_page_escapes_titles_and_renders_remove_links() {
        let entries = vec![
            entry(
                "https://example.com/a?x=1&y=2",
                "<script>alert('x')</script>",
                1_700_000_000,
            ),
            entry("about:test", "Visual & Test", 1_700_000_010),
        ];
        let page = history_page(&entries);
        assert!(page.contains("History"));
        assert!(page.contains("2023-11-14 22:13 UTC"));
        assert!(page.contains("https://example.com/a?x=1&amp;y=2"));
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(page.contains("about:history?remove=https%3A%2F%2Fexample.com%2Fa%3Fx%3D1%26y%3D2"));
        assert!(page.contains("about:history?clear=1"));
        // Most recent first input is preserved as given.
        assert!(page.rfind("about:test").unwrap() > page.find("example.com").unwrap());
    }

    #[test]
    fn bookmarks_page_handles_empty_state() {
        let page = bookmarks_page(&[]);
        assert!(page.contains("No bookmarks yet"));
        assert!(page.contains("about:history"));
        assert!(!page.contains("?remove="));
        assert!(bookmarks_page(&[Bookmark {
            url: "about:test".into(),
            title: "Test".into(),
            timestamp: 0,
        }])
        .contains("about:bookmarks?remove=about%3Atest"));
    }
}
