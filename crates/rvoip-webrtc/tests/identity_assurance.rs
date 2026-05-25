//! D2 — `verify_request_signature` surfaces the negotiated DTLS fingerprint
//! as `IdentityAssurance::DtlsFingerprint` once the route has remote SDP.
//!
//! Pre-D2 the adapter always returned `Anonymous`; the
//! `IdentityAssurance::DtlsFingerprint { algorithm, value }` variant added
//! in rvoip-core now carries the peer's certificate hash so callers can
//! correlate it with a trust store.

use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest, SignatureHeaders};
use rvoip_core::connection::Direction;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn verify_request_signature_returns_dtls_fingerprint_after_handshake_sdp() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter_a = WebRtcAdapter::new(config.clone());
    let adapter_b = WebRtcAdapter::new(config);

    let handle = adapter_a
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: adapter_a.capabilities(),
        })
        .await
        .expect("originate");
    let conn_id = handle.connection.id.clone();
    let offer = adapter_a.local_sdp(&conn_id).expect("offer");

    let inbound_id = adapter_b
        .apply_remote_offer(&offer)
        .await
        .expect("apply offer");
    let answer = adapter_b.local_sdp(&inbound_id).expect("answer");

    adapter_a
        .apply_remote_answer(conn_id.clone(), &answer)
        .await
        .expect("apply answer");

    // Outbound side: verify_request_signature surfaces the answer's fingerprint.
    let assurance_a = adapter_a
        .verify_request_signature(conn_id.clone(), empty_sig())
        .await
        .expect("verify");
    match assurance_a {
        IdentityAssurance::DtlsFingerprint { algorithm, value } => {
            assert!(!algorithm.is_empty(), "algorithm must be set");
            assert!(value.contains(':'), "value must be colon-hex");
        }
        other => panic!("expected DtlsFingerprint, got {other:?}"),
    }

    // Inbound side: verify_request_signature surfaces the offer's fingerprint.
    let assurance_b = adapter_b
        .verify_request_signature(inbound_id.clone(), empty_sig())
        .await
        .expect("verify");
    assert!(
        matches!(assurance_b, IdentityAssurance::DtlsFingerprint { .. }),
        "inbound assurance should be DtlsFingerprint, got {assurance_b:?}"
    );
}

#[tokio::test]
async fn verify_request_signature_unknown_connection_is_anonymous() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let assurance = adapter
        .verify_request_signature(rvoip_core::ids::ConnectionId::new(), empty_sig())
        .await
        .expect("verify");
    assert!(matches!(assurance, IdentityAssurance::Anonymous));
}

fn empty_sig() -> SignatureHeaders {
    SignatureHeaders {
        signature: String::new(),
        signature_input: String::new(),
        signature_key: None,
        signature_agent: None,
    }
}
