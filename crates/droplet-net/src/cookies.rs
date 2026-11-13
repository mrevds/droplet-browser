//! HTTP cookies: an RFC 6265 subset with a bounded jar (DROP-113).
//!
//! * `Set-Cookie` parsing per RFC 6265 §5.2: `Expires` (cookie-date algorithm of §5.1.1),
//!   `Max-Age` (wins over `Expires`), `Domain`, `Path`, `Secure`, `HttpOnly`, `SameSite`.
//! * Storage model per §5.3 with two RFC 6265bis hardenings: a `Secure` cookie can only be
//!   set from `https`, and a plain-`http` response cannot overwrite an existing `Secure` one.
//! * `Domain` must domain-match the request host; single-label domains (`com`, `рф`) are
//!   rejected unless they equal the host itself (then the cookie is host-only).
//! * Hosts and domains are compared after Unicode lowercasing (the URL parser only lowercases
//!   ASCII), so `ПРИМЕР.РФ` and `пример.рф` share cookies. No punycode conversion.
//! * Hard caps: [`MAX_COOKIES_PER_DOMAIN`], [`MAX_COOKIES_TOTAL`], [`MAX_COOKIE_BYTES`]; expired
//!   cookies are dropped first, then the least recently used one.
//! * Session cookies live only in memory. Persistent ones are saved as a small text file through
//!   `droplet-storage` (`cookies.txt` next to history) when [`attach_store`] was called.
//!
//! The process-wide jar is behind a `Mutex`; network code uses [`header_for`] before a request
//! and [`store_response_cookies`] after each response (redirect hops included).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use droplet_storage::{escape_field, now_timestamp, unescape_field};
use droplet_url::{Scheme, Url};

/// Maximum cookies stored for one cookie domain.
pub const MAX_COOKIES_PER_DOMAIN: usize = 50;
/// Maximum cookies stored in the whole jar.
pub const MAX_COOKIES_TOTAL: usize = 3000;
/// Maximum `name` + `value` bytes of a single cookie; larger ones are ignored.
pub const MAX_COOKIE_BYTES: usize = 4096;
/// First line of the persistent cookie file.
const FILE_MAGIC: &str = "DRPLCOOKIE1";
/// Number of tab-separated fields per cookie line in the persistent file.
const FILE_FIELDS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Unset,
    Lax,
    Strict,
    None,
}

impl SameSite {
    fn as_str(self) -> &'static str {
        match self {
            SameSite::Unset => "",
            SameSite::Lax => "lax",
            SameSite::Strict => "strict",
            SameSite::None => "none",
        }
    }

    fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("lax") {
            SameSite::Lax
        } else if value.eq_ignore_ascii_case("strict") {
            SameSite::Strict
        } else if value.eq_ignore_ascii_case("none") {
            SameSite::None
        } else {
            SameSite::Unset
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Canonical (lowercase, no leading/trailing dot) cookie domain.
    pub domain: String,
    /// Sent only to exactly `domain` (no `Domain` attribute was given).
    pub host_only: bool,
    pub path: String,
    /// Unix seconds; `None` for a session cookie.
    pub expires: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    /// Unix seconds of the first creation (kept across overwrites, used for send order).
    pub creation: u64,
    /// Jar tick of the last store or send, for LRU eviction.
    last_access: u64,
}

impl Cookie {
    fn is_expired(&self, now: u64) -> bool {
        self.expires.is_some_and(|t| t <= now)
    }
}

/// Bounded in-memory cookie store, bucketed by cookie domain.
#[derive(Debug, Default)]
pub struct CookieJar {
    by_domain: HashMap<String, Vec<Cookie>>,
    total: usize,
    tick: u64,
    /// The persistent subset changed since the last [`CookieJar::take_dirty`].
    dirty: bool,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// All stored cookies, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &Cookie> {
        self.by_domain.values().flatten()
    }

    /// Returns whether persistent cookies changed since the last call, and resets the flag.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// Applies one `Set-Cookie` header value received from `url`. Returns `true` when the
    /// jar changed (a cookie stored, replaced or deleted); malformed or rejected headers
    /// return `false` and change nothing.
    pub fn set_from_header(&mut self, url: &Url, header: &str, now: u64) -> bool {
        if !matches!(url.scheme, Scheme::Http | Scheme::Https) {
            return false;
        }
        let Some(parsed) = parse_set_cookie(header, now) else {
            return false;
        };
        let host = canonical_host(&url.host);
        if host.is_empty() {
            return false;
        }
        let https = url.scheme == Scheme::Https;

        let (domain, host_only) = match parsed.domain {
            Some(d) if !d.contains('.') => {
                // Public-suffix-ish single label: only acceptable as the host itself.
                if d != host {
                    return false;
                }
                (host, true)
            }
            Some(d) => {
                if !domain_match(&host, &d) {
                    return false;
                }
                (d, false)
            }
            None => (host, true),
        };
        if parsed.secure && !https {
            return false;
        }
        let path = parsed.path.unwrap_or_else(|| default_path(&url.path).to_string());

        let tick = self.next_tick();
        let bucket = self.by_domain.entry(domain.clone()).or_default();
        let existing = bucket
            .iter()
            .position(|c| c.name == parsed.name && c.path == path);
        let mut creation = now;
        if let Some(idx) = existing {
            if bucket[idx].secure && !https {
                return false;
            }
            let old = bucket.swap_remove(idx);
            self.total -= 1;
            if old.expires.is_some() {
                self.dirty = true;
            }
            creation = old.creation;
        }

        if parsed.expires.is_some_and(|t| t <= now) {
            // An expiry in the past is how servers delete cookies.
            if bucket.is_empty() {
                self.by_domain.remove(&domain);
            }
            return existing.is_some();
        }

        if parsed.expires.is_some() {
            self.dirty = true;
        }
        bucket.push(Cookie {
            name: parsed.name,
            value: parsed.value,
            domain: domain.clone(),
            host_only,
            path,
            expires: parsed.expires,
            secure: parsed.secure,
            http_only: parsed.http_only,
            same_site: parsed.same_site,
            creation,
            last_access: tick,
        });
        self.total += 1;
        self.enforce_limits(&domain, now);
        true
    }

    /// Builds the `Cookie` request header value for `url`, or `None` when nothing applies.
    /// Sent cookies count as used for LRU eviction.
    pub fn cookie_header(&mut self, url: &Url, now: u64) -> Option<String> {
        if self.total == 0 || !matches!(url.scheme, Scheme::Http | Scheme::Https) {
            return None;
        }
        let host = canonical_host(&url.host);
        let https = url.scheme == Scheme::Https;
        let path = if url.path.is_empty() { "/" } else { url.path.as_str() };
        let tick = self.next_tick();

        // (path length, creation, domain key, index) of every matching cookie.
        let mut hits: Vec<(usize, u64, String, usize)> = Vec::new();
        let ip = is_ip_host(&host);
        let mut candidate = host.as_str();
        loop {
            if let Some(bucket) = self.by_domain.get(candidate) {
                for (idx, c) in bucket.iter().enumerate() {
                    if (c.host_only && c.domain != host)
                        || (c.secure && !https)
                        || c.is_expired(now)
                        || !path_match(path, &c.path)
                    {
                        continue;
                    }
                    hits.push((c.path.len(), c.creation, candidate.to_string(), idx));
                }
            }
            if ip {
                break;
            }
            match candidate.find('.') {
                Some(dot) => candidate = &candidate[dot + 1..],
                None => break,
            }
        }
        if hits.is_empty() {
            return None;
        }
        // RFC 6265 §5.4: longer paths first, then earlier creation.
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut out = String::new();
        for (_, _, domain, idx) in &hits {
            let Some(c) = self.by_domain.get_mut(domain).and_then(|b| b.get_mut(*idx)) else {
                continue;
            };
            c.last_access = tick;
            if !out.is_empty() {
                out.push_str("; ");
            }
            out.push_str(&c.name);
            out.push('=');
            out.push_str(&c.value);
        }
        Some(out)
    }

    /// Drops expired cookies, then evicts least recently used ones past the caps.
    fn enforce_limits(&mut self, domain: &str, now: u64) {
        let over_domain = self
            .by_domain
            .get(domain)
            .is_some_and(|b| b.len() > MAX_COOKIES_PER_DOMAIN);
        if over_domain {
            self.purge_expired_in(domain, now);
            while let Some(bucket) = self.by_domain.get_mut(domain) {
                if bucket.len() <= MAX_COOKIES_PER_DOMAIN {
                    break;
                }
                let lru = lru_index(bucket);
                let old = bucket.swap_remove(lru);
                self.total -= 1;
                self.dirty |= old.expires.is_some();
            }
        }
        if self.total > MAX_COOKIES_TOTAL {
            let keys: Vec<String> = self.by_domain.keys().cloned().collect();
            for key in keys {
                self.purge_expired_in(&key, now);
            }
            while self.total > MAX_COOKIES_TOTAL {
                let Some((key, idx)) = self
                    .by_domain
                    .iter()
                    .filter(|(_, b)| !b.is_empty())
                    .map(|(k, b)| {
                        let i = lru_index(b);
                        (b[i].last_access, k, i)
                    })
                    .min()
                    .map(|(_, k, i)| (k.clone(), i))
                else {
                    break;
                };
                if let Some(bucket) = self.by_domain.get_mut(&key) {
                    let old = bucket.swap_remove(idx);
                    self.total -= 1;
                    self.dirty |= old.expires.is_some();
                    if bucket.is_empty() {
                        self.by_domain.remove(&key);
                    }
                }
            }
        }
    }

    fn purge_expired_in(&mut self, domain: &str, now: u64) {
        if let Some(bucket) = self.by_domain.get_mut(domain) {
            let before = bucket.len();
            bucket.retain(|c| !c.is_expired(now));
            let removed = before - bucket.len();
            self.total -= removed;
            self.dirty |= removed > 0;
            if bucket.is_empty() {
                self.by_domain.remove(domain);
            }
        }
    }

    /// Serializes the persistent (non-expired, non-session) cookies.
    ///
    /// ```text
    /// DRPLCOOKIE1
    /// domain \t host_only \t path \t name \t value \t expires \t secure \t http_only \t samesite \t creation
    /// ```
    /// Text fields use `droplet_storage::escape_field` (UTF-8 kept, controls escaped);
    /// flags are `0`/`1`, times are Unix seconds.
    pub fn to_file_string(&self, now: u64) -> String {
        let mut out = String::from(FILE_MAGIC);
        out.push('\n');
        for c in self.iter() {
            let Some(expires) = c.expires else { continue };
            if expires <= now {
                continue;
            }
            let fields = [
                escape_field(&c.domain),
                flag(c.host_only).to_string(),
                escape_field(&c.path),
                escape_field(&c.name),
                escape_field(&c.value),
                expires.to_string(),
                flag(c.secure).to_string(),
                flag(c.http_only).to_string(),
                c.same_site.as_str().to_string(),
                c.creation.to_string(),
            ];
            out.push_str(&fields.join("\t"));
            out.push('\n');
        }
        out
    }

    /// Loads cookies written by [`CookieJar::to_file_string`]. Malformed lines, expired
    /// cookies and files with a wrong header are skipped; caps still apply.
    /// Returns the number of cookies loaded.
    pub fn load_file_string(&mut self, text: &str, now: u64) -> usize {
        let mut lines = text.lines();
        if lines.next() != Some(FILE_MAGIC) {
            return 0;
        }
        let mut loaded = 0;
        for line in lines {
            let Some(cookie) = parse_file_line(line, now) else {
                continue;
            };
            let domain = cookie.domain.clone();
            let tick = self.next_tick();
            let bucket = self.by_domain.entry(domain.clone()).or_default();
            if let Some(idx) = bucket
                .iter()
                .position(|c| c.name == cookie.name && c.path == cookie.path)
            {
                bucket.swap_remove(idx);
                self.total -= 1;
            }
            bucket.push(Cookie {
                last_access: tick,
                ..cookie
            });
            self.total += 1;
            loaded += 1;
            self.enforce_limits(&domain, now);
        }
        loaded
    }
}

fn flag(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

fn lru_index(bucket: &[Cookie]) -> usize {
    bucket
        .iter()
        .enumerate()
        .min_by_key(|(_, c)| c.last_access)
        .map_or(0, |(i, _)| i)
}

fn parse_file_line(line: &str, now: u64) -> Option<Cookie> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != FILE_FIELDS {
        return None;
    }
    let bool_field = |s: &str| match s {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    };
    let domain = unescape_field(fields[0])?;
    let name = unescape_field(fields[3])?;
    let value = unescape_field(fields[4])?;
    let expires: u64 = fields[5].parse().ok()?;
    if domain.is_empty()
        || name.is_empty()
        || expires <= now
        || name.len() + value.len() > MAX_COOKIE_BYTES
        || has_forbidden_chars(&name)
        || has_forbidden_chars(&value)
    {
        return None;
    }
    let path = unescape_field(fields[2])?;
    if !path.starts_with('/') {
        return None;
    }
    Some(Cookie {
        name,
        value,
        domain: canonical_host(&domain),
        host_only: bool_field(fields[1])?,
        path,
        expires: Some(expires),
        secure: bool_field(fields[6])?,
        http_only: bool_field(fields[7])?,
        same_site: SameSite::parse(fields[8]),
        creation: fields[9].parse().ok()?,
        last_access: 0,
    })
}

/// Attributes of one parsed `Set-Cookie` header, before the storage model is applied.
#[derive(Debug, PartialEq, Eq)]
struct SetCookie {
    name: String,
    value: String,
    expires: Option<u64>,
    domain: Option<String>,
    path: Option<String>,
    secure: bool,
    http_only: bool,
    same_site: SameSite,
}

fn trim_ws(s: &str) -> &str {
    s.trim_matches(|c| c == ' ' || c == '\t')
}

/// CTLs other than HTAB are not allowed in names/values (and never CR/LF/NUL).
fn has_forbidden_chars(s: &str) -> bool {
    s.chars()
        .any(|c| (c.is_ascii_control() && c != '\t') || c == ';')
}

/// RFC 6265 §5.2 `Set-Cookie` parsing. `None` means "ignore the header entirely".
fn parse_set_cookie(header: &str, now: u64) -> Option<SetCookie> {
    let (pair, attrs) = header.split_once(';').unwrap_or((header, ""));
    let (name, value) = pair.split_once('=')?;
    let name = trim_ws(name);
    let value = trim_ws(value);
    if name.is_empty()
        || has_forbidden_chars(name)
        || has_forbidden_chars(value)
        || name.len() + value.len() > MAX_COOKIE_BYTES
    {
        return None;
    }

    let mut cookie = SetCookie {
        name: name.to_string(),
        value: value.to_string(),
        expires: None,
        domain: None,
        path: None,
        secure: false,
        http_only: false,
        same_site: SameSite::Unset,
    };
    let mut expires_attr = None;
    let mut max_age_attr = None;
    for attr in attrs.split(';') {
        let (key, val) = attr.split_once('=').unwrap_or((attr, ""));
        let key = trim_ws(key);
        let val = trim_ws(val);
        if key.eq_ignore_ascii_case("expires") {
            if let Some(t) = parse_cookie_date(val) {
                expires_attr = Some(t);
            }
        } else if key.eq_ignore_ascii_case("max-age") {
            if let Some(t) = parse_max_age(val, now) {
                max_age_attr = Some(t);
            }
        } else if key.eq_ignore_ascii_case("domain") {
            let d = val.trim_start_matches('.');
            if !d.is_empty() {
                cookie.domain = Some(canonical_host(d));
            }
        } else if key.eq_ignore_ascii_case("path") {
            cookie.path = val.starts_with('/').then(|| val.to_string());
        } else if key.eq_ignore_ascii_case("secure") {
            cookie.secure = true;
        } else if key.eq_ignore_ascii_case("httponly") {
            cookie.http_only = true;
        } else if key.eq_ignore_ascii_case("samesite") {
            cookie.same_site = SameSite::parse(val);
        }
    }
    cookie.expires = max_age_attr.or(expires_attr);
    Some(cookie)
}

/// `Max-Age`: a delta in seconds; zero or negative means "already expired" (time 0).
fn parse_max_age(val: &str, now: u64) -> Option<u64> {
    let digits = val.strip_prefix('-').unwrap_or(val);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if val.starts_with('-') {
        return Some(0);
    }
    // Saturate absurdly large deltas instead of rejecting them.
    let delta: u64 = digits.parse().unwrap_or(u64::MAX);
    if delta == 0 {
        return Some(0);
    }
    Some(now.saturating_add(delta))
}

/// Unicode-lowercased host without trailing dots (`ПРИМЕР.РФ.` → `пример.рф`).
fn canonical_host(host: &str) -> String {
    host.trim_end_matches('.').to_lowercase()
}

fn is_ip_host(host: &str) -> bool {
    host.starts_with('[')
        || host.contains(':')
        || (!host.is_empty() && host.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
}

/// RFC 6265 §5.1.3 domain matching (both arguments canonical).
fn domain_match(host: &str, domain: &str) -> bool {
    if host == domain {
        return true;
    }
    host.len() > domain.len()
        && host.ends_with(domain)
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
        && !is_ip_host(host)
}

/// RFC 6265 §5.1.4 default path: the request path up to, not including, its last `/`.
fn default_path(path: &str) -> &str {
    if !path.starts_with('/') {
        return "/";
    }
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(idx) => &path[..idx],
    }
}

/// RFC 6265 §5.1.4 path matching.
fn path_match(request_path: &str, cookie_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }
    request_path.starts_with(cookie_path)
        && (cookie_path.ends_with('/')
            || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/'))
}

fn is_date_delimiter(c: char) -> bool {
    matches!(c, '\t' | ' '..='/' | ';'..='@' | '['..='`' | '{'..='~')
}

/// Splits leading ASCII digits off `token`; `Some` only if their count is within `min..=max`.
fn leading_number(token: &str, min: usize, max: usize) -> Option<(u32, &str)> {
    let len = token.bytes().take_while(u8::is_ascii_digit).count();
    if len < min || len > max {
        return None;
    }
    Some((token[..len].parse().ok()?, &token[len..]))
}

fn parse_time_token(token: &str) -> Option<(u32, u32, u32)> {
    let (h, rest) = leading_number(token, 1, 2)?;
    let (m, rest) = leading_number(rest.strip_prefix(':')?, 1, 2)?;
    let (s, rest) = leading_number(rest.strip_prefix(':')?, 1, 2)?;
    if rest.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some((h, m, s))
}

fn parse_month_token(token: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let head = token.get(..3)?;
    MONTHS
        .iter()
        .position(|m| head.eq_ignore_ascii_case(m))
        .map(|i| i as u32 + 1)
}

/// RFC 6265 §5.1.1 cookie-date parsing to Unix seconds (dates before 1970 clamp to 0).
fn parse_cookie_date(input: &str) -> Option<u64> {
    let mut time = None;
    let mut day = None;
    let mut month = None;
    let mut year = None;
    for token in input.split(is_date_delimiter).filter(|t| !t.is_empty()) {
        if time.is_none() {
            if let Some(t) = parse_time_token(token) {
                time = Some(t);
                continue;
            }
        }
        if day.is_none() {
            if let Some((d, _)) = leading_number(token, 1, 2) {
                day = Some(d);
                continue;
            }
        }
        if month.is_none() {
            if let Some(m) = parse_month_token(token) {
                month = Some(m);
                continue;
            }
        }
        if year.is_none() {
            if let Some((y, _)) = leading_number(token, 2, 4) {
                year = Some(y);
                continue;
            }
        }
    }
    let (hour, minute, second) = time?;
    let (day, month, mut year) = (day?, month?, year?);
    if (70..=99).contains(&year) {
        year += 1900;
    } else if year <= 69 {
        year += 2000;
    }
    if !(1..=31).contains(&day) || year < 1601 || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(i64::from(year), month, day);
    let secs = days * 86_400 + i64::from(hour * 3600 + minute * 60 + second);
    Some(secs.max(0) as u64)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ---------------------------------------------------------------------------------------------
// Process-wide jar.

#[derive(Default)]
struct GlobalJar {
    jar: CookieJar,
    store_dir: Option<PathBuf>,
}

static GLOBAL_JAR: OnceLock<Mutex<GlobalJar>> = OnceLock::new();

fn global() -> MutexGuard<'static, GlobalJar> {
    GLOBAL_JAR
        .get_or_init(|| Mutex::new(GlobalJar::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Binds the process-wide jar to a data directory and loads its persistent cookies.
/// Later [`flush`] calls save there. Disk errors are non-fatal.
pub fn attach_store(dir: &Path) {
    let text = droplet_storage::load_cookie_file(dir);
    let mut g = global();
    if let Some(text) = text {
        g.jar.load_file_string(&text, now_timestamp());
    }
    g.jar.take_dirty();
    g.store_dir = Some(dir.to_path_buf());
}

/// Saves persistent cookies if they changed since the last save and a store is attached.
pub fn flush() {
    let mut g = global();
    if g.store_dir.is_none() || !g.jar.take_dirty() {
        return;
    }
    let text = g.jar.to_file_string(now_timestamp());
    if let Some(dir) = g.store_dir.as_deref() {
        // Keep the lock while writing so an older snapshot never overwrites a newer one.
        let _ = droplet_storage::save_cookie_file(dir, &text);
    }
}

#[cfg(test)]
fn detach_store() {
    global().store_dir = None;
}

/// `Cookie` header value for a request to `url` from the process-wide jar.
pub(crate) fn header_for(url: &Url) -> Option<String> {
    global().jar.cookie_header(url, now_timestamp())
}

/// Applies every `Set-Cookie` value of a response received from `url`.
pub(crate) fn store_response_cookies(url: &Url, set_cookies: &[String]) {
    if set_cookies.is_empty() {
        return;
    }
    let now = now_timestamp();
    let mut g = global();
    for header in set_cookies {
        g.jar.set_from_header(url, header, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000; // 2027-01-15

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn header(jar: &mut CookieJar, u: &str) -> Option<String> {
        jar.cookie_header(&url(u), NOW)
    }

    #[test]
    fn host_only_cookie_round_trip_with_cyrillic_value() {
        let mut jar = CookieJar::new();
        assert!(jar.set_from_header(&url("http://example.com/a/b"), "lang=русский", NOW));
        assert_eq!(header(&mut jar, "http://example.com/a/c").as_deref(), Some("lang=русский"));
        // Default path is /a, so /other does not match; host-only so sub.example.com doesn't.
        assert_eq!(header(&mut jar, "http://example.com/other"), None);
        assert_eq!(header(&mut jar, "http://sub.example.com/a/c"), None);
    }

    #[test]
    fn domain_and_path_matching_table() {
        let cases: &[(&str, &str, &str, bool)] = &[
            // (set from, Set-Cookie, request, sent?)
            ("http://www.example.com/", "a=1; Domain=example.com", "http://example.com/", true),
            ("http://www.example.com/", "a=1; Domain=.example.com", "http://x.y.example.com/", true),
            ("http://www.example.com/", "a=1; Domain=example.com", "http://notexample.com/", false),
            ("http://www.example.com/", "a=1; Domain=EXAMPLE.com", "http://www.example.com/", true),
            ("http://example.com/", "a=1; Path=/docs", "http://example.com/docs", true),
            ("http://example.com/", "a=1; Path=/docs", "http://example.com/docs/x", true),
            ("http://example.com/", "a=1; Path=/docs", "http://example.com/docsx", false),
            ("http://example.com/", "a=1; Path=/docs/", "http://example.com/docs/x", true),
            ("http://example.com/", "a=1; Path=/docs", "http://example.com/", false),
            ("http://example.com/", "a=1; Path=relative", "http://example.com/", true),
            ("http://127.0.0.1/", "a=1", "http://127.0.0.1:8080/", true),
            ("http://пример.рф/", "a=1; Domain=пример.рф", "http://вики.пример.рф/", true),
        ];
        for (from, set, req, sent) in cases {
            let mut jar = CookieJar::new();
            assert!(jar.set_from_header(&url(from), set, NOW), "{set} from {from} rejected");
            assert_eq!(header(&mut jar, req).is_some(), *sent, "{set} from {from} → {req}");
        }
    }

    #[test]
    fn foreign_and_public_suffix_domains_are_rejected() {
        let mut jar = CookieJar::new();
        let from = url("http://www.example.com/");
        assert!(!jar.set_from_header(&from, "a=1; Domain=com", NOW));
        assert!(!jar.set_from_header(&from, "a=1; Domain=other.com", NOW));
        assert!(!jar.set_from_header(&from, "a=1; Domain=sub.www.example.com", NOW));
        assert!(!jar.set_from_header(&url("http://пример.рф/"), "a=1; Domain=рф", NOW));
        assert!(!jar.set_from_header(&url("http://10.0.0.1/"), "a=1; Domain=0.0.1", NOW));
        assert!(jar.is_empty());
        // A single-label host may set Domain=itself; it becomes host-only.
        assert!(jar.set_from_header(&url("http://localhost/"), "a=1; Domain=localhost", NOW));
        assert!(jar.iter().all(|c| c.host_only && c.domain == "localhost"));
    }

    #[test]
    fn cyrillic_hosts_are_stored_lowercase_consistently() {
        let mut jar = CookieJar::new();
        // The URL parser only lowercases ASCII; the jar lowercases Unicode itself.
        assert!(jar.set_from_header(&url("http://ПРИМЕР.РФ/"), "a=1", NOW));
        assert_eq!(jar.iter().next().unwrap().domain, "пример.рф");
        assert_eq!(header(&mut jar, "http://пример.рф/").as_deref(), Some("a=1"));
        assert_eq!(header(&mut jar, "http://Пример.рф/").as_deref(), Some("a=1"));
    }

    #[test]
    fn secure_cookies_need_https_both_ways() {
        let mut jar = CookieJar::new();
        assert!(!jar.set_from_header(&url("http://example.com/"), "s=1; Secure", NOW));
        assert!(jar.set_from_header(&url("https://example.com/"), "s=1; Secure; HttpOnly", NOW));
        assert_eq!(header(&mut jar, "http://example.com/"), None);
        assert_eq!(header(&mut jar, "https://example.com/").as_deref(), Some("s=1"));
        // Plain http cannot overwrite the Secure cookie.
        assert!(!jar.set_from_header(&url("http://example.com/"), "s=evil", NOW));
        assert_eq!(header(&mut jar, "https://example.com/").as_deref(), Some("s=1"));
        let c = jar.iter().next().unwrap();
        assert!(c.secure && c.http_only);
    }

    #[test]
    fn expiry_max_age_and_deletion() {
        let mut jar = CookieJar::new();
        let u = url("http://example.com/");
        assert!(jar.set_from_header(&u, "a=1; Max-Age=60", NOW));
        assert_eq!(jar.iter().next().unwrap().expires, Some(NOW + 60));
        assert!(header(&mut jar, "http://example.com/").is_some());
        assert_eq!(jar.cookie_header(&u, NOW + 60), None, "expired at NOW+60");

        // Max-Age beats Expires regardless of order.
        assert!(jar.set_from_header(&u, "b=2; Max-Age=10; Expires=Wed, 09 Jun 2021 10:18:14 GMT", NOW));
        assert_eq!(jar.iter().find(|c| c.name == "b").unwrap().expires, Some(NOW + 10));

        // Expires in the past / Max-Age=0 deletes.
        assert!(jar.set_from_header(&u, "b=; Expires=Thu, 01 Jan 1970 00:00:00 GMT", NOW));
        assert!(jar.iter().all(|c| c.name != "b"));
        assert!(jar.set_from_header(&u, "a=1; Max-Age=0", NOW));
        assert!(jar.is_empty());
        assert!(!jar.set_from_header(&u, "c=1; Max-Age=-5", NOW), "nothing to delete");
        assert!(jar.is_empty());
    }

    #[test]
    fn cookie_dates_parse_per_rfc6265() {
        let t = |s| parse_cookie_date(s);
        assert_eq!(t("Wed, 09 Jun 2021 10:18:14 GMT"), Some(1_623_233_894));
        assert_eq!(t("Wednesday, 09-Jun-21 10:18:14 GMT"), Some(1_623_233_894));
        assert_eq!(t("Wed Jun  9 10:18:14 2021"), Some(1_623_233_894));
        assert_eq!(t("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(t("Sat, 01 Jan 1600 00:00:00 GMT"), None);
        assert_eq!(t("Wed, 32 Jun 2021 10:18:14 GMT"), None);
        assert_eq!(t("Wed, 09 Jun 2021 25:18:14 GMT"), None);
        assert_eq!(t("скоро"), None);
        assert_eq!(t(""), None);
    }

    #[test]
    fn malformed_headers_are_ignored() {
        let mut jar = CookieJar::new();
        let u = url("http://example.com/");
        for bad in ["", "novalue", "=1", " ; a=1", "a=1\r\nX: y", "a\u{0}=1"] {
            assert!(!jar.set_from_header(&u, bad, NOW), "accepted {bad:?}");
        }
        let big = format!("a={}", "я".repeat(MAX_COOKIE_BYTES / 2));
        assert!(!jar.set_from_header(&u, &big, NOW), "oversized cookie accepted");
        assert!(jar.is_empty());
        // Unknown attributes and a bad Expires are ignored, the cookie still stored.
        assert!(jar.set_from_header(&u, "  a = 1 ; Foo=bar; Expires=never; SameSite=Lax", NOW));
        let c = jar.iter().next().unwrap();
        assert_eq!((c.name.as_str(), c.value.as_str(), c.expires), ("a", "1", None));
        assert_eq!(c.same_site, SameSite::Lax);
        // file: URLs never get cookies.
        assert!(!jar.set_from_header(&url("file:///tmp/x"), "a=1", NOW));
    }

    #[test]
    fn overwrite_keeps_creation_and_send_order_is_path_then_age() {
        let mut jar = CookieJar::new();
        let u = url("http://example.com/a/b/c");
        assert!(jar.set_from_header(&u, "first=1; Path=/", NOW));
        assert!(jar.set_from_header(&u, "deep=1; Path=/a/b", NOW + 1));
        assert!(jar.set_from_header(&u, "second=1; Path=/", NOW + 2));
        assert!(jar.set_from_header(&u, "first=2; Path=/", NOW + 3));
        assert_eq!(jar.len(), 3);
        assert_eq!(
            header(&mut jar, "http://example.com/a/b/c").as_deref(),
            Some("deep=1; first=2; second=1")
        );
    }

    #[test]
    fn caps_evict_least_recently_used() {
        let mut jar = CookieJar::new();
        let u = url("http://example.com/");
        for i in 0..MAX_COOKIES_PER_DOMAIN {
            assert!(jar.set_from_header(&u, &format!("c{i}=1"), NOW));
        }
        // Touch c0 by storing it again, so c1 becomes the LRU entry.
        assert!(jar.set_from_header(&u, "c0=2", NOW));
        assert!(jar.set_from_header(&u, "extra=1", NOW));
        assert_eq!(jar.len(), MAX_COOKIES_PER_DOMAIN);
        assert!(jar.iter().any(|c| c.name == "c0"));
        assert!(jar.iter().all(|c| c.name != "c1"));

        // Expired cookies go before live ones.
        let mut jar = CookieJar::new();
        assert!(jar.set_from_header(&u, "old=1; Max-Age=5", NOW));
        for i in 0..MAX_COOKIES_PER_DOMAIN - 1 {
            assert!(jar.set_from_header(&u, &format!("c{i}=1"), NOW + 10));
        }
        assert!(jar.set_from_header(&u, "new=1", NOW + 10));
        // `old` expired at NOW+5 but `new` is stored at NOW+10 → old must be the victim.
        assert!(jar.iter().all(|c| c.name != "old"));
        assert_eq!(jar.len(), MAX_COOKIES_PER_DOMAIN);
        assert!(jar.cookie_header(&u, NOW + 10).unwrap().contains("c0=1"));

        // Total cap across domains.
        let mut jar = CookieJar::new();
        let domains = MAX_COOKIES_TOTAL / MAX_COOKIES_PER_DOMAIN + 1;
        for d in 0..domains {
            let du = url(&format!("http://site{d}.example/"));
            for i in 0..MAX_COOKIES_PER_DOMAIN {
                jar.set_from_header(&du, &format!("c{i}=1"), NOW);
            }
        }
        assert_eq!(jar.len(), MAX_COOKIES_TOTAL);
        // The very first cookies stored are the global LRU victims.
        assert_eq!(jar.cookie_header(&url("http://site0.example/"), NOW), None);
        assert!(jar.cookie_header(&url(&format!("http://site{}.example/", domains - 1)), NOW).is_some());
    }

    #[test]
    fn file_round_trip_keeps_only_persistent_cookies() {
        let mut jar = CookieJar::new();
        let u = url("https://вики.пример.рф/статьи/1");
        assert!(jar.set_from_header(&u, "session=1", NOW));
        assert!(jar.set_from_header(&u, "имя=Значение\twith tab; Max-Age=3600; Secure; HttpOnly; SameSite=Strict", NOW));
        assert!(jar.set_from_header(&u, "wide=1; Domain=пример.рф; Path=/; Expires=Fri, 01 Jan 2100 00:00:00 GMT", NOW));
        assert!(jar.take_dirty());

        let text = jar.to_file_string(NOW);
        assert!(text.starts_with("DRPLCOOKIE1\n"));
        assert_eq!(text.lines().count(), 3, "{text}");

        let mut loaded = CookieJar::new();
        assert_eq!(loaded.load_file_string(&text, NOW), 2);
        let c = loaded.iter().find(|c| c.name == "имя").unwrap();
        assert_eq!(c.value, "Значение\twith tab");
        assert!(c.secure && c.http_only && c.host_only);
        assert_eq!(c.same_site, SameSite::Strict);
        assert_eq!(c.path, "/статьи");
        assert_eq!(
            loaded.cookie_header(&url("https://пример.рф/"), NOW).as_deref(),
            Some("wide=1")
        );
        // Expired by load time → skipped; garbage lines and wrong magic are ignored.
        assert_eq!(CookieJar::new().load_file_string(&text, NOW + 7200), 1);
        let junk = format!("{text}garbage\tline\nx\t1\t/\ta\tb\tnotanumber\t0\t0\t\t0\n");
        assert_eq!(CookieJar::new().load_file_string(&junk, NOW), 2);
        assert_eq!(CookieJar::new().load_file_string("DRPLCOOKIE0\n", NOW), 0);
    }

    #[test]
    fn persistent_cookies_survive_a_restart_through_the_data_dir() {
        let dir = std::env::temp_dir().join(format!("droplet_cookies_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        attach_store(&dir);
        let u = url("https://сохранение.drop113.test/");
        store_response_cookies(
            &u,
            &["keep=да; Max-Age=3600; Secure".to_string(), "session=1".to_string()],
        );
        flush();
        detach_store();

        // A fresh process would load exactly the persistent cookie for this host.
        let text = droplet_storage::load_cookie_file(&dir).expect("cookie file written");
        let mut restarted = CookieJar::new();
        assert!(restarted.load_file_string(&text, now_timestamp()) >= 1);
        assert_eq!(restarted.cookie_header(&u, now_timestamp()).as_deref(), Some("keep=да"));
        assert_eq!(restarted.cookie_header(&url("http://сохранение.drop113.test/"), now_timestamp()), None);

        // attach_store loads the file into the process-wide jar (same name+path → replaced).
        // The jar is shared with parallel tests, so change the value instead of resetting it.
        let copy = dir.join("copy");
        droplet_storage::save_cookie_file(&copy, &text.replace("keep\tда", "keep\tнет")).unwrap();
        attach_store(&copy);
        detach_store();
        // The session cookie stored above is still in memory; `keep` now has the file's value.
        assert_eq!(header_for(&u).as_deref(), Some("session=1; keep=нет"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
