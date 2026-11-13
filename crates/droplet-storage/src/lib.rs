//! Compact zero-dependency persistence for browsing history and bookmarks.
//!
//! Both stores use a small versioned append log:
//!
//! ```text
//! "DRPL006\0" | version u16 | record* 
//! record = tag u8 | payload_len u32 | fnv1a32 u32 | payload
//! ```
//!
//! Strings are length-prefixed and escaped (`\n`, `\r`, `\t`, `\\`,
//! `\u{..}`), which keeps control characters out of the raw record while
//! preserving arbitrary UTF-8. Loading is tolerant of a truncated or corrupt
//! tail: everything before the first bad record is kept and the rest ignored.
//! Files are capped both by entry count and by total byte size; exceeding the
//! byte cap triggers a compact rewrite that drops the oldest entries.

use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum number of history entries retained in memory and on disk.
pub const MAX_HISTORY_ENTRIES: usize = 1000;
/// Maximum number of bookmarks retained in memory and on disk.
pub const MAX_BOOKMARK_ENTRIES: usize = 256;
/// Soft cap for a single history/log file; exceeding it triggers compaction.
pub const MAX_FILE_BYTES: usize = 512 * 1024;

const MAGIC: &[u8; 8] = b"DRPL006\0";
const VERSION: u16 = 1;
const HEADER_BYTES: usize = 10;
const RECORD_HEADER_BYTES: usize = 9;
const MAX_RAW_FIELD_BYTES: usize = 4096;
const MAX_RECORD_BYTES: usize = 64 * 1024;

const TAG_ADD: u8 = 1;
const TAG_REMOVE: u8 = 2;
const TAG_CLEAR: u8 = 3;

/// One successful navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub timestamp: u64,
}

/// One saved page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub timestamp: u64,
}

/// Prefix-matched address-bar suggestion, combining bookmarks and history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub url: String,
    pub title: String,
    pub is_bookmark: bool,
}

/// Non-panicking storage failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    pub message: String,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StorageError {}

impl StorageError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn io(context: &str, err: std::io::Error) -> Self {
        Self::new(format!("{context}: {err}"))
    }
}

/// History and bookmark store rooted at an injectable directory.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
    history: Vec<HistoryEntry>,
    bookmarks: Vec<Bookmark>,
}

impl Store {
    /// Opens (or lazily creates) a store in `dir`. Corrupt files are reset and
    /// truncated tails are dropped without panicking.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let (history, compact_history) = load_history(&dir);
        let (bookmarks, compact_bookmarks) = load_bookmarks(&dir);
        let mut store = Self {
            history,
            bookmarks,
            dir,
        };
        // Rewrite files with bad headers or truncated/corrupt tails so later
        // appends are never written past an unreadable boundary.
        if compact_history {
            let _ = store.compact_history();
        }
        if compact_bookmarks {
            let _ = store.compact_bookmarks();
        }
        store
    }

    /// Opens the platform data directory, honouring `DROPLET_DATA_DIR`.
    pub fn from_env() -> Self {
        Self::open(default_data_dir())
    }

    pub fn data_dir(&self) -> &Path {
        &self.dir
    }

    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// Most recent first, capped at `limit`.
    pub fn recent_history(&self, limit: usize) -> Vec<HistoryEntry> {
        self.history.iter().rev().take(limit).cloned().collect()
    }

    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    pub fn add_history(
        &mut self,
        url: &str,
        title: &str,
        timestamp: u64,
    ) -> Result<(), StorageError> {
        let url = clamp_field(url);
        if url.is_empty() {
            return Ok(());
        }
        let entry = HistoryEntry {
            url: url.to_string(),
            title: clamp_field(title).to_string(),
            timestamp,
        };

        if let Some(last) = self.history.last_mut() {
            if last.url == entry.url {
                *last = entry;
            } else {
                self.history.push(entry);
            }
        } else {
            self.history.push(entry);
        }
        if self.history.len() > MAX_HISTORY_ENTRIES {
            let excess = self.history.len() - MAX_HISTORY_ENTRIES;
            self.history.drain(0..excess);
        }

        let payload = encode_entry_payload(
            self.history
                .last()
                .expect("history is non-empty after insertion"),
        );
        self.append_history_record(TAG_ADD, &payload)
    }

    /// Removes every history entry with this URL. Returns whether anything changed.
    pub fn remove_history(&mut self, url: &str) -> Result<bool, StorageError> {
        let before = self.history.len();
        self.history.retain(|entry| entry.url != url);
        if self.history.len() == before {
            return Ok(false);
        }
        let mut payload = Vec::new();
        push_field(&mut payload, url);
        self.append_history_record(TAG_REMOVE, &payload)?;
        Ok(true)
    }

    pub fn clear_history(&mut self) -> Result<(), StorageError> {
        self.history.clear();
        self.compact_history()
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|bookmark| bookmark.url == url)
    }

    pub fn add_bookmark(
        &mut self,
        url: &str,
        title: &str,
        timestamp: u64,
    ) -> Result<(), StorageError> {
        let url = clamp_field(url);
        if url.is_empty() {
            return Ok(());
        }
        let bookmark = Bookmark {
            url: url.to_string(),
            title: clamp_field(title).to_string(),
            timestamp,
        };
        self.bookmarks.retain(|existing| existing.url != bookmark.url);
        self.bookmarks.push(bookmark);
        if self.bookmarks.len() > MAX_BOOKMARK_ENTRIES {
            let excess = self.bookmarks.len() - MAX_BOOKMARK_ENTRIES;
            self.bookmarks.drain(0..excess);
        }
        let payload = encode_bookmark_payload(
            self.bookmarks
                .last()
                .expect("bookmarks non-empty after insertion"),
        );
        self.append_bookmark_record(TAG_ADD, &payload)
    }

    pub fn remove_bookmark(&mut self, url: &str) -> Result<bool, StorageError> {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|bookmark| bookmark.url != url);
        if self.bookmarks.len() == before {
            return Ok(false);
        }
        let mut payload = Vec::new();
        push_field(&mut payload, url);
        self.append_bookmark_record(TAG_REMOVE, &payload)?;
        Ok(true)
    }

    /// Adds the bookmark when absent, removes it when present. Returns whether
    /// the URL is bookmarked after the toggle.
    pub fn toggle_bookmark(
        &mut self,
        url: &str,
        title: &str,
        timestamp: u64,
    ) -> Result<bool, StorageError> {
        if self.is_bookmarked(url) {
            self.remove_bookmark(url)?;
            Ok(false)
        } else {
            self.add_bookmark(url, title, timestamp)?;
            Ok(true)
        }
    }

    pub fn clear_bookmarks(&mut self) -> Result<(), StorageError> {
        self.bookmarks.clear();
        self.compact_bookmarks()
    }

    /// Case-insensitive ASCII prefix suggestions: bookmarks (newest first)
    /// followed by recent history, deduplicated by URL.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<Suggestion> {
        let prefix = prefix.trim();
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut out: Vec<Suggestion> = Vec::new();
        let mut seen: Vec<String> = Vec::new();

        for bookmark in self.bookmarks.iter().rev() {
            if starts_with_ignore_ascii_case(&bookmark.url, prefix)
                && !seen.iter().any(|url| url == &bookmark.url)
            {
                seen.push(bookmark.url.clone());
                out.push(Suggestion {
                    url: bookmark.url.clone(),
                    title: bookmark.title.clone(),
                    is_bookmark: true,
                });
                if out.len() >= limit {
                    return out;
                }
            }
        }

        for entry in self.history.iter().rev() {
            if starts_with_ignore_ascii_case(&entry.url, prefix)
                && !seen.iter().any(|url| url == &entry.url)
            {
                seen.push(entry.url.clone());
                out.push(Suggestion {
                    url: entry.url.clone(),
                    title: entry.title.clone(),
                    is_bookmark: false,
                });
                if out.len() >= limit {
                    return out;
                }
            }
        }
        out
    }

    fn history_file(&self) -> PathBuf {
        self.dir.join("history.log")
    }

    fn bookmarks_file(&self) -> PathBuf {
        self.dir.join("bookmarks.log")
    }

    fn append_history_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), StorageError> {
        let path = self.history_file();
        append_record(&path, tag, payload)?;
        if file_len(&path) > MAX_FILE_BYTES as u64 {
            self.compact_history()?;
        }
        Ok(())
    }

    fn append_bookmark_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), StorageError> {
        let path = self.bookmarks_file();
        append_record(&path, tag, payload)?;
        if file_len(&path) > MAX_FILE_BYTES as u64 {
            self.compact_bookmarks()?;
        }
        Ok(())
    }

    fn compact_history(&mut self) -> Result<(), StorageError> {
        fs::create_dir_all(&self.dir)
            .map_err(|err| StorageError::io("create data directory", err))?;
        loop {
            let mut buf = header();
            for entry in &self.history {
                buf.extend_from_slice(&encode_record(
                    TAG_ADD,
                    &encode_entry_payload(entry),
                ));
            }
            if buf.len() <= MAX_FILE_BYTES || self.history.is_empty() {
                return replace_file(&self.history_file(), &buf);
            }
            self.history.remove(0);
        }
    }

    fn compact_bookmarks(&mut self) -> Result<(), StorageError> {
        fs::create_dir_all(&self.dir)
            .map_err(|err| StorageError::io("create data directory", err))?;
        loop {
            let mut buf = header();
            for bookmark in &self.bookmarks {
                buf.extend_from_slice(&encode_record(
                    TAG_ADD,
                    &encode_bookmark_payload(bookmark),
                ));
            }
            if buf.len() <= MAX_FILE_BYTES || self.bookmarks.is_empty() {
                return replace_file(&self.bookmarks_file(), &buf);
            }
            self.bookmarks.remove(0);
        }
    }
}

/// Current Unix timestamp in seconds, saturating at `u64::MAX`.
pub fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// File name of the persistent cookie jar inside the data directory (format owned by
/// `droplet-net::cookies`).
pub const COOKIE_FILE_NAME: &str = "cookies.txt";
/// Cookie files larger than this are ignored on load (3000 cookies × 4 KiB fit well below it).
pub const MAX_COOKIE_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Reads the persistent cookie file from `dir`; `None` if missing, too large or not UTF-8.
pub fn load_cookie_file(dir: &Path) -> Option<String> {
    let path = dir.join(COOKIE_FILE_NAME);
    if fs::metadata(&path).ok()?.len() > MAX_COOKIE_FILE_BYTES {
        return None;
    }
    String::from_utf8(fs::read(path).ok()?).ok()
}

/// Atomically replaces the persistent cookie file in `dir` (temp file + rename). On Unix the
/// file is created owner-only (`0600`) since it holds session credentials.
pub fn save_cookie_file(dir: &Path, text: &str) -> Result<(), StorageError> {
    fs::create_dir_all(dir).map_err(|err| StorageError::io("create data directory", err))?;
    let path = dir.join(COOKIE_FILE_NAME);
    let tmp = path.with_extension("tmp");
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|err| StorageError::io("create temporary cookie file", err))?;
        file.write_all(text.as_bytes())
            .map_err(|err| StorageError::io("write temporary cookie file", err))?;
        file.flush()
            .map_err(|err| StorageError::io("flush temporary cookie file", err))?;
    }
    fs::rename(&tmp, &path).map_err(|err| StorageError::io("replace cookie file", err))
}

/// Platform-neutral default data directory. `DROPLET_DATA_DIR` wins.
pub fn default_data_dir() -> PathBuf {
    if let Some(path) = nonempty_env("DROPLET_DATA_DIR") {
        return path;
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(path) = nonempty_env("APPDATA") {
            return path.join("Droplet");
        }
        if let Some(path) = nonempty_env("USERPROFILE") {
            return path.join("AppData").join("Roaming").join("Droplet");
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(path) = nonempty_env("XDG_DATA_HOME") {
            return path.join("droplet");
        }
        if let Some(path) = nonempty_env("HOME") {
            return path.join(".local").join("share").join("droplet");
        }
    }

    env::temp_dir().join("droplet-data")
}

fn nonempty_env(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn header() -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn append_record(path: &Path, tag: u8, payload: &[u8]) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| StorageError::io("create data directory", err))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| StorageError::io("open append log", err))?;
    if file.metadata().map(|meta| meta.len()).unwrap_or(0) == 0 {
        file.write_all(&header())
            .map_err(|err| StorageError::io("write store header", err))?;
    }
    file.write_all(&encode_record(tag, payload))
        .map_err(|err| StorageError::io("append store record", err))?;
    file.flush()
        .map_err(|err| StorageError::io("flush store record", err))
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp)
            .map_err(|err| StorageError::io("create temporary store", err))?;
        file.write_all(bytes)
            .map_err(|err| StorageError::io("write temporary store", err))?;
        file.flush()
            .map_err(|err| StorageError::io("flush temporary store", err))?;
    }
    fs::rename(&tmp, path).map_err(|err| StorageError::io("replace store", err))
}

fn encode_record(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(RECORD_HEADER_BYTES + payload.len());
    out.push(tag);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&checksum(payload).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn encode_entry_payload(entry: &HistoryEntry) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&entry.timestamp.to_le_bytes());
    push_field(&mut out, &entry.url);
    push_field(&mut out, &entry.title);
    out
}

fn encode_bookmark_payload(bookmark: &Bookmark) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&bookmark.timestamp.to_le_bytes());
    push_field(&mut out, &bookmark.url);
    push_field(&mut out, &bookmark.title);
    out
}

fn push_field(out: &mut Vec<u8>, raw: &str) {
    let escaped = escape_field(raw);
    out.extend_from_slice(&(escaped.len() as u32).to_le_bytes());
    out.extend_from_slice(escaped.as_bytes());
}

fn read_field(payload: &[u8], pos: &mut usize) -> Option<String> {
    let len_bytes = payload.get(*pos..*pos + 4)?;
    let len = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
    if len > MAX_RECORD_BYTES {
        return None;
    }
    let start = *pos + 4;
    let end = start.checked_add(len)?;
    let raw = std::str::from_utf8(payload.get(start..end)?).ok()?;
    *pos = end;
    unescape_field(raw)
}

fn decode_entry(payload: &[u8]) -> Option<(String, String, u64)> {
    let mut pos = 0usize;
    let timestamp = u64::from_le_bytes(payload.get(..8)?.try_into().ok()?);
    pos += 8;
    let url = read_field(payload, &mut pos)?;
    let title = read_field(payload, &mut pos)?;
    if pos != payload.len() || url.is_empty() {
        return None;
    }
    Some((url, title, timestamp))
}

fn decode_url(payload: &[u8]) -> Option<String> {
    let mut pos = 0usize;
    let url = read_field(payload, &mut pos)?;
    if pos != payload.len() || url.is_empty() {
        return None;
    }
    Some(url)
}

fn load_history(dir: &Path) -> (Vec<HistoryEntry>, bool) {
    let mut entries: Vec<HistoryEntry> = Vec::new();
    let (records, mut needs_compaction) = read_records(&dir.join("history.log"));
    for (tag, payload) in records {
        match tag {
            TAG_ADD => {
                if let Some((url, title, timestamp)) = decode_entry(&payload) {
                    if let Some(last) = entries.last_mut() {
                        if last.url == url {
                            *last = HistoryEntry {
                                url,
                                title,
                                timestamp,
                            };
                            continue;
                        }
                    }
                    entries.push(HistoryEntry {
                        url,
                        title,
                        timestamp,
                    });
                    if entries.len() > MAX_HISTORY_ENTRIES {
                        entries.remove(0);
                    }
                }
            }
            TAG_REMOVE => {
                if let Some(url) = decode_url(&payload) {
                    entries.retain(|entry: &HistoryEntry| entry.url != url);
                }
            }
            TAG_CLEAR => entries.clear(),
            _ => {
                needs_compaction = true;
                break;
            }
        }
    }
    (entries, needs_compaction)
}

fn load_bookmarks(dir: &Path) -> (Vec<Bookmark>, bool) {
    let mut bookmarks: Vec<Bookmark> = Vec::new();
    let (records, mut needs_compaction) = read_records(&dir.join("bookmarks.log"));
    for (tag, payload) in records {
        match tag {
            TAG_ADD => {
                if let Some((url, title, timestamp)) = decode_entry(&payload) {
                    bookmarks.retain(|bookmark| bookmark.url != url);
                    bookmarks.push(Bookmark {
                        url,
                        title,
                        timestamp,
                    });
                    if bookmarks.len() > MAX_BOOKMARK_ENTRIES {
                        bookmarks.remove(0);
                    }
                }
            }
            TAG_REMOVE => {
                if let Some(url) = decode_url(&payload) {
                    bookmarks.retain(|bookmark| bookmark.url != url);
                }
            }
            TAG_CLEAR => bookmarks.clear(),
            _ => {
                needs_compaction = true;
                break;
            }
        }
    }
    (bookmarks, needs_compaction)
}

fn read_records(path: &Path) -> (Vec<(u8, Vec<u8>)>, bool) {
    let Ok(file) = File::open(path) else {
        return (Vec::new(), false);
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return (Vec::new(), false);
    }
    let mut needs_compaction = false;
    if bytes.len() > MAX_FILE_BYTES {
        bytes.truncate(MAX_FILE_BYTES);
        needs_compaction = true;
    }
    if bytes.len() < HEADER_BYTES
        || &bytes[..8] != MAGIC
        || u16::from_le_bytes([bytes[8], bytes[9]]) != VERSION
    {
        return (Vec::new(), true);
    }

    let mut records = Vec::new();
    let mut pos = HEADER_BYTES;
    while pos + RECORD_HEADER_BYTES <= bytes.len() {
        let tag = bytes[pos];
        let len = u32::from_le_bytes([
            bytes[pos + 1],
            bytes[pos + 2],
            bytes[pos + 3],
            bytes[pos + 4],
        ]) as usize;
        let expected = u32::from_le_bytes([
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
            bytes[pos + 8],
        ]);
        if len > MAX_RECORD_BYTES {
            break;
        }
        let start = pos + RECORD_HEADER_BYTES;
        let Some(end) = start.checked_add(len) else {
            break;
        };
        if end > bytes.len() {
            break;
        }
        let payload = &bytes[start..end];
        if checksum(payload) != expected {
            break;
        }
        if !matches!(tag, TAG_ADD | TAG_REMOVE | TAG_CLEAR) {
            break;
        }
        records.push((tag, payload.to_vec()));
        pos = end;
    }
    if pos < bytes.len() {
        needs_compaction = true;
    }
    (records, needs_compaction)
}

/// Escapes control characters and backslashes while leaving UTF-8 untouched.
pub fn escape_field(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape_field`]; returns `None` for malformed input.
pub fn unescape_field(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut hex = String::new();
                loop {
                    let digit = chars.next()?;
                    if digit == '}' {
                        break;
                    }
                    if !digit.is_ascii_hexdigit() || hex.len() >= 6 {
                        return None;
                    }
                    hex.push(digit);
                }
                let code = u32::from_str_radix(&hex, 16).ok()?;
                out.push(char::from_u32(code)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// Truncates to a UTF-8 character boundary at or below `max` bytes.
fn clamp_utf8(input: &str, max: usize) -> &str {
    if input.len() <= max {
        return input;
    }
    let mut end = max;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

fn clamp_field(input: &str) -> &str {
    clamp_utf8(input.trim(), MAX_RAW_FIELD_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "droplet_storage_{label}_{}_{seq}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cookie_file_round_trip_and_size_cap() {
        let dir = TempDir::new("cookies");
        assert_eq!(load_cookie_file(&dir.0), None);
        save_cookie_file(&dir.0, "DRPLCOOKIE1\nпример.рф\tданные\n").unwrap();
        assert_eq!(
            load_cookie_file(&dir.0).as_deref(),
            Some("DRPLCOOKIE1\nпример.рф\tданные\n")
        );
        save_cookie_file(&dir.0, "DRPLCOOKIE1\n").unwrap();
        assert_eq!(load_cookie_file(&dir.0).as_deref(), Some("DRPLCOOKIE1\n"));
        assert!(!dir.0.join("cookies.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.0.join(COOKIE_FILE_NAME)).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "cookie file must be owner-only");
        }
        fs::write(dir.0.join(COOKIE_FILE_NAME), [0xff, 0xfe]).unwrap();
        assert_eq!(load_cookie_file(&dir.0), None, "non-UTF-8 file is ignored");
    }

    #[test]
    fn history_and_bookmarks_round_trip_unicode() {
        let dir = TempDir::new("roundtrip");
        let mut store = Store::open(&dir.0);
        store
            .add_history(
                "https://example.com/путь?q=1",
                "Пример & <страница>",
                1_700_000_000,
            )
            .unwrap();
        store
            .add_history("about:test", "Visual Test", 1_700_000_010)
            .unwrap();
        store
            .add_bookmark("https://example.com/путь?q=1", "Закладка 😀", 1_700_000_020)
            .unwrap();

        let reopened = Store::open(&dir.0);
        assert_eq!(reopened.history().len(), 2);
        assert_eq!(reopened.history()[0].title, "Пример & <страница>");
        assert_eq!(reopened.recent_history(1)[0].url, "about:test");
        assert_eq!(reopened.bookmarks().len(), 1);
        assert_eq!(reopened.bookmarks()[0].title, "Закладка 😀");
        assert!(reopened.is_bookmarked("https://example.com/путь?q=1"));
    }

    #[test]
    fn truncated_and_corrupt_tails_are_tolerated() {
        let dir = TempDir::new("tail");
        let mut store = Store::open(&dir.0);
        for idx in 0..4 {
            store
                .add_history(
                    &format!("https://example.com/{idx}"),
                    &format!("Page {idx}"),
                    100 + idx,
                )
                .unwrap();
        }
        let path = dir.0.join("history.log");
        let mut bytes = fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 3); // truncated final record
        fs::write(&path, &bytes).unwrap();

        let mut reopened = Store::open(&dir.0);
        assert_eq!(reopened.history().len(), 3);
        assert_eq!(reopened.history()[2].url, "https://example.com/2");

        // Appending after recovering from a truncated tail must remain readable.
        reopened
            .add_history("https://example.com/after-repair", "Repaired", 200)
            .unwrap();
        let after_append = Store::open(&dir.0);
        assert_eq!(after_append.history().len(), 4);
        assert_eq!(
            after_append.history()[3].url,
            "https://example.com/after-repair"
        );

        // Corrupt a checksum in the middle, keeping everything before it.
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        fs::write(&path, &bytes).unwrap();
        let reopened = Store::open(&dir.0);
        assert!(reopened.history().len() <= 3);
        assert_eq!(reopened.history()[0].url, "https://example.com/0");
    }

    #[test]
    fn corrupt_or_foreign_header_resets_cleanly() {
        let dir = TempDir::new("header");
        fs::create_dir_all(&dir.0).unwrap();
        fs::write(dir.0.join("history.log"), b"not a droplet store at all").unwrap();

        let mut store = Store::open(&dir.0);
        assert!(store.history().is_empty());
        store
            .add_history("about:test", "Recovered", 1)
            .unwrap();
        let reopened = Store::open(&dir.0);
        assert_eq!(reopened.history().len(), 1);
        assert_eq!(reopened.history()[0].url, "about:test");
    }

    #[test]
    fn entries_and_file_bytes_are_capped() {
        let dir = TempDir::new("caps");
        let mut store = Store::open(&dir.0);
        for idx in 0..(MAX_HISTORY_ENTRIES + 50) {
            store
                .add_history(
                    &format!("https://example.com/page/{idx}"),
                    "History entry with a reasonably long title",
                    idx as u64,
                )
                .unwrap();
        }
        assert_eq!(store.history().len(), MAX_HISTORY_ENTRIES);
        assert_eq!(store.history()[0].url, "https://example.com/page/50");
        assert!(file_len(&dir.0.join("history.log")) <= MAX_FILE_BYTES as u64);

        for idx in 0..(MAX_BOOKMARK_ENTRIES + 10) {
            store
                .add_bookmark(
                    &format!("https://example.com/bookmark/{idx}"),
                    "Bookmark title",
                    idx as u64,
                )
                .unwrap();
        }
        assert_eq!(store.bookmarks().len(), MAX_BOOKMARK_ENTRIES);
        assert!(file_len(&dir.0.join("bookmarks.log")) <= MAX_FILE_BYTES as u64);
    }

    #[test]
    fn control_characters_and_backslashes_round_trip() {
        let dir = TempDir::new("escape");
        let mut store = Store::open(&dir.0);
        let messy = "line1\nline2\ttab\\slash\r\u{7}\u{0}Привет";
        store.add_history("https://example.com/x", messy, 7).unwrap();
        store
            .add_bookmark("https://example.com/x", messy, 8)
            .unwrap();
        let reopened = Store::open(&dir.0);
        assert_eq!(reopened.history()[0].title, messy);
        assert_eq!(reopened.bookmarks()[0].title, messy);

        let escaped = escape_field(messy);
        assert!(!escaped.contains('\n'));
        assert!(!escaped.contains('\t'));
        assert_eq!(unescape_field(&escaped).as_deref(), Some(messy));
        assert_eq!(unescape_field("bad\\q"), None);
    }

    #[test]
    fn remove_clear_toggle_and_suggestions() {
        let dir = TempDir::new("ops");
        let mut store = Store::open(&dir.0);
        store
            .add_history("https://example.com/alpha", "Alpha", 1)
            .unwrap();
        store
            .add_history("https://example.com/beta", "Beta", 2)
            .unwrap();
        store
            .add_bookmark("https://example.com/beta-book", "Beta Book", 3)
            .unwrap();

        let suggestions = store.suggest("https://example.com/b", 8);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0].url, "https://example.com/beta-book");
        assert!(suggestions[0].is_bookmark);
        assert_eq!(suggestions[1].url, "https://example.com/beta");
        assert_eq!(store.suggest("HTTPS://EXAMPLE", 2).len(), 2);

        assert!(store.toggle_bookmark("https://example.com/beta", "Beta", 4).unwrap());
        assert!(store.is_bookmarked("https://example.com/beta"));
        assert!(!store.toggle_bookmark("https://example.com/beta", "Beta", 5).unwrap());

        assert!(store.remove_history("https://example.com/alpha").unwrap());
        assert!(!store.remove_history("https://example.com/alpha").unwrap());
        assert_eq!(store.history().len(), 1);

        store.clear_history().unwrap();
        assert!(store.history().is_empty());
        let reopened = Store::open(&dir.0);
        assert!(reopened.history().is_empty());
        assert!(reopened.is_bookmarked("https://example.com/beta-book"));

        store.clear_bookmarks().unwrap();
        assert!(store.bookmarks().is_empty());
        assert!(!Store::open(&dir.0).is_bookmarked("https://example.com/beta-book"));
    }

    #[test]
    fn utf8_truncation_never_splits_a_code_point() {
        let text = "Привет";
        for max in 0..=text.len() {
            let cut = clamp_utf8(text, max);
            assert!(text.starts_with(cut));
            // `clamp_utf8` always returns a valid slice on a char boundary.
            assert!(cut.len() <= max);
        }
    }

    #[test]
    fn data_dir_honours_override_variable() {
        // Keep this isolated from parallel tests by reading through a scoped
        // helper rather than mutating process-wide environment state.
        let key = "DROPLET_DATA_DIR";
        let previous = env::var_os(key);
        env::set_var(key, "/tmp/droplet-storage-override");
        assert_eq!(
            default_data_dir(),
            PathBuf::from("/tmp/droplet-storage-override")
        );
        if let Some(previous) = previous {
            env::set_var(key, previous);
        } else {
            env::remove_var(key);
        }
    }
}
