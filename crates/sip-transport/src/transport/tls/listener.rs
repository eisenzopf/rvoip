use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener as TokioTcpListener;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};

/// TLS listener that accepts TCP connections and performs TLS handshakes.
pub struct TlsListener {
    /// The underlying TCP listener
    tcp_listener: TokioTcpListener,
    /// The TLS acceptor for performing server-side handshakes
    tls_acceptor: tokio_rustls::TlsAcceptor,
}

impl TlsListener {
    /// Binds a TLS listener to the specified address with the given TLS acceptor
    pub async fn bind(addr: SocketAddr, tls_acceptor: tokio_rustls::TlsAcceptor) -> Result<Self> {
        let tcp_listener = TokioTcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;

        let local_addr = tcp_listener.local_addr()
            .map_err(|e| Error::LocalAddrFailed(e))?;

        info!("TLS listener bound to {}", local_addr);

        Ok(Self {
            tcp_listener,
            tls_acceptor,
        })
    }

    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.tcp_listener.local_addr().map_err(|e| Error::LocalAddrFailed(e))
    }

    /// Accepts a new TLS connection: TCP accept + TLS handshake.
    ///
    /// Returns the TLS server stream and the peer address on success.
    pub async fn accept(&self) -> Result<(tokio_rustls::server::TlsStream<tokio::net::TcpStream>, SocketAddr)> {
        let (tcp_stream, peer_addr) = self.tcp_listener.accept()
            .await
            .map_err(|e| Error::ReceiveFailed(e))?;

        debug!("Accepted TCP connection from {}, performing TLS handshake", peer_addr);

        if let Err(e) = tcp_stream.set_nodelay(true) {
            error!("Failed to set TCP_NODELAY: {}", e);
        }

        let tls_stream = self.tls_acceptor.accept(tcp_stream)
            .await
            .map_err(|e| Error::TlsHandshakeFailed(format!("Server handshake with {}: {}", peer_addr, e)))?;

        debug!("TLS handshake completed with {}", peer_addr);

        Ok((tls_stream, peer_addr))
    }
}
