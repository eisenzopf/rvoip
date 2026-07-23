//! Proves ICE (RFC 8445) connectivity checks also complete when driven
//! through the shared-socket demux bridge over a real `UdpRtpTransport` —
//! not just over two directly-owned sockets, which is all
//! `rvoip-nat-core`'s own `ice_agent_connectivity_test.rs` covers.
//!
//! A `UdpRtpTransport`'s socket is shared and not `connect()`-ed (it
//! receives from whatever peer sent a datagram, same as any real call's
//! RTP socket has to). `classify_rtp_mux_packet` already sorts STUN bytes
//! out from RTP/RTCP on that shared socket (RFC 7983); this test proves
//! those bytes actually reach a real ICE agent through
//! `ice_conn_adapter()` + `subscribe_stun_datagrams()`, and that two
//! independent transports still complete a real connectivity check
//! end to end — mirrors `dtls_srtp_transport_bridge_test.rs` for DTLS.

#![cfg(feature = "ice")]

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_nat_core::{IceAgent, IceRole};
use rvoip_rtp_core::transport::{RtpTransportConfig, UdpRtpTransport};

/// Loopback-only, so the test doesn't depend on which of the machine's
/// (possibly several) real network interfaces the OS happens to prefer
/// for host candidate gathering — same reasoning as
/// `rvoip-nat-core`'s own connectivity test.
fn loopback_only() -> Arc<dyn Fn(IpAddr) -> bool + Send + Sync> {
    Arc::new(|ip: IpAddr| ip.is_loopback())
}

#[tokio::test]
async fn ice_connectivity_completes_through_the_shared_socket_demux_bridge() {
    let _ = tracing_subscriber::fmt::try_init();

    let transport_a = UdpRtpTransport::new(RtpTransportConfig::default())
        .await
        .expect("transport A");
    let transport_b = UdpRtpTransport::new(RtpTransportConfig::default())
        .await
        .expect("transport B");

    let agent_a = Arc::new(
        IceAgent::new_with_shared_socket_and_ip_filter(
            IceRole::Controlling,
            &[],
            transport_a.ice_conn_adapter(),
            Some(loopback_only()),
        )
        .await
        .expect("agent A"),
    );
    let agent_b = Arc::new(
        IceAgent::new_with_shared_socket_and_ip_filter(
            IceRole::Controlled,
            &[],
            transport_b.ice_conn_adapter(),
            Some(loopback_only()),
        )
        .await
        .expect("agent B"),
    );

    // Pump each transport's demuxed STUN datagrams into its agent — the
    // same role a real caller's own event loop plays; a production
    // caller (e.g. `rvoip-sip`) would drive this identically.
    let mut stun_rx_a = transport_a.subscribe_stun_datagrams();
    let pump_agent_a = agent_a.clone();
    tokio::spawn(async move {
        while let Some((buf, addr)) = stun_rx_a.recv().await {
            pump_agent_a.handle_incoming_stun(&buf, addr).await;
        }
    });
    let mut stun_rx_b = transport_b.subscribe_stun_datagrams();
    let pump_agent_b = agent_b.clone();
    tokio::spawn(async move {
        while let Some((buf, addr)) = stun_rx_b.recv().await {
            pump_agent_b.handle_incoming_stun(&buf, addr).await;
        }
    });

    let (a_ufrag, a_pwd) = agent_a.local_credentials().await;
    let (b_ufrag, b_pwd) = agent_b.local_credentials().await;

    let a_candidates = agent_a.gather_candidates().await.expect("gather A");
    let b_candidates = agent_b.gather_candidates().await.expect("gather B");
    assert!(!a_candidates.is_empty());
    assert!(!b_candidates.is_empty());

    for c in &b_candidates {
        agent_a
            .add_remote_candidate(c)
            .expect("add B candidate to A");
    }
    for c in &a_candidates {
        agent_b
            .add_remote_candidate(c)
            .expect("add A candidate to B");
    }

    let connect_a = agent_a.clone();
    let connect_b = agent_b.clone();
    let task_a = tokio::spawn(async move { connect_a.connect(b_ufrag, b_pwd).await });
    let task_b = tokio::spawn(async move { connect_b.connect(a_ufrag, a_pwd).await });

    let (result_a, result_b) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(task_a, task_b)
    })
    .await
    .expect("ICE connectivity check over the demux bridge must complete within 10s");

    let addr_a = result_a
        .expect("task A panicked")
        .expect("A failed to connect");
    let addr_b = result_b
        .expect("task B panicked")
        .expect("B failed to connect");

    // Not asserting the exact port against the originally gathered
    // candidate list: RFC 8445 explicitly allows a peer-reflexive
    // candidate discovered live during the check to end up as the
    // winning pair, which is legitimate ICE behavior, not a bug.
    assert!(addr_a.ip().is_loopback());
    assert!(addr_b.ip().is_loopback());
    assert_ne!(addr_a.port(), 0);
    assert_ne!(addr_b.port(), 0);
}
