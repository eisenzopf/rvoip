//! H6.2: concurrency load — drive N simultaneous WHIP POSTs through one
//! adapter and assert the cap / rate-limit / metrics surfaces hold up.
//!
//! This is a *light* load test (50 sessions) — enough to expose lock
//! contention, dropped events, or task leaks without being slow.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

const N_CONCURRENT: usize = 50;

async fn fresh_offer() -> String {
    let peer = Arc::new(
        RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
            .await
            .expect("offerer peer"),
    );
    peer.add_local_audio_track().await.expect("audio track");
    peer.create_offer_and_gather().await.expect("offer sdp")
}

#[tokio::test]
async fn fifty_concurrent_whip_posts_succeed() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Cap set generously so all 50 should succeed (default loopback rate
    // limit is also disabled).
    let mut config = WebRtcConfig::loopback();
    config.max_concurrent_sessions = 100;
    let adapter = WebRtcAdapter::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move {
            let _ = whip::serve_listener(listener, adapter).await;
        }
    });

    // Pre-generate offers (offer creation involves ICE gather which we don't
    // want to overlap with the WHIP roundtrip we're measuring).
    let mut offers = Vec::with_capacity(N_CONCURRENT);
    for _ in 0..N_CONCURRENT {
        offers.push(fresh_offer().await);
    }

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http");

    let mut inflight = FuturesUnordered::new();
    for offer in offers {
        let http = http.clone();
        let url = format!("http://{addr}/whip/load");
        inflight.push(async move {
            http.post(&url)
                .header("content-type", "application/sdp")
                .body(offer)
                .send()
                .await
                .map(|r| r.status())
        });
    }

    let mut created = 0usize;
    let mut other_ok = 0usize;
    let mut failed = 0usize;
    while let Some(res) = inflight.next().await {
        match res {
            Ok(status) if status == reqwest::StatusCode::CREATED => created += 1,
            Ok(status) if status.is_success() => other_ok += 1,
            Ok(_) => failed += 1,
            Err(_) => failed += 1,
        }
    }

    assert_eq!(
        created + other_ok + failed,
        N_CONCURRENT,
        "every concurrent request must produce a status"
    );
    assert!(
        created >= N_CONCURRENT * 9 / 10,
        "expected at least 90% of {N_CONCURRENT} POSTs to succeed; got created={created} other={other_ok} failed={failed}"
    );

    // Metrics must reflect the load.
    let m = adapter.metrics();
    assert!(
        m.inbound_total as usize >= created,
        "inbound_total ({}) should be ≥ successful POSTs ({created})",
        m.inbound_total
    );
    assert!(
        m.active_sessions >= created,
        "active_sessions ({}) should hold all CREATED routes ({created})",
        m.active_sessions
    );

    serve.abort();
}

#[tokio::test]
async fn cap_truncates_concurrent_inbound_to_max_sessions() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config = WebRtcConfig::loopback();
    config.max_concurrent_sessions = 10;
    let adapter = WebRtcAdapter::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move {
            let _ = whip::serve_listener(listener, adapter).await;
        }
    });

    // 20 concurrent attempts against a 10-session cap.
    let mut offers = Vec::with_capacity(20);
    for _ in 0..20 {
        offers.push(fresh_offer().await);
    }

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("http");

    let mut inflight = FuturesUnordered::new();
    for offer in offers {
        let http = http.clone();
        let url = format!("http://{addr}/whip/cap");
        inflight.push(async move {
            http.post(&url)
                .header("content-type", "application/sdp")
                .body(offer)
                .send()
                .await
                .map(|r| r.status())
        });
    }

    let mut created = 0usize;
    let mut rejected_503 = 0usize;
    let mut other = 0usize;
    while let Some(res) = inflight.next().await {
        match res {
            Ok(s) if s == reqwest::StatusCode::CREATED => created += 1,
            Ok(s) if s == reqwest::StatusCode::SERVICE_UNAVAILABLE => rejected_503 += 1,
            Ok(_) | Err(_) => other += 1,
        }
    }

    assert!(
        created <= 10,
        "at most 10 should have been CREATED (cap=10); got {created}"
    );
    assert!(
        rejected_503 > 0,
        "expected at least one 503 from session cap; got created={created} 503={rejected_503} other={other}"
    );
    let m = adapter.metrics();
    assert!(
        m.sessions_rejected_over_cap >= 1,
        "metrics should record cap rejections"
    );

    serve.abort();
}
