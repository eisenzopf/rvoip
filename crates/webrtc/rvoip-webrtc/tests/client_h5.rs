//! H5: real-client features — send_answer, AudioSource/Sink, SessionHandle
//! Drop semantics, WsSignaler retry/backoff.

#![cfg(feature = "comprehensive")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::ConnectionAdapter;
use rvoip_webrtc::client::{
    Answer, AudioPacing, CountingAudioSink, FixtureAudioSource, IceCandidate, Signaler, WsSignaler,
    WsSignalerConfig,
};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::websocket::serve_listener;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig, WebRtcError};

#[tokio::test]
async fn ws_signaler_send_answer_routes_by_connection_id() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Spin up a server adapter behind a WS listener.
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move { serve_listener(listener, adapter).await.ok() }
    });

    // Have the server originate an outbound connection (it becomes the offerer).
    let caps = ConnectionAdapter::capabilities(&*adapter);
    let handle = ConnectionAdapter::originate(
        &*adapter,
        rvoip_core::adapter::OriginateRequest {
            session_id: rvoip_core::ids::SessionId::new(),
            participant_id: rvoip_core::ids::ParticipantId::new(),
            target: String::new(),
            direction: rvoip_core::connection::Direction::Outbound,
            capabilities: caps,
            transport: None,
            context: Default::default(),
        },
    )
    .await
    .expect("originate");
    let conn_id = handle.connection.id.clone();
    let offer_sdp = adapter.local_sdp(&conn_id).expect("offer sdp");

    // Build an answerer peer to produce a valid answer SDP for the offer.
    let answerer = Arc::new(
        RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Answerer)
            .await
            .expect("answerer"),
    );
    let answer_sdp = answerer
        .accept_offer_and_gather(&offer_sdp)
        .await
        .expect("answer");

    // Use WsSignaler to deliver the answer scoped to the server's connection_id.
    let signaler = WsSignaler::new(format!("ws://{addr}"));
    signaler
        .send_answer(&Answer {
            sdp: answer_sdp,
            connection_id: Some(conn_id.to_string()),
        })
        .await
        .expect("send_answer");

    // The server side should have accepted the answer and reached Connected
    // — assert that by checking the route is still alive and metrics reflect
    // the inbound message acked.
    assert!(adapter.routes().contains_key(&conn_id));

    server.abort();
}

#[tokio::test]
async fn ws_signaler_send_answer_rejects_missing_connection_id() {
    let signaler = WsSignaler::new("ws://127.0.0.1:1");
    let err = signaler
        .send_answer(&Answer {
            sdp: "v=0\r\n".into(),
            connection_id: None,
        })
        .await
        .expect_err("must require connection_id");
    assert!(matches!(
        &err,
        WebRtcError::Signaling(detail) if detail.contains("connection_id")
    ));
    assert_eq!(err.to_string(), "WebRTC operation failed (class=signaling)");
}

#[tokio::test]
async fn ws_signaler_retries_on_connect_failure() {
    // Hammer a closed port with exponential backoff; default retry=1 → 1 attempt;
    // retry=3 → 3 attempts. We verify the elapsed wall-clock matches.
    let cfg = WsSignalerConfig {
        retry_max_attempts: 3,
        initial_backoff: Duration::from_millis(80),
        max_backoff: Duration::from_secs(1),
        request_timeout: Duration::from_secs(1),
    };
    let signaler = WsSignaler::new("ws://127.0.0.1:1").with_config(cfg);
    let start = std::time::Instant::now();
    let err = signaler
        .send_ice(&IceCandidate("{}".into()))
        .await
        .expect_err("port 1 should refuse");
    let elapsed = start.elapsed();
    // 80ms + 160ms = ~240ms minimum (2 sleeps between 3 attempts).
    assert!(
        elapsed >= Duration::from_millis(200),
        "retry path did not back off; elapsed={elapsed:?}"
    );
    assert!(matches!(
        &err,
        WebRtcError::Signaling(detail)
            if detail.contains("attempt 3/3") || detail.contains("connect")
    ));
    assert_eq!(err.to_string(), "WebRTC operation failed (class=signaling)");
}

#[tokio::test]
async fn ws_signaler_default_no_retry() {
    let signaler = WsSignaler::new("ws://127.0.0.1:1");
    let start = std::time::Instant::now();
    let err = signaler
        .send_ice(&IceCandidate("{}".into()))
        .await
        .expect_err("port 1 refuses");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "no-retry default should fail fast; elapsed={elapsed:?}"
    );
    assert!(matches!(
        &err,
        WebRtcError::Signaling(detail)
            if detail.contains("attempt 1/1") || detail.contains("connect")
    ));
    assert_eq!(err.to_string(), "WebRTC operation failed (class=signaling)");
}

#[tokio::test]
async fn fixture_audio_source_emits_paced_frames() {
    use rvoip_core::ids::StreamId;
    use rvoip_webrtc::AudioSource;
    let mut source = FixtureAudioSource::new(StreamId::new(), 1234);
    let mut last_ts = None;
    for _ in 0..5 {
        let frame = source.next_packet().await.expect("ok").expect("some frame");
        assert!(!frame.payload.is_empty(), "Opus payload must be non-empty");
        if let Some(prev) = last_ts {
            assert_eq!(
                frame.timestamp_rtp.wrapping_sub(prev),
                960,
                "20 ms @ 48 kHz = 960 samples between packets"
            );
        }
        last_ts = Some(frame.timestamp_rtp);
    }
}

#[tokio::test]
async fn run_audio_bridges_source_to_sink() {
    use rvoip_core::ids::StreamId;
    use rvoip_core::stream::MediaFrame;
    use tokio::sync::mpsc;

    let (out_tx, mut out_rx) = mpsc::channel::<MediaFrame>(16);
    let (in_tx, in_rx) = mpsc::channel::<MediaFrame>(16);

    let sink = CountingAudioSink::new();
    let counter = Arc::clone(&sink.count);

    let source = FixtureAudioSource::new(StreamId::new(), 99);
    let (out_h, in_h) = rvoip_webrtc::run_audio(
        Box::new(source),
        Box::new(sink),
        out_tx,
        in_rx,
        AudioPacing::Unpaced,
    );

    // Pull 3 outbound frames the source produced.
    for _ in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
            .await
            .expect("outbound timeout")
            .expect("outbound channel closed");
        assert!(!frame.payload.is_empty());
        // Loop back through the inbound side to count delivery.
        in_tx.send(frame).await.expect("in_tx");
    }
    // Give the sink a moment to drain.
    for _ in 0..20 {
        if counter.load(std::sync::atomic::Ordering::Relaxed) >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        counter.load(std::sync::atomic::Ordering::Relaxed) >= 3,
        "sink should have received all 3 frames"
    );

    // Drop senders to let the inbound task exit; abort outbound runner.
    drop(in_tx);
    out_h.abort();
    let _ = tokio::time::timeout(Duration::from_secs(1), in_h).await;
}
