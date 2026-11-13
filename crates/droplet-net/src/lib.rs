mod about;
pub mod adblock;
mod dns;
mod pool;
mod tls;
#[cfg(target_os = "windows")]
mod tls_schannel;
pub mod user_pages;

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use droplet_url::{Scheme, Url};

use adblock::TrackerFilter;

pub use pool::{decode_chunked, parse_http_response};

/// Total budget for TCP connect across all resolved addresses (DROP-111). Lossy links need
/// SYN retransmits at 1 s and 3 s, so a few seconds is not enough.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Total budget for the TLS handshake. A single lost server flight costs 1 + 2 + 4 s of
/// retransmission backoff, so this must exceed any one socket wait.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest silence tolerated while waiting for response bytes (first byte and between reads).
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REDIRECTS: usize = 5;
pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_CONCURRENT_FETCHES: usize = 4;

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

static DEFAULT_TRACKER_FILTER: OnceLock<TrackerFilter> = OnceLock::new();

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

    let mut ordered: Vec<Option<Result<Response, NetError>>> =
        (0..urls.len()).map(|_| None).collect();
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

/// Sends a form-style HTTP POST and returns the decoded response.
///
/// Redirects follow browser semantics: 301/302/303 switch to GET, while
/// 307/308 preserve the POST body. Non-network schemes return an error.
pub fn fetch_post(
    url: &Url,
    body: &str,
    content_type: &str,
) -> Result<Response, NetError> {
    fetch_post_with_filter(url, body, content_type, default_filter())
}

pub fn fetch_post_with_filter(
    url: &Url,
    body: &str,
    content_type: &str,
    filter: &TrackerFilter,
) -> Result<Response, NetError> {
    let (final_url, status, content_type, body_bytes) =
        fetch_post_raw_with_filter(url, body, content_type, filter)?;
    let body = String::from_utf8(body_bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    Ok(Response {
        url: final_url,
        status,
        content_type,
        body,
    })
}

fn fetch_post_raw_with_filter(
    initial_url: &Url,
    initial_body: &str,
    initial_content_type: &str,
    filter: &TrackerFilter,
) -> Result<(Url, u16, String, Vec<u8>), NetError> {
    if !matches!(initial_url.scheme, Scheme::Http | Scheme::Https) {
        return Err(NetError {
            message: format!(
                "POST is only supported for HTTP(S) URLs, got '{}'",
                initial_url.as_str()
            ),
        });
    }

    let mut current_url = initial_url.clone();
    let mut method = "POST";
    let mut body = Some(initial_body.to_string());
    let mut content_type = initial_content_type.to_string();

    for hop in 0..=MAX_REDIRECTS {
        if filter.should_block(&current_url.host) {
            return Err(NetError {
                message: format!("Blocked by tracker filter: {}", current_url.host),
            });
        }

        let resp = if method == "POST" {
            pool::send_http_post(
                &current_url,
                body.as_deref().unwrap_or(""),
                &content_type,
            )?
        } else {
            pool::send_http_get(&current_url)?
        };

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
                if matches!(resp.status, 301..=303) {
                    method = "GET";
                    body = None;
                    content_type.clear();
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

fn fetch_raw_with_filter(
    url: &Url,
    filter: &TrackerFilter,
) -> Result<(Url, u16, String, Vec<u8>), NetError> {
    match url.scheme {
        Scheme::About => {
            let resp = about::fetch_about(url);
            Ok((
                resp.url,
                resp.status,
                resp.content_type,
                resp.body.into_bytes(),
            ))
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

        let resp = pool::send_http_get(&current_url)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
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

    #[test]
    fn fetch_post_sends_url_encoded_body_and_follows_303_to_get() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = Url::parse(&format!("http://127.0.0.1:{port}/submit")).unwrap();

        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 1024];
            let header_end = loop {
                let n = sock.read(&mut buf).unwrap();
                assert!(n > 0);
                raw.extend_from_slice(&buf[..n]);
                if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
            assert!(head.starts_with("POST /submit HTTP/1.1\r\n"));
            let cl: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim().eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())
                        .flatten()
                })
                .expect("Content-Length");
            while raw.len() < header_end + cl {
                let n = sock.read(&mut buf).unwrap();
                assert!(n > 0);
                raw.extend_from_slice(&buf[..n]);
            }
            let body = String::from_utf8_lossy(&raw[header_end..header_end + cl]).to_string();
            assert_eq!(body, "user=%D0%98%D0%B2%D0%B0%D0%BD&ok=1");
            sock.write_all(
                b"HTTP/1.1 303 See Other\r\nLocation: /done\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();

            let (mut sock2, _) = listener.accept().unwrap();
            let req2 = read_http_request_head(&mut sock2);
            assert!(req2.starts_with("GET /done HTTP/1.1\r\n"));
            let resp_body = "<html><body>OK</body></html>";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            sock2.write_all(resp.as_bytes()).unwrap();
        });

        let resp = fetch_post(
            &url,
            "user=%D0%98%D0%B2%D0%B0%D0%BD&ok=1",
            "application/x-www-form-urlencoded",
        )
        .unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.url.as_str().ends_with("/done"));
        assert!(resp.body.contains("OK"));
        server.join().unwrap();
    }

    // ---- DROP-112: Content-Encoding ---------------------------------------------------
    const HTTP_GZ_HTML: &[u8] = &[31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 179, 201, 40, 201, 205, 177, 179, 201, 72, 77, 76, 177, 179, 41, 201, 44, 201, 73, 181, 187, 176, 240, 194, 182, 11, 27, 46, 54, 93, 216, 103, 163, 15, 17, 177, 209, 135, 200, 39, 229, 167, 84, 218, 217, 20, 216, 93, 152, 127, 177, 225, 194, 142, 11, 155, 46, 108, 189, 216, 164, 163, 144, 94, 149, 89, 160, 104, 163, 95, 48, 42, 51, 42, 51, 42, 51, 242, 100, 244, 33, 197, 130, 62, 184, 36, 1, 0, 138, 145, 148, 234, 80, 4, 0, 0];
    const HTTP_RAW_DEFLATE: &[u8] = &[179, 41, 176, 43, 74, 44, 87, 72, 73, 77, 203, 73, 44, 73, 85, 184, 216, 116, 97, 235, 133, 221, 23, 246, 217, 232, 23, 216, 1, 0];
    const HTTP_GZ_HTML_LEN: usize = 1104;


    /// Serves one request with the given raw response; returns (url, request-head receiver).
    fn one_shot_server(response: Vec<u8>) -> (Url, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let head = read_http_request_head(&mut sock);
            let _ = tx.send(head);
            let _ = sock.write_all(&response);
        });
        let url = Url::parse(&format!("http://127.0.0.1:{}/page", addr.port())).unwrap();
        (url, rx, handle)
    }

    fn chunked(body: &[u8], chunk: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for part in body.chunks(chunk) {
            out.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
            out.extend_from_slice(part);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"0\r\n\r\n");
        out
    }

    #[test]
    fn chunked_gzip_response_is_decoded_and_gzip_is_advertised() {
        let mut resp = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        resp.extend_from_slice(&chunked(HTTP_GZ_HTML, 37));
        let (url, head_rx, server) = one_shot_server(resp);
        let r = fetch(&url).unwrap();
        let head = head_rx.recv().unwrap().to_ascii_lowercase();
        assert!(head.contains("accept-encoding: gzip, deflate"), "{head}");
        assert_eq!(r.status, 200);
        assert_eq!(r.body.len(), HTTP_GZ_HTML_LEN);
        assert!(r.body.starts_with("<html><head><title>Сжато</title>"));
        assert_eq!(r.body.matches("Привет, gzip!").count(), 40);
        server.join().unwrap();
    }

    #[test]
    fn raw_deflate_with_content_length_is_decoded() {
        let mut resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: deflate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            HTTP_RAW_DEFLATE.len()
        )
        .into_bytes();
        resp.extend_from_slice(HTTP_RAW_DEFLATE);
        let (url, _rx, server) = one_shot_server(resp);
        assert_eq!(fetch(&url).unwrap().body, "<p>raw deflate тело</p>");
        server.join().unwrap();
    }

    #[test]
    fn corrupt_and_unsupported_encodings_are_reported_not_rendered() {
        let mut bad = HTTP_GZ_HTML.to_vec();
        let n = bad.len();
        bad[n - 6] ^= 0x55; // CRC32 trailer byte
        let mut resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        resp.extend_from_slice(&bad);
        let (url, _rx, server) = one_shot_server(resp);
        let err = fetch(&url).unwrap_err();
        assert!(err.message.contains("Failed to decode gzip") && err.message.contains("CRC-32"), "{}", err.message);
        server.join().unwrap();

        let resp = b"HTTP/1.1 200 OK\r\nContent-Encoding: br\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec();
        let (url, _rx, server) = one_shot_server(resp);
        let err = fetch(&url).unwrap_err();
        assert!(err.message.contains("Unsupported Content-Encoding 'br'"), "{}", err.message);
        server.join().unwrap();
    }

    #[test]
    fn identity_encoding_and_empty_bodies_pass_through() {
        let body = "чисто".as_bytes();
        let mut resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: identity\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        resp.extend_from_slice(body);
        let (url, _rx, server) = one_shot_server(resp);
        assert_eq!(fetch(&url).unwrap().body, "чисто");
        server.join().unwrap();

        let (status, _, _, body) = parse_http_response(
            b"HTTP/1.1 304 Not Modified\r\nContent-Encoding: gzip\r\n\r\n",
            Url::parse("http://example.com/").unwrap(),
        )
        .unwrap();
        assert_eq!(status, 304);
        assert!(body.is_empty());
    }
}

