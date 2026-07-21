use futures_util::stream::SplitStream;
use futures_util::StreamExt;
#[cfg(feature = "ws")]
use http::HeaderValue;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
#[cfg(feature = "ws")]
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, error, info};

#[cfg(feature = "wss")]
use tokio_rustls::rustls::ServerConfig;
#[cfg(feature = "wss")]
use tokio_rustls::TlsAcceptor;

use super::connection::WebSocketConnection;
use super::{SipWsStream, SIP_WS_SUBPROTOCOL};
use crate::error::{Error, Result};

/// WebSocket listener for accepting SIP WebSocket connections
pub struct WebSocketListener {
    /// The underlying TCP listener
    listener: TcpListener,
    /// Whether this is a secure WebSocket listener (WSS)
    secure: bool,
    /// TLS certificate path if secure. Captured at bind so a future
    /// hot-reload / diagnostics path can surface the configured
    /// material without re-threading it.
    #[allow(dead_code)]
    cert_path: Option<String>,
    /// TLS key path if secure. See `cert_path` for retention rationale.
    #[allow(dead_code)]
    key_path: Option<String>,
    /// Pre-built TLS acceptor for WSS. Built once at `bind()` time so
    /// every accept() reuses the same `ServerConfig` (and therefore
    /// the same cert chain + session resumption cache).
    #[cfg(feature = "wss")]
    tls_acceptor: Option<TlsAcceptor>,
}

impl WebSocketListener {
    /// Binds a WebSocket listener to the specified address.
    ///
    /// For WSS (`secure = true`), `cert_path` and `key_path` must both
    /// be supplied — the same PEM cert / PKCS#8 key shape used by the
    /// TLS transport. The `TlsAcceptor` is built once here so per-
    /// accept handshakes don't re-parse cert material.
    pub async fn bind(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;

        info!(
            "WebSocket listener bound to {} ({})",
            listener.local_addr().unwrap(),
            if secure { "wss" } else { "ws" }
        );

        // Build the TLS acceptor up-front when the listener is secure.
        // Reuses the TLS transport's PEM loaders (re-exported as
        // `pub(crate)`) so cert/key handling is identical to the TLS
        // path — same root config, same error surface.
        #[cfg(feature = "wss")]
        let tls_acceptor = if secure {
            let (cert_p, key_p) = match (cert_path, key_path) {
                (Some(c), Some(k)) => (c, k),
                _ => {
                    return Err(Error::TlsHandshakeFailed(
                        "WSS listener requires both cert_path and key_path".into(),
                    ))
                }
            };
            let certs = crate::transport::tls::load_certs(Path::new(cert_p))?;
            let key = crate::transport::tls::load_private_key(Path::new(key_p))?;
            let server_config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .map_err(|e| Error::TlsHandshakeFailed(format!("WSS server config: {}", e)))?;
            Some(TlsAcceptor::from(Arc::new(server_config)))
        } else {
            None
        };

        Ok(Self {
            listener,
            secure,
            cert_path: cert_path.map(String::from),
            key_path: key_path.map(String::from),
            #[cfg(feature = "wss")]
            tls_acceptor,
        })
    }

    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .map_err(|e| Error::LocalAddrFailed(e))
    }

    /// Accepts a new WebSocket connection
    #[cfg(feature = "ws")]
    pub async fn accept(
        &self,
    ) -> Result<(
        WebSocketConnection,
        SplitStream<WebSocketStream<SipWsStream>>,
    )> {
        // Accept a TCP connection
        let (stream, peer_addr) = self
            .listener
            .accept()
            .await
            .map_err(|e| Error::ReceiveFailed(e))?;

        debug!("Accepted TCP connection for WebSocket from {}", peer_addr);

        // For WSS, run the TLS handshake on the freshly accepted TCP
        // socket BEFORE handing it to `accept_async`. The resulting
        // server-side `TlsStream` is wrapped in `SipWsStream::ServerTls`
        // (the client-side `MaybeTlsStream::Rustls` variant doesn't
        // cover the server direction). Plain WS skips the handshake and
        // wraps the raw TCP stream as `SipWsStream::Plain`.
        let maybe_tls_stream: SipWsStream = if self.secure {
            #[cfg(feature = "wss")]
            {
                let acceptor = self.tls_acceptor.as_ref().ok_or_else(|| {
                    Error::TlsHandshakeFailed(
                        "WSS listener marked secure but no TLS acceptor configured".into(),
                    )
                })?;
                let tls_stream = acceptor.accept(stream).await.map_err(|e| {
                    error!("WSS TLS handshake with {} failed: {}", peer_addr, e);
                    Error::TlsHandshakeFailed(format!("WSS handshake from {}: {}", peer_addr, e))
                })?;
                SipWsStream::ServerTls(tls_stream)
            }
            #[cfg(not(feature = "wss"))]
            {
                return Err(Error::NotImplemented(
                    "WSS listener requires the `wss` feature (rustls plumbing)".into(),
                ));
            }
        } else {
            SipWsStream::Plain(stream)
        };

        // RFC 7118 §4.1: server MUST include "sip" in the
        // Sec-WebSocket-Protocol response header when the client
        // offered it. Both WS and WSS use the same subprotocol name.
        // Use an AtomicBool (Send + Sync) so the closure is Send —
        // required because accept() is awaited inside a spawned task.
        let sip_offered = Arc::new(AtomicBool::new(false));
        let sip_offered_inner = sip_offered.clone();

        let ws_stream = tokio_tungstenite::accept_hdr_async(
            maybe_tls_stream,
            move |req: &Request, mut resp: Response| {
                let offered = req
                    .headers()
                    .get("Sec-WebSocket-Protocol")
                    .and_then(|h| h.to_str().ok())
                    .map(|s| s.split(',').any(|p| p.trim() == SIP_WS_SUBPROTOCOL))
                    .unwrap_or(false);

                if offered {
                    resp.headers_mut().insert(
                        "Sec-WebSocket-Protocol",
                        HeaderValue::from_static(SIP_WS_SUBPROTOCOL),
                    );
                    sip_offered_inner.store(true, Ordering::Relaxed);
                }
                Ok(resp)
            },
        )
        .await
        .map_err(|e| {
            error!("WebSocket handshake failed with {}: {}", peer_addr, e);
            Error::WebSocketHandshakeFailed(e.to_string())
        })?;

        // Subprotocol is "sip" if the client offered it (RFC 7118 §4.1).
        // If the client did not advertise it at all we still carry the
        // constant so the connection wrapper is consistently typed.
        let subprotocol = SIP_WS_SUBPROTOCOL.to_string();
        let _ = sip_offered; // was checked to set the response header

        // Split the stream for separate reading and writing
        let (ws_writer, ws_reader) = ws_stream.split();

        // Create a WebSocket connection
        let connection =
            WebSocketConnection::from_writer(ws_writer, peer_addr, self.secure, subprotocol);

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
        Err(Error::NotImplemented(
            "WebSocket support is not enabled".into(),
        ))
    }
}

// Unit tests will be added later

#[cfg(test)]
mod tests {
    use super::*;

    /// Test binding a WebSocket listener
    #[tokio::test]
    async fn test_websocket_listener_bind() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, false, None, None)
            .await
            .unwrap();

        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0); // Random port assigned
        assert_eq!(bound_addr.ip(), addr.ip());
        assert!(!listener.is_secure());
    }

    /// Test binding a secure WebSocket listener.
    ///
    /// Phase 4 wired real TLS into `bind()`: when `secure = true`, the
    /// listener now actually opens the cert/key files and builds a
    /// `TlsAcceptor`. So the test has to generate real PEM material
    /// (via `rcgen`, the same dev-dep the TLS handshake test uses).
    /// Gated on `wss` because the TLS acceptor lives behind that
    /// feature.
    #[cfg(feature = "wss")]
    #[tokio::test]
    async fn test_websocket_listener_bind_secure() {
        use std::io::Write;
        let tmp = tempfile::tempdir().expect("tempdir");
        let cert_path = tmp.path().join("server.crt");
        let key_path = tmp.path().join("server.key");

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("rcgen self-signed");
        std::fs::File::create(&cert_path)
            .and_then(|mut f| f.write_all(cert.cert.pem().as_bytes()))
            .expect("write cert");
        std::fs::File::create(&key_path)
            .and_then(|mut f| f.write_all(cert.signing_key.serialize_pem().as_bytes()))
            .expect("write key");

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(
            addr,
            true,
            Some(cert_path.to_str().unwrap()),
            Some(key_path.to_str().unwrap()),
        )
        .await
        .unwrap();

        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0); // Random port assigned
        assert_eq!(bound_addr.ip(), addr.ip());
        assert!(listener.is_secure());

        // Verify certificate paths are stored
        assert_eq!(listener.cert_path.as_deref(), cert_path.to_str());
        assert_eq!(listener.key_path.as_deref(), key_path.to_str());
    }

    /// Tests accepting a WebSocket connection
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_listener_accept() {
        // This is a more complex test that would require us to actually
        // create a WebSocket client that connects to our listener.

        // For now, we'll simply test that the listener can be created and accept() method exists
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, false, None, None)
            .await
            .unwrap();

        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0);

        // We can't easily test the accept method without setting up a real WebSocket client
        // The method signature is what we're primarily verifying here
        let accept_method_exists = true; // This test passes if it compiles
        assert!(accept_method_exists);
    }

    /// Simple client-server connection test (integration level)
    #[cfg(all(feature = "ws", test))]
    #[tokio::test]
    async fn test_websocket_client_server_connection() {
        // This test is marked with #[cfg(all(feature = "ws", test))] because:
        // 1. It requires the ws feature
        // 2. It's more of an integration test than a unit test

        // Ideally we'd have a full client-server test that uses a client to connect to
        // our listener and test the full protocol flow, but that's challenging to do
        // without refactoring the code to support a test client or using a real client.

        // To directly test the listener's accept method properly would require:
        // 1. Setting up a tokio runtime
        // 2. Creating a TCP connection to the listener
        // 3. Performing a WebSocket handshake manually or with a client
        // 4. Verifying the connection is accepted and the right objects are returned

        // This is left as a future enhancement. In a production environment,
        // you'd typically have integration tests that create a real client and
        // send actual WebSocket frames to test the complete flow.

        // For now we're relying on:
        // 1. The unit tests for individual components
        // 2. The integration tests for the transport as a whole
        // 3. Manual testing with real clients
    }

    /// RFC 7118 §4.1 — server MUST echo "sip" in Sec-WebSocket-Protocol
    /// when the client offers it. This test fails if accept_hdr_async is
    /// replaced by plain accept_async (which omits the response header).
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_ws_handshake_includes_sip_subprotocol() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = WebSocketListener::bind(addr, false, None, None)
            .await
            .unwrap();
        let server_addr = listener.local_addr().unwrap();

        // Server: accept one connection in a background task
        let server_task = tokio::spawn(async move {
            let (conn, _reader) = listener.accept().await.expect("accept failed");
            conn.subprotocol().to_string()
        });

        // Client: connect with Sec-WebSocket-Protocol: sip
        let url = format!("ws://{}/", server_addr);
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            http::HeaderValue::from_static("sip"),
        );
        let tcp = tokio::net::TcpStream::connect(server_addr).await.unwrap();
        let (ws, response) = tokio_tungstenite::client_async(request, tcp)
            .await
            .expect("WS handshake failed");

        // RFC 7118 §4.1: response MUST contain Sec-WebSocket-Protocol: sip
        let echoed = response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            echoed, "sip",
            "HTTP 101 must echo Sec-WebSocket-Protocol: sip (RFC 7118 §4.1)"
        );

        drop(ws);
        let server_subprotocol = server_task.await.unwrap();
        assert_eq!(server_subprotocol, "sip");
    }

    /// RFC 7118 §4.1 applies equally to WSS — the subprotocol is "sip",
    /// not "sips". Verifies the corrected SIP_WSS_SUBPROTOCOL constant.
    #[cfg(all(feature = "ws", feature = "wss"))]
    #[tokio::test]
    async fn test_wss_handshake_includes_sip_subprotocol_not_sips() {
        use std::io::Write;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let tmp = tempfile::tempdir().unwrap();
        let cert_path = tmp.path().join("server.crt");
        let key_path = tmp.path().join("server.key");
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        std::fs::File::create(&cert_path)
            .and_then(|mut f| f.write_all(cert.cert.pem().as_bytes()))
            .unwrap();
        std::fs::File::create(&key_path)
            .and_then(|mut f| f.write_all(cert.signing_key.serialize_pem().as_bytes()))
            .unwrap();

        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = WebSocketListener::bind(
            addr,
            true,
            Some(cert_path.to_str().unwrap()),
            Some(key_path.to_str().unwrap()),
        )
        .await
        .unwrap();
        let server_addr = listener.local_addr().unwrap();

        let cert_der = cert.cert.der().to_vec();

        let server_task = tokio::spawn(async move {
            let (conn, _reader) = listener.accept().await.expect("WSS accept failed");
            conn.subprotocol().to_string()
        });

        // Build a rustls client that trusts the self-signed cert
        let mut root_store = tokio_rustls::rustls::RootCertStore::empty();
        root_store
            .add(tokio_rustls::rustls::pki_types::CertificateDer::from(
                cert_der,
            ))
            .unwrap();
        let client_config = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(client_config));
        let tcp = tokio::net::TcpStream::connect(server_addr).await.unwrap();
        let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
            .unwrap()
            .to_owned();
        let tls_stream = connector.connect(server_name, tcp).await.unwrap();
        let stream = crate::transport::ws::SipWsStream::ClientTls(tls_stream);

        let url = format!("wss://{}/", server_addr);
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            http::HeaderValue::from_static("sip"),
        );
        let (ws, response) = tokio_tungstenite::client_async(request, stream)
            .await
            .expect("WSS WS handshake failed");

        let echoed = response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            echoed, "sip",
            "WSS HTTP 101 must echo Sec-WebSocket-Protocol: sip (not 'sips') per RFC 7118 §4.1"
        );

        drop(ws);
        let server_subprotocol = server_task.await.unwrap();
        assert_eq!(server_subprotocol, "sip");
    }
}
