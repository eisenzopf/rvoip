//! `UctpQuicClient` — dials a UCTP-over-QUIC server. Used by tests and
//! the v0 demo agent binaries.
//!
//! `ConnectionAdapter::originate` is **not** plumbed to this in v0
//! (returns `NotImplemented`); for the loopback test we drive the
//! client directly.

use std::net::SocketAddr;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::substrate::{envelope_reader, envelope_writer};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::errors::{Result, UctpQuicError};

pub struct UctpQuicClient {
    pub connection: quinn::Connection,
    out_tx: mpsc::Sender<UctpEnvelope>,
    in_rx: parking_lot::Mutex<Option<mpsc::Receiver<UctpEnvelope>>>,
}

impl UctpQuicClient {
    /// Dial the given server address.
    ///
    /// `client_config` MUST include ALPN `b"uctp/1"` in `alpn_protocols`.
    pub async fn connect(
        endpoint: &quinn::Endpoint,
        server: SocketAddr,
        server_name: &str,
        client_config: Arc<rustls::ClientConfig>,
    ) -> Result<Arc<Self>> {
        let mut tls = (*client_config).clone();
        if tls.alpn_protocols.is_empty() {
            tls.alpn_protocols = vec![b"uctp/1".to_vec()];
        }
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .map_err(|e| rvoip_uctp::errors::SubstrateError::Tls(rustls::Error::General(e.to_string())))?;
        let mut qc = quinn::ClientConfig::new(Arc::new(crypto));
        let mut t = quinn::TransportConfig::default();
        // Generous idle timeout so loopback tests don't flake.
        t.max_idle_timeout(Some(std::time::Duration::from_secs(30).try_into().unwrap()));
        qc.transport_config(Arc::new(t));

        let connecting = endpoint
            .connect_with(qc, server, server_name)
            .map_err(|e| rvoip_uctp::errors::SubstrateError::Tls(rustls::Error::General(e.to_string())))?;
        let conn = connecting
            .await
            .map_err(rvoip_uctp::errors::SubstrateError::Quinn)?;

        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(rvoip_uctp::errors::SubstrateError::Quinn)?;

        let mut reader = Box::pin(envelope_reader(recv));
        let mut writer = Box::pin(envelope_writer(send));

        let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(256);
        let (in_tx, in_rx) = mpsc::channel::<UctpEnvelope>(256);

        // Write pump.
        tokio::spawn(async move {
            while let Some(env) = out_rx.recv().await {
                if let Err(e) = writer.send(env).await {
                    warn!(error = %e, "rvoip-quic-client: write error");
                    return;
                }
            }
            debug!("rvoip-quic-client: write pump exiting");
        });

        // Read pump.
        tokio::spawn(async move {
            while let Some(item) = reader.next().await {
                match item {
                    Ok(env) => {
                        if in_tx.send(env).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "rvoip-quic-client: read error");
                        return;
                    }
                }
            }
            debug!("rvoip-quic-client: read pump exiting");
        });

        Ok(Arc::new(Self {
            connection: conn,
            out_tx,
            in_rx: parking_lot::Mutex::new(Some(in_rx)),
        }))
    }

    /// Send an envelope to the server. Backpressure: awaits naturally.
    pub async fn send(&self, env: UctpEnvelope) -> Result<()> {
        self.out_tx
            .send(env)
            .await
            .map_err(|_| UctpQuicError::Shutdown)
    }

    /// Take the inbound channel. Single-consumer; returns `None` on
    /// subsequent calls.
    pub fn take_inbound(&self) -> Option<mpsc::Receiver<UctpEnvelope>> {
        self.in_rx.lock().take()
    }
}
