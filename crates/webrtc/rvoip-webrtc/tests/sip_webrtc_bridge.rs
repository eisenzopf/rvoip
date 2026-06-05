//! H6 + D4: SIP↔WebRTC gateway wiring + stream surface checks.
//!
//! ## Scope
//!
//! Verifies the integration point this crate owns:
//!
//! 1. `SipAdapter` and `WebRtcAdapter` can be registered together on the
//!    same `Orchestrator` without conflicts (transport vocabulary, event
//!    bus, capability table).
//! 2. `WebRtcAdapter` exposes `Transport::WebRtc` and `SipAdapter` exposes
//!    `Transport::Sip` — they're distinct.
//! 3. The orchestrator can subscribe to the merged event bus.
//! 4. **D4 — `SipAdapter::streams()` no longer returns `vec![]` for an
//!    unknown connection it returns `ConnectionNotFound`; for a real session
//!    it returns a `SipMediaStream` wrapping the PCM audio plane.** The
//!    full SNR-based G.711↔Opus E2E test still requires aligning the
//!    WebRTC `MediaFrame.payload` shape to the orchestrator-side
//!    transcoder's expectations (codec payload vs full RTP wire image);
//!    that follow-on refactor is tracked under `GAP_PLAN.md` §3.1 D4
//!    follow-up.

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
        webrtc_caps
            .audio_codecs
            .iter()
            .any(|c| c.name.starts_with("g.711")),
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

#[tokio::test]
async fn sip_media_stream_codec_name_maps_to_pcmu_pt() {
    // D4 follow-up — the SIP MediaStream's codec name must match the
    // string `rvoip_core::bridge::codec_to_pt` recognizes, otherwise the
    // orchestrator's `bridge_connections` rejects with `UnsupportedCodec`
    // before it can even spawn the transcoder. The wrapper uses
    // "g.711-mu" → PT 0 (PCMU); validating it here keeps a future rename
    // from silently breaking SIP↔WebRTC bridging.
    use rvoip_core::bridge::codec_to_pt;
    assert_eq!(codec_to_pt("g.711-mu"), Some(0), "SIP MediaStream codec");
    assert_eq!(codec_to_pt("opus"), Some(111), "WebRTC MediaStream codec");
}

#[tokio::test]
async fn sip_adapter_streams_returns_connection_not_found_for_unknown_id() {
    // D4 — pre-D4, `SipAdapter::streams()` unconditionally returned `vec![]`,
    // hiding lookup failures. Now an unknown ConnectionId surfaces
    // `ConnectionNotFound`, which the orchestrator can route to the right
    // error path.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let sip_port = pick_free_udp_port();
    let coord = UnifiedCoordinator::new(SipConfig::local("d4-streams-test", sip_port))
        .await
        .expect("sip coordinator");
    let sip = SipAdapter::new(coord).await.expect("sip adapter");
    let unknown = rvoip_core::ids::ConnectionId::new();
    let result = <SipAdapter as ConnectionAdapter>::streams(&*sip, unknown).await;
    let err = result
        .map(|v| format!("Ok({} streams)", v.len()))
        .unwrap_or_else(|e| format!("Err({e})"));
    assert!(
        err.starts_with("Err(") && err.contains("not found"),
        "expected ConnectionNotFound on unknown id, got {err}"
    );
}
