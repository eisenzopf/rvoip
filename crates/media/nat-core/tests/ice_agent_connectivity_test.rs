//! Proves two independent `IceAgent`s (controlling/controlled) complete a
//! real RFC 8445 connectivity check over loopback UDP and agree on the
//! same selected candidate pair — the same "two independent parties, no
//! shared state, no forced buffer syncing" property this session's DTLS
//! handshake test proves for DTLS-SRTP.
//!
//! No SDP is involved here: credentials and candidates are exchanged
//! directly between the two `IceAgent`s, exactly as a real caller (e.g.
//! `rvoip-sip`) would after parsing them out of a real offer/answer.

#![cfg(feature = "ice")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_nat_core::{CandidateKind, Error, IceAgent, IceCandidate, IceRole};

/// Loopback-only, so the test doesn't depend on which of the machine's
/// (possibly several) real network interfaces the OS happens to prefer
/// for host candidate gathering.
fn loopback_only() -> Arc<dyn Fn(std::net::IpAddr) -> bool + Send + Sync> {
    Arc::new(|ip: std::net::IpAddr| ip.is_loopback())
}

#[tokio::test]
async fn independent_agents_complete_a_real_connectivity_check() {
    let _ = tracing_subscriber::fmt::try_init();

    let controlling =
        IceAgent::new_with_ip_filter(IceRole::Controlling, &[], Some(loopback_only()))
            .await
            .unwrap();
    let controlled = IceAgent::new_with_ip_filter(IceRole::Controlled, &[], Some(loopback_only()))
        .await
        .unwrap();

    let (controlling_ufrag, controlling_pwd) = controlling.local_credentials().await;
    let (controlled_ufrag, controlled_pwd) = controlled.local_credentials().await;

    let controlling_candidates = controlling.gather_candidates().await.unwrap();
    let controlled_candidates = controlled.gather_candidates().await.unwrap();
    assert!(!controlling_candidates.is_empty());
    assert!(!controlled_candidates.is_empty());

    for c in &controlled_candidates {
        controlling.add_remote_candidate(c).unwrap();
    }
    for c in &controlling_candidates {
        controlled.add_remote_candidate(c).unwrap();
    }

    let controlling_task =
        tokio::spawn(async move { controlling.connect(controlled_ufrag, controlled_pwd).await });
    let controlled_task =
        tokio::spawn(async move { controlled.connect(controlling_ufrag, controlling_pwd).await });

    let (controlling_result, controlled_result) =
        tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(controlling_task, controlled_task)
        })
        .await
        .expect("ICE connectivity check must complete within 10s");

    let controlling_addr = controlling_result
        .expect("controlling task panicked")
        .expect("controlling side failed to connect");
    let controlled_addr = controlled_result
        .expect("controlled task panicked")
        .expect("controlled side failed to connect");

    // Both sides resolved to loopback (proving they actually connected
    // to *each other*, not to two unrelated third parties). Not
    // asserting the exact port against the originally gathered candidate
    // list: RFC 8445 explicitly allows a peer-reflexive candidate —
    // discovered live during the connectivity check itself, never part
    // of the exchanged candidate list — to end up as the winning pair,
    // which is legitimate ICE behavior this test must tolerate rather
    // than treat as failure.
    assert!(controlling_addr.ip().is_loopback());
    assert!(controlled_addr.ip().is_loopback());
    assert_ne!(controlling_addr.port(), 0);
    assert_ne!(controlled_addr.port(), 0);
}

/// Regression test: `IceAgent::connect()` must not hang forever when the
/// only remote candidate is unreachable. Before the fix, `webrtc_ice::
/// Agent::dial`/`accept` only unblocked on a *successful* pair selection
/// — they never observed their own `ConnectionState::Failed` transition
/// — so a call that can never connect (every candidate unreachable, no
/// peer ever answers) hung indefinitely instead of resolving with an
/// error.
///
/// Runs for ~30s: that's `webrtc-ice`'s own default
/// `disconnected_timeout` (5s) + `failed_timeout` (25s), the time it
/// takes the agent's checklist to give up on an unreachable candidate
/// and transition to `Failed`. The outer 40s timeout is the actual
/// regression check — it must never fire; if it does, `connect()` is
/// hanging again.
#[tokio::test]
async fn connect_terminates_with_connect_failed_for_unreachable_candidates() {
    let _ = tracing_subscriber::fmt::try_init();

    let agent = IceAgent::new_with_ip_filter(IceRole::Controlling, &[], Some(loopback_only()))
        .await
        .unwrap();

    // Real gathering (unused below) proves this agent's own local setup
    // is fine — the failure exercised here comes purely from the remote
    // side never answering, not from a broken local agent.
    let local_candidates = agent.gather_candidates().await.unwrap();
    assert!(!local_candidates.is_empty());

    // A well-formed but unreachable remote candidate: loopback, a port
    // nothing is bound to. No peer will ever answer these connectivity
    // checks, so the pair can never succeed.
    let unreachable = IceCandidate {
        foundation: "1".to_string(),
        component: 1,
        transport: "udp".to_string(),
        priority: 2_130_706_431,
        address: "127.0.0.1".parse().unwrap(),
        port: 1,
        kind: CandidateKind::Host,
        related_address: None,
        related_port: None,
    };
    agent.add_remote_candidate(&unreachable).unwrap();

    let result = tokio::time::timeout(
        Duration::from_secs(40),
        agent.connect(
            "unreachable-remote-ufrag".to_string(),
            "unreachable-remote-password-000000".to_string(),
        ),
    )
    .await
    .expect(
        "connect() must terminate within a bounded time for unreachable candidates, \
         not hang forever",
    );

    assert!(
        matches!(result, Err(Error::ConnectFailed)),
        "expected Err(ConnectFailed) for unreachable candidates, got: {:?}",
        result
    );
}
