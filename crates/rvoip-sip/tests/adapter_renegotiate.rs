//! Gap plan §4.2C v1 punch list — `SipAdapter::renegotiate_media`.
//!
//! Smoke coverage:
//!
//! 1. **Empty capabilities** returns `RvoipError::UnsupportedCodec`
//!    without touching the SIP layer. The orchestrator should never
//!    drive a re-INVITE with no codec choices.
//!
//! 2. **Unknown connection** returns `RvoipError::ConnectionNotFound`
//!    (same shape as the other adapter methods — hold/resume/dtmf).
//!
//! The full re-INVITE round-trip (originate → 200 OK answer →
//! `NegotiateSDPAsUAC` updates `session.negotiated_config`) needs a
//! real UAS counterparty; that's covered by the higher-level
//! end-to-end suites (audio_roundtrip_integration, etc.) once the
//! orchestrator-driven renegotiate flow is exercised there.

use std::sync::Arc;

use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo};
use rvoip_core::error::RvoipError;
use rvoip_core::ids::ConnectionId;
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;

fn pick_free_udp_port() -> u16 {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral");
    sock.local_addr().expect("local_addr").port()
}

async fn fresh_adapter() -> Arc<SipAdapter> {
    let sip_port = pick_free_udp_port();
    let coord = UnifiedCoordinator::new(SipConfig::local("reneg-test", sip_port))
        .await
        .expect("sip coordinator");
    SipAdapter::new(Arc::clone(&coord))
        .await
        .expect("sip adapter")
}

#[tokio::test]
async fn renegotiate_media_rejects_empty_capabilities() {
    let sip = fresh_adapter().await;
    let caps = CapabilityDescriptor::default(); // empty audio_codecs
    let err = <SipAdapter as ConnectionAdapter>::renegotiate_media(
        &*sip,
        ConnectionId::new(),
        caps,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, RvoipError::UnsupportedCodec(_)),
        "empty capabilities must surface UnsupportedCodec; got {err:?}"
    );
}

#[tokio::test]
async fn renegotiate_media_returns_connection_not_found_for_unknown_conn() {
    let sip = fresh_adapter().await;
    let caps = CapabilityDescriptor {
        audio_codecs: vec![CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
        }],
        ..Default::default()
    };
    let err = <SipAdapter as ConnectionAdapter>::renegotiate_media(
        &*sip,
        ConnectionId::new(),
        caps,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, RvoipError::ConnectionNotFound(_)),
        "unknown ConnectionId must surface ConnectionNotFound; got {err:?}"
    );
}
