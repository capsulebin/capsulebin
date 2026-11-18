//! The client side of `capsulebin healthcheck`.
//!
//! A minimal container image has no `curl`, so the binary probes its own health
//! endpoint. One HTTP/1.1 request over a plain TCP connection is all that needs,
//! which keeps an HTTP client library out of the server.
//!
//! The probe sends no capability, no body and nothing derived from stored data:
//! it asks `/health/ready` or `/health/live` and reads the status line.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

/// How much of the response is read while looking for the status line.
const MAX_STATUS_LINE: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("could not connect to {address}: {source}")]
    Connect {
        address: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the request to {address} failed: {source}")]
    Io {
        address: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{address} did not answer within {} ms", .timeout.as_millis())]
    Timeout { address: String, timeout: Duration },
    #[error("{address} did not answer with an HTTP status line")]
    Malformed { address: String },
    #[error("{address} answered {status} for {path}")]
    Unhealthy { address: String, path: String, status: u16 },
}

/// The address to probe for a `server.bind` value.
///
/// An unspecified address (`0.0.0.0`, `[::]`) is what a container binds to, but it
/// cannot be connected to; it becomes the loopback address of the same family.
/// Anything else, including a host name, is used as it is.
pub fn probe_address(bind: &str) -> String {
    match bind.parse::<SocketAddr>() {
        Ok(addr) if addr.ip().is_unspecified() => {
            let loopback = match addr.ip() {
                IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
            };
            SocketAddr::new(loopback, addr.port()).to_string()
        }
        _ => bind.to_owned(),
    }
}

/// Requests `path` from `address` and succeeds only on `200`.
pub async fn probe(address: &str, path: &str, timeout: Duration) -> Result<(), ProbeError> {
    let status = tokio::time::timeout(timeout, status_of(address, path))
        .await
        .map_err(|_| ProbeError::Timeout {
            address: address.to_owned(),
            timeout,
        })??;

    if status == 200 {
        Ok(())
    } else {
        Err(ProbeError::Unhealthy {
            address: address.to_owned(),
            path: path.to_owned(),
            status,
        })
    }
}

async fn status_of(address: &str, path: &str) -> Result<u16, ProbeError> {
    let io = |source| ProbeError::Io {
        address: address.to_owned(),
        source,
    };

    let mut stream = TcpStream::connect(address).await.map_err(|source| ProbeError::Connect {
        address: address.to_owned(),
        source,
    })?;

    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nUser-Agent: capsulebin-healthcheck\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.map_err(io)?;

    // The status line comes first; the rest of the response is not needed.
    let mut response = Vec::with_capacity(128);
    let mut chunk = [0_u8; 256];
    while !response.contains(&b'\n') && response.len() < MAX_STATUS_LINE {
        let read = stream.read(&mut chunk).await.map_err(io)?;
        if read == 0 {
            break;
        }
        response.extend(chunk.iter().take(read));
    }

    parse_status(&response).ok_or_else(|| ProbeError::Malformed {
        address: address.to_owned(),
    })
}

/// The status code from an `HTTP/1.x` status line.
fn parse_status(response: &[u8]) -> Option<u16> {
    let line = response.split(|&byte| byte == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let mut parts = line.split(' ');
    if !parts.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let code = parts.next()?;
    if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    code.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::net::TcpListener;

    /// A server that answers one connection with `response` once it has read the
    /// request, and returns the request it saw.
    async fn answer_once(response: &'static [u8]) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 512];
            while !request.ends_with(b"\r\n\r\n") {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            socket.write_all(response).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (address, handle)
    }

    const TIMEOUT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn a_200_is_healthy_and_the_request_is_plain() {
        let (address, server) = answer_once(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").await;

        probe(&address, "/health/ready", TIMEOUT).await.unwrap();

        let request = server.await.unwrap();
        assert!(request.starts_with("GET /health/ready HTTP/1.1\r\n"), "{request}");
        assert!(request.contains(&format!("\r\nHost: {address}\r\n")), "{request}");
    }

    #[tokio::test]
    async fn a_503_is_unhealthy() {
        let (address, _server) = answer_once(b"HTTP/1.1 503 Service Unavailable\r\n\r\n").await;

        let error = probe(&address, "/health/ready", TIMEOUT).await.unwrap_err();

        assert!(matches!(error, ProbeError::Unhealthy { status: 503, .. }), "{error}");
    }

    #[tokio::test]
    async fn something_that_is_not_http_is_unhealthy() {
        let (address, _server) = answer_once(b"SSH-2.0-OpenSSH_9.9\r\n").await;

        let error = probe(&address, "/health/ready", TIMEOUT).await.unwrap_err();

        assert!(matches!(error, ProbeError::Malformed { .. }), "{error}");
    }

    #[tokio::test]
    async fn a_closed_port_is_unhealthy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        drop(listener);

        let error = probe(&address, "/health/live", TIMEOUT).await.unwrap_err();

        assert!(matches!(error, ProbeError::Connect { .. }), "{error}");
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();

        let error = probe(&address, "/health/live", Duration::from_millis(200)).await.unwrap_err();

        assert!(matches!(error, ProbeError::Timeout { .. }), "{error}");
        drop(listener);
    }

    #[test]
    fn unspecified_bind_addresses_become_loopback() {
        assert_eq!(probe_address("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(probe_address("[::]:8080"), "[::1]:8080");
        assert_eq!(probe_address("127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(probe_address("192.0.2.10:80"), "192.0.2.10:80");
        assert_eq!(probe_address("localhost:8080"), "localhost:8080");
    }

    #[test]
    fn status_lines() {
        assert_eq!(parse_status(b"HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(parse_status(b"HTTP/1.0 503 Service Unavailable\r\n"), Some(503));
        assert_eq!(parse_status(b"HTTP/1.1 200"), Some(200));
        assert_eq!(parse_status(b"HTTP/2 200\r\n"), None);
        assert_eq!(parse_status(b"HTTP/1.1 20 OK\r\n"), None);
        assert_eq!(parse_status(b"HTTP/1.1 +20 OK\r\n"), None);
        assert_eq!(parse_status(b""), None);
        assert_eq!(parse_status(b"\xff\xfe"), None);
    }
}
