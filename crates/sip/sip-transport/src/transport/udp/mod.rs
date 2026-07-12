mod listener;
mod sender;
mod socket;

pub use listener::UdpListener;
pub use sender::UdpSender;
pub use socket::UdpSocketOptions;

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, error, info, trace, warn};

use crate::diagnostics;
use crate::error::{Error, Result};
use crate::transport::{
    validate_typed_outbound_message, Transport, TransportEvent, TransportReceiveTiming,
    TransportType,
};
use rvoip_sip_core::Message;

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 1000;
const DEFAULT_PARSE_WORKERS: usize = 1;
const MAX_PARSE_WORKERS: usize = 64;
const UDP_RECEIVE_DRAIN_BATCH: usize = 64;

/// RFC 3261 §18.1.1 — outbound SIP requests larger than this MUST be
/// shipped over a congestion-controlled transport (TCP) rather than UDP
/// when path MTU is unknown. This is the safe default; deployments
/// with known path MTU can override via [`UdpTransport::bind_with_mtu`].
pub const UDP_SAFE_MAX_BYTES: usize = 1300;

/// UDP parse worker dispatch strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpParseDispatch {
    /// Preserve per-source ordering by consistently hashing source IP:port.
    SourceHash,
    /// Spread datagrams across workers in receive order.
    RoundRobin,
}

impl Default for UdpParseDispatch {
    fn default() -> Self {
        Self::SourceHash
    }
}

/// UDP receive-side SIP parse worker configuration.
///
/// The socket receive loop drains kernel UDP packets into a bounded worker
/// queue. More workers can help on high-CPS servers when SIP parsing or
/// transaction dispatch is measurable, while the queue capacity bounds memory
/// and makes overload explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpParseConfig {
    /// Number of parse workers fed by the UDP socket task.
    pub worker_count: usize,
    /// Per-worker datagram queue capacity.
    pub queue_capacity: usize,
    /// How the socket task selects parse workers.
    pub dispatch: UdpParseDispatch,
}

impl UdpParseConfig {
    /// Default parse worker count.
    pub const DEFAULT_WORKERS: usize = DEFAULT_PARSE_WORKERS;
    /// Maximum parse worker count accepted by the UDP transport.
    pub const MAX_WORKERS: usize = MAX_PARSE_WORKERS;

    /// Create a parse config, clamping invalid values to safe bounds.
    pub fn new(worker_count: usize, queue_capacity: usize) -> Self {
        Self::new_with_dispatch(worker_count, queue_capacity, UdpParseDispatch::default())
    }

    /// Create a parse config with an explicit dispatch mode, clamping invalid
    /// values to safe bounds.
    pub fn new_with_dispatch(
        worker_count: usize,
        queue_capacity: usize,
        dispatch: UdpParseDispatch,
    ) -> Self {
        Self {
            worker_count: worker_count.clamp(1, MAX_PARSE_WORKERS),
            queue_capacity: queue_capacity.max(1),
            dispatch,
        }
    }

    /// Return this config with a different dispatch mode.
    pub fn with_dispatch(mut self, dispatch: UdpParseDispatch) -> Self {
        self.dispatch = dispatch;
        self
    }

    /// Build a parse config only when at least one optional override is set.
    pub fn from_optional(
        worker_count: Option<usize>,
        queue_capacity: Option<usize>,
        default_queue_capacity: usize,
    ) -> Option<Self> {
        Self::from_optional_with_dispatch(
            worker_count,
            queue_capacity,
            None,
            default_queue_capacity,
        )
    }

    /// Build a parse config only when at least one optional override is set.
    pub fn from_optional_with_dispatch(
        worker_count: Option<usize>,
        queue_capacity: Option<usize>,
        dispatch: Option<UdpParseDispatch>,
        default_queue_capacity: usize,
    ) -> Option<Self> {
        if worker_count.is_none() && queue_capacity.is_none() && dispatch.is_none() {
            return None;
        }

        Some(Self::new_with_dispatch(
            worker_count.unwrap_or(DEFAULT_PARSE_WORKERS),
            queue_capacity.unwrap_or(default_queue_capacity),
            dispatch.unwrap_or_default(),
        ))
    }

    fn effective(config: Option<Self>, default_queue_capacity: usize) -> Self {
        config.unwrap_or_else(|| Self::new(DEFAULT_PARSE_WORKERS, default_queue_capacity))
    }
}

/// UDP transport for SIP messages
#[derive(Clone)]
pub struct UdpTransport {
    inner: Arc<UdpTransportInner>,
}

struct UdpTransportInner {
    sender: UdpSender,
    listener: Arc<UdpListener>,
    closed: AtomicBool,
    events_tx: mpsc::Sender<TransportEvent>,
    receive_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    parse_tasks: tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    /// Per-instance MTU threshold for the RFC 3261 §18.1.1 UDP→TCP
    /// failover. Defaults to [`UDP_SAFE_MAX_BYTES`]; configurable so
    /// deployments with a known smaller path MTU (e.g. tunnelled SIP
    /// over IPSec) or a known larger one (e.g. controlled DC fabric)
    /// can tune the threshold.
    safe_max_bytes: usize,
    socket_options: UdpSocketOptions,
    parse_worker_count: usize,
    parse_worker_queue_capacity: usize,
    parse_dispatch: UdpParseDispatch,
}

#[derive(Debug)]
struct UdpDatagram {
    packet: Bytes,
    source: SocketAddr,
    local_addr: SocketAddr,
    timing: Option<TransportReceiveTiming>,
}

impl UdpTransport {
    /// Creates a new UDP transport bound to the specified address.
    /// Uses the RFC 3261 §18.1.1 default MTU threshold
    /// ([`UDP_SAFE_MAX_BYTES`]).
    pub async fn bind(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_mtu(addr, channel_capacity, UDP_SAFE_MAX_BYTES).await
    }

    /// Creates a new UDP transport with explicit socket options.
    pub async fn bind_with_socket_options(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        socket_options: UdpSocketOptions,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_mtu_and_socket_options(
            addr,
            channel_capacity,
            UDP_SAFE_MAX_BYTES,
            socket_options,
        )
        .await
    }

    /// Same as [`Self::bind`] but with a caller-supplied MTU threshold
    /// for the UDP→TCP failover decision (RFC 3261 §18.1.1). Useful
    /// for deployments with a known smaller path MTU (e.g. SIP over
    /// IPSec) or a known-safe larger one (DC fabric).
    pub async fn bind_with_mtu(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        safe_max_bytes: usize,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_mtu_and_socket_options(
            addr,
            channel_capacity,
            safe_max_bytes,
            UdpSocketOptions::default(),
        )
        .await
    }

    /// Same as [`Self::bind_with_mtu`] but with explicit UDP socket options.
    pub async fn bind_with_mtu_and_socket_options(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        safe_max_bytes: usize,
        socket_options: UdpSocketOptions,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_mtu_socket_options_and_parse_config(
            addr,
            channel_capacity,
            safe_max_bytes,
            socket_options,
            None,
        )
        .await
    }

    /// Same as [`Self::bind_with_mtu_and_socket_options`] but with explicit
    /// UDP parse worker configuration.
    pub async fn bind_with_mtu_socket_options_and_parse_config(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        safe_max_bytes: usize,
        socket_options: UdpSocketOptions,
        parse_config: Option<UdpParseConfig>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);
        let parse_config = UdpParseConfig::effective(parse_config, capacity);
        let parse_worker_count = parse_config.worker_count;
        let parse_worker_queue_capacity = parse_config.queue_capacity;
        let parse_dispatch = parse_config.dispatch;

        // Create the UDP listener
        let listener = UdpListener::bind_with_socket_options(addr, socket_options).await?;
        let local_addr = listener.local_addr()?;
        info!(
            "SIP UDP transport bound to {} (MTU threshold {} bytes, socket options {:?})",
            local_addr, safe_max_bytes, socket_options
        );

        // Create the UDP sender (shares same socket)
        let sender = UdpSender::new(listener.clone_socket())?;

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Create the transport
        let transport = UdpTransport {
            inner: Arc::new(UdpTransportInner {
                sender,
                listener: Arc::new(listener),
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
                receive_task: tokio::sync::Mutex::new(None),
                parse_tasks: tokio::sync::Mutex::new(Vec::new()),
                shutdown_tx,
                shutdown_rx,
                safe_max_bytes,
                socket_options,
                parse_worker_count,
                parse_worker_queue_capacity,
                parse_dispatch,
            }),
        };

        // Start the receive loop
        transport.spawn_receive_loop().await;

        Ok((transport, events_rx))
    }

    /// Create a default dummy UDP transport (used only for creating dummy transaction managers)
    /// This transport doesn't work for real communication
    #[cfg(test)]
    pub fn default() -> Self {
        // Create a dummy event channel
        let (events_tx, _) = mpsc::channel(1);

        // Create a dummy listener and sender
        let listener = UdpListener::default();
        let sender = UdpSender::default();

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Create and return the transport with closed=true so it won't be used
        UdpTransport {
            inner: Arc::new(UdpTransportInner {
                sender,
                listener: Arc::new(listener),
                closed: AtomicBool::new(true), // Mark as closed
                events_tx,
                receive_task: tokio::sync::Mutex::new(None),
                parse_tasks: tokio::sync::Mutex::new(Vec::new()),
                shutdown_tx,
                shutdown_rx,
                safe_max_bytes: UDP_SAFE_MAX_BYTES,
                socket_options: UdpSocketOptions::default(),
                parse_worker_count: 1,
                parse_worker_queue_capacity: DEFAULT_CHANNEL_CAPACITY,
                parse_dispatch: UdpParseDispatch::SourceHash,
            }),
        }
    }

    /// Returns the socket options requested at bind time.
    pub fn socket_options(&self) -> UdpSocketOptions {
        self.inner.socket_options
    }

    /// Returns the parse worker configuration requested at bind time.
    pub fn parse_config(&self) -> UdpParseConfig {
        UdpParseConfig::new_with_dispatch(
            self.inner.parse_worker_count,
            self.inner.parse_worker_queue_capacity,
            self.inner.parse_dispatch,
        )
    }

    // Spawns a task to receive packets from the UDP socket
    async fn spawn_receive_loop(&self) {
        let worker_count = self.inner.parse_worker_count;
        let queue_capacity = self.inner.parse_worker_queue_capacity;
        let dispatch = self.inner.parse_dispatch;
        let round_robin_worker = Arc::new(AtomicUsize::new(0));
        let mut worker_senders = Vec::with_capacity(worker_count);
        let mut worker_handles = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let (tx, rx) = mpsc::channel(queue_capacity);
            worker_senders.push(tx);

            let events_tx = self.inner.events_tx.clone();
            let shutdown_rx = self.inner.shutdown_rx.clone();
            worker_handles.push(tokio::spawn(async move {
                udp_parse_worker(worker_id, rx, events_tx, shutdown_rx).await;
            }));
        }

        {
            let mut parse_task_guard = self.inner.parse_tasks.lock().await;
            *parse_task_guard = worker_handles;
        }

        let mut shutdown_rx = self.inner.shutdown_rx.clone();
        let listener_clone = self.inner.listener.clone();
        let events_tx = self.inner.events_tx.clone();
        let round_robin_worker = Arc::clone(&round_robin_worker);

        let handle = tokio::spawn(async move {
            let mut last_receive_completed_at: Option<Instant> = None;
            loop {
                let receive_poll_started = diagnostics::enabled().then(Instant::now);
                // Use select to listen for both packets and shutdown signal
                tokio::select! {
                    // Watch for shutdown signal
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            debug!("UDP receive loop received shutdown signal");
                            break;
                        }
                    }

                    // Receive packets
                    result = listener_clone.receive() => {

                        match result {
                            Ok((packet, src, local_addr)) => {
                                let receive_completed_at = Instant::now();
                                if let Some(started) = receive_poll_started {
                                    diagnostics::record_udp_receive_poll(
                                        receive_completed_at.duration_since(started),
                                    );
                                }
                                if !enqueue_udp_datagram(
                                    packet,
                                    src,
                                    local_addr,
                                    &worker_senders,
                                    dispatch,
                                    &round_robin_worker,
                                    receive_completed_at,
                                    &mut last_receive_completed_at,
                                ) {
                                    break;
                                }

                                let mut keep_running = true;
                                for _ in 0..UDP_RECEIVE_DRAIN_BATCH {
                                    match listener_clone.try_receive() {
                                        Ok(Some((packet, src, local_addr))) => {
                                            let receive_completed_at = Instant::now();
                                            if !enqueue_udp_datagram(
                                                packet,
                                                src,
                                                local_addr,
                                                &worker_senders,
                                                dispatch,
                                                &round_robin_worker,
                                                receive_completed_at,
                                                &mut last_receive_completed_at,
                                            ) {
                                                keep_running = false;
                                                break;
                                            }
                                        }
                                        Ok(None) => break,
                                        Err(e) => {
                                            error!("Error receiving UDP packet: {}", e);
                                            let _ = events_tx.try_send(TransportEvent::Error {
                                                error: format!("Error receiving packet: {}", e),
                                            });
                                            break;
                                        }
                                    }
                                }
                                if !keep_running {
                                    break;
                                }
                            },
                            Err(e) => {
                                error!("Error receiving UDP packet: {}", e);
                                let _ = events_tx.try_send(TransportEvent::Error {
                                    error: format!("Error receiving packet: {}", e),
                                });
                            }
                        }
                    }
                }
            }

            // Send closed event when the loop exits
            let _ = events_tx.try_send(TransportEvent::Closed);
            info!("UDP receive loop terminated");
        });

        // Store the task handle
        let mut task_guard = self.inner.receive_task.lock().await;
        *task_guard = Some(handle);
    }
}

fn enqueue_udp_datagram(
    packet: Bytes,
    src: SocketAddr,
    local_addr: SocketAddr,
    worker_senders: &[mpsc::Sender<UdpDatagram>],
    dispatch: UdpParseDispatch,
    round_robin_worker: &AtomicUsize,
    receive_completed_at: Instant,
    last_receive_completed_at: &mut Option<Instant>,
) -> bool {
    if let Some(previous) = *last_receive_completed_at {
        diagnostics::record_udp_receive_loop_gap(
            local_addr,
            receive_completed_at.duration_since(previous),
        );
    }
    *last_receive_completed_at = Some(receive_completed_at);
    diagnostics::record_udp_datagram_received();
    trace!("Received UDP datagram from {}", src);
    let received_at = diagnostics::enabled().then_some(receive_completed_at);
    let worker_index = udp_worker_index(src, worker_senders.len(), dispatch, round_robin_worker);
    let datagram = UdpDatagram {
        packet,
        source: src,
        local_addr,
        timing: received_at.map(|received_at| TransportReceiveTiming {
            received_at: Some(received_at),
            ..Default::default()
        }),
    };
    match worker_senders[worker_index].try_send(datagram) {
        Ok(()) => diagnostics::record_udp_worker_queue_enqueued(),
        Err(TrySendError::Full(_)) => {
            diagnostics::record_udp_worker_queue_full();
            if diagnostics::enabled() {
                warn!(
                    worker_index,
                    "UDP parse worker queue full; dropping datagram"
                );
            }
        }
        Err(TrySendError::Closed(_)) => {
            debug!("UDP parse worker queue closed");
            return false;
        }
    }
    true
}

async fn udp_parse_worker(
    worker_id: usize,
    mut rx: mpsc::Receiver<UdpDatagram>,
    events_tx: mpsc::Sender<TransportEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    debug!(worker_id, "UDP parse worker received shutdown signal");
                    break;
                }
            }
            maybe_datagram = rx.recv() => {
                let Some(datagram) = maybe_datagram else {
                    break;
                };
                process_udp_datagram(worker_id, datagram, &events_tx).await;
            }
        }
    }
    debug!(worker_id, "UDP parse worker terminated");
}

async fn process_udp_datagram(
    worker_id: usize,
    mut datagram: UdpDatagram,
    events_tx: &mpsc::Sender<TransportEvent>,
) {
    debug!("Received SIP message from {}", datagram.source);
    if let Some(timing) = datagram.timing.as_mut() {
        let now = Instant::now();
        if let Some(received_at) = timing.received_at {
            diagnostics::record_udp_read_to_worker_queue(now.duration_since(received_at));
        }
        timing.parse_worker_dequeued_at = Some(now);
    }

    let parse_started = datagram.timing.as_ref().map(|_| Instant::now());
    let parsed = rvoip_sip_core::parse_message(&datagram.packet);
    if let (Some(started), Some(timing)) = (parse_started, datagram.timing.as_mut()) {
        let now = Instant::now();
        diagnostics::record_udp_parse(now.duration_since(started));
        timing.parse_completed_at = Some(now);
    }

    let event = match parsed {
        Ok(message) => {
            diagnostics::record_udp_parse_ok();
            diagnostics::record_inbound_message(&message, datagram.source, datagram.local_addr);
            TransportEvent::MessageReceived {
                message,
                source: datagram.source,
                destination: datagram.local_addr,
                transport_type: TransportType::Udp,
                raw_bytes: Some(datagram.packet),
                timing: datagram.timing,
                connection_metadata: None,
            }
        }
        Err(e) => {
            diagnostics::record_udp_parse_failed();
            warn!("Error parsing SIP message: {}", e);
            TransportEvent::Error {
                error: format!("Error parsing SIP message: {}", e),
            }
        }
    };

    match events_tx.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(event)) => {
            let started = Instant::now();
            if let Err(e) = events_tx.send(event).await {
                error!(worker_id, "Error sending UDP transport event: {}", e);
                return;
            }
            diagnostics::record_transport_channel_backpressure(started.elapsed());
        }
        Err(TrySendError::Closed(_)) => {
            debug!(worker_id, "UDP transport event channel closed");
        }
    }
}

fn udp_worker_index(
    source: SocketAddr,
    worker_count: usize,
    dispatch: UdpParseDispatch,
    round_robin_worker: &AtomicUsize,
) -> usize {
    if worker_count <= 1 {
        return 0;
    }
    if dispatch == UdpParseDispatch::RoundRobin {
        return round_robin_worker.fetch_add(1, Ordering::Relaxed) % worker_count;
    }
    let ip_hash = match source.ip() {
        std::net::IpAddr::V4(ip) => u32::from(ip) as usize,
        std::net::IpAddr::V6(ip) => {
            let segments = ip.segments();
            segments.iter().fold(0usize, |acc, segment| {
                acc.wrapping_mul(31) ^ usize::from(*segment)
            })
        }
    };
    (ip_hash ^ usize::from(source.port())) % worker_count
}

#[async_trait::async_trait]
impl Transport for UdpTransport {
    fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.listener.local_addr()
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        validate_typed_outbound_message(&message)?;

        // Convert message to bytes
        let bytes = message.to_bytes();

        debug!("Sending {} byte message to {}", bytes.len(), destination);
        info!(
            "Sending {} message to {}",
            if let Message::Request(ref req) = message {
                format!("{}", req.method)
            } else {
                "response".to_string()
            },
            destination
        );

        // Send the message using the sender
        let started = Instant::now();
        let result = self.inner.sender.send(&bytes, destination).await;
        let local_addr = self.inner.listener.local_addr().unwrap_or_else(|_| {
            "0.0.0.0:0"
                .parse()
                .expect("hardcoded socket address must parse")
        });
        diagnostics::record_outbound_message(
            &message,
            local_addr,
            destination,
            started.elapsed(),
            result.is_err(),
        );
        result
    }

    async fn send_message_raw(&self, bytes: bytes::Bytes, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        debug!(
            "UDP: sending {} pre-built bytes to {}",
            bytes.len(),
            destination
        );
        let started = Instant::now();
        let result = self.inner.sender.send(&bytes, destination).await;
        let local_addr = self.inner.listener.local_addr().unwrap_or_else(|_| {
            "0.0.0.0:0"
                .parse()
                .expect("hardcoded socket address must parse")
        });
        diagnostics::record_outbound_raw(
            bytes.as_ref(),
            local_addr,
            destination,
            started.elapsed(),
            result.is_err(),
        );
        result
    }

    async fn close(&self) -> Result<()> {
        debug!("UDP transport closing...");

        // Step 1: Signal shutdown to receive loop via watch channel
        let _ = self.inner.shutdown_tx.send(true);
        self.inner.closed.store(true, Ordering::Relaxed);

        // Step 2: Take the receive task handle and wait for it to finish
        let mut task_guard = self.inner.receive_task.lock().await;
        if let Some(handle) = task_guard.take() {
            debug!("Waiting for UDP receive loop to terminate...");
            // Wait for the task to finish (with timeout to prevent hanging)
            match tokio::time::timeout(std::time::Duration::from_secs(2), handle).await {
                Ok(Ok(())) => {
                    debug!("UDP receive loop terminated cleanly");
                }
                Ok(Err(e)) => {
                    debug!("UDP receive loop task error: {}", e);
                }
                Err(_) => {
                    warn!("UDP receive loop termination timed out after 2 seconds");
                }
            }
        }
        drop(task_guard);

        let mut parse_task_guard = self.inner.parse_tasks.lock().await;
        for handle in parse_task_guard.drain(..) {
            match tokio::time::timeout(std::time::Duration::from_secs(2), handle).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => debug!("UDP parse worker task error: {}", e),
                Err(_) => warn!("UDP parse worker termination timed out after 2 seconds"),
            }
        }
        drop(parse_task_guard);

        // Step 3: Send a final closed event to notify upper layers
        // But check if the channel is still open
        let _ = self.inner.events_tx.try_send(TransportEvent::Closed);

        info!("UDP transport closed successfully");
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Relaxed)
    }

    fn max_safe_message_size(&self) -> usize {
        self.inner.safe_max_bytes
    }
}

impl fmt::Debug for UdpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(addr) = self.inner.listener.local_addr() {
            write!(f, "UdpTransport({})", addr)
        } else {
            write!(f, "UdpTransport(<e>)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
    use rvoip_sip_core::{Method, Response, StatusCode};

    #[tokio::test]
    async fn bind_uses_default_mtu_threshold() {
        let (transport, _rx) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None)
            .await
            .expect("bind");
        assert_eq!(transport.max_safe_message_size(), UDP_SAFE_MAX_BYTES);
        transport.close().await.ok();
    }

    #[tokio::test]
    async fn bind_with_mtu_honours_explicit_override() {
        let (transport, _rx) =
            UdpTransport::bind_with_mtu("127.0.0.1:0".parse().unwrap(), None, 900)
                .await
                .expect("bind_with_mtu");
        assert_eq!(transport.max_safe_message_size(), 900);
        transport.close().await.ok();
    }

    #[tokio::test]
    async fn bind_with_socket_options_preserves_requested_values() {
        let options = UdpSocketOptions::new(Some(4096), Some(4096));
        let (transport, _rx) =
            UdpTransport::bind_with_socket_options("127.0.0.1:0".parse().unwrap(), None, options)
                .await
                .expect("bind_with_socket_options");
        assert_eq!(transport.socket_options(), options);
        transport.close().await.ok();
    }

    #[tokio::test]
    async fn bind_with_parse_config_preserves_requested_values() {
        let parse_config = UdpParseConfig::new(4, 2048).with_dispatch(UdpParseDispatch::RoundRobin);
        let (transport, _rx) = UdpTransport::bind_with_mtu_socket_options_and_parse_config(
            "127.0.0.1:0".parse().unwrap(),
            None,
            UDP_SAFE_MAX_BYTES,
            UdpSocketOptions::default(),
            Some(parse_config),
        )
        .await
        .expect("bind_with_parse_config");
        assert_eq!(transport.parse_config(), parse_config);
        transport.close().await.ok();
    }

    #[test]
    fn round_robin_worker_index_cycles_across_workers() {
        let round_robin = AtomicUsize::new(0);
        let source = "127.0.0.1:5060".parse().unwrap();

        let observed: Vec<usize> = (0..8)
            .map(|_| udp_worker_index(source, 4, UdpParseDispatch::RoundRobin, &round_robin))
            .collect();

        assert_eq!(observed, vec![0, 1, 2, 3, 0, 1, 2, 3]);
    }

    #[test]
    fn source_hash_worker_index_is_stable_for_same_source() {
        let round_robin = AtomicUsize::new(0);
        let source = "127.0.0.1:5060".parse().unwrap();

        let first = udp_worker_index(source, 4, UdpParseDispatch::SourceHash, &round_robin);
        for _ in 0..8 {
            assert_eq!(
                udp_worker_index(source, 4, UdpParseDispatch::SourceHash, &round_robin),
                first
            );
        }
    }

    #[tokio::test]
    async fn typed_send_rejects_unsafe_fields_but_raw_send_remains_verbatim() {
        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture bind");
        let destination = capture.local_addr().expect("capture address");
        let (transport, _events) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None)
            .await
            .expect("transport bind");
        let malicious = |name| {
            TypedHeader::Other(
                name,
                HeaderValue::Raw(b"Bearer safe\r\nX-Injected: udp".to_vec()),
            )
        };

        let mut request = SimpleRequestBuilder::new(Method::Options, "sip:example.com")
            .unwrap()
            .build();
        request.headers.push(malicious(HeaderName::Authorization));
        let mut response = Response::new(StatusCode::Ok);
        response
            .headers
            .push(malicious(HeaderName::Other("PROXY-authorization".into())));
        let mut malformed_name = SimpleRequestBuilder::new(Method::Options, "sip:example.com")
            .unwrap()
            .build();
        malformed_name.headers.push(TypedHeader::Other(
            HeaderName::Other("X-Context: injected".into()),
            HeaderValue::Raw(b"udp-name-secret".to_vec()),
        ));
        let invalid_reason =
            Response::new(StatusCode::Ok).with_reason("OK\r\nX-Injected: udp-reason-secret");

        for message in [
            Message::Request(request),
            Message::Response(response),
            Message::Request(malformed_name),
            Message::Response(invalid_reason),
        ] {
            let error = transport
                .send_message(message, destination)
                .await
                .expect_err("typed UDP send must reject unsafe fields");
            assert!(matches!(error, Error::ProtocolError(_)));
            assert!(!error.to_string().contains("X-Injected"));
        }
        let mut buffer = [0u8; 256];
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                capture.recv_from(&mut buffer),
            )
            .await
            .is_err(),
            "rejected typed messages must emit no datagram",
        );

        let raw = Bytes::from_static(b"Authorization: raw\r\nX-Verbatim: retained\r\n");
        transport
            .send_message_raw(raw.clone(), destination)
            .await
            .expect("explicit raw send remains available");
        let (received, _) = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            capture.recv_from(&mut buffer),
        )
        .await
        .expect("raw datagram timeout")
        .expect("raw datagram receive");
        assert_eq!(&buffer[..received], raw.as_ref());
        transport.close().await.ok();
    }
}
