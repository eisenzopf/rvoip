//! Gap plan §4.2 — mid-call codec renegotiation.
//!
//! Two layers exercised here:
//!
//! 1. **Wire-layer (existing cases)**: the coordinator's
//!    `connection.update` handler with `action="renegotiate-media"`
//!    runs §8.1 negotiation against the local descriptor and replies
//!    with either a `connection.update` (chosen codec) or
//!    `error 488 capability/incompatible-capabilities`.
//!
//! 2. **Driver-layer (added in §4.2B v1 punch list)**: the shared
//!    `rvoip_uctp::adapter_helpers::renegotiate_via_envelope` helper
//!    that QUIC/WT/WS adapters call from `renegotiate_media`. It
//!    sends the envelope, awaits the correlated reply via `Pending`,
//!    parses the chosen codec or maps `error 488` to
//!    `RvoipError::AdmissionRejected`.

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{
        connection::ConnectionOffer, connection::ConnectionUpdate, connection::StreamOffer,
    },
    state::{UctpCoordinator, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use tokio::sync::mpsc;

mod common;
use common::drive_auth_handshake;

async fn setup_connection(
    in_tx: &mpsc::Sender<UctpEnvelope>,
    out_rx: &mut mpsc::Receiver<UctpEnvelope>,
    sid: &str,
    connid: &str,
    offered_codecs: Vec<String>,
) {
    in_tx
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::SessionInvite,
            id: format!("env_invite_{sid}"),
            ts: Utc::now(),
            cid: Some(format!("conv_{sid}")),
            sid: Some(sid.into()),
            connid: None,
            in_reply_to: None,
            payload: serde_json::json!({
                "from": "part_test",
                "to": ["part_remote"],
                "medium": "voice",
                "intent": "synchronous-engagement",
                "capabilities_offer": {}
            }),
            signature: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let offer = UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionOffer,
        id: format!("env_offer_{}", connid),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", sid)),
        sid: Some(sid.into()),
        connid: Some(connid.into()),
        in_reply_to: None,
        payload: serde_json::to_value(ConnectionOffer {
            by_participant: "part_test".into(),
            substrate: "quic".into(),
            capabilities: serde_json::Value::Object(Default::default()),
            streams_offered: vec![StreamOffer {
                id: "strm_audio".into(),
                kind: "audio".into(),
                direction: "send-recv".into(),
                codec_preferences: offered_codecs,
            }],
            substrate_setup: serde_json::Value::Null,
        })
        .unwrap(),
        signature: None,
    };
    in_tx.send(offer).await.unwrap();
    // Drain any synchronous responses the coordinator emits (in current
    // v0 the offer handler doesn't emit a wire reply, but be tolerant).
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), out_rx.recv()).await;
}

fn renegotiate_env(sid: &str, connid: &str, id: &str, new_prefs: Vec<String>) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionUpdate,
        id: id.into(),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", sid)),
        sid: Some(sid.into()),
        connid: Some(connid.into()),
        in_reply_to: None,
        payload: serde_json::to_value(ConnectionUpdate {
            action: "renegotiate-media".into(),
            streams: vec!["strm_audio".into()],
            codec_preferences: new_prefs,
            details: serde_json::Value::Null,
        })
        .unwrap(),
        signature: None,
    }
}

#[tokio::test]
async fn renegotiate_picks_first_overlapping_codec_and_replies_with_choice() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());
    drive_auth_handshake(&in_tx, &mut out_rx).await;

    setup_connection(
        &in_tx,
        &mut out_rx,
        "sess_re",
        "conn_re",
        vec!["opus".into()],
    )
    .await;

    // Renegotiate: peer now prefers PCMU first, opus fallback. The
    // local descriptor (default_v0_descriptor) supports both, so PCMU
    // wins (first in the preference list).
    in_tx
        .send(renegotiate_env(
            "sess_re",
            "conn_re",
            "env_re_1",
            vec!["g.711-mu".into(), "opus".into()],
        ))
        .await
        .unwrap();

    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("renegotiate reply timeout")
        .expect("out_rx closed");
    assert_eq!(reply.msg_type, MessageType::ConnectionUpdate);
    assert_eq!(reply.in_reply_to.as_deref(), Some("env_re_1"));

    let payload: ConnectionUpdate = reply.decode_payload().unwrap();
    assert_eq!(payload.action, "renegotiate-media");
    assert_eq!(payload.codec_preferences, vec!["g.711-mu".to_string()]);
}

#[tokio::test]
async fn renegotiate_with_no_overlap_replies_488() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());
    drive_auth_handshake(&in_tx, &mut out_rx).await;

    setup_connection(
        &in_tx,
        &mut out_rx,
        "sess_re2",
        "conn_re2",
        vec!["opus".into()],
    )
    .await;

    // Renegotiate to a codec the local descriptor doesn't support.
    in_tx
        .send(renegotiate_env(
            "sess_re2",
            "conn_re2",
            "env_re_2",
            vec!["g.722.1".into(), "speex".into()],
        ))
        .await
        .unwrap();

    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("error reply timeout")
        .expect("out_rx closed");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 488);
    assert_eq!(payload.reason, "incompatible-capabilities");
}

#[tokio::test]
async fn renegotiate_unknown_action_emits_ack_for_forward_compat() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());
    drive_auth_handshake(&in_tx, &mut out_rx).await;

    setup_connection(
        &in_tx,
        &mut out_rx,
        "sess_re3",
        "conn_re3",
        vec!["opus".into()],
    )
    .await;

    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionUpdate,
        id: "env_re_unknown".into(),
        ts: Utc::now(),
        cid: Some("conv_sess_re3".into()),
        sid: Some("sess_re3".into()),
        connid: Some("conn_re3".into()),
        in_reply_to: None,
        payload: serde_json::to_value(ConnectionUpdate {
            action: "tickle".into(),
            streams: Vec::new(),
            codec_preferences: Vec::new(),
            details: serde_json::Value::Null,
        })
        .unwrap(),
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("ack timeout")
        .expect("out_rx closed");
    assert_eq!(reply.msg_type, MessageType::Ack);
    assert_eq!(reply.in_reply_to.as_deref(), Some("env_re_unknown"));
}

// =====================================================================
// Driver-layer cases — gap plan §4.2B v1 punch list.
// =====================================================================
//
// These cases exercise `renegotiate_via_envelope` directly instead of
// going through the full QUIC/WT/WS adapter. We mock the substrate
// side by reading the request envelope off `out_rx` and feeding a
// synthetic reply back via `pending.deliver`. That's the same path
// `UctpCoordinator::dispatch_inner` takes after the v1 punch list's
// §4.2A deliver gate; the helper doesn't care where the reply comes
// from as long as `in_reply_to` matches.

use rvoip_core::capability::{CapabilityDescriptor, CodecInfo};
use rvoip_core::ids::ConnectionId;
use rvoip_uctp::adapter_helpers::renegotiate_via_envelope;
use rvoip_uctp::payloads::control::Error as ErrorPayload;
use rvoip_uctp::substrate::Pending;
use std::sync::Arc;
use std::time::Duration;

fn opus_pcmu_caps() -> CapabilityDescriptor {
    CapabilityDescriptor {
        audio_codecs: vec![
            CodecInfo {
                name: "opus".into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            },
            CodecInfo {
                name: "g.711-mu".into(),
                clock_rate_hz: 8_000,
                channels: 1,
                fmtp: None,
            },
        ],
        ..Default::default()
    }
}

#[tokio::test]
async fn driver_renegotiate_returns_chosen_codec_when_peer_replies() {
    let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(8);
    let pending = Arc::new(Pending::new());
    let conn = ConnectionId::new();
    let caps = opus_pcmu_caps();

    // Spawn a synthetic peer that reads the request, builds a reply
    // with `codec_preferences = ["g.711-mu"]`, and delivers it.
    let pending_for_peer = Arc::clone(&pending);
    let peer = tokio::spawn(async move {
        let req = out_rx.recv().await.expect("request");
        assert_eq!(req.msg_type, MessageType::ConnectionUpdate);
        let reply = UctpEnvelope {
            v: 1,
            msg_type: MessageType::ConnectionUpdate,
            id: format!("env_reply_{}", uuid::Uuid::new_v4().simple()),
            ts: Utc::now(),
            cid: None,
            sid: req.sid.clone(),
            connid: req.connid.clone(),
            in_reply_to: Some(req.id.clone()),
            payload: serde_json::to_value(ConnectionUpdate {
                action: "renegotiate-media".into(),
                streams: vec!["strm_audio".into()],
                codec_preferences: vec!["g.711-mu".into()],
                details: serde_json::Value::Null,
            })
            .unwrap(),
            signature: None,
        };
        assert!(pending_for_peer.deliver(reply).is_ok());
    });

    let result = renegotiate_via_envelope(
        &out_tx,
        &pending,
        "sess_drv",
        &conn,
        &caps,
        Duration::from_secs(2),
    )
    .await
    .expect("renegotiate ok");
    peer.await.unwrap();

    let chosen = result.audio.expect("audio codec");
    assert_eq!(chosen.name, "g.711-mu");
    assert_eq!(chosen.clock_rate_hz, 8_000);
}

#[tokio::test]
async fn driver_renegotiate_maps_488_to_admission_rejected() {
    use rvoip_core::error::RvoipError;

    let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(8);
    let pending = Arc::new(Pending::new());
    let conn = ConnectionId::new();
    let caps = opus_pcmu_caps();

    let pending_for_peer = Arc::clone(&pending);
    let peer = tokio::spawn(async move {
        let req = out_rx.recv().await.expect("request");
        let reply = UctpEnvelope {
            v: 1,
            msg_type: MessageType::Error,
            id: format!("env_err_{}", uuid::Uuid::new_v4().simple()),
            ts: Utc::now(),
            cid: None,
            sid: req.sid.clone(),
            connid: req.connid.clone(),
            in_reply_to: Some(req.id.clone()),
            payload: serde_json::to_value(ErrorPayload {
                code: 488,
                category: "capability".into(),
                reason: "incompatible-capabilities".into(),
                details: serde_json::Value::Null,
            })
            .unwrap(),
            signature: None,
        };
        assert!(pending_for_peer.deliver(reply).is_ok());
    });

    let err = renegotiate_via_envelope(
        &out_tx,
        &pending,
        "sess_drv",
        &conn,
        &caps,
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    peer.await.unwrap();

    assert!(
        matches!(err, RvoipError::AdmissionRejected(_)),
        "expected AdmissionRejected; got {err:?}"
    );
}

#[tokio::test]
async fn driver_renegotiate_times_out_when_peer_never_replies() {
    use rvoip_core::error::RvoipError;

    let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(8);
    let pending = Arc::new(Pending::new());
    let conn = ConnectionId::new();
    let caps = opus_pcmu_caps();

    // Read but never reply.
    let _peer = tokio::spawn(async move {
        let _ = out_rx.recv().await;
    });

    let err = renegotiate_via_envelope(
        &out_tx,
        &pending,
        "sess_drv",
        &conn,
        &caps,
        Duration::from_millis(150),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, RvoipError::Adapter(_)),
        "expected Adapter error on timeout; got {err:?}"
    );
}
