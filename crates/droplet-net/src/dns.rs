use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::NetError;

const DNS_TTL: Duration = Duration::from_secs(60);
const DNS_CACHE_MAX_ENTRIES: usize = 256;

struct DnsEntry {
    addrs: Vec<SocketAddr>,
    expires_at: Instant,
}

type DnsCacheMap = HashMap<(String, u16), DnsEntry>;

static DNS_CACHE: OnceLock<Mutex<DnsCacheMap>> = OnceLock::new();

fn dns_cache() -> &'static Mutex<DnsCacheMap> {
    DNS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn strip_ipv6_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host)
}

pub(crate) fn resolve_addrs(host: &str, port: u16) -> Result<(Vec<SocketAddr>, bool), NetError> {
    let key = (host.to_string(), port);
    let now = Instant::now();
    {
        let mut cache = dns_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, entry| entry.expires_at > now);
        if let Some(entry) = cache.get(&key) {
            return Ok((entry.addrs.clone(), true));
        }
    }

    let addrs = resolve_addrs_uncached(host, port)?;
    cache_dns_entry(key, addrs.clone());
    Ok((addrs, false))
}

pub(crate) fn resolve_addrs_uncached(host: &str, port: u16) -> Result<Vec<SocketAddr>, NetError> {
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

pub(crate) fn cache_dns_entry(key: (String, u16), addrs: Vec<SocketAddr>) {
    let now = Instant::now();
    let mut cache = dns_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, entry| entry.expires_at > now);
    if !cache.contains_key(&key) && cache.len() >= DNS_CACHE_MAX_ENTRIES {
        if let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.expires_at)
            .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest_key);
        }
    }
    cache.insert(
        key,
        DnsEntry {
            addrs,
            expires_at: now + DNS_TTL,
        },
    );
}

pub(crate) fn invalidate_dns_entry(host: &str, port: u16) {
    let mut cache = dns_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.remove(&(host.to_string(), port));
}
