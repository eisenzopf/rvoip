#[cfg(feature = "tls")]
mod connection;
#[cfg(feature = "tls")]
mod listener;

#[cfg(feature = "tls")]
pub use self::tls_impl::TlsTransport;

#[cfg(not(feature = "tls"))]
pub use self::tls_stub::TlsTransport;

// ─── Full TLS implementation (feature = "tls") ──────────────────────────────

#[cfg(feature = "tls")]
mod tls_impl {
    use std::fmt;
    use std::fs;
    use std::io::BufReader;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tracing::{debug, error, info, trace, warn};

    use rustls::{Certificate, PrivateKey, ServerConfig, ClientConfig};
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    use rvoip_sip_core::Message;
    use crate::error::{Error, Result};
    use crate::transport::{Transport, TransportEvent};
    use crate::transport::tcp::pool::{PoolConfig, ConnectionPool};

    use super::connection::TlsConnection;
    use super::listener::TlsListener;

    // Default channel capacity
    const DEFAULT_CHANNEL_CAPACITY: usize = 100;

    /// TLS transport for SIP messages with connection pooling
    #[derive(Clone)]
    pub struct TlsTransport {
        inner: Arc<TlsTransportInner>,
    }

    struct TlsTransportInner {
        listener: Arc<TlsListener>,
        tls_connector: TlsConnector,
        connection_pool: ConnectionPool,
        closed: AtomicBool,
        events_tx: mpsc::Sender<TransportEvent>,
    }

    /// Loads PEM certificates from a file path.
    fn load_certs(path: &str) -> Result<Vec<Certificate>> {
        let file = fs::File::open(path)
            .map_err(|e| Error::TlsCertificateError(format!("Cannot open cert file {}: {}", path, e)))?;
        let mut reader = BufReader::new(file);
        let certs = rustls_pemfile::certs(&mut reader)
            .map_err(|e| Error::TlsCertificateError(format!("Failed to parse certs from {}: {}", path, e)))?;
        Ok(certs.into_iter().map(Certificate).collect())
    }

    /// Loads a PEM private key from a file path.
    /// Tries PKCS8 first, then RSA keys.
    fn load_private_key(path: &str) -> Result<PrivateKey> {
        let file = fs::File::open(path)
            .map_err(|e| Error::TlsCertificateError(format!("Cannot open key file {}: {}", path, e)))?;
        let mut reader = BufReader::new(file);

        // Try PKCS8 keys first
        let mut keys = rustls_pemfile::pkcs8_private_keys(&mut reader)
            .map_err(|e| Error::TlsCertificateError(format!("Failed to parse PKCS8 keys from {}: {}", path, e)))?;

        if !keys.is_empty() {
            return Ok(PrivateKey(keys.remove(0)));
        }

        // Re-read file for RSA keys
        let file = fs::File::open(path)
            .map_err(|e| Error::TlsCertificateError(format!("Cannot re-open key file {}: {}", path, e)))?;
        let mut reader = BufReader::new(file);

        let mut rsa_keys = rustls_pemfile::rsa_private_keys(&mut reader)
            .map_err(|e| Error::TlsCertificateError(format!("Failed to parse RSA keys from {}: {}", path, e)))?;

        if !rsa_keys.is_empty() {
            return Ok(PrivateKey(rsa_keys.remove(0)));
        }

        Err(Error::TlsCertificateError(format!("No private key found in {}", path)))
    }

    impl TlsTransport {
        /// Creates a new TLS transport bound to the specified address.
        ///
        /// # Arguments
        /// * `addr` - Local address to bind to
        /// * `cert_path` - Path to PEM certificate file (server identity)
        /// * `key_path` - Path to PEM private key file
        /// * `_ca_path` - Reserved for future CA/client-cert verification (currently unused)
        /// * `channel_capacity` - Event channel capacity (defaults to 100)
        /// * `pool_config` - Connection pool configuration
        pub async fn bind(
            addr: SocketAddr,
            cert_path: &str,
            key_path: &str,
            _ca_path: Option<&str>,
            channel_capacity: Option<usize>,
            pool_config: Option<PoolConfig>,
        ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
            let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
            let (events_tx, events_rx) = mpsc::channel(capacity);

            // Load certificates and key
            let certs = load_certs(cert_path)?;
            let key = load_private_key(key_path)?;

            // Build server TLS config
            let server_config = ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_single_cert(certs.clone(), key.clone())
                .map_err(|e| Error::TlsCertificateError(format!("Invalid server TLS config: {}", e)))?;

            let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));

            // Build client TLS config (accepts any server cert for outbound connections;
            // production deployments should supply a proper root store)
            let mut root_store = rustls::RootCertStore::empty();
            // Add the server's own cert so it can talk to peers using the same cert
            for cert in &certs {
                let _ = root_store.add(cert);
            }

            let client_config = ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(root_store)
                .with_single_cert(certs, key)
                .map_err(|e| Error::TlsCertificateError(format!("Invalid client TLS config: {}", e)))?;

            let tls_connector = TlsConnector::from(Arc::new(client_config));

            // Bind the TLS listener
            let listener = TlsListener::bind(addr, tls_acceptor).await?;
            let local_addr = listener.local_addr()?;
            info!("SIP TLS transport bound to {}", local_addr);

            let config = pool_config.unwrap_or_default();
            let connection_pool = ConnectionPool::new(config);

            let transport = TlsTransport {
                inner: Arc::new(TlsTransportInner {
                    listener: Arc::new(listener),
                    tls_connector,
                    connection_pool,
                    closed: AtomicBool::new(false),
                    events_tx: events_tx.clone(),
                }),
            };

            // Start the accept loop
            transport.spawn_accept_loop();

            Ok((transport, events_rx))
        }

        /// Spawns a task to accept incoming TLS connections
        fn spawn_accept_loop(&self) {
            let transport = self.clone();

            tokio::spawn(async move {
                let inner = &transport.inner;
                let listener = inner.listener.clone();

                while !inner.closed.load(Ordering::Relaxed) {
                    match listener.accept().await {
                        Ok((tls_stream, peer_addr)) => {
                            debug!("Accepted TLS connection from {}", peer_addr);

                            let local_addr = match inner.listener.local_addr() {
                                Ok(a) => a,
                                Err(e) => {
                                    error!("Failed to get local address: {}", e);
                                    let _ = inner.events_tx.send(TransportEvent::Error {
                                        error: format!("Local addr error: {}", e),
                                    }).await;
                                    continue;
                                }
                            };

                            let connection = TlsConnection::from_server_stream(
                                tls_stream,
                                peer_addr,
                                local_addr,
                            );

                            transport.clone().spawn_connection_handler(connection);
                        }
                        Err(e) => {
                            if inner.closed.load(Ordering::Relaxed) {
                                break;
                            }

                            error!("Error accepting TLS connection: {}", e);
                            let _ = inner.events_tx.send(TransportEvent::Error {
                                error: format!("TLS accept error: {}", e),
                            }).await;

                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        }
                    }
                }

                info!("TLS accept loop terminated");
                let _ = inner.events_tx.send(TransportEvent::Closed).await;
            });
        }

        /// Spawns a handler for a new accepted connection
        fn spawn_connection_handler(&self, connection: TlsConnection) {
            let transport = self.clone();
            let peer_addr = connection.peer_addr();

            tokio::spawn(async move {
                let inner = &transport.inner;
                let events_tx = inner.events_tx.clone();

                while !inner.closed.load(Ordering::Relaxed) {
                    match connection.receive_message().await {
                        Ok(Some(message)) => {
                            debug!("Received SIP message from {} over TLS", peer_addr);

                            let local_addr = match connection.local_addr() {
                                Ok(addr) => addr,
                                Err(e) => {
                                    error!("Failed to get local address: {}", e);
                                    break;
                                }
                            };

                            let event = TransportEvent::MessageReceived {
                                message,
                                source: peer_addr,
                                destination: local_addr,
                            };

                            if let Err(e) = events_tx.send(event).await {
                                error!("Error sending event: {}", e);
                                break;
                            }
                        }
                        Ok(None) => {
                            info!("TLS connection from {} closed gracefully", peer_addr);
                            break;
                        }
                        Err(e) => {
                            if inner.closed.load(Ordering::Relaxed) {
                                break;
                            }

                            error!("Error reading from TLS connection {}: {}", peer_addr, e);
                            let _ = events_tx.send(TransportEvent::Error {
                                error: format!("TLS connection error from {}: {}", peer_addr, e),
                            }).await;
                            break;
                        }
                    }
                }
            });
        }

        /// Connects to a remote address using TLS
        async fn connect_to(&self, addr: SocketAddr) -> Result<Arc<TlsConnection>> {
            // Build a ServerName from the IP address (use DNS name in production)
            let server_name = rustls::ServerName::IpAddress(addr.ip());

            debug!("Creating new TLS connection to {}", addr);
            let connection = TlsConnection::connect(
                addr,
                &self.inner.tls_connector,
                server_name,
            ).await?;

            let connection_arc = Arc::new(connection);
            Ok(connection_arc)
        }
    }

    #[async_trait::async_trait]
    impl Transport for TlsTransport {
        fn local_addr(&self) -> Result<SocketAddr> {
            self.inner.listener.local_addr()
        }

        async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
            if self.is_closed() {
                return Err(Error::TransportClosed);
            }

            debug!(
                "Sending {} message to {} over TLS",
                if let Message::Request(ref req) = message {
                    format!("{}", req.method)
                } else {
                    "response".to_string()
                },
                destination
            );

            let connection = self.connect_to(destination).await?;
            connection.send_message(&message).await
        }

        async fn close(&self) -> Result<()> {
            self.inner.closed.store(true, Ordering::Relaxed);
            self.inner.connection_pool.close_all().await;
            Ok(())
        }

        fn is_closed(&self) -> bool {
            self.inner.closed.load(Ordering::Relaxed)
        }
    }

    impl fmt::Debug for TlsTransport {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if let Ok(addr) = self.inner.listener.local_addr() {
                write!(f, "TlsTransport({})", addr)
            } else {
                write!(f, "TlsTransport(<e>)")
            }
        }
    }
}

// ─── Stub when "tls" feature is disabled ─────────────────────────────────────

#[cfg(not(feature = "tls"))]
mod tls_stub {
    use std::fmt;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tracing::{error, info};

    use rvoip_sip_core::Message;
    use crate::error::{Error, Result};
    use crate::transport::{Transport, TransportEvent};
    use crate::transport::tcp::pool::PoolConfig;

    /// TLS transport stub when the `tls` feature is not enabled.
    #[derive(Clone)]
    pub struct TlsTransport {
        inner: Arc<TlsTransportInner>,
    }

    struct TlsTransportInner {
        local_addr: SocketAddr,
        closed: AtomicBool,
        events_tx: mpsc::Sender<TransportEvent>,
    }

    impl TlsTransport {
        /// Creates a new TLS transport — returns NotImplemented when the `tls` feature is disabled.
        pub async fn bind(
            addr: SocketAddr,
            _cert_path: &str,
            _key_path: &str,
            _ca_path: Option<&str>,
            channel_capacity: Option<usize>,
            _pool_config: Option<PoolConfig>,
        ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
            let capacity = channel_capacity.unwrap_or(100);
            let (events_tx, events_rx) = mpsc::channel(capacity);

            info!("TLS transport stub bound to {} (feature disabled)", addr);

            let transport = TlsTransport {
                inner: Arc::new(TlsTransportInner {
                    local_addr: addr,
                    closed: AtomicBool::new(false),
                    events_tx,
                }),
            };

            Ok((transport, events_rx))
        }
    }

    #[async_trait::async_trait]
    impl Transport for TlsTransport {
        fn local_addr(&self) -> Result<SocketAddr> {
            Ok(self.inner.local_addr)
        }

        async fn send_message(&self, _message: Message, destination: SocketAddr) -> Result<()> {
            if self.is_closed() {
                return Err(Error::TransportClosed);
            }

            error!("TLS transport feature not enabled, cannot send to {}", destination);
            Err(Error::NotImplemented("TLS transport requires the 'tls' feature".into()))
        }

        async fn close(&self) -> Result<()> {
            self.inner.closed.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn is_closed(&self) -> bool {
            self.inner.closed.load(Ordering::Relaxed)
        }
    }

    impl fmt::Debug for TlsTransport {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "TlsTransport({})", self.inner.local_addr)
        }
    }
}
