//! Legacy per-operation WebSocket JSON signaler for
//! [`WebRtcClient`] → [`WebRtcServer`](crate::server::WebRtcServer).
//!
//! Supports both directions:
//! - **Offerer flow**: `send_offer` connects, ships the offer, awaits the
//!   `{type:"answer"}` reply, and returns the SDP + server-side connection id.
//! - **Answerer flow**: `send_answer` connects, ships
//!   `{type:"answer", sdp, connection_id}`, awaits the `{type:"ack"}` reply.
//! - **Trickle ICE**: `send_ice` sends `{type:"ice-candidate", candidate, connection_id}`.
//!
//! This compatibility wrapper does not retain a route socket: offer, answer,
//! and ICE methods each create their own exchange. New adapter-driven
//! originations use the persistent target-contacting session instead.
//!
//! Set [`WsSignalerConfig::retry_max_attempts`] > 1 to enable exponential
//! backoff retry on WS connect failure.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;

use crate::client::{Answer, IceCandidate, Offer, Signaler};
use crate::errors::{Result, WebRtcError};
use crate::signaling::websocket::SignalingMessage;

/// Per-call WS signaler configuration (connect retry, request timeout).
#[derive(Clone, Debug)]
pub struct WsSignalerConfig {
    /// Total WS-connect attempts before failing (1 = no retry). Default: 1.
    pub retry_max_attempts: u32,
    /// Initial backoff between connect attempts.
    pub initial_backoff: Duration,
    /// Cap on the per-attempt backoff (exponential growth otherwise).
    pub max_backoff: Duration,
    /// How long to wait for the server's reply after sending offer/answer.
    pub request_timeout: Duration,
}

impl Default for WsSignalerConfig {
    fn default() -> Self {
        Self {
            retry_max_attempts: 1,
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
        }
    }
}

/// Connect to a `WebRtcServer` WebSocket signaling endpoint.
pub struct WsSignaler {
    ws_url: String,
    config: WsSignalerConfig,
}

impl WsSignaler {
    pub fn new(ws_url: impl Into<String>) -> Self {
        Self {
            ws_url: ws_url.into(),
            config: WsSignalerConfig::default(),
        }
    }

    pub fn with_config(mut self, config: WsSignalerConfig) -> Self {
        self.config = config;
        self
    }

    pub fn config(&self) -> &WsSignalerConfig {
        &self.config
    }

    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }
}

type WsClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_with_retry(url: &str, config: &WsSignalerConfig) -> Result<WsClient> {
    let max_attempts = config.retry_max_attempts.max(1);
    let mut backoff = config.initial_backoff;
    let mut last_err: Option<WebRtcError> = None;
    for attempt in 0..max_attempts {
        match connect_async(url).await {
            Ok((ws, _)) => return Ok(ws),
            Err(e) => {
                last_err = Some(WebRtcError::Signaling(format!(
                    "ws connect {} attempt {}/{}: {e}",
                    url,
                    attempt + 1,
                    max_attempts
                )));
                if attempt + 1 == max_attempts {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.max_backoff);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| WebRtcError::Signaling("ws connect failed".into())))
}

/// Read text payloads, skipping control frames (Ping/Pong/Close) until the
/// deadline expires. Returns the first text frame parsed as `SignalingMessage`.
async fn recv_signaling(
    ws: &mut WsClient,
    deadline: tokio::time::Instant,
) -> Result<SignalingMessage> {
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(WebRtcError::Timeout("ws reply"));
        }
        let frame = tokio::time::timeout(remaining, ws.next())
            .await
            .map_err(|_| WebRtcError::Timeout("ws reply"))?
            .ok_or_else(|| WebRtcError::Signaling("ws closed".into()))?
            .map_err(|e| WebRtcError::Signaling(format!("ws recv: {e}")))?;
        if frame.is_ping() || frame.is_pong() {
            continue;
        }
        if frame.is_close() {
            return Err(WebRtcError::Signaling("ws closed before reply".into()));
        }
        if !frame.is_text() {
            continue;
        }
        let text = frame
            .into_text()
            .map_err(|e| WebRtcError::Signaling(format!("ws text: {e}")))?;
        return serde_json::from_str(&text)
            .map_err(|e| WebRtcError::Signaling(format!("ws json: {e}")));
    }
}

async fn send_text(ws: &mut WsClient, msg: &SignalingMessage) -> Result<()> {
    let payload = serde_json::to_string(msg)
        .map_err(|e| WebRtcError::Signaling(format!("serialize {}: {e}", msg.msg_type)))?;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        payload.into(),
    ))
    .await
    .map_err(|e| WebRtcError::Signaling(format!("ws send: {e}")))?;
    Ok(())
}

#[async_trait::async_trait]
impl Signaler for WsSignaler {
    async fn send_offer(&self, offer: &Offer) -> Result<Answer> {
        let mut ws = connect_with_retry(&self.ws_url, &self.config).await?;
        send_text(
            &mut ws,
            &SignalingMessage {
                msg_type: "offer".into(),
                sdp: offer.0.clone(),
                ..Default::default()
            },
        )
        .await?;

        let deadline = tokio::time::Instant::now() + self.config.request_timeout;
        let parsed = recv_signaling(&mut ws, deadline).await?;
        if parsed.msg_type != "answer" {
            return Err(WebRtcError::Signaling(format!(
                "expected answer, got {}",
                parsed.msg_type
            )));
        }
        Ok(Answer {
            sdp: parsed.sdp,
            connection_id: if parsed.connection_id.is_empty() {
                None
            } else {
                Some(parsed.connection_id)
            },
        })
    }

    async fn send_answer(&self, answer: &Answer) -> Result<()> {
        let connection_id = answer.connection_id.clone().ok_or_else(|| {
            WebRtcError::Signaling("send_answer requires Answer::connection_id".into())
        })?;
        let mut ws = connect_with_retry(&self.ws_url, &self.config).await?;
        send_text(
            &mut ws,
            &SignalingMessage {
                msg_type: "answer".into(),
                sdp: answer.sdp.clone(),
                connection_id,
                candidate: String::new(),
                request_id: String::new(),
            },
        )
        .await?;
        // Wait briefly for the server's ack so callers can detect connection_id
        // typos / route-not-found early. Time-bounded; we don't require ack.
        let deadline = tokio::time::Instant::now() + self.config.request_timeout;
        match recv_signaling(&mut ws, deadline).await {
            Ok(reply) if reply.msg_type == "ack" => Ok(()),
            Ok(other) => Err(WebRtcError::Signaling(format!(
                "expected ack after answer, got {}",
                other.msg_type
            ))),
            Err(WebRtcError::Timeout(_)) | Err(WebRtcError::Signaling(_)) => {
                // Server may close the connection silently after applying the
                // answer (older servers, intermediaries). Treat as success.
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    async fn send_ice(&self, candidate: &IceCandidate) -> Result<()> {
        // candidate.0 is expected to be a JSON-encoded RTCIceCandidateInit;
        // the caller is responsible for scoping it via the Answer's
        // connection_id, which they supply here as a free-form string field.
        let mut ws = connect_with_retry(&self.ws_url, &self.config).await?;
        send_text(
            &mut ws,
            &SignalingMessage {
                msg_type: "ice-candidate".into(),
                candidate: candidate.0.clone(),
                ..Default::default()
            },
        )
        .await
    }
}

/// Convenience helper: send a trickle ICE candidate scoped to a specific
/// server-side `connection_id`. Not part of the `Signaler` trait because the
/// trait predates scoped candidates.
pub async fn send_ice_for(
    signaler: &WsSignaler,
    connection_id: &str,
    candidate_json: &str,
) -> Result<()> {
    let mut ws = connect_with_retry(&signaler.ws_url, &signaler.config).await?;
    send_text(
        &mut ws,
        &SignalingMessage {
            msg_type: "ice-candidate".into(),
            connection_id: connection_id.to_owned(),
            candidate: candidate_json.to_owned(),
            ..Default::default()
        },
    )
    .await
}
