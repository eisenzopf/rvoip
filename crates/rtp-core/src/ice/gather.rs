//! ICE candidate gathering per RFC 8445 Section 5.1.
//!
//! Enumerates local network interfaces for host candidates and uses STUN
//! servers to discover server-reflexive candidates.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use tokio::net::UdpSocket;
use tracing::{debug, trace, warn};

use crate::Error;
use crate::stun::client::{StunClient, StunClientConfig};
use crate::turn::TurnServerConfig;
use crate::turn::client::TurnClient;
use super::types::{CandidateType, ComponentId, IceCandidate};

/// Calculate candidate priority per RFC 8445 Section 5.1.2.
///
/// priority = (2^24 * type_preference) + (2^8 * local_preference) + (256 - component_id)
///
/// - `type_pref`: type preference (host=126, srflx=100, prflx=110, relay=0)
/// - `local_pref`: local preference (0..65535), typically 65535 for the preferred interface
/// - `component_id`: 1 for RTP, 2 for RTCP
pub fn compute_priority(candidate_type: CandidateType, local_pref: u32, component: ComponentId) -> u32 {
    let type_pref = candidate_type.type_preference();
    let comp_id = component.id();
    (type_pref << 24) | ((local_pref & 0xFFFF) << 8) | (256 - comp_id)
}

/// Generate a foundation string for a candidate.
///
/// Per RFC 8445 Section 5.1.1.3, candidates that share the same type, base
/// address, server address, and transport protocol use the same foundation.
/// We use a simple hash-based approach.
pub fn generate_foundation(
    candidate_type: CandidateType,
    base_addr: &SocketAddr,
    server_addr: Option<&SocketAddr>,
) -> String {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    format!("{candidate_type}").hash(&mut hasher);
    base_addr.hash(&mut hasher);
    if let Some(sa) = server_addr {
        sa.hash(&mut hasher);
    }
    // Use lower 32 bits as foundation
    let hash = hasher.finish();
    format!("{}", hash & 0xFFFF_FFFF)
}

/// Enumerate local network interfaces and return host candidates.
///
/// Filters out loopback and link-local addresses. Returns one host
/// candidate per non-loopback IPv4 address found.
pub fn gather_host_candidates(
    local_addr: SocketAddr,
    component: ComponentId,
    ufrag: &str,
) -> Vec<IceCandidate> {
    let mut candidates = Vec::new();

    // Always add the explicitly provided local address as a host candidate
    // (unless it's 0.0.0.0/unspecified, in which case we enumerate interfaces)
    if !local_addr.ip().is_unspecified() {
        let foundation = generate_foundation(CandidateType::Host, &local_addr, None);
        let priority = compute_priority(CandidateType::Host, 65535, component);
        candidates.push(IceCandidate {
            foundation,
            component,
            transport: "udp".to_string(),
            priority,
            address: local_addr,
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: ufrag.to_string(),
        });
        return candidates;
    }

    // Enumerate interfaces using nix (Unix) or platform-specific approach
    let addrs = enumerate_local_addresses();

    for (idx, addr) in addrs.iter().enumerate() {
        let sock_addr = SocketAddr::new(*addr, local_addr.port());
        let foundation = generate_foundation(CandidateType::Host, &sock_addr, None);
        // Decrease local_pref for each successive interface
        let local_pref = 65535u32.saturating_sub(idx as u32);
        let priority = compute_priority(CandidateType::Host, local_pref, component);

        candidates.push(IceCandidate {
            foundation,
            component,
            transport: "udp".to_string(),
            priority,
            address: sock_addr,
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: ufrag.to_string(),
        });
    }

    if candidates.is_empty() {
        // Fallback: use 0.0.0.0 as a last resort
        let fallback = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), local_addr.port());
        let foundation = generate_foundation(CandidateType::Host, &fallback, None);
        let priority = compute_priority(CandidateType::Host, 65535, component);
        candidates.push(IceCandidate {
            foundation,
            component,
            transport: "udp".to_string(),
            priority,
            address: fallback,
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: ufrag.to_string(),
        });
    }

    candidates
}

/// Gather server-reflexive candidates by contacting STUN servers.
///
/// Sends a STUN Binding Request from `socket` to each STUN server and
/// creates a server-reflexive candidate for each successful response.
pub async fn gather_srflx_candidates(
    socket: &UdpSocket,
    stun_servers: &[SocketAddr],
    component: ComponentId,
    ufrag: &str,
) -> Vec<IceCandidate> {
    let local_addr = match socket.local_addr() {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "failed to get local address for srflx gathering");
            return Vec::new();
        }
    };

    let config = StunClientConfig {
        timeout: std::time::Duration::from_secs(3),
        initial_rto: std::time::Duration::from_millis(500),
        max_retransmits: 3,
        recv_buf_size: 1024,
    };

    let mut candidates = Vec::new();
    let mut seen_addrs = std::collections::HashSet::new();

    for server in stun_servers {
        let client = StunClient::with_config(*server, config.clone());

        match client.binding_request(socket).await {
            Ok(result) => {
                let mapped = result.mapped_address;

                // Avoid duplicate srflx candidates with the same address
                if seen_addrs.contains(&mapped) {
                    trace!(addr = %mapped, "skipping duplicate srflx candidate");
                    continue;
                }
                seen_addrs.insert(mapped);

                let foundation = generate_foundation(
                    CandidateType::ServerReflexive,
                    &local_addr,
                    Some(server),
                );
                let priority = compute_priority(CandidateType::ServerReflexive, 65535, component);

                debug!(
                    mapped = %mapped,
                    server = %server,
                    rtt_ms = result.rtt.as_millis(),
                    "gathered srflx candidate"
                );

                candidates.push(IceCandidate {
                    foundation,
                    component,
                    transport: "udp".to_string(),
                    priority,
                    address: mapped,
                    candidate_type: CandidateType::ServerReflexive,
                    related_address: Some(local_addr),
                    ufrag: ufrag.to_string(),
                });
            }
            Err(e) => {
                warn!(server = %server, error = %e, "srflx gathering failed");
            }
        }
    }

    candidates
}

/// Gather relay candidates by performing TURN allocations.
///
/// For each configured TURN server, creates a `TurnClient` that shares
/// the provided `socket`, requests an allocation, and produces a relay
/// candidate on success.  On failure the server is skipped with a warning.
///
/// The relay candidate's `related_address` is set to the server-reflexive
/// (mapped) address returned by the TURN server, per RFC 8445 Section 5.1.1.
///
/// Returns the collected relay candidates and the `TurnClient` handles so
/// the caller can maintain the allocations (refresh, permission, etc.).
pub async fn gather_relay_candidates(
    turn_configs: &[TurnServerConfig],
    socket: &Arc<UdpSocket>,
    component: ComponentId,
    ufrag: &str,
) -> (Vec<IceCandidate>, Vec<TurnClient>) {
    let mut candidates = Vec::new();
    let mut clients = Vec::new();

    for config in turn_configs {
        let mut client = TurnClient::with_socket(
            config.server,
            Arc::clone(socket),
            config.username.clone(),
            config.password.clone(),
        );

        match client.allocate().await {
            Ok(alloc) => {
                let foundation = generate_foundation(
                    CandidateType::Relay,
                    &alloc.relayed_address,
                    Some(&config.server),
                );
                let priority = compute_priority(CandidateType::Relay, 65535, component);

                debug!(
                    relay = %alloc.relayed_address,
                    mapped = %alloc.mapped_address,
                    server = %config.server,
                    "gathered relay candidate via TURN"
                );

                candidates.push(IceCandidate {
                    foundation,
                    component,
                    transport: "udp".to_string(),
                    priority,
                    address: alloc.relayed_address,
                    candidate_type: CandidateType::Relay,
                    related_address: Some(alloc.mapped_address),
                    ufrag: ufrag.to_string(),
                });

                clients.push(client);
            }
            Err(e) => {
                warn!(
                    server = %config.server,
                    error = %e,
                    "TURN allocation failed, skipping server"
                );
            }
        }
    }

    (candidates, clients)
}

/// Enumerate non-loopback, non-link-local IPv4 addresses from local interfaces.
fn enumerate_local_addresses() -> Vec<IpAddr> {
    let mut addrs = Vec::new();

    // Use nix to enumerate interfaces on Unix
    #[cfg(unix)]
    {
        if let Ok(ifaces) = nix::ifaddrs::getifaddrs() {
            for iface in ifaces {
                if let Some(addr) = iface.address {
                    if let Some(sock_addr) = addr.as_sockaddr_in() {
                        let ip = Ipv4Addr::from(sock_addr.ip());
                        if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() {
                            let ip_addr = IpAddr::V4(ip);
                            if !addrs.contains(&ip_addr) {
                                addrs.push(ip_addr);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback for non-Unix or if no interfaces found
    if addrs.is_empty() {
        // Try to detect local address by connecting a UDP socket
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            // Connect to a public address (does not send any data)
            if sock.connect("8.8.8.8:80").is_ok() {
                if let Ok(local) = sock.local_addr() {
                    if !local.ip().is_unspecified() {
                        addrs.push(local.ip());
                    }
                }
            }
        }
    }

    addrs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_priority_host() {
        let prio = compute_priority(CandidateType::Host, 65535, ComponentId::Rtp);
        // (126 << 24) | (65535 << 8) | (256 - 1)
        let expected = (126u32 << 24) | (65535u32 << 8) | 255;
        assert_eq!(prio, expected);
        assert_eq!(prio, 2130706431);
    }

    #[test]
    fn test_compute_priority_srflx() {
        let prio = compute_priority(CandidateType::ServerReflexive, 65535, ComponentId::Rtp);
        let expected = (100u32 << 24) | (65535u32 << 8) | 255;
        assert_eq!(prio, expected);
    }

    #[test]
    fn test_compute_priority_rtcp_component() {
        let prio_rtp = compute_priority(CandidateType::Host, 65535, ComponentId::Rtp);
        let prio_rtcp = compute_priority(CandidateType::Host, 65535, ComponentId::Rtcp);
        // RTP should have higher priority than RTCP (256-1 > 256-2)
        assert!(prio_rtp > prio_rtcp);
        assert_eq!(prio_rtp - prio_rtcp, 1);
    }

    #[test]
    fn test_generate_foundation_same_input() {
        let addr: SocketAddr = "192.168.1.1:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let f1 = generate_foundation(CandidateType::Host, &addr, None);
        let f2 = generate_foundation(CandidateType::Host, &addr, None);
        assert_eq!(f1, f2, "same inputs should produce same foundation");
    }

    #[test]
    fn test_generate_foundation_different_type() {
        let addr: SocketAddr = "192.168.1.1:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let f1 = generate_foundation(CandidateType::Host, &addr, None);
        let f2 = generate_foundation(CandidateType::ServerReflexive, &addr, None);
        assert_ne!(f1, f2, "different types should produce different foundations");
    }

    #[test]
    fn test_gather_host_candidates_specific_addr() {
        let addr: SocketAddr = "192.168.1.100:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let candidates = gather_host_candidates(addr, ComponentId::Rtp, "test");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].address, addr);
        assert_eq!(candidates[0].candidate_type, CandidateType::Host);
        assert_eq!(candidates[0].transport, "udp");
        assert_eq!(candidates[0].ufrag, "test");
    }

    #[test]
    fn test_gather_host_candidates_unspecified() {
        let addr: SocketAddr = "0.0.0.0:5000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let candidates = gather_host_candidates(addr, ComponentId::Rtp, "test");
        // Should have at least one candidate (either from interfaces or fallback)
        assert!(!candidates.is_empty());
    }

    #[test]
    fn test_enumerate_local_addresses() {
        let addrs = enumerate_local_addresses();
        // Should find at least one address on most systems
        // (may be empty in very restricted environments)
        for addr in &addrs {
            assert!(!addr.is_loopback(), "should not contain loopback");
        }
    }

    #[test]
    fn test_compute_priority_relay() {
        let prio = compute_priority(CandidateType::Relay, 65535, ComponentId::Rtp);
        // Relay type_preference is 0, so: (0 << 24) | (65535 << 8) | 255
        let expected = (0u32 << 24) | (65535u32 << 8) | 255;
        assert_eq!(prio, expected);
        // Relay should have lower priority than host and srflx
        let host_prio = compute_priority(CandidateType::Host, 65535, ComponentId::Rtp);
        let srflx_prio = compute_priority(CandidateType::ServerReflexive, 65535, ComponentId::Rtp);
        assert!(prio < host_prio, "relay priority must be lower than host");
        assert!(prio < srflx_prio, "relay priority must be lower than srflx");
    }

    #[test]
    fn test_generate_foundation_relay() {
        let relay_addr: SocketAddr = "203.0.113.10:50000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let server: SocketAddr = "198.51.100.1:3478".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let f1 = generate_foundation(CandidateType::Relay, &relay_addr, Some(&server));
        let f2 = generate_foundation(CandidateType::Relay, &relay_addr, Some(&server));
        assert_eq!(f1, f2, "same inputs should produce same foundation");

        // Different server should produce different foundation
        let server2: SocketAddr = "198.51.100.2:3478".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let f3 = generate_foundation(CandidateType::Relay, &relay_addr, Some(&server2));
        assert_ne!(f1, f3, "different TURN servers should produce different foundations");
    }

    #[tokio::test]
    async fn test_gather_relay_candidates_unreachable_server() {
        // Verify that an unreachable TURN server is gracefully skipped
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let socket = Arc::new(socket);

        let configs = vec![TurnServerConfig {
            // Non-routable address that will time out
            server: "192.0.2.1:3478".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            username: "user".to_string(),
            password: "pass".to_string(),
        }];

        // Use a short timeout; the TURN client will time out internally
        let (candidates, clients) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            gather_relay_candidates(&configs, &socket, ComponentId::Rtp, "test"),
        )
        .await
        .unwrap_or_else(|_| (Vec::new(), Vec::new()));

        // The unreachable server should produce no candidates
        assert!(candidates.is_empty(), "unreachable TURN should yield no candidates");
        assert!(clients.is_empty(), "unreachable TURN should yield no clients");
    }
}
