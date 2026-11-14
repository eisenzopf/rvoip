//! Concurrent first callers of `streams()` / `accept()` must share one audio
//! media stream. Each stream owns an inbound pump on the remote RTP track, so
//! a second stream would start a competing reader on the same track.

use std::sync::Arc;

use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_stream_requests_create_exactly_one_stream() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = Arc::new(WebRtcAdapter::new(WebRtcConfig::loopback()));

    let originate = adapter
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: adapter.capabilities(),
            transport: None,
            context: Default::default(),
        })
        .await
        .expect("originate");
    let conn = originate.connection.id.clone();

    // No answer is applied, so no remote track arrives and every first
    // caller is still inside stream creation while the others start.
    let callers = 4;
    let barrier = Arc::new(tokio::sync::Barrier::new(callers));
    let tasks = (0..callers)
        .map(|_| {
            let adapter = Arc::clone(&adapter);
            let barrier = Arc::clone(&barrier);
            let conn = conn.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                adapter.streams(conn).await.expect("streams")
            })
        })
        .collect::<Vec<_>>();

    let mut ids = Vec::new();
    for task in tasks {
        let streams = task.await.expect("caller task");
        assert_eq!(
            streams.len(),
            1,
            "each caller must observe exactly one audio stream"
        );
        ids.push(streams[0].id());
    }
    assert!(
        ids.windows(2).all(|pair| pair[0] == pair[1]),
        "concurrent callers received different streams: {ids:?}"
    );

    let streams = adapter.streams(conn.clone()).await.expect("streams");
    assert_eq!(streams.len(), 1, "the route retained duplicate streams");
    assert_eq!(streams[0].id(), ids[0]);
}
