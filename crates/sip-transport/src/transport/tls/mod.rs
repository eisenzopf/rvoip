use std::fmt;
use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::rustls::{
    self, client::ServerCertVerified, Certificate, ClientConfig, OwnedTrustAnchor, PrivateKey,
    RootCertStore, ServerConfig, ServerName,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::{debug, error, info, trace, warn};

use crate::error::{Error, Result};
use crate::transport::{Transport, TransportEvent};

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
}

/// TLS transport implementation for SIP.
///
/// A single `TlsTransport` instance handles both inbound (server) and
/// outbound (client) TLS connections. The server side accepts
/// connections on `local_addr`; the client side dials remote peers via
/// [`TlsTransport::connect`] (or implicitly through `send_message` —
/// missing connections are auto-dialed).
pub struct TlsTransport {
    /// Local address the server side listens on.
    local_addr: SocketAddr,

    /// TLS acceptor for inbound connections (server side).
    acceptor: TlsAcceptor,

    /// TLS connector used for outbound dials. Holds the rustls
    /// `ClientConfig` (root store, cert verifier, etc.) so each
    /// `connect()` reuses the same trust state instead of rebuilding
    /// it.
    connector: TlsConnector,

    /// Active TLS connections, keyed by remote address. Used by
    /// `send_message` to find the right write-side mpsc channel.
    /// Connection-lifetime: removed by the per-connection reader task
    /// on EOF/error.
    connections: Arc<tokio::sync::Mutex<Vec<(SocketAddr, mpsc::Sender<Bytes>)>>>,

    /// Transport event sender.
    event_tx: Option<mpsc::Sender<TransportEvent>>,

    /// Closed flag. Once set, `connect()` and `send_message` short-circuit.
    closed: Arc<AtomicBool>,
}

impl fmt::Debug for TlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsTransport")
            .field("local_addr", &self.local_addr)
            .field("connections", &self.connections)
            .field("closed", &self.closed)
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
        Self::bind_with_client_config(
            local_addr,
            cert_path,
            key_path,
            event_tx,
            TlsClientConfig::default(),
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
        // Server-side config (for incoming TLS connections).
        let cert = load_certs(cert_path)?;
        let key = load_private_key(key_path)?;
        let server_config = ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert, key)
            .map_err(|e| Error::TlsHandshakeFailed(format!("TLS server config: {}", e)))?;
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
        let actual_addr = listener
            .local_addr()
            .map_err(Error::LocalAddrFailed)?;
        info!("TLS transport listening on {}", actual_addr);

        let transport = Self {
            local_addr: actual_addr,
            acceptor,
            connector,
            connections: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            event_tx: Some(tx),
            closed: Arc::new(AtomicBool::new(false)),
        };

        tokio::spawn(Self::accept_loop(
            listener,
            actual_addr,
            transport.acceptor.clone(),
            transport.connections.clone(),
            transport.event_tx.clone().unwrap(),
        ));

        Ok((transport, rx))
    }

    /// Accept loop driving an already-bound `TcpListener`.
    async fn accept_loop(
        listener: TcpListener,
        addr: SocketAddr,
        acceptor: TlsAcceptor,
        connections: Arc<tokio::sync::Mutex<Vec<(SocketAddr, mpsc::Sender<Bytes>)>>>,
        event_tx: mpsc::Sender<TransportEvent>,
    ) {
        loop {
            match listener.accept().await {
                Ok((stream, remote_addr)) => {
                    debug!("New TCP connection from {}", remote_addr);
                    let acceptor = acceptor.clone();
                    let connections = connections.clone();
                    let event_tx = event_tx.clone();
                    let local_addr = addr;

                    tokio::spawn(async move {
                        match acceptor.accept(stream).await {
                            Ok(tls_stream) => {
                                debug!("TLS handshake with {} successful", remote_addr);
                                Self::handle_connection(
                                    tls_stream,
                                    remote_addr,
                                    local_addr,
                                    connections,
                                    event_tx,
                                )
                                .await;
                            }
                            Err(e) => {
                                error!("TLS handshake with {} failed: {}", remote_addr, e);
                            }
                        }
                    });
                }
                Err(e) => {
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
        remote_addr: SocketAddr,
        local_addr: SocketAddr,
        connections: Arc<tokio::sync::Mutex<Vec<(SocketAddr, mpsc::Sender<Bytes>)>>>,
        event_tx: mpsc::Sender<TransportEvent>,
    ) where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (mut reader, mut writer) = tokio::io::split(tls_stream);
        let (tx, mut rx) = mpsc::channel::<Bytes>(100);

        {
            let mut connections_guard = connections.lock().await;
            connections_guard.push((remote_addr, tx.clone()));
        }

        let write_task = tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                if let Err(e) = writer.write_all(&data).await {
                    error!("Failed to write to TLS stream: {}", e);
                    break;
                }
                if let Err(e) = writer.flush().await {
                    error!("Failed to flush TLS stream: {}", e);
                    break;
                }
            }
        });

        // Buffered read loop with RFC 3261 §18.3 Content-Length framing.
        // TLS records can split a single SIP message across reads (or
        // bundle several into one), so we accumulate into a `BytesMut`
        // and pull off complete messages with `try_parse_one`.
        let mut buffer = BytesMut::with_capacity(8192);
        let mut tmp = vec![0u8; 8192];
        loop {
            match reader.read(&mut tmp).await {
                Ok(0) => break,
                Ok(n) => {
                    buffer.extend_from_slice(&tmp[..n]);
                    while let Some(message) = try_parse_one(&mut buffer) {
                        let _ = event_tx
                            .send(TransportEvent::MessageReceived {
                                message,
                                source: remote_addr,
                                destination: local_addr,
                            })
                            .await;
                    }
                }
                Err(e) => {
                    error!("Failed to read from TLS stream: {}", e);
                    break;
                }
            }
        }

        write_task.abort();

        {
            let mut connections_guard = connections.lock().await;
            connections_guard.retain(|(addr, _)| *addr != remote_addr);
        }

        debug!("TLS connection closed: {}", remote_addr);
    }

    /// Send data to a specific remote address. Auto-dials if no
    /// connection exists yet, using the destination's IP literal as the
    /// SNI server name. Hostname-aware dialing should go through
    /// [`TlsTransport::connect_with_server_name`] before the send so the
    /// caller can supply the URI's host instead of an IP.
    async fn send_to_addr(&self, data: Bytes, addr: SocketAddr) -> Result<()> {
        // Fast path: existing connection. Clone the bytes for the fast
        // path send so we still have the original on hand for the
        // auto-dial fallback when the channel is closed.
        {
            let connections_guard = self.connections.lock().await;
            if let Some((_, tx)) = connections_guard.iter().find(|(a, _)| *a == addr) {
                if tx.send(data.clone()).await.is_ok() {
                    return Ok(());
                }
                // Sender closed — fall through to reconnect.
            }
        }

        // Auto-dial.
        self.connect(addr).await?;

        let connections_guard = self.connections.lock().await;
        let (_, tx) = connections_guard
            .iter()
            .find(|(a, _)| *a == addr)
            .ok_or_else(|| {
                Error::Other(format!(
                    "TLS auto-dial succeeded but no connection registered for {}",
                    addr
                ))
            })?;
        tx.send(data).await.map_err(|_| {
            Error::Other(format!("Failed to push bytes to TLS write channel for {}", addr))
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
        self.connect_with_server_name(remote_addr, server_name).await
    }

    /// Connect to a remote address with an explicit SNI server name.
    /// Prefer this over [`connect`] when the caller knows the URI's
    /// host (e.g. `sips:alice@sip.example.com` → `"sip.example.com"`).
    pub async fn connect_with_server_name(
        &self,
        remote_addr: SocketAddr,
        server_name: ServerName,
    ) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        // Already connected? — short-circuit.
        {
            let connections_guard = self.connections.lock().await;
            if connections_guard.iter().any(|(addr, _)| *addr == remote_addr) {
                return Ok(());
            }
        }

        debug!("TLS dial → {} (SNI {:?})", remote_addr, server_name);

        let tcp_stream = TcpStream::connect(remote_addr)
            .await
            .map_err(|e| Error::ConnectFailed(remote_addr, e))?;

        let tls_stream = self
            .connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| Error::TlsHandshakeFailed(format!("TLS handshake to {}: {}", remote_addr, e)))?;

        info!("TLS handshake to {} succeeded", remote_addr);

        let connections = self.connections.clone();
        let event_tx = self
            .event_tx
            .clone()
            .ok_or_else(|| Error::TlsHandshakeFailed("TLS transport has no event sender".into()))?;
        let local_addr = self.local_addr;

        // Spawn the same generic read/write loop used for inbound
        // connections; it registers the connection in the registry as
        // its first action so a subsequent `send_to_addr` finds it.
        tokio::spawn(async move {
            Self::handle_connection(tls_stream, remote_addr, local_addr, connections, event_tx)
                .await;
        });

        // Wait briefly for the spawned task to register the connection
        // before we return; otherwise a back-to-back
        // `connect` + `send_message` race can lose the very first
        // outgoing bytes.
        for _ in 0..50 {
            {
                let connections_guard = self.connections.lock().await;
                if connections_guard.iter().any(|(addr, _)| *addr == remote_addr) {
                    return Ok(());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        // Even if the timing race lost, the registration will land
        // shortly; subsequent sends will succeed.
        trace!("TLS connect to {} returned before reader task registered", remote_addr);
        Ok(())
    }
}

#[async_trait]
impl Transport for TlsTransport {
    async fn send_message(
        &self,
        message: rvoip_sip_core::Message,
        destination: SocketAddr,
    ) -> Result<()> {
        // `Message::to_bytes` produces wire-format SIP (header CRLFs +
        // trailing CRLF separator + body) — required by RFC 3261 §7.2.
        // `to_string()` is for display/debug only and omits the final
        // separator, which then breaks Content-Length framing on the
        // peer's read side.
        let bytes = message.to_bytes();
        self.send_to_addr(bytes.into(), destination).await
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);
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
            Ok(guard) => guard.iter().any(|(addr, _)| *addr == remote_addr),
            Err(_) => false,
        }
    }
}

/// Build a rustls `ClientConfig` honouring the supplied
/// [`TlsClientConfig`]. Default behaviour: load system roots via
/// `rustls-native-certs`, fall back to bundled `webpki-roots`, refuse
/// any cert that fails standard validation. Optional extras: an extra
/// CA bundle (added to the same root store) and an insecure-skip mode
/// (dev only — accepts any cert without identity verification).
fn build_client_config(cfg: &TlsClientConfig) -> Result<ClientConfig> {
    if cfg.insecure_skip_verify {
        warn!(
            "TLS client built with insecure_skip_verify=true — \
             server certificates will NOT be validated. Dev only."
        );
        let cfg = ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier))
            .with_no_client_auth();
        return Ok(cfg);
    }

    let mut root_store = RootCertStore::empty();

    let mut loaded_any_system = false;
    match rustls_native_certs::load_native_certs() {
        Ok(certs) => {
            for cert in certs {
                if root_store.add(&Certificate(cert.0)).is_ok() {
                    loaded_any_system = true;
                }
            }
            debug!(
                "TLS client root store loaded {} system certs",
                root_store.len()
            );
        }
        Err(e) => {
            warn!(
                "TLS client: failed to read system trust store ({}); falling back to webpki-roots",
                e
            );
        }
    }

    if !loaded_any_system {
        root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
            OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));
        debug!(
            "TLS client root store fell back to webpki-roots ({} anchors)",
            root_store.len()
        );
    }

    if let Some(extra_path) = &cfg.extra_ca_path {
        let extras = load_certs(extra_path)?;
        for cert in extras {
            root_store.add(&cert).map_err(|e| {
                Error::TlsHandshakeFailed(format!(
                    "Failed to add extra CA from {}: {}",
                    extra_path.display(),
                    e
                ))
            })?;
        }
        info!(
            "TLS client added {} extra CA cert(s) from {}",
            root_store.len(),
            extra_path.display()
        );
    }

    Ok(ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth())
}

/// Cert verifier that accepts every server cert. Dev only — gated
/// behind `TlsClientConfig::insecure_skip_verify`.
struct InsecureCertVerifier;

impl rustls::client::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
}

/// Best-effort SNI server-name from a destination `SocketAddr`.
/// Loopback maps to `"localhost"` so test certs that include the
/// `localhost` SAN match.
fn ip_to_server_name(addr: SocketAddr) -> ServerName {
    if addr.ip().is_loopback() {
        if let Ok(name) = ServerName::try_from("localhost") {
            return name;
        }
    }
    ServerName::IpAddress(addr.ip())
}

/// Load PEM-encoded certificates from a file.
fn load_certs(path: &Path) -> Result<Vec<Certificate>> {
    let mut cert_file = File::open(path)
        .map_err(|e| Error::Other(format!("Failed to open cert {}: {}", path.display(), e)))?;
    let mut cert_data = Vec::new();
    cert_file
        .read_to_end(&mut cert_data)
        .map_err(|e| Error::Other(format!("Failed to read cert {}: {}", path.display(), e)))?;
    let certs = rustls_pemfile::certs(&mut cert_data.as_slice())
        .map_err(|e| {
            Error::TlsHandshakeFailed(format!("Failed to parse cert {}: {}", path.display(), e))
        })?
        .iter()
        .map(|v| Certificate(v.clone()))
        .collect();
    Ok(certs)
}

/// Try to parse a single complete SIP message off the front of
/// `buffer`. Returns `Some(message)` (and removes those bytes) when a
/// complete message is present per RFC 3261 §18.3 Content-Length
/// framing; returns `None` when more bytes are needed. Mirrors
/// `transport::tcp::connection::TcpConnection::try_parse_message` so
/// TLS framing matches TCP's behaviour exactly.
fn try_parse_one(buffer: &mut BytesMut) -> Option<rvoip_sip_core::Message> {
    if buffer.is_empty() {
        return None;
    }

    // End-of-headers = double CRLF.
    let header_end = (0..buffer.len().saturating_sub(3))
        .find(|&i| &buffer[i..i + 4] == b"\r\n\r\n")?;
    let body_start = header_end + 4;

    // Pull Content-Length from the header section. SIP allows the
    // compact form "l:" but the bulk of senders use the long form.
    let content_length = {
        let header_str = std::str::from_utf8(&buffer[..header_end + 4]).ok()?;
        let mut len = 0usize;
        for line in header_str.lines() {
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();
            if lower.starts_with("content-length:") || lower.starts_with("l:") {
                if let Some(value) = trimmed.split(':').nth(1) {
                    if let Ok(n) = value.trim().parse::<usize>() {
                        len = n;
                    }
                }
            }
        }
        len
    };

    let total = body_start + content_length;
    if buffer.len() < total {
        return None;
    }

    let slice = buffer[..total].to_vec();
    match rvoip_sip_core::parse_message(&slice) {
        Ok(message) => {
            buffer.advance(total);
            Some(message)
        }
        Err(e) => {
            warn!(
                "TLS: failed to parse SIP message ({} bytes, content_length={}): {}",
                total, content_length, e
            );
            // Skip past the malformed message so we don't loop on it.
            buffer.advance(total);
            None
        }
    }
}

/// Load a PEM-encoded PKCS#8 private key from a file.
fn load_private_key(path: &Path) -> Result<PrivateKey> {
    let mut key_file = File::open(path)
        .map_err(|e| Error::Other(format!("Failed to open key {}: {}", path.display(), e)))?;
    let mut key_data = Vec::new();
    key_file
        .read_to_end(&mut key_data)
        .map_err(|e| Error::Other(format!("Failed to read key {}: {}", path.display(), e)))?;
    let keys = rustls_pemfile::pkcs8_private_keys(&mut key_data.as_slice()).map_err(|e| {
        Error::TlsHandshakeFailed(format!("Failed to parse key {}: {}", path.display(), e))
    })?;
    if keys.is_empty() {
        return Err(Error::TlsHandshakeFailed(format!(
            "No PKCS#8 private keys found in {}",
            path.display()
        )));
    }
    Ok(PrivateKey(keys[0].clone()))
}
