use std::collections::HashMap;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::adblock::TrackerFilter;
use crate::url::{Scheme, Url};

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REDIRECTS: usize = 5;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_CONCURRENT_FETCHES: usize = 4;

const DNS_TTL: Duration = Duration::from_secs(60);
const DNS_CACHE_MAX_ENTRIES: usize = 256;
const POOL_IDLE_TTL: Duration = Duration::from_secs(30);
const POOL_MAX_PER_ORIGIN: usize = 4;
const POOL_MAX_TOTAL: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub url: Url,
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetError {
    pub message: String,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NetError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SchemeKey {
    Http,
    Https,
}

struct DnsEntry {
    addrs: Vec<SocketAddr>,
    expires_at: Instant,
}

struct PooledConn {
    stream: TransportStream,
    last_used: Instant,
}

struct NetState {
    dns_cache: HashMap<(String, u16), DnsEntry>,
    pool: HashMap<(SchemeKey, String, u16), Vec<PooledConn>>,
}

impl NetState {
    fn new() -> Self {
        Self {
            dns_cache: HashMap::new(),
            pool: HashMap::new(),
        }
    }
}

static NET_STATE: OnceLock<Mutex<NetState>> = OnceLock::new();
static DEFAULT_TRACKER_FILTER: OnceLock<TrackerFilter> = OnceLock::new();

fn net_state() -> &'static Mutex<NetState> {
    NET_STATE.get_or_init(|| Mutex::new(NetState::new()))
}

fn default_filter() -> &'static TrackerFilter {
    DEFAULT_TRACKER_FILTER.get_or_init(TrackerFilter::with_defaults)
}

pub fn fetch(url: &Url) -> Result<Response, NetError> {
    fetch_with_filter(url, default_filter())
}

pub fn fetch_with_filter(url: &Url, filter: &TrackerFilter) -> Result<Response, NetError> {
    let (final_url, status, content_type, body_bytes) = fetch_raw_with_filter(url, filter)?;
    let body = String::from_utf8(body_bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

    Ok(Response {
        url: final_url,
        status,
        content_type,
        body,
    })
}

pub fn fetch_many(urls: &[Url]) -> Vec<Result<Response, NetError>> {
    fetch_many_with_filter(urls, default_filter())
}

pub fn fetch_many_with_filter(
    urls: &[Url],
    filter: &TrackerFilter,
) -> Vec<Result<Response, NetError>> {
    if urls.is_empty() {
        return Vec::new();
    }
    if urls.len() == 1 {
        return vec![fetch_with_filter(&urls[0], filter)];
    }

    let worker_count = urls.len().min(MAX_CONCURRENT_FETCHES);
    let next = AtomicUsize::new(0);

    let batches: Vec<Vec<(usize, Result<Response, NetError>)>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            handles.push(scope.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    if idx >= urls.len() {
                        break;
                    }
                    let res = fetch_with_filter(&urls[idx], filter);
                    local.push((idx, res));
                }
                local
            }));
        }

        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });

    let mut ordered: Vec<Option<Result<Response, NetError>>> = (0..urls.len()).map(|_| None).collect();
    for batch in batches {
        for (idx, res) in batch {
            ordered[idx] = Some(res);
        }
    }

    ordered
        .into_iter()
        .map(|slot| {
            slot.unwrap_or_else(|| {
                Err(NetError {
                    message: "Concurrent fetch worker terminated unexpectedly".into(),
                })
            })
        })
        .collect()
}

pub fn fetch_bytes(url: &Url) -> Result<(String, Vec<u8>), NetError> {
    let (final_url, status, content_type, bytes) = fetch_raw_with_filter(url, default_filter())?;
    if status >= 400 {
        return Err(NetError {
            message: format!("HTTP error status {status} for '{}'", final_url.as_str()),
        });
    }
    Ok((content_type, bytes))
}

pub fn fetch_uri_bytes(raw_uri: &str, base: &Url) -> Result<(String, Vec<u8>), NetError> {
    let trimmed = raw_uri.trim();
    if trimmed.is_empty() {
        return Err(NetError {
            message: "Empty URI".into(),
        });
    }

    if trimmed
        .as_bytes()
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"data:"))
    {
        return parse_data_uri(&trimmed[5..]);
    }

    let resolved = base.resolve(trimmed).map_err(|err| NetError {
        message: format!("Invalid resource URI '{trimmed}': {err}"),
    })?;
    fetch_bytes(&resolved)
}

struct RawHttpResponse {
    status: u16,
    content_type: String,
    location: Option<String>,
    body: Vec<u8>,
}

fn fetch_raw_with_filter(
    url: &Url,
    filter: &TrackerFilter,
) -> Result<(Url, u16, String, Vec<u8>), NetError> {
    match url.scheme {
        Scheme::About => {
            let resp = fetch_about(url);
            Ok((resp.url, resp.status, resp.content_type, resp.body.into_bytes()))
        }
        Scheme::File => {
            let bytes = std::fs::read(&url.path).map_err(|err| NetError {
                message: format!("Failed to read file '{}': {err}", url.path),
            })?;
            Ok((url.clone(), 200, guess_content_type(&url.path), bytes))
        }
        Scheme::Http | Scheme::Https => fetch_network_with_redirects(url, filter),
    }
}

fn fetch_network_with_redirects(
    initial_url: &Url,
    filter: &TrackerFilter,
) -> Result<(Url, u16, String, Vec<u8>), NetError> {
    let mut current_url = initial_url.clone();

    for hop in 0..=MAX_REDIRECTS {
        if filter.should_block(&current_url.host) {
            return Err(NetError {
                message: format!("Blocked by tracker filter: {}", current_url.host),
            });
        }

        let resp = send_http_get(&current_url)?;

        if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
            if let Some(loc) = resp.location {
                if hop == MAX_REDIRECTS {
                    return Err(NetError {
                        message: format!("Too many HTTP redirects (max {MAX_REDIRECTS})"),
                    });
                }
                current_url = current_url.resolve(&loc).map_err(|err| NetError {
                    message: format!("Invalid redirect target '{loc}': {err}"),
                })?;
                if !matches!(current_url.scheme, Scheme::Http | Scheme::Https) {
                    return Err(NetError {
                        message: format!(
                            "Refusing HTTP redirect to non-network URL: {}",
                            current_url.as_str()
                        ),
                    });
                }
                continue;
            }
        }

        return Ok((current_url, resp.status, resp.content_type, resp.body));
    }

    Err(NetError {
        message: format!("Too many HTTP redirects (max {MAX_REDIRECTS})"),
    })
}

fn guess_content_type(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".css") {
        "text/css; charset=utf-8".into()
    } else if lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".rs") {
        "text/plain; charset=utf-8".into()
    } else if lower.ends_with(".png") {
        "image/png".into()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".into()
    } else if lower.ends_with(".gif") {
        "image/gif".into()
    } else if lower.ends_with(".webp") {
        "image/webp".into()
    } else if lower.ends_with(".svg") {
        "image/svg+xml".into()
    } else if lower.ends_with(".bmp") {
        "image/bmp".into()
    } else if lower.ends_with(".ico") {
        "image/x-icon".into()
    } else if lower.ends_with(".json") {
        "application/json; charset=utf-8".into()
    } else {
        "text/html; charset=utf-8".into()
    }
}

fn parse_data_uri(rest: &str) -> Result<(String, Vec<u8>), NetError> {
    let Some((meta, payload)) = rest.split_once(',') else {
        return Err(NetError {
            message: "Malformed data: URI (missing ',')".into(),
        });
    };

    let mut is_base64 = false;
    let mut parts: Vec<&str> = Vec::new();
    for raw_part in meta.split(';') {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }
        if part.eq_ignore_ascii_case("base64") {
            is_base64 = true;
        } else {
            parts.push(part);
        }
    }

    let content_type = if parts.is_empty() {
        "text/plain;charset=US-ASCII".to_string()
    } else if parts[0].contains('=') {
        format!("text/plain;{}", parts.join(";"))
    } else {
        parts.join(";")
    };

    let bytes = if is_base64 {
        if payload.as_bytes().contains(&b'%') {
            let unescaped = percent_decode_bytes(payload.as_bytes());
            decode_base64(&unescaped)?
        } else {
            decode_base64(payload.as_bytes())?
        }
    } else {
        let decoded = percent_decode_bytes(payload.as_bytes());
        if decoded.len() > MAX_RESPONSE_BYTES {
            return Err(NetError {
                message: format!(
                    "data: URI exceeded maximum size ({MAX_RESPONSE_BYTES} bytes)"
                ),
            });
        }
        decoded
    };

    Ok((content_type, bytes))
}

fn percent_decode_bytes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let (Some(hi), Some(lo)) = (hex_nibble(input[i + 1]), hex_nibble(input[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_base64(input: &[u8]) -> Result<Vec<u8>, NetError> {
    let mut vals: [u8; 4] = [0; 4];
    let mut qlen = 0usize;
    let mut saw_pad = false;
    let mut pad_count = 0usize;
    let mut out = Vec::with_capacity((input.len() / 4) * 3);

    for &b in input {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            saw_pad = true;
            pad_count += 1;
            vals[qlen] = 0;
            qlen += 1;
            if qlen == 4 {
                if pad_count > 2 {
                    return Err(NetError {
                        message: "Invalid base64 padding in data: URI".into(),
                    });
                }
                push_base64_quantum(&vals, 4 - pad_count, &mut out)?;
                qlen = 0;
                pad_count = 0;
            }
            continue;
        }
        if saw_pad {
            return Err(NetError {
                message: "Unexpected data after base64 '=' padding in data: URI".into(),
            });
        }
        let val = base64_val(b).ok_or_else(|| NetError {
            message: format!("Invalid base64 byte 0x{b:02X} in data: URI"),
        })?;
        vals[qlen] = val;
        qlen += 1;
        if qlen == 4 {
            push_base64_quantum(&vals, 4, &mut out)?;
            qlen = 0;
        }
    }

    if saw_pad && qlen != 0 {
        return Err(NetError {
            message: "Truncated base64 padding quantum in data: URI".into(),
        });
    }
    if qlen == 1 {
        return Err(NetError {
            message: "Invalid base64 length in data: URI".into(),
        });
    }
    if qlen > 1 {
        push_base64_quantum(&vals, qlen, &mut out)?;
    }
    Ok(out)
}

fn push_base64_quantum(
    vals: &[u8; 4],
    valid_chars: usize,
    out: &mut Vec<u8>,
) -> Result<(), NetError> {
    if out.len() + 3 > MAX_RESPONSE_BYTES {
        return Err(NetError {
            message: format!("data: URI exceeded maximum size ({MAX_RESPONSE_BYTES} bytes)"),
        });
    }
    let n = ((vals[0] as u32) << 18)
        | ((vals[1] as u32) << 12)
        | ((vals[2] as u32) << 6)
        | (vals[3] as u32);
    match valid_chars {
        4 => {
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
            out.push(n as u8);
        }
        3 => {
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
        }
        2 => {
            out.push((n >> 16) as u8);
        }
        _ => {
            return Err(NetError {
                message: "Invalid base64 quantum in data: URI".into(),
            });
        }
    }
    Ok(())
}

fn base64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn scheme_key(scheme: &Scheme) -> Option<SchemeKey> {
    match scheme {
        Scheme::Http => Some(SchemeKey::Http),
        Scheme::Https => Some(SchemeKey::Https),
        Scheme::File | Scheme::About => None,
    }
}

fn strip_ipv6_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host)
}

fn resolve_addrs(host: &str, port: u16) -> Result<(Vec<SocketAddr>, bool), NetError> {
    let key = (host.to_string(), port);
    let now = Instant::now();
    {
        let mut state = net_state().lock().unwrap_or_else(|e| e.into_inner());
        state.dns_cache.retain(|_, entry| entry.expires_at > now);
        if let Some(entry) = state.dns_cache.get(&key) {
            return Ok((entry.addrs.clone(), true));
        }
    }

    let addrs = resolve_addrs_uncached(host, port)?;
    cache_dns_entry(key, addrs.clone());
    Ok((addrs, false))
}

fn resolve_addrs_uncached(host: &str, port: u16) -> Result<Vec<SocketAddr>, NetError> {
    let lookup_host = strip_ipv6_brackets(host);
    let addrs: Vec<SocketAddr> = (lookup_host, port)
        .to_socket_addrs()
        .map_err(|err| NetError {
            message: format!("DNS resolution failed for '{host}': {err}"),
        })?
        .collect();

    if addrs.is_empty() {
        return Err(NetError {
            message: format!("No addresses found for '{host}'"),
        });
    }
    Ok(addrs)
}

fn cache_dns_entry(key: (String, u16), addrs: Vec<SocketAddr>) {
    let now = Instant::now();
    let mut state = net_state().lock().unwrap_or_else(|e| e.into_inner());
    state.dns_cache.retain(|_, entry| entry.expires_at > now);
    if !state.dns_cache.contains_key(&key) && state.dns_cache.len() >= DNS_CACHE_MAX_ENTRIES {
        if let Some(oldest_key) = state
            .dns_cache
            .iter()
            .min_by_key(|(_, entry)| entry.expires_at)
            .map(|(k, _)| k.clone())
        {
            state.dns_cache.remove(&oldest_key);
        }
    }
    state.dns_cache.insert(
        key,
        DnsEntry {
            addrs,
            expires_at: now + DNS_TTL,
        },
    );
}

fn invalidate_dns_entry(host: &str, port: u16) {
    let mut state = net_state().lock().unwrap_or_else(|e| e.into_inner());
    state.dns_cache.remove(&(host.to_string(), port));
}

fn take_pooled_conn(key: &(SchemeKey, String, u16)) -> Option<TransportStream> {
    let now = Instant::now();
    let mut state = net_state().lock().unwrap_or_else(|e| e.into_inner());
    let list = state.pool.get_mut(key)?;
    while let Some(conn) = list.pop() {
        if now.saturating_duration_since(conn.last_used) <= POOL_IDLE_TTL {
            if list.is_empty() {
                state.pool.remove(key);
            }
            return Some(conn.stream);
        }
    }
    state.pool.remove(key);
    None
}

fn return_pooled_conn(key: (SchemeKey, String, u16), stream: TransportStream) {
    let now = Instant::now();
    let mut evicted: Vec<TransportStream> = Vec::new();
    {
        let mut state = net_state().lock().unwrap_or_else(|e| e.into_inner());
        state.pool.retain(|_, list| {
            list.retain(|c| now.saturating_duration_since(c.last_used) <= POOL_IDLE_TTL);
            !list.is_empty()
        });

        let mut total_count: usize = state.pool.values().map(Vec::len).sum();
        while total_count >= POOL_MAX_TOTAL {
            let oldest_origin = state
                .pool
                .iter()
                .filter_map(|(k, v)| v.first().map(|c| (k.clone(), c.last_used)))
                .min_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k);
            let Some(origin_key) = oldest_origin else {
                break;
            };
            if let Some(vec) = state.pool.get_mut(&origin_key) {
                if !vec.is_empty() {
                    evicted.push(vec.remove(0).stream);
                    total_count -= 1;
                }
                if vec.is_empty() {
                    state.pool.remove(&origin_key);
                }
            }
        }

        let entry = state.pool.entry(key).or_default();
        if entry.len() >= POOL_MAX_PER_ORIGIN {
            evicted.push(entry.remove(0).stream);
        }
        entry.push(PooledConn {
            stream,
            last_used: now,
        });
    }
    drop(evicted);
}

enum TransportStream {
    Plain(TcpStream),
    Tls(tls::TlsStream),
}

impl Read for TransportStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            TransportStream::Plain(s) => s.read(buf),
            TransportStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for TransportStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            TransportStream::Plain(s) => s.write(buf),
            TransportStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            TransportStream::Plain(s) => s.flush(),
            TransportStream::Tls(s) => s.flush(),
        }
    }
}

fn connect_tcp_addrs(addrs: &[SocketAddr]) -> Result<TcpStream, std::io::Error> {
    let mut last_err = None;
    for addr in addrs {
        match TcpStream::connect_timeout(addr, HTTP_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no addresses")))
}

fn connect_stream(url: &Url) -> Result<TransportStream, NetError> {
    let (addrs, from_cache) = resolve_addrs(&url.host, url.port)?;
    let tcp = match connect_tcp_addrs(&addrs) {
        Ok(stream) => stream,
        Err(first_err) => {
            if from_cache {
                invalidate_dns_entry(&url.host, url.port);
                let fresh_addrs = resolve_addrs_uncached(&url.host, url.port)?;
                cache_dns_entry((url.host.clone(), url.port), fresh_addrs.clone());
                connect_tcp_addrs(&fresh_addrs).map_err(|err| NetError {
                    message: format!("Failed to connect to '{}:{}': {err}", url.host, url.port),
                })?
            } else {
                return Err(NetError {
                    message: format!(
                        "Failed to connect to '{}:{}': {first_err}",
                        url.host, url.port
                    ),
                });
            }
        }
    };

    let _ = tcp.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(HTTP_TIMEOUT));
    let _ = tcp.set_nodelay(true);

    match url.scheme {
        Scheme::Http => Ok(TransportStream::Plain(tcp)),
        Scheme::Https => Ok(TransportStream::Tls(tls::TlsStream::connect(tcp, &url.host)?)),
        Scheme::File | Scheme::About => Err(NetError {
            message: format!("Non-network scheme passed to connect_stream: {}", url.as_str()),
        }),
    }
}

enum ExchangeOutcome {
    Complete {
        parsed: RawHttpResponse,
        can_reuse: bool,
    },
    Stale,
}

fn send_http_get(url: &Url) -> Result<RawHttpResponse, NetError> {
    let Some(skey) = scheme_key(&url.scheme) else {
        return Err(NetError {
            message: format!("Unsupported network scheme for {}", url.as_str()),
        });
    };
    let pool_key = (skey, url.host.clone(), url.port);

    if let Some(mut pooled) = take_pooled_conn(&pool_key) {
        match execute_http_exchange(&mut pooled, url, true)? {
            ExchangeOutcome::Complete { parsed, can_reuse } => {
                if can_reuse {
                    return_pooled_conn(pool_key, pooled);
                }
                return Ok(parsed);
            }
            ExchangeOutcome::Stale => {
                drop(pooled);
            }
        }
    }

    let mut fresh = connect_stream(url)?;
    match execute_http_exchange(&mut fresh, url, false)? {
        ExchangeOutcome::Complete { parsed, can_reuse } => {
            if can_reuse {
                return_pooled_conn(pool_key, fresh);
            }
            Ok(parsed)
        }
        ExchangeOutcome::Stale => Err(NetError {
            message: format!("Connection closed before HTTP status line from '{}'", url.host),
        }),
    }
}

fn execute_http_exchange(
    stream: &mut TransportStream,
    url: &Url,
    is_reused: bool,
) -> Result<ExchangeOutcome, NetError> {
    let target = url.request_target();
    let host_header = url.host_header();

    let request = format!(
        "GET {target} HTTP/1.1\r\n\
         Host: {host_header}\r\n\
         User-Agent: Droplet/0.1\r\n\
         Accept: text/html,text/css,image/*,*/*\r\n\
         Accept-Encoding: identity\r\n\
         Connection: keep-alive\r\n\
         \r\n"
    );

    if let Err(err) = write_all_retry_eintr(stream, request.as_bytes()) {
        if is_reused {
            return Ok(ExchangeOutcome::Stale);
        }
        return Err(NetError {
            message: format!("Failed to send HTTP request to '{}': {err}", url.host),
        });
    }

    read_http_response(stream, url, is_reused)
}

fn write_all_retry_eintr(stream: &mut TransportStream, mut data: &[u8]) -> std::io::Result<()> {
    while !data.is_empty() {
        match stream.write(data) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::WriteZero,
                    "failed to write HTTP request",
                ));
            }
            Ok(n) => data = &data[n..],
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

struct ParsedHeaders {
    status: u16,
    content_type: String,
    location: Option<String>,
    is_chunked: bool,
    content_length: Option<usize>,
    server_close: bool,
}

fn parse_headers_block(header_bytes: &[u8], url: &Url) -> Result<ParsedHeaders, NetError> {
    let header_text = String::from_utf8_lossy(header_bytes);
    let mut lines = header_text.split('\n');

    let status_line = lines
        .next()
        .map(|l| l.trim_end_matches('\r').trim())
        .unwrap_or("");
    let (is_http_10, status) = parse_status_line(status_line, url)?;

    let mut content_type = "text/html; charset=utf-8".to_string();
    let mut location = None;
    let mut is_chunked = false;
    let mut content_length: Option<usize> = None;
    let mut has_close = false;
    let mut has_keep_alive = false;

    for raw_line in lines {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let Some((name, val)) = line.split_once(':') else {
            continue;
        };
        let key = name.trim().to_ascii_lowercase();
        let value = val.trim();
        match key.as_str() {
            "content-type" if !value.is_empty() => content_type = value.to_string(),
            "location" if !value.is_empty() => location = Some(value.to_string()),
            "transfer-encoding" => {
                if value.to_ascii_lowercase().contains("chunked") {
                    is_chunked = true;
                }
            }
            "content-length" => {
                let len = value.parse::<usize>().map_err(|_| NetError {
                    message: format!("Invalid Content-Length header '{value}' from {url}"),
                })?;
                if let Some(prev) = content_length {
                    if prev != len {
                        return Err(NetError {
                            message: format!("Conflicting Content-Length headers from {url}"),
                        });
                    }
                } else {
                    content_length = Some(len);
                }
            }
            "connection" => {
                for token in value.split(',') {
                    let tok = token.trim();
                    if tok.eq_ignore_ascii_case("close") {
                        has_close = true;
                    } else if tok.eq_ignore_ascii_case("keep-alive") {
                        has_keep_alive = true;
                    }
                }
            }
            _ => {}
        }
    }

    let server_close = has_close || (is_http_10 && !has_keep_alive);
    Ok(ParsedHeaders {
        status,
        content_type,
        location,
        is_chunked,
        content_length,
        server_close,
    })
}

fn read_http_response(
    stream: &mut TransportStream,
    url: &Url,
    is_reused: bool,
) -> Result<ExchangeOutcome, NetError> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 8192];

    let (header_end, body_start) = loop {
        let prev_len = buf.len();
        match stream.read(&mut tmp) {
            Ok(0) => {
                if buf.is_empty() && is_reused {
                    return Ok(ExchangeOutcome::Stale);
                }
                return Err(NetError {
                    message: format!(
                        "Malformed HTTP response from {url}: unexpected EOF before headers"
                    ),
                });
            }
            Ok(n) => {
                if buf.len() + n > MAX_HEADER_BYTES + MAX_RESPONSE_BYTES {
                    return Err(NetError {
                        message: format!(
                            "HTTP response exceeded maximum size ({MAX_RESPONSE_BYTES} bytes)"
                        ),
                    });
                }
                buf.extend_from_slice(&tmp[..n]);
                let search_from = prev_len.saturating_sub(3);
                if let Some((rel_end, rel_body)) = find_header_terminator(&buf[search_from..]) {
                    break (search_from + rel_end, search_from + rel_body);
                }
                if buf.len() > MAX_HEADER_BYTES {
                    return Err(NetError {
                        message: format!(
                            "HTTP response headers exceeded maximum size ({MAX_HEADER_BYTES} bytes)"
                        ),
                    });
                }
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err)
                if is_reused
                    && buf.is_empty()
                    && matches!(
                        err.kind(),
                        ErrorKind::BrokenPipe
                            | ErrorKind::ConnectionReset
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::UnexpectedEof
                    ) =>
            {
                return Ok(ExchangeOutcome::Stale);
            }
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(NetError {
                    message: format!("Timed out reading HTTP headers from '{}'", url.host),
                });
            }
            Err(err) => {
                return Err(NetError {
                    message: format!("Failed reading HTTP response from '{}': {err}", url.host),
                });
            }
        }
    };

    let headers = parse_headers_block(&buf[..header_end], url)?;
    let leftover = buf.split_off(body_start);

    let no_body_status =
        matches!(headers.status, 204 | 304) || (100..200).contains(&headers.status);

    let (body, clean_boundary) = if no_body_status {
        (Vec::new(), leftover.is_empty())
    } else if headers.is_chunked {
        read_chunked_body_streaming(stream, leftover, url)?
    } else if let Some(len) = headers.content_length {
        read_fixed_body_streaming(stream, leftover, len, url)?
    } else {
        let body = read_until_eof_streaming(stream, leftover, url)?;
        (body, false)
    };

    let can_reuse = !headers.server_close && clean_boundary;
    Ok(ExchangeOutcome::Complete {
        parsed: RawHttpResponse {
            status: headers.status,
            content_type: headers.content_type,
            location: headers.location,
            body,
        },
        can_reuse,
    })
}

fn read_socket_chunk(
    stream: &mut TransportStream,
    buf: &mut Vec<u8>,
    max_read: usize,
    url: &Url,
) -> Result<usize, NetError> {
    let mut tmp = [0u8; 8192];
    let limit = max_read.min(tmp.len());
    loop {
        match stream.read(&mut tmp[..limit]) {
            Ok(n) => {
                if n > 0 {
                    buf.extend_from_slice(&tmp[..n]);
                }
                return Ok(n);
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(NetError {
                    message: format!("Timed out reading HTTP body from '{}'", url.host),
                });
            }
            Err(err) => {
                return Err(NetError {
                    message: format!("Failed reading HTTP body from '{}': {err}", url.host),
                });
            }
        }
    }
}

fn read_fixed_body_streaming(
    stream: &mut TransportStream,
    mut body: Vec<u8>,
    expected_len: usize,
    url: &Url,
) -> Result<(Vec<u8>, bool), NetError> {
    if expected_len > MAX_RESPONSE_BYTES {
        return Err(NetError {
            message: format!("HTTP response exceeded maximum size ({MAX_RESPONSE_BYTES} bytes)"),
        });
    }

    if body.len() >= expected_len {
        let exact = body.len() == expected_len;
        body.truncate(expected_len);
        return Ok((body, exact));
    }

    body.reserve(expected_len - body.len());
    while body.len() < expected_len {
        let remaining = expected_len - body.len();
        let n = read_socket_chunk(stream, &mut body, remaining, url)?;
        if n == 0 {
            return Err(NetError {
                message: format!(
                    "Unexpected EOF reading HTTP body from '{}' ({} of {expected_len} bytes)",
                    url.host,
                    body.len()
                ),
            });
        }
    }
    Ok((body, true))
}

fn read_chunked_body_streaming(
    stream: &mut TransportStream,
    mut buf: Vec<u8>,
    url: &Url,
) -> Result<(Vec<u8>, bool), NetError> {
    let mut out = Vec::new();
    let mut pos = 0usize;

    loop {
        let (line_rel, next_pos) = loop {
            if let Some(found) = find_line_end(&buf[pos..]) {
                break found;
            }
            if buf.len().saturating_sub(pos) > MAX_HEADER_BYTES {
                return Err(NetError {
                    message: "HTTP chunk size line exceeded maximum length".into(),
                });
            }
            if read_socket_chunk(stream, &mut buf, 8192, url)? == 0 {
                return Err(NetError {
                    message: "Unexpected EOF while reading chunk size".into(),
                });
            }
        };

        let line_str = std::str::from_utf8(&buf[pos..pos + line_rel]).map_err(|_| NetError {
            message: "Invalid UTF-8 in chunk size line".into(),
        })?;
        let hex_part = line_str.split(';').next().unwrap_or("").trim();
        if hex_part.is_empty() {
            return Err(NetError {
                message: "Empty HTTP chunk size".into(),
            });
        }
        let chunk_size = usize::from_str_radix(hex_part, 16).map_err(|_| NetError {
            message: format!("Invalid HTTP chunk size: '{hex_part}'"),
        })?;
        pos += next_pos;

        if chunk_size == 0 {
            loop {
                let (trailer_rel, trailer_next) = loop {
                    if let Some(found) = find_line_end(&buf[pos..]) {
                        break found;
                    }
                    if buf.len().saturating_sub(pos) > MAX_HEADER_BYTES {
                        return Err(NetError {
                            message: "HTTP chunked trailers exceeded maximum length".into(),
                        });
                    }
                    if read_socket_chunk(stream, &mut buf, 8192, url)? == 0 {
                        return Err(NetError {
                            message: "Unexpected EOF while reading chunked trailer terminator"
                                .into(),
                        });
                    }
                };
                pos += trailer_next;
                if trailer_rel == 0 {
                    break;
                }
            }
            let clean_boundary = pos == buf.len();
            return Ok((out, clean_boundary));
        }

        if out.len().saturating_add(chunk_size) > MAX_RESPONSE_BYTES {
            return Err(NetError {
                message: format!(
                    "HTTP chunked response exceeded maximum size ({MAX_RESPONSE_BYTES} bytes)"
                ),
            });
        }

        while buf.len() < pos + chunk_size {
            if read_socket_chunk(stream, &mut buf, 8192, url)? == 0 {
                return Err(NetError {
                    message: "Unexpected EOF inside HTTP chunk payload".into(),
                });
            }
        }
        out.extend_from_slice(&buf[pos..pos + chunk_size]);
        pos += chunk_size;

        while buf.len() <= pos {
            if read_socket_chunk(stream, &mut buf, 8192, url)? == 0 {
                return Err(NetError {
                    message: "Unexpected EOF waiting for CRLF after HTTP chunk".into(),
                });
            }
        }
        if buf[pos] == b'\r' {
            while buf.len() < pos + 2 {
                if read_socket_chunk(stream, &mut buf, 8192, url)? == 0 {
                    return Err(NetError {
                        message: "Unexpected EOF waiting for LF after HTTP chunk".into(),
                    });
                }
            }
            if buf[pos + 1] != b'\n' {
                return Err(NetError {
                    message: "Missing CRLF after HTTP chunk payload".into(),
                });
            }
            pos += 2;
        } else if buf[pos] == b'\n' {
            pos += 1;
        } else {
            return Err(NetError {
                message: "Missing CRLF after HTTP chunk payload".into(),
            });
        }

        if pos > 65536 {
            buf.drain(..pos);
            pos = 0;
        }
    }
}

fn read_until_eof_streaming(
    stream: &mut TransportStream,
    mut body: Vec<u8>,
    url: &Url,
) -> Result<Vec<u8>, NetError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(NetError {
            message: format!("HTTP response exceeded maximum size ({MAX_RESPONSE_BYTES} bytes)"),
        });
    }
    loop {
        let prev = body.len();
        let n = read_socket_chunk(stream, &mut body, 8192, url)?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(NetError {
                message: format!(
                    "HTTP response exceeded maximum size ({MAX_RESPONSE_BYTES} bytes)"
                ),
            });
        }
        if n == 0 || body.len() == prev {
            break;
        }
    }
    Ok(body)
}

pub fn parse_http_response(
    raw: &[u8],
    url: Url,
) -> Result<(u16, String, Option<String>, Vec<u8>), NetError> {
    let (header_end, body_start) = find_header_terminator(raw).ok_or_else(|| NetError {
        message: format!("Malformed HTTP response from {url}: missing header terminator"),
    })?;

    let headers = parse_headers_block(&raw[..header_end], &url)?;
    let raw_body = &raw[body_start..];
    let body = if headers.is_chunked {
        decode_chunked(raw_body)?
    } else if let Some(len) = headers.content_length {
        raw_body[..len.min(raw_body.len())].to_vec()
    } else {
        raw_body.to_vec()
    };

    Ok((headers.status, headers.content_type, headers.location, body))
}

fn find_header_terminator(raw: &[u8]) -> Option<(usize, usize)> {
    let crlfcrlf = raw.windows(4).position(|w| w == b"\r\n\r\n");
    let lflf = raw.windows(2).position(|w| w == b"\n\n");
    match (crlfcrlf, lflf) {
        (Some(a), Some(b)) => {
            if a <= b {
                Some((a, a + 4))
            } else {
                Some((b, b + 2))
            }
        }
        (Some(a), None) => Some((a, a + 4)),
        (None, Some(b)) => Some((b, b + 2)),
        (None, None) => None,
    }
}

fn parse_status_line(line: &str, url: &Url) -> Result<(bool, u16), NetError> {
    let mut parts = line.split_whitespace();
    let proto = parts.next().unwrap_or("");
    if !proto
        .as_bytes()
        .get(..5)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"HTTP/"))
    {
        return Err(NetError {
            message: format!("Malformed HTTP status line from {url}: '{line}'"),
        });
    }
    let is_http_10 = proto.eq_ignore_ascii_case("HTTP/1.0");
    let code_str = parts.next().ok_or_else(|| NetError {
        message: format!("Missing HTTP status code from {url}: '{line}'"),
    })?;
    let status = code_str.parse::<u16>().map_err(|_| NetError {
        message: format!("Invalid HTTP status code '{code_str}' from {url}"),
    })?;
    Ok((is_http_10, status))
}

pub fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, NetError> {
    let mut out = Vec::new();
    let mut pos = 0;

    loop {
        let Some((line_rel, next_pos)) = find_line_end(&data[pos..]) else {
            return Err(NetError {
                message: "Unexpected EOF while reading chunk size".into(),
            });
        };
        let line_bytes = &data[pos..pos + line_rel];
        let line_str = std::str::from_utf8(line_bytes).map_err(|_| NetError {
            message: "Invalid UTF-8 in chunk size line".into(),
        })?;
        let hex_part = line_str.split(';').next().unwrap_or("").trim();
        if hex_part.is_empty() {
            return Err(NetError {
                message: "Empty HTTP chunk size".into(),
            });
        }
        let chunk_size = usize::from_str_radix(hex_part, 16).map_err(|_| NetError {
            message: format!("Invalid HTTP chunk size: '{hex_part}'"),
        })?;
        pos += next_pos;

        if chunk_size == 0 {
            break;
        }

        if pos + chunk_size > data.len() {
            return Err(NetError {
                message: "Unexpected EOF inside HTTP chunk payload".into(),
            });
        }
        out.extend_from_slice(&data[pos..pos + chunk_size]);
        pos += chunk_size;

        if data.get(pos..pos + 2) == Some(b"\r\n") {
            pos += 2;
        } else if data.get(pos) == Some(&b'\n') {
            pos += 1;
        } else {
            return Err(NetError {
                message: "Missing CRLF after HTTP chunk payload".into(),
            });
        }
    }

    Ok(out)
}

fn find_line_end(slice: &[u8]) -> Option<(usize, usize)> {
    for (i, &b) in slice.iter().enumerate() {
        if b == b'\n' {
            if i > 0 && slice[i - 1] == b'\r' {
                return Some((i - 1, i + 1));
            }
            return Some((i, i + 1));
        }
    }
    None
}

#[cfg(target_os = "linux")]
mod tls {
    use std::ffi::{CStr, CString};
    use std::io::{ErrorKind, Read, Write};
    use std::net::{IpAddr, TcpStream};
    use std::os::raw::{c_char, c_int, c_long, c_void};
    use std::os::unix::io::AsRawFd;
    use std::ptr;
    use std::sync::OnceLock;
    use std::time::Instant;

    use super::{strip_ipv6_brackets, NetError, HTTP_TIMEOUT};

    const RTLD_LAZY: c_int = 1;
    const SSL_VERIFY_PEER: c_int = 1;
    const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
    #[allow(non_upper_case_globals)]
    const TLSEXT_NAMETYPE_host_name: c_long = 0;
    const X509_V_OK: c_long = 0;

    const SSL_ERROR_WANT_READ: c_int = 2;
    const SSL_ERROR_WANT_WRITE: c_int = 3;
    const SSL_ERROR_SYSCALL: c_int = 5;
    const SSL_ERROR_ZERO_RETURN: c_int = 6;

    #[link(name = "c")]
    extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }

    #[allow(non_snake_case)]
    struct OpenSslRuntime {
        _handle: *mut c_void,
        ctx: *mut c_void,
        SSL_new: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        SSL_free: unsafe extern "C" fn(*mut c_void),
        SSL_set_fd: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
        SSL_ctrl: unsafe extern "C" fn(*mut c_void, c_int, c_long, *mut c_void) -> c_long,
        SSL_set1_host: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int,
        SSL_connect: unsafe extern "C" fn(*mut c_void) -> c_int,
        SSL_get_verify_result: unsafe extern "C" fn(*const c_void) -> c_long,
        SSL_read: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int) -> c_int,
        SSL_write: unsafe extern "C" fn(*mut c_void, *const c_void, c_int) -> c_int,
        SSL_get_error: unsafe extern "C" fn(*const c_void, c_int) -> c_int,
    }

    unsafe impl Send for OpenSslRuntime {}
    unsafe impl Sync for OpenSslRuntime {}

    static TLS_RUNTIME: OnceLock<Result<OpenSslRuntime, String>> = OnceLock::new();

    unsafe fn load_sym<T: Copy>(handle: *mut c_void, name: &CStr) -> Result<T, String> {
        let ptr = dlsym(handle, name.as_ptr());
        if ptr.is_null() {
            return Err(format!(
                "Failed to resolve OpenSSL symbol '{}'",
                name.to_string_lossy()
            ));
        }
        debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
        Ok(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
    }

    #[allow(non_snake_case)]
    impl OpenSslRuntime {
        fn load() -> Result<Self, String> {
            let handle = unsafe {
                let h4 = dlopen(c"libssl.so.4".as_ptr(), RTLD_LAZY);
                if !h4.is_null() {
                    h4
                } else {
                    let h3 = dlopen(c"libssl.so.3".as_ptr(), RTLD_LAZY);
                    if !h3.is_null() {
                        h3
                    } else {
                        dlopen(c"libssl.so".as_ptr(), RTLD_LAZY)
                    }
                }
            };
            if handle.is_null() {
                return Err(
                    "Could not dynamically load libssl.so.4, libssl.so.3, or libssl.so".into(),
                );
            }

            let init_res = unsafe { Self::init_with_handle(handle) };
            if init_res.is_err() {
                unsafe {
                    dlclose(handle);
                }
            }
            init_res
        }

        unsafe fn init_with_handle(handle: *mut c_void) -> Result<Self, String> {
            let OPENSSL_init_ssl: unsafe extern "C" fn(u64, *const c_void) -> c_int =
                load_sym(handle, c"OPENSSL_init_ssl")?;
            let TLS_client_method: unsafe extern "C" fn() -> *const c_void =
                load_sym(handle, c"TLS_client_method")?;
            let SSL_CTX_new: unsafe extern "C" fn(*const c_void) -> *mut c_void =
                load_sym(handle, c"SSL_CTX_new")?;
            let SSL_CTX_free: unsafe extern "C" fn(*mut c_void) =
                load_sym(handle, c"SSL_CTX_free")?;
            let SSL_CTX_set_default_verify_paths: unsafe extern "C" fn(*mut c_void) -> c_int =
                load_sym(handle, c"SSL_CTX_set_default_verify_paths")?;
            let SSL_CTX_set_verify: unsafe extern "C" fn(*mut c_void, c_int, *const c_void) =
                load_sym(handle, c"SSL_CTX_set_verify")?;

            let ssl_new = load_sym(handle, c"SSL_new")?;
            let ssl_free = load_sym(handle, c"SSL_free")?;
            let ssl_set_fd = load_sym(handle, c"SSL_set_fd")?;
            let ssl_ctrl = load_sym(handle, c"SSL_ctrl")?;
            let ssl_set1_host = load_sym(handle, c"SSL_set1_host")?;
            let ssl_connect = load_sym(handle, c"SSL_connect")?;
            let ssl_get_verify_result = load_sym(handle, c"SSL_get_verify_result")?;
            let ssl_read = load_sym(handle, c"SSL_read")?;
            let ssl_write = load_sym(handle, c"SSL_write")?;
            let ssl_get_error = load_sym(handle, c"SSL_get_error")?;

            OPENSSL_init_ssl(0, ptr::null());
            let method = TLS_client_method();
            if method.is_null() {
                return Err("OpenSSL TLS_client_method() returned null".into());
            }

            let ctx = SSL_CTX_new(method);
            if ctx.is_null() {
                return Err("OpenSSL SSL_CTX_new() returned null".into());
            }

            if SSL_CTX_set_default_verify_paths(ctx) != 1 {
                SSL_CTX_free(ctx);
                return Err("Failed to load system CA verify paths in OpenSSL".into());
            }
            SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, ptr::null());

            Ok(Self {
                _handle: handle,
                ctx,
                SSL_new: ssl_new,
                SSL_free: ssl_free,
                SSL_set_fd: ssl_set_fd,
                SSL_ctrl: ssl_ctrl,
                SSL_set1_host: ssl_set1_host,
                SSL_connect: ssl_connect,
                SSL_get_verify_result: ssl_get_verify_result,
                SSL_read: ssl_read,
                SSL_write: ssl_write,
                SSL_get_error: ssl_get_error,
            })
        }
    }

    fn runtime() -> Result<&'static OpenSslRuntime, NetError> {
        let res = TLS_RUNTIME.get_or_init(OpenSslRuntime::load);
        match res {
            Ok(rt) => Ok(rt),
            Err(msg) => Err(NetError {
                message: format!("HTTPS unavailable: {msg}"),
            }),
        }
    }

    pub struct TlsStream {
        ssl: *mut c_void,
        _tcp: TcpStream,
    }

    unsafe impl Send for TlsStream {}

    impl Drop for TlsStream {
        fn drop(&mut self) {
            if !self.ssl.is_null() {
                if let Ok(rt) = runtime() {
                    unsafe {
                        (rt.SSL_free)(self.ssl);
                    }
                }
                self.ssl = ptr::null_mut();
            }
        }
    }

    impl TlsStream {
        pub fn connect(tcp: TcpStream, host: &str) -> Result<Self, NetError> {
            let rt = runtime()?;
            let clean_host = strip_ipv6_brackets(host);
            let c_host = CString::new(clean_host).map_err(|_| NetError {
                message: format!("Invalid hostname containing NUL byte: '{host}'"),
            })?;

            let ssl = unsafe { (rt.SSL_new)(rt.ctx) };
            if ssl.is_null() {
                return Err(NetError {
                    message: format!("OpenSSL SSL_new() failed for '{host}'"),
                });
            }

            let stream = Self { ssl, _tcp: tcp };
            let fd = stream._tcp.as_raw_fd();
            if unsafe { (rt.SSL_set_fd)(stream.ssl, fd as c_int) } != 1 {
                return Err(NetError {
                    message: format!("OpenSSL SSL_set_fd() failed for '{host}'"),
                });
            }

            if clean_host.parse::<IpAddr>().is_err() {
                let sni_ok = unsafe {
                    (rt.SSL_ctrl)(
                        stream.ssl,
                        SSL_CTRL_SET_TLSEXT_HOSTNAME,
                        TLSEXT_NAMETYPE_host_name,
                        c_host.as_ptr() as *mut c_void,
                    )
                };
                if sni_ok <= 0 {
                    return Err(NetError {
                        message: format!("Failed setting TLS SNI hostname '{host}'"),
                    });
                }
            }

            if unsafe { (rt.SSL_set1_host)(stream.ssl, c_host.as_ptr()) } != 1 {
                return Err(NetError {
                    message: format!("Failed setting TLS verification host '{host}'"),
                });
            }

            let deadline = Instant::now() + HTTP_TIMEOUT;
            loop {
                let ret = unsafe { (rt.SSL_connect)(stream.ssl) };
                if ret == 1 {
                    break;
                }

                let ssl_err = unsafe { (rt.SSL_get_error)(stream.ssl, ret) };
                if ssl_err == SSL_ERROR_WANT_READ || ssl_err == SSL_ERROR_WANT_WRITE {
                    let os_err = std::io::Error::last_os_error();
                    if matches!(os_err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                        || Instant::now() >= deadline
                    {
                        return Err(NetError {
                            message: format!("TLS handshake timed out for '{host}'"),
                        });
                    }
                    if os_err.kind() == ErrorKind::Interrupted
                        || os_err.raw_os_error().unwrap_or(0) == 0
                    {
                        continue;
                    }
                    return Err(NetError {
                        message: format!("TLS handshake I/O error for '{host}': {os_err}"),
                    });
                }

                let verify_res = unsafe { (rt.SSL_get_verify_result)(stream.ssl) };
                if verify_res != X509_V_OK {
                    return Err(NetError {
                        message: format!(
                            "TLS certificate verification failed for '{host}' (X509 verify code {verify_res})"
                        ),
                    });
                }

                return Err(NetError {
                    message: format!("TLS handshake failed for '{host}' (SSL error {ssl_err})"),
                });
            }

            let verify_res = unsafe { (rt.SSL_get_verify_result)(stream.ssl) };
            if verify_res != X509_V_OK {
                return Err(NetError {
                    message: format!(
                        "TLS certificate verification failed for '{host}' (X509 verify code {verify_res})"
                    ),
                });
            }

            Ok(stream)
        }
    }

    impl Read for TlsStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let rt = runtime().map_err(|e| std::io::Error::other(e.message))?;
            let max_len = buf.len().min(c_int::MAX as usize) as c_int;
            let deadline = Instant::now() + HTTP_TIMEOUT;

            loop {
                let ret = unsafe { (rt.SSL_read)(self.ssl, buf.as_mut_ptr() as *mut c_void, max_len) };
                if ret > 0 {
                    return Ok(ret as usize);
                }

                let ssl_err = unsafe { (rt.SSL_get_error)(self.ssl, ret) };
                match ssl_err {
                    SSL_ERROR_ZERO_RETURN => return Ok(0),
                    SSL_ERROR_WANT_READ | SSL_ERROR_WANT_WRITE => {
                        let os_err = std::io::Error::last_os_error();
                        if matches!(os_err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                            || Instant::now() >= deadline
                        {
                            return Err(std::io::Error::new(
                                ErrorKind::TimedOut,
                                "TLS read timed out",
                            ));
                        }
                        if os_err.kind() == ErrorKind::Interrupted
                            || os_err.raw_os_error().unwrap_or(0) == 0
                        {
                            continue;
                        }
                        return Err(os_err);
                    }
                    SSL_ERROR_SYSCALL => {
                        let os_err = std::io::Error::last_os_error();
                        if os_err.raw_os_error().unwrap_or(0) == 0 {
                            return Ok(0);
                        }
                        return Err(os_err);
                    }
                    other => {
                        return Err(std::io::Error::other(format!(
                            "OpenSSL SSL_read error ({other})"
                        )));
                    }
                }
            }
        }
    }

    impl Write for TlsStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let rt = runtime().map_err(|e| std::io::Error::other(e.message))?;
            let max_len = buf.len().min(c_int::MAX as usize) as c_int;
            let deadline = Instant::now() + HTTP_TIMEOUT;

            loop {
                let ret =
                    unsafe { (rt.SSL_write)(self.ssl, buf.as_ptr() as *const c_void, max_len) };
                if ret > 0 {
                    return Ok(ret as usize);
                }

                let ssl_err = unsafe { (rt.SSL_get_error)(self.ssl, ret) };
                match ssl_err {
                    SSL_ERROR_ZERO_RETURN => {
                        return Err(std::io::Error::new(
                            ErrorKind::BrokenPipe,
                            "TLS connection closed",
                        ));
                    }
                    SSL_ERROR_WANT_READ | SSL_ERROR_WANT_WRITE => {
                        let os_err = std::io::Error::last_os_error();
                        if matches!(os_err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                            || Instant::now() >= deadline
                        {
                            return Err(std::io::Error::new(
                                ErrorKind::TimedOut,
                                "TLS write timed out",
                            ));
                        }
                        if os_err.kind() == ErrorKind::Interrupted
                            || os_err.raw_os_error().unwrap_or(0) == 0
                        {
                            continue;
                        }
                        return Err(os_err);
                    }
                    SSL_ERROR_SYSCALL => {
                        let os_err = std::io::Error::last_os_error();
                        if os_err.raw_os_error().unwrap_or(0) == 0 {
                            return Err(std::io::Error::new(
                                ErrorKind::BrokenPipe,
                                "TLS syscall EOF on write",
                            ));
                        }
                        return Err(os_err);
                    }
                    other => {
                        return Err(std::io::Error::other(format!(
                            "OpenSSL SSL_write error ({other})"
                        )));
                    }
                }
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod tls {
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpStream;

    use super::NetError;

    pub struct TlsStream;

    impl TlsStream {
        pub fn connect(_tcp: TcpStream, host: &str) -> Result<Self, NetError> {
            Err(NetError {
                message: format!(
                    "HTTPS to '{host}' is not yet enabled on this OS (Windows/non-Linux TLS will use Schannel/system TLS)"
                ),
            })
        }
    }

    impl Read for TlsStream {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "Non-Linux TLS is not enabled",
            ))
        }
    }

    impl Write for TlsStream {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "Non-Linux TLS is not enabled",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

fn fetch_about(url: &Url) -> Response {
    let (status, body) = match url.path.as_str() {
        "droplet" | "home" => (200, about_droplet_html()),
        "roadmap" => (200, about_roadmap_html()),
        "architecture" => (200, about_architecture_html()),
        "test" => (200, about_test_html()),
        "blank" => (200, about_blank_html()),
        other => (404, about_not_found_html(other)),
    };

    Response {
        url: url.clone(),
        status,
        content_type: "text/html; charset=utf-8".into(),
        body,
    }
}

const ABOUT_BASE_CSS: &str = r#"
body {
  background-color: #f4f7fb;
  color: #162032;
  font-size: 16px;
  margin: 0;
  padding: 20px;
}
.shell {
  background-color: #ffffff;
  border: 1px solid #cbd5e1;
  padding: 24px;
  margin-bottom: 16px;
}
.nav {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  background-color: #0f172a;
  color: #e2e8f0;
  padding: 12px 18px;
  margin-bottom: 18px;
}
.nav-links {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.nav a {
  display: inline-block;
  color: #7dd3fc;
  text-decoration: none;
  font-weight: bold;
  padding: 4px 10px;
  background-color: #1e293b;
  border: 1px solid #334155;
}
.nav a:hover {
  background-color: #2563eb;
  color: #ffffff;
  border: 1px solid #93c5fd;
}
.nav a:active {
  background-color: #1d4ed8;
  color: #ffffff;
}
.brand {
  color: #ffffff;
  font-weight: bold;
  font-size: 17px;
}
h1 {
  color: #0f172a;
  font-size: 26px;
  margin-top: 0;
  margin-bottom: 10px;
}
h2 {
  color: #1e3a8a;
  font-size: 20px;
  margin-top: 18px;
  margin-bottom: 8px;
}
h3 {
  color: #0f172a;
  font-size: 17px;
  margin-top: 12px;
  margin-bottom: 6px;
}
p {
  margin-top: 0;
  margin-bottom: 12px;
}
a {
  color: #1d4ed8;
}
a:hover {
  color: #1e40af;
  background-color: #dbeafe;
}
.flex-row {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: 12px;
  margin-bottom: 14px;
}
.flex-card {
  flex-grow: 1;
  flex-basis: 240px;
  background-color: #f8fafc;
  border: 1px solid #cbd5e1;
  padding: 14px 16px;
}
.flex-card:hover {
  background-color: #eff6ff;
  border: 1px solid #60a5fa;
}
.btn {
  display: inline-block;
  background-color: #1d4ed8;
  color: #ffffff;
  text-decoration: none;
  font-weight: bold;
  padding: 7px 14px;
  border: 1px solid #1e40af;
  margin-right: 8px;
  margin-bottom: 6px;
}
.btn:hover {
  background-color: #2563eb;
  color: #ffffff;
}
.btn:active {
  background-color: #1e3a8a;
  color: #ffffff;
}
.badge {
  display: inline-block;
  background-color: #dbeafe;
  color: #1e3a8a;
  border: 1px solid #93c5fd;
  padding: 2px 8px;
  font-size: 13px;
  font-weight: bold;
}
.card {
  background-color: #f8fafc;
  border: 1px solid #cbd5e1;
  padding: 14px 16px;
  margin-bottom: 12px;
}
.accent {
  background-color: #eff6ff;
  border: 1px solid #93c5fd;
  padding: 14px 16px;
  margin-bottom: 14px;
}
.done {
  background-color: #ecfdf5;
  border: 1px solid #6ee7b7;
  padding: 14px 16px;
  margin-bottom: 12px;
}
.meta {
  color: #475569;
  font-size: 14px;
}
pre {
  background-color: #0f172a;
  color: #e2e8f0;
  padding: 12px 14px;
  border: 1px solid #334155;
  margin-bottom: 12px;
}
code {
  background-color: #e2e8f0;
  color: #0f172a;
  padding: 2px 4px;
}
pre code {
  background-color: #0f172a;
  color: #e2e8f0;
  padding: 0;
}
ul {
  margin-top: 4px;
  margin-bottom: 12px;
  padding-left: 22px;
}
li {
  margin-bottom: 5px;
}
hr {
  border: 1px solid #cbd5e1;
  margin-top: 14px;
  margin-bottom: 14px;
}
"#;

fn about_nav_html(active: &str) -> String {
    format!(
        "<div class=\"nav\">\
         <span class=\"brand\">Droplet Engine</span>\
         <div class=\"nav-links\">\
         <a href=\"about:droplet\">Home{h}</a>\
         <a href=\"about:roadmap\">Roadmap{r}</a>\
         <a href=\"about:architecture\">Architecture{a}</a>\
         <a href=\"about:test\">Visual Test{t}</a>\
         <a href=\"about:blank\">Blank</a>\
         </div>\
         </div>",
        h = if active == "droplet" { " *" } else { "" },
        r = if active == "roadmap" { " *" } else { "" },
        a = if active == "architecture" { " *" } else { "" },
        t = if active == "test" { " *" } else { "" },
    )
}

fn about_droplet_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Fast Browser Engine</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Droplet Browser Engine</h1>
    <p class="meta">Zero-bloat graphical web browser written from scratch in pure Rust (&laquo;очень быстрый браузер без мусора&raquo;).</p>

    <div class="accent">
      <h3>Why Droplet? / Зачем Droplet?</h3>
      <p>Modern browsers ship tens of millions of lines of legacy code and eat gigabytes of RAM. Droplet targets sub-20ms cold startup, &lt; 15 MB idle RSS, and a crystal-clear pipeline built for speed.</p>
      <p>Цель Droplet &mdash; минимальный вес, мгновенный старт и чистая модульная архитектура без лишних зависимостей.</p>
    </div>

    <div class="flex-row">
      <div class="flex-card">
        <h3>Zero Bloat Core <span class="badge">Pure Rust</span></h3>
        <p>Zero Cargo dependency bloat. Dynamic <code>dlopen</code> for <code>libX11</code>, <code>libssl</code>, and system fonts keeps binary size tiny and builds sub-second.</p>
      </div>
      <div class="flex-card">
        <h3>Flexbox + :hover <span class="badge">Phase 2</span></h3>
        <p>Full <code>display: flex</code>, <code>inline-block</code>, relative units (<code>%</code>, <code>em</code>, <code>rem</code>, <code>vh</code>, <code>vw</code>), and reactive <code>:hover</code> / <code>:active</code> states.</p>
      </div>
      <div class="flex-card">
        <h3>HTTPS + Keep-Alive <span class="badge">Phase 3</span></h3>
        <p>Verified OpenSSL TLS, Keep-Alive connection pool, bounded 4-thread loader, 60s DNS cache, <code>data:</code> URIs, and zero-alloc tracker blocker.</p>
      </div>
    </div>

    <div class="card">
      <h2>Explore Built-In Pages</h2>
      <p>Hover and click any button below to test interactive <code>:hover</code> / <code>:active</code> highlights and Flexbox wrapping:</p>
      <div>
        <a class="btn" href="about:roadmap">Roadmap (Linux + Windows) &rarr;</a>
        <a class="btn" href="about:architecture">Low-RAM Architecture &rarr;</a>
        <a class="btn" href="about:test">Layout, Flexbox &amp; Cyrillic Suite &rarr;</a>
      </div>
    </div>

    <div class="card">
      <h2>Sample External HTTP &amp; HTTPS Links</h2>
      <p>Phase 3 enables verified <code>https://</code> via dynamic OpenSSL alongside <code>http://</code>, <code>file://</code>, <code>data:</code>, and <code>about:*</code>:</p>
      <ul>
        <li><a href="https://example.com/">https://example.com/ (TLS + HTTP/1.1 Keep-Alive)</a></li>
        <li><a href="http://example.com/">http://example.com/</a></li>
        <li><a href="http://info.cern.ch/">http://info.cern.ch/ (First website on the Web)</a></li>
      </ul>
    </div>
  </div>
</body>
</html>"#,
        nav = about_nav_html("droplet")
    )
}

fn about_roadmap_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Roadmap</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Droplet Development Roadmap (Linux + Windows)</h1>
    <p><a href="about:droplet">&larr; Back to about:droplet</a></p>

    <div class="flex-row">
      <div class="flex-card">
        <h3>Linux Target <span class="badge">Active</span></h3>
        <p>Dynamic <code>libX11.so.6</code> windowing, <code>libssl.so.4</code>/<code>.3</code> TLS, system font discovery, and headless CLI.</p>
      </div>
      <div class="flex-card">
        <h3>Windows Target <span class="badge">Portable</span></h3>
        <p>Clean OS boundaries ready for Win32 GDI/D3D presentation and native Schannel TLS without <code>Cargo.toml</code> bloat.</p>
      </div>
    </div>

    <div class="done">
      <h2>Phase 1 (Done) &mdash; Core Pipeline</h2>
      <ul>
        <li>URL parser &amp; relative dot-segment resolver (<code>http://</code>, <code>file://</code>, <code>about:*</code>).</li>
        <li>HTTP/1.1 client with 5s timeouts, <code>chunked</code> decoder, and redirect following.</li>
        <li>Stack-based HTML5-style parser + cache-friendly Arena DOM (<code>Vec&lt;Node&gt;</code>).</li>
        <li>CSS parser, selector specificity, inheritance, and Box Model + inline wrapping layout.</li>
      </ul>
    </div>

    <div class="done">
      <h2>Phase 2 (Done / DROP-002) &mdash; Inline-Block, Flexbox &amp; Pseudo-Classes</h2>
      <ul>
        <li><code>display: inline-block</code> and Flexbox (<code>flex-direction</code>, <code>justify-content</code>, <code>align-items</code>, <code>flex-wrap</code>, <code>gap</code>, <code>flex-grow</code>).</li>
        <li>Relative units (<code>%</code>, <code>em</code>, <code>rem</code>, <code>vh</code>, <code>vw</code>) and dirty-flagged <code>:hover</code> / <code>:active</code> styling.</li>
      </ul>
    </div>

    <div class="done">
      <h2>Phase 3 (Done / DROP-003) &mdash; HTTPS, Keep-Alive Pool &amp; Tracker Filter</h2>
      <ul>
        <li>Dynamic OpenSSL TLS (<code>SSL_VERIFY_PEER</code> + SNI + hostname check), Keep-Alive connection pool, 60s DNS cache.</li>
        <li>Concurrent 4-worker <code>fetch_many</code>, RFC 2397 <code>data:</code> URI decoder, and zero-alloc <code>TrackerFilter</code>.</li>
      </ul>
    </div>

    <div class="accent">
      <h2>Phase 4 (DROP-004) &mdash; TrueType Fonts &amp; Image Decoding</h2>
      <p>System TTF/OTF discovery, anti-aliased glyph atlas cache, PNG/JPEG/BMP/GIF/SVG decoding, and <code>&lt;img&gt;</code> replaced layout boxes.</p>
    </div>

    <div class="card">
      <h2>Phases 5&ndash;8 &mdash; Forms, Tabs, Scripting, GPU &amp; Sandbox</h2>
      <ul>
        <li><b>Phase 5</b>: Forms (<code>&lt;input&gt;</code>, <code>&lt;textarea&gt;</code>, <code>&lt;button&gt;</code>), focus ring, clipboard, text selection.</li>
        <li><b>Phase 6</b>: Multi-tab shell, binary/SQLite history &amp; bookmarks, Box Model DevTools overlay.</li>
        <li><b>Phase 7</b>: Minimalist ES subset bytecode VM &amp; incremental dirty DOM relayout.</li>
        <li><b>Phase 8</b>: GPU rasterization (<code>wgpu</code>/Vulkan or MIT-SHM) &amp; process sandbox.</li>
      </ul>
    </div>
  </div>
</body>
</html>"#,
        nav = about_nav_html("roadmap")
    )
}

fn about_architecture_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Architecture</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Droplet Low-RAM Architecture</h1>
    <p><a href="about:droplet">&larr; Back to about:droplet</a></p>

    <div class="accent">
      <h2>Rendering Pipeline</h2>
      <pre><code>URL -&gt; TrackerFilter -&gt; DNS Cache + Keep-Alive / TLS Pool
    -&gt; Concurrent Loader -&gt; HTML Parser -&gt; Arena DOM
    -&gt; CSS + Style Tree -&gt; Block / Inline / Flexbox Layout
    -&gt; Flat Display List -&gt; Software Rasterizer -&gt; X11 / PPM</code></pre>
    </div>

    <div class="flex-row">
      <div class="flex-card">
        <h3>Arena DOM <span class="badge">Vec&lt;Node&gt;</span></h3>
        <p>Nodes live contiguously in one vector addressed by integer <code>NodeId(usize)</code>. Zero <code>Rc&lt;RefCell&gt;</code> overhead and iterative traversals prevent stack overflow.</p>
      </div>
      <div class="flex-card">
        <h3>Bounded Caches <span class="badge">&lt; 15 MB RSS</span></h3>
        <p>DNS cache capped at 256 entries (60s TTL), connection pool capped at 32 sockets (30s idle TTL), and reusable framebuffers keep memory flat.</p>
      </div>
      <div class="flex-card">
        <h3>Dirty Repainting <span class="badge">~0% Idle CPU</span></h3>
        <p>Scrolling and hover transitions cull off-screen <code>DisplayCommand</code>s and only re-style or re-rasterize when dirty flags change.</p>
      </div>
    </div>

    <div class="card">
      <h2>Zero-Overhead Tracker Blocker (src/adblock.rs)</h2>
      <p>Uses a sorted static domain table with ASCII case-insensitive binary search over dot-separated suffixes &mdash; zero heap allocations per URL check, blocking trackers before DNS or TCP connect.</p>
    </div>
  </div>
</body>
</html>"#,
        nav = about_nav_html("architecture")
    )
}

fn about_test_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Visual Test Suite</title>
  <style>{ABOUT_BASE_CSS}
  .box-a {{
    background-color: #fef3c7;
    border: 2px solid #d97706;
    padding: 12px 16px;
    margin-bottom: 12px;
  }}
  .box-b {{
    background-color: #e0f2fe;
    border: 1px solid #0284c7;
    padding: 10px 14px;
    margin: 8px 0 14px 16px;
  }}
  .demo-flex {{
    display: flex;
    flex-direction: row;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    background-color: #f1f5f9;
    border: 1px solid #cbd5e1;
    padding: 12px;
    margin-bottom: 14px;
  }}
  .demo-item {{
    flex-grow: 1;
    background-color: #ffffff;
    border: 1px solid #94a3b8;
    padding: 10px;
  }}
  .demo-item:hover {{
    background-color: #dbeafe;
    border: 1px solid #2563eb;
  }}
  </style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Heading H1 &mdash; Тест рендеринга</h1>
    <h2>Heading H2 &mdash; Flexbox, :hover &amp; Box Model</h2>
    <h3>Heading H3 &mdash; Кириллица и ASCII</h3>

    <p>This paragraph tests inline text flow with <a href="about:droplet">clickable links (about:droplet)</a> embedded inside wrapping prose, alongside Russian Cyrillic text: Быстрая капля бежит по экрану без лишнего мусора и тормозов.</p>

    <div class="demo-flex">
      <div class="demo-item"><b>Flex Card 1</b>: hover highlight test</div>
      <div class="demo-item"><b>Flex Card 2</b>: equal <code>flex-grow: 1</code></div>
      <div class="demo-item"><b>Flex Card 3</b>: <code>gap: 12px</code></div>
    </div>

    <div class="box-a">
      <b>Box A</b>: 2px amber border, 12px/16px padding, 12px bottom margin.
      <div class="box-b">
        <b>Nested Box B</b>: 1px blue border with left margin offset and <a href="about:roadmap">link to Roadmap</a>.
      </div>
    </div>

    <hr>

    <h3>List &amp; Code Block</h3>
    <ul>
      <li>Элемент списка один &bull; Entity check: &laquo;Droplet&raquo; &ndash; fast &amp; tiny</li>
      <li>Элемент списка два &bull; Arrows: &larr; &rarr; &bull; Quotes: &ldquo;ok&rdquo;</li>
      <li>Nested link inside list: <a href="about:architecture">Open Architecture</a></li>
    </ul>

    <pre><code>fn hello_droplet() {{
    println!("Привет, Droplet! 2 &lt; 5 &amp;&amp; 5 &gt; 2");
}}</code></pre>
  </div>
</body>
</html>"#,
        nav = about_nav_html("test")
    )
}

fn about_blank_html() -> String {
    "<!DOCTYPE html><html><head><title>about:blank</title></head><body></body></html>".into()
}

fn about_not_found_html(page: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Droplet &mdash; Unknown Page</title>
  <style>{ABOUT_BASE_CSS}</style>
</head>
<body>
  <div class="shell">
    {nav}
    <h1>Unknown built-in page: about:{page}</h1>
    <p>Available built-in pages:</p>
    <ul>
      <li><a href="about:droplet">about:droplet</a></li>
      <li><a href="about:roadmap">about:roadmap</a></li>
      <li><a href="about:architecture">about:architecture</a></li>
      <li><a href="about:test">about:test</a></li>
      <li><a href="about:blank">about:blank</a></li>
    </ul>
  </div>
</body>
</html>"#,
        nav = about_nav_html("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn read_http_request_head(sock: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = sock.read(&mut tmp).unwrap();
            assert!(n > 0, "unexpected EOF reading request");
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[test]
    fn fetch_builtin_about_pages() {
        for page in [
            "about:droplet",
            "about:roadmap",
            "about:architecture",
            "about:test",
            "about:blank",
        ] {
            let url = Url::parse(page).unwrap();
            let resp = fetch(&url).unwrap();
            assert_eq!(resp.status, 200);
            assert!(resp.content_type.contains("text/html"));
            assert!(!resp.body.is_empty());
        }

        let missing = Url::parse("about:missing").unwrap();
        let resp = fetch(&missing).unwrap();
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn decode_chunked_handles_multiple_chunks_extensions_and_crlf() {
        let raw = b"5;ext=1\r\nHello\r\n2\r\n, \r\n8\r\nDroplet!\r\n0\r\nX-Trailer: 1\r\n\r\n";
        let decoded = decode_chunked(raw).unwrap();
        assert_eq!(decoded, b"Hello, Droplet!");
    }

    #[test]
    fn decode_chunked_rejects_truncated_and_invalid() {
        assert!(decode_chunked(b"").is_err());
        assert!(decode_chunked(b"ZZZZ\r\nhi\r\n0\r\n\r\n").is_err());
        assert!(decode_chunked(b"10\r\nhi\r\n0\r\n\r\n").is_err());
        assert!(decode_chunked(b"2\r\nhiXX0\r\n\r\n").is_err());
    }

    #[test]
    fn parse_http_response_plain_and_chunked_and_redirect() {
        let url = Url::parse("http://example.com/").unwrap();

        let plain = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 13\r\n\r\n<html></html>extra";
        let (status, ctype, loc, body) = parse_http_response(plain, url.clone()).unwrap();
        assert_eq!(status, 200);
        assert_eq!(ctype, "text/html; charset=utf-8");
        assert_eq!(loc, None);
        assert_eq!(body, b"<html></html>");

        let chunked = b"HTTP/1.1 200 OK\r\nTRANSFER-ENCODING: Chunked\r\n\r\n6\r\n<html>\r\n7\r\n</html>\r\n0\r\n\r\n";
        let (status, _, _, body) = parse_http_response(chunked, url.clone()).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"<html></html>");

        let redirect = b"HTTP/1.1 302 Found\r\nLocation: /next/page.html\r\n\r\n";
        let (status, _, loc, _) = parse_http_response(redirect, url).unwrap();
        assert_eq!(status, 302);
        assert_eq!(loc.as_deref(), Some("/next/page.html"));
    }

    #[test]
    fn fetch_file_url_reads_temp_file() {
        let path = std::env::temp_dir().join(format!(
            "droplet_net_test_page_{}.html",
            std::process::id()
        ));
        std::fs::write(&path, "<h1>Привет</h1>").unwrap();
        let url = Url::parse(path.to_str().unwrap()).unwrap();
        let resp = fetch(&url).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "<h1>Привет</h1>");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn keep_alive_reuses_single_tcp_connection_for_content_length_and_chunked() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (done_tx, done_rx) = mpsc::channel();

        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();

            let req1 = read_http_request_head(&mut sock);
            assert!(req1.starts_with("GET /first HTTP/1.1\r\n"));
            assert!(req1.to_ascii_lowercase().contains("connection: keep-alive"));
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: keep-alive\r\n\r\nHello One!!",
            )
            .unwrap();

            let req2 = read_http_request_head(&mut sock);
            assert!(req2.starts_with("GET /second HTTP/1.1\r\n"));
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n6\r\nChunk \r\n5\r\nTwo!!\r\n0\r\nX-Trailer: done\r\n\r\n",
            )
            .unwrap();

            let _ = done_rx.recv_timeout(Duration::from_secs(3));
        });

        let u1 = Url::parse(&format!("http://127.0.0.1:{}/first", addr.port())).unwrap();
        let u2 = Url::parse(&format!("http://127.0.0.1:{}/second", addr.port())).unwrap();

        let r1 = fetch(&u1).unwrap();
        assert_eq!(r1.status, 200);
        assert_eq!(r1.body, "Hello One!!");

        let r2 = fetch(&u2).unwrap();
        assert_eq!(r2.status, 200);
        assert_eq!(r2.body, "Chunk Two!!");

        let _ = done_tx.send(());
        server.join().unwrap();
    }

    #[test]
    fn stale_pooled_socket_triggers_transparent_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (closed_tx, closed_rx) = mpsc::channel();

        let server = std::thread::spawn(move || {
            let (mut sock1, _) = listener.accept().unwrap();
            sock1.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let req1 = read_http_request_head(&mut sock1);
            assert!(req1.starts_with("GET /one HTTP/1.1\r\n"));
            sock1
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: keep-alive\r\n\r\none",
                )
                .unwrap();
            drop(sock1);
            closed_tx.send(()).unwrap();

            let (mut sock2, _) = listener.accept().unwrap();
            sock2.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let req2 = read_http_request_head(&mut sock2);
            assert!(req2.starts_with("GET /two HTTP/1.1\r\n"));
            sock2
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\ntwo",
                )
                .unwrap();
        });

        let u1 = Url::parse(&format!("http://127.0.0.1:{}/one", addr.port())).unwrap();
        let u2 = Url::parse(&format!("http://127.0.0.1:{}/two", addr.port())).unwrap();

        let r1 = fetch(&u1).unwrap();
        assert_eq!(r1.body, "one");

        closed_rx.recv().unwrap();
        let r2 = fetch(&u2).unwrap();
        assert_eq!(r2.body, "two");

        server.join().unwrap();
    }

    #[test]
    fn fetch_many_runs_concurrently_and_preserves_order() {
        let mut listeners = Vec::new();
        let mut servers = Vec::new();
        let mut urls = Vec::new();

        for i in 0..5usize {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            urls.push(Url::parse(&format!("http://127.0.0.1:{port}/item/{i}")).unwrap());
            listeners.push((i, listener));
        }

        for (i, listener) in listeners {
            servers.push(std::thread::spawn(move || {
                let (mut sock, _) = listener.accept().unwrap();
                let _ = read_http_request_head(&mut sock);
                let payload = format!("payload-{i}");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                sock.write_all(resp.as_bytes()).unwrap();
            }));
        }

        let results = fetch_many(&urls);
        assert_eq!(results.len(), 5);
        for (i, res) in results.into_iter().enumerate() {
            let resp = res.unwrap();
            assert_eq!(resp.status, 200);
            assert_eq!(resp.body, format!("payload-{i}"));
        }

        for s in servers {
            s.join().unwrap();
        }
    }

    #[test]
    fn fetch_uri_bytes_supports_data_uris_and_file_urls() {
        let base = Url::parse("about:droplet").unwrap();

        let (ct1, bytes1) =
            fetch_uri_bytes("data:text/plain;base64,SGVsbG8sIERyb3BsZXQh", &base).unwrap();
        assert_eq!(ct1, "text/plain");
        assert_eq!(bytes1, b"Hello, Droplet!");

        let (ct2, bytes2) = fetch_uri_bytes("data:image/png;base64,iVBORw0KGgo=", &base).unwrap();
        assert_eq!(ct2, "image/png");
        assert_eq!(bytes2, b"\x89PNG\r\n\x1a\n");

        let (ct3, bytes3) = fetch_uri_bytes("data:,hi%20there%2B1", &base).unwrap();
        assert_eq!(ct3, "text/plain;charset=US-ASCII");
        assert_eq!(bytes3, b"hi there+1");

        assert!(fetch_uri_bytes("data:text/plain;base64,A", &base).is_err());
        assert!(fetch_uri_bytes("data:missingcomma", &base).is_err());

        let path = std::env::temp_dir().join(format!(
            "droplet_uri_bytes_test_{}.png",
            std::process::id()
        ));
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
        let file_base = Url::parse(&format!("file://{}/", std::env::temp_dir().display())).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        let (ct4, bytes4) = fetch_uri_bytes(filename, &file_base).unwrap();
        assert_eq!(ct4, "image/png");
        assert_eq!(bytes4, b"\x89PNG\r\n\x1a\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tracker_filter_blocks_before_network() {
        let tracker_url = Url::parse("https://ads.doubleclick.net/beacon.js").unwrap();
        let err = fetch_with_filter(&tracker_url, &TrackerFilter::with_defaults()).unwrap_err();
        assert!(err.message.contains("Blocked by tracker filter"));

        let mut custom = TrackerFilter::empty();
        custom.add_domain("blocked.local");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let redir_url = Url::parse(&format!("http://127.0.0.1:{port}/go")).unwrap();

        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let _ = read_http_request_head(&mut sock);
            sock.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: http://sub.blocked.local/pixel\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        });

        let redir_err = fetch_with_filter(&redir_url, &custom).unwrap_err();
        assert!(redir_err.message.contains("Blocked by tracker filter"));
        server.join().unwrap();
    }
}
