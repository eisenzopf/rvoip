//! Announcing an external address from an `IceAgent` built on a shared socket.
//!
//! A media server behind static NAT listens on a private address and is reached
//! on a public one with the same ports. Without a way to say so, host candidates
//! carry the private address, the peer sends RTP there, and the call connects
//! silent.
//!
//! `stun_servers` cannot fill that gap on this path: `webrtc-ice` skips
//! server-reflexive gathering entirely when the socket is multiplexed
//! (`agent_gather.rs`: `UDPNetwork::Muxed(_) => continue`). RFC 8445 §5.1.1.2
//! would allow sending the Binding Request from the host candidate — which is
//! what browsers do — but that path is not implemented. The second test pins
//! that limitation so it stays documented instead of being rediscovered.

#![cfg(feature = "ice")]

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use rvoip_nat_core::{CandidateKind, IceAgent, IceRole, SharedIceSocket};

/// An address that exists on no interface of any machine running this test.
/// TEST-NET-3 (RFC 5737) is reserved for documentation, so a host candidate
/// carrying it can only have come from the external mapping.
const EXTERNAL_IP: &str = "203.0.113.10";

fn external_ip() -> IpAddr {
    EXTERNAL_IP.parse().expect("external address")
}

fn loopback() -> IpAddr {
    "127.0.0.1".parse().expect("loopback address")
}

/// Writes nowhere. Gathering never sends, and these tests never reach the
/// connectivity-check phase, so a sink is enough to build the agent.
struct NullSocket {
    local_addr: SocketAddr,
}

#[async_trait::async_trait]
impl SharedIceSocket for NullSocket {
    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

fn null_socket() -> Arc<dyn SharedIceSocket> {
    Arc::new(NullSocket {
        local_addr: "127.0.0.1:40000".parse().expect("socket address"),
    })
}

/// Loopback-only, so the result does not depend on which of the machine's
/// interfaces the OS happens to prefer.
fn loopback_only() -> Arc<dyn Fn(std::net::IpAddr) -> bool + Send + Sync> {
    Arc::new(|ip: std::net::IpAddr| ip.is_loopback())
}

#[tokio::test]
async fn external_ips_are_announced_instead_of_the_local_address() {
    let _ = tracing_subscriber::fmt::try_init();

    let agent = IceAgent::new_with_shared_socket_and_external_ips(
        IceRole::Controlling,
        &[],
        null_socket(),
        Some(loopback_only()),
        &[EXTERNAL_IP.to_string()],
    )
    .await
    .expect("agent with external IP mapping");

    let candidates = agent.gather_candidates().await.expect("gather candidates");
    assert!(!candidates.is_empty(), "gathering produced no candidates");

    let hosts: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.kind == CandidateKind::Host)
        .collect();
    assert!(
        !hosts.is_empty(),
        "no host candidate was gathered: {candidates:?}"
    );

    assert!(
        hosts
            .iter()
            .any(|candidate| candidate.address == external_ip()),
        "no host candidate carried the external address {EXTERNAL_IP}: {hosts:?}"
    );

    // The mapping replaces the local address rather than adding to it. This is
    // the trap worth pinning: peers on the same LAN now depend on hairpin NAT.
    assert!(
        !hosts
            .iter()
            .any(|candidate| candidate.address == loopback()),
        "the local address is still advertised, so the mapping did not replace it: {hosts:?}"
    );

    agent.close().await.expect("close agent");
}

/// Without the mapping, the same agent announces the local address — proving
/// the test above measures the mapping and not some ambient default.
#[tokio::test]
async fn without_external_ips_the_local_address_is_announced() {
    let _ = tracing_subscriber::fmt::try_init();

    let agent = IceAgent::new_with_shared_socket_and_ip_filter(
        IceRole::Controlling,
        &[],
        null_socket(),
        Some(loopback_only()),
    )
    .await
    .expect("agent without external IP mapping");

    let candidates = agent.gather_candidates().await.expect("gather candidates");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CandidateKind::Host
                && candidate.address == loopback()),
        "expected a loopback host candidate: {candidates:?}"
    );

    agent.close().await.expect("close agent");
}

/// `stun_servers` alone produces no server-reflexive candidate on a shared
/// socket. This is a limitation of `webrtc-ice`, not of the configuration, and
/// it used to be silent — which is what made it expensive to diagnose.
#[tokio::test]
async fn stun_servers_alone_gather_no_reflexive_candidate_on_a_shared_socket() {
    let _ = tracing_subscriber::fmt::try_init();

    let agent = IceAgent::new_with_shared_socket_and_ip_filter(
        IceRole::Controlling,
        &["stun:stun.l.google.com:19302".to_string()],
        null_socket(),
        Some(loopback_only()),
    )
    .await
    .expect("agent with STUN configured");

    let candidates = agent.gather_candidates().await.expect("gather candidates");
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.kind == CandidateKind::ServerReflexive),
        "a server-reflexive candidate appeared on a muxed socket; if webrtc-ice \
         gained that path, the warning in new_with_shared_socket_and_external_ips \
         is now wrong: {candidates:?}"
    );

    agent.close().await.expect("close agent");
}

/// A well-formed pair whose local side is not an address this host has is
/// refused when the agent is built, naming the offending entry.
///
/// Left to `webrtc-ice`, the same input builds fine and fails much later, at
/// gathering, with `ErrCandidateIpNotFound` and no indication of which entry
/// caused it. Refusing beats the alternative of dropping the entry: that would
/// fall back to announcing the private address, which is the silent call this
/// mapping exists to prevent.
#[tokio::test]
async fn a_pair_naming_a_local_address_this_host_lacks_is_refused_up_front() {
    let _ = tracing_subscriber::fmt::try_init();

    // TEST-NET-1, reserved for documentation, so it is on no real interface.
    let bogus_local = "192.0.2.99";

    let built = IceAgent::new_with_shared_socket_and_external_ips(
        IceRole::Controlling,
        &[],
        null_socket(),
        Some(loopback_only()),
        &[format!("{EXTERNAL_IP}/{bogus_local}")],
    )
    .await;

    let message = match built {
        Ok(_) => panic!("a mapping that maps nothing must not build"),
        Err(error) => error.to_string(),
    };
    assert!(
        message.contains(bogus_local),
        "the error must name the offending local address: {message}"
    );
    assert!(
        message.contains("ErrCandidateIpNotFound"),
        "the error must say what would have happened otherwise: {message}"
    );
}

/// The paired form is accepted when its local side really is an interface
/// address, so the validation rejects the broken case and not the shape.
#[tokio::test]
async fn a_pair_naming_a_real_local_address_is_accepted() {
    let _ = tracing_subscriber::fmt::try_init();

    let agent = IceAgent::new_with_shared_socket_and_external_ips(
        IceRole::Controlling,
        &[],
        null_socket(),
        Some(loopback_only()),
        &[format!("{EXTERNAL_IP}/127.0.0.1")],
    )
    .await
    .expect("a mapping whose local side exists must build");

    let candidates = agent.gather_candidates().await.expect("gather candidates");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.address == external_ip()),
        "the paired mapping did not take effect: {candidates:?}"
    );

    agent.close().await.expect("close agent");
}
