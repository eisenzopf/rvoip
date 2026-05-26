//! WS↔WS end-to-end bridge proof (Phase G3).
//!
//! Verifies the gap-completion claim that with `rvoip-webrtc` in place,
//! WebSocket connections under a single Orchestrator can be bridged
//! end-to-end with real audio flowing across the cross-transport pump.
//!
//! Architecture:
//!
//! 1. **One** `UctpWsAdapter` (server) registered on an `Orchestrator`.
//! 2. **Two** `UctpWsClient`s dial in over plain ws:// — they drive the
//!    UCTP bearer-auth handshake + send `session.invite`. The server's
//!    coordinator emits an `InboundConnection` event per client.
//! 3. Under `media-webrtc`, the server's InboundInvite handler kicks off
//!    a per-Connection `WebRtcMediaBridge::new_answerer()` construction
//!    task. The test waits on `adapter.wait_bridge_for(...)` to obtain
//!    each answerer handle.
//! 4. The test creates two offerer `WebRtcMediaBridge`s client-side, then
//!    drives the substrate_setup SDP exchange directly against each
//!    answerer (offerer.local → answerer.set_remote → answerer.local →
//!    offerer.set_remote). This is the in-test analog of the production
//!    `connection.offer` / `connection.answer` envelope round-trip; v0.x
//!    will move the SDP exchange into envelope interception inside the
//!    WS server. For v0 the direct-access path proves the audio plane
//!    works end-to-end.
//! 5. After both peer connections reach `wait_connected`, the server's
//!    ready-watcher (spawned by `spawn_bridge_setup`) inserts each
//!    answerer's `WebRtcMediaStream` into the `Route.streams` map, which
//!    makes `adapter.streams(conn_id)` return a non-empty Vec.
//! 6. `orchestrator.bridge_connections(a, b)` resolves both audio streams
//!    and spawns the cross-transport frame pump.
//! 7. Frames injected at `offerer_1.media_stream().frames_out()` flow
//!    over SRTP → answerer_1 → pump → answerer_2 → SRTP → and arrive at
//!    `offerer_2.media_stream().frames_in()`. The test asserts 10 frames
//!    are delivered in order with payload bytes preserved (Opus PT 111
//!    passthrough; no transcoding).

#![cfg(feature = "media-webrtc")]

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::events::Event;
use rvoip_core::ids::ConnectionId;
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_core::{Config, Orchestrator};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::types::MessageType;
use rvoip_webrtc::media::pump::opus_rtp_payload;
use rvoip_webrtc::peer::RvoipPeerConnection;
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig, WebRtcMediaBridge};
use tokio::net::TcpListener;
use url::Url;

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn auth_hello() -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_hello_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_ws_bridge".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "ws-bridge/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    signature: None,
    }
}

fn auth_response(in_reply_to: String) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: format!("env_resp_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(in_reply_to),
        payload: serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
        actor_token: None,
        })
        .unwrap(),
    signature: None,
    }
}

fn invite(sid: &str, participant: &str) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_inv_{}", sid),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", sid)),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(SessionInvite {
            from: participant.into(),
            to: vec!["part_ws_bridge".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    signature: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_to_ws_bridge_flows_frames_end_to_end() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // --- Server ---
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");
    let adapter = UctpWsAdapter::new(UctpWsConfig::new(listener, bearer_stub()))
        .await
        .expect("adapter");

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register ws");
    let mut events = orchestrator.subscribe_events();

    // --- Two WS clients dial in over plain ws:// ---
    let url = Url::parse(&format!("ws://{}", server_addr)).expect("parse url");
    let client_a = UctpWsClient::connect(&url).await.expect("client A connect");
    let client_b = UctpWsClient::connect(&url).await.expect("client B connect");
    let mut in_a = client_a.take_inbound().expect("client A take_inbound");
    let mut in_b = client_b.take_inbound().expect("client B take_inbound");

    // --- Bearer auth on both clients ---
    client_a.send(auth_hello()).await.expect("A hello");
    client_b.send(auth_hello()).await.expect("B hello");

    let ca = tokio::time::timeout(Duration::from_secs(5), in_a.recv())
        .await
        .expect("A challenge timeout")
        .expect("A inbound closed");
    let cb = tokio::time::timeout(Duration::from_secs(5), in_b.recv())
        .await
        .expect("B challenge timeout")
        .expect("B inbound closed");
    assert_eq!(ca.msg_type, MessageType::AuthChallenge);
    assert_eq!(cb.msg_type, MessageType::AuthChallenge);

    client_a
        .send(auth_response(ca.id))
        .await
        .expect("A response");
    client_b
        .send(auth_response(cb.id))
        .await
        .expect("B response");

    let sa = tokio::time::timeout(Duration::from_secs(5), in_a.recv())
        .await
        .expect("A session timeout")
        .expect("A inbound closed");
    let sb = tokio::time::timeout(Duration::from_secs(5), in_b.recv())
        .await
        .expect("B session timeout")
        .expect("B inbound closed");
    assert_eq!(sa.msg_type, MessageType::AuthSession);
    assert_eq!(sb.msg_type, MessageType::AuthSession);

    // --- session.invite from each client ---
    client_a
        .send(invite("sess_ws_a", "part_ws_a"))
        .await
        .expect("A invite");
    client_b
        .send(invite("sess_ws_b", "part_ws_b"))
        .await
        .expect("B invite");

    // --- Collect two ConnectionInbound events ---
    let mut conn_ids: Vec<ConnectionId> = Vec::new();
    for _ in 0..80 {
        if conn_ids.len() == 2 {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { connection_id, .. })) => {
                conn_ids.push(connection_id);
            }
            _ => continue,
        }
    }
    assert_eq!(conn_ids.len(), 2, "expected two ConnectionInbound events");

    // --- Construct two offerer bridges (client side, test-owned) ---
    let offerer_1 = Arc::new(
        WebRtcMediaBridge::new_offerer()
            .await
            .expect("offerer 1 construction"),
    );
    let offerer_2 = Arc::new(
        WebRtcMediaBridge::new_offerer()
            .await
            .expect("offerer 2 construction"),
    );

    // --- Wait for the server's answerer bridges to materialize ---
    let answerer_1 = adapter
        .wait_bridge_for(&conn_ids[0], Duration::from_secs(10))
        .await
        .expect("answerer 1 ready");
    let answerer_2 = adapter
        .wait_bridge_for(&conn_ids[1], Duration::from_secs(10))
        .await
        .expect("answerer 2 ready");

    // --- Drive SDP exchange directly (offerer ↔ answerer × 2) ---
    for (offerer, answerer) in [(&offerer_1, &answerer_1), (&offerer_2, &answerer_2)] {
        let offer = offerer
            .local_substrate_setup()
            .await
            .expect("offerer local SDP");
        answerer
            .set_remote_substrate_setup(offer)
            .await
            .expect("answerer applies offer");
        let answer = answerer
            .local_substrate_setup()
            .await
            .expect("answerer local SDP");
        offerer
            .set_remote_substrate_setup(answer)
            .await
            .expect("offerer applies answer");
    }

    // --- Wait for ICE/DTLS on all four endpoints ---
    let connect_timeout = Duration::from_secs(15);
    let (o1, o2, a1, a2) = tokio::join!(
        offerer_1.wait_connected(connect_timeout),
        offerer_2.wait_connected(connect_timeout),
        answerer_1.wait_connected(connect_timeout),
        answerer_2.wait_connected(connect_timeout),
    );
    o1.expect("offerer 1 connected");
    o2.expect("offerer 2 connected");
    a1.expect("answerer 1 connected");
    a2.expect("answerer 2 connected");
    eprintln!("✓ all 4 peers DTLS-connected");

    // --- Prime the forward-direction tracks so answerer_1 + offerer_2 see
    // their `on_track` events. webrtc-rs fires `on_track` only after the
    // receiver sees its first inbound RTP packet; `WebRtcMediaBridge::wait_connected`
    // does a brief `attach_remote_if_ready`, but no RTP has flowed yet at
    // that point so the track isn't attached.
    //
    // For the A→B direction we need:
    //   1. answerer_1.attach_remote(track from offerer_1) — primed below.
    //   2. offerer_2.attach_remote(track from answerer_2) — primed AFTER
    //      bridge_connections, because answerer_2 only sends RTP once the
    //      bridge pump writes to its frames_out_tx. (Priming offerer_2
    //      directly from answerer_2 would work, but the WebRTC peer
    //      connection only has one inbound audio track and the bridge
    //      pump's pushed frames would race the prime helper for it.)
    //
    // The B→A direction (offerer_2 → answerer_2 → bridge → answerer_1 →
    // offerer_1) is not exercised by this test; we leave those tracks
    // unattached. ---
    let prime_timeout = Duration::from_secs(10);
    let a1_remote = RvoipPeerConnection::prime_remote_track(
        offerer_1.peer(),
        answerer_1.peer(),
        prime_timeout,
    )
    .await
    .expect("answerer 1 receives offerer 1 track");

    eprintln!("✓ primed offerer_1 → answerer_1; attaching remote");
    answerer_1
        .media_stream()
        .expect("answerer 1 stream")
        .attach_remote(a1_remote);

    // --- Wait for the server's ready-watcher to push streams into routes.
    //     `bridge_connections` polls internally too, but we explicitly verify
    //     so a failure here surfaces as a stream-registration issue rather
    //     than as an opaque "no audio stream within deadline" error. ---
    for conn_id in &conn_ids {
        for attempt in 0..100u32 {
            if !adapter
                .streams(conn_id.clone())
                .await
                .expect("streams query")
                .is_empty()
            {
                break;
            }
            assert!(
                attempt < 99,
                "stream never appeared in Route.streams for {:?}",
                conn_id
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // --- Bridge ---
    eprintln!("✓ ready-watcher pushed streams to both routes; calling bridge_connections");
    let _bridge_id = orchestrator
        .bridge_connections(conn_ids[0].clone(), conn_ids[1].clone())
        .await
        .expect("bridge succeeds — both sides have streams");
    eprintln!("✓ orchestrator bridge created");

    // --- After bridge_connections, the orchestrator's frame pump starts
    // consuming answerer_1.frames_in() and forwarding to answerer_2.frames_out().
    // The priming silent-RTP packets that landed in answerer_1.frames_in
    // (during the earlier prime_remote_track call) now flow through the
    // bridge: answerer_2 sends them via WebRTC to offerer_2. Once offerer_2
    // sees the first inbound RTP packet, its on_track event fires; we then
    // attach the remote to offerer_2's MediaStream so its inbound pump
    // begins emitting frames into frames_in_rx.
    //
    // Poll up to 5s for the on_track event. ---
    let o2_remote = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut found = None;
        while tokio::time::Instant::now() < deadline {
            if let Some(t) = offerer_2.peer().try_recv_remote_track().await {
                found = Some(t);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        found.expect("offerer_2 never observed remote track from answerer_2")
    };
    eprintln!("✓ offerer_2 observed remote track from answerer_2; attaching");
    offerer_2
        .media_stream()
        .expect("offerer 2 stream")
        .attach_remote(o2_remote);

    // --- Inject 10 frames at offerer_1; observe on offerer_2 ---
    let stream_1 = offerer_1.media_stream().expect("offerer 1 media stream");
    let stream_2 = offerer_2.media_stream().expect("offerer 2 media stream");

    let out_1 = rvoip_core::stream::MediaStream::frames_out(stream_1.as_ref());
    let mut in_2 = rvoip_core::stream::MediaStream::frames_in(stream_2.as_ref());

    // Drain any priming-related frames already queued in offerer_2.frames_in
    // (silent RTP from the prime_remote_track step that flowed through the
    // bridge). We wait up to 500ms of quiescence to be confident the drain
    // is complete before injecting our test markers.
    let mut drained = 0;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), in_2.recv()).await {
            Ok(Some(_)) => {
                drained += 1;
                continue;
            }
            _ => break,
        }
    }
    eprintln!("✓ drained {} priming frames; injecting test markers", drained);

    // Inject 10 frames as **full RTP wire bytes** (legacy path through the
    // outbound pump), with sequence numbers well above what
    // `prime_remote_track` used so webrtc-rs's RTP receiver doesn't drop
    // them as duplicates. The pump's `bytes_to_rtp_packet` will parse
    // these and forward as-is.
    //
    // On the answerer_1 side, the inbound pump strips the RTP header and
    // emits `MediaFrame { payload: pkt.payload, ... 
    // emits `MediaFrame { payload: pkt.payload, ... payload_type: None,
    // emits `MediaFrame { payload: pkt.payload, ... }` — i.e. just the
    // codec payload (our 5-byte marker). The bridge forwards that to
    // answerer_2's outbound pump, which wraps it in a fresh RTP packet
    // (V=2, the marker still fails bytes_to_rtp because of the leading
    // 0xFA = V=3, so the wrap path is used; that's fine because the
    // marker survives as the inner payload). offerer_2's inbound pump
    // strips the RTP header again, exposing the marker bytes for the test.
    let ssrc = offerer_1
        .peer()
        .local_audio_ssrc()
        .expect("offerer_1 local audio ssrc");
    for i in 0u8..10 {
        let marker = bytes::Bytes::from(vec![0xFA, 0xCE, 0xFE, 0xED, i]);
        let rtp_bytes = opus_rtp_payload(
            ssrc,
            /* seq */ 100 + i as u16,
            /* timestamp */ (100 + i as u32) * 960,
            /* marker */ false,
            marker,
        );
        let frame = MediaFrame {
            stream_id: stream_1.id(),
            kind: StreamKind::Audio,
            payload: rtp_bytes,
            timestamp_rtp: 0,
            captured_at: Utc::now(),
        payload_type: None,
        };
        out_1.send(frame).await.expect("inject");
    }

    // Collect 10 marker frames, filtering out any stray priming packets
    // that race past our drain window (unlikely but defensive).
    let mut received: Vec<Vec<u8>> = Vec::new();
    while received.len() < 10 {
        let frame = tokio::time::timeout(Duration::from_secs(15), in_2.recv())
            .await
            .expect("timed out waiting for bridged frame on offerer_2")
            .expect("offerer_2 stream closed");
        let payload = frame.payload.to_vec();
        // Skip stray priming packets — markers start with 0xFA 0xCE 0xFE 0xED.
        if payload.len() == 5 && payload[..4] == [0xFA, 0xCE, 0xFE, 0xED] {
            received.push(payload);
        }
    }

    for (i, payload) in received.iter().enumerate() {
        assert_eq!(
            payload,
            &vec![0xFA, 0xCE, 0xFE, 0xED, i as u8],
            "WS→WS frame {} corrupted or out of order: {:?}",
            i,
            payload
        );
    }
}
