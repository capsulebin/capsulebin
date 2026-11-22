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

/// The client address: the peer, or - when the peer is a trusted proxy - the
/// right-most `X-Forwarded-For` entry that is not itself a trusted proxy.
///
/// Every `X-Forwarded-For` line counts, in order, as one comma-separated list
/// (RFC 9110, section 5.3): a proxy that appends its own line instead of
/// extending the client's must not leave the client's line as the only one
/// read. The walk goes from the right and stops at the first entry that is not
/// a bare address (`ip:port`, `unknown`, an empty entry or a line that is not
/// text): everything left of it may come from the client, so the last trusted
/// address reached is used, never an entry further left. IPv4-mapped IPv6
/// addresses (`::ffff:a.b.c.d`, how a dual-stack listener sees IPv4 peers) are
/// compared and returned as IPv4.
fn resolve(request: &Request, trusted: &[TrustedCidr]) -> IpAddr {
    let peer = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]))
        .to_canonical();

    if !is_trusted(peer, trusted) {
        return peer;
    }

    // `None` stands for a line that is not text: it cannot be split into
    // entries, so it ends the walk like an entry that does not parse.
    let entries: Vec<Option<&str>> = request
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .flat_map(|line| match line.to_str() {
            Ok(text) => text.split(',').map(Some).collect::<Vec<_>>(),
            Err(_) => vec![None],
        })
        .collect();

    let mut candidate = peer;
    for entry in entries.into_iter().rev() {
        let Some(ip) = entry.and_then(|part| part.trim().parse::<IpAddr>().ok()) else {
            return candidate;
        };
        let ip = ip.to_canonical();
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
    use axum::body::Body;

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

    /// The address [`resolve`] finds for a request from `peer` with the
    /// `X-Forwarded-For` lines `lines`, behind the proxies `10.0.0.0/8`.
    fn client(peer: &str, lines: &[&[u8]]) -> IpAddr {
        let trusted = vec![parse_cidr("10.0.0.0/8").expect("valid")];
        let mut builder = Request::builder().uri("/");
        for line in lines {
            builder = builder.header("x-forwarded-for", *line);
        }
        let mut request = builder.body(Body::empty()).expect("request");
        let peer: IpAddr = peer.parse().expect("peer");
        request.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::new(peer, 4711)));
        resolve(&request, &trusted)
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("ip")
    }

    #[test]
    fn an_untrusted_peer_is_the_client() {
        assert_eq!(client("203.0.113.7", &[b"198.51.100.1"]), ip("203.0.113.7"));
    }

    #[test]
    fn the_right_most_untrusted_entry_is_the_client() {
        assert_eq!(client("10.0.0.1", &[b"198.51.100.9, 203.0.113.7, 10.0.0.2"]), ip("203.0.113.7"));
        assert_eq!(client("10.0.0.1", &[]), ip("10.0.0.1"));
    }

    /// A proxy that appends its own line: the client's line comes first and
    /// must not be the only one read.
    #[test]
    fn every_header_line_counts_in_order() {
        assert_eq!(client("10.0.0.1", &[b"198.51.100.9", b"203.0.113.7"]), ip("203.0.113.7"));
        assert_eq!(client("10.0.0.1", &[b"198.51.100.9", b"203.0.113.7, 10.0.0.2"]), ip("203.0.113.7"));
    }

    /// An entry that is not a bare address ends the walk at the last trusted
    /// address: the entries left of it are never used.
    #[test]
    fn an_unparsable_entry_stops_the_walk() {
        for line in [
            &b"198.51.100.9, 203.0.113.7:5678"[..],
            b"198.51.100.9, unknown",
            b"198.51.100.9,",
            b"",
        ] {
            assert_eq!(client("10.0.0.1", &[line]), ip("10.0.0.1"), "{}", String::from_utf8_lossy(line));
        }
        assert_eq!(client("10.0.0.1", &[b"198.51.100.9, unknown, 10.0.0.2"]), ip("10.0.0.2"));
        assert_eq!(client("10.0.0.1", &[b"198.51.100.9", b"\xff"]), ip("10.0.0.1"));
    }

    /// A dual-stack listener sees an IPv4 proxy as `::ffff:a.b.c.d`; it is
    /// still the trusted proxy, and mapped entries count as IPv4.
    #[test]
    fn ipv4_mapped_addresses_count_as_ipv4() {
        assert_eq!(client("::ffff:10.0.0.1", &[b"203.0.113.7"]), ip("203.0.113.7"));
        assert_eq!(client("::ffff:203.0.113.7", &[b"198.51.100.1"]), ip("203.0.113.7"));
        assert_eq!(client("10.0.0.1", &[b"203.0.113.7, ::ffff:10.0.0.2"]), ip("203.0.113.7"));
    }
}
