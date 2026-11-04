//! Experimental ordered, unpaced Vapi session for application-protocol gateways.
//!
//! Unlike the RTP-facing adapter this API does not add a media clock or split
//! events from audio. One owner consumes frames in WebSocket arrival order.
//! Provider speech-stop is still not a guaranteed audio-completion barrier.
//! This type does not register an Orchestrator connection; its host owns teardown.
//! Call `close` explicitly: dropping the value cannot await provider termination.
//! An open/handshake failure after call creation does not guarantee provider-side
//! cleanup. Hosts must use bounded provider call durations and reconcile ambiguous
//! creation outcomes; do not retry creation automatically.

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use crate::client::{connect_websocket, VapiHttpClient, VapiSocket};
use crate::types::{AddedMessage, VapiCommand};
use crate::{Result, VapiCallOptions, VapiConfig, VapiError, VapiEvent};

/// Items retain wire order. No completion events are synthesized.
pub enum RawVapiItem {
    Audio(Bytes),
    Event(VapiEvent),
    Closed,
}

impl std::fmt::Debug for RawVapiItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Audio(data) => f.debug_tuple("AudioBytes").field(&data.len()).finish(),
            Self::Event(event) => f.debug_tuple("Event").field(event).finish(),
            Self::Closed => f.write_str("Closed"),
        }
    }
}

pub struct RawVapiSession {
    socket: VapiSocket,
    call_id: String,
    config: VapiConfig,
}

impl RawVapiSession {
    /// Creates exactly once; ambiguous HTTP failures are not retried.
    pub async fn open(mut config: VapiConfig, options: VapiCallOptions) -> Result<Self> {
        if let Some(key) = options.api_key.clone() {
            config.api_key = Some(key);
        }
        let http = VapiHttpClient::new(&config)?;
        let call = http.create_call(&config, &options).await?;
        let host = call
            .websocket_url
            .host_str()
            .ok_or(VapiError::WebSocketSetup)?;
        let provider_host = host == "vapi.ai" || host.ends_with(".vapi.ai");
        let local_test =
            config.allow_insecure_transport && matches!(host, "localhost" | "127.0.0.1" | "::1");
        if !provider_host && !local_test {
            return Err(VapiError::InvalidResponse("unapproved WebSocket host"));
        }
        let socket = connect_websocket(&config, &call.websocket_url).await?;
        Ok(Self {
            socket,
            call_id: call.id,
            config,
        })
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub async fn next(&mut self) -> Result<RawVapiItem> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Binary(bytes))) => return Ok(RawVapiItem::Audio(bytes)),
                Some(Ok(Message::Text(text))) => {
                    return Ok(RawVapiItem::Event(VapiEvent::parse(&text)))
                }
                Some(Ok(Message::Close(_))) | None => return Ok(RawVapiItem::Closed),
                Some(Ok(Message::Ping(_))) => {
                    tokio::time::timeout(self.config.websocket_io_timeout, self.socket.flush())
                        .await
                        .map_err(|_| VapiError::WebSocketTimeout)?
                        .map_err(|_| VapiError::WebSocketIo)?;
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => return Err(VapiError::WebSocketIo),
            }
        }
    }

    pub async fn send_audio(&mut self, bytes: Bytes) -> Result<()> {
        if bytes.len() > self.config.max_message_bytes {
            return Err(VapiError::ControlMessageTooLarge);
        }
        self.send(Message::Binary(bytes)).await
    }

    pub async fn add_user_message(&mut self, content: String) -> Result<()> {
        let command = VapiCommand::AddMessage {
            message: AddedMessage {
                role: "user".into(),
                content,
            },
            trigger_response_enabled: true,
        };
        self.command(command).await
    }

    async fn command(&mut self, command: VapiCommand) -> Result<()> {
        let text = serde_json::to_string(&command).map_err(|_| VapiError::WebSocketIo)?;
        if text.len() > self.config.max_message_bytes {
            return Err(VapiError::ControlMessageTooLarge);
        }
        self.send(Message::Text(text.into())).await
    }

    async fn send(&mut self, message: Message) -> Result<()> {
        tokio::time::timeout(self.config.websocket_io_timeout, self.socket.send(message))
            .await
            .map_err(|_| VapiError::WebSocketTimeout)?
            .map_err(|_| VapiError::WebSocketIo)
    }

    pub async fn close(&mut self) -> Result<()> {
        let end = self
            .send(Message::Text(r#"{"type":"end-call"}"#.into()))
            .await;
        let close = tokio::time::timeout(
            self.config.graceful_shutdown_timeout,
            self.socket.close(None),
        )
        .await
        .map_err(|_| VapiError::WebSocketTimeout)?
        .map_err(|_| VapiError::WebSocketIo);
        end.and(close)
    }
}
