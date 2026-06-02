use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, mpsc::error::TrySendError, Mutex};
use tracing::{debug, error, info, trace, warn};

use rvoip_sip_core::Message;
use rvoip_sip_transport::diagnostics as udp_diagnostics;
use rvoip_sip_transport::factory::{TransportFactory, TransportFactoryConfig};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    TcpTransport, Transport, TransportEvent, UdpParseConfig, UdpParseDispatch, UdpSocketOptions,
    UdpTransport, WebSocketTransport,
};

use crate::transaction::error::{Error, Result};

pub mod multiplexed;
pub mod trace;
pub use multiplexed::MultiplexedTransport;
pub use rvoip_sip_transport::transport::tls::TlsRole;
pub(crate) use trace::SipTraceRuntime;
pub use trace::TraceRedactorFn;

const DEFAULT_TRANSPORT_EVENT_DISPATCH_WORKERS: usize = 1;
pub const MAX_TRANSPORT_EVENT_DISPATCH_WORKERS: usize = 64;

/// Configuration options for the TransportManager
#[derive(Debug, Clone)]
pub struct TransportManagerConfig {
    /// Whether to enable UDP transport
    pub enable_udp: bool,
    /// Whether to enable TCP transport
    pub enable_tcp: bool,
    /// Whether to enable WebSocket transport
    pub enable_ws: bool,
    /// Whether to enable TLS (for TCP and WebSocket)
    pub enable_tls: bool,
    /// TLS socket role for SIP TLS. Client-only mode dials TLS peers
    /// without binding a TLS listener or requiring a local cert/key.
    pub tls_role: TlsRole,
    /// Local addresses to bind to (if empty, will bind to all interfaces)
    pub bind_addresses: Vec<SocketAddr>,
    /// Explicit local addresses for SIP TLS listeners. When empty,
    /// listener-capable TLS roles retain the legacy behavior of deriving
    /// TLS ports from `bind_addresses` by adding 1.
    pub tls_bind_addresses: Vec<SocketAddr>,
    /// Default event channel capacity
    pub default_channel_capacity: usize,
    /// Optional UDP socket receive buffer size (`SO_RCVBUF`) in bytes.
    pub udp_recv_buffer_size: Option<usize>,
    /// Optional UDP socket send buffer size (`SO_SNDBUF`) in bytes.
    pub udp_send_buffer_size: Option<usize>,
    /// Optional UDP parse worker count.
    pub udp_parse_workers: Option<usize>,
    /// Optional per-worker UDP parse queue capacity.
    pub udp_parse_queue_capacity: Option<usize>,
    /// Optional UDP parse worker dispatch strategy.
    pub udp_parse_dispatch: Option<UdpParseDispatch>,
    /// Optional transport-manager event forwarding worker count.
    ///
    /// Values above `1` enable keyed sharding between each concrete transport
    /// event stream and the transaction manager ingress channel.
    pub transport_event_dispatch_workers: Option<usize>,
    /// Optional transport-manager event forwarding queue capacity.
    ///
    /// `None` uses [`TransportManagerConfig::default_channel_capacity`].
    /// When dispatch workers are enabled, this capacity is divided across
    /// workers.
    pub transport_event_dispatch_queue_capacity: Option<usize>,
    /// TLS certificate path
    pub tls_cert_path: Option<String>,
    /// TLS key path
    pub tls_key_path: Option<String>,
    /// Optional path to a PEM-encoded CA bundle to *add to* the system
    /// trust store on the client side. Useful for enterprise PKI /
    /// private carriers.
    pub tls_extra_ca_path: Option<String>,
    /// Optional client certificate for mutual TLS.
    pub tls_client_cert_path: Option<String>,
    /// Optional client private key for mutual TLS.
    pub tls_client_key_path: Option<String>,
    /// **Dev only.** When `true`, server certificates are accepted
    /// without identity verification. The TLS handshake still runs
    /// end-to-end (encrypted), but a malicious peer can MITM. Required
    /// for self-signed test certs; **must not** be enabled in
    /// production builds.
    pub tls_insecure_skip_verify: bool,
}

impl Default for TransportManagerConfig {
    fn default() -> Self {
        Self {
            enable_udp: true,
            enable_tcp: true,
            enable_ws: true,
            enable_tls: false,
            tls_role: TlsRole::ClientAndServer,
            bind_addresses: vec![],
            tls_bind_addresses: vec![],
            // NEXT_STEPS B.2 — single combined inbound-event channel
            // for ALL transports. At ≥100 CPS of INVITE+ACK+BYE we
            // see >300 msg/s aggregate; a 100-slot buffer back-pressures
            // the UDP recv task within seconds and stalls the whole
            // stack. Widen so the consumer (transaction manager) has
            // breathing room across momentary cleanup-path latency
            // bursts. The actual flow-control is at the per-transaction
            // level downstream; this just keeps the funnel from being
            // the bottleneck.
            default_channel_capacity: 10_000,
            udp_recv_buffer_size: None,
            udp_send_buffer_size: None,
            udp_parse_workers: None,
            udp_parse_queue_capacity: None,
            udp_parse_dispatch: None,
            transport_event_dispatch_workers: None,
            transport_event_dispatch_queue_capacity: None,
            tls_cert_path: None,
            tls_key_path: None,
            tls_extra_ca_path: None,
            tls_client_cert_path: None,
            tls_client_key_path: None,
            tls_insecure_skip_verify: false,
        }
    }
}

fn transport_event_dispatch_worker_count(workers: Option<usize>) -> usize {
    workers
        .unwrap_or(DEFAULT_TRANSPORT_EVENT_DISPATCH_WORKERS)
        .clamp(1, MAX_TRANSPORT_EVENT_DISPATCH_WORKERS)
}

fn transport_event_dispatch_queue_capacity(
    capacity: Option<usize>,
    default_capacity: usize,
) -> usize {
    capacity.unwrap_or(default_capacity).max(1)
}

fn transport_event_route_hash(event: &TransportEvent) -> Option<u64> {
    let TransportEvent::MessageReceived { message, .. } = event else {
        return None;
    };

    let mut hasher = DefaultHasher::new();
    match message {
        Message::Request(request) => {
            let call_id = request.call_id()?;
            call_id.value().hash(&mut hasher);
            if let Some(from_tag) = request.from_tag() {
                from_tag.hash(&mut hasher);
            }
        }
        Message::Response(response) => {
            let call_id = response.call_id()?;
            call_id.value().hash(&mut hasher);
            if let Some(from_tag) = response.from().and_then(|from| from.tag()) {
                from_tag.hash(&mut hasher);
            }
            if let Some(cseq) = response.cseq() {
                cseq.method().hash(&mut hasher);
            }
        }
    }
    Some(hasher.finish())
}

fn transport_event_dispatch_worker_index(
    event: &TransportEvent,
    worker_count: usize,
    fallback_worker: &AtomicUsize,
) -> usize {
    if worker_count <= 1 {
        return 0;
    }

    if let Some(hash) = transport_event_route_hash(event) {
        return (hash as usize) % worker_count;
    }

    fallback_worker.fetch_add(1, Ordering::Relaxed) % worker_count
}

fn start_transport_event_dispatch_workers(
    event_tx: mpsc::Sender<TransportEvent>,
    worker_count: usize,
    queue_capacity: usize,
) -> Arc<Vec<mpsc::Sender<TransportEvent>>> {
    let worker_count = worker_count.clamp(1, MAX_TRANSPORT_EVENT_DISPATCH_WORKERS);
    let per_worker_capacity = (queue_capacity / worker_count).max(1);
    let mut senders = Vec::with_capacity(worker_count);

    for worker_id in 0..worker_count {
        let (tx, mut rx) = mpsc::channel::<TransportEvent>(per_worker_capacity);
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if !forward_transport_event(&event_tx, event).await {
                    break;
                }
            }
            debug!(worker_id, "Transport event dispatch worker terminated");
        });
        senders.push(tx);
    }

    info!(
        workers = worker_count,
        per_worker_capacity, "Transport event dispatch workers enabled"
    );

    Arc::new(senders)
}

async fn forward_transport_event(
    event_tx: &mpsc::Sender<TransportEvent>,
    event: TransportEvent,
) -> bool {
    match event_tx.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(event)) => {
            let started = Instant::now();
            if let Err(e) = event_tx.send(event).await {
                error!("Failed to forward transport event: {}", e);
                false
            } else {
                udp_diagnostics::record_manager_channel_backpressure(started.elapsed());
                true
            }
        }
        Err(TrySendError::Closed(_)) => {
            error!("Failed to forward transport event: receiver closed");
            false
        }
    }
}

async fn dispatch_transport_event(
    event: TransportEvent,
    dispatch_senders: &Arc<Vec<mpsc::Sender<TransportEvent>>>,
    fallback_worker: &AtomicUsize,
) {
    let worker_index =
        transport_event_dispatch_worker_index(&event, dispatch_senders.len(), fallback_worker);

    match dispatch_senders[worker_index].try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(event)) => {
            warn!(
                worker_index,
                "Transport event dispatch worker queue full; applying backpressure"
            );
            if dispatch_senders[worker_index].send(event).await.is_err() {
                warn!(
                    worker_index,
                    "Transport event dispatch worker channel closed"
                );
            }
        }
        Err(TrySendError::Closed(_)) => {
            warn!(
                worker_index,
                "Transport event dispatch worker channel closed"
            );
        }
    }
}

/// Manages multiple transport types for SIP messages
#[derive(Clone)]
pub struct TransportManager {
    /// Configuration
    config: TransportManagerConfig,
    /// Collection of active transports by type and address
    transports: Arc<Mutex<HashMap<String, Arc<dyn Transport>>>>,
    /// Default transport
    default_transport: Option<Arc<dyn Transport>>,
    /// Default UDP transport (required for SIP)
    udp_transport: Option<Arc<dyn Transport>>,
    /// Transport factory. Captured at construction so a future
    /// dynamic transport-add path can spin new transports without
    /// re-creating the factory.
    #[allow(dead_code)]
    transport_factory: Arc<TransportFactory>,
    /// Combined event channel
    event_tx: mpsc::Sender<TransportEvent>,
    /// Flag indicating whether the manager is running
    running: Arc<Mutex<bool>>,
    /// Optional SIP trace publisher shared with the transaction manager.
    sip_trace: Option<Arc<SipTraceRuntime>>,
}

impl TransportManager {
    /// Creates a new TransportManager with the given configuration
    pub async fn new(
        config: TransportManagerConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let (event_tx, event_rx) = mpsc::channel(config.default_channel_capacity);

        let transports = Arc::new(Mutex::new(HashMap::new()));
        let transport_factory = Arc::new(TransportFactory::new(TransportFactoryConfig {
            channel_capacity: config.default_channel_capacity,
            udp_recv_buffer_size: config.udp_recv_buffer_size,
            udp_send_buffer_size: config.udp_send_buffer_size,
            udp_parse_workers: config.udp_parse_workers,
            udp_parse_queue_capacity: config.udp_parse_queue_capacity,
            udp_parse_dispatch: config.udp_parse_dispatch,
            ..Default::default()
        }));

        let manager = Self {
            config,
            transports,
            default_transport: None,
            udp_transport: None,
            transport_factory,
            event_tx,
            running: Arc::new(Mutex::new(false)),
            sip_trace: None,
        };

        Ok((manager, event_rx))
    }

    /// Creates a new TransportManager with default configuration
    pub async fn with_defaults() -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::new(TransportManagerConfig::default()).await
    }

    /// Initializes the transport manager with configured transport types
    pub async fn initialize(&mut self) -> Result<()> {
        let mut initialized = false;
        let mut initialization_errors = Vec::new();

        // Initialize UDP transport if enabled
        if self.config.enable_udp {
            let addresses = if self.config.bind_addresses.is_empty() {
                vec!["0.0.0.0:5060".parse().unwrap()]
            } else {
                self.config.bind_addresses.clone()
            };

            for addr in addresses {
                match self.add_udp_transport(addr).await {
                    Ok(transport) => {
                        // If this is the first transport, set it as default
                        if self.default_transport.is_none() {
                            self.default_transport = Some(transport.clone());
                        }
                        // Set as UDP transport
                        self.udp_transport = Some(transport);
                        initialized = true;
                    }
                    Err(e) => {
                        error!("Failed to initialize UDP transport on {}: {}", addr, e);
                        initialization_errors
                            .push(format!("UDP transport on {} failed: {}", addr, e));
                    }
                }
            }
        }

        // Initialize TCP transport if enabled
        if self.config.enable_tcp {
            let addresses = if self.config.bind_addresses.is_empty() {
                vec!["0.0.0.0:5060".parse().unwrap()]
            } else {
                self.config.bind_addresses.clone()
            };

            for addr in addresses {
                match self.add_tcp_transport(addr, false).await {
                    Ok(_) => {
                        initialized = true;
                    }
                    Err(e) => {
                        error!("Failed to initialize TCP transport on {}: {}", addr, e);
                        initialization_errors
                            .push(format!("TCP transport on {} failed: {}", addr, e));
                    }
                }
            }
        }

        // Add TLS transport if enabled. TLS is TCP-over-TLS, but it is
        // configured independently from the plain TCP listener. Listener
        // roles may supply explicit TLS bind addresses; otherwise retain
        // the legacy `bind_addr.port() + 1` behavior for compatibility.
        if self.config.enable_tls {
            if self.config.tls_role != TlsRole::ClientOnly
                && (self.config.tls_cert_path.is_none() || self.config.tls_key_path.is_none())
            {
                warn!("TLS is enabled but certificate or key path is missing");
            } else {
                let base_addresses = if self.config.bind_addresses.is_empty() {
                    vec!["0.0.0.0:5061".parse().unwrap()]
                } else {
                    self.config.bind_addresses.clone()
                };
                let addresses = if self.config.tls_role == TlsRole::ClientOnly {
                    if self.config.tls_bind_addresses.is_empty() {
                        base_addresses
                    } else {
                        self.config.tls_bind_addresses.clone()
                    }
                } else if !self.config.tls_bind_addresses.is_empty() {
                    self.config.tls_bind_addresses.clone()
                } else if self.config.bind_addresses.is_empty() {
                    base_addresses
                } else {
                    self.config
                        .bind_addresses
                        .iter()
                        .map(|addr| {
                            let mut tls_addr = *addr;
                            if tls_addr.port() != 0 {
                                tls_addr.set_port(tls_addr.port().saturating_add(1));
                            }
                            tls_addr
                        })
                        .collect::<Vec<_>>()
                };

                for addr in addresses {
                    match self.add_tcp_transport(addr, true).await {
                        Ok(_) => {
                            initialized = true;
                        }
                        Err(e) => {
                            error!("Failed to initialize TLS transport on {}: {}", addr, e);
                            initialization_errors
                                .push(format!("TLS transport on {} failed: {}", addr, e));
                        }
                    }
                }
            }
        }

        // Initialize WebSocket transport if enabled
        if self.config.enable_ws {
            let addresses = if self.config.bind_addresses.is_empty() {
                vec!["0.0.0.0:8080".parse().unwrap()]
            } else {
                self.config.bind_addresses.clone()
            };

            for addr in addresses {
                match self.add_websocket_transport(addr, false).await {
                    Ok(_) => {
                        initialized = true;
                    }
                    Err(e) => {
                        error!(
                            "Failed to initialize WebSocket transport on {}: {}",
                            addr, e
                        );
                        initialization_errors
                            .push(format!("WebSocket transport on {} failed: {}", addr, e));
                    }
                }
            }

            // Add WSS transport if enabled
            if self.config.enable_tls {
                if self.config.tls_cert_path.is_none() || self.config.tls_key_path.is_none() {
                    warn!("TLS is enabled but certificate or key path is missing");
                } else {
                    let addresses = if self.config.bind_addresses.is_empty() {
                        vec!["0.0.0.0:8443".parse().unwrap()]
                    } else {
                        self.config.bind_addresses.clone()
                    };

                    for addr in addresses {
                        match self.add_websocket_transport(addr, true).await {
                            Ok(_) => {
                                initialized = true;
                            }
                            Err(e) => {
                                error!("Failed to initialize WSS transport on {}: {}", addr, e);
                                initialization_errors
                                    .push(format!("WSS transport on {} failed: {}", addr, e));
                            }
                        }
                    }
                }
            }
        }

        // Return error if no transports were initialized
        if !initialized {
            let detail = if initialization_errors.is_empty() {
                "no transport initialization attempts were made".to_string()
            } else {
                initialization_errors.join("; ")
            };
            return Err(Error::Transport(format!(
                "Failed to initialize any transport: {}",
                detail
            )));
        }

        // Start event processing
        self.start_event_processing();

        Ok(())
    }

    /// Enable transport-boundary SIP tracing for this manager.
    pub fn enable_sip_trace(
        &mut self,
        owner_id: String,
        config: rvoip_infra_common::events::cross_crate::SipTraceConfig,
        coordinator: Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>,
    ) {
        self.sip_trace = SipTraceRuntime::new(owner_id, config, coordinator);
    }

    /// Enable transport-boundary SIP tracing with an application-supplied
    /// trace redactor (see SIP_API_DESIGN_2.md §12.4). The redactor
    /// transforms the rendered SIP message text before the static
    /// `format_sip_trace_message` pipeline runs; the wire form is
    /// untouched.
    pub fn enable_sip_trace_with_redactor(
        &mut self,
        owner_id: String,
        config: rvoip_infra_common::events::cross_crate::SipTraceConfig,
        coordinator: Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>,
        redactor: Option<TraceRedactorFn>,
    ) {
        self.sip_trace =
            SipTraceRuntime::new_with_redactor(owner_id, config, coordinator, redactor);
    }

    /// Return the configured SIP trace runtime, if any.
    pub(crate) fn sip_trace_runtime(&self) -> Option<Arc<SipTraceRuntime>> {
        self.sip_trace.clone()
    }

    /// Adds a UDP transport to the manager
    pub async fn add_udp_transport(&self, bind_addr: SocketAddr) -> Result<Arc<dyn Transport>> {
        let socket_options = UdpSocketOptions::new(
            self.config.udp_recv_buffer_size,
            self.config.udp_send_buffer_size,
        );
        let parse_config = UdpParseConfig::from_optional_with_dispatch(
            self.config.udp_parse_workers,
            self.config.udp_parse_queue_capacity,
            self.config.udp_parse_dispatch,
            self.config.default_channel_capacity,
        );
        let (transport, rx) = UdpTransport::bind_with_mtu_socket_options_and_parse_config(
            bind_addr,
            Some(self.config.default_channel_capacity),
            rvoip_sip_transport::transport::udp::UDP_SAFE_MAX_BYTES,
            socket_options,
            parse_config,
        )
        .await
        .map_err(|e| {
            Error::Transport(format!(
                "Failed to bind UDP transport to {}: {}",
                bind_addr, e
            ))
        })?;

        let transport_arc = Arc::new(transport);

        // Store the transport
        let key = format!("udp:{}", bind_addr);
        {
            let mut transports = self.transports.lock().await;
            transports.insert(key, transport_arc.clone());
        }

        // Process events from this transport
        self.clone()
            .process_transport_events(transport_arc.clone(), rx);

        info!("Added UDP transport bound to {}", bind_addr);

        Ok(transport_arc)
    }

    /// Adds a TCP transport — or, when `tls = true`, a TLS-over-TCP
    /// transport — to the manager. TLS listener roles require
    /// `tls_cert_path` + `tls_key_path`; client-only TLS does not bind
    /// a listener and only needs client trust settings.
    pub async fn add_tcp_transport(
        &self,
        bind_addr: SocketAddr,
        tls: bool,
    ) -> Result<Arc<dyn Transport>> {
        let (transport_arc, key): (Arc<dyn Transport>, String) = if tls {
            use rvoip_sip_transport::transport::tls::{TlsClientConfig, TlsTransport};
            use std::path::{Path, PathBuf};

            let client_cfg = TlsClientConfig {
                extra_ca_path: self.config.tls_extra_ca_path.as_ref().map(PathBuf::from),
                insecure_skip_verify: self.config.tls_insecure_skip_verify,
                client_cert_path: self.config.tls_client_cert_path.as_ref().map(PathBuf::from),
                client_key_path: self.config.tls_client_key_path.as_ref().map(PathBuf::from),
            };

            // Pass the manager's combined event sender so TLS events
            // (incoming SIP messages, errors) flow through the same
            // pipeline as UDP/TCP — no separate forwarder task needed.
            let (transport, _rx_unused) = match self.config.tls_role {
                TlsRole::ClientOnly => {
                    TlsTransport::client_only(bind_addr, Some(self.event_tx.clone()), client_cfg)
                        .await
                        .map_err(|e| {
                            Error::Transport(format!(
                                "Failed to configure client-only TLS transport at {}: {}",
                                bind_addr, e
                            ))
                        })?
                }
                TlsRole::ServerOnly | TlsRole::ClientAndServer => {
                    let cert_path = self.config.tls_cert_path.as_ref().ok_or_else(|| {
                        Error::Transport("TLS enabled but tls_cert_path is missing".into())
                    })?;
                    let key_path = self.config.tls_key_path.as_ref().ok_or_else(|| {
                        Error::Transport("TLS enabled but tls_key_path is missing".into())
                    })?;
                    let result = if self.config.tls_role == TlsRole::ServerOnly {
                        TlsTransport::bind_server_only_with_client_config(
                            bind_addr,
                            Path::new(cert_path),
                            Path::new(key_path),
                            Some(self.event_tx.clone()),
                            client_cfg,
                        )
                        .await
                    } else {
                        TlsTransport::bind_with_client_config(
                            bind_addr,
                            Path::new(cert_path),
                            Path::new(key_path),
                            Some(self.event_tx.clone()),
                            client_cfg,
                        )
                        .await
                    };
                    result.map_err(|e| {
                        Error::Transport(format!(
                            "Failed to bind TLS transport to {}: {}",
                            bind_addr, e
                        ))
                    })?
                }
            };
            // `local_addr()` reports the OS-assigned port (important
            // when `bind_addr` used port 0). Use the actual port in the
            // registry key so MultiplexedTransport can find it.
            let actual = transport.local_addr().map_err(|e| {
                Error::Transport(format!("TLS bind: failed to read local_addr: {}", e))
            })?;
            let arc: Arc<dyn Transport> = Arc::new(transport);
            (arc, format!("tls:{}", actual))
        } else {
            let (transport, rx) =
                TcpTransport::bind(bind_addr, Some(self.config.default_channel_capacity), None)
                    .await
                    .map_err(|e| {
                        Error::Transport(format!(
                            "Failed to bind TCP transport to {}: {}",
                            bind_addr, e
                        ))
                    })?;
            let arc: Arc<dyn Transport> = Arc::new(transport);
            // TCP path retains its own event channel; bridge it into
            // the manager's combined channel.
            self.clone().process_transport_events(arc.clone(), rx);
            (arc, format!("tcp:{}", bind_addr))
        };

        {
            let mut transports = self.transports.lock().await;
            transports.insert(key, transport_arc.clone());
        }

        info!(
            "Added {} transport bound to {}",
            if tls { "TLS" } else { "TCP" },
            bind_addr
        );

        Ok(transport_arc)
    }

    /// Adds a WebSocket transport to the manager
    pub async fn add_websocket_transport(
        &self,
        bind_addr: SocketAddr,
        secure: bool,
    ) -> Result<Arc<dyn Transport>> {
        // `cert_path` / `key_path` are only consumed inside the
        // `feature = "ws"` block below; the `not` arm doesn't read
        // them. Prefix with `_` so the default lib build doesn't flag
        // the bindings as unused.
        let _cert_path = if secure {
            self.config.tls_cert_path.as_deref()
        } else {
            None
        };

        let _key_path = if secure {
            self.config.tls_key_path.as_deref()
        } else {
            None
        };

        #[cfg(feature = "ws")]
        let result = WebSocketTransport::bind(
            bind_addr,
            secure,
            _cert_path,
            _key_path,
            Some(self.config.default_channel_capacity),
        )
        .await;

        #[cfg(not(feature = "ws"))]
        let result: Result<(WebSocketTransport, mpsc::Receiver<TransportEvent>)> =
            Err(Error::Transport("WebSocket support is not enabled".into()));

        let (transport, rx) = result.map_err(|e| {
            Error::Transport(format!(
                "Failed to bind WebSocket transport to {}: {}",
                bind_addr, e
            ))
        })?;

        let transport_arc = Arc::new(transport);

        // Store the transport
        let key = format!("{}:{}", if secure { "wss" } else { "ws" }, bind_addr);
        {
            let mut transports = self.transports.lock().await;
            transports.insert(key, transport_arc.clone());
        }

        // Process events from this transport
        self.clone()
            .process_transport_events(transport_arc.clone(), rx);

        info!(
            "Added {} transport bound to {}",
            if secure { "WSS" } else { "WS" },
            bind_addr
        );

        Ok(transport_arc)
    }

    /// Gets the default transport
    pub async fn default_transport(&self) -> Option<Arc<dyn Transport>> {
        self.default_transport.clone()
    }

    /// Gets a transport by key
    pub async fn get_transport(&self, key: &str) -> Option<Arc<dyn Transport>> {
        let transports = self.transports.lock().await;
        transports.get(key).cloned()
    }

    /// Gets a transport by type and address
    pub async fn get_transport_by_type_and_addr(
        &self,
        transport_type: &str,
        addr: SocketAddr,
    ) -> Option<Arc<dyn Transport>> {
        let key = format!("{}:{}", transport_type, addr);
        self.get_transport(&key).await
    }

    /// Gets a transport appropriate for the given destination
    pub async fn get_transport_for_destination(
        &self,
        _destination: SocketAddr,
    ) -> Option<Arc<dyn Transport>> {
        // For now, we just return the UDP transport
        // In the future, we'll add URI-based transport selection
        self.udp_transport.clone()
    }

    /// Extract the active transports as a `TransportType`-keyed map.
    ///
    /// Used by `MultiplexedTransport::new` to build a URI-aware
    /// dispatcher: the multiplexer holds one underlying `Transport` per
    /// flavour and routes outbound requests by reading the Request-URI's
    /// scheme + `transport=` parameter.
    ///
    /// Multiple transports of the same flavour bound to different
    /// addresses collapse to whichever one the underlying HashMap
    /// iteration sees first; this is acceptable today because session-core
    /// only ever binds one address per flavour. When that assumption
    /// breaks (multi-homed deployments), this helper should grow a
    /// destination-aware variant.
    pub async fn transports_by_flavour(&self) -> HashMap<TransportType, Arc<dyn Transport>> {
        let transports = self.transports.lock().await;
        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        for (key, transport) in transports.iter() {
            let flavour = match key.split(':').next().unwrap_or("") {
                "udp" => TransportType::Udp,
                "tcp" => TransportType::Tcp,
                "tls" => TransportType::Tls,
                "ws" => TransportType::Ws,
                "wss" => TransportType::Wss,
                other => {
                    warn!("transports_by_flavour: unrecognised key prefix '{}'", other);
                    continue;
                }
            };
            // First write wins — see doc above.
            by_flavour
                .entry(flavour)
                .or_insert_with(|| transport.clone());
        }
        by_flavour
    }

    /// Build a `MultiplexedTransport` over the manager's currently
    /// registered transports, suitable for installing as the single
    /// `Arc<dyn Transport>` that
    /// [`crate::transaction::TransactionManager::with_transport_manager`]
    /// stores. The default transport is reported via the multiplexer's
    /// `local_addr()` and used as the fallback when no flavour-specific
    /// transport is registered for an outbound request's URI scheme.
    pub async fn build_multiplexed_transport(&self) -> Result<Arc<dyn Transport>> {
        let default = self.default_transport().await.ok_or_else(|| {
            Error::Transport(
                "build_multiplexed_transport: TransportManager has no default transport".into(),
            )
        })?;
        let by_flavour = self.transports_by_flavour().await;
        let mux = MultiplexedTransport::new(default, by_flavour, self.sip_trace_runtime())
            .map_err(|e| Error::Transport(format!("MultiplexedTransport: {}", e)))?;
        Ok(Arc::new(mux))
    }

    /// Starts processing transport events
    fn start_event_processing(&self) {
        *self.running.try_lock().unwrap() = true;
    }

    /// Processes events from a specific transport
    fn process_transport_events(
        self,
        transport: Arc<dyn Transport>,
        mut rx: mpsc::Receiver<TransportEvent>,
    ) {
        tokio::spawn(async move {
            let transport_name = format!("{:?}", transport);
            let dispatch_workers =
                transport_event_dispatch_worker_count(self.config.transport_event_dispatch_workers);
            let dispatch_queue_capacity = transport_event_dispatch_queue_capacity(
                self.config.transport_event_dispatch_queue_capacity,
                self.config.default_channel_capacity,
            );
            let dispatch_senders = if dispatch_workers > DEFAULT_TRANSPORT_EVENT_DISPATCH_WORKERS {
                Some(start_transport_event_dispatch_workers(
                    self.event_tx.clone(),
                    dispatch_workers,
                    dispatch_queue_capacity,
                ))
            } else {
                None
            };
            let fallback_dispatch_worker = Arc::new(AtomicUsize::new(0));

            while let Some(event) = rx.recv().await {
                trace!("Received event from {}: {:?}", transport_name, event);
                let mut event = event;
                mark_transport_manager_forwarded(&mut event, Instant::now());

                // Forward the event to the main event channel. Avoid the
                // async send fast path unless the channel is actually full so
                // the per-transport event bridge does not add scheduler churn
                // to every UDP datagram under load.
                if let Some(dispatch_senders) = dispatch_senders.as_ref() {
                    dispatch_transport_event(event, dispatch_senders, &fallback_dispatch_worker)
                        .await;
                } else if !forward_transport_event(&self.event_tx, event).await {
                    break;
                }
            }

            debug!("Transport event processor for {} stopped", transport_name);
        });
    }

    /// Sends a message using the appropriate transport
    pub async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        // Get the appropriate transport
        let transport = self
            .get_transport_for_destination(destination)
            .await
            .ok_or_else(|| Error::Transport("No transport available for destination".into()))?;

        // Send the message
        transport
            .send_message(message, destination)
            .await
            .map_err(|e| Error::Transport(format!("Failed to send message: {}", e)))?;

        Ok(())
    }

    /// Closes all transports
    pub async fn close(&self) -> Result<()> {
        let transports = self.transports.lock().await;

        for (key, transport) in transports.iter() {
            if let Err(e) = transport.close().await {
                error!("Failed to close transport {}: {}", key, e);
            }
        }

        *self.running.lock().await = false;

        Ok(())
    }
}

fn mark_transport_manager_forwarded(event: &mut TransportEvent, forwarded_at: Instant) {
    let TransportEvent::MessageReceived {
        timing: Some(timing),
        ..
    } = event
    else {
        return;
    };

    if let Some(parse_completed_at) = timing.parse_completed_at {
        udp_diagnostics::record_parse_to_transport_manager(
            forwarded_at.duration_since(parse_completed_at),
        );
    }
    timing.transport_manager_forwarded_at = Some(forwarded_at);
}

/// Information about available transport types and capabilities
#[derive(Debug, Clone)]
pub struct TransportCapabilities {
    /// Whether UDP transport is supported
    pub supports_udp: bool,
    /// Whether TCP transport is supported
    pub supports_tcp: bool,
    /// Whether TLS transport is supported
    pub supports_tls: bool,
    /// Whether WebSocket transport is supported
    pub supports_ws: bool,
    /// Whether Secure WebSocket transport is supported
    pub supports_wss: bool,
    /// Local address used by the transport
    pub local_addr: Option<std::net::SocketAddr>,
    /// Default transport type
    pub default_transport: TransportType,
}

/// Detailed information about a specific transport type
#[derive(Debug, Clone)]
pub struct TransportInfo {
    /// Transport type
    pub transport_type: TransportType,
    /// Whether the transport is currently connected
    pub is_connected: bool,
    /// Local address for this transport
    pub local_addr: Option<std::net::SocketAddr>,
    /// Number of active connections (for connection-oriented transports)
    pub connection_count: usize,
}

/// Network information for SDP generation
#[derive(Debug, Clone)]
pub struct NetworkInfoForSdp {
    /// Local IP address to use in SDP
    pub local_ip: std::net::IpAddr,
    /// Port range for RTP (min, max)
    pub rtp_port_range: (u16, u16),
}

/// WebSocket connection status
#[derive(Debug, Clone)]
pub struct WebSocketStatus {
    /// Number of active insecure WebSocket connections
    pub ws_connections: usize,
    /// Number of active secure WebSocket connections
    pub wss_connections: usize,
    /// Whether there is at least one active WebSocket connection
    pub has_active_connection: bool,
}

/// Extension trait for the transport to provide additional capabilities
pub trait TransportCapabilitiesExt {
    /// Check if UDP transport is supported
    fn supports_udp(&self) -> bool;

    /// Check if TCP transport is supported
    fn supports_tcp(&self) -> bool;

    /// Check if TLS transport is supported
    fn supports_tls(&self) -> bool;

    /// Check if WebSocket transport is supported
    fn supports_ws(&self) -> bool;

    /// Check if Secure WebSocket transport is supported
    fn supports_wss(&self) -> bool;

    /// Check if a specific transport type is supported
    fn supports_transport(&self, transport_type: TransportType) -> bool;

    /// Get the default transport type
    fn default_transport_type(&self) -> TransportType;

    /// Check if a specific transport is currently connected
    fn is_transport_connected(&self, transport_type: TransportType) -> bool;

    /// Get the local address for a specific transport type
    fn get_transport_local_addr(
        &self,
        transport_type: TransportType,
    ) -> crate::transaction::error::Result<std::net::SocketAddr>;

    /// Get the number of active connections for a transport type
    fn get_connection_count(&self, transport_type: TransportType) -> usize;
}

impl<T: rvoip_sip_transport::Transport + ?Sized> TransportCapabilitiesExt for T {
    fn supports_udp(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_udp(self)
    }

    fn supports_tcp(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_tcp(self)
    }

    fn supports_tls(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_tls(self)
    }

    fn supports_ws(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_ws(self)
    }

    fn supports_wss(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_wss(self)
    }

    fn supports_transport(&self, transport_type: TransportType) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_transport(self, transport_type)
    }

    fn default_transport_type(&self) -> TransportType {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::default_transport_type(self)
    }

    fn is_transport_connected(&self, transport_type: TransportType) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::is_transport_connected(self, transport_type)
    }

    fn get_transport_local_addr(
        &self,
        _transport_type: TransportType,
    ) -> crate::transaction::error::Result<std::net::SocketAddr> {
        // Default implementation just returns the main local address
        self.local_addr().map_err(|e| {
            crate::transaction::error::Error::transport_error(e, "Failed to get local address")
        })
    }

    fn get_connection_count(&self, transport_type: TransportType) -> usize {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::get_connection_count(self, transport_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::Method;
    use rvoip_sip_transport::transport::TransportType;

    fn dispatch_request(
        method: Method,
        branch: &str,
        cseq: u32,
    ) -> std::result::Result<TransportEvent, Box<dyn std::error::Error>> {
        let request = rvoip_sip_core::builder::SimpleRequestBuilder::new(
            method.clone(),
            "sip:bob@example.com",
        )?
        .from("Alice", "sip:alice@example.com", Some("alice-dispatch-tag"))
        .to("Bob", "sip:bob@example.com", Some("bob-dispatch-tag"))
        .contact("sip:alice@127.0.0.1:5060", None)
        .call_id("dispatch-call-id-1234")
        .cseq(cseq)
        .via("127.0.0.1:5060", "UDP", Some(branch))
        .max_forwards(70)
        .build();

        Ok(TransportEvent::MessageReceived {
            message: Message::Request(request),
            source: "127.0.0.1:5060".parse().unwrap(),
            destination: "127.0.0.1:5061".parse().unwrap(),
            transport_type: TransportType::Udp,
            raw_bytes: None,
            timing: None,
        })
    }

    #[test]
    fn transport_event_dispatch_routes_dialog_requests_to_same_worker(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let fallback_worker = AtomicUsize::new(0);
        let worker_count = 4;

        let invite = dispatch_request(Method::Invite, "z9hG4bK.dispatch-invite", 101)?;
        let ack = dispatch_request(Method::Ack, "z9hG4bK.dispatch-ack", 101)?;
        let bye = dispatch_request(Method::Bye, "z9hG4bK.dispatch-bye", 102)?;
        let cancel = dispatch_request(Method::Cancel, "z9hG4bK.dispatch-cancel", 101)?;

        let expected =
            transport_event_dispatch_worker_index(&invite, worker_count, &fallback_worker);
        assert_eq!(
            transport_event_dispatch_worker_index(&ack, worker_count, &fallback_worker),
            expected
        );
        assert_eq!(
            transport_event_dispatch_worker_index(&bye, worker_count, &fallback_worker),
            expected
        );
        assert_eq!(
            transport_event_dispatch_worker_index(&cancel, worker_count, &fallback_worker),
            expected
        );
        assert_eq!(fallback_worker.load(Ordering::Relaxed), 0);

        Ok(())
    }

    #[test]
    fn transport_event_dispatch_round_robins_unkeyed_events() {
        let fallback_worker = AtomicUsize::new(0);
        let worker_count = 3;

        assert_eq!(
            transport_event_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            0
        );
        assert_eq!(
            transport_event_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            1
        );
        assert_eq!(
            transport_event_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            2
        );
        assert_eq!(
            transport_event_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            0
        );
    }

    #[tokio::test]
    async fn test_transport_manager_creation() {
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
            ..Default::default()
        };

        let (mut manager, _rx) = TransportManager::new(config).await.unwrap();

        // Initialize the manager
        let result = manager.initialize().await;
        assert!(
            result.is_ok(),
            "Failed to initialize transport manager: {:?}",
            result
        );

        // Verify UDP transport was created
        let udp_transport = manager.udp_transport.clone();
        assert!(udp_transport.is_some(), "UDP transport should exist");

        // Check if default transport is set
        let default_transport = manager.default_transport.clone();
        assert!(
            default_transport.is_some(),
            "Default transport should exist"
        );

        // Clean up
        let result = manager.close().await;
        assert!(
            result.is_ok(),
            "Failed to close transport manager: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_transport_manager_with_defaults() {
        let (mut manager, _rx) = TransportManager::with_defaults().await.unwrap();

        // Initialize the manager - will fail without UDP binding config
        let result = manager.initialize().await;
        assert!(
            result.is_ok(),
            "Failed to initialize transport manager: {:?}",
            result
        );

        // Clean up
        let result = manager.close().await;
        assert!(
            result.is_ok(),
            "Failed to close transport manager: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_send_message() {
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
            ..Default::default()
        };

        let (mut manager, mut rx) = TransportManager::new(config).await.unwrap();

        // Initialize the manager
        let result = manager.initialize().await;
        assert!(
            result.is_ok(),
            "Failed to initialize transport manager: {:?}",
            result
        );

        // Create a test message
        let message = Message::Request(
            rvoip_sip_core::builder::SimpleRequestBuilder::new(
                rvoip_sip_core::Method::Register,
                "sip:example.com",
            )
            .unwrap()
            .from("user", "sip:user@example.com", None)
            .to("user", "sip:user@example.com", None)
            .call_id("test-call-id")
            .cseq(1)
            .build(),
        );

        // Get local address to send message to
        let local_addr = manager
            .udp_transport
            .as_ref()
            .unwrap()
            .local_addr()
            .unwrap();

        // Send message
        let result = manager.send_message(message.clone(), local_addr).await;
        assert!(result.is_ok(), "Failed to send message: {:?}", result);

        // Should receive a transport event
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("Timed out waiting for transport event")
            .expect("No transport event received");

        // Clean up
        let result = manager.close().await;
        assert!(
            result.is_ok(),
            "Failed to close transport manager: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_transport_send_message() {
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
            ..Default::default()
        };

        let (mut manager, mut rx) = TransportManager::new(config).await.unwrap();

        // Initialize the manager
        let result = manager.initialize().await;
        assert!(
            result.is_ok(),
            "Failed to initialize transport manager: {:?}",
            result
        );

        // Create a test message
        let message = Message::Request(
            rvoip_sip_core::builder::SimpleRequestBuilder::new(
                rvoip_sip_core::Method::Register,
                "sip:example.com",
            )
            .unwrap()
            .from("user", "sip:user@example.com", None)
            .to("user", "sip:user@example.com", None)
            .call_id("test-call-id")
            .cseq(1)
            .build(),
        );

        // Get local address to send message to
        let local_addr = manager
            .udp_transport
            .as_ref()
            .unwrap()
            .local_addr()
            .unwrap();

        // Send message
        let result = manager.send_message(message.clone(), local_addr).await;
        assert!(result.is_ok(), "Failed to send message: {:?}", result);

        // Should receive a transport event
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("Timed out waiting for transport event")
            .expect("No transport event received");

        // Clean up
        let result = manager.close().await;
        assert!(
            result.is_ok(),
            "Failed to close transport manager: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_integration_with_transaction_manager() {
        use crate::transaction::manager::TransactionManager;

        // Create a transport manager
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
            ..Default::default()
        };

        let (mut transport_manager, transport_rx) = TransportManager::new(config).await.unwrap();

        // Initialize the transport manager
        let result = transport_manager.initialize().await;
        assert!(
            result.is_ok(),
            "Failed to initialize transport manager: {:?}",
            result
        );

        // Create transaction manager with the transport manager
        let (transaction_manager, mut tx_events_rx) = TransactionManager::with_transport_manager(
            transport_manager.clone(),
            transport_rx,
            Some(100),
        )
        .await
        .unwrap();

        // Create a test message and send it
        let request = rvoip_sip_core::builder::SimpleRequestBuilder::new(
            rvoip_sip_core::Method::Register,
            "sip:example.com",
        )
        .unwrap()
        .from("user", "sip:user@example.com", None)
        .to("user", "sip:user@example.com", None)
        .call_id("test-call-id")
        .cseq(1)
        .max_forwards(70)
        .contact("sip:user@127.0.0.1:5060", None)
        .build();

        // Get local address
        let local_addr = transport_manager
            .udp_transport
            .as_ref()
            .unwrap()
            .local_addr()
            .unwrap();

        // Create a client transaction
        let tx_id = transaction_manager
            .create_client_transaction(request.clone(), local_addr)
            .await
            .expect("Failed to create client transaction");

        // Send the request
        let result = transaction_manager.send_request(&tx_id).await;
        assert!(
            result.is_ok(),
            "Failed to send request through transaction manager: {:?}",
            result
        );

        // Verify transaction state change event
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), tx_events_rx.recv())
            .await
            .expect("Timed out waiting for transaction event")
            .expect("No transaction event received");

        match event {
            crate::transaction::TransactionEvent::StateChanged { transaction_id, .. } => {
                assert_eq!(transaction_id, tx_id, "Transaction IDs should match");
            }
            _ => panic!("Expected StateChanged event, got {:?}", event),
        }

        // Clean up - shutdown transaction manager
        transaction_manager.shutdown().await;
    }
}
