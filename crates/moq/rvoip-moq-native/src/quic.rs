// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::HashSet,
    fmt,
    fs::File,
    io::BufWriter,
    net::{self, IpAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time,
};

use anyhow::Context;
use clap::Parser;
use socket2::{Domain, Protocol, Socket, Type};
use url::Url;

use moq_transport::session::{NegotiatedTransport, SessionTarget, Transport};

use crate::tls;

use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;

/// Represents the address family of the local QUIC socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
    /// IPv6 with dual-stack support (IPV6_V6ONLY=false)
    Ipv6DualStack,
}

pub enum Host {
    Ip(IpAddr),
    Name(String),
}

/// Substrate selection policy for a canonical `moqt://` session target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstratePolicy {
    /// Offer raw MOQT ALPNs and HTTP/3; the TLS peer selects the substrate.
    Auto,
    /// Require a raw QUIC MOQT ALPN.
    RawQuic,
    /// Require HTTP/3 followed by a WebTransport CONNECT request.
    WebTransport,
}

/// Connected transport plus the protocol actually negotiated with the peer.
pub struct SessionConnection {
    pub session: web_transport::Session,
    pub connection_id: String,
    pub negotiated: NegotiatedTransport,
    pub peer_identity: tls::PeerIdentity,
}

impl SessionConnection {
    /// Split a connection into the historical transport tuple.
    ///
    /// New callers that need authenticated peer metadata should retain the
    /// [`SessionConnection`] or call [`Self::into_parts_with_identity`].
    pub fn into_parts(self) -> (web_transport::Session, String, NegotiatedTransport) {
        (self.session, self.connection_id, self.negotiated)
    }

    /// Split a connection while retaining authenticated peer metadata.
    pub fn into_parts_with_identity(
        self,
    ) -> (
        web_transport::Session,
        String,
        NegotiatedTransport,
        tls::PeerIdentity,
    ) {
        (
            self.session,
            self.connection_id,
            self.negotiated,
            self.peer_identity,
        )
    }
}

fn peer_identity(
    connection: &quinn::Connection,
    verified_by_tls: bool,
) -> anyhow::Result<tls::PeerIdentity> {
    let Some(identity) = connection.peer_identity() else {
        return Ok(tls::PeerIdentity::Anonymous);
    };
    let chain = identity
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .map_err(|_| anyhow::anyhow!("unexpected TLS peer identity metadata type"))?;
    Ok(
        match tls::CertificateIdentity::from_verified_chain(&chain)? {
            Some(identity) if verified_by_tls => tls::PeerIdentity::Certificate(identity),
            Some(identity) => tls::PeerIdentity::UnverifiedCertificate(identity),
            None => tls::PeerIdentity::Anonymous,
        },
    )
}

/// Translate the historical scheme-selected input into a canonical target and
/// explicit policy. `https://` remains accepted only as a deprecated
/// compatibility alias for WebTransport and emits an operator-visible warning.
pub fn compatibility_target(url: &Url) -> anyhow::Result<(SessionTarget, SubstratePolicy)> {
    match url.scheme() {
        "moqt" => Ok((
            SessionTarget::try_from_url(url.clone())?,
            SubstratePolicy::RawQuic,
        )),
        "https" => {
            let target = SessionTarget::from_webtransport_url(url)?;
            tracing::warn!(
                target = %target.redacted_for_logging(),
                "https:// MOQT inputs are deprecated; use a canonical moqt:// target with an explicit WebTransport policy"
            );
            Ok((target, SubstratePolicy::WebTransport))
        }
        scheme => {
            anyhow::bail!("unsupported MOQT URL scheme {scheme:?}; canonical targets use 'moqt'")
        }
    }
}

fn format_url_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

fn supported_protocol(protocol: &[u8]) -> Option<&'static str> {
    moq_transport::setup::SUPPORTED_ALPNS
        .iter()
        .copied()
        .find(|supported| supported.as_bytes() == protocol)
}

fn policy_allows(policy: SubstratePolicy, substrate: Transport) -> bool {
    matches!(policy, SubstratePolicy::Auto)
        || matches!(
            (policy, substrate),
            (SubstratePolicy::RawQuic, Transport::RawQuic)
                | (SubstratePolicy::WebTransport, Transport::WebTransport)
        )
}

fn alpn_protocols(policy: SubstratePolicy) -> Vec<Vec<u8>> {
    let mut protocols = Vec::new();
    if policy_allows(policy, Transport::WebTransport) {
        protocols.push(web_transport_quinn::ALPN.as_bytes().to_vec());
    }
    if policy_allows(policy, Transport::RawQuic) {
        protocols.extend(
            moq_transport::setup::SUPPORTED_ALPNS
                .iter()
                .map(|protocol| protocol.as_bytes().to_vec()),
        );
    }
    protocols
}

fn selected_webtransport_protocol(protocol: Option<&str>) -> anyhow::Result<&'static str> {
    let protocol = protocol.context("WebTransport response did not select a MOQT protocol")?;
    moq_transport::setup::SUPPORTED_ALPNS
        .iter()
        .copied()
        .find(|supported| *supported == protocol)
        .with_context(|| format!("WebTransport selected unsupported MOQT protocol {protocol:?}"))
}

fn webtransport_authority_matches_tls(tls_host: &str, request_url: &Url) -> bool {
    request_url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(tls_host))
}

impl fmt::Display for AddressFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddressFamily::Ipv4 => write!(f, "IPv4"),
            AddressFamily::Ipv6 => write!(f, "IPv6"),
            AddressFamily::Ipv6DualStack => write!(f, "IPv6 (dual stack)"),
        }
    }
}

/// Bind a UDP socket, attempting dual-stack if the address is IPv6.
///
/// For IPv6 addresses, attempts to set `IPV6_V6ONLY = false` to enable
/// dual-stack operation (accepting both IPv4 and IPv6 traffic). This is
/// the default on Linux but must be explicitly requested on macOS/Windows.
///
/// Returns `(socket, is_dual_stack)` where `is_dual_stack` indicates
/// whether the socket can handle both IPv4 and IPv6 destinations.
fn bind_smart(addr: net::SocketAddr) -> anyhow::Result<(net::UdpSocket, bool)> {
    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create UDP socket")?;

    let mut is_dual_stack = false;

    if addr.is_ipv6() {
        match socket.set_only_v6(false) {
            Ok(()) => {
                is_dual_stack = true;
                tracing::debug!(addr = %addr, "IPv6 dual-stack enabled (IPV6_V6ONLY=false)");
            }
            Err(e) => {
                tracing::warn!(
                    addr = %addr,
                    error = %e,
                    "Could not enable dual-stack on IPv6 socket; \
                     IPv4-only destinations may be unreachable"
                );
            }
        }
    }

    socket
        .bind(&addr.into())
        .with_context(|| format!("failed to bind UDP socket to {}", addr))?;

    let local_addr = match socket.local_addr() {
        Ok(a) => a
            .as_socket()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<non-IP address>".to_string()),
        Err(e) => {
            tracing::warn!(error = %e, "failed to get local address after successful bind");
            "<unknown>".to_string()
        }
    };

    tracing::info!(
        bind = %addr,
        local = %local_addr,
        dual_stack = is_dual_stack,
        "UDP socket bound"
    );

    Ok((socket.into(), is_dual_stack))
}

/// Build a TransportConfig with our standard settings
///
/// This is used both for the base endpoint config and when creating
/// per-connection configs with qlog enabled.
fn build_transport_config() -> quinn::TransportConfig {
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(time::Duration::from_secs(10).try_into().unwrap()));
    transport.keep_alive_interval(Some(time::Duration::from_secs(4))); // TODO make this smarter
    transport.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    transport.mtu_discovery_config(None); // Disable MTU discovery
    transport
}

#[derive(Parser, Clone)]
pub struct Args {
    /// Listen for UDP packets on the given address.
    ///
    /// Defaults to `[::]:0` (IPv6 with dual-stack). If the default IPv6 bind
    /// fails, automatically falls back to 0.0.0.0 (IPv4-only) with a warning.
    /// Explicitly provided IPv6 addresses will not fall back.
    #[arg(long, default_value = Args::DEFAULT_BIND)]
    pub bind: net::SocketAddr,

    /// Directory to write qlog files (one per connection)
    #[arg(long)]
    pub qlog_dir: Option<PathBuf>,

    #[command(flatten)]
    pub tls: tls::Args,

    /// Explicit development override: accept an unvalidated source address
    /// without first sending a QUIC Retry packet.
    #[arg(long)]
    pub disable_stateless_retry: bool,

    /// Explicit development diagnostics: honor SSLKEYLOGFILE for TLS secrets.
    #[arg(long)]
    pub tls_key_log: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            bind: Self::DEFAULT_BIND.parse().unwrap(),
            qlog_dir: None,
            tls: Default::default(),
            disable_stateless_retry: false,
            tls_key_log: false,
        }
    }
}

impl Args {
    /// The default bind address used when `--bind` is not explicitly provided.
    const DEFAULT_BIND: &str = "[::]:0";

    pub fn load(&self) -> anyhow::Result<Config> {
        let tls = self.tls.load()?;

        match Config::new(self.bind, self.qlog_dir.clone(), tls.clone()).map(|config| {
            config
                .with_stateless_retry(!self.disable_stateless_retry)
                .with_tls_key_log(self.tls_key_log)
        }) {
            Ok(config) => Ok(config),
            Err(e) if self.bind.to_string() == Self::DEFAULT_BIND => {
                // IPv6 default bind failed -- try falling back to IPv4.
                // Only do this for the default; if the user explicitly
                // requested an IPv6 address, respect that and propagate
                // the error.
                let fallback = net::SocketAddr::new(
                    net::IpAddr::V4(net::Ipv4Addr::UNSPECIFIED),
                    self.bind.port(),
                );
                tracing::warn!(
                    requested = %self.bind,
                    fallback = %fallback,
                    error = %e,
                    "IPv6 bind failed, falling back to IPv4"
                );
                Config::new(fallback, self.qlog_dir.clone(), tls)
                    .map(|config| {
                        config
                            .with_stateless_retry(!self.disable_stateless_retry)
                            .with_tls_key_log(self.tls_key_log)
                    })
                    .with_context(|| {
                        format!("IPv4 fallback also failed (original IPv6 error: {})", e)
                    })
            }
            Err(e) => Err(e),
        }
    }
}

pub struct Config {
    pub bind: Option<net::SocketAddr>,
    pub socket: net::UdpSocket,
    pub is_dual_stack: bool,
    pub qlog_dir: Option<PathBuf>,
    pub tls: tls::Config,
    pub tags: HashSet<String>,
    max_pending_handshakes: usize,
    handshake_timeout: time::Duration,
    stateless_retry: bool,
    tls_key_log: bool,
}

impl Config {
    pub const DEFAULT_MAX_PENDING_HANDSHAKES: usize = 128;
    pub const DEFAULT_HANDSHAKE_TIMEOUT: time::Duration = time::Duration::from_secs(10);

    pub fn new(
        bind: net::SocketAddr,
        qlog_dir: Option<PathBuf>,
        tls: tls::Config,
    ) -> anyhow::Result<Self> {
        let (socket, is_dual_stack) = bind_smart(bind)?;
        Ok(Self {
            bind: Some(bind),
            socket,
            is_dual_stack,
            qlog_dir,
            tls,
            tags: HashSet::new(),
            max_pending_handshakes: Self::DEFAULT_MAX_PENDING_HANDSHAKES,
            handshake_timeout: Self::DEFAULT_HANDSHAKE_TIMEOUT,
            stateless_retry: true,
            tls_key_log: false,
        })
    }

    pub fn with_socket(
        socket: net::UdpSocket,
        qlog_dir: Option<PathBuf>,
        tls: tls::Config,
    ) -> Self {
        // Probe the socket to detect dual-stack capability rather than assuming.
        let is_dual_stack = socket.local_addr().is_ok_and(|addr| {
            addr.is_ipv6() && {
                let sock_ref = socket2::SockRef::from(&socket);
                sock_ref.only_v6().map(|v6only| !v6only).unwrap_or(false)
            }
        });

        Self {
            bind: None,
            socket,
            is_dual_stack,
            qlog_dir,
            tls,
            tags: HashSet::new(),
            max_pending_handshakes: Self::DEFAULT_MAX_PENDING_HANDSHAKES,
            handshake_timeout: Self::DEFAULT_HANDSHAKE_TIMEOUT,
            stateless_retry: true,
            tls_key_log: false,
        }
    }

    pub fn with_tag(mut self, tag: String) -> Self {
        self.tags.insert(tag);
        self
    }

    pub fn with_accept_limits(
        mut self,
        max_pending_handshakes: usize,
        handshake_timeout: time::Duration,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            max_pending_handshakes > 0,
            "pending handshake limit must be positive"
        );
        anyhow::ensure!(
            !handshake_timeout.is_zero(),
            "handshake timeout must be positive"
        );
        self.max_pending_handshakes = max_pending_handshakes;
        self.handshake_timeout = handshake_timeout;
        Ok(self)
    }

    /// Require source-address validation with a QUIC Retry packet before
    /// allocating handshake state. Enabled by default; disabling it is an
    /// explicit local-development compatibility override.
    pub fn with_stateless_retry(mut self, enabled: bool) -> Self {
        self.stateless_retry = enabled;
        self
    }

    /// Explicitly enable TLS secret logging through rustls KeyLogFile.
    /// Disabled by default because key logs can decrypt auth and media.
    pub fn with_tls_key_log(mut self, enabled: bool) -> Self {
        self.tls_key_log = enabled;
        self
    }
}

pub struct Endpoint {
    pub client: Client,
    pub server: Option<Server>,
    client_auth: tls::ClientAuthMode,
    writes_per_connection_diagnostics: bool,
    stateless_retry: bool,
    tls_key_log: bool,
    /// Tags associated with this endpoint
    /// These are used to filter endpoints for different purposes, for eg-
    /// "server" tag is used to filter endpoints for relay server
    /// "forward" tag is used to filter endpoints for forwarder
    /// This is upto the user to define and use
    pub tags: HashSet<String>,
}

impl Endpoint {
    pub fn client_auth_mode(&self) -> tls::ClientAuthMode {
        self.client_auth
    }

    pub fn verifies_server_certificates(&self) -> bool {
        self.client.verifies_server_certificates()
    }

    /// Whether accepting a connection can create a per-session qlog file.
    /// Production embeddings should reject this mode unless they supply a
    /// bounded retention implementation outside this crate.
    pub fn writes_per_connection_diagnostics(&self) -> bool {
        self.writes_per_connection_diagnostics
    }

    pub fn uses_stateless_retry(&self) -> bool {
        self.stateless_retry
    }

    pub fn tls_key_logging_enabled(&self) -> bool {
        self.tls_key_log
    }

    pub fn new(config: Config) -> anyhow::Result<Self> {
        let writes_per_connection_diagnostics = config.qlog_dir.is_some();
        let tls_key_log = config.tls_key_log;
        // Validate qlog directory if provided

        if let Some(qlog_dir) = &config.qlog_dir {
            if !qlog_dir.exists() {
                anyhow::bail!("qlog directory does not exist: {}", qlog_dir.display());
            }
            if !qlog_dir.is_dir() {
                anyhow::bail!("qlog path is not a directory: {}", qlog_dir.display());
            }
            tracing::info!("qlog output enabled: {}", qlog_dir.display());
        }

        // Build transport config with our standard settings
        let transport = Arc::new(build_transport_config());

        let mut server_config = None;
        let tls = config.tls.into_quic_parts();
        let client_auth = tls.client_auth;
        let verifies_server_certificates = tls.verifies_server_certificates;

        if let Some(mut config) = tls.server {
            // Offer WebTransport ALPN plus all supported MoQT versions for raw QUIC.
            config.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
            for alpn in moq_transport::setup::SUPPORTED_ALPNS {
                config.alpn_protocols.push(alpn.as_bytes().to_vec());
            }
            if tls_key_log {
                config.key_log = Arc::new(rustls::KeyLogFile::new());
            }

            let config: quinn::crypto::rustls::QuicServerConfig = config.try_into()?;
            let mut config = quinn::ServerConfig::with_crypto(Arc::new(config));
            config.transport_config(transport.clone());

            server_config = Some(config);
        }

        // There's a bit more boilerplate to make a generic endpoint.
        let runtime = quinn::default_runtime().context("no async runtime")?;
        let endpoint_config = quinn::EndpointConfig::default();
        let socket = config.socket;

        // Create the generic QUIC endpoint.
        let quic = quinn::Endpoint::new(endpoint_config, server_config.clone(), socket, runtime)
            .context("failed to create QUIC endpoint")?;

        let server = server_config.map(|base_server_config| Server {
            quic: quic.clone(),
            accept: Default::default(),
            qlog_dir: config.qlog_dir.map(Arc::new),
            base_server_config: Arc::new(base_server_config),
            verifies_client_certificates: client_auth != tls::ClientAuthMode::Disabled,
            max_pending_handshakes: config.max_pending_handshakes,
            handshake_timeout: config.handshake_timeout,
            stateless_retry: config.stateless_retry,
            stateless_retries_sent: 0,
        });

        let client = Client {
            quic,
            config: tls.client,
            transport,
            is_dual_stack: config.is_dual_stack,
            verifies_server_certificates,
            tls_key_log,
        };

        Ok(Self {
            client,
            server,
            client_auth,
            writes_per_connection_diagnostics,
            stateless_retry: config.stateless_retry,
            tls_key_log,
            tags: config.tags,
        })
    }
}

pub struct Server {
    quic: quinn::Endpoint,
    accept: FuturesUnordered<BoxFuture<'static, anyhow::Result<SessionConnection>>>,
    qlog_dir: Option<Arc<PathBuf>>,
    base_server_config: Arc<quinn::ServerConfig>,
    verifies_client_certificates: bool,
    max_pending_handshakes: usize,
    handshake_timeout: time::Duration,
    stateless_retry: bool,
    stateless_retries_sent: u64,
}

impl Server {
    /// Accept a connection and retain its actual negotiated protocol metadata.
    pub async fn accept_connection(&mut self) -> Option<SessionConnection> {
        loop {
            tokio::select! {
                res = self.quic.accept(), if self.accept.len() < self.max_pending_handshakes => {
                    let conn = res?;
                    if self.stateless_retry && !conn.remote_address_validated() {
                        match conn.retry() {
                            Ok(()) => {
                                self.stateless_retries_sent = self.stateless_retries_sent.saturating_add(1);
                            }
                            Err(error) => {
                                tracing::warn!(error = %error, "failed to send QUIC stateless retry");
                            }
                        }
                        continue;
                    }
                    let qlog_dir = self.qlog_dir.clone();
                    let base_server_config = self.base_server_config.clone();
                    let verifies_client_certificates = self.verifies_client_certificates;
                    let handshake_timeout = self.handshake_timeout;
                    self.accept.push(async move {
                        tokio::time::timeout(
                            handshake_timeout,
                            Self::accept_session(
                                conn,
                                qlog_dir,
                                base_server_config,
                                verifies_client_certificates,
                            ),
                        )
                        .await
                        .context("QUIC/WebTransport handshake timed out")?
                    }.boxed());
                },
                res = self.accept.next(), if !self.accept.is_empty() => {
                    match res? {
                        Ok(result) => return Some(result),
                        Err(err) => {
                            tracing::warn!("failed to accept QUIC connection: {}", err.root_cause());
                            continue;
                        }
                    }
                }
            }
        }
    }

    /// Compatibility tuple wrapper retaining the historical three-part API.
    /// Use [`Self::accept_connection`] when peer identity is required.
    pub async fn accept(
        &mut self,
    ) -> Option<(web_transport::Session, String, NegotiatedTransport)> {
        self.accept_connection()
            .await
            .map(SessionConnection::into_parts)
    }

    async fn accept_session(
        conn: quinn::Incoming,
        qlog_dir: Option<Arc<PathBuf>>,
        base_server_config: Arc<quinn::ServerConfig>,
        verifies_client_certificates: bool,
    ) -> anyhow::Result<SessionConnection> {
        // Capture the original destination connection ID BEFORE accepting
        // This is the actual QUIC CID that can be used for qlog/mlog correlation
        let orig_dst_cid = conn.orig_dst_cid();
        let connection_id_hex = orig_dst_cid.to_string();

        // Configure per-connection qlog if enabled
        let mut conn = if let Some(qlog_dir) = qlog_dir {
            // Create qlog file path using connection ID
            let qlog_path = qlog_dir.join(format!("{}_server.qlog", connection_id_hex));

            // Create transport config with our standard settings plus qlog
            let mut transport = build_transport_config();

            let file = File::create(&qlog_path).context("failed to create qlog file")?;
            let writer = BufWriter::new(file);

            let mut qlog = quinn::QlogConfig::default();
            qlog.writer(Box::new(writer))
                .title(Some("moq-relay".into()));
            transport.qlog_stream(qlog.into_stream());

            // Create custom server config with qlog-enabled transport
            let mut server_config = (*base_server_config).clone();
            server_config.transport_config(Arc::new(transport));

            tracing::debug!(
                "qlog enabled: cid={} path={}",
                connection_id_hex,
                qlog_path.display()
            );

            // Accept with custom config
            conn.accept_with(Arc::new(server_config))?
        } else {
            // No qlog - use default config
            conn.accept()?
        };

        let handshake = conn
            .handshake_data()
            .await?
            .downcast::<quinn::crypto::rustls::HandshakeData>()
            .unwrap();

        let alpn = handshake.protocol.context("missing ALPN")?;
        let alpn = String::from_utf8_lossy(&alpn);
        let server_name = handshake.server_name.unwrap_or_default();

        tracing::debug!(
            "received QUIC handshake: cid={} ip={} alpn={} server={}",
            connection_id_hex,
            conn.remote_address(),
            alpn,
            server_name,
        );

        // Wait for the QUIC connection to be established.
        let conn = conn.await.context("failed to establish QUIC connection")?;
        let peer_identity = peer_identity(&conn, verifies_client_certificates)?;
        let tls_host = if server_name.is_empty() {
            conn.local_ip()
                .map(|ip| ip.to_string())
                .context("TLS handshake has neither SNI nor a local IP")?
        } else {
            server_name.clone()
        };

        tracing::debug!(
            "established QUIC connection: cid={} stable_id={} ip={} alpn={} server={}",
            connection_id_hex,
            conn.stable_id(),
            conn.remote_address(),
            alpn,
            server_name,
        );

        let alpn_bytes = alpn.as_bytes();
        let (session, negotiated) = if alpn_bytes == web_transport_quinn::ALPN.as_bytes() {
            // Wait for the WebTransport CONNECT request (includes H3 SETTINGS exchange).
            let request = web_transport_quinn::Request::accept(conn)
                .await
                .context("failed to receive WebTransport request")?;

            // Bind the HTTP/3 CONNECT authority to the TLS virtual host. A
            // client must not authenticate one SNI name and then select a
            // different tenant/virtual host in :authority.
            if !webtransport_authority_matches_tls(&tls_host, &request.url) {
                let request_host = request.url.host_str().unwrap_or("<missing>").to_string();
                request
                    .reject(http::StatusCode::MISDIRECTED_REQUEST)
                    .await
                    .context("failed to reject mismatched WebTransport authority")?;
                anyhow::bail!(
                    "WebTransport CONNECT authority host {request_host:?} does not match the TLS virtual host"
                );
            }

            // Negotiate the MoQT version from the clients offered protocols.
            // Reject if no mutually-supported version exists.
            let selected = moq_transport::setup::negotiate_version(&request.protocols)
                .context("no mutually supported MoQT version in WT-Available-Protocols")?;
            let response =
                web_transport_quinn::proto::ConnectResponse::OK.with_protocol(selected.to_string());

            // Accept the CONNECT request.
            let session = request
                .respond(response)
                .await
                .context("failed to respond to WebTransport request")?;
            (
                session,
                NegotiatedTransport::new(Transport::WebTransport, selected),
            )
        } else if let Some(selected) = moq_transport::setup::SUPPORTED_ALPNS
            .iter()
            .find(|version| version.as_bytes() == alpn_bytes)
            .copied()
        {
            // Raw QUIC mode — create a "fake" WebTransport session with no H3 framing.
            let accepted_host = if server_name.is_empty() {
                conn.local_ip()
                    .map(format_url_host)
                    .context("raw QUIC connection has neither SNI nor a local IP")?
            } else {
                server_name.clone()
            };
            let request = url::Url::parse(&format!("moqt://{accepted_host}"))?;
            let session = web_transport_quinn::Session::raw(
                conn,
                request,
                web_transport_quinn::proto::ConnectResponse::default(),
            );
            (
                session,
                NegotiatedTransport::new(Transport::RawQuic, selected),
            )
        } else {
            anyhow::bail!("unsupported ALPN: {}", alpn)
        };

        Ok(SessionConnection {
            session: session.into(),
            connection_id: connection_id_hex,
            negotiated,
            peer_identity,
        })
    }

    pub fn local_addr(&self) -> anyhow::Result<net::SocketAddr> {
        self.quic
            .local_addr()
            .context("failed to get local address")
    }

    pub fn stateless_retries_sent(&self) -> u64 {
        self.stateless_retries_sent
    }
}

#[derive(Clone)]
pub struct Client {
    quic: quinn::Endpoint,
    config: rustls::ClientConfig,
    transport: Arc<quinn::TransportConfig>,
    is_dual_stack: bool,
    verifies_server_certificates: bool,
    tls_key_log: bool,
}

impl Client {
    pub fn verifies_server_certificates(&self) -> bool {
        self.verifies_server_certificates
    }
    /// Returns the local address of the QUIC socket.
    pub fn local_addr(&self) -> anyhow::Result<net::SocketAddr> {
        self.quic
            .local_addr()
            .context("failed to get local address")
    }

    /// Returns the address family of the local QUIC socket.
    ///
    /// Uses the dual-stack state determined at bind time rather than
    /// compile-time platform assumptions.
    pub fn address_family(&self) -> anyhow::Result<AddressFamily> {
        let local_addr = self
            .quic
            .local_addr()
            .context("failed to get local socket address")?;

        if local_addr.is_ipv4() {
            Ok(AddressFamily::Ipv4)
        } else if self.is_dual_stack {
            Ok(AddressFamily::Ipv6DualStack)
        } else {
            Ok(AddressFamily::Ipv6)
        }
    }

    /// Connect to a canonical `moqt://` target using the requested substrate
    /// policy and return the actual protocol selected by TLS/HTTP negotiation.
    pub async fn connect_target(
        &self,
        target: &SessionTarget,
        policy: SubstratePolicy,
        socket_addr: Option<net::SocketAddr>,
    ) -> anyhow::Result<SessionConnection> {
        let mut config = self.config.clone();

        config.alpn_protocols = alpn_protocols(policy);

        if self.tls_key_log {
            config.key_log = Arc::new(rustls::KeyLogFile::new());
        }

        let config: quinn::crypto::rustls::QuicClientConfig = config.try_into()?;
        let mut config = quinn::ClientConfig::new(Arc::new(config));
        config.transport_config(self.transport.clone());

        // Capture the initial destination CID that will be sent to the server
        // This is the CID used for qlog/mlog correlation on the server side
        let cid_capture: Arc<Mutex<Option<quinn::ConnectionId>>> = Arc::new(Mutex::new(None));
        let cid_capture_clone = cid_capture.clone();
        config.initial_dst_cid_provider(Arc::new(move || {
            // Generate a random CID (Quinn's default behavior)
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let random_bytes: [u8; 16] = rng.gen();
            let cid = quinn::ConnectionId::new(&random_bytes);
            *cid_capture_clone.lock().unwrap() = Some(cid);
            cid
        }));

        let host = match target.host().context("missing host")? {
            url::Host::Domain(d) => d.to_string(),
            url::Host::Ipv4(ip) => ip.to_string(),
            url::Host::Ipv6(ip) => ip.to_string(), // No brackets
        };
        let port = target.port().unwrap_or(443);

        // Look up the DNS entry and filter by socket address family.
        let addr = match socket_addr {
            Some(addr) => addr,
            None => {
                // Default DNS resolution logic
                self.resolve_dns(&host, port, self.address_family()?)
                    .await?
            }
        };

        let connection = self.quic.connect_with(config, addr, &host)?.await?;
        let peer_identity = peer_identity(&connection, self.verifies_server_certificates)?;

        let handshake = connection
            .handshake_data()
            .context("established QUIC connection is missing handshake metadata")?
            .downcast::<quinn::crypto::rustls::HandshakeData>()
            .map_err(|_| anyhow::anyhow!("unexpected QUIC handshake metadata type"))?;
        let selected_alpn = handshake
            .protocol
            .context("QUIC peer did not select an ALPN")?;

        // Extract the CID that was used
        let connection_id_hex = cid_capture
            .lock()
            .unwrap()
            .as_ref()
            .context("CID not captured")?
            .to_string();

        let (session, negotiated) = if selected_alpn == web_transport_quinn::ALPN.as_bytes() {
            if !policy_allows(policy, Transport::WebTransport) {
                anyhow::bail!(
                    "peer selected WebTransport contrary to the requested substrate policy"
                );
            }

            let request_url = target.webtransport_url();
            // Offer all supported MoQT versions via WT-Available-Protocols.
            let mut request = web_transport_quinn::proto::ConnectRequest::new(request_url);
            for protocol in moq_transport::setup::SUPPORTED_ALPNS {
                request = request.with_protocol(protocol.to_string());
            }
            let session = web_transport_quinn::Session::connect(connection, request)
                .await
                .context("failed to establish WebTransport session")?;
            let protocol = selected_webtransport_protocol(session.response().protocol.as_deref())?;
            (
                session,
                NegotiatedTransport::new(Transport::WebTransport, protocol),
            )
        } else if let Some(protocol) = supported_protocol(&selected_alpn) {
            if !policy_allows(policy, Transport::RawQuic) {
                anyhow::bail!("peer selected raw QUIC contrary to the requested substrate policy");
            }
            (
                web_transport_quinn::Session::raw(
                    connection,
                    target.network_url(),
                    web_transport_quinn::proto::ConnectResponse::default(),
                ),
                NegotiatedTransport::new(Transport::RawQuic, protocol),
            )
        } else {
            anyhow::bail!(
                "QUIC peer selected unsupported ALPN {:?}",
                String::from_utf8_lossy(&selected_alpn)
            )
        };

        Ok(SessionConnection {
            session: session.into(),
            connection_id: connection_id_hex,
            negotiated,
            peer_identity,
        })
    }

    /// Compatibility entry point for historical scheme-selected URLs.
    ///
    /// `https://` is a deprecated alias for a canonical `moqt://` target with
    /// [`SubstratePolicy::WebTransport`]. New callers should use
    /// [`Self::connect_target`] and select the substrate independently.
    pub async fn connect(
        &self,
        url: &Url,
        socket_addr: Option<net::SocketAddr>,
    ) -> anyhow::Result<(web_transport::Session, String, NegotiatedTransport)> {
        let (target, policy) = compatibility_target(url)?;
        Ok(self
            .connect_target(&target, policy, socket_addr)
            .await?
            .into_parts())
    }

    /// Default DNS resolution logic that filters results by address family.
    async fn resolve_dns(
        &self,
        host: &str,
        port: u16,
        address_family: AddressFamily,
    ) -> anyhow::Result<net::SocketAddr> {
        let local_addr = self.local_addr()?;

        // Collect all DNS results
        let addrs: Vec<net::SocketAddr> = match Self::parse_socket_addr(host, port) {
            Ok(addr) => {
                vec![addr]
            }
            Err(_) => tokio::net::lookup_host((host, port))
                .await
                .context("failed DNS lookup")?
                .collect(),
        };

        if addrs.is_empty() {
            anyhow::bail!("DNS lookup for host '{}' returned no addresses", host);
        }

        // Log all DNS results for debugging
        tracing::debug!(
            "DNS lookup for {}, family {:?}: found {} results",
            host,
            address_family,
            addrs.len()
        );
        for (i, addr) in addrs.iter().enumerate() {
            tracing::debug!(
                "  DNS[{}]: {} ({})",
                i,
                addr,
                if addr.is_ipv4() { "IPv4" } else { "IPv6" }
            );
        }

        // Filter DNS results to match our local socket's address family
        let compatible_addr = match address_family {
            AddressFamily::Ipv4 => {
                // IPv4 socket: filter to IPv4 addresses
                addrs
                    .iter()
                    .find(|a| a.is_ipv4())
                    .cloned()
                    .context(format!(
                        "No IPv4 address found for host '{}' (local socket is IPv4: {})",
                        host, local_addr
                    ))?
            }
            AddressFamily::Ipv6DualStack => {
                // Dual-stack socket: any address family works, use first result
                tracing::debug!("Using first DNS result (IPv6 dual-stack): {}", addrs[0]);
                addrs[0]
            }
            AddressFamily::Ipv6 => {
                // IPv6-only socket: filter to IPv6 addresses
                addrs
                    .iter()
                    .find(|a| a.is_ipv6())
                    .cloned()
                    .context(format!(
                        "No IPv6 address found for host '{}' (local socket is IPv6: {})",
                        host, local_addr
                    ))?
            }
        };

        tracing::debug!(
            "Connecting from {} to {} (selected from {} DNS results)",
            local_addr,
            compatible_addr,
            addrs.len()
        );

        Ok(compatible_addr)
    }

    fn parse_socket_addr(host: &str, port: u16) -> Result<net::SocketAddr, net::AddrParseError> {
        let host = format!("{}:{}", host, port);
        host.parse::<net::SocketAddr>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substrate_policy_controls_offered_alpns() {
        let h3 = web_transport_quinn::ALPN.as_bytes().to_vec();
        let moqt = moq_transport::setup::ALPN.to_vec();

        assert_eq!(alpn_protocols(SubstratePolicy::RawQuic), vec![moqt.clone()]);
        assert_eq!(
            alpn_protocols(SubstratePolicy::WebTransport),
            vec![h3.clone()]
        );
        let auto = alpn_protocols(SubstratePolicy::Auto);
        assert!(auto.contains(&h3));
        assert!(auto.contains(&moqt));
    }

    #[test]
    fn webtransport_protocol_must_be_explicit_and_supported() {
        assert_eq!(
            selected_webtransport_protocol(Some("moqt-19")).unwrap(),
            "moqt-19"
        );
        assert!(selected_webtransport_protocol(None).is_err());
        assert!(selected_webtransport_protocol(Some("moqt-16")).is_err());
    }

    #[test]
    fn legacy_https_alias_derives_a_canonical_target() {
        let url = Url::parse("https://Relay.Example/live?q=1").unwrap();
        let (target, policy) = compatibility_target(&url).unwrap();
        assert_eq!(
            target.canonical_url().as_str(),
            "moqt://relay.example/live?q=1"
        );
        assert_eq!(policy, SubstratePolicy::WebTransport);
    }

    #[test]
    fn webtransport_authority_is_bound_to_tls_virtual_host() {
        let matching = Url::parse("https://relay.example/live?token=secret").unwrap();
        let confused = Url::parse("https://other-tenant.example/live").unwrap();
        assert!(webtransport_authority_matches_tls(
            "Relay.Example",
            &matching
        ));
        assert!(!webtransport_authority_matches_tls(
            "relay.example",
            &confused
        ));
    }
}
