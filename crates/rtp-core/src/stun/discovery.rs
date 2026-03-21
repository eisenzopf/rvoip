//! Higher-level NAT discovery using multiple STUN servers.
//!
//! Provides `discover_nat_type` which probes several STUN servers to determine
//! the NAT mapping behavior, and `get_public_address` as a convenience for
//! populating SDP connection addresses.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use crate::Error;
use super::client::{StunBindingResult, StunClient, StunClientConfig};

/// Well-known public STUN servers.
pub const DEFAULT_STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// Broad classification of NAT type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatType {
    /// No NAT detected — public address matches local address.
    OpenInternet,
    /// Full-cone NAT: any external host can send to the mapped address.
    FullCone,
    /// Address-restricted cone NAT: only hosts we've sent to can reply.
    RestrictedCone,
    /// Port-restricted cone NAT: only the exact host:port can reply.
    PortRestricted,
    /// Symmetric NAT: mapping changes per destination.
    Symmetric,
    /// Could not determine NAT type (e.g. only one server responded).
    Unknown,
}

impl std::fmt::Display for NatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenInternet => write!(f, "Open Internet (no NAT)"),
            Self::FullCone => write!(f, "Full Cone NAT"),
            Self::RestrictedCone => write!(f, "Restricted Cone NAT"),
            Self::PortRestricted => write!(f, "Port Restricted Cone NAT"),
            Self::Symmetric => write!(f, "Symmetric NAT"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Information about the local NAT environment.
#[derive(Debug, Clone)]
pub struct NatInfo {
    /// The public (server-reflexive) address.
    pub public_addr: SocketAddr,
    /// The detected NAT type.
    pub nat_type: NatType,
    /// Individual binding results from each server probed.
    pub binding_results: Vec<StunBindingResult>,
}

/// Resolve a STUN server hostname to a `SocketAddr`.
///
/// Uses `tokio::net::lookup_host` for async DNS resolution.
async fn resolve_stun_server(server: &str) -> Result<SocketAddr, Error> {
    let mut addrs = tokio::net::lookup_host(server).await.map_err(|e| {
        Error::StunError(format!("failed to resolve STUN server {server}: {e}"))
    })?;

    addrs.next().ok_or_else(|| {
        Error::StunError(format!("DNS returned no addresses for {server}"))
    })
}

/// Discover the NAT type by probing multiple STUN servers.
///
/// Sends Binding Requests to each provided server (as `host:port` strings)
/// using the same local socket. Compares the mapped addresses to classify
/// the NAT type.
///
/// If `stun_servers` is empty, the default public servers are used.
pub async fn discover_nat_type(stun_servers: &[&str]) -> Result<NatInfo, Error> {
    let servers = if stun_servers.is_empty() {
        DEFAULT_STUN_SERVERS
    } else {
        stun_servers
    };

    // Bind a single UDP socket so all probes share the same local port
    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| {
        Error::StunError(format!("failed to bind UDP socket: {e}"))
    })?;

    let local_addr = socket.local_addr().map_err(|e| {
        Error::StunError(format!("failed to get local address: {e}"))
    })?;

    debug!(local = %local_addr, "starting NAT discovery");

    let config = StunClientConfig {
        timeout: Duration::from_secs(3),
        initial_rto: Duration::from_millis(500),
        max_retransmits: 3,
        recv_buf_size: 1024,
    };

    let mut results: Vec<StunBindingResult> = Vec::new();

    for server_str in servers {
        let server_addr = match resolve_stun_server(server_str).await {
            Ok(addr) => addr,
            Err(e) => {
                warn!(server = server_str, error = %e, "skipping STUN server");
                continue;
            }
        };

        let client = StunClient::with_config(server_addr, config.clone());

        match client.binding_request(&socket).await {
            Ok(result) => {
                debug!(
                    server = server_str,
                    mapped = %result.mapped_address,
                    rtt_ms = result.rtt.as_millis(),
                    "STUN binding result"
                );
                results.push(result);
            }
            Err(e) => {
                warn!(server = server_str, error = %e, "STUN binding failed");
            }
        }
    }

    if results.is_empty() {
        return Err(Error::StunError(
            "no STUN servers responded; cannot determine NAT type".into(),
        ));
    }

    let public_addr = results[0].mapped_address;
    let nat_type = classify_nat(&results, local_addr);

    info!(
        public = %public_addr,
        nat_type = %nat_type,
        servers_responded = results.len(),
        "NAT discovery complete"
    );

    Ok(NatInfo {
        public_addr,
        nat_type,
        binding_results: results,
    })
}

/// Get the public (server-reflexive) address for a given local address.
///
/// Binds a UDP socket to `local_addr`, sends a STUN Binding Request to
/// the first reachable default STUN server, and returns the mapped address.
/// This is the primary convenience function for populating SDP connection
/// addresses with the external IP.
pub async fn get_public_address(local_addr: SocketAddr) -> Result<SocketAddr, Error> {
    let socket = UdpSocket::bind(local_addr).await.map_err(|e| {
        Error::StunError(format!("failed to bind to {local_addr}: {e}"))
    })?;

    let config = StunClientConfig {
        timeout: Duration::from_secs(3),
        initial_rto: Duration::from_millis(500),
        max_retransmits: 3,
        recv_buf_size: 1024,
    };

    for server_str in DEFAULT_STUN_SERVERS {
        let server_addr = match resolve_stun_server(server_str).await {
            Ok(addr) => addr,
            Err(e) => {
                warn!(server = server_str, error = %e, "skipping STUN server");
                continue;
            }
        };

        let client = StunClient::with_config(server_addr, config.clone());

        match client.binding_request(&socket).await {
            Ok(result) => {
                info!(
                    public = %result.mapped_address,
                    server = server_str,
                    "discovered public address"
                );
                return Ok(result.mapped_address);
            }
            Err(e) => {
                warn!(server = server_str, error = %e, "STUN request failed, trying next server");
            }
        }
    }

    Err(Error::StunError(
        "all STUN servers failed; could not determine public address".into(),
    ))
}

/// Classify the NAT type based on binding results from multiple servers.
fn classify_nat(results: &[StunBindingResult], local_addr: SocketAddr) -> NatType {
    if results.is_empty() {
        return NatType::Unknown;
    }

    let first_mapped = results[0].mapped_address;

    // Check if public address matches local address (no NAT)
    if first_mapped.ip() == local_addr.ip() && first_mapped.port() == local_addr.port() {
        return NatType::OpenInternet;
    }

    // Need at least 2 results to distinguish NAT types
    if results.len() < 2 {
        // Single result: we know the public address but can't classify further.
        // If IP matches but port differs, likely port-restricted or symmetric.
        // If IP differs, definitely behind NAT but type unknown.
        return NatType::Unknown;
    }

    // Compare mapped addresses across different servers.
    // If all servers see the same mapped address, it's cone NAT.
    // If different servers see different addresses, it's symmetric NAT.
    let all_same = results.iter().all(|r| r.mapped_address == first_mapped);

    if all_same {
        // Cone NAT (full, restricted, or port-restricted).
        // Distinguishing between these requires additional tests
        // (sending from different source ports, testing filterability)
        // which need cooperation from the STUN server (CHANGE-REQUEST).
        // With basic binding tests only, we classify as FullCone optimistically.
        // A more precise classification would require RFC 5780 (NAT Behavior Discovery).
        NatType::FullCone
    } else {
        NatType::Symmetric
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, IpAddr};

    #[test]
    fn test_nat_type_display() {
        assert_eq!(format!("{}", NatType::OpenInternet), "Open Internet (no NAT)");
        assert_eq!(format!("{}", NatType::Symmetric), "Symmetric NAT");
    }

    #[test]
    fn test_classify_nat_open_internet() {
        let local: SocketAddr = "1.2.3.4:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let results = vec![
            StunBindingResult {
                mapped_address: local,
                local_address: local,
                server_address: "10.0.0.1:3478".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                rtt: Duration::from_millis(10),
            },
        ];

        assert_eq!(classify_nat(&results, local), NatType::OpenInternet);
    }

    #[test]
    fn test_classify_nat_symmetric() {
        let local: SocketAddr = "192.168.1.100:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let results = vec![
            StunBindingResult {
                mapped_address: "1.2.3.4:10000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                local_address: local,
                server_address: "10.0.0.1:3478".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                rtt: Duration::from_millis(10),
            },
            StunBindingResult {
                mapped_address: "1.2.3.4:10001".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                local_address: local,
                server_address: "10.0.0.2:3478".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                rtt: Duration::from_millis(12),
            },
        ];

        assert_eq!(classify_nat(&results, local), NatType::Symmetric);
    }

    #[test]
    fn test_classify_nat_cone() {
        let local: SocketAddr = "192.168.1.100:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let mapped: SocketAddr = "1.2.3.4:10000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let results = vec![
            StunBindingResult {
                mapped_address: mapped,
                local_address: local,
                server_address: "10.0.0.1:3478".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                rtt: Duration::from_millis(10),
            },
            StunBindingResult {
                mapped_address: mapped,
                local_address: local,
                server_address: "10.0.0.2:3478".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                rtt: Duration::from_millis(12),
            },
        ];

        assert_eq!(classify_nat(&results, local), NatType::FullCone);
    }

    #[test]
    fn test_classify_nat_unknown_single_result() {
        let local: SocketAddr = "192.168.1.100:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let results = vec![
            StunBindingResult {
                mapped_address: "1.2.3.4:10000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                local_address: local,
                server_address: "10.0.0.1:3478".parse().unwrap_or_else(|e| panic!("parse: {e}")),
                rtt: Duration::from_millis(10),
            },
        ];

        assert_eq!(classify_nat(&results, local), NatType::Unknown);
    }

    #[test]
    fn test_classify_nat_empty() {
        let local: SocketAddr = "192.168.1.100:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        assert_eq!(classify_nat(&[], local), NatType::Unknown);
    }

    /// Integration test that contacts a real STUN server.
    /// Requires network access; run with: cargo test -p rvoip-rtp-core -- --ignored stun
    #[tokio::test]
    #[ignore]
    async fn test_get_public_address_real_server() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        let local: SocketAddr = "0.0.0.0:0".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let result = get_public_address(local).await;
        match result {
            Ok(addr) => {
                println!("Public address: {addr}");
                // Should not be a private/loopback address
                match addr.ip() {
                    IpAddr::V4(v4) => {
                        assert!(!v4.is_loopback(), "public addr should not be loopback");
                        assert!(!v4.is_private(), "public addr should not be private");
                    }
                    IpAddr::V6(v6) => {
                        assert!(!v6.is_loopback(), "public addr should not be loopback");
                    }
                }
            }
            Err(e) => {
                // May fail in CI without network
                println!("STUN test skipped (no network?): {e}");
            }
        }
    }

    /// Integration test for full NAT discovery.
    #[tokio::test]
    #[ignore]
    async fn test_discover_nat_type_real() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        let result = discover_nat_type(&[]).await;
        match result {
            Ok(info) => {
                println!("Public address: {}", info.public_addr);
                println!("NAT type: {}", info.nat_type);
                println!("Servers responded: {}", info.binding_results.len());
                for r in &info.binding_results {
                    println!("  {} -> {} (rtt: {:?})", r.server_address, r.mapped_address, r.rtt);
                }
            }
            Err(e) => {
                println!("NAT discovery skipped (no network?): {e}");
            }
        }
    }
}
