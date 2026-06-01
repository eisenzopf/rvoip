//! Gap plan §2.2 — SIP adapter eager-stream parity.
//!
//! Confirms that `Connection.streams` is populated synchronously off the
//! adapter's `build_connection` path, matching the QUIC/WT adapters'
//! behavior. Pre-fix, the SIP adapter returned an empty `streams: vec![]`
//! and lazy-created the `SipMediaStream` only on the first `streams()`
//! call — a footgun for consumers that inspect `connection.streams`
//! synchronously off the inbound event.

use std::sync::Arc;

use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;

fn pick_free_udp_port() -> u16 {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral");
    sock.local_addr().expect("local_addr").port()
}

#[tokio::test]
async fn originate_populates_connection_streams_eagerly() {
    let sip_port = pick_free_udp_port();
    let coord = UnifiedCoordinator::new(SipConfig::local("eager-streams-test", sip_port))
        .await
        .expect("sip coordinator");
    let sip = SipAdapter::new(Arc::clone(&coord)).await.expect("sip adapter");

    let caps = <SipAdapter as ConnectionAdapter>::capabilities(&*sip);
    let handle = <SipAdapter as ConnectionAdapter>::originate(
        &*sip,
        OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: format!("sip:loopback@127.0.0.1:{}", sip_port),
            direction: Direction::Outbound,
            capabilities: caps,
            transport: None,
        },
    )
    .await
    .expect("originate");

    assert_eq!(
        handle.connection.streams.len(),
        1,
        "SipAdapter must populate Connection.streams eagerly at build_connection time \
         (QUIC/WT parity, gap plan §2.2). Got {} streams.",
        handle.connection.streams.len()
    );

    // The eager stream and the `streams()` lookup must agree.
    let lookup = <SipAdapter as ConnectionAdapter>::streams(&*sip, handle.connection.id.clone())
        .await
        .expect("streams lookup");
    assert_eq!(lookup.len(), 1, "streams() must return the eagerly-cached stream");
    assert_eq!(
        lookup[0].id(),
        handle.connection.streams[0].id(),
        "eager stream id must match streams() lookup id"
    );
}
