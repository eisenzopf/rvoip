//! WebSocket JSON SDP signaler (feature `signaling-ws`).

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::ids::ConnectionId;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

use crate::adapter::WebRtcAdapter;
use crate::errors::{Result, WebRtcError};

#[derive(Debug, Deserialize, Serialize)]
pub struct SignalingMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub sdp: String,
    /// Routes `{type:"answer"}` to an outbound originate connection.
    #[serde(default, rename = "connection_id")]
    pub connection_id: String,
    /// Trickle ICE candidate JSON (not handled in v1 — surfaces capability gap).
    #[serde(default)]
    pub candidate: String,
}

/// Accept WebSocket connections and exchange `{type, sdp, connection_id?}` JSON messages.
pub async fn serve(bind: &str, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("bind {bind}: {e}")))?;
    serve_listener(listener, adapter).await
}

/// Serve on an already-bound listener (integration tests).
pub async fn serve_listener(listener: TcpListener, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        let adapter = Arc::clone(&adapter);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, adapter).await {
                tracing::warn!("ws signaling connection error: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    adapter: Arc<WebRtcAdapter>,
) -> Result<()> {
    let ws = accept_async(stream)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
    let (mut write, mut read) = ws.split();

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        if !msg.is_text() {
            continue;
        }
        let parsed: SignalingMessage = serde_json::from_str(msg.to_text().unwrap_or(""))
            .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;

        match parsed.msg_type.as_str() {
            "offer" => {
                let conn_id = adapter.apply_remote_offer(&parsed.sdp).await?;
                let answer = adapter.local_sdp(&conn_id)?;
                let out = SignalingMessage {
                    msg_type: "answer".into(),
                    sdp: answer,
                    connection_id: conn_id.to_string(),
                    candidate: String::new(),
                };
                write
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::to_string(&out).unwrap().into(),
                    ))
                    .await
                    .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
            }
            "answer" => {
                if parsed.connection_id.is_empty() {
                    return Err(WebRtcError::Signaling(
                        "answer requires connection_id".into(),
                    ));
                }
                let conn_id = ConnectionId::from_string(parsed.connection_id.clone());
                adapter
                    .accept_remote_answer(conn_id, &parsed.sdp)
                    .await?;
                let out = SignalingMessage {
                    msg_type: "ack".into(),
                    sdp: String::new(),
                    connection_id: parsed.connection_id,
                    candidate: String::new(),
                };
                write
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::to_string(&out).unwrap().into(),
                    ))
                    .await
                    .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
            }
            "ice-candidate" => {
                return Err(WebRtcError::NotImplemented(
                    "trickle ICE over WebSocket signaling (v1 uses full SDP gather)".into(),
                ));
            }
            "bye" => {
                if !parsed.connection_id.is_empty() {
                    let conn_id = ConnectionId::from_string(parsed.connection_id);
                    let _ = adapter
                        .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                        .await;
                }
                break;
            }
            other => {
                return Err(WebRtcError::Signaling(format!(
                    "unknown signaling message type: {other}"
                )));
            }
        }
    }

    Ok(())
}
