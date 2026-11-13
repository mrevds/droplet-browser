use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use droplet_url::{Scheme, Url};

use crate::dns::{cache_dns_entry, invalidate_dns_entry, resolve_addrs, resolve_addrs_uncached};
use crate::tls::TlsStream;
use crate::{NetError, CONNECT_TIMEOUT, MAX_HEADER_BYTES, MAX_RESPONSE_BYTES, READ_TIMEOUT};

const POOL_IDLE_TTL: Duration = Duration::from_secs(30);
const POOL_MAX_PER_ORIGIN: usize = 4;
const POOL_MAX_TOTAL: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SchemeKey {
    Http,
    Https,
}

struct PooledConn {
    stream: TransportStream,
    last_used: Instant,
}

type PoolKey = (SchemeKey, String, u16);

static CONN_POOL: OnceLock<Mutex<HashMap<PoolKey, Vec<PooledConn>>>> = OnceLock::new();

fn conn_pool() -> &'static Mutex<HashMap<PoolKey, Vec<PooledConn>>> {
    CONN_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scheme_key(scheme: &Scheme) -> Option<SchemeKey> {
    match scheme {
        Scheme::Http => Some(SchemeKey::Http),
        Scheme::Https => Some(SchemeKey::Https),
        Scheme::File | Scheme::About => None,
    }
}

fn take_pooled_conn(key: &PoolKey) -> Option<TransportStream> {
    let now = Instant::now();
    let mut pool = conn_pool().lock().unwrap_or_else(|e| e.into_inner());
    let list = pool.get_mut(key)?;
    while let Some(conn) = list.pop() {
        if now.saturating_duration_since(conn.last_used) <= POOL_IDLE_TTL {
            if list.is_empty() {
                pool.remove(key);
            }
            return Some(conn.stream);
        }
    }
    pool.remove(key);
    None
}

fn return_pooled_conn(key: PoolKey, stream: TransportStream) {
    let now = Instant::now();
    let mut evicted: Vec<TransportStream> = Vec::new();
    {
        let mut pool = conn_pool().lock().unwrap_or_else(|e| e.into_inner());
        pool.retain(|_, list| {
            list.retain(|c| now.saturating_duration_since(c.last_used) <= POOL_IDLE_TTL);
            !list.is_empty()
        });

        let mut total_count: usize = pool.values().map(Vec::len).sum();
        while total_count >= POOL_MAX_TOTAL {
            let oldest_origin = pool
                .iter()
                .filter_map(|(k, v)| v.first().map(|c| (k.clone(), c.last_used)))
                .min_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k);
            let Some(origin_key) = oldest_origin else {
                break;
            };
            if let Some(vec) = pool.get_mut(&origin_key) {
                if !vec.is_empty() {
                    evicted.push(vec.remove(0).stream);
                    total_count -= 1;
                }
                if vec.is_empty() {
                    pool.remove(&origin_key);
                }
            }
        }

        let entry = pool.entry(key).or_default();
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
    Tls(TlsStream),
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

/// Tries every address in order within one shared `budget`. Each attempt gets an equal
/// share of what is left (so an unreachable first address cannot eat the whole budget),
/// but never less than `MIN_ATTEMPT` unless the budget itself is smaller.
fn connect_tcp_addrs(addrs: &[SocketAddr], budget: Duration) -> Result<TcpStream, std::io::Error> {
    const MIN_ATTEMPT: Duration = Duration::from_millis(1500);
    let deadline = Instant::now() + budget;
    let mut last_err = None;
    for (i, addr) in addrs.iter().enumerate() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let left = (addrs.len() - i) as u32;
        let attempt = (remaining / left).max(MIN_ATTEMPT.min(remaining));
        match TcpStream::connect_timeout(addr, attempt) {
            Ok(stream) => return Ok(stream),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::new(ErrorKind::TimedOut, "connect timed out")))
}

fn connect_stream(url: &Url) -> Result<TransportStream, NetError> {
    let (addrs, from_cache) = resolve_addrs(&url.host, url.port)?;
    let tcp = match connect_tcp_addrs(&addrs, CONNECT_TIMEOUT) {
        Ok(stream) => stream,
        Err(first_err) => {
            if from_cache {
                invalidate_dns_entry(&url.host, url.port);
                let fresh_addrs = resolve_addrs_uncached(&url.host, url.port)?;
                cache_dns_entry((url.host.clone(), url.port), fresh_addrs.clone());
                connect_tcp_addrs(&fresh_addrs, CONNECT_TIMEOUT).map_err(|err| NetError {
                    message: format!("connect to '{}:{}' failed: {err}", url.host, url.port),
                })?
            } else {
                return Err(NetError {
                    message: format!(
                        "connect to '{}:{}' failed: {first_err}",
                        url.host, url.port
                    ),
                });
            }
        }
    };

    // Idle limits for the HTTP exchange; the TLS handshake manages its own deadline.
    let _ = tcp.set_read_timeout(Some(READ_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(READ_TIMEOUT));
    let _ = tcp.set_nodelay(true);

    match url.scheme {
        Scheme::Http => Ok(TransportStream::Plain(tcp)),
        Scheme::Https => Ok(TransportStream::Tls(TlsStream::connect(tcp, &url.host)?)),
        Scheme::File | Scheme::About => Err(NetError {
            message: format!("Non-network scheme passed to connect_stream: {}", url.as_str()),
        }),
    }
}

pub(crate) struct RawHttpResponse {
    pub status: u16,
    pub content_type: String,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

enum ExchangeOutcome {
    Complete {
        parsed: RawHttpResponse,
        can_reuse: bool,
    },
    Stale,
}

pub(crate) fn send_http_get(url: &Url) -> Result<RawHttpResponse, NetError> {
    send_http_request(url, "GET", None, None)
}

pub(crate) fn send_http_post(
    url: &Url,
    body: &str,
    content_type: &str,
) -> Result<RawHttpResponse, NetError> {
    send_http_request(url, "POST", Some(body), Some(content_type))
}

fn send_http_request(
    url: &Url,
    method: &str,
    body: Option<&str>,
    content_type: Option<&str>,
) -> Result<RawHttpResponse, NetError> {
    let Some(skey) = scheme_key(&url.scheme) else {
        return Err(NetError {
            message: format!("Unsupported network scheme for {}", url.as_str()),
        });
    };
    let pool_key = (skey, url.host.clone(), url.port);

    if let Some(mut pooled) = take_pooled_conn(&pool_key) {
        match execute_http_exchange(&mut pooled, url, true, method, body, content_type)? {
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
    match execute_http_exchange(&mut fresh, url, false, method, body, content_type)? {
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
    method: &str,
    body: Option<&str>,
    content_type: Option<&str>,
) -> Result<ExchangeOutcome, NetError> {
    let target = url.request_target();
    let host_header = url.host_header();

    let mut request = format!(
        "{method} {target} HTTP/1.1\r\n\
         Host: {host_header}\r\n\
         User-Agent: Droplet/0.1\r\n\
         Accept: text/html,text/css,image/*,*/*\r\n\
         Accept-Encoding: gzip, deflate\r\n\
         Connection: keep-alive\r\n"
    );
    if let Some(body) = body {
        let content_type = content_type.unwrap_or("application/x-www-form-urlencoded");
        if content_type.contains('\r') || content_type.contains('\n') {
            return Err(NetError {
                message: "Invalid HTTP content type".into(),
            });
        }
        request.push_str(&format!(
            "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }

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
    /// `Content-Encoding` tokens in the order they were applied (lowercase).
    content_encoding: Vec<String>,
}

fn parse_headers_block(header_bytes: &[u8], url: &Url) -> Result<ParsedHeaders, NetError> {
    let header_text = String::from_utf8_lossy(header_bytes);
    let mut lines = header_text.split('\n');

    let status_line = lines
        .next()
        .map_or("", |l| l.trim_end_matches('\r').trim());
    let (is_http_10, status) = parse_status_line(status_line, url)?;

    let mut content_type = "text/html; charset=utf-8".to_string();
    let mut location = None;
    let mut is_chunked = false;
    let mut content_length: Option<usize> = None;
    let mut has_close = false;
    let mut has_keep_alive = false;
    let mut content_encoding: Vec<String> = Vec::new();

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
            "content-encoding" => content_encoding.extend(
                value
                    .split(',')
                    .map(|t| t.trim().to_ascii_lowercase())
                    .filter(|t| !t.is_empty()),
            ),
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
        content_encoding,
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

    let body = decode_content_encoding(body, &headers.content_encoding, url)?;

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

        while buf.len() - pos < chunk_size {
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
    let body = decode_content_encoding(body, &headers.content_encoding, &url)?;

    Ok((headers.status, headers.content_type, headers.location, body))
}

/// Undoes `Content-Encoding` (applied left to right, so removed right to left). The decoded
/// size is capped at `MAX_RESPONSE_BYTES` while inflating, so a compression bomb fails fast
/// instead of allocating (DROP-112).
fn decode_content_encoding(
    body: Vec<u8>,
    encodings: &[String],
    url: &Url,
) -> Result<Vec<u8>, NetError> {
    if body.is_empty() {
        return Ok(body);
    }
    let mut data = body;
    for encoding in encodings.iter().rev() {
        let decoded = match encoding.as_str() {
            "identity" => continue,
            "gzip" | "x-gzip" => droplet_inflate::gunzip(&data, MAX_RESPONSE_BYTES),
            // RFC 9110 says zlib, but some servers send raw DEFLATE: accept both.
            "deflate" if droplet_inflate::looks_like_zlib(&data) => {
                droplet_inflate::inflate_zlib(&data, MAX_RESPONSE_BYTES)
            }
            "deflate" => droplet_inflate::inflate_raw(&data, MAX_RESPONSE_BYTES),
            other => {
                return Err(NetError {
                    message: format!("Unsupported Content-Encoding '{other}' from '{}'", url.host),
                });
            }
        };
        data = decoded.map_err(|err| NetError {
            message: format!(
                "Failed to decode {encoding} response from '{}': {err}",
                url.host
            ),
        })?;
    }
    Ok(data)
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

        if chunk_size > data.len() - pos {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn connect_falls_through_refused_address_to_working_one() {
        // A port that was bound and released: connecting to it is refused immediately.
        let refused = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let live = TcpListener::bind("127.0.0.1:0").unwrap();
        let live_addr = live.local_addr().unwrap();
        let start = Instant::now();
        let stream = connect_tcp_addrs(&[refused, live_addr], Duration::from_secs(5))
            .expect("second address accepts");
        assert_eq!(stream.peer_addr().unwrap(), live_addr);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn connect_reports_last_error_when_every_address_fails() {
        let a = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let b = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let err = connect_tcp_addrs(&[a, b], Duration::from_secs(2)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConnectionRefused);
        let err = connect_tcp_addrs(&[], Duration::from_secs(1)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::TimedOut);
    }

    #[test]
    fn connect_error_message_names_the_phase() {
        let refused = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}/путь", refused.port())).unwrap();
        let err = match connect_stream(&url) {
            Err(err) => err,
            Ok(_) => panic!("refused port must not connect"),
        };
        assert!(err.message.starts_with("connect to '127.0.0.1:"), "{}", err.message);
    }
}
