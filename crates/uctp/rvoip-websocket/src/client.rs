//! `UctpWsClient` — dials a UCTP-over-WebSocket server.
//!
//! Used by tests, the demo `uctp_agent_ws` binary, and indirectly by
//! `UctpWsAdapter::originate` when configured with a `client_url`.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use rvoip_uctp::envelope::UctpEnvelope;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};
use url::Url;

use crate::errors::{Result, UctpWsError};

pub struct UctpWsClient {
    out_tx: mpsc::Sender<UctpEnvelope>,
    in_rx: parking_lot::Mutex<Option<mpsc::Receiver<UctpEnvelope>>>,
}

impl UctpWsClient {
    pub async fn connect(url: &Url) -> Result<Arc<Self>> {
        let (ws, _resp) = tokio_tungstenite::connect_async(url.as_str()).await?;
        let (sink, stream) = ws.split();
        Ok(Self::spawn_pumps(sink, stream))
    }

    /// Dial a `wss://` URL, pinning the given `rustls::ClientConfig` for
    /// certificate verification. Use [`rvoip_uctp::substrate::tls::dev_client_config_trusting`]
    /// to build a config that trusts a single self-signed cert (test /
    /// dev only).
    #[cfg(feature = "wss")]
    pub async fn connect_with_tls(
        url: &Url,
        tls: Arc<rustls::ClientConfig>,
    ) -> Result<Arc<Self>> {
        let connector = tokio_tungstenite::Connector::Rustls(tls);
        let (ws, _resp) = tokio_tungstenite::connect_async_tls_with_config(
            url.as_str(),
            None,
            false,
            Some(connector),
        )
        .await?;
        let (sink, stream) = ws.split();
        Ok(Self::spawn_pumps(sink, stream))
    }

    fn spawn_pumps<S>(
        mut sink: futures::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<S>,
            Message,
        >,
        mut stream: futures::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<S>,
        >,
    ) -> Arc<Self>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(256);
        let (in_tx, in_rx) = mpsc::channel::<UctpEnvelope>(256);

        // Write pump.
        tokio::spawn(async move {
            while let Some(env) = out_rx.recv().await {
                let text = match serde_json::to_string(&env) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(error = %e, "rvoip-websocket-client: encode failed");
                        continue;
                    }
                };
                if let Err(e) = sink.send(Message::Text(text.into())).await {
                    warn!(error = %e, "rvoip-websocket-client: write error");
                    return;
                }
            }
            debug!("rvoip-websocket-client: write pump exiting");
        });

        // Read pump.
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<UctpEnvelope>(&text) {
                            Ok(env) => {
                                if in_tx.send(env).await.is_err() {
                                    return;
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "rvoip-websocket-client: malformed envelope");
                            }
                        }
                    }
                    Ok(Message::Close(_)) => return,
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, "rvoip-websocket-client: read error");
                        return;
                    }
                }
            }
            debug!("rvoip-websocket-client: read pump exiting");
        });

        Arc::new(Self {
            out_tx,
            in_rx: parking_lot::Mutex::new(Some(in_rx)),
        })
    }

    pub async fn send(&self, env: UctpEnvelope) -> Result<()> {
        self.out_tx
            .send(env)
            .await
            .map_err(|_| UctpWsError::Shutdown)
    }

    pub fn take_inbound(&self) -> Option<mpsc::Receiver<UctpEnvelope>> {
        self.in_rx.lock().take()
    }
}
