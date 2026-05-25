//! D2 — DTLS-SRTP fingerprint pinning enforcement.
//!
//! Covers three policy modes against the live SDP from a real loopback
//! `RvoipPeerConnection`:
//!   A. Pin list includes the real fingerprint → `apply_remote_offer` accepts.
//!   B. Pin list has a bogus value          → rejected with `FingerprintNotPinned`.
//!   C. Empty pin list (default)            → accepts (current behavior).
//!
//! Also exercises the [`FingerprintPolicyHook`] for per-route overrides.

use std::sync::Arc;

use async_trait::async_trait;
use rvoip_core::ids::ConnectionId;
use rvoip_webrtc::adapter::FingerprintPolicyHook;
use rvoip_webrtc::identity::{extract_fingerprints, DtlsFingerprint};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

/// Produce a fresh offerer SDP and the first fingerprint inside it.
async fn fresh_offer_with_fingerprint() -> (String, DtlsFingerprint) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let offerer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("offerer");
    offerer.add_local_audio_track().await.expect("local audio");
    let offer = offerer.create_offer_and_gather().await.expect("offer");
    let fp = extract_fingerprints(&offer)
        .into_iter()
        .next()
        .expect("offer carries an a=fingerprint: line");
    (offer, fp)
}

#[tokio::test]
async fn pinned_fingerprint_matching_real_offer_is_accepted() {
    let (offer, fp) = fresh_offer_with_fingerprint().await;
    let config = WebRtcConfig {
        pinned_fingerprints: vec![fp.clone()],
        ..WebRtcConfig::loopback()
    };
    let adapter = WebRtcAdapter::new(config);
    let conn_id = adapter
        .apply_remote_offer(&offer)
        .await
        .expect("pinned fingerprint must accept the matching offer");
    // Sanity: the stored remote DTLS fingerprint round-trips.
    let remote_fps = adapter
        .remote_dtls_fingerprint(&conn_id)
        .expect("remote fp");
    assert!(remote_fps.iter().any(|r| r.algorithm == fp.algorithm
        && r.value.eq_ignore_ascii_case(&fp.value)));
}

#[tokio::test]
async fn bogus_pinned_fingerprint_rejects_offer() {
    let (offer, _) = fresh_offer_with_fingerprint().await;
    let bogus = DtlsFingerprint {
        algorithm: "sha-256".into(),
        value: "00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00".into(),
    };
    let config = WebRtcConfig {
        pinned_fingerprints: vec![bogus],
        ..WebRtcConfig::loopback()
    };
    let adapter = WebRtcAdapter::new(config);
    let result = adapter.apply_remote_offer(&offer).await;
    let err = result.expect_err("bogus pin must reject");
    assert!(
        matches!(err, rvoip_webrtc::errors::WebRtcError::FingerprintNotPinned),
        "expected FingerprintNotPinned, got {err:?}"
    );
}

#[tokio::test]
async fn empty_pin_list_accepts_any_fingerprint() {
    let (offer, _) = fresh_offer_with_fingerprint().await;
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    adapter
        .apply_remote_offer(&offer)
        .await
        .expect("empty pin list = no enforcement = accept");
}

/// Hook that always returns the supplied fingerprint set, ignoring the
/// session hint — sufficient to verify the union behavior.
struct StaticHook(Vec<DtlsFingerprint>);

#[async_trait]
impl FingerprintPolicyHook for StaticHook {
    async fn allowed_fingerprints(
        &self,
        _conn: &ConnectionId,
        _session_hint: Option<&str>,
    ) -> Vec<DtlsFingerprint> {
        self.0.clone()
    }
}

#[tokio::test]
async fn policy_hook_union_with_static_list_accepts_match_from_hook() {
    let (offer, fp) = fresh_offer_with_fingerprint().await;
    // Static list is empty; hook supplies the real fingerprint.
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    adapter.set_fingerprint_policy(Arc::new(StaticHook(vec![fp.clone()])));
    adapter
        .apply_remote_offer(&offer)
        .await
        .expect("hook-supplied fingerprint must accept the matching offer");
}

#[tokio::test]
async fn policy_hook_with_bogus_entry_rejects_when_static_list_also_bogus() {
    let (offer, _) = fresh_offer_with_fingerprint().await;
    let bogus_a = DtlsFingerprint {
        algorithm: "sha-256".into(),
        value: "aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa".into(),
    };
    let bogus_b = DtlsFingerprint {
        algorithm: "sha-256".into(),
        value: "bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb:bb".into(),
    };
    let config = WebRtcConfig {
        pinned_fingerprints: vec![bogus_a],
        ..WebRtcConfig::loopback()
    };
    let adapter = WebRtcAdapter::new(config);
    adapter.set_fingerprint_policy(Arc::new(StaticHook(vec![bogus_b])));
    let err = adapter
        .apply_remote_offer(&offer)
        .await
        .expect_err("union of bogus values must still reject");
    assert!(matches!(
        err,
        rvoip_webrtc::errors::WebRtcError::FingerprintNotPinned
    ));
}
