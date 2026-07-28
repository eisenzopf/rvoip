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
//! | `POST`   | `/whep/{tag}`  | `application/sdp` player offer        | 201 + server answer, or typed 406 counter-offer when configured |
//! | `PATCH`  | `/whep/{id}`   | counter-offer answer or trickle ICE   | 204 |
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
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::MediaDirection;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use webrtc::peer_connection::RTCIceCandidateInit;

use crate::adapter::{
    HttpSignalingResource, HttpSignalingResourcePhase, RouteAuthorization, WebRtcAdapter,
};
use crate::errors::{Result, WebRtcError};
use crate::signaling::auth::{
    extract_bearer, AnonymousAuth, AuthContext, AuthRejection, WhipAuthHook,
};

const CT_TRICKLE: &str = "application/trickle-ice-sdpfrag";
const CT_SDP: &str = "application/sdp";
const MAX_SDPFRAG_BYTES: usize = 64 * 1024;
const MAX_SDPFRAG_MUTATIONS: usize = 256;
const WHEP_COUNTER_OFFER_LIFETIME: Duration = Duration::from_secs(30);

/// WHEP listener behavior.
///
/// Draft-04 client-offer handling is the default. The counter-offer variant
/// is an explicit endpoint policy used when the origin needs to replace the
/// player's offer. The historical empty-POST/server-offer exchange is kept
/// only as an opt-in compatibility mode and is observable through metrics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WhepServerMode {
    #[default]
    Draft04,
    Draft04CounterOffer,
    LegacyServerOffer,
}

#[derive(Clone)]
pub struct WhipState {
    pub adapter: Arc<WebRtcAdapter>,
    /// Per-IP token bucket. Refilled at 1 token per (60_000 / per_min_cap) ms.
    /// Empty cap (cap=0) disables rate limiting entirely.
    rate: Arc<DashMap<std::net::IpAddr, (u32, Instant)>>,
    /// Pluggable Bearer-token enforcement (RFC 9725 §4.1). Default
    /// [`AnonymousAuth`] accepts everything — back-compat with pre-G2.
    auth: Arc<dyn WhipAuthHook>,
    whep_mode: WhepServerMode,
}

impl WhipState {
    pub fn new(adapter: Arc<WebRtcAdapter>) -> Self {
        Self {
            adapter,
            rate: Arc::new(DashMap::new()),
            auth: Arc::new(AnonymousAuth),
            whep_mode: WhepServerMode::Draft04,
        }
    }

    /// Register a [`WhipAuthHook`]. When unset the default [`AnonymousAuth`]
    /// hook accepts every request — backward compatible with pre-G2.
    pub fn with_auth(mut self, auth: Arc<dyn WhipAuthHook>) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_whep_mode(mut self, mode: WhepServerMode) -> Self {
        self.whep_mode = mode;
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

    fn register_resource(
        &self,
        connection_id: &ConnectionId,
        phase: HttpSignalingResourcePhase,
    ) -> Option<(String, Arc<HttpSignalingResource>)> {
        self.adapter.register_http_resource(connection_id, phase)
    }

    fn resource_version(&self, connection_id: &ConnectionId) -> Option<Arc<HttpSignalingResource>> {
        self.adapter.http_resource(connection_id)
    }

    fn remove_resource(&self, connection_id: &ConnectionId, version: &Arc<HttpSignalingResource>) {
        self.adapter.remove_http_resource_if(connection_id, version);
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
    serve_listener_with_auth_and_mode(listener, adapter, auth, WhepServerMode::Draft04).await
}

/// Serve WHIP plus an explicitly selected WHEP protocol mode.
pub async fn serve_listener_with_auth_and_mode(
    listener: TcpListener,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WhipAuthHook>,
    whep_mode: WhepServerMode,
) -> Result<()> {
    let state = WhipState::new(Arc::clone(&adapter))
        .with_auth(auth)
        .with_whep_mode(whep_mode);
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
    serve_listener_with_auth_mode_and_shutdown(
        listener,
        adapter,
        auth,
        WhepServerMode::Draft04,
        shutdown,
    )
    .await
}

pub async fn serve_listener_with_auth_mode_and_shutdown(
    listener: TcpListener,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WhipAuthHook>,
    whep_mode: WhepServerMode,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let state = WhipState::new(Arc::clone(&adapter))
        .with_auth(auth)
        .with_whep_mode(whep_mode);
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
    serve_tls_with_auth_mode_and_shutdown(
        listener,
        tls,
        adapter,
        auth,
        WhepServerMode::Draft04,
        shutdown,
    )
    .await
}

#[cfg(feature = "tls-rustls")]
pub async fn serve_tls_with_auth_mode_and_shutdown(
    listener: std::net::TcpListener,
    tls: crate::tls::TlsConfig,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WhipAuthHook>,
    whep_mode: WhepServerMode,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let state = WhipState::new(Arc::clone(&adapter))
        .with_auth(auth)
        .with_whep_mode(whep_mode);
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
                .get(whep_get)
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

/// Draft-04 endpoint/session discovery. WHEP resources have no HTTP
/// representation, so GET and Axum's corresponding HEAD handling return a
/// successful empty response while retaining the SDP content type.
async fn whep_get() -> Response {
    (
        StatusCode::OK,
        [("content-type", CT_SDP)],
        axum::body::Body::empty(),
    )
        .into_response()
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
fn build_session_headers(
    adapter: &Arc<WebRtcAdapter>,
    conn_id: &ConnectionId,
    resource_kind: &str,
    etag: &str,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("/{resource_kind}/{conn_id}")) {
        headers.insert("location", v);
    }
    if let Ok(v) = HeaderValue::from_str(etag) {
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

fn new_strong_etag() -> String {
    // A fresh opaque UUID on every successful mutation prevents ABA across
    // ICE restarts and across server/resource recreation.
    format!("\"{}\"", ConnectionId::new())
}

fn require_exact_if_match(
    headers: &HeaderMap,
    expected: &str,
) -> std::result::Result<(), Response> {
    let mut supplied = headers.get_all("if-match").iter();
    let Some(first) = supplied.next() else {
        return Err((StatusCode::PRECONDITION_REQUIRED, "If-Match required").into_response());
    };
    if supplied.next().is_some() {
        return Err((StatusCode::PRECONDITION_FAILED, "ETag mismatch").into_response());
    }
    let Ok(first) = first.to_str() else {
        return Err((StatusCode::PRECONDITION_FAILED, "ETag mismatch").into_response());
    };
    if first != expected {
        return Err((StatusCode::PRECONDITION_FAILED, "ETag mismatch").into_response());
    }
    Ok(())
}

fn etag_response(status: StatusCode, etag: &str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(etag) = HeaderValue::from_str(etag) {
        headers.insert("etag", etag);
    }
    (status, headers).into_response()
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
        .apply_remote_offer_authorized_with_hint_and_ice_policy(
            &body,
            auth.route_authorization(),
            Some(routing_hint),
            crate::WebRtcIceExchangePolicy::FullGather,
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

    let Some((etag, _)) = state.register_resource(&conn_id, HttpSignalingResourcePhase::Whip)
    else {
        return (StatusCode::CONFLICT, "WHIP route ended during creation").into_response();
    };
    let headers = build_session_headers(&state.adapter, &conn_id, "whip", &etag);
    (StatusCode::CREATED, headers, answer).into_response()
}

/// WHIP PATCH: dispatch by `Content-Type` between ICE restart (full SDP) and
/// trickle ICE candidate update (RFC 8840 sdpfrag).
///
/// Every resource mutation is serialized behind its current strong ETag.
/// Missing preconditions return 428 and stale, weak, wildcard, duplicated, or
/// otherwise inexact validators return 412.
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

    if let Err(error) = state.adapter.authorize_network_route(&id, &authorization) {
        return route_error_response(&state, error);
    }
    let Some(resource) = state.resource_version(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut version = resource.version.lock().await;
    if version.phase != HttpSignalingResourcePhase::Whip {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(response) = require_exact_if_match(&headers, &version.etag) {
        return response;
    }

    if content_type == CT_TRICKLE {
        if body.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, "empty WHIP trickle body").into_response();
        }
        return match apply_sdpfrag(&state.adapter, &id, &body, &authorization).await {
            Ok(count) => {
                if count == 0 {
                    (StatusCode::BAD_REQUEST, "no candidates in sdpfrag").into_response()
                } else {
                    version.etag = new_strong_etag();
                    etag_response(StatusCode::NO_CONTENT, &version.etag)
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

    if body.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty WHIP ICE restart offer body").into_response();
    }
    match state
        .adapter
        .apply_ice_restart_offer_authorized(id, &body, &authorization)
        .await
    {
        Ok(sdp) => {
            version.etag = new_strong_etag();
            let mut response_headers = HeaderMap::new();
            response_headers.insert("content-type", HeaderValue::from_static(CT_SDP));
            if let Ok(etag) = HeaderValue::from_str(&version.etag) {
                response_headers.insert("etag", etag);
            }
            (StatusCode::OK, response_headers, sdp).into_response()
        }
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
    if body.len() > MAX_SDPFRAG_BYTES {
        return Err(WebRtcError::Signaling(
            "trickle ICE fragment exceeds its bound".into(),
        ));
    }
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
            if applied >= MAX_SDPFRAG_MUTATIONS {
                return Err(WebRtcError::Signaling(
                    "trickle ICE fragment exceeds its mutation bound".into(),
                ));
            }
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
        } else if line == "a=end-of-candidates" {
            if applied >= MAX_SDPFRAG_MUTATIONS {
                return Err(WebRtcError::Signaling(
                    "trickle ICE fragment exceeds its mutation bound".into(),
                ));
            }
            adapter
                .apply_trickle_candidate_authorized(
                    conn_id,
                    RTCIceCandidateInit {
                        candidate: String::new(),
                        sdp_mid: current_mid.clone(),
                        sdp_mline_index: Some(mline_index),
                        username_fragment: None,
                        url: None,
                    },
                    authorization,
                )
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
    body: String,
) -> Response {
    let auth = match check_auth(&state, "POST", &format!("/whep/{tag}"), &headers, addr).await {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    if !state.allow_request(addr.ip()) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    if state.whep_mode == WhepServerMode::LegacyServerOffer {
        return whep_legacy_post(&state, auth, &body).await;
    }

    let content_type = content_type_of(&headers);
    if content_type != CT_SDP {
        let mut response_headers = HeaderMap::new();
        response_headers.insert("accept-post", HeaderValue::from_static(CT_SDP));
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            response_headers,
            "WHEP draft-04 requires Content-Type: application/sdp",
        )
            .into_response();
    }
    if let Err(response) = validate_whep_player_offer(&body) {
        return response;
    }
    let routing_hint = match InboundRoutingHint::new(tag) {
        Ok(routing_hint) => routing_hint,
        Err(_) => {
            state.adapter.note_signaling_error();
            return (StatusCode::BAD_REQUEST, "invalid WHEP resource tag").into_response();
        }
    };
    let authorization = auth.route_authorization();

    match state.whep_mode {
        WhepServerMode::Draft04 => {
            let conn_id = match state
                .adapter
                .apply_remote_offer_authorized_with_hint_and_ice_policy(
                    &body,
                    authorization,
                    Some(routing_hint),
                    crate::WebRtcIceExchangePolicy::FullGather,
                )
                .await
            {
                Ok(connection_id) => connection_id,
                Err(error) => return whep_creation_error_response(&state, error),
            };
            let answer = match state.adapter.local_sdp(&conn_id) {
                Ok(answer) if whep_answer_is_send_only(&answer) => answer,
                Ok(_) | Err(_) => {
                    let _ = state
                        .adapter
                        .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                        .await;
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "WHEP answer did not satisfy playback direction",
                    )
                        .into_response();
                }
            };
            let Some((etag, _)) =
                state.register_resource(&conn_id, HttpSignalingResourcePhase::WhepEstablished)
            else {
                return (StatusCode::CONFLICT, "WHEP route ended during creation").into_response();
            };
            let response_headers = build_session_headers(&state.adapter, &conn_id, "whep", &etag);
            (StatusCode::CREATED, response_headers, answer).into_response()
        }
        WhepServerMode::Draft04CounterOffer => {
            let (conn_id, offer) = match state
                .adapter
                .create_whep_counter_offer_authorized_with_hint(authorization, routing_hint)
                .await
            {
                Ok(created) => created,
                Err(error) => return whep_creation_error_response(&state, error),
            };
            if !whep_counter_offer_is_send_only(&offer) {
                let _ = state
                    .adapter
                    .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                    .await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "WHEP counter-offer direction is invalid",
                )
                    .into_response();
            }
            let expires_at = tokio::time::Instant::now() + WHEP_COUNTER_OFFER_LIFETIME;
            let Some((etag, resource)) = state.register_resource(
                &conn_id,
                HttpSignalingResourcePhase::WhepAwaitingCounterOfferAnswer { expires_at },
            ) else {
                return (StatusCode::CONFLICT, "WHEP route ended during creation").into_response();
            };
            spawn_whep_counter_offer_expiry(
                Arc::clone(&state.adapter),
                conn_id.clone(),
                Arc::clone(&resource),
                expires_at,
            );
            let mut response_headers =
                build_session_headers(&state.adapter, &conn_id, "whep", &etag);
            let valid_until = chrono::Utc::now()
                + chrono::Duration::from_std(WHEP_COUNTER_OFFER_LIFETIME)
                    .unwrap_or_else(|_| chrono::Duration::seconds(30));
            let content_type = format!(
                "{CT_SDP}; valid-until=\"{}\"",
                valid_until.format("%a, %d %b %Y %H:%M:%S GMT")
            );
            if let Ok(value) = HeaderValue::from_str(&content_type) {
                response_headers.insert("content-type", value);
            }
            (StatusCode::NOT_ACCEPTABLE, response_headers, offer).into_response()
        }
        // The early return above owns this compatibility path. Keeping this
        // arm non-panicking protects library callers if mode dispatch is
        // refactored later.
        WhepServerMode::LegacyServerOffer => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "legacy WHEP mode dispatch failed",
        )
            .into_response(),
    }
}

async fn whep_legacy_post(state: &WhipState, auth: AuthContext, body: &str) -> Response {
    if !body.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "legacy WHEP server-offer mode requires an empty POST body",
        )
            .into_response();
    }
    state.adapter.note_legacy_whep_session();
    tracing::warn!(
        protocol = "whep",
        mode = "legacy-server-offer",
        "serving explicitly enabled legacy WHEP exchange"
    );
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
                    let expires_at = tokio::time::Instant::now() + WHEP_COUNTER_OFFER_LIFETIME;
                    let Some((etag, resource)) = state.register_resource(
                        &conn_id,
                        HttpSignalingResourcePhase::WhepAwaitingCounterOfferAnswer { expires_at },
                    ) else {
                        return (StatusCode::CONFLICT, "WHEP route ended during creation")
                            .into_response();
                    };
                    spawn_whep_counter_offer_expiry(
                        Arc::clone(&state.adapter),
                        conn_id.clone(),
                        resource,
                        expires_at,
                    );
                    let headers = build_session_headers(&state.adapter, &conn_id, "whep", &etag);
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

fn validate_whep_player_offer(body: &str) -> std::result::Result<(), Response> {
    if body.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty WHEP offer body").into_response());
    }
    let session = body
        .parse::<SdpSession>()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid WHEP SDP offer").into_response())?;
    let mut active_media = 0usize;
    for media in &session.media_descriptions {
        if media.port == 0
            || !(media.media.eq_ignore_ascii_case("audio")
                || media.media.eq_ignore_ascii_case("video"))
        {
            continue;
        }
        active_media += 1;
        let direction = media
            .direction
            .or(session.direction)
            .unwrap_or(MediaDirection::SendRecv);
        if matches!(
            direction,
            MediaDirection::SendOnly | MediaDirection::Inactive
        ) {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "WHEP player offer has a forbidden media direction",
            )
                .into_response());
        }
    }
    if active_media == 0 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "WHEP offer contains no active audio or video media",
        )
            .into_response());
    }
    Ok(())
}

fn whep_answer_is_send_only(sdp: &str) -> bool {
    whep_sdp_has_direction(sdp, MediaDirection::SendOnly)
}

fn whep_counter_offer_is_send_only(sdp: &str) -> bool {
    whep_sdp_has_direction(sdp, MediaDirection::SendOnly)
}

fn whep_sdp_has_direction(sdp: &str, required: MediaDirection) -> bool {
    let Ok(session) = sdp.parse::<SdpSession>() else {
        return false;
    };
    let mut active_media = 0usize;
    for media in &session.media_descriptions {
        if media.port == 0
            || !(media.media.eq_ignore_ascii_case("audio")
                || media.media.eq_ignore_ascii_case("video"))
        {
            continue;
        }
        active_media += 1;
        if media
            .direction
            .or(session.direction)
            .unwrap_or(MediaDirection::SendRecv)
            != required
        {
            return false;
        }
    }
    active_media > 0
}

fn whep_creation_error_response(state: &WhipState, error: WebRtcError) -> Response {
    state.adapter.note_signaling_error();
    match error {
        WebRtcError::InboundAdmissionRejected => {
            (StatusCode::FORBIDDEN, "WHEP attachment was not admitted").into_response()
        }
        WebRtcError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
        WebRtcError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden").into_response(),
        WebRtcError::Adapter(ref detail) if detail.contains("cap reached") => (
            StatusCode::SERVICE_UNAVAILABLE,
            "WebRTC capacity unavailable",
        )
            .into_response(),
        WebRtcError::Sdp(_) | WebRtcError::Webrtc(_) | WebRtcError::IncompatibleCapabilities => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "WHEP offer could not be negotiated",
        )
            .into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "WebRTC signaling failed").into_response(),
    }
}

fn spawn_whep_counter_offer_expiry(
    adapter: Arc<WebRtcAdapter>,
    connection_id: ConnectionId,
    resource: Arc<HttpSignalingResource>,
    expires_at: tokio::time::Instant,
) {
    let mut cancelled = resource.expiry_cancelled();
    let task_guard = adapter.start_http_resource_task();
    tokio::spawn(async move {
        let _task_guard = task_guard;
        if *cancelled.borrow() {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep_until(expires_at) => {}
            changed = cancelled.changed() => {
                let _ = changed;
                return;
            }
        }
        let current = adapter.http_resource(&connection_id);
        if current
            .as_ref()
            .is_none_or(|candidate| !Arc::ptr_eq(candidate, &resource))
        {
            return;
        }
        let expired = {
            let version = resource.version.lock().await;
            matches!(
                version.phase,
                HttpSignalingResourcePhase::WhepAwaitingCounterOfferAnswer { expires_at: deadline }
                    if tokio::time::Instant::now() >= deadline
            )
        };
        if expired {
            let _ = adapter
                .end(
                    connection_id,
                    rvoip_core::adapter::EndReason::Failed {
                        detail: "WHEP counter-offer expired".into(),
                    },
                )
                .await;
        }
    });
}

/// WHEP subscriber answer or trickle update. Both paths require and rotate
/// the exact current strong ETag under one per-resource mutation lock.
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
    if !state.allow_request(addr.ip()) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    let content_type = content_type_of(&headers);
    if content_type != CT_SDP && content_type != CT_TRICKLE {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("expected {CT_SDP} or {CT_TRICKLE}, got '{content_type}'"),
        )
            .into_response();
    }
    if body.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty WHEP PATCH body").into_response();
    }
    let id = ConnectionId::from_string(conn_id);
    let authorization = auth.route_authorization();
    if let Err(error) = state.adapter.authorize_network_route(&id, &authorization) {
        return route_error_response(&state, error);
    }
    let Some(resource) = state.resource_version(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut version = resource.version.lock().await;
    if version.phase == HttpSignalingResourcePhase::Whip {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(response) = require_exact_if_match(&headers, &version.etag) {
        return response;
    }
    if let HttpSignalingResourcePhase::WhepAwaitingCounterOfferAnswer { expires_at } = version.phase
    {
        if tokio::time::Instant::now() >= expires_at {
            drop(version);
            let _ = state
                .adapter
                .end(
                    id.clone(),
                    rvoip_core::adapter::EndReason::Failed {
                        detail: "WHEP counter-offer expired".into(),
                    },
                )
                .await;
            state.remove_resource(&id, &resource);
            return StatusCode::GONE.into_response();
        }
    }
    let result = if content_type == CT_TRICKLE {
        if version.phase != HttpSignalingResourcePhase::WhepEstablished {
            return (
                StatusCode::CONFLICT,
                "WHEP counter-offer answer is still pending",
            )
                .into_response();
        }
        apply_sdpfrag(&state.adapter, &id, &body, &authorization)
            .await
            .and_then(|count| {
                (count > 0).then_some(()).ok_or_else(|| {
                    WebRtcError::Signaling("WHEP trickle fragment had no mutations".into())
                })
            })
    } else {
        if !matches!(
            version.phase,
            HttpSignalingResourcePhase::WhepAwaitingCounterOfferAnswer { .. }
        ) {
            return (
                StatusCode::CONFLICT,
                "WHEP does not permit SDP renegotiation",
            )
                .into_response();
        }
        state
            .adapter
            .accept_remote_answer_authorized(id, &body, &authorization)
            .await
    };
    match result {
        Ok(()) => {
            if content_type == CT_SDP {
                version.phase = HttpSignalingResourcePhase::WhepEstablished;
                resource.cancel_expiry_task();
            }
            version.etag = new_strong_etag();
            etag_response(StatusCode::NO_CONTENT, &version.etag)
        }
        Err(error) => route_error_response(&state, error),
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
    let authorization = auth.route_authorization();
    if let Err(error) = state.adapter.authorize_network_route(&id, &authorization) {
        return route_error_response(&state, error);
    }
    let Some(resource) = state.resource_version(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let version = resource.version.lock().await;
    if version.phase != HttpSignalingResourcePhase::Whip {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(response) = require_exact_if_match(&headers, &version.etag) {
        return response;
    }
    match state
        .adapter
        .end_authorized(
            id.clone(),
            rvoip_core::adapter::EndReason::Normal,
            &authorization,
        )
        .await
    {
        Ok(()) => {
            drop(version);
            state.remove_resource(&id, &resource);
            StatusCode::OK.into_response()
        }
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
    let authorization = auth.route_authorization();
    if let Err(error) = state.adapter.authorize_network_route(&id, &authorization) {
        return route_error_response(&state, error);
    }
    let Some(resource) = state.resource_version(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let version = resource.version.lock().await;
    if version.phase == HttpSignalingResourcePhase::Whip {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(response) = require_exact_if_match(&headers, &version.etag) {
        return response;
    }
    match state
        .adapter
        .end_authorized(
            id.clone(),
            rvoip_core::adapter::EndReason::Normal,
            &authorization,
        )
        .await
    {
        Ok(()) => {
            drop(version);
            state.remove_resource(&id, &resource);
            StatusCode::OK.into_response()
        }
        Err(error) => route_error_response(&state, error),
    }
}
