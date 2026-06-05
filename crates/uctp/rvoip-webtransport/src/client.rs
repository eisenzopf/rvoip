//! `UctpWtClient` — dials a UCTP-over-WebTransport server.

use std::net::SocketAddr;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::substrate::{envelope_reader, envelope_writer};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use url::Url;

use crate::errors::{Result, UctpWtError};

pub struct UctpWtClient {
    pub session: web_transport_quinn::Session,
    out_tx: mpsc::Sender<UctpEnvelope>,
    in_rx: parking_lot::Mutex<Option<mpsc::Receiver<UctpEnvelope>>>,
}

impl UctpWtClient {
    /// Dial a WT URL like `https://127.0.0.1:4433/uctp`.
    ///
    /// `client_config` MUST include ALPN `b"h3"` in `alpn_protocols`.
    pub async fn connect(
        endpoint: &quinn::Endpoint,
        server: SocketAddr,
        url: &Url,
        client_config: Arc<rustls::ClientConfig>,
    ) -> Result<Arc<Self>> {
        let mut tls = (*client_config).clone();
        if tls.alpn_protocols.is_empty() {
            tls.alpn_protocols = vec![b"h3".to_vec()];
        }
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|e| {
            rvoip_uctp::errors::SubstrateError::Tls(rustls::Error::General(e.to_string()))
        })?;
        let mut qc = quinn::ClientConfig::new(Arc::new(crypto));
        let mut t = quinn::TransportConfig::default();
        t.max_idle_timeout(Some(std::time::Duration::from_secs(30).try_into().unwrap()));
        qc.transport_config(Arc::new(t));

        let server_name = url
            .host_str()
            .ok_or_else(|| UctpWtError::Session("URL has no host".into()))?;

        let connecting = endpoint
            .connect_with(qc, server, server_name)
            .map_err(|e| {
                rvoip_uctp::errors::SubstrateError::Tls(rustls::Error::General(e.to_string()))
            })?;
        let conn = connecting
            .await
            .map_err(rvoip_uctp::errors::SubstrateError::Quinn)?;

        // Upgrade to a WT session.
        let session = web_transport_quinn::Session::connect(conn, url.clone())
            .await
            .map_err(|e| UctpWtError::Session(format!("{}", e)))?;

        let (send, recv) = session
            .open_bi()
            .await
            .map_err(|e| UctpWtError::Session(format!("open_bi: {}", e)))?;

        let mut reader = Box::pin(envelope_reader(recv));
        let mut writer = Box::pin(envelope_writer(send));

        let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(256);
        let (in_tx, in_rx) = mpsc::channel::<UctpEnvelope>(256);

        tokio::spawn(async move {
            while let Some(env) = out_rx.recv().await {
                if let Err(e) = writer.send(env).await {
                    warn!(error = %e, "rvoip-wt-client: write error");
                    return;
                }
            }
            debug!("rvoip-wt-client: write pump exiting");
        });

        tokio::spawn(async move {
            while let Some(item) = reader.next().await {
                match item {
                    Ok(env) => {
                        if in_tx.send(env).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "rvoip-wt-client: read error");
                        return;
                    }
                }
            }
            debug!("rvoip-wt-client: read pump exiting");
        });

        Ok(Arc::new(Self {
            session,
            out_tx,
            in_rx: parking_lot::Mutex::new(Some(in_rx)),
        }))
    }

    pub async fn send(&self, env: UctpEnvelope) -> Result<()> {
        self.out_tx
            .send(env)
            .await
            .map_err(|_| UctpWtError::Shutdown)
    }

    pub fn take_inbound(&self) -> Option<mpsc::Receiver<UctpEnvelope>> {
        self.in_rx.lock().take()
    }
}
