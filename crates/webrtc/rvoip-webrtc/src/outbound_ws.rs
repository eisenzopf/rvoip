//! Multiplexed target-contacting WebSocket signaling client.
//!
//! One bounded hub owns an authenticated `rvoip.webrtc.v1` socket for an exact
//! endpoint, network policy, credential provider, and credential partition.
//! Logical WebRTC routes retain independent command/event queues and are
//! correlated by request id until the server assigns a connection id. Ending
//! one route sends one BYE without closing sibling routes on the same socket.

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch, Mutex as AsyncMutex, Notify};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream};
use webrtc::peer_connection::RTCIceCandidateInit;
use zeroize::Zeroize;

use crate::errors::{Result, WebRtcError};
use crate::originate::{WebRtcOriginateContext, WebRtcOriginateContextError};
use crate::signaling::websocket::SignalingMessage;

pub(crate) const RVOIP_WEBRTC_SUBPROTOCOL: &str = "rvoip.webrtc.v1";
const COMMAND_CAPACITY: usize = 128;
const EVENT_CAPACITY: usize = 128;
const HUB_COMMAND_CAPACITY: usize = 1_024;
const MAX_ROUTES_PER_HUB: usize = 1_024;
const MAX_POOL_KEYS: usize = 1_024;
const MAX_SIGNALING_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_HUB_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_HUB_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
const MIN_HUB_PING_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);

#[derive(Debug)]
pub(crate) enum WsClientCommand {
    Offer {
        sdp: String,
        request_id: String,
        /// Negotiate the request-bound `ready`/`rejected` extension. False
        /// retains the original `offer` wire contract.
        require_ready: bool,
    },
    Candidate {
        connection_id: String,
        candidate: RTCIceCandidateInit,
    },
    Complete {
        connection_id: String,
    },
    Bye {
        connection_id: String,
    },
}

#[derive(Debug)]
pub(crate) enum WsClientEvent {
    Answer {
        sdp: String,
        connection_id: String,
        request_id: String,
    },
    Candidate {
        connection_id: String,
        candidate: RTCIceCandidateInit,
    },
    Complete {
        connection_id: String,
    },
    /// The remote application accepted this exact request/connection pair.
    Ready {
        connection_id: String,
        request_id: String,
    },
    /// The remote application rejected this exact request/connection pair.
    Rejected {
        connection_id: String,
        request_id: String,
    },
    Bye {
        connection_id: String,
    },
    Closed,
}

pub(crate) struct WsClientSession {
    pub commands: mpsc::Sender<WsClientCommand>,
    pub events: mpsc::Receiver<WsClientEvent>,
    pub task: JoinHandle<()>,
}

#[derive(Clone)]
pub(crate) struct WsClientPool {
    inner: Arc<WsClientPoolInner>,
}

struct WsClientPoolInner {
    slots: DashMap<WsPoolKey, Arc<WsHubSlot>>,
    key_count: AtomicUsize,
    next_route: AtomicU64,
    next_hub: AtomicU64,
    drivers: DashMap<u64, WsHubDriver>,
    driver_exit: Notify,
    draining: AtomicBool,
}

struct WsHubDriver {
    shutdown: watch::Sender<bool>,
    abort: AbortHandle,
}

struct WsHubSlot {
    hub: AsyncMutex<Weak<WsHub>>,
    generation: AtomicU64,
}

struct WsHub {
    commands: mpsc::Sender<HubCommand>,
    closed: Arc<AtomicBool>,
    // Retain the provider allocation used by the redacted pool key. This
    // prevents allocator address reuse from aliasing a still-live socket to a
    // newly-created credential authority.
    _security_context: Arc<WebRtcOriginateContext>,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct WsPoolKey([u8; 32]);

impl std::fmt::Debug for WsPoolKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WsPoolKey([redacted])")
    }
}

enum HubCommand {
    Register {
        route: u64,
        events: mpsc::Sender<WsClientEvent>,
        acknowledgement: oneshot::Sender<std::result::Result<(), ()>>,
    },
    Signal {
        route: u64,
        command: WsClientCommand,
    },
    Release {
        route: u64,
    },
}

struct HubRoute {
    events: mpsc::Sender<WsClientEvent>,
    request_id: Option<String>,
    connection_id: Option<String>,
}

impl Default for WsClientPool {
    fn default() -> Self {
        Self {
            inner: Arc::new(WsClientPoolInner {
                slots: DashMap::new(),
                key_count: AtomicUsize::new(0),
                next_route: AtomicU64::new(1),
                next_hub: AtomicU64::new(1),
                drivers: DashMap::new(),
                driver_exit: Notify::new(),
                draining: AtomicBool::new(false),
            }),
        }
    }
}

impl WsClientPool {
    pub async fn open(&self, context: Arc<WebRtcOriginateContext>) -> Result<WsClientSession> {
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(WebRtcError::Signaling(
                "WebRTC signaling hub pool is draining".into(),
            ));
        }
        let key = WsPoolKey::for_context(&context);
        for _ in 0..2 {
            let slot = self.slot(key)?;
            let hub = self
                .acquire_hub(key, Arc::clone(&slot), Arc::clone(&context))
                .await?;
            match self.register_route(&hub, &context).await {
                Ok(session) => return Ok(session),
                Err(()) => continue,
            }
        }
        Err(WebRtcError::Signaling(
            "WebRTC signaling hub stopped during route registration".into(),
        ))
    }

    fn slot(&self, key: WsPoolKey) -> Result<Arc<WsHubSlot>> {
        match self.inner.slots.entry(key) {
            Entry::Occupied(entry) => Ok(Arc::clone(entry.get())),
            Entry::Vacant(entry) => {
                self.inner
                    .key_count
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                        (count < MAX_POOL_KEYS).then_some(count + 1)
                    })
                    .map_err(|_| {
                        WebRtcError::Signaling("WebRTC signaling origin cap reached".into())
                    })?;
                Ok(Arc::clone(&entry.insert(Arc::new(WsHubSlot {
                    hub: AsyncMutex::new(Weak::new()),
                    generation: AtomicU64::new(0),
                }))))
            }
        }
    }

    async fn acquire_hub(
        &self,
        key: WsPoolKey,
        slot: Arc<WsHubSlot>,
        context: Arc<WebRtcOriginateContext>,
    ) -> Result<Arc<WsHub>> {
        let mut retained = slot.hub.lock().await;
        if let Some(hub) = retained.upgrade() {
            if !hub.closed.load(Ordering::Acquire) {
                return Ok(hub);
            }
        }

        let stream = match connect_checked(&context).await {
            Ok(stream) => stream,
            Err(error) => {
                drop(retained);
                self.remove_unused_slot(key, &slot);
                return Err(error);
            }
        };
        if self.inner.draining.load(Ordering::Acquire) {
            drop(retained);
            self.remove_unused_slot(key, &slot);
            return Err(WebRtcError::Signaling(
                "WebRTC signaling hub pool is draining".into(),
            ));
        }
        let generation = slot.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (commands, command_rx) = mpsc::channel(HUB_COMMAND_CAPACITY);
        let closed = Arc::new(AtomicBool::new(false));
        let hub = Arc::new(WsHub {
            commands,
            closed: Arc::clone(&closed),
            _security_context: Arc::clone(&context),
        });
        *retained = Arc::downgrade(&hub);
        drop(retained);

        let pool = Arc::downgrade(&self.inner);
        let driver_id = self.inner.next_hub.fetch_add(1, Ordering::Relaxed);
        let heartbeat_timeout = context.target_policy().signaling_timeout();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (start, started) = oneshot::channel();
        let driver = tokio::spawn(async move {
            let _exit = HubDriverExitGuard {
                pool: pool.clone(),
                driver_id,
            };
            if started.await.is_err() {
                return;
            }
            drive_hub_socket(
                stream,
                command_rx,
                closed,
                pool,
                key,
                slot,
                generation,
                driver_id,
                heartbeat_timeout,
                shutdown_rx,
            )
            .await;
        });
        self.inner.drivers.insert(
            driver_id,
            WsHubDriver {
                shutdown,
                abort: driver.abort_handle(),
            },
        );
        let _ = start.send(());
        Ok(hub)
    }

    async fn register_route(
        &self,
        hub: &Arc<WsHub>,
        context: &WebRtcOriginateContext,
    ) -> std::result::Result<WsClientSession, ()> {
        let route = self.inner.next_route.fetch_add(1, Ordering::Relaxed);
        let (event_tx, events) = mpsc::channel(EVENT_CAPACITY);
        let (acknowledgement, acknowledged) = oneshot::channel();
        let deadline = context.target_policy().signaling_timeout();
        tokio::time::timeout(
            deadline,
            hub.commands.send(HubCommand::Register {
                route,
                events: event_tx,
                acknowledgement,
            }),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
        tokio::time::timeout(deadline, acknowledged)
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?
            .map_err(|_| ())?;

        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let hub = Arc::clone(hub);
        let task = tokio::spawn(drive_route_proxy(hub, route, command_rx));
        Ok(WsClientSession {
            commands,
            events,
            task,
        })
    }

    fn remove_unused_slot(&self, key: WsPoolKey, slot: &Arc<WsHubSlot>) {
        if self
            .inner
            .slots
            .remove_if(&key, |_, candidate| {
                Arc::ptr_eq(candidate, slot) && Arc::strong_count(candidate) <= 2
            })
            .is_some()
        {
            self.inner.key_count.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub(crate) fn live_driver_count(&self) -> usize {
        self.inner.drivers.len()
    }

    pub(crate) async fn drain(&self, timeout: std::time::Duration) -> bool {
        self.inner.draining.store(true, Ordering::Release);
        let deadline = tokio::time::Instant::now() + timeout;
        for driver in self.inner.drivers.iter() {
            driver.shutdown.send_replace(true);
        }
        loop {
            let exited = self.inner.driver_exit.notified();
            if self.inner.drivers.is_empty() {
                return true;
            }
            if tokio::time::timeout_at(deadline, exited).await.is_err() {
                self.abort_all();
                let abort_deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_millis(100);
                loop {
                    let exited = self.inner.driver_exit.notified();
                    if self.inner.drivers.is_empty() {
                        return false;
                    }
                    if tokio::time::timeout_at(abort_deadline, exited)
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
            }
        }
    }

    pub(crate) fn abort_all(&self) {
        self.inner.draining.store(true, Ordering::Release);
        for driver in self.inner.drivers.iter() {
            driver.abort.abort();
        }
    }
}

impl WsPoolKey {
    fn for_context(context: &WebRtcOriginateContext) -> Self {
        let policy = context.target_policy();
        let mut digest = Sha256::new();
        digest.update(context.endpoint().as_str().as_bytes());
        digest.update([0]);
        digest.update(policy.credential_partition().as_bytes());
        digest.update([0]);
        digest.update(context.bearer_provider_identity().to_le_bytes());
        digest.update([u8::from(policy.allows_loopback())]);
        digest.update([u8::from(policy.allows_private_networks())]);
        digest.update(policy.max_resolved_addresses().to_le_bytes());
        digest.update(policy.connect_timeout().as_nanos().to_le_bytes());
        digest.update(policy.signaling_timeout().as_nanos().to_le_bytes());
        #[cfg(feature = "tls-rustls")]
        match context.tls_trust_profile_identity() {
            Some(identity) => {
                digest.update([1]);
                digest.update(identity);
            }
            None => digest.update([0]),
        }
        Self(digest.finalize().into())
    }
}

struct HubDriverExitGuard {
    pool: Weak<WsClientPoolInner>,
    driver_id: u64,
}

impl Drop for HubDriverExitGuard {
    fn drop(&mut self) {
        let Some(pool) = self.pool.upgrade() else {
            return;
        };
        pool.drivers.remove(&self.driver_id);
        pool.driver_exit.notify_waiters();
    }
}

#[cfg(test)]
impl WsClientSession {
    pub async fn connect(context: Arc<WebRtcOriginateContext>) -> Result<Self> {
        WsClientPool::default().open(context).await
    }
}

type ClientStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect_checked(context: &WebRtcOriginateContext) -> Result<ClientStream> {
    context.validate().map_err(context_error_to_webrtc_error)?;
    let endpoint = context.endpoint();
    let host = endpoint
        .host_str()
        .ok_or_else(|| WebRtcError::Signaling("outbound target has no host".into()))?;
    let port = endpoint
        .port_or_known_default()
        .ok_or_else(|| WebRtcError::Signaling("outbound target has no port".into()))?;
    let policy = context.target_policy();
    let resolved = tokio::time::timeout(
        policy.connect_timeout(),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| WebRtcError::Timeout("WebRTC target resolution"))?
    .map_err(|_| WebRtcError::Signaling("WebRTC target resolution failed".into()))?;
    let mut addresses = BTreeSet::new();
    for address in resolved {
        addresses.insert(address);
        if addresses.len() > policy.max_resolved_addresses() {
            return Err(context_error_to_webrtc_error(
                WebRtcOriginateContextError::TooManyResolvedAddresses,
            ));
        }
    }
    if addresses.is_empty() {
        return Err(WebRtcError::Signaling(
            "WebRTC target resolution returned no addresses".into(),
        ));
    }
    // Reject the entire answer set when it contains any forbidden address.
    // Selecting only a public member would leave DNS rebinding and mixed-set
    // policy dependent on resolver ordering.
    if addresses
        .iter()
        .any(|address| !policy.address_allowed(address.ip()))
    {
        return Err(context_error_to_webrtc_error(
            WebRtcOriginateContextError::AddressForbidden,
        ));
    }

    let tcp = connect_one(&addresses, policy.connect_timeout()).await?;
    tcp.set_nodelay(true)
        .map_err(|_| WebRtcError::Signaling("WebRTC socket configuration failed".into()))?;

    let mut request = endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| WebRtcError::Signaling("WebRTC upgrade request is invalid".into()))?;
    let credential = context
        .bearer_credential()
        .await
        .map_err(context_error_to_webrtc_error)?;
    let mut protocols = RVOIP_WEBRTC_SUBPROTOCOL.to_owned();
    if let Some(credential) = credential.as_ref() {
        protocols.push_str(", token.");
        protocols.push_str(credential.expose_secret());
    }
    let mut protocol_header = HeaderValue::from_str(&protocols)
        .map_err(|_| WebRtcError::Signaling("WebRTC authentication header is invalid".into()))?;
    protocol_header.set_sensitive(true);
    protocols.zeroize();
    request
        .headers_mut()
        .insert(header::SEC_WEBSOCKET_PROTOCOL, protocol_header);

    let websocket_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_SIGNALING_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_SIGNALING_MESSAGE_BYTES));
    #[cfg(feature = "tls-rustls")]
    let tls_connector = context.tls_trust().map(build_tls_connector).transpose()?;
    #[cfg(not(feature = "tls-rustls"))]
    let tls_connector: Option<Connector> = None;
    let (stream, response) = tokio::time::timeout(
        policy.connect_timeout(),
        client_async_tls_with_config(request, tcp, Some(websocket_config), tls_connector),
    )
    .await
    .map_err(|_| WebRtcError::Timeout("WebRTC WebSocket upgrade"))?
    .map_err(|_| WebRtcError::Signaling("WebRTC WebSocket upgrade failed".into()))?;

    let selected = response
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok());
    if selected != Some(RVOIP_WEBRTC_SUBPROTOCOL) {
        return Err(WebRtcError::Signaling(
            "WebRTC WebSocket subprotocol mismatch".into(),
        ));
    }
    Ok(stream)
}

#[cfg(feature = "tls-rustls")]
fn build_tls_connector(trust: &crate::originate::WebRtcTlsClientTrust) -> Result<Connector> {
    let mut roots = rustls::RootCertStore::empty();
    // Custom profile roots are additive. Public WebRTC signaling targets
    // must continue to validate against the ordinary WebPKI root set.
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for certificate in trust.certificates() {
        roots.add(certificate.clone()).map_err(|_| {
            WebRtcError::Signaling("WebRTC TLS trust profile construction failed".into())
        })?;
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

async fn connect_one(
    addresses: &BTreeSet<SocketAddr>,
    timeout: std::time::Duration,
) -> Result<TcpStream> {
    let deadline = tokio::time::Instant::now() + timeout;
    for address in addresses {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(_)) => continue,
            Err(_) => break,
        }
    }
    Err(WebRtcError::Signaling(
        "WebRTC target connection failed".into(),
    ))
}

async fn drive_route_proxy(
    hub: Arc<WsHub>,
    route: u64,
    mut commands: mpsc::Receiver<WsClientCommand>,
) {
    let release = RouteRelease {
        commands: hub.commands.clone(),
        route,
    };
    while let Some(command) = commands.recv().await {
        if hub
            .commands
            .send(HubCommand::Signal { route, command })
            .await
            .is_err()
        {
            break;
        }
    }
    drop(release);
}

struct RouteRelease {
    commands: mpsc::Sender<HubCommand>,
    route: u64,
}

impl Drop for RouteRelease {
    fn drop(&mut self) {
        let _ = self
            .commands
            .try_send(HubCommand::Release { route: self.route });
    }
}

#[allow(clippy::too_many_arguments)]
async fn drive_hub_socket(
    stream: ClientStream,
    mut commands: mpsc::Receiver<HubCommand>,
    closed: Arc<AtomicBool>,
    pool: Weak<WsClientPoolInner>,
    key: WsPoolKey,
    slot: Arc<WsHubSlot>,
    generation: u64,
    driver_id: u64,
    heartbeat_timeout: std::time::Duration,
    mut shutdown: watch::Receiver<bool>,
) {
    let (mut write, mut read) = stream.split();
    let mut routes: HashMap<u64, HubRoute> = HashMap::new();
    let mut requests: HashMap<String, u64> = HashMap::new();
    let mut connections: HashMap<String, u64> = HashMap::new();
    let mut prune = tokio::time::interval(std::time::Duration::from_secs(1));
    prune.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    prune.tick().await;
    let ping_interval = (heartbeat_timeout / 2)
        .max(MIN_HUB_PING_INTERVAL)
        .min(MAX_HUB_PING_INTERVAL);
    let io_timeout = heartbeat_timeout.min(MAX_HUB_IO_TIMEOUT);
    let mut heartbeat = tokio::time::interval(ping_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;
    let mut ping_sequence = 0_u64;
    let mut expected_pong = None;
    let mut pong_deadline = None;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            _ = wait_for_deadline(pong_deadline) => {
                break;
            }
            _ = heartbeat.tick(), if expected_pong.is_none() => {
                ping_sequence = ping_sequence.wrapping_add(1);
                let mut payload = Vec::with_capacity(16);
                payload.extend_from_slice(&driver_id.to_be_bytes());
                payload.extend_from_slice(&ping_sequence.to_be_bytes());
                if !send_hub_frame(&mut write, Message::Ping(payload.clone().into()), io_timeout).await {
                    break;
                }
                expected_pong = Some(payload);
                pong_deadline = Some(tokio::time::Instant::now() + heartbeat_timeout);
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    break;
                };
                match command {
                    HubCommand::Register { route, events, acknowledgement } => {
                        if routes.len() >= MAX_ROUTES_PER_HUB || routes.contains_key(&route) {
                            let _ = acknowledgement.send(Err(()));
                            continue;
                        }
                        routes.insert(route, HubRoute {
                            events,
                            request_id: None,
                            connection_id: None,
                        });
                        let _ = acknowledgement.send(Ok(()));
                    }
                    HubCommand::Signal { route, command } => {
                        let valid = match (&command, routes.get(&route)) {
                            (WsClientCommand::Offer { request_id, .. }, Some(state)) => {
                                state.request_id.is_none()
                                    && valid_request_id(request_id)
                                    && !requests.contains_key(request_id)
                            }
                            (WsClientCommand::Candidate { connection_id, .. }, Some(state))
                            | (WsClientCommand::Complete { connection_id }, Some(state))
                            | (WsClientCommand::Bye { connection_id }, Some(state)) => {
                                state.connection_id.as_deref() == Some(connection_id.as_str())
                            }
                            (_, None) => false,
                        };
                        if !valid {
                            send_route_bye(&mut write, route_connection(&routes, route), io_timeout).await;
                            evict_route(route, &mut routes, &mut requests, &mut connections, true);
                            continue;
                        }
                        let is_bye = matches!(command, WsClientCommand::Bye { .. });
                        let request_id = match &command {
                            WsClientCommand::Offer { request_id, .. } => Some(request_id.clone()),
                            _ => None,
                        };
                        let message = match command_to_message(command) {
                            Ok(message) => message,
                            Err(_) => {
                                send_route_bye(&mut write, route_connection(&routes, route), io_timeout).await;
                                evict_route(route, &mut routes, &mut requests, &mut connections, true);
                                continue;
                            }
                        };
                        if !send_hub_signaling_message(&mut write, &message, io_timeout).await {
                            break;
                        }
                        if let Some(request_id) = request_id {
                            if let Some(state) = routes.get_mut(&route) {
                                state.request_id = Some(request_id.clone());
                                requests.insert(request_id, route);
                            }
                        }
                        if is_bye {
                            evict_route(route, &mut routes, &mut requests, &mut connections, false);
                        }
                    }
                    HubCommand::Release { route } => {
                        if let Some(connection_id) = routes
                            .get(&route)
                            .and_then(|state| state.connection_id.clone())
                        {
                            if let Ok(message) = command_to_message(WsClientCommand::Bye {
                                connection_id,
                            }) {
                                let _ = send_hub_signaling_message(&mut write, &message, io_timeout).await;
                            }
                        }
                        evict_route(route, &mut routes, &mut requests, &mut connections, false);
                    }
                }
            }
            frame = read.next() => {
                let Some(frame) = frame else { break; };
                let Ok(frame) = frame else { break; };
                match frame {
                    Message::Text(text) => {
                        if text.len() > MAX_SIGNALING_MESSAGE_BYTES {
                            break;
                        }
                        let Ok(message) = serde_json::from_str::<SignalingMessage>(&text) else {
                            break;
                        };
                        let Ok(event) = message_to_event(message) else {
                            break;
                        };
                        match event {
                            WsClientEvent::Answer { sdp, connection_id, request_id } => {
                                let Some(route) = requests.get(&request_id).copied() else {
                                    continue;
                                };
                                if connections.contains_key(&connection_id)
                                    || routes.get(&route).is_some_and(|state| state.connection_id.is_some())
                                {
                                    break;
                                }
                                let event = WsClientEvent::Answer {
                                    sdp,
                                    connection_id: connection_id.clone(),
                                    request_id,
                                };
                                if !deliver_route_event(&routes, route, event) {
                                    send_route_bye(&mut write, Some(connection_id), io_timeout).await;
                                    evict_route(route, &mut routes, &mut requests, &mut connections, false);
                                    continue;
                                }
                                if let Some(state) = routes.get_mut(&route) {
                                    state.connection_id = Some(connection_id.clone());
                                    connections.insert(connection_id, route);
                                }
                            }
                            WsClientEvent::Candidate { connection_id, candidate } => {
                                let Some(route) = connections.get(&connection_id).copied() else {
                                    continue;
                                };
                                if !deliver_route_event(
                                    &routes,
                                    route,
                                    WsClientEvent::Candidate { connection_id, candidate },
                                ) {
                                    send_route_bye(&mut write, route_connection(&routes, route), io_timeout).await;
                                    evict_route(route, &mut routes, &mut requests, &mut connections, false);
                                }
                            }
                            WsClientEvent::Complete { connection_id } => {
                                let Some(route) = connections.get(&connection_id).copied() else {
                                    continue;
                                };
                                if !deliver_route_event(
                                    &routes,
                                    route,
                                    WsClientEvent::Complete { connection_id },
                                ) {
                                    send_route_bye(&mut write, route_connection(&routes, route), io_timeout).await;
                                    evict_route(route, &mut routes, &mut requests, &mut connections, false);
                                }
                            }
                            WsClientEvent::Ready { connection_id, request_id } => {
                                let request_route = requests.get(&request_id).copied();
                                let connection_route = connections.get(&connection_id).copied();
                                let (Some(request_route), Some(connection_route)) =
                                    (request_route, connection_route)
                                else {
                                    break;
                                };
                                if request_route != connection_route
                                    || !deliver_route_event(
                                        &routes,
                                        request_route,
                                        WsClientEvent::Ready {
                                            connection_id,
                                            request_id,
                                        },
                                    )
                                {
                                    break;
                                }
                            }
                            WsClientEvent::Rejected { connection_id, request_id } => {
                                let request_route = requests.get(&request_id).copied();
                                let connection_route = connections.get(&connection_id).copied();
                                let (Some(request_route), Some(connection_route)) =
                                    (request_route, connection_route)
                                else {
                                    break;
                                };
                                if request_route != connection_route {
                                    break;
                                }
                                let _ = deliver_route_event(
                                    &routes,
                                    request_route,
                                    WsClientEvent::Rejected {
                                        connection_id,
                                        request_id,
                                    },
                                );
                                evict_route(
                                    request_route,
                                    &mut routes,
                                    &mut requests,
                                    &mut connections,
                                    false,
                                );
                            }
                            WsClientEvent::Bye { connection_id } => {
                                let Some(route) = connections.get(&connection_id).copied() else {
                                    continue;
                                };
                                let _ = deliver_route_event(
                                    &routes,
                                    route,
                                    WsClientEvent::Bye { connection_id },
                                );
                                evict_route(route, &mut routes, &mut requests, &mut connections, false);
                            }
                            WsClientEvent::Closed => break,
                        }
                    }
                    Message::Ping(payload) => {
                        if !send_hub_frame(&mut write, Message::Pong(payload), io_timeout).await {
                            break;
                        }
                    }
                    Message::Pong(payload) => {
                        if expected_pong
                            .as_ref()
                            .is_some_and(|expected| expected.as_slice() == payload.as_ref())
                        {
                            expected_pong = None;
                            pong_deadline = None;
                        }
                    }
                    Message::Close(_) => break,
                    Message::Binary(_) | Message::Frame(_) => {}
                }
            }
            _ = prune.tick() => {
                let abandoned: Vec<u64> = routes
                    .iter()
                    .filter_map(|(route, state)| state.events.is_closed().then_some(*route))
                    .collect();
                for route in abandoned {
                    if let Some(connection_id) = routes
                        .get(&route)
                        .and_then(|state| state.connection_id.clone())
                    {
                        if let Ok(message) = command_to_message(WsClientCommand::Bye { connection_id }) {
                            let _ = send_hub_signaling_message(&mut write, &message, io_timeout).await;
                        }
                    }
                    evict_route(route, &mut routes, &mut requests, &mut connections, false);
                }
            }
        }
    }
    closed.store(true, Ordering::Release);
    for (_, route) in routes.drain() {
        let _ = route.events.try_send(WsClientEvent::Closed);
    }
    let _ = send_hub_frame(&mut write, Message::Close(None), io_timeout).await;
    cleanup_pool_slot(pool, key, slot, generation).await;
}

async fn wait_for_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn deliver_route_event(routes: &HashMap<u64, HubRoute>, route: u64, event: WsClientEvent) -> bool {
    routes
        .get(&route)
        .is_some_and(|state| state.events.try_send(event).is_ok())
}

fn route_connection(routes: &HashMap<u64, HubRoute>, route: u64) -> Option<String> {
    routes
        .get(&route)
        .and_then(|state| state.connection_id.clone())
}

async fn send_route_bye<S>(
    write: &mut S,
    connection_id: Option<String>,
    timeout: std::time::Duration,
) where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::fmt::Debug,
{
    let Some(connection_id) = connection_id else {
        return;
    };
    if let Ok(message) = command_to_message(WsClientCommand::Bye { connection_id }) {
        let _ = send_hub_signaling_message(write, &message, timeout).await;
    }
}

async fn send_hub_signaling_message<S>(
    write: &mut S,
    message: &SignalingMessage,
    timeout: std::time::Duration,
) -> bool
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::fmt::Debug,
{
    matches!(
        tokio::time::timeout(timeout, send_signaling_message(write, message)).await,
        Ok(Ok(()))
    )
}

async fn send_hub_frame<S>(write: &mut S, message: Message, timeout: std::time::Duration) -> bool
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::fmt::Debug,
{
    matches!(
        tokio::time::timeout(timeout, write.send(message)).await,
        Ok(Ok(()))
    )
}

fn evict_route(
    route: u64,
    routes: &mut HashMap<u64, HubRoute>,
    requests: &mut HashMap<String, u64>,
    connections: &mut HashMap<String, u64>,
    notify: bool,
) {
    let Some(state) = routes.remove(&route) else {
        return;
    };
    if let Some(request_id) = state.request_id {
        requests.remove(&request_id);
    }
    if let Some(connection_id) = state.connection_id {
        connections.remove(&connection_id);
    }
    if notify {
        let _ = state.events.try_send(WsClientEvent::Closed);
    }
}

async fn cleanup_pool_slot(
    pool: Weak<WsClientPoolInner>,
    key: WsPoolKey,
    slot: Arc<WsHubSlot>,
    generation: u64,
) {
    let mut retained = slot.hub.lock().await;
    if slot.generation.load(Ordering::Acquire) != generation {
        return;
    }
    *retained = Weak::new();
    drop(retained);
    let Some(pool) = pool.upgrade() else {
        return;
    };
    if pool
        .slots
        .remove_if(&key, |_, candidate| {
            Arc::ptr_eq(candidate, &slot)
                && candidate.generation.load(Ordering::Acquire) == generation
                && Arc::strong_count(candidate) <= 2
        })
        .is_some()
    {
        pool.key_count.fetch_sub(1, Ordering::AcqRel);
    }
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn command_to_message(command: WsClientCommand) -> Result<SignalingMessage> {
    match command {
        WsClientCommand::Offer {
            sdp,
            request_id,
            require_ready,
        } => Ok(SignalingMessage {
            msg_type: if require_ready {
                "offer-ready".into()
            } else {
                "offer".into()
            },
            sdp,
            request_id,
            ..Default::default()
        }),
        WsClientCommand::Candidate {
            connection_id,
            candidate,
        } => {
            let candidate = serde_json::to_string(&candidate).map_err(|_| {
                WebRtcError::Signaling("WebRTC ICE candidate serialization failed".into())
            })?;
            Ok(SignalingMessage {
                msg_type: "ice-candidate".into(),
                connection_id,
                candidate,
                request_id: String::new(),
                ..Default::default()
            })
        }
        WsClientCommand::Complete { connection_id } => Ok(SignalingMessage {
            msg_type: "ice-complete".into(),
            connection_id,
            request_id: String::new(),
            ..Default::default()
        }),
        WsClientCommand::Bye { connection_id } => Ok(SignalingMessage {
            msg_type: "bye".into(),
            connection_id,
            request_id: String::new(),
            ..Default::default()
        }),
    }
}

fn message_to_event(message: SignalingMessage) -> Result<WsClientEvent> {
    match message.msg_type.as_str() {
        "answer"
            if !message.sdp.is_empty()
                && !message.connection_id.is_empty()
                && !message.request_id.is_empty() =>
        {
            Ok(WsClientEvent::Answer {
                sdp: message.sdp,
                connection_id: message.connection_id,
                request_id: message.request_id,
            })
        }
        "ice-candidate" if !message.candidate.is_empty() && !message.connection_id.is_empty() => {
            let candidate = serde_json::from_str(&message.candidate).map_err(|_| {
                WebRtcError::Signaling("WebRTC ICE candidate payload is invalid".into())
            })?;
            Ok(WsClientEvent::Candidate {
                connection_id: message.connection_id,
                candidate,
            })
        }
        "ice-complete" if message.candidate.is_empty() && !message.connection_id.is_empty() => {
            Ok(WsClientEvent::Complete {
                connection_id: message.connection_id,
            })
        }
        "ready"
            if message.sdp.is_empty()
                && message.candidate.is_empty()
                && !message.connection_id.is_empty()
                && valid_request_id(&message.request_id) =>
        {
            Ok(WsClientEvent::Ready {
                connection_id: message.connection_id,
                request_id: message.request_id,
            })
        }
        "rejected"
            if message.sdp.is_empty()
                && message.candidate.is_empty()
                && !message.connection_id.is_empty()
                && valid_request_id(&message.request_id) =>
        {
            Ok(WsClientEvent::Rejected {
                connection_id: message.connection_id,
                request_id: message.request_id,
            })
        }
        "bye" if !message.connection_id.is_empty() => Ok(WsClientEvent::Bye {
            connection_id: message.connection_id,
        }),
        _ => Err(WebRtcError::Signaling(
            "WebRTC signaling message is invalid for this session".into(),
        )),
    }
}

async fn send_signaling_message<S>(write: &mut S, message: &SignalingMessage) -> Result<()>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::fmt::Debug,
{
    let payload = serde_json::to_string(message)
        .map_err(|_| WebRtcError::Signaling("WebRTC signaling serialization failed".into()))?;
    write
        .send(Message::Text(payload.into()))
        .await
        .map_err(|_| WebRtcError::Signaling("WebRTC signaling send failed".into()))
}

fn context_error_to_webrtc_error(error: WebRtcOriginateContextError) -> WebRtcError {
    WebRtcError::InvalidArgument(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::originate::{
        StaticWebRtcBearerCredentialProvider, WebRtcBearerCredential, WebRtcTargetPolicy,
    };
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    fn select_rvoip_subprotocol(
        _request: &Request,
        mut response: Response,
    ) -> std::result::Result<Response, ErrorResponse> {
        response.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static(RVOIP_WEBRTC_SUBPROTOCOL),
        );
        Ok(response)
    }

    fn loopback_context(address: std::net::SocketAddr) -> Arc<WebRtcOriginateContext> {
        let policy = WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true)
            .with_timeouts(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_millis(100),
            )
            .expect("timeouts");
        Arc::new(
            WebRtcOriginateContext::websocket(format!("ws://{address}/signal"), policy)
                .expect("context"),
        )
    }

    async fn wait_for_zero_drivers(pool: &WsClientPool) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while pool.live_driver_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("hub driver did not terminate");
    }

    #[test]
    fn rejects_unscoped_or_unknown_server_messages() {
        assert!(message_to_event(SignalingMessage {
            msg_type: "answer".into(),
            sdp: "v=0".into(),
            connection_id: String::new(),
            candidate: String::new(),
            request_id: String::new(),
        })
        .is_err());
        assert!(message_to_event(SignalingMessage {
            msg_type: "ack".into(),
            ..Default::default()
        })
        .is_err());
        for msg_type in ["ready", "rejected"] {
            assert!(message_to_event(SignalingMessage {
                msg_type: msg_type.into(),
                connection_id: "remote-connection".into(),
                ..Default::default()
            })
            .is_err());
            assert!(message_to_event(SignalingMessage {
                msg_type: msg_type.into(),
                request_id: "local-request".into(),
                ..Default::default()
            })
            .is_err());
        }
    }

    #[test]
    fn parses_exact_remote_admission_outcomes() {
        let ready = message_to_event(SignalingMessage {
            msg_type: "ready".into(),
            connection_id: "remote-connection".into(),
            request_id: "local-request".into(),
            ..Default::default()
        })
        .expect("exact ready outcome");
        assert!(matches!(
            ready,
            WsClientEvent::Ready {
                connection_id,
                request_id,
            } if connection_id == "remote-connection" && request_id == "local-request"
        ));

        let rejected = message_to_event(SignalingMessage {
            msg_type: "rejected".into(),
            connection_id: "remote-connection".into(),
            request_id: "local-request".into(),
            ..Default::default()
        })
        .expect("exact rejected outcome");
        assert!(matches!(
            rejected,
            WsClientEvent::Rejected {
                connection_id,
                request_id,
            } if connection_id == "remote-connection" && request_id == "local-request"
        ));
    }

    #[test]
    fn readiness_extension_is_explicit_and_default_off() {
        let legacy = command_to_message(WsClientCommand::Offer {
            sdp: "v=0".into(),
            request_id: "legacy-request".into(),
            require_ready: false,
        })
        .expect("legacy offer");
        assert_eq!(legacy.msg_type, "offer");

        let gated = command_to_message(WsClientCommand::Offer {
            sdp: "v=0".into(),
            request_id: "gated-request".into(),
            require_ready: true,
        })
        .expect("readiness-negotiated offer");
        assert_eq!(gated.msg_type, "offer-ready");
    }

    #[test]
    fn pool_key_separates_credential_and_network_policy_boundaries() {
        let provider: Arc<dyn crate::originate::WebRtcBearerCredentialProvider> =
            Arc::new(StaticWebRtcBearerCredentialProvider::new(
                WebRtcBearerCredential::new("secret").expect("credential"),
            ));
        let policy = WebRtcTargetPolicy::default()
            .allow_port(8080)
            .allow_insecure(true)
            .allow_loopback(true)
            .with_credential_partition("tenant-a")
            .expect("partition");
        let context = WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", policy)
            .expect("context")
            .with_bearer_provider(Arc::clone(&provider));
        assert_eq!(
            WsPoolKey::for_context(&context),
            WsPoolKey::for_context(&context.clone()),
            "cloned contexts retain their security partition"
        );

        let other_partition = WebRtcOriginateContext::websocket(
            "ws://127.0.0.1:8080/signal",
            WebRtcTargetPolicy::default()
                .allow_port(8080)
                .allow_insecure(true)
                .allow_loopback(true)
                .with_credential_partition("tenant-b")
                .expect("partition"),
        )
        .expect("context")
        .with_bearer_provider(Arc::clone(&provider));
        assert_ne!(
            WsPoolKey::for_context(&context),
            WsPoolKey::for_context(&other_partition)
        );

        let other_provider: Arc<dyn crate::originate::WebRtcBearerCredentialProvider> =
            Arc::new(StaticWebRtcBearerCredentialProvider::new(
                WebRtcBearerCredential::new("secret").expect("credential"),
            ));
        let other_identity = WebRtcOriginateContext::websocket(
            "ws://127.0.0.1:8080/signal",
            WebRtcTargetPolicy::default()
                .allow_port(8080)
                .allow_insecure(true)
                .allow_loopback(true)
                .with_credential_partition("tenant-a")
                .expect("partition"),
        )
        .expect("context")
        .with_bearer_provider(other_provider);
        assert_ne!(
            WsPoolKey::for_context(&context),
            WsPoolKey::for_context(&other_identity),
            "different credential authorities must never share a socket"
        );
    }

    #[tokio::test]
    async fn rejects_inexact_response_subprotocol() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let callback = |_request: &Request, mut response: Response| {
                response.headers_mut().insert(
                    header::SEC_WEBSOCKET_PROTOCOL,
                    HeaderValue::from_static("rvoip.webrtc.v1.variant"),
                );
                Ok(response)
            };
            let _ = accept_hdr_async(stream, callback).await;
        });
        let context = WebRtcOriginateContext::websocket(
            format!("ws://{address}/signal"),
            crate::originate::WebRtcTargetPolicy::default()
                .allow_port(address.port())
                .allow_insecure(true)
                .allow_loopback(true),
        )
        .expect("context");
        assert!(WsClientSession::connect(Arc::new(context)).await.is_err());
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn expires_a_stalled_peer_that_never_pongs() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let _socket = accept_hdr_async(stream, select_rvoip_subprotocol)
                .await
                .expect("upgrade");
            // Intentionally do not poll the socket: no automatic Pong can be
            // read, queued, or flushed by tungstenite.
            std::future::pending::<()>().await;
        });

        let pool = WsClientPool::default();
        let mut session = pool
            .open(loopback_context(address))
            .await
            .expect("open session");
        assert_eq!(pool.live_driver_count(), 1);
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), session.events.recv())
                .await
                .expect("closed timeout"),
            Some(WsClientEvent::Closed)
        ));
        wait_for_zero_drivers(&pool).await;
        drop(session.commands);
        session.task.await.expect("route proxy");
        server.abort();
    }

    #[tokio::test]
    async fn drain_closes_stalled_hubs_within_the_bound() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let _socket = accept_hdr_async(stream, select_rvoip_subprotocol)
                .await
                .expect("upgrade");
            std::future::pending::<()>().await;
        });

        let pool = WsClientPool::default();
        let mut session = pool
            .open(loopback_context(address))
            .await
            .expect("open session");
        assert!(
            pool.drain(std::time::Duration::from_millis(250)).await,
            "cooperative hub drain required an abort"
        );
        assert_eq!(pool.live_driver_count(), 0);
        assert!(matches!(
            session.events.recv().await,
            Some(WsClientEvent::Closed)
        ));
        assert!(pool.open(loopback_context(address)).await.is_err());
        drop(session.commands);
        session.task.await.expect("route proxy");
        server.abort();
    }

    #[tokio::test]
    async fn remote_disconnect_releases_the_retained_hub_driver() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let socket = accept_hdr_async(stream, select_rvoip_subprotocol)
                .await
                .expect("upgrade");
            drop(socket);
        });

        let pool = WsClientPool::default();
        let mut session = pool
            .open(loopback_context(address))
            .await
            .expect("open session");
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), session.events.recv())
                .await
                .expect("closed timeout"),
            Some(WsClientEvent::Closed)
        ));
        wait_for_zero_drivers(&pool).await;
        drop(session.commands);
        session.task.await.expect("route proxy");
        server.await.expect("server task");
    }
}
