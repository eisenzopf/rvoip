//! WHIP / WHEP HTTP signaling (feature `signaling-whip`).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, patch, post},
    Router,
};
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::ids::ConnectionId;
use tokio::net::TcpListener;

use crate::adapter::WebRtcAdapter;
use crate::errors::{Result, WebRtcError};

#[derive(Clone)]
pub struct WhipState {
    pub adapter: Arc<WebRtcAdapter>,
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
    let state = WhipState { adapter };
    let app = Router::new()
        .route(
            "/whip/:id",
            post(whip_post)
                .patch(whip_ice_restart)
                .delete(whip_delete),
        )
        .route(
            "/whep/:id",
            post(whep_post)
                .patch(whep_patch)
                .delete(whep_delete),
        )
        .with_state(state);

    axum::serve(listener, app)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
    Ok(())
}

async fn whip_post(
    State(state): State<WhipState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    match whip_post_inner(&state, &id, &headers, &body).await {
        Ok(resp) => resp,
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn whip_post_inner(
    state: &WhipState,
    session: &str,
    headers: &HeaderMap,
    body: &str,
) -> Result<Response> {
    let _ = session;
    let offer = if body.is_empty() {
        headers
            .get("content-type")
            .and_then(|_| Some(body))
            .unwrap_or(body)
    } else {
        body
    };

    if offer.trim().is_empty() {
        return Err(WebRtcError::Signaling("empty WHIP offer body".into()));
    }

    let conn_id = state.adapter.apply_remote_offer(offer).await?;
    let answer = state.adapter.local_sdp(&conn_id)?;

    Ok((
        StatusCode::CREATED,
        [("Location", format!("/whip/{conn_id}"))],
        answer,
    )
        .into_response())
}

/// WHIP ICE restart: PATCH with a new offer SDP → updated answer SDP.
async fn whip_ice_restart(
    State(state): State<WhipState>,
    Path(conn_id): Path<String>,
    body: String,
) -> Response {
    if body.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty WHIP ICE restart offer body").into_response();
    }
    let id = ConnectionId::from_string(conn_id);
    match state.adapter.apply_ice_restart_offer(id, &body).await {
        Ok(sdp) => (StatusCode::OK, sdp).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn whep_post(
    State(state): State<WhipState>,
    Path(id): Path<String>,
    _headers: HeaderMap,
    _body: String,
) -> Response {
    let _ = id;
    match state
        .adapter
        .originate(rvoip_core::adapter::OriginateRequest {
            session_id: rvoip_core::ids::SessionId::new(),
            participant_id: rvoip_core::ids::ParticipantId::new(),
            target: String::new(),
            direction: rvoip_core::connection::Direction::Outbound,
            capabilities: state.adapter.capabilities(),
        })
        .await
    {
        Ok(handle) => {
            let conn_id = handle.connection.id.clone();
            match state.adapter.local_sdp(&conn_id) {
                Ok(sdp) => (
                    StatusCode::CREATED,
                    [("Location", format!("/whep/{conn_id}"))],
                    sdp,
                )
                    .into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// WHEP subscriber answer: PATCH SDP answer onto the offerer connection.
async fn whep_patch(
    State(state): State<WhipState>,
    Path(conn_id): Path<String>,
    body: String,
) -> Response {
    if body.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty WHEP answer body").into_response();
    }
    let id = ConnectionId::from_string(conn_id);
    match state.adapter.accept_remote_answer(id, &body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn whip_delete(
    State(state): State<WhipState>,
    Path(conn_id): Path<String>,
) -> Response {
    let id = ConnectionId::from_string(conn_id);
    let _ = state
        .adapter
        .end(id, rvoip_core::adapter::EndReason::Normal)
        .await;
    StatusCode::OK.into_response()
}

async fn whep_delete(
    State(state): State<WhipState>,
    Path(conn_id): Path<String>,
) -> Response {
    whip_delete(State(state), Path(conn_id)).await
}
