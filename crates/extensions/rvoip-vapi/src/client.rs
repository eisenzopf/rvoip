//! Vapi REST call creation and authenticated WebSocket setup.

use std::time::Duration;

use bytes::BytesMut;
use reqwest::Client;
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;
use zeroize::Zeroizing;

use crate::config::VapiConfig;
use crate::error::{Result, VapiError};
use crate::types::VapiCallOptions;

pub(crate) type VapiSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone)]
pub(crate) struct VapiHttpClient {
    client: Client,
}

impl VapiHttpClient {
    pub(crate) fn new(config: &VapiConfig) -> Result<Self> {
        config.validate()?;
        let client = Client::builder()
            .timeout(config.http_timeout)
            .https_only(!config.allow_insecure_transport)
            .build()
            .map_err(|_| VapiError::InvalidConfiguration("HTTP client setup failed"))?;
        Ok(Self { client })
    }

    pub(crate) async fn create_call(
        &self,
        config: &VapiConfig,
        options: &VapiCallOptions,
    ) -> Result<CreatedCall> {
        options.validate()?;
        let endpoint = config
            .api_base
            .join("call")
            .map_err(|_| VapiError::InvalidConfiguration("the API base cannot join /call"))?;
        let operation = async {
            let mut response = self
                .client
                .post(endpoint)
                .bearer_auth(config.api_key.expose_secret())
                .json(&options.create_call_payload())
                .send()
                .await
                .map_err(|_| VapiError::Http)?;
            let status = response.status();
            if !status.is_success() {
                return Err(VapiError::HttpStatus(status.as_u16()));
            }
            if response
                .content_length()
                .is_some_and(|length| length > config.max_message_bytes as u64)
            {
                return Err(VapiError::InvalidResponse(
                    "the response exceeded the configured size limit",
                ));
            }
            let mut body = BytesMut::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| VapiError::Http)? {
                if body
                    .len()
                    .checked_add(chunk.len())
                    .is_none_or(|length| length > config.max_message_bytes)
                {
                    return Err(VapiError::InvalidResponse(
                        "the response exceeded the configured size limit",
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            parse_created_call(&body, config)
        };
        tokio::time::timeout(config.http_timeout, operation)
            .await
            .map_err(|_| VapiError::HttpTimeout)?
    }
}

#[derive(Deserialize)]
struct CallResponse {
    id: String,
    transport: CallTransport,
}

#[derive(Deserialize)]
struct CallTransport {
    #[serde(rename = "websocketCallUrl")]
    websocket_call_url: String,
}

pub(crate) struct CreatedCall {
    pub(crate) id: String,
    pub(crate) websocket_url: Url,
}

fn parse_created_call(body: &[u8], config: &VapiConfig) -> Result<CreatedCall> {
    let parsed: CallResponse = serde_json::from_slice(body)
        .map_err(|_| VapiError::InvalidResponse("the response was not a Vapi call object"))?;
    if parsed.id.trim().is_empty() || parsed.id.chars().any(char::is_control) {
        return Err(VapiError::InvalidResponse("the call ID was invalid"));
    }
    let websocket_url = Url::parse(&parsed.transport.websocket_call_url)
        .map_err(|_| VapiError::InvalidResponse("the WebSocket URL was invalid"))?;
    if !config.permits_websocket_url(&websocket_url) {
        return Err(VapiError::InvalidResponse(
            "the WebSocket URL did not use an allowed secure transport",
        ));
    }
    Ok(CreatedCall {
        id: parsed.id,
        websocket_url,
    })
}

pub(crate) async fn connect_websocket(
    config: &VapiConfig,
    websocket_url: &Url,
) -> Result<VapiSocket> {
    if !config.permits_websocket_url(websocket_url) {
        return Err(VapiError::WebSocketSetup);
    }
    let mut request = websocket_url
        .as_str()
        .into_client_request()
        .map_err(|_| VapiError::WebSocketSetup)?;
    let bearer = Zeroizing::new(format!("Bearer {}", config.api_key.expose_secret()));
    let mut authorization =
        HeaderValue::from_str(&bearer).map_err(|_| VapiError::WebSocketSetup)?;
    authorization.set_sensitive(true);
    request.headers_mut().insert(AUTHORIZATION, authorization);

    let socket_config = WebSocketConfig::default()
        .write_buffer_size(0)
        .max_write_buffer_size(config.max_message_bytes.saturating_add(1))
        .max_message_size(Some(config.max_message_bytes))
        .max_frame_size(Some(config.max_message_bytes));
    let (socket, _) = tokio::time::timeout(
        config.websocket_timeout,
        tokio_tungstenite::connect_async_with_config(request, Some(socket_config), true),
    )
    .await
    .map_err(|_| VapiError::WebSocketTimeout)?
    .map_err(|_| VapiError::WebSocketSetup)?;
    Ok(socket)
}

pub(crate) async fn send_with_timeout<S>(
    timeout: Duration,
    socket: &mut WebSocketStream<S>,
    message: tokio_tungstenite::tungstenite::Message,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use futures::SinkExt;
    tokio::time::timeout(timeout, socket.send(message))
        .await
        .map_err(|_| VapiError::WebSocketTimeout)?
        .map_err(|_| VapiError::WebSocketIo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VapiApiKey;

    fn test_config() -> VapiConfig {
        VapiConfig::new(VapiApiKey::new("test-key").unwrap())
            .with_api_base(Url::parse("http://127.0.0.1:3000/").unwrap())
            .with_loopback_test_transport()
    }

    #[test]
    fn parses_exact_returned_websocket_url() {
        let config = test_config();
        let call = parse_created_call(
            br#"{"id":"call-1","transport":{"websocketCallUrl":"ws://127.0.0.1:9000/exact?token=opaque"}}"#,
            &config,
        )
        .unwrap();
        assert_eq!(call.id, "call-1");
        assert_eq!(
            call.websocket_url.as_str(),
            "ws://127.0.0.1:9000/exact?token=opaque"
        );
    }

    #[test]
    fn rejects_insecure_non_loopback_return_url() {
        let config = test_config();
        let result = parse_created_call(
            br#"{"id":"call-1","transport":{"websocketCallUrl":"ws://example.com/socket"}}"#,
            &config,
        );
        assert!(matches!(result, Err(VapiError::InvalidResponse(_))));
    }

    #[test]
    fn rejects_malformed_or_incomplete_call_responses() {
        let config = test_config();
        for body in [
            br#"{"id":"call-1","transport":{}}"#.as_slice(),
            br#"{"id":"","transport":{"websocketCallUrl":"ws://127.0.0.1:9000/socket"}}"#,
            br#"{"not":"a-call"}"#,
            b"{bad-json",
        ] {
            assert!(matches!(
                parse_created_call(body, &config),
                Err(VapiError::InvalidResponse(_))
            ));
        }
    }
}
