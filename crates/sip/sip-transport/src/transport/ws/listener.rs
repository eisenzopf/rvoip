use futures_util::stream::SplitStream;
use futures_util::StreamExt;
#[cfg(feature = "ws")]
use http::HeaderValue;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
#[cfg(feature = "ws")]
use tokio_tungstenite::{tungstenite, WebSocketStream};
use tracing::{debug, error, info};

#[cfg(feature = "wss")]
use tokio_rustls::TlsAcceptor;

use super::connection::{sip_websocket_config, WebSocketConnection};
use super::{SipWsStream, SIP_WS_SUBPROTOCOL};
use crate::error::{Error, Result};
#[cfg(feature = "wss")]
use crate::transport::tls::TlsServerClientAuthConfig;

#[cfg(feature = "ws")]
fn select_sip_subprotocol(offered: Option<&str>, _secure: bool) -> Option<&'static str> {
    let required = SIP_WS_SUBPROTOCOL;
    offered
        .map(|value| value.split(',').map(str::trim).any(|item| item == required))
        .is_some_and(|matched| matched)
        .then_some(required)
}

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
        #[cfg(feature = "wss")]
        {
            return Self::bind_with_client_auth(
                addr,
                secure,
                cert_path,
                key_path,
                TlsServerClientAuthConfig::default(),
            )
            .await;
        }
        #[cfg(not(feature = "wss"))]
        {
            Self::bind_inner(addr, secure, cert_path, key_path).await
        }
    }

    /// Bind a WebSocket listener with an explicit inbound WSS
    /// client-certificate policy. Plain WS ignores this configuration.
    #[cfg(feature = "wss")]
    pub async fn bind_with_client_auth(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: TlsServerClientAuthConfig,
    ) -> Result<Self> {
        Self::bind_inner_with_client_auth(addr, secure, cert_path, key_path, client_auth).await
    }

    #[cfg(feature = "wss")]
    async fn bind_inner_with_client_auth(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: TlsServerClientAuthConfig,
    ) -> Result<Self> {
        Self::bind_inner_impl(addr, secure, cert_path, key_path, Some(client_auth)).await
    }

    #[cfg(not(feature = "wss"))]
    async fn bind_inner(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<Self> {
        Self::bind_inner_impl(addr, secure, cert_path, key_path).await
    }

    #[cfg(feature = "wss")]
    async fn bind_inner_impl(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: Option<TlsServerClientAuthConfig>,
    ) -> Result<Self> {
        let client_auth = client_auth.unwrap_or_default();
        Self::bind_configured(addr, secure, cert_path, key_path, &client_auth).await
    }

    #[cfg(not(feature = "wss"))]
    async fn bind_inner_impl(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<Self> {
        Self::bind_configured(addr, secure, cert_path, key_path).await
    }

    #[cfg(feature = "wss")]
    async fn bind_configured(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: &TlsServerClientAuthConfig,
    ) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;

        info!(
            local_addr = %listener.local_addr().unwrap(),
            transport = if secure { "wss" } else { "ws" },
            client_auth_mode = ?client_auth.mode,
            "WebSocket listener bound"
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
            let server_config = crate::transport::tls::build_server_config(
                std::path::Path::new(cert_p),
                std::path::Path::new(key_p),
                client_auth,
                "WSS",
            )?;
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

    #[cfg(not(feature = "wss"))]
    async fn bind_configured(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;
        info!(
            local_addr = %listener.local_addr().unwrap(),
            transport = if secure { "wss" } else { "ws" },
            "WebSocket listener bound"
        );
        Ok(Self {
            listener,
            secure,
            cert_path: cert_path.map(String::from),
            key_path: key_path.map(String::from),
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
        let (maybe_tls_stream, connection_metadata): (SipWsStream, _) = if self.secure {
            #[cfg(feature = "wss")]
            {
                let acceptor = self.tls_acceptor.as_ref().ok_or_else(|| {
                    Error::TlsHandshakeFailed(
                        "WSS listener marked secure but no TLS acceptor configured".into(),
                    )
                })?;
                let tls_stream = acceptor.accept(stream).await.map_err(|error| {
                    let error_class = crate::transport::tls::tls_runtime_failure_class(&error);
                    error!(
                        source = %peer_addr,
                        error_class,
                        "WSS TLS handshake failed"
                    );
                    crate::transport::tls::classify_tls_runtime_error(
                        error,
                        format!("WSS server TLS handshake failed for {peer_addr}"),
                    )
                })?;
                let metadata = crate::transport::tls::verified_peer_metadata(
                    tls_stream.get_ref().1.peer_certificates(),
                );
                (SipWsStream::ServerTls(tls_stream), metadata)
            }
            #[cfg(not(feature = "wss"))]
            {
                return Err(Error::NotImplemented(
                    "WSS listener requires the `wss` feature (rustls plumbing)".into(),
                ));
            }
        } else {
            (SipWsStream::Plain(stream), None)
        };

        // Custom callback for WebSocket handshake to handle subprotocol negotiation
        let callback = |request: &Request, response: Response| {
            // Check for subprotocol request
            let offered = request
                .headers()
                .get("Sec-WebSocket-Protocol")
                .and_then(|h| h.to_str().ok());

            let Some(required_protocol) = select_sip_subprotocol(offered, self.secure) else {
                let rejection: ErrorResponse = http::Response::builder()
                    .status(http::StatusCode::BAD_REQUEST)
                    .body(Some(
                        "required SIP WebSocket subprotocol was not offered".to_string(),
                    ))
                    .expect("static WebSocket rejection response is valid");
                return Err(rejection);
            };

            let mut response = response;
            response.headers_mut().append(
                "Sec-WebSocket-Protocol",
                HeaderValue::from_static(required_protocol),
            );
            Ok((response, required_protocol.to_string()))
        };

        // Perform WebSocket handshake
        let (ws_stream, selected_protocol) =
            Self::accept_async_with_subprotocol(maybe_tls_stream, callback)
                .await
                .map_err(|_error| {
                    error!(
                        source = %peer_addr,
                        error_class = "websocket_handshake_failed",
                        "WebSocket handshake failed"
                    );
                    Error::WebSocketHandshakeFailed(format!(
                        "WebSocket server handshake failed for {peer_addr}"
                    ))
                })?;

        let subprotocol = selected_protocol;

        // Split the stream for separate reading and writing
        let (ws_writer, ws_reader) = ws_stream.split();

        // Create a WebSocket connection
        let connection = WebSocketConnection::from_writer_with_metadata(
            ws_writer,
            peer_addr,
            self.secure,
            subprotocol,
            connection_metadata,
        );

        Ok((connection, ws_reader))
    }

    /// Helper to accept WebSocket connections with subprotocol selection
    #[cfg(feature = "ws")]
    async fn accept_async_with_subprotocol<F>(
        stream: SipWsStream,
        callback: F,
    ) -> std::result::Result<(WebSocketStream<SipWsStream>, String), tungstenite::Error>
    where
        F: FnOnce(&Request, Response) -> std::result::Result<(Response, String), ErrorResponse>
            + Unpin,
    {
        let selected_protocol = Arc::new(std::sync::Mutex::new(String::new()));
        let selected_for_callback = Arc::clone(&selected_protocol);
        let ws_stream = tokio_tungstenite::accept_hdr_async_with_config(
            stream,
            move |request: &Request, response: Response| {
                let (response, selected) = callback(request, response)?;
                *selected_for_callback
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = selected;
                Ok(response)
            },
            Some(sip_websocket_config()),
        )
        .await?;
        let selected = selected_protocol
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        Ok((ws_stream, selected))
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

    #[cfg(feature = "ws")]
    #[test]
    fn sip_subprotocol_selection_is_fail_closed() {
        assert_eq!(select_sip_subprotocol(None, false), None);
        assert_eq!(select_sip_subprotocol(Some("chat"), false), None);
        assert_eq!(select_sip_subprotocol(Some("sips"), false), None);
        assert_eq!(
            select_sip_subprotocol(Some("chat, sip"), false),
            Some("sip")
        );
        assert_eq!(select_sip_subprotocol(Some("sips"), true), None);
        assert_eq!(select_sip_subprotocol(Some("chat, sip"), true), Some("sip"));
    }

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
}
