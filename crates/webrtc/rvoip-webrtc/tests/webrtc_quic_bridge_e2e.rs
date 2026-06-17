//! Phase 11 — WHIP WebRTC leg bridged to real `rvoip-quic` via the orchestrator.

#![cfg(feature = "bridge-quic")]

#[path = "support/quic_leg.rs"]
mod quic_leg;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use quic_leg::{install_crypto_provider, QuicLegHarness};
use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::capability::CodecInfo;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::Direction;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, StreamId, TenantId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_quic::{spawn_datagram_reader, QuicDatagramMediaStream};
use rvoip_webrtc::media::silent_rtp_payload_for_ssrc;
use rvoip_webrtc::peer::RvoipPeerConnection;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig, WebRtcServerBuilder};

async fn wait_quic_inbound(events: &mut tokio::sync::broadcast::Receiver<Event>) -> ConnectionId {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("quic inbound timeout")
            .expect("event bus");
        if let Event::ConnectionInbound { connection_id, .. } = event {
            return connection_id;
        }
    }
}

async fn wait_webrtc_inbound(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    server: &rvoip_webrtc::WebRtcServer,
) -> ConnectionId {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("webrtc inbound timeout")
            .expect("event bus");
        if let Event::ConnectionInbound { connection_id, .. } = event {
            if server.adapter().routes().contains_key(&connection_id) {
                return connection_id;
            }
        }
    }
}

#[tokio::test]
async fn whip_webrtc_bridged_to_real_quic_leg() {
    install_crypto_provider();

    let quic = QuicLegHarness::start("127.0.0.1:0".parse().unwrap()).await;

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_whip("127.0.0.1:0")
        .build()
        .await
        .expect("webrtc server");

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(server.adapter() as Arc<dyn ConnectionAdapter>)
        .expect("register webrtc");
    orchestrator
        .register(quic.adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register quic");

    let mut events = orchestrator.subscribe_events();

    let conversation_id = orchestrator
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            std::collections::HashMap::new(),
        )
        .await
        .expect("open conversation");
    let session_id = orchestrator
        .start_session(conversation_id, SessionMedium::Voice, vec![])
        .await
        .expect("start session");

    // Real UCTP/QUIC inbound leg (auth + session.invite).
    let quic_client = quic.dial_invite(&session_id.to_string(), "quic_peer").await;
    let quic_conn = wait_quic_inbound(&mut events).await;

    // WHIP publisher (foreign WebRTC client).
    let publisher = WebRtcAdapter::new(WebRtcConfig::loopback());
    let pub_handle = publisher
        .originate(OriginateRequest {
            session_id: session_id.clone(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: publisher.capabilities(),
            transport: None,
        })
        .await
        .expect("originate");
    let pub_conn = pub_handle.connection.id.clone();
    let offer_sdp = publisher.local_sdp(&pub_conn).expect("offer sdp");

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client");
    let whip_base = format!("http://{}", server.whip_addr().expect("whip addr"));
    let resp = http
        .post(format!("{whip_base}/whip/live"))
        .header("content-type", "application/sdp")
        .body(offer_sdp)
        .send()
        .await
        .expect("whip post");
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    let answer_sdp = resp.text().await.expect("answer sdp");
    publisher
        .apply_remote_answer(pub_conn.clone(), &answer_sdp)
        .await
        .expect("apply answer");

    let webrtc_conn = wait_webrtc_inbound(&mut events, &server).await;

    orchestrator
        .route_inbound_connection(
            webrtc_conn.clone(),
            InboundAction::Accept {
                session_id: session_id.clone(),
                participant_id: ParticipantId::new(),
            },
        )
        .await
        .expect("accept webrtc");

    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("connected timeout")
            .expect("event bus");
        if let Event::ConnectionConnected { connection_id, .. } = &event {
            if *connection_id == webrtc_conn {
                break;
            }
        }
    }

    let _bridge_id = orchestrator
        .bridge_connections(webrtc_conn.clone(), quic_conn.clone())
        .await
        .expect("bridge webrtc ↔ quic");

    let bridged = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("bridged event timeout")
        .expect("event bus");
    assert!(
        matches!(bridged, Event::ConnectionsBridged { .. }),
        "expected ConnectionsBridged, got {bridged:?}"
    );

    // Client-side QUIC datagram stream (stream_local_id = 1 matches adapter default).
    let codec = CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 1,
        fmtp: None,
    };
    let client_stream = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec,
        Direction::Inbound,
        1,
        quic_client.connection.clone(),
    );
    let router = Arc::new(parking_lot::RwLock::new(vec![Arc::clone(&client_stream)]));
    spawn_datagram_reader(quic_client.connection.clone(), router, None);
    let mut client_in = client_stream.frames_in();

    publisher
        .accept(pub_conn.clone())
        .await
        .expect("publisher accept");

    // webrtc-rs fires on_track only after the first inbound RTP packet.
    let offerer_peer = publisher
        .routes()
        .get(&pub_conn)
        .expect("pub route")
        .peer
        .clone();
    let answerer_peer = server
        .adapter()
        .routes()
        .get(&webrtc_conn)
        .expect("server route")
        .peer
        .clone();
    RvoipPeerConnection::prime_remote_track(&offerer_peer, &answerer_peer, Duration::from_secs(10))
        .await
        .expect("remote track on WHIP answerer");

    let pub_streams = publisher
        .streams(pub_conn.clone())
        .await
        .expect("pub streams");
    let pub_stream = pub_streams.first().expect("publisher stream");
    let pub_ssrc = offerer_peer.local_audio_ssrc().expect("publisher ssrc");

    for seq in 1..=3u16 {
        let payload = silent_rtp_payload_for_ssrc(pub_ssrc, seq, seq as u32 * 960);
        pub_stream
            .frames_out()
            .send(MediaFrame {
                stream_id: pub_stream.id(),
                kind: StreamKind::Audio,
                payload,
                timestamp_rtp: seq as u32 * 960,
                captured_at: Utc::now(),
                payload_type: None,
            })
            .await
            .expect("publisher send");
    }

    let frame = tokio::time::timeout(Duration::from_secs(5), client_in.recv())
        .await
        .expect("quic client recv timeout")
        .expect("quic client stream closed");
    assert!(
        !frame.payload.is_empty(),
        "expected bridged media on QUIC client"
    );

    server.shutdown().await;
}
