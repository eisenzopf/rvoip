//! Target-contacting WHEP draft-04 playback origination.

#![cfg(feature = "signaling-whip")]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, ETAG, IF_MATCH, LOCATION};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{patch, post};
use axum::Router;
use parking_lot::Mutex;
use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{
    StaticWebRtcBearerCredentialProvider, WebRtcAdapter, WebRtcBearerCredential, WebRtcConfig,
    WebRtcIceExchangePolicy, WebRtcOriginateContext, WebRtcSignalingMode, WebRtcTargetPolicy,
};
use webrtc::peer_connection::RTCIceCandidateInit;

const SESSION_PATH: &str = "/sessions/playback-1";

#[derive(Clone, Copy)]
enum ResponseMode {
    Answer,
    CounterOffer,
}

#[derive(Clone)]
struct WhepOriginState {
    mode: ResponseMode,
    config: WebRtcConfig,
    peer: Arc<Mutex<Option<Arc<RvoipPeerConnection>>>>,
    initial_offer: Arc<Mutex<Option<String>>>,
    server_sdp: Arc<Mutex<Option<String>>>,
    client_answer: Arc<Mutex<Option<String>>>,
    candidate_patches: Arc<AtomicUsize>,
    etag_generation: Arc<AtomicUsize>,
    deleted: Arc<AtomicBool>,
}

impl WhepOriginState {
    fn new(mode: ResponseMode) -> Self {
        let mut config = WebRtcConfig::loopback();
        config.trickle_ice = false;
        Self {
            mode,
            config,
            peer: Arc::new(Mutex::new(None)),
            initial_offer: Arc::new(Mutex::new(None)),
            server_sdp: Arc::new(Mutex::new(None)),
            client_answer: Arc::new(Mutex::new(None)),
            candidate_patches: Arc::new(AtomicUsize::new(0)),
            etag_generation: Arc::new(AtomicUsize::new(1)),
            deleted: Arc::new(AtomicBool::new(false)),
        }
    }

    fn current_etag(&self) -> String {
        format!("\"ice-{}\"", self.etag_generation.load(Ordering::SeqCst))
    }

    fn rotate_etag(&self) -> String {
        let generation = self.etag_generation.fetch_add(1, Ordering::SeqCst) + 1;
        format!("\"ice-{generation}\"")
    }
}

async fn create_playback(
    State(state): State<WhepOriginState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !is_authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let offer = match String::from_utf8(body.to_vec()) {
        Ok(offer) => offer,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    *state.initial_offer.lock() = Some(offer.clone());

    match state.mode {
        ResponseMode::Answer => {
            let peer = match RvoipPeerConnection::new(&state.config, PeerRole::Answerer).await {
                Ok(peer) => peer,
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            let answer = match peer.accept_offer_and_gather(&offer).await {
                Ok(answer) => answer,
                Err(_) => return StatusCode::UNPROCESSABLE_ENTITY.into_response(),
            };
            *state.server_sdp.lock() = Some(answer.clone());
            *state.peer.lock() = Some(peer);
            (
                StatusCode::CREATED,
                [
                    (CONTENT_TYPE, "application/sdp"),
                    (LOCATION, SESSION_PATH),
                    (ETAG, "\"ice-1\""),
                ],
                answer,
            )
                .into_response()
        }
        ResponseMode::CounterOffer => {
            let peer = match RvoipPeerConnection::new(&state.config, PeerRole::Offerer).await {
                Ok(peer) => peer,
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            let counter_offer = match peer.create_offer_and_gather().await {
                Ok(offer) => offer,
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            *state.server_sdp.lock() = Some(counter_offer.clone());
            *state.peer.lock() = Some(peer);
            (
                StatusCode::NOT_ACCEPTABLE,
                [
                    (CONTENT_TYPE, "application/sdp"),
                    (LOCATION, SESSION_PATH),
                    (ETAG, "\"ice-1\""),
                ],
                counter_offer,
            )
                .into_response()
        }
    }
}

async fn update_playback(
    State(state): State<WhepOriginState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !is_authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if headers.get(IF_MATCH).and_then(|value| value.to_str().ok())
        != Some(state.current_etag().as_str())
    {
        return StatusCode::PRECONDITION_FAILED.into_response();
    }
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let body = match String::from_utf8(body.to_vec()) {
        Ok(body) => body,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let peer = state.peer.lock().clone();
    let Some(peer) = peer else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match content_type {
        "application/sdp" => {
            if peer.set_remote_answer(&body).await.is_err() {
                return StatusCode::UNPROCESSABLE_ENTITY.into_response();
            }
            *state.client_answer.lock() = Some(body);
        }
        "application/trickle-ice-sdpfrag" => {
            let mut current_mid = None;
            let mut applied = 0usize;
            for line in body.lines().map(str::trim) {
                if let Some(mid) = line.strip_prefix("a=mid:") {
                    current_mid = Some(mid.to_owned());
                } else if let Some(candidate) = line.strip_prefix("a=candidate:") {
                    let candidate = RTCIceCandidateInit {
                        candidate: format!("candidate:{candidate}"),
                        sdp_mid: current_mid.clone(),
                        ..Default::default()
                    };
                    if peer.add_remote_ice_candidate(candidate).await.is_err() {
                        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
                    }
                    applied += 1;
                } else if line == "a=end-of-candidates" {
                    let candidate = RTCIceCandidateInit {
                        candidate: String::new(),
                        sdp_mid: current_mid.clone(),
                        ..Default::default()
                    };
                    if peer.add_remote_ice_candidate(candidate).await.is_err() {
                        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
                    }
                    applied += 1;
                }
            }
            if applied == 0 {
                return StatusCode::BAD_REQUEST.into_response();
            }
            state.candidate_patches.fetch_add(1, Ordering::SeqCst);
        }
        _ => return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response(),
    }
    let etag = state.rotate_etag();
    (StatusCode::NO_CONTENT, [(ETAG, etag)]).into_response()
}

async fn delete_playback(State(state): State<WhepOriginState>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if headers.get(IF_MATCH).and_then(|value| value.to_str().ok())
        != Some(state.current_etag().as_str())
    {
        return StatusCode::PRECONDITION_FAILED.into_response();
    }
    state.deleted.store(true, Ordering::SeqCst);
    let peer = state.peer.lock().take();
    if let Some(peer) = peer {
        let _ = peer.close().await;
    }
    StatusCode::OK.into_response()
}

#[tokio::test]
async fn whep_accepts_client_offer_and_terminates_the_retained_resource() {
    run_playback_case(ResponseMode::Answer).await;
}

#[tokio::test]
async fn whep_rolls_back_and_answers_a_server_counter_offer() {
    run_playback_case(ResponseMode::CounterOffer).await;
}

async fn run_playback_case(mode: ResponseMode) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let state = WhepOriginState::new(mode);
    let server_state = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WHEP origin");
    let address = listener.local_addr().expect("origin address");
    let app = Router::new()
        .route("/whep/playback", post(create_playback))
        .route(SESSION_PATH, patch(update_playback).delete(delete_playback))
        .with_state(server_state);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve WHEP origin");
    });

    let mut config = WebRtcConfig::loopback();
    let ice_policy = match mode {
        ResponseMode::Answer => WebRtcIceExchangePolicy::Trickle,
        ResponseMode::CounterOffer => WebRtcIceExchangePolicy::FullGather,
    };
    config.trickle_ice = ice_policy == WebRtcIceExchangePolicy::Trickle;
    let adapter = WebRtcAdapter::new(config);
    let endpoint = format!("http://{address}/whep/playback");
    let policy = WebRtcTargetPolicy::default()
        .allow_port(address.port())
        .allow_insecure(true)
        .allow_loopback(true)
        .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
        .expect("bounded WHEP policy");
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whep,
        ice_policy,
        policy,
        Some(Arc::new(StaticWebRtcBearerCredentialProvider::new(
            WebRtcBearerCredential::new("secret").expect("WHEP bearer"),
        ))),
    )
    .expect("validated WHEP context");
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        endpoint,
        Direction::Outbound,
        adapter.capabilities(),
    )
    .with_context(context);
    let handle = adapter
        .originate(request)
        .await
        .expect("prepare WHEP route");
    let connection_id = handle.connection.id;
    assert!(
        state.initial_offer.lock().is_none(),
        "originate must not POST"
    );
    let initial_local_sdp = adapter.local_sdp(&connection_id).expect("initial offer");
    assert_media_is_recvonly(&initial_local_sdp);

    if let Err(error) = adapter.activate_outbound(connection_id.clone()).await {
        panic!(
            "activate WHEP route: {error:?}; answer_patched={}; resource_deleted={}",
            state.client_answer.lock().is_some(),
            state.deleted.load(Ordering::SeqCst),
        );
    }
    let origin_peer = state.peer.lock().clone().expect("origin retained its peer");
    let (client_connected, origin_connected) = tokio::join!(
        adapter.accept(connection_id.clone()),
        origin_peer.wait_connected(Duration::from_secs(10)),
    );
    client_connected.expect("WHEP player ICE/DTLS connected");
    origin_connected.expect("WHEP origin ICE/DTLS connected");

    let posted_offer = state.initial_offer.lock().clone().expect("posted offer");
    assert_media_is_recvonly(&posted_offer);
    match mode {
        ResponseMode::Answer => {
            let answer = state.server_sdp.lock().clone().expect("server answer");
            assert_media_is_sendonly(&answer);
            assert!(state.client_answer.lock().is_none());
            assert!(state.candidate_patches.load(Ordering::SeqCst) > 0);
        }
        ResponseMode::CounterOffer => {
            let answer = state
                .client_answer
                .lock()
                .clone()
                .expect("PATCHed counter-offer answer");
            assert_media_is_recvonly(&answer);
            assert_eq!(
                adapter.local_sdp(&connection_id).expect("stored answer"),
                answer
            );
        }
    }

    adapter
        .end(connection_id.clone(), EndReason::Normal)
        .await
        .expect("delete WHEP resource");
    assert!(state.deleted.load(Ordering::SeqCst));
    assert!(!adapter.is_connection_live(&connection_id));
    server.abort();
}

fn is_authorized(headers: &HeaderMap) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some("Bearer secret")
}

fn assert_media_is_recvonly(sdp: &str) {
    assert!(sdp.contains("a=recvonly"), "missing recvonly media: {sdp}");
    assert!(
        !sdp.contains("a=sendonly"),
        "unexpected sendonly media: {sdp}"
    );
}

fn assert_media_is_sendonly(sdp: &str) {
    assert!(sdp.contains("a=sendonly"), "missing sendonly media: {sdp}");
    assert!(
        !sdp.contains("a=recvonly"),
        "unexpected recvonly media: {sdp}"
    );
}
