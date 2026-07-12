//! Smoke-test every `ConnectionAdapter` method on `WebRtcAdapter`.

use std::time::Duration;

use rvoip_core::adapter::{
    ConnectionAdapter, EndReason, OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::message::{ContentType, Message, MessageRecipients};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn adapter_smoke_all_methods() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config.clone());

    let caps = adapter.capabilities();
    assert!(!caps.audio_codecs.is_empty());

    let originate = adapter
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: caps.clone(),
            transport: None,
            context: Default::default(),
        })
        .await
        .expect("originate");

    let conn_id = originate.connection.id.clone();
    let local_sdp = adapter.local_sdp(&conn_id).expect("local sdp");
    assert!(!local_sdp.is_empty());

    let adapter2 = WebRtcAdapter::new(config);
    let inbound_id = adapter2
        .apply_remote_offer(&local_sdp)
        .await
        .expect("apply offer");
    let answer_sdp = adapter2.local_sdp(&inbound_id).expect("answer sdp");

    adapter
        .apply_remote_answer(conn_id.clone(), &answer_sdp)
        .await
        .expect("apply answer");

    adapter.accept(conn_id.clone()).await.expect("accept");

    let streams = tokio::time::timeout(Duration::from_secs(5), adapter.streams(conn_id.clone()))
        .await
        .expect("streams timeout")
        .expect("streams");
    assert!(!streams.is_empty());

    adapter.hold(conn_id.clone()).await.expect("hold");
    adapter.resume(conn_id.clone()).await.expect("resume");

    let msg = Message {
        id: rvoip_core::ids::MessageId::new(),
        conversation_id: rvoip_core::ids::ConversationId::new(),
        origin: rvoip_core::message::MessageOrigin::System,
        from_participant: ParticipantId::new(),
        to: MessageRecipients::All,
        direction: Direction::Outbound,
        content_type: ContentType::Text,
        body: bytes::Bytes::from_static(b"hello"),
        attachments: vec![],
        in_reply_to: None,
        timestamp: chrono::Utc::now(),
    };
    // Data channel may not be ready under webrtc-rs alpha — errors are acceptable.
    let _ = adapter.send_message(conn_id.clone(), msg).await;

    let dtmf = adapter.send_dtmf(conn_id.clone(), "1", 100).await;
    assert!(dtmf.is_ok(), "RFC 4733 DTMF expected to succeed: {dtmf:?}");

    let _ = adapter.renegotiate_media(conn_id.clone(), caps).await;

    let assurance = adapter
        .verify_request_signature(
            conn_id.clone(),
            SignatureHeaders {
                signature: String::new(),
                signature_input: String::new(),
                signature_key: None,
                signature_agent: None,
            },
        )
        .await
        .expect("verify signature");
    // D2 — adapter surfaces the negotiated peer's DTLS fingerprint as the
    // assurance. (Used to be Anonymous before rvoip-core gained the
    // DtlsFingerprint variant.)
    assert!(
        matches!(
            assurance,
            rvoip_core::identity::IdentityAssurance::DtlsFingerprint { .. }
        ),
        "expected DtlsFingerprint assurance, got {assurance:?}"
    );

    assert!(adapter
        .transfer(conn_id.clone(), TransferTarget::Uri("x".into()))
        .await
        .is_err());

    adapter
        .end(conn_id.clone(), EndReason::Normal)
        .await
        .expect("end");

    adapter2.reject(inbound_id, RejectReason::Busy).await.ok();
}

#[tokio::test]
async fn subscribe_events_receives_inbound_connection() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config);
    let mut events = adapter.subscribe_events();

    let adapter2 = WebRtcAdapter::new(WebRtcConfig::loopback());
    let handle = adapter2
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: adapter2.capabilities(),
            transport: None,
            context: Default::default(),
        })
        .await
        .expect("originate");
    let sdp = adapter2.local_sdp(&handle.connection.id).expect("sdp");

    let _inbound = adapter.apply_remote_offer(&sdp).await.expect("inbound");

    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("event timeout")
        .expect("event channel open");
}
