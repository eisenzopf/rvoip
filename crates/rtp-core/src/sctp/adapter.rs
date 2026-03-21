//! Production-grade SCTP adapter backed by `webrtc-sctp`.
//!
//! This module wraps the [`webrtc_sctp`] crate (full RFC 4960 + RFC 8831)
//! behind an API that integrates with our existing [`DtlsConnection`].
//!
//! The key bridge piece is [`DtlsConnBridge`], which implements
//! [`webrtc_util::conn::Conn`] on top of a `DtlsConnection`, allowing
//! `webrtc-sctp` to read/write SCTP packets over our DTLS transport.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{Mutex, Notify};
use webrtc_sctp::association::{Association, Config as SctpConfig};
use webrtc_sctp::chunk::chunk_payload_data::PayloadProtocolIdentifier;
use webrtc_sctp::stream::Stream as SctpStream;

use crate::dtls::DtlsConnection;
use crate::error::Error;

// ---------------------------------------------------------------------------
// DtlsConnBridge -- implements webrtc_util::conn::Conn for our DtlsConnection
// ---------------------------------------------------------------------------

/// Bridges our [`DtlsConnection`] to the [`webrtc_util::conn::Conn`] trait
/// required by `webrtc-sctp`.
///
/// `webrtc-sctp` drives the transport through `send` (write SCTP packets)
/// and `recv` (read SCTP packets). We translate those into
/// `DtlsConnection::send_application_data` and a polling loop over
/// `DtlsConnection::read_application_data`.
pub struct DtlsConnBridge {
    /// The underlying DTLS connection, behind a Mutex because `Conn`
    /// methods take `&self` while `DtlsConnection` requires `&mut self`.
    dtls: Mutex<DtlsConnection>,

    /// Notification channel used to wake `recv` when new application
    /// data may have arrived from the DTLS layer.
    data_notify: Notify,

    /// Whether the bridge has been closed.
    closed: std::sync::atomic::AtomicBool,
}

impl DtlsConnBridge {
    /// Create a new bridge wrapping an established DTLS connection.
    pub fn new(dtls: DtlsConnection) -> Self {
        Self {
            dtls: Mutex::new(dtls),
            data_notify: Notify::new(),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Notify the bridge that new DTLS application data may be available.
    ///
    /// Call this after feeding raw DTLS packets into the connection so
    /// that any blocked `recv` call can wake up and check for data.
    pub fn notify_data_available(&self) {
        self.data_notify.notify_waiters();
    }
}

#[async_trait]
impl webrtc_util::conn::Conn for DtlsConnBridge {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        // The DTLS connection is already established; this is a no-op.
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        loop {
            if self.closed.load(std::sync::atomic::Ordering::Acquire) {
                return Err(webrtc_util::Error::ErrUseClosedNetworkConn);
            }

            // Try to read buffered application data from DTLS.
            {
                let mut dtls = self.dtls.lock().await;
                if let Some(data) = dtls.read_application_data() {
                    if data.len() > buf.len() {
                        return Err(webrtc_util::Error::Other(format!(
                            "DTLS application data ({} bytes) exceeds recv buffer ({} bytes)",
                            data.len(),
                            buf.len(),
                        )));
                    }
                    let len = data.len();
                    buf[..len].copy_from_slice(&data[..len]);
                    return Ok(len);
                }
            }

            // No data yet -- wait for a notification or a short timeout so we
            // can re-check. The timeout prevents permanent stalls if a
            // notification is missed.
            tokio::select! {
                _ = self.data_notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
            }
        }
    }

    async fn recv_from(
        &self,
        buf: &mut [u8],
    ) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        // SCTP-over-DTLS is point-to-point; return the DTLS peer's remote address
        // if available, otherwise fall back to unspecified.
        let addr = {
            let dtls = self.dtls.lock().await;
            dtls.remote_addr()
                .unwrap_or_else(|| SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0))
        };
        Ok((n, addr))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(webrtc_util::Error::ErrUseClosedNetworkConn);
        }

        let mut dtls = self.dtls.lock().await;
        dtls.send_application_data(buf)
            .await
            .map_err(|e| webrtc_util::Error::Other(e.to_string()))?;
        Ok(buf.len())
    }

    async fn send_to(
        &self,
        buf: &[u8],
        _target: SocketAddr,
    ) -> webrtc_util::Result<usize> {
        // Point-to-point; ignore the target address.
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        // The DTLS connection doesn't expose the underlying transport's local
        // address through its public API. Return unspecified -- callers using
        // SCTP-over-DTLS operate in a point-to-point mode where the local
        // socket address is not meaningful at this layer.
        Ok(SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            0,
        ))
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        // Return the actual DTLS peer address if available.
        self.dtls
            .try_lock()
            .ok()
            .and_then(|dtls| dtls.remote_addr())
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.data_notify.notify_waiters();
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

// ---------------------------------------------------------------------------
// SctpAssociationAdapter
// ---------------------------------------------------------------------------

/// Default maximum receive buffer size (256 KiB).
const DEFAULT_MAX_RECEIVE_BUFFER_SIZE: u32 = 256 * 1024;

/// Default maximum SCTP message size (256 KiB).
const DEFAULT_MAX_MESSAGE_SIZE: u32 = 256 * 1024;

/// Production-grade SCTP association backed by `webrtc-sctp`.
///
/// This adapter owns a [`webrtc_sctp::association::Association`] and
/// exposes a simplified API for opening/accepting streams and
/// performing graceful shutdown.
pub struct SctpAssociationAdapter {
    /// The inner webrtc-sctp association.
    association: Association,
    /// Local SCTP port used during setup.
    local_port: u16,
    /// Remote SCTP port used during setup.
    remote_port: u16,
}

impl SctpAssociationAdapter {
    /// Initiate an SCTP association as the client (DTLS client role).
    ///
    /// `dtls` must already be in the `Connected` state. The caller
    /// provides the SCTP port pair (typically both 5000 for WebRTC).
    pub async fn connect(
        dtls: DtlsConnection,
        local_port: u16,
        remote_port: u16,
    ) -> Result<Self, Error> {
        let bridge = Arc::new(DtlsConnBridge::new(dtls));

        let config = SctpConfig {
            net_conn: bridge as Arc<dyn webrtc_util::conn::Conn + Send + Sync>,
            max_receive_buffer_size: DEFAULT_MAX_RECEIVE_BUFFER_SIZE,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            name: "rvoip-sctp-client".to_string(),
            local_port,
            remote_port,
        };

        let association = Association::client(config)
            .await
            .map_err(|e| Error::SctpError(format!("SCTP client handshake failed: {e}")))?;

        tracing::debug!(local_port, remote_port, "SCTP association established (client)");

        Ok(Self {
            association,
            local_port,
            remote_port,
        })
    }

    /// Accept an incoming SCTP association as the server (DTLS server role).
    ///
    /// `dtls` must already be in the `Connected` state.
    pub async fn accept(
        dtls: DtlsConnection,
        local_port: u16,
        remote_port: u16,
    ) -> Result<Self, Error> {
        let bridge = Arc::new(DtlsConnBridge::new(dtls));

        let config = SctpConfig {
            net_conn: bridge as Arc<dyn webrtc_util::conn::Conn + Send + Sync>,
            max_receive_buffer_size: DEFAULT_MAX_RECEIVE_BUFFER_SIZE,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            name: "rvoip-sctp-server".to_string(),
            local_port,
            remote_port,
        };

        let association = Association::server(config)
            .await
            .map_err(|e| Error::SctpError(format!("SCTP server handshake failed: {e}")))?;

        tracing::debug!(local_port, remote_port, "SCTP association established (server)");

        Ok(Self {
            association,
            local_port,
            remote_port,
        })
    }

    /// Open a new SCTP stream (data channel) on this association.
    ///
    /// `stream_id` is the SCTP stream identifier (even for the DTLS
    /// client role, odd for the server role per WebRTC convention).
    pub async fn open_stream(&self, stream_id: u16) -> Result<SctpStreamAdapter, Error> {
        let stream = self
            .association
            .open_stream(stream_id, PayloadProtocolIdentifier::String)
            .await
            .map_err(|e| Error::SctpError(format!("Failed to open SCTP stream {stream_id}: {e}")))?;

        tracing::debug!(stream_id, "SCTP stream opened");
        Ok(SctpStreamAdapter { inner: stream })
    }

    /// Wait for and accept an incoming SCTP stream from the remote peer.
    ///
    /// Returns `None` if the association has been closed.
    pub async fn accept_stream(&self) -> Result<SctpStreamAdapter, Error> {
        let stream = self
            .association
            .accept_stream()
            .await
            .ok_or_else(|| Error::SctpError("No incoming SCTP stream (association closed)".to_string()))?;

        let stream_id = stream.stream_identifier();
        tracing::debug!(stream_id, "SCTP stream accepted");
        Ok(SctpStreamAdapter { inner: stream })
    }

    /// Gracefully shut down the SCTP association.
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.association
            .shutdown()
            .await
            .map_err(|e| Error::SctpError(format!("SCTP shutdown failed: {e}")))?;
        tracing::debug!("SCTP association shut down");
        Ok(())
    }

    /// Close the SCTP association and release resources.
    pub async fn close(&self) -> Result<(), Error> {
        self.association
            .close()
            .await
            .map_err(|e| Error::SctpError(format!("SCTP close failed: {e}")))?;
        tracing::debug!("SCTP association closed");
        Ok(())
    }

    /// The local SCTP port.
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// The remote SCTP port.
    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }
}

// ---------------------------------------------------------------------------
// SctpStreamAdapter
// ---------------------------------------------------------------------------

/// Wrapper around a single `webrtc-sctp` stream (data channel transport).
pub struct SctpStreamAdapter {
    inner: Arc<SctpStream>,
}

impl SctpStreamAdapter {
    /// The SCTP stream identifier.
    pub fn stream_id(&self) -> u16 {
        self.inner.stream_identifier()
    }

    /// Send data on this stream using the default payload protocol identifier.
    pub async fn send(&self, data: &[u8]) -> Result<usize, Error> {
        let payload = Bytes::copy_from_slice(data);
        let n = self
            .inner
            .write(&payload)
            .await
            .map_err(|e| Error::SctpError(format!("SCTP stream write failed: {e}")))?;
        Ok(n)
    }

    /// Send data with an explicit payload protocol identifier.
    pub async fn send_with_ppi(
        &self,
        data: &[u8],
        ppi: PayloadProtocolIdentifier,
    ) -> Result<usize, Error> {
        let payload = Bytes::copy_from_slice(data);
        let n = self
            .inner
            .write_sctp(&payload, ppi)
            .await
            .map_err(|e| Error::SctpError(format!("SCTP stream write_sctp failed: {e}")))?;
        Ok(n)
    }

    /// Receive data from this stream.
    ///
    /// Returns the payload bytes. Returns an empty `Vec` if the stream
    /// has been shut down or reset by the remote peer.
    pub async fn recv(&self) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; 65536];
        let n = self
            .inner
            .read(&mut buf)
            .await
            .map_err(|e| Error::SctpError(format!("SCTP stream read failed: {e}")))?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Receive data along with the payload protocol identifier.
    pub async fn recv_sctp(&self) -> Result<(Vec<u8>, PayloadProtocolIdentifier), Error> {
        let mut buf = vec![0u8; 65536];
        let (n, ppi) = self
            .inner
            .read_sctp(&mut buf)
            .await
            .map_err(|e| Error::SctpError(format!("SCTP stream read_sctp failed: {e}")))?;
        buf.truncate(n);
        Ok((buf, ppi))
    }

    /// Shut down both read and write halves of this stream.
    pub async fn close(&self) -> Result<(), Error> {
        self.inner
            .shutdown(std::net::Shutdown::Both)
            .await
            .map_err(|e| Error::SctpError(format!("SCTP stream shutdown failed: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtls::{DtlsConfig, DtlsConnection};
    use webrtc_util::conn::Conn;

    #[test]
    fn test_dtls_conn_bridge_local_addr() {
        let dtls = DtlsConnection::new(DtlsConfig::default());
        let bridge = DtlsConnBridge::new(dtls);
        let addr = bridge.local_addr();
        assert!(addr.is_ok());
    }

    #[test]
    fn test_dtls_conn_bridge_remote_addr_is_none() {
        let dtls = DtlsConnection::new(DtlsConfig::default());
        let bridge = DtlsConnBridge::new(dtls);
        assert!(bridge.remote_addr().is_none());
    }

    #[tokio::test]
    async fn test_dtls_conn_bridge_send_requires_connected_dtls() {
        let dtls = DtlsConnection::new(DtlsConfig::default());
        let bridge = DtlsConnBridge::new(dtls);
        // The DTLS connection is in New state, so send should fail.
        let result = bridge.send(b"test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_dtls_conn_bridge_close() {
        let dtls = DtlsConnection::new(DtlsConfig::default());
        let bridge = DtlsConnBridge::new(dtls);
        let result = webrtc_util::conn::Conn::close(&bridge).await;
        assert!(result.is_ok());
        assert!(bridge.closed.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test]
    async fn test_dtls_conn_bridge_recv_after_close() {
        let dtls = DtlsConnection::new(DtlsConfig::default());
        let bridge = DtlsConnBridge::new(dtls);
        webrtc_util::conn::Conn::close(&bridge).await.ok();
        let mut buf = [0u8; 64];
        let result = bridge.recv(&mut buf).await;
        assert!(result.is_err());
    }
}
