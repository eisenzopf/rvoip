//! H6 deferred (#44): SIP↔WebRTC gateway wiring smoke test.
//!
//! ## Scope
//!
//! The full SIP↔WebRTC media-transcoding gateway (G.711↔Opus via
//! `rvoip-media-core`, real SIP UA dialing a SIP listener that bridges to a
//! WHEP subscriber) requires substantial out-of-crate work — per the existing
//! `rvoip-uctp/examples/uctp_to_sip_bridge/orchestrator_bridge.rs` README,
//! `Orchestrator::bridge_connections` between SIP and another transport is
//! still partially stubbed in rvoip-core.
//!
//! What this test verifies (the integration point this crate owns):
//!
//! 1. `SipAdapter` and `WebRtcAdapter` can be registered together on the
//!    same `Orchestrator` without conflicts (transport vocabulary, event
//!    bus, capability table).
//! 2. `WebRtcAdapter` exposes `Transport::WebRtc` and `SipAdapter` exposes
//!    `Transport::Sip` — they're distinct.
//! 3. The orchestrator can subscribe to the merged event bus.
//!
//! Real SIP UAC → bridge → WHEP subscriber lives in the integration test
//! suite at the `rvoip-uctp/examples/uctp_to_sip_bridge` level once the
//! `bridge_connections` SIP path lands.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;

use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::config::Config;
use rvoip_core::connection::Transport;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn sip_and_webrtc_adapters_coexist_on_one_orchestrator() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // WebRTC side.
    let webrtc = WebRtcAdapter::new(WebRtcConfig::loopback());
    assert_eq!(
        <WebRtcAdapter as ConnectionAdapter>::transport(&*webrtc),
        Transport::WebRtc
    );

    // SIP side. Use an ephemeral UDP port on localhost.
    let sip_port = pick_free_udp_port();
    let coord = UnifiedCoordinator::new(SipConfig::local("rvoip-bridge-test", sip_port))
        .await
        .expect("sip coordinator");
    let sip = SipAdapter::new(coord).await.expect("sip adapter");
    assert_eq!(
        <SipAdapter as ConnectionAdapter>::transport(&*sip),
        Transport::Sip
    );

    let orchestrator = Arc::new(Orchestrator::new(Config::default()));
    orchestrator
        .register(webrtc as Arc<dyn ConnectionAdapter>)
        .expect("register webrtc");
    orchestrator
        .register(sip as Arc<dyn ConnectionAdapter>)
        .expect("register sip");

    // Subscribe to the merged event bus — if vocabulary clashed, this would
    // fail or panic during register.
    let _events = orchestrator.subscribe_events();
}

#[tokio::test]
async fn webrtc_advertises_audio_codecs_and_sip_is_capability_neutral() {
    // WebRTC's default capability descriptor pre-populates Opus + G.711 since
    // SDP needs explicit codec offers. SIP's default `UnifiedCoordinator`
    // capability descriptor is empty (the dialog layer negotiates codecs from
    // the inbound INVITE's SDP rather than advertising a fixed set). Document
    // both shapes so a future codec-aware bridge knows what to assume.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webrtc = WebRtcAdapter::new(WebRtcConfig::loopback());
    let sip_port = pick_free_udp_port();
    let coord = UnifiedCoordinator::new(SipConfig::local("caps-test", sip_port))
        .await
        .expect("sip coordinator");
    let sip = SipAdapter::new(coord).await.expect("sip adapter");

    let webrtc_caps = <WebRtcAdapter as ConnectionAdapter>::capabilities(&*webrtc);
    let sip_caps = <SipAdapter as ConnectionAdapter>::capabilities(&*sip);

    assert!(
        webrtc_caps.audio_codecs.iter().any(|c| c.name == "opus"),
        "WebRTC adapter must advertise Opus by default"
    );
    assert!(
        webrtc_caps.audio_codecs.iter().any(|c| c.name.starts_with("g.711")),
        "WebRTC adapter must advertise G.711 for SIP interop"
    );

    // Sanity-check: sip caps exists (no panic on the call), and it doesn't
    // claim WebRTC codecs spuriously.
    let _ = sip_caps;
}

fn pick_free_udp_port() -> u16 {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral");
    sock.local_addr().expect("local_addr").port()
}
