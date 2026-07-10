//! Adapter-level WebRTC DataMessage loopback coverage.

use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::{DataMessage, DataReliability, MessageId, ParticipantId, SessionId};
use rvoip_webrtc::data_message::{
    decode_data_message, encode_data_message, options_for_reliability, EncodedDataMessage,
    DATA_MESSAGE_SUBPROTOCOL,
};
use rvoip_webrtc::peer::{DataChannelOptions, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};
use tokio::sync::mpsc;
use webrtc::data_channel::{DataChannel, DataChannelEvent};

async fn connect_client_to_adapter(
    client: &Arc<RvoipPeerConnection>,
) -> (
    Arc<WebRtcAdapter>,
    rvoip_core::ConnectionId,
    mpsc::Receiver<AdapterEvent>,
) {
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let events = adapter.subscribe_events();
    let offer = client.create_offer_and_gather().await.expect("offer");
    let connection_id = adapter
        .apply_remote_offer(&offer)
        .await
        .expect("apply offer");
    let answer = adapter.local_sdp(&connection_id).expect("answer");
    client.set_remote_answer(&answer).await.expect("set answer");
    let (client_connected, adapter_connected) = tokio::join!(
        client.wait_connected(Duration::from_secs(10)),
        adapter.accept(connection_id.clone())
    );
    client_connected.expect("client connected");
    adapter_connected.expect("adapter connected");
    (adapter, connection_id, events)
}

async fn connect_two_adapters() -> (
    Arc<WebRtcAdapter>,
    rvoip_core::ConnectionId,
    Arc<WebRtcAdapter>,
    rvoip_core::ConnectionId,
    mpsc::Receiver<AdapterEvent>,
) {
    let offerer = WebRtcAdapter::new(WebRtcConfig::loopback());
    let answerer = WebRtcAdapter::new(WebRtcConfig::loopback());
    let mut offerer_events = offerer.subscribe_events();
    let answerer_events = answerer.subscribe_events();
    let handle = offerer
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: "loopback".into(),
            direction: Direction::Outbound,
            capabilities: offerer.capabilities(),
            transport: None,
        })
        .await
        .expect("originate");
    let offerer_id = handle.connection.id;
    let offer = offerer.local_sdp(&offerer_id).expect("offer");
    let answerer_id = answerer
        .apply_remote_offer(&offer)
        .await
        .expect("apply offer");
    let answer = answerer.local_sdp(&answerer_id).expect("answer");
    offerer
        .apply_remote_answer(offerer_id.clone(), &answer)
        .await
        .expect("apply answer");
    let (offerer_accept, answerer_accept) = tokio::join!(
        offerer.accept(offerer_id.clone()),
        answerer.accept(answerer_id.clone())
    );
    offerer_accept.expect("offerer accepted");
    answerer_accept.expect("answerer accepted");
    // Drain connection lifecycle events so this receiver cannot apply
    // backpressure while the answerer-side assertions run.
    while offerer_events.try_recv().is_ok() {}
    (offerer, offerer_id, answerer, answerer_id, answerer_events)
}

async fn next_data_message(events: &mut mpsc::Receiver<AdapterEvent>) -> DataMessage {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await.expect("adapter event channel closed") {
                AdapterEvent::DataMessage { message, .. } => return message,
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for DataMessage")
}

async fn send_encoded(channel: &Arc<dyn DataChannel>, message: &DataMessage) {
    match encode_data_message(message).expect("encode") {
        EncodedDataMessage::Text(frame) => channel.send_text(&frame).await.expect("send text"),
        EncodedDataMessage::Binary(frame) => channel.send(frame).await.expect("send binary"),
    }
}

async fn receive_decoded(
    channel: &Arc<dyn DataChannel>,
    label: &str,
    reliability: DataReliability,
) -> DataMessage {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(DataChannelEvent::OnMessage(frame)) = channel.poll().await {
                return decode_data_message(
                    label,
                    DATA_MESSAGE_SUBPROTOCOL,
                    reliability.clone(),
                    &frame.data,
                    frame.is_string,
                )
                .expect("decode");
            }
        }
    })
    .await
    .expect("timed out waiting for client DataMessage")
}

#[tokio::test]
async fn adapter_preserves_text_binary_labels_reliability_and_ids() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("client");
    let text_reliability = DataReliability::ReliableOrdered;
    let text_channel = client
        .create_data_channel(
            "bridgefu.context.v1",
            options_for_reliability(&text_reliability).expect("text options"),
        )
        .await
        .expect("text channel");
    let binary_reliability = DataReliability::MaxRetransmits {
        ordered: false,
        count: 0,
    };
    let binary_channel = client
        .create_data_channel(
            "telemetry.raw",
            options_for_reliability(&binary_reliability).expect("binary options"),
        )
        .await
        .expect("binary channel");

    let (adapter, connection_id, mut events) = connect_client_to_adapter(&client).await;
    RvoipPeerConnection::wait_data_channel_open(&text_channel, Duration::from_secs(10))
        .await
        .expect("text channel open");
    RvoipPeerConnection::wait_data_channel_open(&binary_channel, Duration::from_secs(10))
        .await
        .expect("binary channel open");

    let text = DataMessage {
        label: "bridgefu.context.v1".into(),
        content_type: "application/json".into(),
        bytes: Bytes::from_static(br#"{"correlation_id":"call-42"}"#),
        reliability: text_reliability,
        message_id: MessageId::from_string("msg_text_preserved"),
    };
    send_encoded(&text_channel, &text).await;
    assert_eq!(next_data_message(&mut events).await, text);

    let binary = DataMessage {
        label: "telemetry.raw".into(),
        content_type: "application/octet-stream".into(),
        bytes: Bytes::from_static(b"\0\xff\x10\x80"),
        reliability: binary_reliability.clone(),
        message_id: MessageId::from_string("msg_binary_preserved"),
    };
    adapter
        .send_data_message(connection_id.clone(), binary.clone())
        .await
        .expect("adapter binary send");
    assert_eq!(
        receive_decoded(&binary_channel, "telemetry.raw", binary_reliability).await,
        binary
    );

    adapter
        .end(connection_id, EndReason::Normal)
        .await
        .expect("end");
}

#[tokio::test]
async fn adapter_accepts_unframed_messages_from_every_seen_label() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("client");
    let first = client
        .create_data_channel("legacy-first", DataChannelOptions::reliable())
        .await
        .expect("first channel");
    let second = client
        .create_data_channel("legacy-second", DataChannelOptions::unreliable())
        .await
        .expect("second channel");
    let (adapter, connection_id, mut events) = connect_client_to_adapter(&client).await;
    RvoipPeerConnection::wait_data_channel_open(&first, Duration::from_secs(10))
        .await
        .expect("first open");
    RvoipPeerConnection::wait_data_channel_open(&second, Duration::from_secs(10))
        .await
        .expect("second open");

    first.send_text("unframed text").await.expect("text send");
    let received = next_data_message(&mut events).await;
    assert_eq!(received.label, "legacy-first");
    assert_eq!(received.content_type, "text/plain; charset=utf-8");
    assert_eq!(received.bytes, Bytes::from_static(b"unframed text"));

    second
        .send(BytesMut::from(&b"\0\xff"[..]))
        .await
        .expect("binary send");
    let received = next_data_message(&mut events).await;
    assert_eq!(received.label, "legacy-second");
    assert_eq!(received.content_type, "application/octet-stream");
    assert_eq!(received.bytes, Bytes::from_static(b"\0\xff"));
    assert_eq!(
        received.reliability,
        DataReliability::MaxRetransmits {
            ordered: false,
            count: 0,
        }
    );

    adapter
        .end(connection_id, EndReason::Normal)
        .await
        .expect("end");
}

#[tokio::test]
async fn closed_cached_channel_is_replaced_by_exact_live_duplicate() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("client");
    let reliability = DataReliability::ReliableOrdered;
    let first = client
        .create_data_channel(
            "replaceable",
            options_for_reliability(&reliability).expect("options"),
        )
        .await
        .expect("first");
    let second = client
        .create_data_channel(
            "replaceable",
            options_for_reliability(&reliability).expect("options"),
        )
        .await
        .expect("second");
    let (adapter, connection_id, _events) = connect_client_to_adapter(&client).await;
    RvoipPeerConnection::wait_data_channel_open(&first, Duration::from_secs(10))
        .await
        .expect("first open");
    RvoipPeerConnection::wait_data_channel_open(&second, Duration::from_secs(10))
        .await
        .expect("second open");

    let cached_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let cached = adapter.routes().get(&connection_id).and_then(|route| {
                route
                    .data_channel
                    .iter()
                    .next()
                    .map(|entry| entry.value().id())
            });
            if let Some(id) = cached {
                return id;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("initial cache");
    let (closed, survivor) = if first.id() == cached_id {
        (&first, &second)
    } else {
        (&second, &first)
    };
    closed.close().await.expect("close cached channel");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let replacement = adapter.routes().get(&connection_id).and_then(|route| {
                route
                    .data_channel
                    .iter()
                    .next()
                    .map(|entry| entry.value().id())
            });
            if replacement == Some(survivor.id()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("live duplicate replaces exact closed channel");

    let message = DataMessage {
        label: "replaceable".into(),
        content_type: "text/plain".into(),
        bytes: Bytes::from_static(b"after replacement"),
        reliability: reliability.clone(),
        message_id: MessageId::from_string("msg_after_replacement"),
    };
    adapter
        .send_data_message(connection_id.clone(), message.clone())
        .await
        .expect("send through replacement");
    assert_eq!(
        receive_decoded(survivor, "replaceable", reliability).await,
        message
    );

    adapter
        .end(connection_id, EndReason::Normal)
        .await
        .expect("end");
}

#[tokio::test]
async fn post_connect_channels_open_for_arbitrary_labels_and_reliability() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (offerer, offerer_id, answerer, answerer_id, mut answerer_events) =
        connect_two_adapters().await;

    let messages = [
        DataMessage {
            label: "dynamic.context".into(),
            content_type: "application/json".into(),
            bytes: Bytes::from_static(br#"{"call":"abc"}"#),
            reliability: DataReliability::ReliableUnordered,
            message_id: MessageId::from_string("msg_dynamic_text"),
        },
        DataMessage {
            label: "dynamic.binary".into(),
            content_type: "application/octet-stream".into(),
            bytes: Bytes::from_static(b"\0\x01\xff"),
            reliability: DataReliability::MaxLifetime {
                ordered: false,
                milliseconds: 400,
            },
            message_id: MessageId::from_string("msg_dynamic_binary"),
        },
    ];

    for message in messages {
        offerer
            .send_data_message(offerer_id.clone(), message.clone())
            .await
            .expect("post-connect DataChannel send");
        assert_eq!(next_data_message(&mut answerer_events).await, message);
    }

    offerer
        .end(offerer_id, EndReason::Normal)
        .await
        .expect("end offerer");
    answerer
        .end(answerer_id, EndReason::Normal)
        .await
        .expect("end answerer");
}
