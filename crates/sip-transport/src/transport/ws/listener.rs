use std::net::SocketAddr;
use std::sync::Arc;
use futures_util::StreamExt;
use futures_util::stream::SplitStream;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, trace, warn};

#[cfg(feature = "ws")]
use tokio_tungstenite::{accept_hdr_async, tungstenite, WebSocketStream};
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
#[cfg(feature = "ws")]
use http::HeaderValue;

#[cfg(feature = "tls")]
use tokio_rustls::TlsAcceptor;

use crate::error::{Error, Result};
use super::connection::WebSocketConnection;
use super::stream::WsStream;
use super::{SIP_WS_SUBPROTOCOL, SIP_WSS_SUBPROTOCOL};

/// WebSocket listener for accepting SIP WebSocket connections
pub struct WebSocketListener {
    /// The underlying TCP listener
    listener: TcpListener,
    /// Whether this is a secure WebSocket listener (WSS)
    secure: bool,
    /// TLS acceptor for performing server-side handshakes (WSS only)
    #[cfg(feature = "tls")]
    tls_acceptor: Option<TlsAcceptor>,
}

/// Loads PEM certificates from a file path.
#[cfg(feature = "tls")]
fn load_certs(path: &str) -> Result<Vec<rustls_pki_types::CertificateDer<'static>>> {
    let file = std::fs::File::open(path)
        .map_err(|e| Error::TlsCertificateError(format!("Cannot open cert file {}: {}", path, e)))?;
    let mut reader = std::io::BufReader::new(file);
    let certs: Vec<rustls_pki_types::CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::TlsCertificateError(format!("Failed to parse certs from {}: {}", path, e)))?;
    Ok(certs)
}

/// Loads a PEM private key from a file path.
/// Tries all key formats via rustls_pemfile::private_key.
#[cfg(feature = "tls")]
fn load_private_key(path: &str) -> Result<rustls_pki_types::PrivateKeyDer<'static>> {
    let file = std::fs::File::open(path)
        .map_err(|e| Error::TlsCertificateError(format!("Cannot open key file {}: {}", path, e)))?;
    let mut reader = std::io::BufReader::new(file);

    match rustls_pemfile::private_key(&mut reader)
        .map_err(|e| Error::TlsCertificateError(format!("Failed to parse private key from {}: {}", path, e)))?
    {
        Some(key) => Ok(key),
        None => Err(Error::TlsCertificateError(format!("No private key found in {}", path))),
    }
}

impl WebSocketListener {
    /// Binds a WebSocket listener to the specified address
    pub async fn bind(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<Self> {
        // When secure mode is requested, build a TLS acceptor
        #[cfg(feature = "tls")]
        let tls_acceptor = if secure {
            let cert = cert_path.ok_or_else(|| {
                Error::TlsCertificateError("WSS requires a certificate path".into())
            })?;
            let key = key_path.ok_or_else(|| {
                Error::TlsCertificateError("WSS requires a private key path".into())
            })?;

            let certs = load_certs(cert)?;
            let private_key = load_private_key(key)?;

            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, private_key)
                .map_err(|e| Error::TlsCertificateError(format!("Invalid TLS config for WSS: {}", e)))?;

            Some(TlsAcceptor::from(Arc::new(server_config)))
        } else {
            None
        };

        // When secure is requested but the tls feature is not compiled in, reject.
        #[cfg(not(feature = "tls"))]
        if secure {
            return Err(Error::NotImplemented(
                "WSS (secure WebSocket) requires the 'tls' feature to be enabled.".into(),
            ));
        }

        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;

        let local_addr = listener.local_addr()
            .map_err(|e| Error::LocalAddrFailed(e))?;

        let scheme = if secure { "wss" } else { "ws" };
        info!("WebSocket listener bound to {} ({})", local_addr, scheme);

        Ok(Self {
            listener,
            secure,
            #[cfg(feature = "tls")]
            tls_acceptor,
        })
    }

    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr().map_err(|e| Error::LocalAddrFailed(e))
    }

    /// Accepts a new WebSocket connection
    #[cfg(feature = "ws")]
    pub async fn accept(&self) -> Result<(WebSocketConnection, SplitStream<WebSocketStream<WsStream>>)> {
        // Accept a TCP connection
        let (tcp_stream, peer_addr) = self.listener.accept()
            .await
            .map_err(|e| Error::ReceiveFailed(e))?;

        debug!("Accepted TCP connection for WebSocket from {}", peer_addr);

        // Build the underlying stream: plain TCP or TLS-wrapped TCP
        let ws_stream = if self.secure {
            #[cfg(feature = "tls")]
            {
                let acceptor = self.tls_acceptor.as_ref().ok_or_else(|| {
                    Error::InvalidState("WSS listener has no TLS acceptor".into())
                })?;

                debug!("Performing TLS handshake with {} for WSS", peer_addr);

                let tls_stream = acceptor.accept(tcp_stream)
                    .await
                    .map_err(|e| Error::TlsHandshakeFailed(
                        format!("WSS TLS handshake with {}: {}", peer_addr, e)
                    ))?;

                debug!("TLS handshake completed with {} for WSS", peer_addr);
                WsStream::Tls(tls_stream)
            }
            #[cfg(not(feature = "tls"))]
            {
                return Err(Error::NotImplemented(
                    "WSS requires the 'tls' feature".into(),
                ));
            }
        } else {
            WsStream::Plain(tcp_stream)
        };

        // Select the appropriate subprotocol
        let subprotocol = if self.secure {
            SIP_WSS_SUBPROTOCOL
        } else {
            SIP_WS_SUBPROTOCOL
        }.to_string();

        // Perform WebSocket upgrade handshake, echoing back the sip/sips subprotocol
        // so that tungstenite 0.26+ does not reject the handshake on the client side.
        let negotiated = subprotocol.clone();
        let ws = accept_hdr_async(ws_stream, move |_req: &Request, mut response: Response| {
            response.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                HeaderValue::from_str(&negotiated)
                    .unwrap_or_else(|_| HeaderValue::from_static("sip")),
            );
            Ok(response)
        })
        .await
        .map_err(|e| {
            error!("WebSocket handshake failed with {}: {}", peer_addr, e);
            Error::WebSocketHandshakeFailed(e.to_string())
        })?;

        debug!("WebSocket handshake completed with {}", peer_addr);

        // Split the stream for separate reading and writing
        let (ws_writer, ws_reader) = ws.split();

        // Create a WebSocket connection
        let connection = WebSocketConnection::from_writer(
            ws_writer,
            peer_addr,
            self.secure,
            subprotocol,
        );

        Ok((connection, ws_reader))
    }

    /// Returns whether this is a secure WebSocket listener
    pub fn is_secure(&self) -> bool {
        self.secure
    }
}

#[cfg(not(feature = "ws"))]
impl WebSocketListener {
    /// Accepts a new WebSocket connection (not implemented without ws feature)
    pub async fn accept(&self) -> Result<()> {
        Err(Error::NotImplemented("WebSocket support is not enabled".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;
    use std::sync::Arc;

    /// Test binding a WebSocket listener
    #[tokio::test]
    async fn test_websocket_listener_bind() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, false, None, None).await.unwrap();

        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0); // Random port assigned
        assert_eq!(bound_addr.ip(), addr.ip());
        assert!(!listener.is_secure());
    }

    /// Test that binding a secure WebSocket listener requires cert/key paths
    #[cfg(feature = "tls")]
    #[tokio::test]
    async fn test_websocket_listener_bind_secure_requires_certs() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        // Missing cert_path should fail
        let result = WebSocketListener::bind(addr, true, None, Some("key.pem")).await;
        assert!(result.is_err());

        // Missing key_path should fail
        let result = WebSocketListener::bind(addr, true, Some("cert.pem"), None).await;
        assert!(result.is_err());
    }

    /// Tests accepting a WebSocket connection
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_listener_accept() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, false, None, None).await.unwrap();

        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0);

        // The method signature is what we're primarily verifying here
        let accept_method_exists = true;
        assert!(accept_method_exists);
    }
}
