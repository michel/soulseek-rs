//! Hand-rolled NAT-PMP (RFC 6886) over `std::net::UdpSocket`, no dependencies.
//!
//! The gateway address is always passed in as a `SocketAddr`, so tests point it
//! at an in-process mock gateway on `127.0.0.1`. Only [`default_gateway`] touches
//! the OS routing table.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

/// The well-known NAT-PMP gateway UDP port.
pub const NAT_PMP_PORT: u16 = 5351;

const PROTO_VERSION: u8 = 0;
const OP_MAP_TCP: u8 = 2;
const RESPONSE_FLAG: u8 = 128; // response opcode == request opcode + 128
/// RFC-recommended mapping lifetime (2 hours).
pub const RECOMMENDED_LIFETIME: u32 = 7200;

#[derive(Debug)]
pub enum NatPmpError {
    Io(io::Error),
    ShortResponse,
    BadVersion(u8),
    BadOpcode(u8),
    /// A non-zero RFC 6886 result code (1..=5).
    ResultCode(u16),
    /// Every retransmission timed out.
    NoResponse,
}

impl std::fmt::Display for NatPmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "network error: {e}"),
            Self::ShortResponse => write!(f, "truncated gateway response"),
            Self::BadVersion(v) => write!(f, "unexpected version {v}"),
            Self::BadOpcode(o) => write!(f, "unexpected opcode {o}"),
            Self::ResultCode(c) => write!(f, "gateway result code {c}"),
            Self::NoResponse => write!(f, "no response from gateway"),
        }
    }
}

impl From<io::Error> for NatPmpError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A successful TCP mapping. The gateway may grant a different external port or
/// a shorter lifetime than requested, so callers must use these values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapResponse {
    /// Seconds since the gateway's port table was initialised (reboot detector).
    pub epoch: u32,
    pub internal_port: u16,
    pub external_port: u16,
    pub lifetime: u32,
}

/// Request (or, with `lifetime == 0`, remove) a TCP port mapping from the
/// NAT-PMP gateway at `gateway`. `attempts` bounds the retransmissions; RFC 6886
/// allows up to 9 (~64s), but an interactive tool should pass a smaller number
/// so a non-NAT-PMP router doesn't stall for a minute.
///
/// # Errors
/// Returns a [`NatPmpError`] on I/O failure, a malformed/short reply, a non-zero
/// gateway result code, or if no reply arrives within `attempts` retransmits.
pub fn map_tcp(
    gateway: SocketAddr,
    internal_port: u16,
    suggested_external: u16,
    lifetime: u32,
    attempts: u32,
) -> Result<MapResponse, NatPmpError> {
    let mut request = Vec::with_capacity(12);
    request.push(PROTO_VERSION);
    request.push(OP_MAP_TCP);
    request.extend_from_slice(&[0, 0]); // reserved
    request.extend_from_slice(&internal_port.to_be_bytes());
    request.extend_from_slice(&suggested_external.to_be_bytes());
    request.extend_from_slice(&lifetime.to_be_bytes());

    let response = transact(gateway, &request, OP_MAP_TCP, attempts)?;
    if response.len() < 16 {
        return Err(NatPmpError::ShortResponse);
    }
    Ok(MapResponse {
        epoch: u32::from_be_bytes([
            response[4],
            response[5],
            response[6],
            response[7],
        ]),
        internal_port: u16::from_be_bytes([response[8], response[9]]),
        external_port: u16::from_be_bytes([response[10], response[11]]),
        lifetime: u32::from_be_bytes([
            response[12],
            response[13],
            response[14],
            response[15],
        ]),
    })
}

/// Remove a TCP mapping for `internal_port` (external port 0, lifetime 0).
///
/// # Errors
/// As [`map_tcp`].
pub fn unmap_tcp(
    gateway: SocketAddr,
    internal_port: u16,
    attempts: u32,
) -> Result<(), NatPmpError> {
    map_tcp(gateway, internal_port, 0, 0, attempts).map(|_| ())
}

/// Send `request` to `gateway` and return the validated response body, retrying
/// with RFC 6886's 250ms exponential backoff up to `attempts` times.
fn transact(
    gateway: SocketAddr,
    request: &[u8],
    expected_op: u8,
    attempts: u32,
) -> Result<Vec<u8>, NatPmpError> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.connect(gateway)?;

    let mut timeout = Duration::from_millis(250);
    let mut buf = [0u8; 32];
    for _ in 0..attempts.max(1) {
        socket.send(request)?;
        socket.set_read_timeout(Some(timeout))?;
        match socket.recv(&mut buf) {
            Ok(n) => {
                let response = &buf[..n];
                if response.len() < 4 {
                    return Err(NatPmpError::ShortResponse);
                }
                if response[0] != PROTO_VERSION {
                    return Err(NatPmpError::BadVersion(response[0]));
                }
                if response[1] != expected_op + RESPONSE_FLAG {
                    return Err(NatPmpError::BadOpcode(response[1]));
                }
                let result = u16::from_be_bytes([response[2], response[3]]);
                if result != 0 {
                    return Err(NatPmpError::ResultCode(result));
                }
                return Ok(response.to_vec());
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                timeout = timeout.saturating_mul(2);
            }
            Err(e) => return Err(NatPmpError::Io(e)),
        }
    }
    Err(NatPmpError::NoResponse)
}

/// Best-effort discovery of the IPv4 default-gateway address from the OS routing
/// table. Supports macOS and Linux; returns `None` elsewhere or on any parse
/// failure (callers should treat NAT-PMP as unavailable then).
#[must_use]
pub fn default_gateway() -> Option<Ipv4Addr> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("route")
            .args(["-n", "get", "default"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("gateway:") {
                return rest.trim().parse().ok();
            }
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        // /proc/net/route: tab-separated; the default route has Destination
        // 00000000, and Gateway is a little-endian hex u32.
        let text = std::fs::read_to_string("/proc/net/route").ok()?;
        for line in text.lines().skip(1) {
            let mut fields = line.split_whitespace();
            let _iface = fields.next()?;
            let destination = fields.next()?;
            let gateway = fields.next()?;
            if destination == "00000000" {
                let raw = u32::from_str_radix(gateway, 16).ok()?;
                return Some(Ipv4Addr::from(raw.to_le_bytes()));
            }
        }
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    /// A minimal in-process NAT-PMP gateway for tests. It receives one request,
    /// hands the raw bytes back to the test, and replies with `reply`.
    fn mock_gateway(reply: Vec<u8>) -> (SocketAddr, mpsc::Receiver<Vec<u8>>) {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = socket.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 64];
            if let Ok((n, from)) = socket.recv_from(&mut buf) {
                let _ = tx.send(buf[..n].to_vec());
                let _ = socket.send_to(&reply, from);
            }
        });
        (addr, rx)
    }

    fn map_reply(
        result: u16,
        internal: u16,
        external: u16,
        lifetime: u32,
    ) -> Vec<u8> {
        let mut r = vec![PROTO_VERSION, OP_MAP_TCP + RESPONSE_FLAG];
        r.extend_from_slice(&result.to_be_bytes());
        r.extend_from_slice(&123u32.to_be_bytes()); // epoch
        r.extend_from_slice(&internal.to_be_bytes());
        r.extend_from_slice(&external.to_be_bytes());
        r.extend_from_slice(&lifetime.to_be_bytes());
        r
    }

    #[test]
    fn map_tcp_sends_correct_request_and_parses_response() {
        // Gateway grants a DIFFERENT external port and a SHORTER lease than asked.
        let (gateway, requests) = mock_gateway(map_reply(0, 2234, 40000, 3600));
        let resp = map_tcp(gateway, 2234, 2234, 7200, 3)
            .expect("mapping should succeed");

        // The request bytes must match the RFC 6886 map-TCP layout.
        let req = requests.recv().unwrap();
        assert_eq!(req.len(), 12);
        assert_eq!(req[0], 0, "version");
        assert_eq!(req[1], 2, "opcode TCP");
        assert_eq!(&req[2..4], &[0, 0], "reserved");
        assert_eq!(u16::from_be_bytes([req[4], req[5]]), 2234, "internal port");
        assert_eq!(
            u16::from_be_bytes([req[6], req[7]]),
            2234,
            "suggested external"
        );
        assert_eq!(
            u32::from_be_bytes([req[8], req[9], req[10], req[11]]),
            7200,
            "lifetime"
        );

        // The response's granted values (not the requested ones) are returned.
        assert_eq!(resp.external_port, 40000);
        assert_eq!(resp.lifetime, 3600);
        assert_eq!(resp.internal_port, 2234);
    }

    #[test]
    fn map_tcp_surfaces_gateway_result_code() {
        // Result code 2 = "not authorized/refused" per RFC 6886.
        let (gateway, _req) = mock_gateway(map_reply(2, 2234, 0, 0));
        match map_tcp(gateway, 2234, 2234, 7200, 3) {
            Err(NatPmpError::ResultCode(2)) => {}
            other => panic!("expected ResultCode(2), got {other:?}"),
        }
    }

    #[test]
    fn map_tcp_times_out_when_gateway_is_silent() {
        // Bind a socket but never reply: the mapper must give up, not hang.
        let dead = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = dead.local_addr().unwrap();
        drop(dead); // nothing listening now
        match map_tcp(addr, 2234, 2234, 7200, 2) {
            Err(NatPmpError::NoResponse | NatPmpError::Io(_)) => {}
            other => panic!("expected timeout/io error, got {other:?}"),
        }
    }

    #[test]
    fn unmap_sends_zero_lifetime_and_port() {
        let (gateway, requests) = mock_gateway(map_reply(0, 2234, 0, 0));
        unmap_tcp(gateway, 2234, 3).expect("unmap should succeed");
        let req = requests.recv().unwrap();
        assert_eq!(
            u16::from_be_bytes([req[6], req[7]]),
            0,
            "external port 0 to delete"
        );
        assert_eq!(
            u32::from_be_bytes([req[8], req[9], req[10], req[11]]),
            0,
            "lifetime 0 to delete"
        );
    }
}
