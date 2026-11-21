//! Trusted-proxy-aware client IP resolution.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;

#[derive(Debug, Clone, Copy)]
pub struct TrustedClientIp(pub IpAddr);

#[derive(Debug, Clone)]
struct TrustedCidr {
    addr: IpAddr,
    prefix_len: u8,
}

/// Parsed CIDR list, cheap to clone (Arc).
#[derive(Clone)]
pub struct CidrList(Arc<Vec<TrustedCidr>>);

impl CidrList {
    /// Parses `server.trusted_proxies`. Configuration validation refuses an
    /// entry [`is_cidr`] does not accept, so none is dropped here in practice.
    pub fn parse(entries: &[String]) -> Self {
        let cidrs: Vec<TrustedCidr> = entries.iter().filter_map(|entry| parse_cidr(entry)).collect();
        Self(Arc::new(cidrs))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The middleware function. Use with `axum::middleware::from_fn_with_state`.
pub async fn middleware(State(cidrs): State<CidrList>, mut request: Request, next: Next) -> Response {
    let ip = resolve(&request, &cidrs.0);
    request.extensions_mut().insert(TrustedClientIp(ip));
    next.run(request).await
}

fn resolve(request: &Request, trusted: &[TrustedCidr]) -> IpAddr {
    let peer = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));

    if !is_trusted(peer, trusted) {
        return peer;
    }

    let header = request.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()).unwrap_or("");

    let mut candidate = peer;
    for part in header.split(',').rev() {
        let ip: IpAddr = match part.trim().parse() {
            Ok(ip) => ip,
            Err(_) => continue,
        };
        if !is_trusted(ip, trusted) {
            return ip;
        }
        candidate = ip;
    }

    candidate
}

fn is_trusted(ip: IpAddr, trusted: &[TrustedCidr]) -> bool {
    trusted.iter().any(|cidr| cidr.contains(ip))
}

impl TrustedCidr {
    fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(addr)) => {
                let mask = u32::MAX.checked_shl(32 - self.prefix_len as u32).unwrap_or(0);
                u32::from(net) & mask == u32::from(addr) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(addr)) => {
                let mask = u128::MAX.checked_shl(128 - self.prefix_len as u32).unwrap_or(0);
                u128::from(net) & mask == u128::from(addr) & mask
            }
            _ => false,
        }
    }
}

/// True if `entry` is an address with a prefix length (`127.0.0.1/32`,
/// `10.0.0.0/8`, `::1/128`), the only form `server.trusted_proxies` accepts.
pub fn is_cidr(entry: &str) -> bool {
    parse_cidr(entry).is_some()
}

fn parse_cidr(entry: &str) -> Option<TrustedCidr> {
    let (addr_str, len_str) = entry.split_once('/')?;
    let addr: IpAddr = addr_str.parse().ok()?;
    let prefix_len: u8 = len_str.parse().ok()?;

    let max_len = match addr {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };

    if prefix_len > max_len {
        return None;
    }

    Some(TrustedCidr { addr, prefix_len })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_cidr_matches() {
        let cidr = parse_cidr("10.0.0.0/8").expect("valid");
        assert!(cidr.contains("10.1.2.3".parse().unwrap()));
        assert!(!cidr.contains("11.0.0.1".parse().unwrap()));
    }

    #[test]
    fn ipv4_host_cidr_matches_exact() {
        let cidr = parse_cidr("127.0.0.1/32").expect("valid");
        assert!(cidr.contains("127.0.0.1".parse().unwrap()));
        assert!(!cidr.contains("127.0.0.2".parse().unwrap()));
    }

    #[test]
    fn rejects_invalid_cidr() {
        assert!(parse_cidr("not-an-ip").is_none());
        assert!(parse_cidr("10.0.0.0/33").is_none());
    }
}
