//! The readiness probe (ADR 0046 decision 6): `mensura health <address>`
//! requests `GET /_mensura/health` over plain HTTP and succeeds when the
//! instance answers ready.  It is the container's readiness check, and it
//! lives in the binary because the runtime image carries no shell, no
//! `curl`, and no `wget`.  A minimal HTTP/1.1 client over a socket is all
//! it needs: one request, `Connection: close`, and a JSON body.

use std::fmt;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// What a ready instance reported.
#[derive(Clone, Debug)]
pub struct Ready {
    pub namespace: String,
    pub position: u64,
}

/// Why the probe failed.
#[derive(Debug)]
pub enum ProbeError {
    /// The address does not resolve or the connection fails.
    Unreachable(String),
    /// The instance answered, but not with a ready health report.
    NotReady(String),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeError::Unreachable(msg) | ProbeError::NotReady(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Probe the instance at `address` (`host:port`), giving up after
/// `timeout` on each of connecting, writing, and reading.
pub fn health(address: &str, timeout: Duration) -> Result<Ready, ProbeError> {
    let unreachable = |e: std::io::Error| ProbeError::Unreachable(format!("{address}: {e}"));
    let addr = address
        .to_socket_addrs()
        .map_err(unreachable)?
        .next()
        .ok_or_else(|| {
            ProbeError::Unreachable(format!("{address}: the address resolves to nothing"))
        })?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(unreachable)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(unreachable)?;
    write!(
        stream,
        "GET /_mensura/health HTTP/1.1\r\nHost: {address}\r\nAccept: application/json\r\n\
         Connection: close\r\n\r\n"
    )
    .map_err(unreachable)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(unreachable)?;
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<Ready, ProbeError> {
    let not_ready = |msg: &str| ProbeError::NotReady(msg.to_string());
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| not_ready("the response is not HTTP"))?;
    let head = std::str::from_utf8(&response[..split])
        .map_err(|_| not_ready("the response head is not text"))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| not_ready("the response has no status line"))?;
    if status != "200" {
        return Err(ProbeError::NotReady(format!(
            "health answered with status {status}"
        )));
    }
    let body: serde_json::Value = serde_json::from_slice(&response[split + 4..])
        .map_err(|_| not_ready("the health report is not JSON"))?;
    if body["ready"] != serde_json::Value::Bool(true) {
        return Err(not_ready("the instance reports that it is not ready"));
    }
    Ok(Ready {
        namespace: body["namespace"].as_str().unwrap_or_default().to_string(),
        position: body["position"].as_u64().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ready_report_parses() {
        let ready = parse_response(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n\
              {\"ready\":true,\"namespace\":\"fleet\",\"position\":7}",
        )
        .unwrap();
        assert_eq!(ready.namespace, "fleet");
        assert_eq!(ready.position, 7);
    }

    #[test]
    fn anything_else_is_not_ready() {
        for response in [
            &b"HTTP/1.1 503 Service Unavailable\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\n\r\n{\"ready\":false}",
            b"HTTP/1.1 200 OK\r\n\r\nnot json",
            b"garbage",
        ] {
            assert!(parse_response(response).is_err());
        }
    }

    #[test]
    fn a_closed_port_is_unreachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let err = health(&addr.to_string(), Duration::from_millis(500)).unwrap_err();
        assert!(matches!(err, ProbeError::Unreachable(_)), "{err}");
    }
}
