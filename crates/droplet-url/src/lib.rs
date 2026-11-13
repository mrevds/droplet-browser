use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scheme {
    Http,
    Https,
    File,
    About,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub raw: String,
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlError {
    pub message: String,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UrlError {}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Url {
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(UrlError {
                message: "Empty URL".into(),
            });
        }

        if starts_with_ignore_ascii_case(trimmed, "about:") {
            return parse_about(&trimmed[6..]);
        }

        if starts_with_ignore_ascii_case(trimmed, "file://") {
            return parse_file(&trimmed[7..]);
        }

        if trimmed.starts_with('/') {
            let synthetic = format!("file://{trimmed}");
            return parse_file(&synthetic[7..]);
        }

        if starts_with_ignore_ascii_case(trimmed, "https://") {
            return parse_http_like(Scheme::Https, &trimmed[8..]);
        }

        if starts_with_ignore_ascii_case(trimmed, "http://") {
            return parse_http_like(Scheme::Http, &trimmed[7..]);
        }

        if !trimmed.contains("://") && !trimmed.contains(':') {
            return parse_http_like(Scheme::Http, trimmed);
        }

        Err(UrlError {
            message: format!("Unsupported URL scheme: {trimmed}"),
        })
    }

    pub fn resolve(&self, reference: &str) -> Result<Self, UrlError> {
        let r = reference.trim();
        if r.is_empty() {
            return Ok(self.with_parts(self.path.clone(), self.query.clone(), None));
        }

        if let Some(frag_raw) = r.strip_prefix('#') {
            let fragment = non_empty_opt(frag_raw);
            return Ok(self.with_parts(self.path.clone(), self.query.clone(), fragment));
        }

        if starts_with_ignore_ascii_case(r, "https://")
            || starts_with_ignore_ascii_case(r, "https:")
            || starts_with_ignore_ascii_case(r, "http://")
            || starts_with_ignore_ascii_case(r, "file://")
            || starts_with_ignore_ascii_case(r, "about:")
            || has_explicit_scheme(r)
        {
            return Url::parse(r);
        }

        if r.starts_with("//") {
            let scheme_prefix = match self.scheme {
                Scheme::File => "file:",
                Scheme::Https => "https:",
                Scheme::Http | Scheme::About => "http:",
            };
            return Url::parse(&format!("{scheme_prefix}{r}"));
        }

        if self.scheme == Scheme::About {
            return Err(UrlError {
                message: format!("Cannot resolve relative URL '{r}' against about: URL"),
            });
        }

        if let Some(rest) = r.strip_prefix('?') {
            let (query_str, fragment) = split_fragment(rest);
            return Ok(self.with_parts(
                self.path.clone(),
                non_empty_opt(query_str),
                fragment,
            ));
        }

        let (before_frag, fragment) = split_fragment(r);
        let (rel_path, query) = split_query(before_frag);

        let merged_path = if rel_path.starts_with('/') {
            normalize_path(rel_path)
        } else {
            let base_dir = match self.path.rfind('/') {
                Some(idx) => &self.path[..=idx],
                None => "/",
            };
            normalize_path(&format!("{base_dir}{rel_path}"))
        };

        Ok(self.with_parts(merged_path, query, fragment))
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn is_about(&self) -> bool {
        self.scheme == Scheme::About
    }

    pub fn host_header(&self) -> String {
        if self.host.is_empty() {
            return String::new();
        }
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let is_default_port = matches!(
            (&self.scheme, self.port),
            (Scheme::Http, 80) | (Scheme::Https, 443)
        );
        if is_default_port || self.port == 0 {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    pub fn request_target(&self) -> String {
        let path = if self.path.is_empty() {
            "/"
        } else {
            self.path.as_str()
        };
        match &self.query {
            Some(q) if !q.is_empty() => format!("{path}?{q}"),
            _ => path.to_string(),
        }
    }

    fn with_parts(&self, path: String, query: Option<String>, fragment: Option<String>) -> Self {
        let raw = build_raw(&self.scheme, &self.host, self.port, &path, &query, &fragment);
        Self {
            raw,
            scheme: self.scheme.clone(),
            host: self.host.clone(),
            port: self.port,
            path,
            query,
            fragment,
        }
    }
}

fn parse_about(rest: &str) -> Result<Url, UrlError> {
    let (before_frag, fragment) = split_fragment(rest);
    let (name_raw, query) = split_query(before_frag);
    let name = name_raw.trim().to_ascii_lowercase();
    if name.is_empty() {
        return Err(UrlError {
            message: "Invalid about: URL: missing page name".into(),
        });
    }
    let raw = build_raw(&Scheme::About, "", 0, &name, &query, &fragment);
    Ok(Url {
        raw,
        scheme: Scheme::About,
        host: String::new(),
        port: 0,
        path: name,
        query,
        fragment,
    })
}

fn parse_file(rest: &str) -> Result<Url, UrlError> {
    let (before_frag, fragment) = split_fragment(rest);
    let (before_query, query) = split_query(before_frag);
    if before_query.is_empty() {
        return Err(UrlError {
            message: "Invalid file:// URL: missing path".into(),
        });
    }

    let raw_path = if before_query.starts_with('/') {
        before_query
    } else if let Some(slash_idx) = before_query.find('/') {
        let authority = &before_query[..slash_idx];
        if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
            return Err(UrlError {
                message: format!("Unsupported remote file host: {authority}"),
            });
        }
        &before_query[slash_idx..]
    } else if before_query.eq_ignore_ascii_case("localhost") {
        "/"
    } else {
        return Err(UrlError {
            message: format!("Invalid file:// URL path: {before_query}"),
        });
    };

    let path = normalize_path(raw_path);
    let raw = build_raw(&Scheme::File, "", 0, &path, &query, &fragment);
    Ok(Url {
        raw,
        scheme: Scheme::File,
        host: String::new(),
        port: 0,
        path,
        query,
        fragment,
    })
}

fn default_port_for(scheme: &Scheme) -> u16 {
    match scheme {
        Scheme::Http => 80,
        Scheme::Https => 443,
        Scheme::File | Scheme::About => 0,
    }
}

fn parse_http_like(scheme: Scheme, rest: &str) -> Result<Url, UrlError> {
    let default_port = default_port_for(&scheme);
    let (before_frag, fragment) = split_fragment(rest);
    let (before_query, query) = split_query(before_frag);

    let (authority_raw, path_raw) = match before_query.find('/') {
        Some(idx) => (&before_query[..idx], &before_query[idx..]),
        None => (before_query, "/"),
    };

    let authority = match authority_raw.rsplit_once('@') {
        Some((_, host_part)) => host_part,
        None => authority_raw,
    };

    let (host, port) = parse_host_port(authority, default_port)?;
    let path = normalize_path(path_raw);
    let raw = build_raw(&scheme, &host, port, &path, &query, &fragment);

    Ok(Url {
        raw,
        scheme,
        host,
        port,
        path,
        query,
        fragment,
    })
}

fn parse_host_port(authority: &str, default_port: u16) -> Result<(String, u16), UrlError> {
    if authority.is_empty() {
        return Err(UrlError {
            message: "Missing host in URL".into(),
        });
    }

    if authority.starts_with('[') {
        let Some(close_idx) = authority.find(']') else {
            return Err(UrlError {
                message: format!("Malformed IPv6 host: {authority}"),
            });
        };
        let host = authority[..=close_idx].to_ascii_lowercase();
        if host.len() <= 2 {
            return Err(UrlError {
                message: "Empty IPv6 host".into(),
            });
        }
        let after = &authority[close_idx + 1..];
        let port = if after.is_empty() {
            default_port
        } else if let Some(port_str) = after.strip_prefix(':') {
            parse_port_str(port_str, default_port)?
        } else {
            return Err(UrlError {
                message: format!("Invalid authority after IPv6 host: {authority}"),
            });
        };
        return Ok((host, port));
    }

    if let Some((host_raw, port_str)) = authority.split_once(':') {
        let host = host_raw.to_ascii_lowercase();
        if host.is_empty() {
            return Err(UrlError {
                message: "Missing host before port".into(),
            });
        }
        let port = parse_port_str(port_str, default_port)?;
        Ok((host, port))
    } else {
        Ok((authority.to_ascii_lowercase(), default_port))
    }
}

fn parse_port_str(port_str: &str, default_port: u16) -> Result<u16, UrlError> {
    if port_str.is_empty() {
        return Ok(default_port);
    }
    let port = port_str.parse::<u16>().map_err(|_| UrlError {
        message: format!("Invalid port: {port_str}"),
    })?;
    if port == 0 {
        return Err(UrlError {
            message: "Port 0 is not valid".into(),
        });
    }
    Ok(port)
}

fn split_fragment(input: &str) -> (&str, Option<String>) {
    match input.split_once('#') {
        Some((before, frag)) => (before, non_empty_opt(frag)),
        None => (input, None),
    }
}

fn split_query(input: &str) -> (&str, Option<String>) {
    match input.split_once('?') {
        Some((before, q)) => (before, non_empty_opt(q)),
        None => (input, None),
    }
}

fn non_empty_opt(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn build_raw(
    scheme: &Scheme,
    host: &str,
    port: u16,
    path: &str,
    query: &Option<String>,
    fragment: &Option<String>,
) -> String {
    let mut raw = match scheme {
        Scheme::Http => {
            if port == 80 {
                format!("http://{host}{path}")
            } else {
                format!("http://{host}:{port}{path}")
            }
        }
        Scheme::Https => {
            if port == 443 {
                format!("https://{host}{path}")
            } else {
                format!("https://{host}:{port}{path}")
            }
        }
        Scheme::File => format!("file://{path}"),
        Scheme::About => format!("about:{path}"),
    };
    if let Some(q) = query {
        raw.push('?');
        raw.push_str(q);
    }
    if let Some(f) = fragment {
        raw.push('#');
        raw.push_str(f);
    }
    raw
}

fn normalize_path(path: &str) -> String {
    let trailing_slash = path.ends_with('/')
        || path == "."
        || path == ".."
        || path.ends_with("/.")
        || path.ends_with("/..");

    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            s => stack.push(s),
        }
    }

    if stack.is_empty() {
        return "/".to_string();
    }

    let mut out = String::new();
    for seg in stack {
        out.push('/');
        out.push_str(seg);
    }
    if trailing_slash {
        out.push('/');
    }
    out
}

fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn has_explicit_scheme(reference: &str) -> bool {
    let before_delim = reference.split(['/', '?', '#']).next().unwrap_or("");
    let Some((scheme_part, _)) = before_delim.split_once(':') else {
        return false;
    };
    let mut bytes = scheme_part.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_about_urls() {
        let u = Url::parse("about:droplet").unwrap();
        assert_eq!(u.scheme, Scheme::About);
        assert_eq!(u.path, "droplet");
        assert_eq!(u.as_str(), "about:droplet");
        assert!(u.is_about());

        let u2 = Url::parse("ABOUT:RoadMap#phase2").unwrap();
        assert_eq!(u2.scheme, Scheme::About);
        assert_eq!(u2.path, "roadmap");
        assert_eq!(u2.fragment.as_deref(), Some("phase2"));
        assert_eq!(u2.as_str(), "about:roadmap#phase2");
    }

    #[test]
    fn parse_file_and_bare_filesystem_paths() {
        let bare = Url::parse("/foo/bar.html").unwrap();
        assert_eq!(bare.scheme, Scheme::File);
        assert_eq!(bare.path, "/foo/bar.html");
        assert_eq!(bare.as_str(), "file:///foo/bar.html");

        let file_url = Url::parse("file:///tmp/../home/user/test.html?a=1#top").unwrap();
        assert_eq!(file_url.scheme, Scheme::File);
        assert_eq!(file_url.path, "/home/user/test.html");
        assert_eq!(file_url.query.as_deref(), Some("a=1"));
        assert_eq!(file_url.fragment.as_deref(), Some("top"));
        assert_eq!(file_url.as_str(), "file:///home/user/test.html?a=1#top");

        let localhost = Url::parse("file://localhost/etc/hosts").unwrap();
        assert_eq!(localhost.path, "/etc/hosts");
        assert_eq!(localhost.as_str(), "file:///etc/hosts");
    }

    #[test]
    fn parse_http_urls_and_bare_domains() {
        let bare = Url::parse("example.com").unwrap();
        assert_eq!(bare.scheme, Scheme::Http);
        assert_eq!(bare.host, "example.com");
        assert_eq!(bare.port, 80);
        assert_eq!(bare.path, "/");
        assert_eq!(bare.as_str(), "http://example.com/");

        let full = Url::parse("HTTP://Example.COM:8080/a/b/../c?x=1&y=2#sec").unwrap();
        assert_eq!(full.scheme, Scheme::Http);
        assert_eq!(full.host, "example.com");
        assert_eq!(full.port, 8080);
        assert_eq!(full.path, "/a/c");
        assert_eq!(full.query.as_deref(), Some("x=1&y=2"));
        assert_eq!(full.fragment.as_deref(), Some("sec"));
        assert_eq!(full.to_string(), "http://example.com:8080/a/c?x=1&y=2#sec");

        let ipv6 = Url::parse("http://[::1]:3000/status").unwrap();
        assert_eq!(ipv6.host, "[::1]");
        assert_eq!(ipv6.port, 3000);
        assert_eq!(ipv6.path, "/status");
    }

    #[test]
    fn reject_invalid_urls() {
        assert!(Url::parse("").is_err());
        assert!(Url::parse("http://").is_err());
        assert!(Url::parse("https://").is_err());
        assert!(Url::parse("https:example.com").is_err());
        assert!(Url::parse("http://example.com:notaport").is_err());
        assert!(Url::parse("https://example.com:0/path").is_err());
        assert!(Url::parse("ftp://example.com/file").is_err());
        assert!(Url::parse("about:").is_err());
    }

    #[test]
    fn parse_https_urls_and_defaults() {
        let bare = Url::parse("https://example.com/a?x=1").unwrap();
        assert_eq!(bare.scheme, Scheme::Https);
        assert_eq!(bare.host, "example.com");
        assert_eq!(bare.port, 443);
        assert_eq!(bare.path, "/a");
        assert_eq!(bare.query.as_deref(), Some("x=1"));
        assert_eq!(bare.fragment, None);
        assert_eq!(bare.as_str(), "https://example.com/a?x=1");

        let explicit_default = Url::parse("HTTPS://Example.COM:443/a/../b#top").unwrap();
        assert_eq!(explicit_default.scheme, Scheme::Https);
        assert_eq!(explicit_default.host, "example.com");
        assert_eq!(explicit_default.port, 443);
        assert_eq!(explicit_default.path, "/b");
        assert_eq!(explicit_default.fragment.as_deref(), Some("top"));
        assert_eq!(explicit_default.as_str(), "https://example.com/b#top");

        let empty_port = Url::parse("https://example.com:/status").unwrap();
        assert_eq!(empty_port.port, 443);
        assert_eq!(empty_port.as_str(), "https://example.com/status");

        let custom = Url::parse("https://api.example.com:8443/v1/items?limit=10").unwrap();
        assert_eq!(custom.scheme, Scheme::Https);
        assert_eq!(custom.host, "api.example.com");
        assert_eq!(custom.port, 8443);
        assert_eq!(custom.path, "/v1/items");
        assert_eq!(custom.query.as_deref(), Some("limit=10"));
        assert_eq!(custom.as_str(), "https://api.example.com:8443/v1/items?limit=10");

        let ipv6 = Url::parse("https://[::1]/metrics").unwrap();
        assert_eq!(ipv6.scheme, Scheme::Https);
        assert_eq!(ipv6.host, "[::1]");
        assert_eq!(ipv6.port, 443);
        assert_eq!(ipv6.path, "/metrics");
    }

    #[test]
    fn host_header_and_request_target() {
        let http_default = Url::parse("http://example.com/index.html?q=1#frag").unwrap();
        assert_eq!(http_default.host_header(), "example.com");
        assert_eq!(http_default.request_target(), "/index.html?q=1");

        let http_custom = Url::parse("http://example.com:8080").unwrap();
        assert_eq!(http_custom.host_header(), "example.com:8080");
        assert_eq!(http_custom.request_target(), "/");

        let http_443 = Url::parse("http://example.com:443/x").unwrap();
        assert_eq!(http_443.host_header(), "example.com:443");

        let https_default = Url::parse("https://example.com/app.css?v=2#s").unwrap();
        assert_eq!(https_default.host_header(), "example.com");
        assert_eq!(https_default.request_target(), "/app.css?v=2");

        let https_custom = Url::parse("https://example.com:8443/api").unwrap();
        assert_eq!(https_custom.host_header(), "example.com:8443");
        assert_eq!(https_custom.request_target(), "/api");

        let https_80 = Url::parse("https://example.com:80/api").unwrap();
        assert_eq!(https_80.host_header(), "example.com:80");

        let ipv6_default = Url::parse("https://[::1]/health").unwrap();
        assert_eq!(ipv6_default.host_header(), "[::1]");

        let ipv6_custom = Url::parse("https://[::1]:8443/health").unwrap();
        assert_eq!(ipv6_custom.host_header(), "[::1]:8443");

        let unbracketed_ipv6 = Url {
            raw: "https://[::1]:8443/".into(),
            scheme: Scheme::Https,
            host: "::1".into(),
            port: 8443,
            path: "/".into(),
            query: None,
            fragment: None,
        };
        assert_eq!(unbracketed_ipv6.host_header(), "[::1]:8443");
    }

    #[test]
    fn resolve_relative_and_dot_segments() {
        let base = Url::parse("http://example.com/dir/sub/page.html?v=1#old").unwrap();

        assert_eq!(
            base.resolve("#new").unwrap().as_str(),
            "http://example.com/dir/sub/page.html?v=1#new"
        );
        assert_eq!(
            base.resolve("about:roadmap").unwrap().as_str(),
            "about:roadmap"
        );
        assert_eq!(
            base.resolve("http://other.org/x").unwrap().as_str(),
            "http://other.org/x"
        );
        assert_eq!(
            base.resolve("//cdn.example.com/app.css").unwrap().as_str(),
            "http://cdn.example.com/app.css"
        );
        assert_eq!(
            base.resolve("/abs/path.html").unwrap().as_str(),
            "http://example.com/abs/path.html"
        );
        assert_eq!(
            base.resolve("foo.html").unwrap().as_str(),
            "http://example.com/dir/sub/foo.html"
        );
        assert_eq!(
            base.resolve("../bar.html").unwrap().as_str(),
            "http://example.com/dir/bar.html"
        );
        assert_eq!(
            base.resolve("./baz/").unwrap().as_str(),
            "http://example.com/dir/sub/baz/"
        );
        assert_eq!(
            base.resolve("../../../outside.html").unwrap().as_str(),
            "http://example.com/outside.html"
        );
        assert_eq!(
            base.resolve("?q=2#top").unwrap().as_str(),
            "http://example.com/dir/sub/page.html?q=2#top"
        );
        assert_eq!(
            base.resolve("https://secure.example.com").unwrap().as_str(),
            "https://secure.example.com/"
        );
    }

    #[test]
    fn resolve_against_https_base() {
        let https_base = Url::parse("https://example.com/docs/index.html").unwrap();

        let proto_rel = https_base.resolve("//cdn.example.com/app.css").unwrap();
        assert_eq!(proto_rel.scheme, Scheme::Https);
        assert_eq!(proto_rel.port, 443);
        assert_eq!(proto_rel.as_str(), "https://cdn.example.com/app.css");

        let proto_rel_port = https_base.resolve("//cdn.example.com:8443/app.css").unwrap();
        assert_eq!(proto_rel_port.scheme, Scheme::Https);
        assert_eq!(proto_rel_port.port, 8443);
        assert_eq!(
            proto_rel_port.as_str(),
            "https://cdn.example.com:8443/app.css"
        );

        assert_eq!(
            https_base.resolve("styles/main.css").unwrap().as_str(),
            "https://example.com/docs/styles/main.css"
        );
        assert_eq!(
            https_base.resolve("/root.css").unwrap().as_str(),
            "https://example.com/root.css"
        );
        assert_eq!(
            https_base.resolve("http://insecure.example.com/a").unwrap().as_str(),
            "http://insecure.example.com/a"
        );
    }

    #[test]
    fn resolve_against_file_and_about() {
        let file_base = Url::parse("file:///home/user/docs/index.html").unwrap();
        assert_eq!(
            file_base.resolve("../img/pic.html").unwrap().as_str(),
            "file:///home/user/img/pic.html"
        );

        let about_base = Url::parse("about:droplet").unwrap();
        assert_eq!(
            about_base.resolve("about:test").unwrap().as_str(),
            "about:test"
        );
        assert_eq!(
            about_base.resolve("#top").unwrap().as_str(),
            "about:droplet#top"
        );
        assert!(about_base.resolve("relative.html").is_err());
    }
}
