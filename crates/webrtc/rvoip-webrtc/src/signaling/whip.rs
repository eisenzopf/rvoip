//! WHIP / WHEP HTTP signaling (feature `signaling-whip`).
//!
//! Implements the subset of RFC 9725 (WHIP) needed for production browser
//! interop, plus RFC 8840 trickle ICE.
//!
//! ## Routing model — one connection per request
//!
//! Both WHIP and WHEP are **one-`PeerConnection`-per-HTTP-POST**. There is no
//! ingest-to-subscriber fan-out: every `POST /whep/{tag}` allocates a fresh
//! `connection_id` and runs an independent ICE / DTLS / SDP negotiation.
//! Multi-subscriber fan-out is SFU territory — see the crate-level README
//! "Limitations" section.
//!
//! ## Endpoints
//!
//! | Verb     | Path           | Body                                  | Success |
//! |----------|----------------|---------------------------------------|---------|
//! | `POST`   | `/whip/{tag}`  | `application/sdp` offer               | 201 + `Location`, `ETag`, `Accept-Patch`, optional `Link: rel=ice-server` |
//! | `PATCH`  | `/whip/{id}`   | `application/sdp` offer (ICE restart) | 200 + new answer SDP |
//! | `PATCH`  | `/whip/{id}`   | `application/trickle-ice-sdpfrag`     | 204 |
//! | `DELETE` | `/whip/{id}`   | -                                     | 200 |
//! | `OPTIONS`| any            | -                                     | 204 + CORS preflight headers |
//! | `POST`   | `/whep/{tag}`  | -                                     | 201 offer (subscriber answers via PATCH) |
//! | `PATCH`  | `/whep/{id}`   | `application/sdp` answer              | 204 |
//! | `DELETE` | `/whep/{id}`   | -                                     | 200 |
//! | `GET`    | `/healthz`     | -                                     | 200 plain text |
//! | `GET`    | `/readyz`      | -                                     | 200 plain text + active session count |

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, Path, State},
    http::{header::HeaderName, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, options, post},
    Router,
};
use dashmap::DashMap;
use rvoip_core::adapter::{ConnectionAdapter, InboundRoutingHint};
use rvoip_core::ids::ConnectionId;
use rvoip_sip_core::sdp::parser::parse_attribute;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use webrtc::peer_connection::RTCIceCandidateInit;

use crate::adapter::{RouteAuthorization, WebRtcAdapter};
use crate::errors::{Result, WebRtcError};
use crate::signaling::auth::{
    extract_bearer, AnonymousAuth, AuthContext, AuthRejection, WhipAuthHook,
};

const CT_TRICKLE: &str = "application/trickle-ice-sdpfrag";
const CT_SDP: &str = "application/sdp";

#[derive(Clone)]
pub struct WhipState {
    pub adapter: Arc<WebRtcAdapter>,
    /// Per-IP token bucket. Refilled at 1 token per (60_000 / per_min_cap) ms.
    /// Empty cap (cap=0) disables rate limiting entirely.
    rate: Arc<DashMap<std::net::IpAddr, (u32, Instant)>>,
    /// Pluggable Bearer-token enforcement (RFC 9725 §4.1). Default
    /// [`AnonymousAuth`] accepts everything — back-compat with pre-G2.
    auth: Arc<dyn WhipAuthHook>,
}

impl WhipState {
    pub fn new(adapter: Arc<WebRtcAdapter>) -> Self {
        Self {
            adapter,
            rate: Arc::new(DashMap::new()),
            auth: Arc::new(AnonymousAuth),
        }
    }

    /// Register a [`WhipAuthHook`]. When unset the default [`AnonymousAuth`]
    /// hook accepts every request — backward compatible with pre-G2.
    pub fn with_auth(mut self, auth: Arc<dyn WhipAuthHook>) -> Self {
        self.auth = auth;
        self
    }

    fn allow_request(&self, ip: std::net::IpAddr) -> bool {
        let cap = self.adapter.metrics(); // for inspection; not used here
        let _ = cap;
        // Pull config directly from the adapter via a public knob.
        let cap = whip_rate_limit_cap(&self.adapter);
        if cap == 0 {
            return true;
        }
        let window = Duration::from_secs(60);
        let now = Instant::now();
        let mut entry = self.rate.entry(ip).or_insert((cap, now));
        let (tokens, last) = *entry;
        // Refill linearly based on elapsed time since `last`.
        let elapsed = now.duration_since(last);
        let refill = (elapsed.as_secs_f64() / window.as_secs_f64() * cap as f64) as u32;
        let new_tokens = (tokens.saturating_add(refill)).min(cap);
        if new_tokens == 0 {
            *entry = (0, last);
            return false;
        }
        *entry = (new_tokens - 1, now);
        true
    }
}

fn whip_rate_limit_cap(_adapter: &Arc<WebRtcAdapter>) -> u32 {
    // Avoid leaking the full WebRtcConfig through a getter; expose only the
    // single field we need.
    _adapter.whip_rate_limit_cap_per_min()
}

/// Start a WHIP/WHEP HTTP server on `bind`.
pub async fn serve(bind: &str, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("bind {bind}: {e}")))?;
    serve_listener(listener, adapter).await
}

/// Serve WHIP/WHEP on an already-bound listener (used by integration tests).
pub async fn serve_listener(listener: TcpListener, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    serve_listener_with_auth(listener, adapter, Arc::new(AnonymousAuth)).await
}

/// Serve WHIP/WHEP with a custom auth hook.
pub async fn serve_listener_with_auth(
    listener: TcpListener,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WhipAuthHook>,
) -> Result<()> {
    let state = WhipState::new(Arc::clone(&adapter)).with_auth(auth);
    let app = build_router(state, &adapter);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
    Ok(())
}

/// Same as [`serve_listener`] but accepts a shutdown future for graceful drain.
pub async fn serve_listener_with_shutdown(
    listener: TcpListener,
    adapter: Arc<WebRtcAdapter>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    serve_listener_with_auth_and_shutdown(listener, adapter, Arc::new(AnonymousAuth), shutdown)
        .await
}

/// Same as [`serve_listener_with_auth`] plus graceful shutdown.
pub async fn serve_listener_with_auth_and_shutdown(
    listener: TcpListener,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WhipAuthHook>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let state = WhipState::new(Arc::clone(&adapter)).with_auth(auth);
    let app = build_router(state, &adapter);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
    Ok(())
}

/// HTTPS variant — TLS-terminating WHIP/WHEP server using `axum-server`.
/// Requires the `tls-rustls` feature.
#[cfg(feature = "tls-rustls")]
pub async fn serve_tls_with_shutdown(
    listener: std::net::TcpListener,
    tls: crate::tls::TlsConfig,
    adapter: Arc<WebRtcAdapter>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    serve_tls_with_auth_and_shutdown(listener, tls, adapter, Arc::new(AnonymousAuth), shutdown)
        .await
}

/// HTTPS WHIP/WHEP with the same auth hook used by the plaintext listener.
#[cfg(feature = "tls-rustls")]
pub async fn serve_tls_with_auth_and_shutdown(
    listener: std::net::TcpListener,
    tls: crate::tls::TlsConfig,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WhipAuthHook>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let state = WhipState::new(Arc::clone(&adapter)).with_auth(auth);
    let app = build_router(state, &adapter);
    let handle = axum_server::Handle::new();
    let handle_for_shutdown = handle.clone();
    tokio::spawn(async move {
        shutdown.await;
        handle_for_shutdown.shutdown();
    });
    axum_server::from_tcp_rustls(listener, tls.axum)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .map_err(|e| WebRtcError::Signaling(format!("WHIPS serve: {e}")))?;
    Ok(())
}

fn build_router(state: WhipState, adapter: &Arc<WebRtcAdapter>) -> Router {
    let mut app = Router::new()
        .route(
            "/whip/:id",
            post(whip_post)
                .patch(whip_patch)
                .delete(whip_delete)
                .options(whip_options),
        )
        .route(
            "/whep/:id",
            post(whep_post)
                .patch(whep_patch)
                .delete(whep_delete)
                .options(whip_options),
        )
        .route("/whip", options(whip_options))
        .route("/whep", options(whip_options))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(state);

    let origins = adapter.cors_origins().to_vec();
    if !origins.is_empty() {
        let cors = if origins.iter().any(|o| o == "*") {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers(Any)
                .expose_headers([
                    axum::http::header::HeaderName::from_static("location"),
                    axum::http::header::HeaderName::from_static("etag"),
                    axum::http::header::HeaderName::from_static("accept-patch"),
                    // RFC 9725 §4.6 — WHIP clients need to read the
                    // `Link: …; rel="ice-server"` header to discover
                    // server-advertised STUN/TURN. Without it in the
                    // expose list, browsers return null from
                    // Response.headers.get('Link').
                    axum::http::header::HeaderName::from_static("link"),
                ])
        } else {
            let mut layer = CorsLayer::new()
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers(Any)
                .expose_headers([
                    axum::http::header::HeaderName::from_static("location"),
                    axum::http::header::HeaderName::from_static("etag"),
                    axum::http::header::HeaderName::from_static("accept-patch"),
                    // RFC 9725 §4.6 — WHIP clients need to read the
                    // `Link: …; rel="ice-server"` header to discover
                    // server-advertised STUN/TURN. Without it in the
                    // expose list, browsers return null from
                    // Response.headers.get('Link').
                    axum::http::header::HeaderName::from_static("link"),
                ]);
            for o in origins {
                if let Ok(v) = o.parse::<HeaderValue>() {
                    layer = layer.allow_origin(v);
                }
            }
            layer
        };
        app = app.layer(cors);
    }

    app
}

async fn healthz() -> Response {
    (StatusCode::OK, "ok").into_response()
}

async fn readyz(State(state): State<WhipState>) -> Response {
    let m = state.adapter.metrics();
    (
        StatusCode::OK,
        format!(
            "ready\nactive_sessions={}\ninbound_total={}\nrejected={}\n",
            m.active_sessions, m.inbound_total, m.sessions_rejected_over_cap
        ),
    )
        .into_response()
}

/// Prometheus text-format metrics. Body shape produced by
/// [`crate::observability::render_prometheus_with_stats`] (G4) — includes
/// inbound + outbound RTP counters and the selected-pair RTT/bitrate gauges.
async fn metrics(State(state): State<WhipState>) -> Response {
    let (_n, snapshot) = state.adapter.aggregated_stats();
    let body =
        crate::observability::render_prometheus_with_stats(&state.adapter.metrics(), &snapshot);
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str("text/plain; version=0.0.4; charset=utf-8") {
        headers.insert("content-type", v);
    }
    (StatusCode::OK, headers, body).into_response()
}

fn content_type_of(headers: &HeaderMap) -> String {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Build the standard set of response headers for a WHIP 201/200 response.
/// Includes `Location`, `ETag`, `Accept-Patch`, `Content-Type` (the SDP body
/// type), and one `Link: <…>; rel="ice-server"` per configured ICE server
/// (RFC 9725 §4.6, auto-populated in G2).
fn build_session_headers(adapter: &Arc<WebRtcAdapter>, conn_id: &ConnectionId) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("/whip/{conn_id}")) {
        headers.insert("location", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("\"{conn_id}\"")) {
        headers.insert("etag", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{CT_SDP}, {CT_TRICKLE}")) {
        headers.insert("accept-patch", v);
    }
    if let Ok(v) = HeaderValue::from_str(CT_SDP) {
        headers.insert("content-type", v);
    }
    let link_name = HeaderName::from_static("link");
    for srv in adapter.ice_servers() {
        for url in &srv.urls {
            let value = match (&srv.username, &srv.credential) {
                (Some(u), Some(c)) => format!(
                    "<{}>; rel=\"ice-server\"; username=\"{}\"; credential=\"{}\"; credential-type=\"password\"",
                    url, u, c
                ),
                _ => format!("<{}>; rel=\"ice-server\"", url),
            };
            if let Ok(v) = HeaderValue::from_str(&value) {
                headers.append(&link_name, v);
            }
        }
    }
    headers
}

/// Run the configured auth hook and translate an [`AuthRejection`] into the
/// appropriate HTTP response. Returns `Ok(())` on accept.
async fn check_auth(
    state: &WhipState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> std::result::Result<AuthContext, Response> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| extract_bearer(Some(h)).map(|s| s.to_string()));
    match state
        .auth
        .authenticate(method, path, bearer.as_deref(), addr)
        .await
    {
        Ok(ctx) => Ok(ctx),
        Err(AuthRejection::Unauthorized { www_authenticate }) => {
            state.adapter.note_signaling_error();
            let mut resp_headers = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&www_authenticate) {
                resp_headers.insert("www-authenticate", v);
            }
            Err((StatusCode::UNAUTHORIZED, resp_headers, "unauthorized").into_response())
        }
        Err(AuthRejection::Forbidden) => {
            state.adapter.note_signaling_error();
            Err((StatusCode::FORBIDDEN, "forbidden").into_response())
        }
        Err(AuthRejection::Throttled { retry_after_secs }) => {
            let mut resp_headers = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp_headers.insert("retry-after", v);
            }
            Err((StatusCode::TOO_MANY_REQUESTS, resp_headers, "throttled").into_response())
        }
    }
}

fn route_error_response(state: &WhipState, error: WebRtcError) -> Response {
    state.adapter.note_signaling_error();
    match error {
        WebRtcError::ConnectionNotFound => StatusCode::NOT_FOUND.into_response(),
        WebRtcError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden").into_response(),
        WebRtcError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
        WebRtcError::InboundAdmissionRejected => {
            (StatusCode::FORBIDDEN, "inbound signaling was not admitted").into_response()
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "WebRTC signaling failed").into_response(),
    }
}

/// Stable ETag derivation from a connection id. Stays in sync with
/// [`build_session_headers`]; both must produce the same value.
fn etag_for(conn_id: &ConnectionId) -> String {
    format!("\"{conn_id}\"")
}

/// OPTIONS handler — RFC 9725 §4.6 / browser-side feature detection.
/// Advertises `Accept-Post: application/sdp` and `Accept-Patch` so JS clients
/// can probe support without a body.
async fn whip_options() -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(CT_SDP) {
        headers.insert("accept-post", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{CT_SDP}, {CT_TRICKLE}")) {
        headers.insert("accept-patch", v);
    }
    if let Ok(v) = HeaderValue::from_str("POST, PATCH, DELETE, OPTIONS") {
        headers.insert("allow", v);
    }
    (StatusCode::NO_CONTENT, headers).into_response()
}

async fn whip_post(
    State(state): State<WhipState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(tag): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let auth = match check_auth(&state, "POST", &format!("/whip/{tag}"), &headers, addr).await {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    if !state.allow_request(addr.ip()) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    let ct = content_type_of(&headers);
    if !ct.is_empty() && ct != CT_SDP {
        state.adapter.note_signaling_error();
        let mut resp_headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(CT_SDP) {
            resp_headers.insert("accept-post", v);
        }
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            resp_headers,
            format!("expected Content-Type: {CT_SDP}, got '{ct}'"),
        )
            .into_response();
    }
    if body.trim().is_empty() {
        state.adapter.note_signaling_error();
        return (StatusCode::BAD_REQUEST, "empty WHIP offer body").into_response();
    }

    let routing_hint = match InboundRoutingHint::new(tag) {
        Ok(routing_hint) => routing_hint,
        Err(_) => {
            state.adapter.note_signaling_error();
            return (StatusCode::BAD_REQUEST, "invalid WHIP routing tag").into_response();
        }
    };

    let conn_id = match state
        .adapter
        .apply_remote_offer_authorized_with_hint(
            &body,
            auth.route_authorization(),
            Some(routing_hint),
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            if matches!(&e, WebRtcError::Adapter(detail) if detail.contains("cap reached")) {
                state.adapter.note_signaling_error();
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "WebRTC capacity unavailable",
                )
                    .into_response();
            }
            return route_error_response(&state, e);
        }
    };
    let answer = match state.adapter.local_sdp(&conn_id) {
        Ok(s) => s,
        Err(e) => {
            let _ = state
                .adapter
                .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                .await;
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let headers = build_session_headers(&state.adapter, &conn_id);
    (StatusCode::CREATED, headers, answer).into_response()
}

/// WHIP PATCH: dispatch by `Content-Type` between ICE restart (full SDP) and
/// trickle ICE candidate update (RFC 8840 sdpfrag).
///
/// For ICE restart (`application/sdp`), enforces `If-Match: "<etag>"` per
/// RFC 9725 §4.4.1 — 428 when missing, 412 on mismatch. Trickle updates
/// do not require If-Match (per the spec).
async fn whip_patch(
    State(state): State<WhipState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(conn_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let auth = match check_auth(&state, "PATCH", &format!("/whip/{conn_id}"), &headers, addr).await
    {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    if !state.allow_request(addr.ip()) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    let content_type = content_type_of(&headers);
    let id = ConnectionId::from_string(conn_id);
    let authorization = auth.route_authorization();

    if content_type == CT_TRICKLE {
        if body.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, "empty WHIP trickle body").into_response();
        }
        return match apply_sdpfrag(&state.adapter, &id, &body, &authorization).await {
            Ok(count) => {
                if count == 0 {
                    (StatusCode::BAD_REQUEST, "no candidates in sdpfrag").into_response()
                } else {
                    StatusCode::NO_CONTENT.into_response()
                }
            }
            Err(e) => route_error_response(&state, e),
        };
    }

    if !content_type.is_empty() && content_type != CT_SDP {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("expected {CT_SDP} or {CT_TRICKLE}, got '{content_type}'"),
        )
            .into_response();
    }

    // ICE restart path: require If-Match per RFC 9725 §4.4.1.
    let expected_etag = etag_for(&id);
    match headers.get("if-match").and_then(|v| v.to_str().ok()) {
        None => {
            return (
                StatusCode::PRECONDITION_REQUIRED,
                "If-Match required for ICE restart (RFC 9725 §4.4.1)",
            )
                .into_response();
        }
        Some(supplied) if supplied != expected_etag && supplied != "*" => {
            return (StatusCode::PRECONDITION_FAILED, "ETag mismatch").into_response();
        }
        Some(_) => {}
    }

    if body.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty WHIP ICE restart offer body").into_response();
    }
    match state
        .adapter
        .apply_ice_restart_offer_authorized(id, &body, &authorization)
        .await
    {
        Ok(sdp) => (StatusCode::OK, [("content-type", CT_SDP)], sdp).into_response(),
        Err(e) => route_error_response(&state, e),
    }
}

/// Parse an RFC 8840 sdpfrag and apply each `a=candidate:` line as a trickle
/// ICE candidate, scoped by the latest seen `a=mid:` value. Returns the
/// number of candidates accepted.
async fn apply_sdpfrag(
    adapter: &Arc<WebRtcAdapter>,
    conn_id: &ConnectionId,
    body: &str,
    authorization: &RouteAuthorization,
) -> Result<usize> {
    // Authorize even a syntactically empty fragment so callers cannot use
    // PATCH response differences to probe another principal's route.
    adapter.authorize_network_route(conn_id, authorization)?;
    let mut current_mid: Option<String> = None;
    let mut mline_index: u16 = 0;
    let mut applied = 0usize;
    for line in body.lines() {
        let line = line.trim();
        if let Some(mid) = line.strip_prefix("a=mid:") {
            parse_attribute(&format!("mid:{}", mid.trim()))
                .map_err(|err| WebRtcError::Sdp(format!("invalid sdpfrag mid: {err}")))?;
            current_mid = Some(mid.trim().to_owned());
        } else if line.starts_with("m=") {
            if applied > 0 || current_mid.is_some() {
                mline_index = mline_index.saturating_add(1);
            }
            current_mid = None;
        } else if let Some(cand) = line.strip_prefix("a=candidate:") {
            parse_attribute(&format!("candidate:{cand}"))
                .map_err(|err| WebRtcError::Sdp(format!("invalid sdpfrag candidate: {err}")))?;
            let init = RTCIceCandidateInit {
                candidate: format!("candidate:{cand}"),
                sdp_mid: current_mid.clone(),
                sdp_mline_index: Some(mline_index),
                username_fragment: None,
                url: None,
            };
            adapter
                .apply_trickle_candidate_authorized(conn_id, init, authorization)
                .await?;
            applied += 1;
        }
    }
    Ok(applied)
}

async fn whep_post(
    State(state): State<WhipState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(tag): Path<String>,
    headers: HeaderMap,
    _body: String,
) -> Response {
    let auth = match check_auth(&state, "POST", &format!("/whep/{tag}"), &headers, addr).await {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    if !state.allow_request(addr.ip()) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    match state
        .adapter
        .originate(rvoip_core::adapter::OriginateRequest {
            session_id: rvoip_core::ids::SessionId::new(),
            participant_id: rvoip_core::ids::ParticipantId::new(),
            target: String::new(),
            direction: rvoip_core::connection::Direction::Outbound,
            capabilities: state.adapter.capabilities(),
            transport: None,
            context: Default::default(),
        })
        .await
    {
        Ok(handle) => {
            let conn_id = handle.connection.id.clone();
            let participant_id = handle.connection.participant_id.to_string();
            if let Err(error) = state.adapter.assign_route_authorization(
                &conn_id,
                auth.route_authorization(),
                participant_id,
            ) {
                let _ = state
                    .adapter
                    .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                    .await;
                return route_error_response(&state, error);
            }
            // WHEP creates an outbound adapter route directly rather than
            // through `Orchestrator::originate_connection`. Commit its
            // dormant lifecycle stage before exposing the Location id, just
            // as the orchestrator would after durable connection ownership.
            if state
                .adapter
                .activate_outbound(conn_id.clone())
                .await
                .is_err()
            {
                let _ = state
                    .adapter
                    .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                    .await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "outbound signaling activation failed",
                )
                    .into_response();
            }
            match state.adapter.local_sdp(&conn_id) {
                Ok(sdp) => {
                    let mut headers = build_session_headers(&state.adapter, &conn_id);
                    if let Ok(v) = HeaderValue::from_str(&format!("/whep/{conn_id}")) {
                        headers.insert("location", v);
                    }
                    (StatusCode::CREATED, headers, sdp).into_response()
                }
                Err(_) => {
                    let _ = state
                        .adapter
                        .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                        .await;
                    (StatusCode::INTERNAL_SERVER_ERROR, "WebRTC signaling failed").into_response()
                }
            }
        }
        Err(_) => {
            state.adapter.note_signaling_error();
            (StatusCode::INTERNAL_SERVER_ERROR, "WebRTC signaling failed").into_response()
        }
    }
}

/// WHEP subscriber answer: PATCH SDP answer onto the offerer connection.
async fn whep_patch(
    State(state): State<WhipState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(conn_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let auth = match check_auth(&state, "PATCH", &format!("/whep/{conn_id}"), &headers, addr).await
    {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let ct = content_type_of(&headers);
    if !ct.is_empty() && ct != CT_SDP {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("expected {CT_SDP}, got '{ct}'"),
        )
            .into_response();
    }
    if body.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty WHEP answer body").into_response();
    }
    let id = ConnectionId::from_string(conn_id);
    match state
        .adapter
        .accept_remote_answer_authorized(id, &body, &auth.route_authorization())
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => route_error_response(&state, e),
    }
}

async fn whip_delete(
    State(state): State<WhipState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(conn_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let auth = match check_auth(
        &state,
        "DELETE",
        &format!("/whip/{conn_id}"),
        &headers,
        addr,
    )
    .await
    {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let id = ConnectionId::from_string(conn_id);
    match state
        .adapter
        .end_authorized(
            id,
            rvoip_core::adapter::EndReason::Normal,
            &auth.route_authorization(),
        )
        .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => route_error_response(&state, error),
    }
}

async fn whep_delete(
    State(state): State<WhipState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(conn_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let auth = match check_auth(
        &state,
        "DELETE",
        &format!("/whep/{conn_id}"),
        &headers,
        addr,
    )
    .await
    {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let id = ConnectionId::from_string(conn_id);
    match state
        .adapter
        .end_authorized(
            id,
            rvoip_core::adapter::EndReason::Normal,
            &auth.route_authorization(),
        )
        .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => route_error_response(&state, error),
    }
}
