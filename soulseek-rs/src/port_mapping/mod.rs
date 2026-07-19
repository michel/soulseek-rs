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
use std::sync::mpsc::{self, Receiver};
use std::thread;
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
    done: Receiver<()>,
}

impl PortMapper {
    /// Spawn a background best-effort mapper for TCP `port`. Returns immediately;
    /// discovery/mapping happens on the thread so startup never blocks.
    #[must_use]
    pub fn spawn(port: u16) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let (done_tx, done) = mpsc::channel();
        let _ = thread::Builder::new().name("port-mapper".into()).spawn(
            move || {
                run(port, &thread_stop);
                let _ = done_tx.send(());
            },
        );
        Self { stop, done }
    }
}

impl Drop for PortMapper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wait briefly for the thread to remove its mapping, but NEVER freeze
        // shutdown: if it's still in a blocking discovery/mapping call (which
        // doesn't observe `stop`), detach it and let the finite lease expire.
        let _ = self.done.recv_timeout(Duration::from_secs(1));
    }
}

/// Synchronously probe whether automatic port mapping works on this network:
/// try to map `port` (UPnP then NAT-PMP), report the result in human-readable
/// form, then remove the probe mapping. Used by the `portmap` CLI command so a
/// user can verify reachability on their actual router.
#[must_use]
pub fn diagnose(port: u16) -> String {
    let Some(local_ip) = local_lan_ip() else {
        return "Could not determine this machine's LAN IP — are you connected \
                to a network?"
            .to_string();
    };

    match map_once(local_ip, port) {
        Some((backend, _lease)) => {
            let detail = match &backend {
                Backend::Upnp { gateway, .. } => {
                    let ip = gateway.get_external_ip().map_or_else(
                        |_| "unknown".to_string(),
                        |ip| ip.to_string(),
                    );
                    format!("UPnP-IGD (external address {ip}:{port})")
                }
                Backend::NatPmp { .. } => "NAT-PMP".to_string(),
            };
            remove(&backend); // clean up the probe mapping
            format!(
                "✅ Automatic port mapping works via {detail}.\n   TCP {port} \
                 was opened on your router, so firewalled peers and the server \
                 can reach you. The app does this automatically while it runs."
            )
        }
        None => format!(
            "❌ No UPnP/NAT-PMP router responded for TCP {port}.\n   Automatic \
             mapping won't work on this network. To browse/download firewalled \
             peers, forward TCP {port} to this machine on your router (or \
             enable UPnP there)."
        ),
    }
}

/// Which backend created the active mapping, carrying what's needed to renew and
/// remove it.
enum Backend {
    Upnp {
        gateway: Box<igd_next::Gateway>,
        local_addr: SocketAddr,
        external_port: u16,
        /// The lease that the router accepted (0 == permanent).
        lease: u32,
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
    map_via_gateway(gateway, local_addr, port)
}

/// Map `port` on an already-discovered IGD `gateway`. Split out from discovery
/// so it can be exercised against a mock IGD in tests.
fn map_via_gateway(
    gateway: igd_next::Gateway,
    local_addr: SocketAddr,
    port: u16,
) -> Option<(Backend, u32)> {
    // We advertise our listen port verbatim to the Soulseek server, so the
    // EXTERNAL port must equal it. If the exact port can't be mapped (e.g.
    // another LAN device already claimed it), a different external port would
    // be unreachable for our advertised address, so we give up rather than map
    // a useless port. Some IGDs only accept permanent leases, so if the timed
    // lease is refused, retry with lease 0 (permanent) before giving up.
    let lease = match gateway.add_port(
        PortMappingProtocol::TCP,
        port,
        local_addr,
        LEASE_SECS,
        MAPPING_DESCRIPTION,
    ) {
        Ok(()) => LEASE_SECS,
        Err(_) => match gateway.add_port(
            PortMappingProtocol::TCP,
            port,
            local_addr,
            0,
            MAPPING_DESCRIPTION,
        ) {
            Ok(()) => 0,
            Err(e) => {
                warn(&format!(
                    "UPnP found a router but couldn't map port {port} ({e}); \
                     try a different --listener-port or forward it manually"
                ));
                return None;
            }
        },
    };
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
            lease,
        },
        // A permanent (0) lease needs no renewal timer; use the requested lease
        // for the renewal cadence otherwise.
        if lease == 0 { LEASE_SECS } else { lease },
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
            lease,
        } => gateway
            .add_port(
                PortMappingProtocol::TCP,
                *external_port,
                *local_addr,
                *lease,
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
        let (done_tx, done) = mpsc::channel();
        let _ = thread::Builder::new()
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
                let _ = done_tx.send(());
            });
        Self { stop, done }
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

    /// A minimal in-process UPnP IGD: an HTTP server that answers every SOAP
    /// control request with a canned envelope containing the Add/Delete/GetIP
    /// response elements, and forwards each request body to the test.
    fn mock_igd() -> (SocketAddr, mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                // Read headers, then Content-Length bytes of body.
                let body = loop {
                    let n = match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break String::new(),
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf);
                    if let Some(hdr_end) = text.find("\r\n\r\n") {
                        let len = text
                            .lines()
                            .find_map(|l| {
                                l.strip_prefix("Content-Length: ").or_else(
                                    || l.strip_prefix("content-length: "),
                                )
                            })
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buf.len() >= hdr_end + 4 + len {
                            break text[hdr_end + 4..].to_string();
                        }
                    }
                };
                if tx.send(body).is_err() {
                    break;
                }
                let soap = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
<s:Body>
<u:AddPortMappingResponse xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"></u:AddPortMappingResponse>
<u:DeletePortMappingResponse xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"></u:DeletePortMappingResponse>
<u:GetExternalIPAddressResponse xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"><NewExternalIPAddress>203.0.113.7</NewExternalIPAddress></u:GetExternalIPAddressResponse>
</s:Body>
</s:Envelope>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{soap}",
                    soap.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (addr, rx)
    }

    fn igd_control_schema() -> std::collections::HashMap<String, Vec<String>> {
        let mut schema = std::collections::HashMap::new();
        schema.insert(
            "AddPortMapping".to_string(),
            [
                "NewRemoteHost",
                "NewExternalPort",
                "NewProtocol",
                "NewInternalPort",
                "NewInternalClient",
                "NewEnabled",
                "NewPortMappingDescription",
                "NewLeaseDuration",
            ]
            .iter()
            .map(ToString::to_string)
            .collect(),
        );
        schema.insert(
            "DeletePortMapping".to_string(),
            ["NewRemoteHost", "NewExternalPort", "NewProtocol"]
                .iter()
                .map(ToString::to_string)
                .collect(),
        );
        schema
    }

    #[test]
    fn upnp_maps_a_port_against_a_mock_igd() {
        let (igd_addr, requests) = mock_igd();
        let gateway = igd_next::Gateway {
            addr: igd_addr,
            root_url: format!("http://{igd_addr}/rootDesc.xml"),
            control_url: "/ctl".to_string(),
            control_schema_url: format!("http://{igd_addr}/scpd.xml"),
            control_schema: igd_control_schema(),
        };

        let local_addr =
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)), 2234);
        let result = map_via_gateway(gateway, local_addr, 2234);

        let (backend, _lease) =
            result.expect("mapping against a healthy IGD should succeed");
        match backend {
            Backend::Upnp { external_port, .. } => {
                assert_eq!(
                    external_port, 2234,
                    "must map the exact listen port"
                );
            }
            Backend::NatPmp { .. } => panic!("expected a UPnP backend"),
        }

        // The IGD must have received an AddPortMapping SOAP call quoting our
        // external and internal port and the LAN client address.
        let body = requests
            .recv_timeout(StdDuration::from_secs(3))
            .expect("IGD should receive an AddPortMapping request");
        assert!(
            body.contains("<NewExternalPort>2234</NewExternalPort>"),
            "request should map external port 2234: {body}"
        );
        assert!(
            body.contains("<NewInternalPort>2234</NewInternalPort>"),
            "request should target internal port 2234: {body}"
        );
        assert!(
            body.contains("192.168.1.50"),
            "request should point at the LAN client: {body}"
        );
    }

    #[test]
    fn drop_does_not_hang_when_gateway_is_unresponsive() {
        // A bound-but-silent gateway: the mapper thread blocks in the NAT-PMP
        // retransmit budget (~3.75s). Dropping the mapper must NOT wait that
        // long — it detaches after a bounded grace period.
        let silent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = silent.local_addr().unwrap();
        let mapper = PortMapper::spawn_natpmp_test(2234, addr);
        // Ensure the thread is mid-transaction before we drop.
        thread::sleep(StdDuration::from_millis(100));

        let start = Instant::now();
        drop(mapper);
        let elapsed = start.elapsed();
        assert!(
            elapsed < StdDuration::from_millis(1500),
            "drop blocked for {elapsed:?}; it should detach promptly"
        );
        drop(silent);
    }
}
