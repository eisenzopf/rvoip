use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use rvoip_sip_core::framing::{inspect_sip_frame_with_policy, SipFrameStatus, SipFramingPolicy};
use rvoip_sip_core::types::uri::Host;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_rustls::rustls::{
    self,
    pki_types::{CertificateDer, PrivateKeyDer, ServerName},
    server::WebPkiClientVerifier,
    ClientConfig, RootCertStore, ServerConfig,
};
// `dev-insecure-tls` is the only path that implements `ServerCertVerifier`;
// everything below is only reachable inside that gate.
#[cfg(feature = "dev-insecure-tls")]
use tokio_rustls::rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::UnixTime,
    DigitallySignedStruct, SignatureScheme,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::transport::{
    runtime::{
        next_trust_context, ConnectionDirection, ConnectionLifecycleConfig, DialAdmission,
        OutboundDialCoordinator, TransportTaskSet,
    },
    validate_typed_outbound_message, HandshakeAdmissionConfig, TlsPeerIdentity, Transport,
    TransportConnectionMetadata, TransportEvent, TransportType,
};

#[derive(Clone, Debug)]
struct TlsConnectionRecord {
    generation: u64,
    sender: mpsc::Sender<Bytes>,
}

struct AbortTaskOnDrop(tokio::task::AbortHandle);

impl AbortTaskOnDrop {
    fn abort(&self) {
        self.0.abort();
    }
}

impl Drop for AbortTaskOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TlsConnectionKey {
    remote_addr: SocketAddr,
    direction: ConnectionDirection,
    authority: String,
    trust_context: u64,
}

impl TlsConnectionKey {
    fn outbound(remote_addr: SocketAddr, server_name: &ServerName<'_>, trust_context: u64) -> Self {
        Self {
            remote_addr,
            direction: ConnectionDirection::Outbound,
            authority: normalized_server_name(server_name),
            trust_context,
        }
    }

    fn inbound(remote_addr: SocketAddr, local_addr: SocketAddr, trust_context: u64) -> Self {
        Self {
            remote_addr,
            direction: ConnectionDirection::Inbound,
            authority: format!("listener:{local_addr}"),
            trust_context,
        }
    }
}

fn remove_tls_connection_if_generation(
    connections: &mut HashMap<TlsConnectionKey, TlsConnectionRecord>,
    key: &TlsConnectionKey,
    generation: u64,
) -> bool {
    if connections
        .get(key)
        .is_some_and(|record| record.generation == generation)
    {
        connections.remove(key);
        true
    } else {
        false
    }
}

/// Builder-friendly TLS client configuration. Mirrors the knobs we
/// expect to expose through `session-core::Config` once Step 1C wires
/// it up.
#[derive(Debug, Clone, Default)]
pub struct TlsClientConfig {
    /// Optional path to a PEM-encoded CA bundle to *add to* the system
    /// trust store. Useful for enterprise PKI / private carriers.
    pub extra_ca_path: Option<PathBuf>,
    /// **Dev-only.** When `true`, server certificates are accepted
    /// without validation. Required for self-signed test certs; **must
    /// not** be enabled in production builds. The TLS handshake still
    /// runs end-to-end (encrypted), but identity is not verified.
    pub insecure_skip_verify: bool,
    /// Optional PEM-encoded client certificate chain for mutual TLS.
    /// Normal SIP TLS client/B2BUA deployments do not need this; set it
    /// only when the remote server explicitly requires client auth.
    pub client_cert_path: Option<PathBuf>,
    /// Optional PEM-encoded PKCS#8 private key for
    /// [`TlsClientConfig::client_cert_path`].
    pub client_key_path: Option<PathBuf>,
}

/// Inbound TLS client-certificate policy for SIP TLS and SIP WSS listeners.
///
/// This is intentionally separate from [`TlsClientConfig`], which controls
/// outbound server verification and the certificate this endpoint presents
/// when acting as a client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TlsClientAuthMode {
    /// Preserve the historical server-only TLS behavior: do not request a
    /// client certificate.
    #[default]
    Disabled,
    /// Request and verify a client certificate when one is presented, while
    /// allowing clients that present no certificate.
    Optional,
    /// Require every client to present a certificate chaining to the
    /// configured trust anchors.
    Required,
}

/// Configuration for inbound TLS/WSS client-certificate authentication.
#[derive(Debug, Clone, Default)]
pub struct TlsServerClientAuthConfig {
    /// Whether client certificates are disabled, optional, or required.
    pub mode: TlsClientAuthMode,
    /// PEM bundle containing trust anchors for client certificates. Required
    /// for [`TlsClientAuthMode::Optional`] and
    /// [`TlsClientAuthMode::Required`]. System roots are deliberately not
    /// loaded for inbound client authentication.
    pub client_ca_path: Option<PathBuf>,
}

impl TlsServerClientAuthConfig {
    /// Require a client certificate chaining to `client_ca_path`.
    pub fn required(client_ca_path: impl Into<PathBuf>) -> Self {
        Self {
            mode: TlsClientAuthMode::Required,
            client_ca_path: Some(client_ca_path.into()),
        }
    }

    /// Verify a client certificate when presented, but allow anonymous TLS
    /// clients as well.
    pub fn optional(client_ca_path: impl Into<PathBuf>) -> Self {
        Self {
            mode: TlsClientAuthMode::Optional,
            client_ca_path: Some(client_ca_path.into()),
        }
    }
}

/// TLS socket role for a SIP transport instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsRole {
    /// Outbound TLS client only. Does not bind a TLS listener and does
    /// not require a local server certificate/key.
    ClientOnly,
    /// Inbound TLS listener only. Requires a local certificate/key.
    ServerOnly,
    /// Both inbound listener and outbound client. Requires a local
    /// certificate/key for the listener side.
    ClientAndServer,
}

/// TLS transport implementation for SIP.
///
/// A single `TlsTransport` instance handles both inbound (server) and
/// outbound (client) TLS connections. The server side accepts
/// connections on `local_addr`; the client side dials remote peers via
/// [`TlsTransport::connect`] (or implicitly through `send_message` —
/// missing connections are auto-dialed).
pub struct TlsTransport {
    /// Socket role for this transport. Server-only transports may send
    /// on accepted connections but must not open new outbound TLS
    /// connections.
    role: TlsRole,

    /// Local address the server side listens on.
    local_addr: SocketAddr,

    /// TLS acceptor for inbound connections (server side). `None` in
    /// client-only mode.
    acceptor: Option<TlsAcceptor>,

    /// TLS connector used for outbound dials. Holds the rustls
    /// `ClientConfig` (root store, cert verifier, etc.) so each
    /// `connect()` reuses the same trust state instead of rebuilding
    /// it.
    connector: TlsConnector,

    /// Active TLS connections, keyed by remote address. Used by
    /// `send_message` to find the right write-side mpsc channel.
    /// Connection-lifetime: removed by the per-connection reader task
    /// on EOF/error.
    connections: Arc<tokio::sync::Mutex<HashMap<TlsConnectionKey, TlsConnectionRecord>>>,

    /// Monotonic identity used to prevent an old reader from removing a
    /// replacement connection to the same destination.
    next_connection_generation: Arc<AtomicU64>,

    /// Transport event sender.
    event_tx: Option<mpsc::Sender<TransportEvent>>,

    /// Closed flag. Once set, `connect()` and `send_message` short-circuit.
    closed: Arc<AtomicBool>,

    /// Owns listener, handshake, reader, and writer tasks so `close()` can
    /// deterministically cancel and join all transport activity.
    tasks: Arc<TransportTaskSet>,

    /// Serializes complete close/drain operations, including connection-map
    /// cleanup after managed tasks have stopped.
    close_gate: Arc<Mutex<()>>,

    /// Inbound and outbound TLS handshake admission policy. Bidirectional
    /// transports maintain independent concurrency budgets for each direction.
    handshake_admission: HandshakeAdmissionConfig,

    lifecycle: ConnectionLifecycleConfig,

    /// Bounded pending calls, true authority-aware singleflight, and a short
    /// shared failure backoff for outbound TCP/TLS establishment.
    outbound_dials: Arc<OutboundDialCoordinator<TlsConnectionKey>>,

    inbound_established: Arc<Semaphore>,
    outbound_established: Arc<Semaphore>,
    outbound_trust_context: u64,
}

impl fmt::Debug for TlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsTransport")
            .field("role", &self.role)
            .field("local_addr", &self.local_addr)
            .field("has_acceptor", &self.acceptor.is_some())
            .field("connections", &self.connections)
            .field("closed", &self.closed)
            .field("handshake_admission", &self.handshake_admission)
            .finish()
    }
}

impl TlsTransport {
    /// Create a new TLS transport bound to `local_addr` for inbound
    /// connections. The server uses the supplied cert/key pair; the
    /// client side is built with default validation (system root CAs).
    pub async fn bind(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_handshake_config(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind with explicit admission for inbound and outbound TLS handshakes.
    /// Outbound certificate validation keeps its existing policy.
    pub async fn bind_with_handshake_config(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_configs_and_handshake(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            TlsClientConfig::default(),
            TlsServerClientAuthConfig::default(),
            handshake_admission,
        )
        .await
    }

    /// Like [`bind`](Self::bind) but with explicit client-side TLS
    /// configuration (extra CA, insecure-skip).
    pub async fn bind_with_client_config(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_configs(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            client_cfg,
            TlsServerClientAuthConfig::default(),
        )
        .await
    }

    /// Bind a bidirectional TLS transport with independent outbound-client
    /// and inbound-client-certificate policies.
    pub async fn bind_with_configs(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
        server_client_auth: TlsServerClientAuthConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_configs_and_handshake(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            client_cfg,
            server_client_auth,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind a bidirectional TLS transport with explicit client/server TLS
    /// policies and inbound/outbound handshake admission.
    pub async fn bind_with_configs_and_handshake(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
        server_client_auth: TlsServerClientAuthConfig,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_role_and_configs(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            client_cfg,
            server_client_auth,
            TlsRole::ClientAndServer,
            handshake_admission,
        )
        .await
    }

    /// Bind a TLS listener that does not open new outbound TLS
    /// connections. Replies over already-accepted TLS connections remain
    /// valid.
    pub async fn bind_server_only_with_client_config(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_role_and_configs(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            client_cfg,
            TlsServerClientAuthConfig::default(),
            TlsRole::ServerOnly,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind a server-only TLS listener with explicit inbound
    /// client-certificate authentication.
    pub async fn bind_server_only_with_client_auth(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        server_client_auth: TlsServerClientAuthConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_role_and_configs(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            TlsClientConfig::default(),
            server_client_auth,
            TlsRole::ServerOnly,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind a server-only TLS listener with explicit client/server TLS policy
    /// and inbound/outbound handshake admission.
    pub async fn bind_server_only_with_configs_and_handshake(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
        server_client_auth: TlsServerClientAuthConfig,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_role_and_configs(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            client_cfg,
            server_client_auth,
            TlsRole::ServerOnly,
            handshake_admission,
        )
        .await
    }

    async fn bind_with_role_and_configs(
        local_addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
        server_client_auth: TlsServerClientAuthConfig,
        role: TlsRole,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let handshake_admission = handshake_admission.validate("TLS")?;
        let lifecycle = ConnectionLifecycleConfig::from_handshake(handshake_admission);
        // Server-side config (for incoming TLS connections).
        let server_config = build_server_config(cert_path, key_path, &server_client_auth, "TLS")?;
        let acceptor = TlsAcceptor::from(Arc::new(server_config));

        // Client-side config (for outgoing TLS dials).
        let connector = TlsConnector::from(Arc::new(build_client_config(&client_cfg)?));

        let (tx, rx) = if let Some(tx) = event_tx {
            (tx, mpsc::channel::<TransportEvent>(100).1)
        } else {
            mpsc::channel::<TransportEvent>(100)
        };

        // Bind the listener synchronously so we can report the
        // actually-allocated port back via `local_addr()`. (Important
        // for tests that bind on port 0 and need to know which
        // ephemeral port the OS picked.)
        let listener = TcpListener::bind(local_addr)
            .await
            .map_err(|e| Error::BindFailed(local_addr, e))?;
        let actual_addr = listener.local_addr().map_err(Error::LocalAddrFailed)?;
        info!(
            %actual_addr,
            client_auth_mode = ?server_client_auth.mode,
            "TLS transport listening"
        );

        let tasks = TransportTaskSet::new();
        let inbound_established = Arc::new(Semaphore::new(lifecycle.max_established_per_direction));
        let outbound_established =
            Arc::new(Semaphore::new(lifecycle.max_established_per_direction));
        let inbound_trust_context = next_trust_context();
        let outbound_trust_context = next_trust_context();
        let transport = Self {
            role,
            local_addr: actual_addr,
            acceptor: Some(acceptor),
            connector,
            connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            next_connection_generation: Arc::new(AtomicU64::new(1)),
            event_tx: Some(tx),
            closed: Arc::new(AtomicBool::new(false)),
            tasks: tasks.clone(),
            close_gate: Arc::new(Mutex::new(())),
            handshake_admission,
            lifecycle,
            outbound_dials: OutboundDialCoordinator::new(
                handshake_admission.max_concurrent,
                lifecycle.max_pending_dials,
                lifecycle.failure_backoff,
            ),
            inbound_established: inbound_established.clone(),
            outbound_established,
            outbound_trust_context,
        };

        let started = tasks
            .spawn(Self::accept_loop(
                listener,
                actual_addr,
                transport
                    .acceptor
                    .clone()
                    .expect("TLS listener mode must have an acceptor"),
                transport.connections.clone(),
                transport.next_connection_generation.clone(),
                transport.event_tx.clone().unwrap(),
                Arc::downgrade(&tasks),
                handshake_admission,
                lifecycle,
                inbound_established,
                inbound_trust_context,
            ))
            .await;
        if started.is_none() {
            return Err(Error::TransportClosed);
        }

        Ok((transport, rx))
    }

    /// Create a TLS transport that only dials outbound TLS
    /// connections. This is the correct shape for a registered SIP UA
    /// behind an upstream proxy/B2BUA such as Asterisk: the peer owns
    /// the TLS server certificate, and this endpoint verifies it.
    ///
    /// `local_addr` is the logical SIP address used by upper layers for
    /// Via/Contact construction. No TCP listener is bound here, so the
    /// OS may choose a different ephemeral source port for each outbound
    /// connection. Responses and inbound requests from the peer are
    /// expected to arrive on the established TLS flow.
    pub async fn client_only(
        local_addr: SocketAddr,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::client_only_with_handshake_config(
            local_addr,
            event_tx,
            client_cfg,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Create a client-only transport with bounded, end-to-end outbound
    /// TCP/TLS establishment. The concurrency limit is global to this
    /// transport and same-destination dials are always single-flight.
    pub async fn client_only_with_handshake_config(
        local_addr: SocketAddr,
        event_tx: Option<mpsc::Sender<TransportEvent>>,
        client_cfg: TlsClientConfig,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let handshake_admission = handshake_admission.validate("TLS")?;
        let lifecycle = ConnectionLifecycleConfig::from_handshake(handshake_admission);
        let connector = TlsConnector::from(Arc::new(build_client_config(&client_cfg)?));

        let (tx, rx) = if let Some(tx) = event_tx {
            (tx, mpsc::channel::<TransportEvent>(100).1)
        } else {
            mpsc::channel::<TransportEvent>(100)
        };

        info!(
            "TLS client-only transport configured with logical local address {}",
            local_addr
        );

        Ok((
            Self {
                role: TlsRole::ClientOnly,
                local_addr,
                acceptor: None,
                connector,
                connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
                next_connection_generation: Arc::new(AtomicU64::new(1)),
                event_tx: Some(tx),
                closed: Arc::new(AtomicBool::new(false)),
                tasks: TransportTaskSet::new(),
                close_gate: Arc::new(Mutex::new(())),
                handshake_admission,
                lifecycle,
                outbound_dials: OutboundDialCoordinator::new(
                    handshake_admission.max_concurrent,
                    lifecycle.max_pending_dials,
                    lifecycle.failure_backoff,
                ),
                inbound_established: Arc::new(Semaphore::new(
                    lifecycle.max_established_per_direction,
                )),
                outbound_established: Arc::new(Semaphore::new(
                    lifecycle.max_established_per_direction,
                )),
                outbound_trust_context: next_trust_context(),
            },
            rx,
        ))
    }

    /// Accept loop driving an already-bound `TcpListener`.
    async fn accept_loop(
        listener: TcpListener,
        addr: SocketAddr,
        acceptor: TlsAcceptor,
        connections: Arc<tokio::sync::Mutex<HashMap<TlsConnectionKey, TlsConnectionRecord>>>,
        next_connection_generation: Arc<AtomicU64>,
        event_tx: mpsc::Sender<TransportEvent>,
        weak_tasks: Weak<TransportTaskSet>,
        handshake_admission: HandshakeAdmissionConfig,
        lifecycle: ConnectionLifecycleConfig,
        established: Arc<Semaphore>,
        trust_context: u64,
    ) {
        let semaphore = Arc::new(Semaphore::new(handshake_admission.max_concurrent));
        loop {
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => break,
            };
            match listener.accept().await {
                Ok((stream, remote_addr)) => {
                    debug!("New TCP connection from {}", remote_addr);
                    let acceptor = acceptor.clone();
                    let connections = connections.clone();
                    let event_tx = event_tx.clone();
                    let next_connection_generation = next_connection_generation.clone();
                    let established = established.clone();
                    let local_addr = addr;
                    let weak_tasks_for_connection = weak_tasks.clone();
                    let Some(tasks) = weak_tasks.upgrade() else {
                        break;
                    };

                    let _ = tasks
                        .spawn(async move {
                            let registration_deadline =
                                tokio::time::Instant::now() + handshake_admission.timeout;
                            match tokio::time::timeout_at(
                                registration_deadline,
                                acceptor.accept(stream),
                            )
                            .await
                            {
                                Ok(Ok(tls_stream)) => {
                                    let established_permit = match established
                                        .clone()
                                        .try_acquire_owned()
                                    {
                                        Ok(permit) => permit,
                                        Err(_) => {
                                            warn!(
                                                source = %remote_addr,
                                                "TLS established inbound connection limit reached"
                                            );
                                            return;
                                        }
                                    };
                                    debug!("TLS handshake with {} successful", remote_addr);
                                    let connection_metadata = verified_peer_metadata(
                                        tls_stream.get_ref().1.peer_certificates(),
                                    );
                                    if weak_tasks_for_connection
                                        .upgrade()
                                        .is_none_or(|tasks| tasks.is_closing())
                                    {
                                        return;
                                    }
                                    Self::handle_connection(
                                        tls_stream,
                                        TlsConnectionKey::inbound(
                                            remote_addr,
                                            local_addr,
                                            trust_context,
                                        ),
                                        local_addr,
                                        connections,
                                        event_tx,
                                        connection_metadata,
                                        weak_tasks_for_connection,
                                        next_connection_generation.fetch_add(1, Ordering::Relaxed),
                                        None,
                                        lifecycle,
                                        established_permit,
                                        registration_deadline,
                                        Some(permit),
                                    )
                                    .await;
                                }
                                Ok(Err(error)) => {
                                    drop(permit);
                                    // Warn, not error: a failed inbound
                                    // handshake is recoverable and usually
                                    // benign (misconfigured peer, expired
                                    // cert, port probe). The accept loop
                                    // resumes on the next iteration. Real
                                    // operators still see this at default
                                    // log levels; CI test output doesn't
                                    // look like a regression.
                                    warn!(
                                        destination = %remote_addr,
                                        error_class = tls_runtime_failure_class(&error),
                                        "TLS handshake failed"
                                    );
                                }
                                Err(_) => {
                                    drop(permit);
                                    warn!(
                                        destination = %remote_addr,
                                        timeout_ms = handshake_admission.timeout.as_millis(),
                                        "TLS handshake timed out"
                                    );
                                }
                            }
                        })
                        .await;
                }
                Err(e) => {
                    drop(permit);
                    error!("Failed to accept TCP connection: {}", e);
                }
            }
        }
    }

    /// Handle a TLS connection (server- or client-side). Generic over
    /// the stream type so the same read-loop / write-channel plumbing
    /// services both `tokio_rustls::server::TlsStream` (inbound) and
    /// `tokio_rustls::client::TlsStream` (outbound).
    async fn handle_connection<S>(
        tls_stream: S,
        key: TlsConnectionKey,
        local_addr: SocketAddr,
        connections: Arc<tokio::sync::Mutex<HashMap<TlsConnectionKey, TlsConnectionRecord>>>,
        event_tx: mpsc::Sender<TransportEvent>,
        connection_metadata: Option<TransportConnectionMetadata>,
        weak_tasks: Weak<TransportTaskSet>,
        generation: u64,
        registered: Option<tokio::sync::oneshot::Sender<()>>,
        lifecycle: ConnectionLifecycleConfig,
        _established_permit: OwnedSemaphorePermit,
        registration_deadline: tokio::time::Instant,
        handshake_permit: Option<OwnedSemaphorePermit>,
    ) where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let remote_addr = key.remote_addr;
        let (mut reader, mut writer) = tokio::io::split(tls_stream);
        let (tx, mut rx) = mpsc::channel::<Bytes>(lifecycle.writer_queue_capacity);
        let (activity_tx, mut activity_rx) = watch::channel(tokio::time::Instant::now());
        let (writer_done_tx, mut writer_done_rx) = watch::channel(false);

        let Some(tasks) = weak_tasks.upgrade() else {
            return;
        };
        let writer_activity = activity_tx.clone();
        let write_task = tokio::time::timeout_at(
            registration_deadline,
            tasks.spawn(async move {
                while let Some(data) = rx.recv().await {
                    let write = async {
                        writer.write_all(&data).await?;
                        writer.flush().await
                    };
                    match tokio::time::timeout(lifecycle.write_timeout, write).await {
                        Ok(Ok(())) => {
                            writer_activity.send_replace(tokio::time::Instant::now());
                        }
                        Ok(Err(e)) => {
                            error!("Failed to write to TLS stream: {}", e);
                            break;
                        }
                        Err(_) => {
                            warn!(destination = %remote_addr, "TLS write timed out");
                            break;
                        }
                    }
                }
                writer_done_tx.send_replace(true);
            }),
        )
        .await;
        let Some(write_task) = (match write_task {
            Ok(task) => task,
            Err(_) => {
                warn!(destination = %remote_addr, "TLS writer registration timed out");
                return;
            }
        }) else {
            return;
        };
        let write_task = AbortTaskOnDrop(write_task);

        let mut connections_guard =
            match tokio::time::timeout_at(registration_deadline, connections.lock()).await {
                Ok(connections) => connections,
                Err(_) => {
                    warn!(destination = %remote_addr, "TLS connection registration timed out");
                    return;
                }
            };
        connections_guard.insert(
            key.clone(),
            TlsConnectionRecord {
                generation,
                sender: tx.clone(),
            },
        );
        drop(connections_guard);
        if let Some(registered) = registered {
            let _ = registered.send(());
        }
        drop(handshake_permit);
        drop(tasks);

        // Buffered read loop with RFC 3261 §18.3 Content-Length framing,
        // plus RFC 5626 §3.5.1 keep-alive frame detection at buffer
        // offset 0. TLS records can split a single SIP message across
        // reads (or bundle several into one), so we accumulate into a
        // `BytesMut` and pull frames off the front.
        let mut buffer = BytesMut::with_capacity(8192);
        let mut tmp = vec![0u8; 8192];
        let tx_for_pong = tx.clone();
        let established_at = tokio::time::Instant::now();
        'connection: loop {
            let deadline = lifecycle.next_deadline(*activity_rx.borrow(), established_at);
            let read = tokio::select! {
                read = reader.read(&mut tmp) => Some(read),
                changed = activity_rx.changed() => {
                    if changed.is_err() {
                        break 'connection;
                    }
                    None
                }
                _ = writer_done_rx.changed() => break 'connection,
                _ = tokio::time::sleep_until(deadline) => {
                    debug!(destination = %remote_addr, "TLS connection lifecycle deadline reached");
                    break 'connection;
                }
            };
            let Some(read) = read else {
                continue;
            };
            match read {
                Ok(0) => break,
                Ok(n) => {
                    activity_tx.send_replace(tokio::time::Instant::now());
                    buffer.extend_from_slice(&tmp[..n]);
                    // Drain all complete frames (keep-alive or SIP).
                    // RFC 5626 frames are only recognised at offset 0;
                    // `try_consume_keepalive_frame` strips them, then we
                    // fall through to `try_parse_one` for stacked SIP
                    // messages.
                    loop {
                        match try_consume_keepalive_frame(&mut buffer) {
                            Some(KeepAliveFrame::Pong) => {
                                let _ = event_tx.try_send(TransportEvent::KeepAlivePongReceived {
                                    source: remote_addr,
                                    destination: local_addr,
                                });
                                continue;
                            }
                            Some(KeepAliveFrame::Ping) => {
                                // RFC 5626 §3.5.1: reply with CRLF pong.
                                if tx_for_pong.try_send(Bytes::from_static(b"\r\n")).is_err() {
                                    warn!(
                                        destination = %remote_addr,
                                        "TLS keepalive reply queue is unavailable"
                                    );
                                    break 'connection;
                                }
                                continue;
                            }
                            None => {}
                        }
                        match try_parse_one(&mut buffer) {
                            Ok(Some((message, raw_bytes))) => {
                                if event_tx
                                    .try_send(TransportEvent::MessageReceived {
                                        message,
                                        source: remote_addr,
                                        destination: local_addr,
                                        transport_type: TransportType::Tls,
                                        raw_bytes: Some(raw_bytes),
                                        timing: None,
                                        connection_metadata: connection_metadata.clone(),
                                    })
                                    .is_err()
                                {
                                    warn!(
                                        source = %remote_addr,
                                        "TLS event queue unavailable; closing flow instead of blocking lifecycle cleanup"
                                    );
                                    break 'connection;
                                }
                            }
                            Ok(None) => break,
                            Err(_) => break 'connection,
                        }
                    }
                }
                Err(e) => {
                    // `UnexpectedEof` is the normal close shape when
                    // the peer hangs up (RFC 8446 §6.1 close_notify
                    // followed by TCP FIN, or just TCP FIN against a
                    // long-running listener). Log at debug so a clean
                    // BYE → connection-close sequence doesn't look
                    // like a transport failure. Anything else
                    // (handshake-after-hello errors, broken pipe with
                    // unflushed records, etc.) stays ERROR.
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        debug!("TLS stream closed by peer ({})", remote_addr);
                    } else {
                        error!("Failed to read from TLS stream: {}", e);
                    }
                    break;
                }
            }
        }

        write_task.abort();

        // Emit ConnectionClosed *before* the registry eviction so any
        // observer (e.g. RFC 5626 OutboundFlow) sees the lifecycle
        // event before a subsequent `has_connection_to` query returns
        // false.
        let _ = event_tx.try_send(TransportEvent::ConnectionClosed {
            remote_addr,
            transport_type: TransportType::Tls,
        });

        {
            let mut connections_guard = connections.lock().await;
            remove_tls_connection_if_generation(&mut connections_guard, &key, generation);
        }

        debug!("TLS connection closed: {}", remote_addr);
    }

    /// Send data to a specific remote address. Auto-dials if no
    /// connection exists yet. Request sends supply SNI from the next-hop
    /// URI host; response/raw sends fall back to destination-address SNI.
    async fn send_to_addr(
        &self,
        data: Bytes,
        addr: SocketAddr,
        server_name: Option<ServerName<'static>>,
        prefer_inbound: bool,
    ) -> Result<()> {
        let server_name = server_name.unwrap_or_else(|| ip_to_server_name(addr));
        let outbound_key =
            TlsConnectionKey::outbound(addr, &server_name, self.outbound_trust_context);
        let existing = {
            let connections =
                tokio::time::timeout(self.handshake_admission.timeout, self.connections.lock())
                    .await
                    .map_err(|_| Error::ConnectionTimeout(addr))?;
            select_tls_sender(&connections, &outbound_key, prefer_inbound)?
        };
        if let Some(sender) = existing {
            match sender.try_send(data.clone()) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    return Err(Error::BufferCapacityExceeded)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }

        if self.role == TlsRole::ServerOnly {
            return Err(Error::InvalidState(format!(
                "TLS server-only transport has no existing connection to {}; outbound auto-dial is disabled",
                addr
            )));
        }

        // Auto-dial.
        self.connect_with_server_name(addr, server_name).await?;

        let sender =
            tokio::time::timeout(self.handshake_admission.timeout, self.connections.lock())
                .await
                .map_err(|_| Error::ConnectionTimeout(addr))?
                .get(&outbound_key)
                .ok_or_else(|| {
                    Error::Other(format!(
                        "TLS auto-dial succeeded but no connection registered for {}",
                        addr
                    ))
                })?
                .sender
                .clone();
        sender.try_send(data).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::BufferCapacityExceeded,
            mpsc::error::TrySendError::Closed(_) => Error::TransportClosed,
        })
    }

    /// Connect to a remote address. The SNI `ServerName` is derived
    /// from `remote_addr` — IP literal for IP destinations, falls back
    /// to "localhost" for the loopback IP (so default rustls hostname
    /// validation still works against test certs that include
    /// "localhost"). For hostname-based SNI use
    /// [`TlsTransport::connect_with_server_name`].
    pub async fn connect(&self, remote_addr: SocketAddr) -> Result<()> {
        let server_name = ip_to_server_name(remote_addr);
        self.connect_with_server_name(remote_addr, server_name)
            .await
    }

    /// Connect to a remote address with an explicit SNI server name.
    /// Prefer this over `connect` when the caller knows the URI's
    /// host (e.g. `sips:alice@sip.example.com` → `"sip.example.com"`).
    pub async fn connect_with_server_name(
        &self,
        remote_addr: SocketAddr,
        server_name: ServerName<'static>,
    ) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        let deadline = tokio::time::Instant::now() + self.handshake_admission.timeout;

        let key =
            TlsConnectionKey::outbound(remote_addr, &server_name, self.outbound_trust_context);
        {
            let mut connections = tokio::time::timeout_at(deadline, self.connections.lock())
                .await
                .map_err(|_| Error::ConnectionTimeout(remote_addr))?;
            if connections
                .get(&key)
                .is_some_and(|record| !record.sender.is_closed())
            {
                return Ok(());
            }
            connections.remove(&key);
        }

        let (sni_present, sni_len) = sni_diagnostic_metadata(&server_name);
        debug!(
            destination = %remote_addr,
            sni_present,
            sni_len,
            "TLS dial"
        );

        let connector = self.connector.clone();
        let connections = self.connections.clone();
        let event_tx = self
            .event_tx
            .clone()
            .ok_or_else(|| Error::TlsHandshakeFailed("TLS transport has no event sender".into()))?;
        let local_addr = self.local_addr;
        let coordinator = self.outbound_dials.clone();
        let lifecycle = self.lifecycle;
        let next_generation = self.next_connection_generation.clone();
        let managed_tasks = self.tasks.clone();
        let connection_tasks = self.tasks.clone();
        let outbound_established = self.outbound_established.clone();

        match coordinator.begin(key.clone())? {
            DialAdmission::Follower { outcome, .. } => {
                OutboundDialCoordinator::<TlsConnectionKey>::wait(outcome, deadline, remote_addr)
                    .await?;
                let connections = tokio::time::timeout_at(deadline, self.connections.lock())
                    .await
                    .map_err(|_| Error::ConnectionTimeout(remote_addr))?;
                if connections
                    .get(&key)
                    .is_some_and(|record| !record.sender.is_closed())
                {
                    Ok(())
                } else {
                    Err(Error::TransportClosed)
                }
            }
            DialAdmission::Leader {
                key,
                flight,
                _pending,
                cancellation,
            } => {
                let coordinator_for_task = coordinator.clone();
                let pending_permit = _pending;
                managed_tasks
                    .run(async move {
                        let mut cancellation = cancellation;
                        let _pending_permit = pending_permit;
                        let result = async {
                            let _handshake = coordinator_for_task
                                .acquire_handshake(deadline, remote_addr)
                                .await?;
                            {
                                let mut connections =
                                    tokio::time::timeout_at(deadline, connections.lock())
                                        .await
                                        .map_err(|_| Error::ConnectionTimeout(remote_addr))?;
                                if connections
                                    .get(&key)
                                    .is_some_and(|record| !record.sender.is_closed())
                                {
                                    return Ok(());
                                }
                                connections.remove(&key);
                            }

                            let established_permit = outbound_established
                                .try_acquire_owned()
                                .map_err(|_| Error::ConnectionPoolExhausted)?;
                            let tls_stream = tokio::time::timeout_at(deadline, async {
                                let tcp_stream = TcpStream::connect(remote_addr)
                                    .await
                                    .map_err(|e| Error::ConnectFailed(remote_addr, e))?;
                                connector
                                    .connect(server_name, tcp_stream)
                                    .await
                                    .map_err(|error| {
                                        classify_tls_runtime_error(
                                            error,
                                            format!(
                                                "TLS client handshake failed for {remote_addr}"
                                            ),
                                        )
                                    })
                            })
                            .await
                            .map_err(|_| Error::ConnectionTimeout(remote_addr))??;
                            if connection_tasks.is_closing() {
                                return Err(Error::TransportClosed);
                            }
                            info!("TLS handshake to {} succeeded", remote_addr);

                            let generation = next_generation.fetch_add(1, Ordering::Relaxed);
                            let (registered_tx, registered_rx) = tokio::sync::oneshot::channel();
                            let weak_tasks = Arc::downgrade(&connection_tasks);
                            let connection_task = tokio::time::timeout_at(
                                deadline,
                                connection_tasks.spawn(Self::handle_connection(
                                    tls_stream,
                                    key.clone(),
                                    local_addr,
                                    connections,
                                    event_tx,
                                    None,
                                    weak_tasks,
                                    generation,
                                    Some(registered_tx),
                                    lifecycle,
                                    established_permit,
                                    deadline,
                                    None,
                                )),
                            )
                            .await
                            .map_err(|_| Error::ConnectionTimeout(remote_addr))?
                            .ok_or(Error::TransportClosed)?;
                            match tokio::time::timeout_at(deadline, registered_rx).await {
                                Ok(Ok(())) => Ok(()),
                                Ok(Err(_)) => {
                                    connection_task.abort();
                                    Err(Error::TransportClosed)
                                }
                                Err(_) => {
                                    connection_task.abort();
                                    Err(Error::ConnectionTimeout(remote_addr))
                                }
                            }
                        }
                        .await;
                        coordinator_for_task.complete(&key, &flight, &result, &mut cancellation);
                        result
                    })
                    .await
            }
        }
    }
}

fn select_tls_sender(
    connections: &HashMap<TlsConnectionKey, TlsConnectionRecord>,
    outbound_key: &TlsConnectionKey,
    prefer_inbound: bool,
) -> Result<Option<mpsc::Sender<Bytes>>> {
    if prefer_inbound {
        if let Some(record) = connections.iter().find_map(|(key, record)| {
            (key.remote_addr == outbound_key.remote_addr
                && key.direction == ConnectionDirection::Inbound
                && !record.sender.is_closed())
            .then_some(record)
        }) {
            return Ok(Some(record.sender.clone()));
        }

        // Client-only/RFC 5626 flows may receive requests over a DNS-SNI
        // authenticated outbound connection. A response has no Request-URI
        // authority, so reuse that flow only when it is unambiguous.
        let mut outbound = connections.iter().filter(|(key, record)| {
            key.remote_addr == outbound_key.remote_addr
                && key.direction == ConnectionDirection::Outbound
                && !record.sender.is_closed()
        });
        let first = outbound.next().map(|(_, record)| record.sender.clone());
        if outbound.next().is_some() {
            return Err(Error::InvalidState(format!(
                "Multiple authenticated TLS flows exist for {}; an explicit authority is required",
                outbound_key.remote_addr
            )));
        }
        if first.is_some() {
            return Ok(first);
        }
    }
    Ok(connections
        .get(outbound_key)
        .filter(|record| !record.sender.is_closed())
        .map(|record| record.sender.clone()))
}

fn normalized_server_name(server_name: &ServerName<'_>) -> String {
    match server_name {
        ServerName::DnsName(name) => name.as_ref().trim_end_matches('.').to_ascii_lowercase(),
        ServerName::IpAddress(address) => std::net::IpAddr::from(address.clone()).to_string(),
        _ => "unsupported-server-name".to_string(),
    }
}

fn sni_diagnostic_metadata(server_name: &ServerName<'_>) -> (bool, usize) {
    match server_name {
        ServerName::DnsName(name) => (true, name.as_ref().len()),
        ServerName::IpAddress(_) => (false, 0),
        _ => (false, 0),
    }
}

pub(crate) fn tls_runtime_failure_class(error: &std::io::Error) -> &'static str {
    let certificate_failure = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<rustls::Error>())
        .is_some_and(|error| {
            matches!(
                error,
                rustls::Error::InvalidCertificate(_)
                    | rustls::Error::InvalidCertRevocationList(_)
                    | rustls::Error::NoCertificatesPresented
                    | rustls::Error::AlertReceived(
                        rustls::AlertDescription::NoCertificate
                            | rustls::AlertDescription::BadCertificate
                            | rustls::AlertDescription::UnsupportedCertificate
                            | rustls::AlertDescription::CertificateRevoked
                            | rustls::AlertDescription::CertificateExpired
                            | rustls::AlertDescription::CertificateUnknown
                            | rustls::AlertDescription::UnknownCA
                            | rustls::AlertDescription::BadCertificateStatusResponse
                            | rustls::AlertDescription::BadCertificateHashValue
                            | rustls::AlertDescription::CertificateRequired
                    )
            )
        });
    if certificate_failure {
        "tls_certificate_failed"
    } else {
        "tls_handshake_failed"
    }
}

pub(crate) fn classify_tls_runtime_error(error: std::io::Error, context: String) -> Error {
    if tls_runtime_failure_class(&error) == "tls_certificate_failed" {
        Error::TlsCertificateError(context)
    } else {
        Error::TlsHandshakeFailed(context)
    }
}

#[async_trait]
impl Transport for TlsTransport {
    async fn send_message(
        &self,
        message: rvoip_sip_core::Message,
        destination: SocketAddr,
    ) -> Result<()> {
        validate_typed_outbound_message(&message)?;
        // `Message::to_bytes` produces wire-format SIP (header CRLFs +
        // trailing CRLF separator + body) — required by RFC 3261 §7.2.
        // `to_string()` is for display/debug only and omits the final
        // separator, which then breaks Content-Length framing on the
        // peer's read side.
        let server_name = tls_server_name_for_message(&message, destination);
        let prefer_inbound = matches!(message, rvoip_sip_core::Message::Response(_));
        let bytes = message.to_bytes();
        self.send_to_addr(bytes.into(), destination, server_name, prefer_inbound)
            .await
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> Result<()> {
        let _close_guard = self.close_gate.lock().await;
        self.closed.store(true, Ordering::SeqCst);
        self.outbound_dials.close();
        self.inbound_established.close();
        self.outbound_established.close();
        self.tasks.close().await;
        self.connections.lock().await.clear();
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn supports_tls(&self) -> bool {
        true
    }

    fn has_connection_to(&self, remote_addr: SocketAddr) -> bool {
        // `try_lock` so this is non-blocking — the multiplexer may call
        // it from inside its own dispatch path. A momentary lock-busy
        // is acceptable to report `false` (the multiplexer will fall
        // through to its default transport).
        match self.connections.try_lock() {
            Ok(guard) => guard
                .iter()
                .any(|(key, record)| key.remote_addr == remote_addr && !record.sender.is_closed()),
            Err(_) => false,
        }
    }

    async fn send_raw(&self, destination: SocketAddr, data: Bytes) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        // RFC 5626 keep-alive: only reuse an existing TLS connection.
        // A fresh dial would defeat the purpose — the flow we'd keep
        // alive is already gone.
        let sender = {
            let connections = self.connections.lock().await;
            let candidates = connections.iter().filter(|(key, record)| {
                key.remote_addr == destination && !record.sender.is_closed()
            });
            let inbound = candidates
                .clone()
                .find(|(key, _)| key.direction == ConnectionDirection::Inbound)
                .map(|(_, record)| record.sender.clone());
            inbound.or_else(|| {
                let mut outbound =
                    candidates.filter(|(key, _)| key.direction == ConnectionDirection::Outbound);
                let first = outbound.next().map(|(_, record)| record.sender.clone());
                if outbound.next().is_some() {
                    None
                } else {
                    first
                }
            })
        };
        let Some(sender) = sender else {
            return Err(Error::InvalidState(format!(
                "No unambiguous active TLS connection to {} for send_raw",
                destination
            )));
        };
        sender.try_send(data).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::BufferCapacityExceeded,
            mpsc::error::TrySendError::Closed(_) => Error::TransportClosed,
        })
    }

    async fn send_message_raw(&self, bytes: Bytes, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        // No server-name hint — verbatim-bytes callers pre-canonicalised
        // their request; we route by destination IP. Auto-dial when no
        // pooled connection exists (modulo ServerOnly role).
        self.send_to_addr(bytes, destination, None, true).await
    }
}

#[cfg(test)]
mod auth_boundary_tests {
    use super::*;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
    use rvoip_sip_core::{CallId, Message, Method, Request, Response, StatusCode, Uri};
    use std::time::Duration;

    fn write_test_certificate() -> (tempfile::TempDir, PathBuf, PathBuf) {
        use std::io::Write;

        let directory = tempfile::tempdir().unwrap();
        let cert_path = directory.path().join("server.pem");
        let key_path = directory.path().join("server.key");
        let certificate =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        std::fs::File::create(&cert_path)
            .unwrap()
            .write_all(certificate.cert.pem().as_bytes())
            .unwrap();
        std::fs::File::create(&key_path)
            .unwrap()
            .write_all(certificate.signing_key.serialize_pem().as_bytes())
            .unwrap();
        (directory, cert_path, key_path)
    }

    #[test]
    fn response_reuses_only_an_unambiguous_authenticated_outbound_tls_flow() {
        let destination: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let dns_name = ServerName::try_from("sip.example.test".to_string()).unwrap();
        let response_key =
            TlsConnectionKey::outbound(destination, &ip_to_server_name(destination), 7);
        let (first_tx, _first_rx) = mpsc::channel(1);
        let mut connections = HashMap::new();
        connections.insert(
            TlsConnectionKey::outbound(destination, &dns_name, 7),
            TlsConnectionRecord {
                generation: 1,
                sender: first_tx.clone(),
            },
        );

        let selected = select_tls_sender(&connections, &response_key, true)
            .unwrap()
            .expect("sole authenticated flow must carry the response");
        assert!(selected.same_channel(&first_tx));

        let (second_tx, _second_rx) = mpsc::channel(1);
        let second_name = ServerName::try_from("other.example.test".to_string()).unwrap();
        connections.insert(
            TlsConnectionKey::outbound(destination, &second_name, 7),
            TlsConnectionRecord {
                generation: 2,
                sender: second_tx,
            },
        );
        assert!(matches!(
            select_tls_sender(&connections, &response_key, true),
            Err(Error::InvalidState(_))
        ));
    }

    #[tokio::test]
    async fn typed_tls_send_rejects_auth_before_connect() {
        let (transport, _rx) = TlsTransport::client_only(
            "127.0.0.1:0".parse().unwrap(),
            None,
            TlsClientConfig::default(),
        )
        .await
        .unwrap();
        let destination = "127.0.0.1:9".parse().unwrap();
        let mut request = SimpleRequestBuilder::new(Method::Options, "sips:example.com")
            .unwrap()
            .build();
        request.headers.push(TypedHeader::Other(
            HeaderName::Other("proxy-Authorization".into()),
            HeaderValue::Raw(b"Digest safe\r\nX-Injected: tls".to_vec()),
        ));

        let invalid_reason =
            Response::new(StatusCode::Ok).with_reason("OK\r\nX-Injected: tls-reason-secret");
        let mut invalid_header = Request::new(Method::Options, Uri::sip("example.test"));
        invalid_header.headers.push(TypedHeader::CallId(CallId::new(
            "safe\r\nX-Injected: tls-header-secret",
        )));
        let invalid_uri = Request::new(
            Method::Options,
            Uri::custom("sips:bob@example.test\r\nX-Injected: tls-uri-secret"),
        );

        for message in [
            Message::Request(request),
            Message::Response(invalid_reason),
            Message::Request(invalid_header),
            Message::Request(invalid_uri),
        ] {
            let error = transport
                .send_message(message, destination)
                .await
                .expect_err("typed TLS send must reject unsafe fields");
            assert!(matches!(error, Error::ProtocolError(_)));
            assert!(!error.to_string().contains("X-Injected"));
        }
        transport.close().await.unwrap();
    }

    #[test]
    fn stale_tls_reader_cannot_evict_replacement() {
        let remote_addr = "127.0.0.1:5061".parse().unwrap();
        let key = TlsConnectionKey::outbound(remote_addr, &ip_to_server_name(remote_addr), 7);
        let (sender, _receiver) = mpsc::channel(1);
        let mut connections = HashMap::from([(
            key.clone(),
            TlsConnectionRecord {
                generation: 2,
                sender,
            },
        )]);

        assert!(!remove_tls_connection_if_generation(
            &mut connections,
            &key,
            1,
        ));
        assert_eq!(connections[&key].generation, 2);
        assert!(remove_tls_connection_if_generation(
            &mut connections,
            &key,
            2,
        ));
    }

    #[tokio::test]
    async fn outbound_tls_handshake_has_end_to_end_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = listener.local_addr().unwrap();
        let stalled = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let (transport, _events) = TlsTransport::client_only_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            None,
            TlsClientConfig::default(),
            HandshakeAdmissionConfig::new(Duration::from_millis(50), 1),
        )
        .await
        .unwrap();

        assert!(matches!(
            transport.connect(destination).await,
            Err(Error::ConnectionTimeout(address)) if address == destination
        ));
        transport.close().await.unwrap();
        stalled.abort();
    }

    #[tokio::test]
    async fn close_cancels_and_joins_outbound_tls_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let stalled = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
        });
        let (transport, _events) = TlsTransport::client_only_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            None,
            TlsClientConfig::default(),
            HandshakeAdmissionConfig::new(Duration::from_secs(30), 1),
        )
        .await
        .unwrap();
        let transport = Arc::new(transport);
        let dialing = {
            let transport = transport.clone();
            tokio::spawn(async move { transport.connect(destination).await })
        };
        accepted_rx.await.unwrap();
        transport.close().await.unwrap();

        assert!(matches!(
            dialing.await.unwrap(),
            Err(Error::TransportClosed)
        ));
        stalled.abort();
    }

    #[tokio::test]
    async fn concurrent_tls_dials_to_one_destination_are_singleflight() {
        let (_directory, cert_path, key_path) = write_test_certificate();
        let (server, _server_events) =
            TlsTransport::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path, None)
                .await
                .unwrap();
        let destination = server.local_addr().unwrap();
        let (client, _client_events) = TlsTransport::client_only_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            None,
            TlsClientConfig {
                extra_ca_path: Some(cert_path),
                ..TlsClientConfig::default()
            },
            HandshakeAdmissionConfig::new(Duration::from_secs(2), 8),
        )
        .await
        .unwrap();

        let (first, second) =
            tokio::join!(client.connect(destination), client.connect(destination));
        assert!(first.is_ok(), "first dial failed: {first:?}");
        assert!(second.is_ok(), "second dial failed: {second:?}");
        tokio::task::yield_now().await;
        assert_eq!(client.connections.lock().await.len(), 1);
        assert_eq!(server.connections.lock().await.len(), 1);

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[cfg(feature = "dev-insecure-tls")]
    #[tokio::test]
    async fn same_address_different_tls_authorities_never_share_connection() {
        let (_directory, cert_path, key_path) = write_test_certificate();
        let (server, _server_events) =
            TlsTransport::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path, None)
                .await
                .unwrap();
        let destination = server.local_addr().unwrap();
        let (client, _client_events) = TlsTransport::client_only(
            "127.0.0.1:0".parse().unwrap(),
            None,
            TlsClientConfig {
                insecure_skip_verify: true,
                ..TlsClientConfig::default()
            },
        )
        .await
        .unwrap();

        client
            .connect_with_server_name(
                destination,
                ServerName::try_from("authority-a.example".to_string()).unwrap(),
            )
            .await
            .unwrap();
        client
            .connect_with_server_name(
                destination,
                ServerName::try_from("authority-b.example".to_string()).unwrap(),
            )
            .await
            .unwrap();

        let connections = client.connections.lock().await;
        assert_eq!(connections.len(), 2);
        let authorities = connections
            .keys()
            .map(|key| key.authority.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert!(authorities.contains("authority-a.example"));
        assert!(authorities.contains("authority-b.example"));
        drop(connections);

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[tokio::test]
    async fn connection_registry_lock_is_inside_tls_dial_deadline() {
        let (client, _events) = TlsTransport::client_only_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            None,
            TlsClientConfig::default(),
            HandshakeAdmissionConfig::new(Duration::from_millis(40), 1),
        )
        .await
        .unwrap();
        let destination = "127.0.0.1:9".parse().unwrap();
        let _registry_guard = client.connections.lock().await;
        let started = tokio::time::Instant::now();
        assert!(matches!(
            client.connect(destination).await,
            Err(Error::ConnectionTimeout(address)) if address == destination
        ));
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn tls_dial_diagnostics_expose_only_sni_presence_and_length() {
        const SECRET_SNI: &str = "tenant-secret.sip.example";
        let server_name = ServerName::try_from(SECRET_SNI.to_string()).unwrap();
        assert_eq!(
            sni_diagnostic_metadata(&server_name),
            (true, SECRET_SNI.len())
        );

        let source = include_str!("mod.rs");
        for fragments in [
            ["SNI ", "{:?}"],
            ["server_name = ", "?"],
            ["%server", "_name"],
        ] {
            assert!(!source.contains(&fragments.concat()));
        }
        assert!(source.contains("sni_present"));
        assert!(source.contains("sni_len"));
    }

    #[test]
    fn tls_and_websocket_handshakes_never_format_lower_errors() {
        let tls = include_str!("mod.rs");
        let websocket = include_str!("../ws/mod.rs");
        let listener = include_str!("../ws/listener.rs");
        for fragments in [
            ["TLS handshake with {}", " failed: {}"],
            ["TLS handshake to {}", ": {}"],
            ["WSS client handshake with {}", ": {}"],
            ["WSS TLS handshake with {}", " failed: {}"],
            ["WebSocket handshake failed with {}", ": {}"],
            ["WebSocketHandshakeFailed(e", ".to_string())"],
        ] {
            let forbidden = fragments.concat();
            assert!(
                !tls.contains(&forbidden)
                    && !websocket.contains(&forbidden)
                    && !listener.contains(&forbidden),
                "lower handshake error relay returned: {forbidden}"
            );
        }
    }

    #[test]
    fn runtime_tls_failures_preserve_certificate_vs_handshake_class() {
        let certificate = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding),
        );
        assert_eq!(
            tls_runtime_failure_class(&certificate),
            "tls_certificate_failed"
        );
        assert!(matches!(
            classify_tls_runtime_error(certificate, "certificate".to_string()),
            Error::TlsCertificateError(_)
        ));

        for alert in [
            rustls::AlertDescription::NoCertificate,
            rustls::AlertDescription::BadCertificate,
            rustls::AlertDescription::UnsupportedCertificate,
            rustls::AlertDescription::CertificateRevoked,
            rustls::AlertDescription::CertificateExpired,
            rustls::AlertDescription::CertificateUnknown,
            rustls::AlertDescription::UnknownCA,
            rustls::AlertDescription::BadCertificateStatusResponse,
            rustls::AlertDescription::BadCertificateHashValue,
            rustls::AlertDescription::CertificateRequired,
        ] {
            let certificate_alert = std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                rustls::Error::AlertReceived(alert),
            );
            assert_eq!(
                tls_runtime_failure_class(&certificate_alert),
                "tls_certificate_failed"
            );
            assert!(matches!(
                classify_tls_runtime_error(certificate_alert, "certificate alert".into()),
                Error::TlsCertificateError(_)
            ));
        }

        let invalid_crl = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertRevocationList(rustls::CertRevocationListError::BadSignature),
        );
        assert_eq!(
            tls_runtime_failure_class(&invalid_crl),
            "tls_certificate_failed"
        );
        assert!(matches!(
            classify_tls_runtime_error(invalid_crl, "invalid CRL".into()),
            Error::TlsCertificateError(_)
        ));

        let handshake = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::General("handshake".to_string()),
        );
        assert_eq!(
            tls_runtime_failure_class(&handshake),
            "tls_handshake_failed"
        );
        assert!(matches!(
            classify_tls_runtime_error(handshake, "handshake".to_string()),
            Error::TlsHandshakeFailed(_)
        ));
    }
}

pub(crate) fn tls_server_name_for_message(
    message: &rvoip_sip_core::Message,
    destination: SocketAddr,
) -> Option<ServerName<'static>> {
    let rvoip_sip_core::Message::Request(request) = message else {
        return Some(ip_to_server_name(destination));
    };

    match &request.uri().host {
        Host::Domain(domain) => ServerName::try_from(domain.to_string()).ok(),
        Host::Address(_) => Some(ip_to_server_name(destination)),
    }
}

/// A keep-alive frame pulled off the front of a TLS receive buffer.
#[derive(Debug)]
enum KeepAliveFrame {
    /// CRLF received — server pong.
    Pong,
    /// CRLFCRLF received — server-initiated ping.
    Ping,
}

/// Strip a leading RFC 5626 §3.5.1 keep-alive frame (pong / ping) off
/// `buffer` if present. Mirrors the logic in
/// `transport::tcp::connection::TcpConnection::receive_frame` so TCP
/// and TLS see the same semantics.
fn try_consume_keepalive_frame(buffer: &mut BytesMut) -> Option<KeepAliveFrame> {
    if buffer.len() >= 4 && &buffer[0..4] == b"\r\n\r\n" {
        buffer.advance(4);
        return Some(KeepAliveFrame::Ping);
    }
    if buffer.len() >= 2 && &buffer[0..2] == b"\r\n" {
        buffer.advance(2);
        return Some(KeepAliveFrame::Pong);
    }
    None
}

/// Build the inbound rustls server configuration shared by SIP TLS and WSS.
pub(crate) fn build_server_config(
    cert_path: &Path,
    key_path: &Path,
    client_auth: &TlsServerClientAuthConfig,
    transport_label: &str,
) -> Result<ServerConfig> {
    let certs = load_certs(cert_path)?;
    let key = load_private_key(key_path)?;
    let builder = ServerConfig::builder();
    let builder = match client_auth.mode {
        TlsClientAuthMode::Disabled => builder.with_no_client_auth(),
        TlsClientAuthMode::Optional | TlsClientAuthMode::Required => {
            let ca_path = client_auth.client_ca_path.as_ref().ok_or_else(|| {
                Error::InvalidState(format!(
                    "{} client authentication requires client_ca_path",
                    transport_label
                ))
            })?;
            let mut roots = RootCertStore::empty();
            for cert in load_certs(ca_path)? {
                roots.add(cert).map_err(|error| {
                    Error::TlsCertificateError(format!(
                        "{} client CA {} is invalid: {}",
                        transport_label,
                        ca_path.display(),
                        error
                    ))
                })?;
            }
            if roots.is_empty() {
                return Err(Error::InvalidState(format!(
                    "{} client CA bundle {} contains no certificates",
                    transport_label,
                    ca_path.display()
                )));
            }
            let verifier = WebPkiClientVerifier::builder(Arc::new(roots));
            let verifier = if client_auth.mode == TlsClientAuthMode::Optional {
                verifier.allow_unauthenticated().build()
            } else {
                verifier.build()
            }
            .map_err(|error| {
                Error::TlsCertificateError(format!(
                    "{} client certificate verifier: {}",
                    transport_label, error
                ))
            })?;
            builder.with_client_cert_verifier(verifier)
        }
    };
    builder.with_single_cert(certs, key).map_err(|error| {
        Error::TlsCertificateError(format!(
            "{} server certificate/key config: {}",
            transport_label, error
        ))
    })
}

/// Convert the successfully verified peer certificate chain retained by
/// rustls into bounded transport metadata. `None` means the optional verifier
/// admitted an unauthenticated client or client authentication was disabled.
pub(crate) fn verified_peer_metadata(
    peer_certificates: Option<&[CertificateDer<'static>]>,
) -> Option<TransportConnectionMetadata> {
    let certificates = peer_certificates?;
    let leaf = certificates.first()?;
    let digest = Sha256::digest(leaf.as_ref());
    let mut fingerprint = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut fingerprint, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Some(TransportConnectionMetadata {
        tls_peer_identity: TlsPeerIdentity {
            leaf_certificate_sha256: fingerprint,
            presented_chain_len: certificates.len(),
        },
    })
}

/// Build a rustls `ClientConfig` honouring the supplied
/// [`TlsClientConfig`]. Default behaviour: load system roots via
/// `rustls-native-certs`, fall back to bundled `webpki-roots`, refuse
/// any cert that fails standard validation. Optional extras: an extra
/// CA bundle (added to the same root store) and an insecure-skip mode
/// (dev only — accepts any cert without identity verification).
pub(crate) fn build_client_config(cfg: &TlsClientConfig) -> Result<ClientConfig> {
    if cfg.client_cert_path.is_some() ^ cfg.client_key_path.is_some() {
        return Err(Error::InvalidState(
            "TLS client certificate and key must be provided together".to_string(),
        ));
    }

    #[cfg(feature = "dev-insecure-tls")]
    {
        if cfg.insecure_skip_verify {
            warn!(
                "TLS client built with insecure_skip_verify=true — \
                 server certificates will NOT be validated. Dev only."
            );
            let builder = ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier));
            return match (&cfg.client_cert_path, &cfg.client_key_path) {
                (Some(cert_path), Some(key_path)) => {
                    let certs = load_certs(cert_path)?;
                    let key = load_private_key(key_path)?;
                    builder.with_client_auth_cert(certs, key).map_err(|e| {
                        Error::TlsCertificateError(format!(
                            "TLS client certificate/key config failed: {}",
                            e
                        ))
                    })
                }
                _ => Ok(builder.with_no_client_auth()),
            };
        }
    }
    #[cfg(not(feature = "dev-insecure-tls"))]
    {
        // Field exists (kept for API stability) but has no effect under
        // the default build — the `InsecureCertVerifier` type isn't even
        // compiled.
        let _ = cfg.insecure_skip_verify;
    }

    // System trust anchors are loaded once per process and cached. Reading the
    // OS trust store (`rustls_native_certs::load_native_certs()` → the macOS
    // keychain via the Security framework) can take *seconds* — pathologically
    // so on some hosts — and the anchors don't change over a process's
    // lifetime. Load them once, then clone the cached store per config; extras
    // supplied by the caller are still added below.
    let mut root_store = {
        static SYSTEM_ROOTS: std::sync::OnceLock<RootCertStore> = std::sync::OnceLock::new();
        SYSTEM_ROOTS
            .get_or_init(|| {
                let mut root_store = RootCertStore::empty();
                let mut loaded_any_system = false;
                let certs = rustls_native_certs::load_native_certs();
                for cert in certs.certs {
                    if root_store.add(cert).is_ok() {
                        loaded_any_system = true;
                    }
                }
                if !certs.errors.is_empty() {
                    warn!(
                        failed_anchor_count = certs.errors.len(),
                        "TLS client failed to load system trust anchors"
                    );
                }
                if loaded_any_system {
                    debug!(
                        "TLS client root store loaded {} system certs",
                        root_store.len()
                    );
                } else if !certs.errors.is_empty() {
                    warn!(
                        error_class = "system_trust_store_unavailable",
                        "TLS client is falling back to bundled trust roots"
                    );
                } else {
                    warn!(
                        "TLS client: no system trust anchors found; falling back to webpki-roots"
                    );
                }
                if !loaded_any_system {
                    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                    debug!(
                        "TLS client root store fell back to webpki-roots ({} anchors)",
                        root_store.len()
                    );
                }
                root_store
            })
            .clone()
    };

    if let Some(extra_path) = &cfg.extra_ca_path {
        let extras = load_certs(extra_path)?;
        for cert in extras {
            root_store.add(cert).map_err(|e| {
                Error::TlsCertificateError(format!(
                    "Failed to add extra CA from {}: {}",
                    extra_path.display(),
                    e
                ))
            })?;
        }
        info!(
            root_count = root_store.len(),
            extra_ca_configured = true,
            "TLS client added configured trust roots"
        );
    }

    let builder = ClientConfig::builder().with_root_certificates(root_store);
    match (&cfg.client_cert_path, &cfg.client_key_path) {
        (Some(cert_path), Some(key_path)) => {
            let certs = load_certs(cert_path)?;
            let key = load_private_key(key_path)?;
            builder.with_client_auth_cert(certs, key).map_err(|e| {
                Error::TlsCertificateError(format!(
                    "TLS client certificate/key config failed: {}",
                    e
                ))
            })
        }
        _ => Ok(builder.with_no_client_auth()),
    }
}

/// Cert verifier that accepts every server cert. Dev only — gated
/// behind the `dev-insecure-tls` Cargo feature so production builds
/// physically cannot bypass TLS validation.
#[cfg(feature = "dev-insecure-tls")]
#[derive(Debug)]
struct InsecureCertVerifier;

#[cfg(feature = "dev-insecure-tls")]
impl ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

/// Best-effort SNI server-name from a destination `SocketAddr`.
/// Loopback maps to `"localhost"` so test certs that include the
/// `localhost` SAN match.
pub(crate) fn ip_to_server_name(addr: SocketAddr) -> ServerName<'static> {
    if addr.ip().is_loopback() {
        if let Ok(name) = ServerName::try_from("localhost".to_string()) {
            return name;
        }
    }
    ServerName::from(addr.ip()).to_owned()
}

/// Load PEM-encoded certificates from a file.
pub(crate) fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let mut cert_file = File::open(path).map_err(|error| {
        Error::TlsCertificateError(format!(
            "failed to open certificate (I/O class {:?})",
            error.kind()
        ))
    })?;
    let mut cert_data = Vec::new();
    cert_file.read_to_end(&mut cert_data).map_err(|error| {
        Error::TlsCertificateError(format!(
            "failed to read certificate (I/O class {:?})",
            error.kind()
        ))
    })?;
    rustls_pemfile::certs(&mut cert_data.as_slice())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| {
            let _ = e;
            Error::TlsCertificateError("failed to parse certificate".to_string())
        })
}

/// Load a PEM-encoded private key from a file.
pub(crate) fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let mut key_file = File::open(path).map_err(|error| {
        Error::TlsCertificateError(format!(
            "failed to open private key (I/O class {:?})",
            error.kind()
        ))
    })?;
    let mut key_data = Vec::new();
    key_file.read_to_end(&mut key_data).map_err(|error| {
        Error::TlsCertificateError(format!(
            "failed to read private key (I/O class {:?})",
            error.kind()
        ))
    })?;
    rustls_pemfile::private_key(&mut key_data.as_slice())
        .map_err(|e| {
            let _ = e;
            Error::TlsCertificateError("failed to parse private key".to_string())
        })?
        .ok_or_else(|| Error::TlsCertificateError("no private key found".to_string()))
}
/// Try to parse a single complete SIP message off the front of `buffer`.
///
/// Returns `Ok(Some(message))` (and removes those bytes) when a complete
/// message is present per RFC 3261 §18.3 Content-Length framing,
/// `Ok(None)` when more bytes are needed, or an error for malformed or
/// over-limit framing. Mirrors
/// `transport::tcp::connection::TcpConnection::try_parse_message` so TLS
/// framing matches TCP's behaviour exactly.
fn try_parse_one(buffer: &mut BytesMut) -> Result<Option<(rvoip_sip_core::Message, bytes::Bytes)>> {
    if buffer.is_empty() {
        return Ok(None);
    }

    let frame = match inspect_sip_frame_with_policy(buffer, SipFramingPolicy::Stream) {
        Ok(SipFrameStatus::Incomplete { .. }) => return Ok(None),
        Ok(SipFrameStatus::Complete(frame)) => frame,
        Err(error) => {
            warn!(
                error_class = error.class(),
                "Rejecting malformed SIP TLS frame"
            );
            return Err(Error::ParseError(error.to_string()));
        }
    };

    // Snapshot the wire bytes BEFORE parsing so we can hand them
    // downstream byte-exact (RFC 8224 STIR/SHAKEN, SBC signature
    // preservation, replay tooling). Matches the TCP path's shape.
    let raw_bytes = bytes::Bytes::copy_from_slice(&buffer[..frame.total_bytes]);
    match rvoip_sip_core::parse_message(&raw_bytes) {
        Ok(message) => {
            buffer.advance(frame.total_bytes);
            Ok(Some((message, raw_bytes)))
        }
        Err(_) => {
            warn!(
                error_class = "sip-syntax",
                "Rejecting malformed SIP TLS message"
            );
            Err(Error::ParseError(
                "SIP parser rejected framed TLS message".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod inbound_framing_tests {
    use super::*;
    use rvoip_sip_core::framing::{MAX_SIP_BODY_BYTES, MAX_SIP_HEADER_BYTES};

    fn minimal_request(content_length_headers: &[u8], body: &[u8]) -> Vec<u8> {
        let mut message = b"OPTIONS sip:service.example SIP/2.0\r\n".to_vec();
        message.extend_from_slice(content_length_headers);
        message.extend_from_slice(b"\r\n\r\n");
        message.extend_from_slice(body);
        message
    }

    #[test]
    fn tls_framing_accepts_compact_and_hcolon_content_length() {
        for header in [b"l: 4".as_slice(), b"Content-Length \t : 4".as_slice()] {
            let message = minimal_request(header, b"body");
            let mut buffer = BytesMut::from(message.as_slice());
            let (parsed, raw) = try_parse_one(&mut buffer)
                .unwrap()
                .expect("Content-Length frame");
            assert!(buffer.is_empty());
            assert_eq!(raw.as_ref(), message);
            let rvoip_sip_core::Message::Request(request) = parsed else {
                panic!("request expected");
            };
            assert_eq!(request.body(), b"body");
        }
    }

    #[test]
    fn tls_rejects_all_ambiguous_content_lengths_without_consuming_following_request() {
        let valid_second = minimal_request(b"Content-Length: 0", b"");
        for (headers, class) in [
            (b"Via: missing-length".as_slice(), "missing-content-length"),
            (
                b"Content-Length: 0\r\nContent-Length: 0".as_slice(),
                "duplicate-content-length",
            ),
            (
                b"Content-Length: 0\r\nl: 1".as_slice(),
                "duplicate-content-length",
            ),
            (
                b"Content-Length \t: 0\r\nl : 1".as_slice(),
                "duplicate-content-length",
            ),
            (
                b"L: 1\r\nCONTENT-LENGTH: 0".as_slice(),
                "duplicate-content-length",
            ),
            (b"Content-Length: nope".as_slice(), "invalid-content-length"),
            (b"l: \xff".as_slice(), "invalid-content-length"),
            (
                b"Content-Length: 184467440737095516160".as_slice(),
                "content-length-overflow",
            ),
        ] {
            let mut bytes = minimal_request(headers, b"");
            bytes.extend_from_slice(&valid_second);
            let original = bytes.clone();
            let mut buffer = BytesMut::from(bytes.as_slice());

            let error = try_parse_one(&mut buffer).expect_err("ambiguous frame must be rejected");
            assert!(
                matches!(&error, Error::ParseError(detail) if detail.contains(class)),
                "unexpected framing class: {error}"
            );
            assert_eq!(buffer.as_ref(), original);
        }
    }

    #[test]
    fn tls_rejects_slow_endless_headers_and_huge_bodies_at_shared_bounds() {
        let mut slow =
            BytesMut::from(&b"OPTIONS sip:service.example SIP/2.0\r\nX-Fold: value\r\n"[..]);
        let mut continuation = vec![b'a'; 1_022];
        continuation[0] = b' ';
        continuation.extend_from_slice(b"\r\n");
        while slow.len() <= MAX_SIP_HEADER_BYTES {
            slow.extend_from_slice(&continuation);
            if slow.len() <= MAX_SIP_HEADER_BYTES {
                assert!(try_parse_one(&mut slow).unwrap().is_none());
            }
        }
        let original = slow.clone();
        let error = try_parse_one(&mut slow).unwrap_err();
        assert!(matches!(
            &error,
            Error::ParseError(detail) if detail.contains("header-too-large")
        ));
        assert_eq!(slow, original);

        let huge = minimal_request(
            format!("Content-Length: {}", MAX_SIP_BODY_BYTES + 1).as_bytes(),
            b"",
        );
        let mut buffer = BytesMut::from(huge.as_slice());
        let error = try_parse_one(&mut buffer).unwrap_err();
        assert!(matches!(
            &error,
            Error::ParseError(detail) if detail.contains("body-too-large")
        ));
        assert_eq!(buffer.as_ref(), huge);
    }
}
