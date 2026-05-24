//! WebSocket JSON signaler for [`WebRtcClient`] → [`WebRtcServer`](crate::server::WebRtcServer).

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;

use crate::client::{Answer, IceCandidate, Offer, Signaler};
use crate::errors::{Result, WebRtcError};
use crate::signaling::websocket::SignalingMessage;

/// Connect to a `WebRtcServer` WebSocket signaling endpoint.
pub struct WsSignaler {
    ws_url: String,
}

impl WsSignaler {
    pub fn new(ws_url: impl Into<String>) -> Self {
        Self {
            ws_url: ws_url.into(),
        }
    }
}

#[async_trait::async_trait]
impl Signaler for WsSignaler {
    async fn send_offer(&self, offer: &Offer) -> Result<Answer> {
        let (mut ws, _) = connect_async(&self.ws_url)
            .await
            .map_err(|e| WebRtcError::Signaling(format!("ws connect {}: {e}", self.ws_url)))?;

        let out = SignalingMessage {
            msg_type: "offer".into(),
            sdp: offer.0.clone(),
            connection_id: String::new(),
            candidate: String::new(),
        };
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&out)
                .map_err(|e| WebRtcError::Signaling(format!("{e}")))?
                .into(),
        ))
        .await
        .map_err(|e| WebRtcError::Signaling(format!("ws send: {e}")))?;

        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            ws.next(),
        )
        .await
        .map_err(|_| WebRtcError::Timeout("ws answer"))?
        .ok_or_else(|| WebRtcError::Signaling("ws closed".into()))?
        .map_err(|e| WebRtcError::Signaling(format!("ws recv: {e}")))?;

        let text = reply
            .into_text()
            .map_err(|e| WebRtcError::Signaling(format!("ws text: {e}")))?;
        let parsed: SignalingMessage = serde_json::from_str(&text)
            .map_err(|e| WebRtcError::Signaling(format!("ws json: {e}")))?;
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

    async fn send_answer(&self, _answer: &Answer) -> Result<()> {
        Err(WebRtcError::NotImplemented("WsSignaler is offerer-only"))
    }

    async fn send_ice(&self, _candidate: &IceCandidate) -> Result<()> {
        // Full-SDP gathering — no trickle ICE in v1.
        Ok(())
    }
}
