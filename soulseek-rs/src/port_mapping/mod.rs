//! Best-effort automatic port mapping so a client behind a home router becomes
//! reachable for incoming peer connections (browse/download of firewalled peers
//! and the server-brokered connect-back both need this).
//!
//! Tries UPnP-IGD (via the `igd-next` crate) first, then hand-rolled NAT-PMP
//! (see [`nat_pmp`]). Everything is best-effort: any failure is logged and the
//! program continues (the user can still reach non-firewalled peers, and can
//! port-forward manually). The mapping is renewed on a timer and removed on
//! shutdown.

mod nat_pmp;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use igd_next::{PortMappingProtocol, SearchOptions, search_gateway};
use soulseek_rs::utils::logger::{self, LogLevel};

const MAPPING_DESCRIPTION: &str = "soulseek-rs";
/// Lease we request; both backends renew at half this interval.
const LEASE_SECS: u32 = nat_pmp::RECOMMENDED_LIFETIME;
/// Interactive NAT-PMP retransmit budget (~3.75s worst case), not the full 9.
const NATPMP_ATTEMPTS: u32 = 4;

/// A live port mapping and its renewal thread. Dropping it stops renewal and
/// removes the mapping (best-effort).
pub struct PortMapper {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PortMapper {
    /// Spawn a background best-effort mapper for TCP `port`. Returns immediately;
    /// discovery/mapping happens on the thread so startup never blocks.
    #[must_use]
    pub fn spawn(port: u16) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = thread::Builder::new()
            .name("port-mapper".into())
            .spawn(move || run(port, &thread_stop))
            .ok();
        Self { stop, handle }
    }
}

impl Drop for PortMapper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Which backend created the active mapping, carrying what's needed to renew and
/// remove it.
enum Backend {
    Upnp {
        gateway: Box<igd_next::Gateway>,
        local_addr: SocketAddr,
        external_port: u16,
    },
    NatPmp {
        gateway: SocketAddr,
        internal_port: u16,
    },
}

fn run(port: u16, stop: &AtomicBool) {
    let Some(local_ip) = local_lan_ip() else {
        warn(
            "could not determine this machine's LAN IP; skipping automatic \
              port mapping (you may need to forward the port manually)",
        );
        return;
    };

    let Some((backend, lease)) = map_once(local_ip, port) else {
        warn(&format!(
            "automatic port mapping unavailable (no UPnP/NAT-PMP router \
             found for port {port}); browsing/downloading firewalled peers \
             may fail unless you forward TCP {port} on your router"
        ));
        return;
    };

    manage(&backend, lease, port, stop);
}

/// Renew `backend`'s mapping until `stop`, then remove it. Renews at half the
/// granted lease, floored at 30s so a pathologically short lease can't spin us
/// into a hot re-map loop.
fn manage(backend: &Backend, lease: u32, port: u16, stop: &AtomicBool) {
    let renew_after = Duration::from_secs(u64::from((lease / 2).max(30)));
    loop {
        if sleep_until(renew_after, stop) {
            break; // asked to stop
        }
        if !renew(backend, port) {
            // Renewal failed; the lease will expire on its own. Stop trying.
            warn("failed to renew the port mapping; it may expire");
            return;
        }
    }
    remove(backend);
}

/// Attempt UPnP then NAT-PMP once. Returns the backend and granted lease seconds.
fn map_once(local_ip: IpAddr, port: u16) -> Option<(Backend, u32)> {
    if let Some(result) = map_upnp(local_ip, port) {
        return Some(result);
    }
    map_natpmp(port)
}

fn map_upnp(local_ip: IpAddr, port: u16) -> Option<(Backend, u32)> {
    let options = SearchOptions {
        bind_addr: SocketAddr::new(local_ip, 0),
        timeout: Some(Duration::from_secs(5)),
        ..SearchOptions::default()
    };
    let gateway = search_gateway(options).ok()?;
    let local_addr = SocketAddr::new(local_ip, port);
    // We advertise our listen port verbatim to the Soulseek server, so the
    // EXTERNAL port must equal it. If the exact port can't be mapped (e.g.
    // another LAN device already claimed it), a different external port would
    // be unreachable for our advertised address, so we give up rather than map
    // a useless port.
    if let Err(e) = gateway.add_port(
        PortMappingProtocol::TCP,
        port,
        local_addr,
        LEASE_SECS,
        MAPPING_DESCRIPTION,
    ) {
        warn(&format!(
            "UPnP found a router but couldn't map port {port} ({e}); \
             try a different --listener-port or forward it manually"
        ));
        return None;
    }
    let external = gateway
        .get_external_ip()
        .map_or_else(|_| "?".to_string(), |ip| ip.to_string());
    info(&format!(
        "UPnP: mapped external {external}:{port} -> local {port}"
    ));
    Some((
        Backend::Upnp {
            gateway: Box::new(gateway),
            local_addr,
            external_port: port,
        },
        LEASE_SECS,
    ))
}

fn map_natpmp(port: u16) -> Option<(Backend, u32)> {
    let gateway_ip = nat_pmp::default_gateway()?;
    let gateway =
        SocketAddr::new(IpAddr::V4(gateway_ip), nat_pmp::NAT_PMP_PORT);
    let response =
        nat_pmp::map_tcp(gateway, port, port, LEASE_SECS, NATPMP_ATTEMPTS)
            .ok()?;
    // We advertise our listen port verbatim, so a different external port would
    // be unreachable for peers. Keep the mapping but warn loudly.
    if response.external_port != port {
        warn(&format!(
            "NAT-PMP granted external port {} instead of {port}; peers may not \
             reach you — try forwarding TCP {port} manually",
            response.external_port
        ));
    }
    info(&format!(
        "NAT-PMP: mapped external port {} -> local {port} (lease {}s)",
        response.external_port, response.lifetime
    ));
    Some((
        Backend::NatPmp {
            gateway,
            internal_port: port,
        },
        response.lifetime,
    ))
}

fn renew(backend: &Backend, port: u16) -> bool {
    match backend {
        Backend::Upnp {
            gateway,
            local_addr,
            external_port,
        } => gateway
            .add_port(
                PortMappingProtocol::TCP,
                *external_port,
                *local_addr,
                LEASE_SECS,
                MAPPING_DESCRIPTION,
            )
            .is_ok(),
        Backend::NatPmp {
            gateway,
            internal_port,
        } => nat_pmp::map_tcp(
            *gateway,
            *internal_port,
            port,
            LEASE_SECS,
            NATPMP_ATTEMPTS,
        )
        .is_ok(),
    }
}

fn remove(backend: &Backend) {
    match backend {
        Backend::Upnp {
            gateway,
            external_port,
            ..
        } => {
            let _ =
                gateway.remove_port(PortMappingProtocol::TCP, *external_port);
        }
        Backend::NatPmp {
            gateway,
            internal_port,
        } => {
            let _ = nat_pmp::unmap_tcp(*gateway, *internal_port, 2);
        }
    }
}

/// Sleep for `dur`, checking `stop` every 500ms. Returns `true` if `stop` was
/// signalled (so the caller should exit promptly).
fn sleep_until(dur: Duration, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        thread::sleep(Duration::from_millis(500));
    }
    stop.load(Ordering::SeqCst)
}

/// The LAN IP of the interface that routes to the internet, found by asking the
/// OS which local address a UDP socket would use to reach a public address. No
/// packets are sent (UDP `connect` only sets the default route).
fn local_lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    // A loopback/unspecified address can't be a forward target.
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

fn info(message: &str) {
    logger::log(LogLevel::Info, &format!("[port-mapping] {message}"));
}

fn warn(message: &str) {
    logger::log(LogLevel::Warn, &format!("[port-mapping] {message}"));
}

#[cfg(test)]
impl PortMapper {
    /// Spawn a mapper that uses ONLY the NAT-PMP backend against an injected
    /// gateway (no UPnP, no routing-table discovery), so the full spawn → map →
    /// renew → cleanup-on-drop lifecycle can be driven against a mock gateway.
    fn spawn_natpmp_test(port: u16, gateway: SocketAddr) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = thread::Builder::new()
            .name("port-mapper-test".into())
            .spawn(move || {
                if let Ok(response) = nat_pmp::map_tcp(
                    gateway,
                    port,
                    port,
                    LEASE_SECS,
                    NATPMP_ATTEMPTS,
                ) {
                    let backend = Backend::NatPmp {
                        gateway,
                        internal_port: port,
                    };
                    manage(&backend, response.lifetime, port, &thread_stop);
                }
            })
            .ok();
        Self { stop, handle }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration as StdDuration;

    /// A mock NAT-PMP gateway that answers every map-TCP request with the same
    /// granted values and forwards each raw request to the test.
    fn mock_gateway() -> (SocketAddr, mpsc::Receiver<Vec<u8>>) {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = socket.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 64];
            // Answer many requests (initial map + cleanup unmap + any renews).
            while let Ok((n, from)) = socket.recv_from(&mut buf) {
                let request = buf[..n].to_vec();
                let internal = u16::from_be_bytes([request[4], request[5]]);
                let lifetime = u32::from_be_bytes([
                    request[8],
                    request[9],
                    request[10],
                    request[11],
                ]);
                // Response: version 0, opcode 130, result 0, echo internal port
                // and grant the SAME external port + requested lifetime.
                let mut reply = vec![0u8, 130, 0, 0];
                reply.extend_from_slice(&7u32.to_be_bytes()); // epoch
                reply.extend_from_slice(&internal.to_be_bytes());
                reply.extend_from_slice(&internal.to_be_bytes());
                reply.extend_from_slice(&lifetime.to_be_bytes());
                if tx.send(request).is_err() {
                    break;
                }
                let _ = socket.send_to(&reply, from);
            }
        });
        (addr, rx)
    }

    #[test]
    fn service_maps_on_start_and_unmaps_on_drop() {
        let (gateway, requests) = mock_gateway();
        let mapper = PortMapper::spawn_natpmp_test(2234, gateway);

        // The first request must be a map with our full requested lifetime.
        let map_req = requests
            .recv_timeout(StdDuration::from_secs(3))
            .expect("mapper should send an initial map request");
        assert_eq!(u16::from_be_bytes([map_req[4], map_req[5]]), 2234);
        assert_eq!(
            u32::from_be_bytes([
                map_req[8],
                map_req[9],
                map_req[10],
                map_req[11]
            ]),
            LEASE_SECS,
            "initial map should request the full lease"
        );

        // Dropping the mapper must remove the mapping (a lifetime-0 request).
        drop(mapper);
        let unmap_req = requests
            .recv_timeout(StdDuration::from_secs(3))
            .expect("dropping the mapper should send an unmap request");
        assert_eq!(
            u32::from_be_bytes([
                unmap_req[8],
                unmap_req[9],
                unmap_req[10],
                unmap_req[11]
            ]),
            0,
            "cleanup should request lifetime 0"
        );
    }
}
