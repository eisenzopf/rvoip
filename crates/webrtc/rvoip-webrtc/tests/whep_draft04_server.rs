//! WHEP draft-04 server conformance and lifecycle regression coverage.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::{ConnectionAdapter, EndReason};
use rvoip_core::ids::ConnectionId;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcConfig, WebRtcServer, WebRtcServerBuilder, WhepServerMode};

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("HTTP client")
}

async fn server(mode: WhepServerMode) -> WebRtcServer {
    let _ = rustls::crypto::ring::default_provider().install_default();
    WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_whep_server_mode(mode)
        .with_whip("127.0.0.1:0")
        .build()
        .await
        .expect("WHEP server")
}

async fn player_offer() -> (Arc<RvoipPeerConnection>, String) {
    let player = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("WHEP player");
    player
        .prepare_receive_only_offer()
        .await
        .expect("receive-only player media");
    let offer = player
        .create_offer_and_gather()
        .await
        .expect("player offer");
    (player, offer)
}

fn resource_id(location: &str) -> ConnectionId {
    ConnectionId::from_string(location.rsplit('/').next().expect("resource connection id"))
}

async fn wait_for_no_http_tasks(server: &WebRtcServer) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.adapter().metrics().http_resource_tasks != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("WHEP HTTP supervisors drained");
}

#[tokio::test]
async fn draft04_discovery_and_client_offer_return_a_send_only_answer() {
    let server = server(WhepServerMode::Draft04).await;
    let base = format!("http://{}", server.whip_addr().expect("WHEP address"));
    let client = http();

    let discovery = client
        .get(format!("{base}/whep/discovery"))
        .send()
        .await
        .expect("WHEP discovery GET");
    assert_eq!(discovery.status(), reqwest::StatusCode::OK);
    assert_eq!(
        discovery
            .headers()
            .get("content-type")
            .expect("discovery content type"),
        "application/sdp"
    );

    let (player, offer) = player_offer().await;
    let created = client
        .post(format!("{base}/whep/player"))
        .header("content-type", "application/sdp")
        .body(offer)
        .send()
        .await
        .expect("WHEP POST");
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let location = created
        .headers()
        .get("location")
        .expect("Location")
        .to_str()
        .expect("Location text")
        .to_owned();
    let etag = created
        .headers()
        .get("etag")
        .expect("ETag")
        .to_str()
        .expect("ETag text")
        .to_owned();
    let answer = created.text().await.expect("WHEP answer");
    assert!(answer.contains("a=sendonly"));
    player
        .set_remote_answer(&answer)
        .await
        .expect("apply server answer");

    let connection_id = resource_id(&location);
    let adapter = server.adapter();
    let (server_connected, player_connected) = tokio::join!(
        adapter.accept(connection_id.clone()),
        player.wait_connected(Duration::from_secs(10)),
    );
    server_connected.expect("server connected");
    player_connected.expect("player connected");

    let deleted = client
        .delete(format!("{base}{location}"))
        .header("if-match", etag)
        .send()
        .await
        .expect("WHEP DELETE");
    assert_eq!(deleted.status(), reqwest::StatusCode::OK);
    assert_eq!(server.adapter().metrics().active_http_resources, 0);
    player.close().await.expect("close player");
    server.shutdown().await;
}

#[tokio::test]
async fn typed_counter_offer_rotates_one_etag_under_concurrent_patch() {
    let server = server(WhepServerMode::Draft04CounterOffer).await;
    let base = format!("http://{}", server.whip_addr().expect("WHEP address"));
    let client = http();
    let (player, offer) = player_offer().await;

    let counter = client
        .post(format!("{base}/whep/counter"))
        .header("content-type", "application/sdp")
        .body(offer)
        .send()
        .await
        .expect("counter-offer POST");
    assert_eq!(counter.status(), reqwest::StatusCode::NOT_ACCEPTABLE);
    assert!(counter
        .headers()
        .get("content-type")
        .expect("counter-offer content type")
        .to_str()
        .expect("content type text")
        .starts_with("application/sdp; valid-until="));
    let location = counter
        .headers()
        .get("location")
        .expect("Location")
        .to_str()
        .expect("Location text")
        .to_owned();
    let etag = counter
        .headers()
        .get("etag")
        .expect("ETag")
        .to_str()
        .expect("ETag text")
        .to_owned();
    let counter_offer = counter.text().await.expect("counter-offer body");
    assert!(counter_offer.contains("a=sendonly"));
    let answer = player
        .answer_counter_offer_after_rollback(&counter_offer)
        .await
        .expect("answer counter-offer");
    let resource_url = format!("{base}{location}");

    let first = client
        .patch(&resource_url)
        .header("content-type", "application/sdp")
        .header("if-match", &etag)
        .body(answer.clone())
        .send();
    let second = client
        .patch(&resource_url)
        .header("content-type", "application/sdp")
        .header("if-match", &etag)
        .body(answer)
        .send();
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first concurrent PATCH");
    let second = second.expect("second concurrent PATCH");
    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == reqwest::StatusCode::NO_CONTENT)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == reqwest::StatusCode::PRECONDITION_FAILED)
            .count(),
        1
    );
    let winning_etag = [&first, &second]
        .into_iter()
        .find(|response| response.status() == reqwest::StatusCode::NO_CONTENT)
        .and_then(|response| response.headers().get("etag"))
        .expect("winning rotated ETag")
        .to_str()
        .expect("winning ETag text")
        .to_owned();
    assert_ne!(winning_etag, etag);

    wait_for_no_http_tasks(&server).await;
    let deleted = client
        .delete(resource_url)
        .header("if-match", winning_etag)
        .send()
        .await
        .expect("counter-offer DELETE");
    assert_eq!(deleted.status(), reqwest::StatusCode::OK);
    assert_eq!(server.adapter().metrics().active_http_resources, 0);
    player.close().await.expect("close player");
    server.shutdown().await;
}

#[tokio::test]
async fn malformed_player_offers_never_receive_a_counter_offer() {
    let server = server(WhepServerMode::Draft04CounterOffer).await;
    let base = format!("http://{}", server.whip_addr().expect("WHEP address"));
    let client = http();

    let malformed = client
        .post(format!("{base}/whep/malformed"))
        .header("content-type", "application/sdp")
        .body("not SDP")
        .send()
        .await
        .expect("malformed POST");
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_ne!(malformed.status(), reqwest::StatusCode::NOT_ACCEPTABLE);

    let empty = client
        .post(format!("{base}/whep/empty"))
        .header("content-type", "application/sdp")
        .send()
        .await
        .expect("empty POST");
    assert_eq!(empty.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_ne!(empty.status(), reqwest::StatusCode::NOT_ACCEPTABLE);
    assert_eq!(server.adapter().metrics().active_sessions, 0);
    assert_eq!(server.adapter().metrics().active_http_resources, 0);
    server.shutdown().await;
}

#[tokio::test]
async fn non_http_terminal_paths_and_churn_leave_no_resource_state() {
    let server = server(WhepServerMode::Draft04).await;
    let adapter = server.adapter();
    let base = format!("http://{}", server.whip_addr().expect("WHEP address"));
    let client = http();
    let (_player, offer) = player_offer().await;

    for iteration in 0..16 {
        let created = client
            .post(format!("{base}/whep/churn-{iteration}"))
            .header("content-type", "application/sdp")
            .body(offer.clone())
            .send()
            .await
            .expect("churn POST");
        assert_eq!(created.status(), reqwest::StatusCode::CREATED);
        let location = created
            .headers()
            .get("location")
            .expect("Location")
            .to_str()
            .expect("Location text")
            .to_owned();
        let connection_id = resource_id(&location);
        adapter
            .end(connection_id, EndReason::Cancelled)
            .await
            .expect("non-HTTP terminal cleanup");
        assert_eq!(adapter.metrics().active_sessions, 0);
        assert_eq!(adapter.metrics().active_http_resources, 0);
        assert_eq!(adapter.metrics().peer_session_tasks, 0);
        assert_eq!(adapter.metrics().media_tasks, 0);
    }
    assert_eq!(adapter.metrics().http_resource_tasks, 0);
    assert_eq!(adapter.outbound_signaling_task_count(), 0);
    server.shutdown().await;
}

#[tokio::test]
async fn legacy_server_offer_requires_explicit_mode_and_is_observable() {
    let server = server(WhepServerMode::LegacyServerOffer).await;
    let base = format!("http://{}", server.whip_addr().expect("WHEP address"));
    let client = http();
    let created = client
        .post(format!("{base}/whep/legacy"))
        .send()
        .await
        .expect("legacy WHEP POST");
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    assert_eq!(server.adapter().metrics().legacy_whep_sessions_total, 1);
    let location = created
        .headers()
        .get("location")
        .expect("Location")
        .to_str()
        .expect("Location text")
        .to_owned();
    let etag = created
        .headers()
        .get("etag")
        .expect("ETag")
        .to_str()
        .expect("ETag text")
        .to_owned();
    let deleted = client
        .delete(format!("{base}{location}"))
        .header("if-match", etag)
        .send()
        .await
        .expect("legacy WHEP DELETE");
    assert_eq!(deleted.status(), reqwest::StatusCode::OK);
    wait_for_no_http_tasks(&server).await;
    assert_eq!(server.adapter().metrics().active_http_resources, 0);
    server.shutdown().await;
}
