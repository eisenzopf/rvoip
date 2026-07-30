//! RTP Session Management
//!
//! This module provides functionality for managing RTP sessions, including
//! configuration, packet sending/receiving, and jitter buffer management.

mod scheduling;
mod stream;

pub use scheduling::{RtpScheduler, RtpSchedulerStats};
pub use stream::{RtpStream, RtpStreamStats};

use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use rand::Rng;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc, Mutex, Semaphore};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace, warn};

use crate::error::Error;
use crate::packet::{RtpHeader, RtpPacket};
use crate::transport::{
    RtpTransport, RtpTransportBufferConfig, RtpTransportConfig, SymmetricRtpPolicy, UdpRtpTransport,
};
use crate::{Result, RtpSsrc, RtpTimestamp};

#[cfg(feature = "memory-diagnostics")]
fn spawn_memory_tracked<F>(kind: &'static str, future: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let guard = rvoip_infra_common::memory_diagnostics::ObjectGuard::new(kind, 0);
    crate::task_runtime::spawn_media_task(async move {
        let _guard = guard;
        future.await
    })
}

#[cfg(not(feature = "memory-diagnostics"))]
fn spawn_memory_tracked<F>(_: &'static str, future: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    crate::task_runtime::spawn_media_task(future)
}

/// Bounded queue depth for per-session RTP send/event channels.
///
/// RTP is real-time traffic; keeping many seconds of packet backlog per call
/// hides overload and retains packet payloads. At 20 ms packets, 64 entries is
/// roughly 1.3 seconds of headroom for one stream.
pub const RTP_SESSION_CHANNEL_CAPACITY: usize = 64;

/// Small best-effort queue for the legacy polling receive API.
///
/// Media-core consumes RTP packets through the event broadcast path, so this
/// queue must not become an unbounded duplicate packet buffer when nobody calls
/// [`RtpSession::receive_packet`].
pub const RTP_SESSION_RECEIVE_QUEUE_CAPACITY: usize = 32;

fn take_rtcp_report_blocks(
    streams: &DashMap<RtpSsrc, RtpStream>,
) -> Vec<crate::packet::rtcp::RtcpReportBlock> {
    streams
        .iter_mut()
        .take(31)
        .map(|mut stream| stream.take_report_block())
        .collect()
}

fn sender_report_totals(
    stats: &parking_lot::Mutex<RtpSessionStats>,
    sender_octets: &AtomicU64,
) -> (u32, u32) {
    // Packet sends update both counters while holding this lock. Keep the
    // guard while loading the atomic octet counter so an RTCP report cannot
    // combine the packet total from one send with the octet total from the
    // next one.
    let stats = stats.lock();
    (
        stats.packets_sent as u32,
        sender_octets.load(Ordering::Relaxed) as u32,
    )
}

#[derive(Debug, Clone, Copy)]
struct ReceivedSenderReport {
    lsr: u32,
    received_at: Instant,
}

/// RTP session queue sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpSessionBufferConfig {
    /// Bounded sender queue capacity in RTP packets.
    pub sender_channel_capacity: usize,
    /// Bounded legacy polling receive queue capacity in RTP packets.
    pub receiver_channel_capacity: usize,
    /// Broadcast ring capacity for RTP session events.
    pub event_channel_capacity: usize,
}

impl Default for RtpSessionBufferConfig {
    fn default() -> Self {
        Self {
            sender_channel_capacity: RTP_SESSION_CHANNEL_CAPACITY,
            receiver_channel_capacity: RTP_SESSION_RECEIVE_QUEUE_CAPACITY,
            event_channel_capacity: RTP_SESSION_CHANNEL_CAPACITY,
        }
    }
}

/// Stats for an RTP session
#[derive(Debug, Clone, Default)]
pub struct RtpSessionStats {
    /// Total packets sent
    pub packets_sent: u64,

    /// Total packets received
    pub packets_received: u64,

    /// Total bytes sent
    pub bytes_sent: u64,

    /// Total bytes received
    pub bytes_received: u64,

    /// Packets lost (based on sequence numbers)
    pub packets_lost: u64,

    /// Duplicate packets received
    pub packets_duplicated: u64,

    /// Out-of-order packets received
    pub packets_out_of_order: u64,

    /// Packets discarded by jitter buffer (too old)
    pub packets_discarded_by_jitter: u64,

    /// Current jitter estimate (in milliseconds)
    pub jitter_ms: f64,

    /// Remote address of the most recent packet
    pub remote_addr: Option<SocketAddr>,
}

/// Snapshot of bounded queue occupancy inside an RTP session.
#[derive(Debug, Clone, Copy, Default)]
pub struct RtpSessionQueueDiagnostics {
    /// Packets waiting to be sent by the RTP send task.
    pub sender_queue_packets: usize,
    /// Configured sender queue capacity.
    pub sender_capacity_packets: usize,
    /// Packets waiting in the receive queue for explicit `receive_packet` users.
    pub receiver_queue_packets: usize,
    /// Configured receiver queue capacity.
    pub receiver_capacity_packets: usize,
    /// Events retained in the broadcast ring.
    pub event_queue_events: usize,
    /// Current subscribers to the event broadcast ring.
    pub event_receiver_count: usize,
    #[cfg(feature = "memory-diagnostics")]
    /// Current SSRC stream entries retained by this session.
    pub stream_count: usize,
}

/// RTP session configuration options
#[derive(Debug, Clone)]
pub struct RtpSessionConfig {
    /// Local address to bind to
    pub local_addr: SocketAddr,

    /// Remote address to send packets to
    pub remote_addr: Option<SocketAddr>,

    /// SSRC to use for sending packets
    pub ssrc: Option<RtpSsrc>,

    /// Payload type
    pub payload_type: u8,

    /// Clock rate for the payload type (needed for jitter buffer)
    pub clock_rate: u32,

    /// Jitter buffer size in packets
    pub jitter_buffer_size: Option<usize>,

    /// Maximum packet age in the jitter buffer (ms)
    pub max_packet_age_ms: Option<u32>,

    /// Enable jitter buffer
    pub enable_jitter_buffer: bool,

    /// RTP session queue and reusable send-buffer sizing.
    pub session_buffer_config: RtpSessionBufferConfig,

    /// UDP transport buffer sizing used when the session creates its transport.
    pub transport_buffer_config: RtpTransportBufferConfig,
}

impl Default for RtpSessionConfig {
    fn default() -> Self {
        Self {
            local_addr: "0.0.0.0:0".parse().unwrap(),
            remote_addr: None,
            ssrc: None,
            payload_type: 0,
            clock_rate: 8000, // Default for most audio codecs (8kHz)
            jitter_buffer_size: Some(50),
            max_packet_age_ms: Some(200),
            enable_jitter_buffer: true,
            session_buffer_config: RtpSessionBufferConfig::default(),
            transport_buffer_config: RtpTransportBufferConfig::default(),
        }
    }
}

struct RtpPacketSender {
    transport: Arc<dyn RtpTransport>,
    remote_addr: parking_lot::RwLock<Option<SocketAddr>>,
    ssrc: RtpSsrc,
    sequence: Arc<AtomicU16>,
    stats: Arc<parking_lot::Mutex<RtpSessionStats>>,
    sender_octets: Arc<AtomicU64>,
    event_tx: broadcast::Sender<RtpSessionEvent>,
    state: Mutex<RtpPacketSenderState>,
    slots: Arc<Semaphore>,
    capacity: usize,
    closed: AtomicBool,
}

struct RtpPacketSenderState {
    send_buffer: BytesMut,
}

impl RtpPacketSender {
    fn new(
        transport: Arc<dyn RtpTransport>,
        remote_addr: Option<SocketAddr>,
        ssrc: RtpSsrc,
        sequence: Arc<AtomicU16>,
        stats: Arc<parking_lot::Mutex<RtpSessionStats>>,
        sender_octets: Arc<AtomicU64>,
        event_tx: broadcast::Sender<RtpSessionEvent>,
        capacity: usize,
    ) -> Self {
        let capacity = capacity.max(1);
        Self {
            transport,
            remote_addr: parking_lot::RwLock::new(remote_addr),
            ssrc,
            sequence,
            stats,
            sender_octets,
            event_tx,
            state: Mutex::new(RtpPacketSenderState {
                send_buffer: BytesMut::with_capacity(crate::DEFAULT_MAX_PACKET_SIZE),
            }),
            slots: Arc::new(Semaphore::new(capacity)),
            capacity,
            closed: AtomicBool::new(false),
        }
    }

    async fn send_payload(
        &self,
        timestamp: RtpTimestamp,
        payload: Bytes,
        marker: bool,
        payload_type: u8,
    ) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::SessionError("RTP session is closed".to_string()));
        }

        // Preserve the configured per-session backpressure bound without a
        // second per-session queue and forwarding task. The state lock below
        // is also the exact wire-order authority for sequence assignment.
        let _slot = self
            .slots
            .acquire()
            .await
            .map_err(|_| Error::SessionError("RTP session is closed".to_string()))?;
        let mut state = self.state.lock().await;

        if self.closed.load(Ordering::Acquire) {
            return Err(Error::SessionError("RTP session is closed".to_string()));
        }

        let destination = if let Some(udp) =
            self.transport.as_any().downcast_ref::<UdpRtpTransport>()
        {
            udp.remote_rtp_addr()
                .await
                .or_else(|| *self.remote_addr.read())
        } else {
            *self.remote_addr.read()
        }
        .ok_or_else(|| Error::SessionError("No destination address for RTP packet".to_string()))?;

        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let mut header = RtpHeader::new(payload_type, sequence, timestamp, self.ssrc);
        header.marker = marker;
        let packet = RtpPacket::new(header, payload);

        debug!(
            "Sending RTP packet to {} (seq={}, timestamp={})",
            destination, packet.header.sequence_number, packet.header.timestamp
        );

        let send_result =
            if let Some(udp) = self.transport.as_any().downcast_ref::<UdpRtpTransport>() {
                udp.send_rtp_with_buffer(&packet, destination, &mut state.send_buffer)
                    .await
            } else {
                self.transport.send_rtp(&packet, destination).await
            };

        match send_result {
            Ok(()) => {
                let mut stats = self.stats.lock();
                stats.packets_sent += 1;
                stats.bytes_sent += packet.size() as u64;
                self.sender_octets
                    .fetch_add(packet.payload.len() as u64, Ordering::Relaxed);
                Ok(())
            }
            Err(err) => {
                error!("Failed to send RTP packet: {}", err);
                let _ = self.event_tx.send(RtpSessionEvent::Error(err.clone()));
                Err(err)
            }
        }
    }

    fn set_remote_addr(&self, addr: SocketAddr) {
        *self.remote_addr.write() = Some(addr);
    }

    fn queue_diagnostics(&self) -> (usize, usize) {
        (
            self.capacity.saturating_sub(self.slots.available_permits()),
            self.capacity,
        )
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.slots.close();
    }
}

/// Handle for sending RTP packets through an existing
/// [`RtpSession`] without touching the outer `Arc<Mutex<RtpSession>>`.
///
/// Cheap to clone. Issued by
/// [`RtpSession::send_handle`]; multiple handles for the same session
/// use the same ordered transport writer and sequence cursor.
#[derive(Clone)]
pub struct RtpSendHandle {
    packet_sender: Arc<RtpPacketSender>,
    default_payload_type: u8,
}

impl RtpSendHandle {
    /// Send an RTP packet with the session's default payload type.
    pub async fn send_packet(
        &self,
        timestamp: RtpTimestamp,
        payload: Bytes,
        marker: bool,
    ) -> Result<()> {
        self.send_packet_with_pt(timestamp, payload, marker, self.default_payload_type)
            .await
    }

    /// Send an RTP packet overriding the configured payload type
    /// (e.g. RFC 4733 telephone-event PT 101).
    pub async fn send_packet_with_pt(
        &self,
        timestamp: RtpTimestamp,
        payload: Bytes,
        marker: bool,
        payload_type: u8,
    ) -> Result<()> {
        self.packet_sender
            .send_payload(timestamp, payload, marker, payload_type)
            .await
    }

    /// Get the session's SSRC (immutable post-construction).
    pub fn ssrc(&self) -> RtpSsrc {
        self.packet_sender.ssrc
    }
}

/// Events emitted by the RTP session
#[derive(Debug, Clone)]
pub enum RtpSessionEvent {
    /// New packet received
    PacketReceived(RtpPacket),

    /// Error in the session
    Error(Error),

    /// BYE RTCP packet received (a party is leaving the session)
    Bye {
        /// SSRC of the source that sent the BYE
        ssrc: RtpSsrc,

        /// Optional reason text
        reason: Option<String>,
    },

    /// New stream detected with a specific SSRC
    /// This event is emitted as soon as the first packet for a new SSRC is received,
    /// even if the packet is being held in a jitter buffer.
    NewStreamDetected {
        /// SSRC of the new stream
        ssrc: RtpSsrc,
    },

    /// RTCP Sender Report received
    RtcpSenderReport {
        /// SSRC of the sender
        ssrc: RtpSsrc,

        /// NTP timestamp
        ntp_timestamp: crate::packet::rtcp::NtpTimestamp,

        /// RTP timestamp
        rtp_timestamp: RtpTimestamp,

        /// Packet count
        packet_count: u32,

        /// Octet count
        octet_count: u32,

        /// Report blocks
        report_blocks: Vec<crate::packet::rtcp::RtcpReportBlock>,
    },

    /// RTCP Receiver Report received
    RtcpReceiverReport {
        /// SSRC of the receiver
        ssrc: RtpSsrc,

        /// Report blocks
        report_blocks: Vec<crate::packet::rtcp::RtcpReportBlock>,
    },

    /// RFC 4733 telephone-event (DTMF / fax / modem tone) received.
    /// Forwarded verbatim from the transport-level `RtpEvent::DtmfEvent`.
    /// Consumers should forward the digit up to the application only on
    /// the frame where `end_of_event == true` — RFC 4733 §2.5.1.3
    /// requires three final retransmissions so the last three frames
    /// of each tone all set the `E` bit — and dedup on `(ssrc, timestamp)`
    /// which uniquely identifies a tone.
    DtmfReceived {
        /// Event code (0-15 for DTMF).
        event: u8,
        /// End-of-event `E` bit.
        end_of_event: bool,
        /// -dBm0 volume (0-63).
        volume: u8,
        /// Duration in RTP timestamp units.
        duration: u16,
        /// RTP packet timestamp (dedup key for retransmits).
        timestamp: u32,
        /// SSRC that sent the event.
        ssrc: RtpSsrc,
    },
}

/// RTP session for sending and receiving RTP packets
///
/// This class manages an RTP session, including sending and receiving packets,
/// jitter buffer management, and demultiplexing of multiple streams.
///
/// # SSRC Demultiplexing
///
/// An RTP session can receive packets from multiple sources, each identified by
/// a unique Synchronization Source identifier (SSRC). This implementation
/// automatically demultiplexes incoming packets based on their SSRC:
///
/// 1. When a packet arrives, its SSRC is extracted
/// 2. If this is the first packet from this SSRC, a new stream is created
/// 3. The packet is processed by the appropriate stream, which handles:
///    - Sequence number tracking
///    - Jitter calculation
///    - Duplicate detection
///    - Packet reordering (via jitter buffer)
///
/// Each stream maintains its own statistics and state. You can access information
/// about individual streams using the `get_stream()`, `get_all_streams()`, and
/// `stream_count()` methods.
///
/// This approach aligns with RFC 3550 Section 8.2, which describes how to handle
/// multiple sources in a single RTP session.
pub struct RtpSession {
    /// Session configuration
    config: RtpSessionConfig,

    /// SSRC for this session
    ssrc: RtpSsrc,

    /// Transport for sending/receiving packets
    transport: Arc<dyn RtpTransport>,

    /// Map of received streams by SSRC. `DashMap` so the per-packet
    /// demultiplex hot path (`session/mod.rs:620`+) doesn't serialise
    /// every receive through a single mutex, and so `get_stream` /
    /// `stream_count` readers don't block the demux task.
    streams: Arc<DashMap<RtpSsrc, RtpStream>>,

    /// Sender Reports retained even when RTCP arrives before the source's
    /// first RTP packet.
    received_sender_reports: Arc<DashMap<RtpSsrc, ReceivedSenderReport>>,

    /// Packet scheduler for sending packets
    scheduler: Option<RtpScheduler>,

    /// Channel for receiving packets
    receiver: mpsc::Receiver<RtpPacket>,

    /// Canonical ordered RTP writer shared by every session send handle.
    packet_sender: Arc<RtpPacketSender>,

    /// Whether received RTP packets should also be mirrored into the legacy
    /// polling receive queue.
    receive_queue_enabled: bool,

    /// Event broadcaster
    event_tx: broadcast::Sender<RtpSessionEvent>,

    /// Receiving task handle
    recv_task: Option<JoinHandle<()>>,

    /// Session statistics. `parking_lot::Mutex` because every guard is
    /// CPU-only (counter updates, snapshot reads); the std variant
    /// added avoidable lock-acquire overhead on the send/recv hot
    /// paths and forced everything to unwrap poison.
    stats: Arc<parking_lot::Mutex<RtpSessionStats>>,

    /// RFC 3550 sender octet count (RTP payload bytes only).
    sender_octets: Arc<AtomicU64>,

    /// Media synchronization context
    media_sync: Option<Arc<std::sync::RwLock<crate::sync::MediaSync>>>,

    /// Whether the session is active
    active: bool,

    /// RTCP report generator
    rtcp_generator: Option<crate::stats::reports::RtcpReportGenerator>,

    /// RTCP sender task
    rtcp_task: Option<JoinHandle<()>>,

    /// Session bandwidth (bits per second)
    bandwidth_bps: u32,

    #[cfg(feature = "memory-diagnostics")]
    _memory_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard,
    #[cfg(feature = "memory-diagnostics")]
    _sender_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard,
    #[cfg(feature = "memory-diagnostics")]
    _receiver_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard,
    #[cfg(feature = "memory-diagnostics")]
    _event_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard,
}

impl RtpSession {
    /// Create a new RTP session
    pub async fn new(config: RtpSessionConfig) -> Result<Self> {
        Self::new_with_receive_queue(config, true, SymmetricRtpPolicy::default()).await
    }

    /// Create a new RTP session with an explicit symmetric-RTP policy.
    pub async fn new_with_symmetric_rtp_policy(
        config: RtpSessionConfig,
        policy: SymmetricRtpPolicy,
    ) -> Result<Self> {
        Self::new_with_receive_queue(config, true, policy).await
    }

    /// Create a new RTP session for event-driven consumers.
    ///
    /// Packets are still emitted through [`RtpSessionEvent::PacketReceived`],
    /// but they are not duplicated into the polling queue used by
    /// [`RtpSession::receive_packet`].
    pub async fn new_event_driven(config: RtpSessionConfig) -> Result<Self> {
        Self::new_with_receive_queue(config, false, SymmetricRtpPolicy::default()).await
    }

    /// Create an event-driven RTP session with an explicit symmetric-RTP
    /// learning/rebinding policy.
    pub async fn new_event_driven_with_symmetric_rtp_policy(
        config: RtpSessionConfig,
        policy: SymmetricRtpPolicy,
    ) -> Result<Self> {
        Self::new_with_receive_queue(config, false, policy).await
    }

    async fn new_with_receive_queue(
        config: RtpSessionConfig,
        receive_queue_enabled: bool,
        symmetric_rtp_policy: SymmetricRtpPolicy,
    ) -> Result<Self> {
        let session_buffer_config = config.session_buffer_config;
        let transport_buffer_config = config.transport_buffer_config;

        // Generate SSRC if not provided
        let ssrc = config.ssrc.unwrap_or_else(|| {
            let mut rng = rand::thread_rng();
            rng.gen::<u32>()
        });

        // Create transport config - respect provided ports!
        let transport_config = RtpTransportConfig {
            local_rtp_addr: config.local_addr,
            local_rtcp_addr: None, // RTCP on same port for now
            symmetric_rtp: true,
            rtcp_mux: true, // Enable RTCP multiplexing by default
            session_id: Some(format!("rtp-session-{}", ssrc)),
            // Don't allocate a new port - use the one provided in config
            use_port_allocator: false,
            buffer_config: transport_buffer_config,
        };

        // Create UDP transport
        let transport = Arc::new(
            UdpRtpTransport::new_with_symmetric_rtp_policy(transport_config, symmetric_rtp_policy)
                .await?,
        );

        // Create channels for receive-side internal communication.
        let (receiver_tx, receiver_rx) =
            mpsc::channel(session_buffer_config.receiver_channel_capacity.max(1));
        let (event_tx, _) = broadcast::channel(session_buffer_config.event_channel_capacity.max(1));

        // Create scheduler if needed
        let scheduler = Some(RtpScheduler::new(
            config.clock_rate,
            rand::thread_rng().gen::<u16>(), // Random starting sequence
            rand::thread_rng().gen::<u32>(), // Random starting timestamp
        ));
        let stats = Arc::new(parking_lot::Mutex::new(RtpSessionStats::default()));
        let sender_octets = Arc::new(AtomicU64::new(0));
        let packet_sender = Arc::new(RtpPacketSender::new(
            transport.clone(),
            config.remote_addr,
            ssrc,
            scheduler
                .as_ref()
                .expect("RTP session scheduler")
                .sequence_handle(),
            stats.clone(),
            sender_octets.clone(),
            event_tx.clone(),
            session_buffer_config.sender_channel_capacity,
        ));

        // Create RTCP report generator
        let hostname = hostname::get().unwrap_or_else(|_| "unknown".into());
        let hostname_str = hostname.to_string_lossy();
        let cname = format!(
            "{}@{}",
            std::env::var("USER").unwrap_or_else(|_| "user".to_string()),
            hostname_str
        );
        let rtcp_generator = crate::stats::reports::RtcpReportGenerator::new(ssrc, cname);

        let mut session = Self {
            config,
            ssrc,
            transport,
            streams: Arc::new(DashMap::new()),
            received_sender_reports: Arc::new(DashMap::new()),
            scheduler,
            receiver: receiver_rx,
            packet_sender,
            receive_queue_enabled,
            event_tx,
            recv_task: None,
            stats,
            sender_octets,
            media_sync: None,
            active: false,
            rtcp_generator: Some(rtcp_generator),
            rtcp_task: None,
            bandwidth_bps: 64000, // Default bandwidth: 64 kbps
            #[cfg(feature = "memory-diagnostics")]
            _memory_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "rtp_core.rtp_session",
                std::mem::size_of::<Self>(),
            ),
            #[cfg(feature = "memory-diagnostics")]
            _sender_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "rtp_core.rtp_session.sender_channel_capacity",
                session_buffer_config.sender_channel_capacity * std::mem::size_of::<RtpPacket>(),
            ),
            #[cfg(feature = "memory-diagnostics")]
            _receiver_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "rtp_core.rtp_session.receiver_channel_capacity",
                session_buffer_config.receiver_channel_capacity * std::mem::size_of::<RtpPacket>(),
            ),
            #[cfg(feature = "memory-diagnostics")]
            _event_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "rtp_core.rtp_session.event_broadcast_capacity",
                session_buffer_config.event_channel_capacity
                    * std::mem::size_of::<RtpSessionEvent>(),
            ),
        };

        // Start the session
        session.start(receiver_tx).await?;

        Ok(session)
    }

    /// Start the session tasks
    async fn start(&mut self, receiver_tx: mpsc::Sender<RtpPacket>) -> Result<()> {
        if self.active {
            return Ok(());
        }

        let transport = self.transport.clone();
        let stats_recv = self.stats.clone();
        let remote_addr = self.config.remote_addr;
        let event_tx_recv = self.event_tx.clone();
        let clock_rate = self.config.clock_rate;
        let _payload_type = self.config.payload_type;
        let ssrc = self.ssrc;
        let streams_map = self.streams.clone();
        let received_sender_reports = self.received_sender_reports.clone();
        let _jitter_buffer_enabled = self.config.enable_jitter_buffer;
        let _jitter_size = self.config.jitter_buffer_size.unwrap_or(50);
        let _max_age_ms = self.config.max_packet_age_ms.unwrap_or(200);
        let receive_queue_enabled = self.receive_queue_enabled;

        let media_sync = self.media_sync.clone();

        // If we have a remote address, set it on the transport
        if let Some(addr) = remote_addr {
            // Set the remote RTP address on the UDP transport
            if let Some(t) = transport.as_any().downcast_ref::<UdpRtpTransport>() {
                t.set_remote_rtp_addr(addr).await;
            }
        }

        // Prepare the scheduler's timestamp state, but do not start its
        // millisecond polling task. Session sends use the single ordered
        // packet writer above; no production code uses the scheduler queue.
        // Starting one 1 ms timer per call was measurable CPU load under
        // SIPp fan-out.
        if let Some(scheduler) = &mut self.scheduler {
            // Set appropriate timestamp increment based on packet interval
            let interval_ms = 20; // Default 20ms packet interval
            let samples_per_packet = (clock_rate as f64 * (interval_ms as f64 / 1000.0)) as u32;
            scheduler.set_interval(interval_ms, samples_per_packet);
        }

        // Start receiving task
        let recv_transport = transport.clone();

        // Subscribe to transport events to handle RTCP packets
        let mut transport_events = recv_transport.subscribe();

        let recv_task = spawn_memory_tracked("rtp_core.rtp_session.recv_task", async move {
            // IMPORTANT: Only handle events from transport, no direct packet reception
            // to avoid race conditions where two tasks read from the same socket
            loop {
                match transport_events.recv().await {
                    Ok(crate::traits::RtpEvent::RtcpReceived { data, source: _ }) => {
                        // Parse the complete compound packet. Unknown but
                        // well-formed members are retained by the tolerant
                        // parser and ignored by the production handler.
                        match crate::packet::rtcp::RtcpCompoundPacket::parse_tolerant(&data) {
                            Ok(compound) => {
                                for rtcp_member in compound.packets {
                                    let rtcp_packet = match rtcp_member {
                                        crate::packet::rtcp::RtcpCompoundMember::Known(packet) => {
                                            packet
                                        }
                                        crate::packet::rtcp::RtcpCompoundMember::Unknown(
                                            unknown,
                                        ) => {
                                            trace!(
                                                "Ignoring unimplemented RTCP packet type {}",
                                                unknown.packet_type
                                            );
                                            continue;
                                        }
                                    };
                                    match rtcp_packet {
                                        crate::packet::rtcp::RtcpPacket::Goodbye(bye) => {
                                            // Extract the SSRC and reason
                                            if !bye.sources.is_empty() {
                                                let source_ssrc = bye.sources[0];

                                                // Broadcast BYE event
                                                let _ = event_tx_recv.send(RtpSessionEvent::Bye {
                                                    ssrc: source_ssrc,
                                                    reason: bye.reason,
                                                });

                                                info!(
                                                    "Received RTCP BYE from SSRC={:08x}",
                                                    source_ssrc
                                                );
                                            }
                                        }
                                        crate::packet::rtcp::RtcpPacket::SenderReport(sr) => {
                                            // Process sender report
                                            let report_ssrc = sr.ssrc;

                                            debug!(
                                                "Received RTCP SR from SSRC={:08x}",
                                                report_ssrc
                                            );

                                            let sender_report = ReceivedSenderReport {
                                                lsr: sr.ntp_timestamp.to_u32(),
                                                received_at: Instant::now(),
                                            };
                                            received_sender_reports
                                                .insert(report_ssrc, sender_report);

                                            // Update stream statistics if RTP for this source
                                            // has already created the stream. The retained map
                                            // above covers the SR-before-RTP ordering.
                                            if let Some(mut stream) =
                                                streams_map.get_mut(&report_ssrc)
                                            {
                                                stream.update_last_sr_info(
                                                    sender_report.lsr,
                                                    sender_report.received_at,
                                                );

                                                debug!(
                                                    "Updated RTCP SR info for stream SSRC={:08x}",
                                                    report_ssrc
                                                );
                                            }

                                            // If media sync is enabled, update it
                                            if let Some(sync) = &media_sync {
                                                if let Ok(mut media_sync) = sync.write() {
                                                    // Update synchronization data
                                                    media_sync.update_from_sr(
                                                        report_ssrc,
                                                        sr.ntp_timestamp,
                                                        sr.rtp_timestamp,
                                                    );
                                                }
                                            }

                                            // Emit SR event for external processing
                                            let _ = event_tx_recv.send(
                                                RtpSessionEvent::RtcpSenderReport {
                                                    ssrc: report_ssrc,
                                                    ntp_timestamp: sr.ntp_timestamp,
                                                    rtp_timestamp: sr.rtp_timestamp,
                                                    packet_count: sr.sender_packet_count,
                                                    octet_count: sr.sender_octet_count,
                                                    report_blocks: sr.report_blocks,
                                                },
                                            );
                                        }
                                        crate::packet::rtcp::RtcpPacket::ReceiverReport(rr) => {
                                            // Process receiver report
                                            let report_ssrc = rr.ssrc;

                                            debug!(
                                        "Received RTCP RR from SSRC={:08x} with {} report blocks",
                                        report_ssrc,
                                        rr.report_blocks.len()
                                    );

                                            // If there's a report block about our SSRC, process it
                                            for block in &rr.report_blocks {
                                                if block.ssrc == ssrc {
                                                    debug!(
                                                "Processing report block about our SSRC={:08x}",
                                                ssrc
                                            );

                                                    // This block describes loss of our outbound
                                                    // stream. Keep the session's `packets_lost`
                                                    // counter reserved for locally observed
                                                    // inbound sequence gaps.
                                                    let fraction_lost =
                                                        block.fraction_lost as f64 / 256.0;
                                                    debug!(
                                                        "Remote-reported outbound packet loss: {}% (fraction={}, cumulative={})",
                                                        fraction_lost * 100.0,
                                                        block.fraction_lost,
                                                        block.cumulative_lost
                                                    );
                                                }
                                            }

                                            // Emit RR event for external processing
                                            let _ = event_tx_recv.send(
                                                RtpSessionEvent::RtcpReceiverReport {
                                                    ssrc: report_ssrc,
                                                    report_blocks: rr.report_blocks,
                                                },
                                            );
                                        }
                                        // Handle other RTCP packet types as needed.
                                        other => {
                                            trace!("Received RTCP packet: {:?}", other);
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                warn!("Failed to parse compound RTCP packet: {}", error);
                            }
                        }
                    }
                    Ok(crate::traits::RtpEvent::MediaReceived {
                        payload_type,
                        sequence_number,
                        timestamp,
                        payload,
                        padding_size,
                        source,
                        ssrc: ssrc_from_event,
                        marker,
                        ..
                    }) => {
                        // Handle RTP packets received via transport events
                        // This is the ONLY path for RTP packets to avoid race conditions

                        // Reconstruct minimal RTP header for processing
                        let header = RtpHeader {
                            version: 2,
                            padding: padding_size != 0,
                            extension: false,
                            cc: 0,
                            marker,
                            payload_type,
                            sequence_number,
                            timestamp,
                            ssrc: ssrc_from_event,
                            csrc: vec![],
                            extensions: None,
                        };

                        let packet = RtpPacket {
                            header,
                            payload: payload.clone(),
                            padding_size,
                        };

                        // Use the SSRC from the event
                        let packet_ssrc = ssrc_from_event;

                        // Get or create the stream for this SSRC. The
                        // `entry` runs the closure exactly once per
                        // first insert, so `created` flips iff this
                        // packet's SSRC has never been seen — that's
                        // also the signal for the `NewStreamDetected`
                        // event downstream. The shard guard is dropped
                        // before we forward the packet.
                        let (is_new_stream, output_packet) = {
                            let mut created = false;
                            let mut entry = streams_map.entry(packet_ssrc).or_insert_with(|| {
                                created = true;
                                info!("New RTP stream detected with SSRC={:08x}", packet_ssrc);
                                let mut stream = RtpStream::new(packet_ssrc, clock_rate);
                                if let Some(sender_report) =
                                    received_sender_reports.get(&packet_ssrc)
                                {
                                    stream.update_last_sr_info(
                                        sender_report.lsr,
                                        sender_report.received_at,
                                    );
                                }
                                stream
                            });

                            let before = entry.get_stats();
                            let output = entry.process_packet(packet);
                            let after = entry.get_stats();
                            let jitter_ms = entry.get_jitter_ms();
                            drop(entry);

                            // Session counters are aggregate stream deltas.
                            // Every datagram, including duplicates and late
                            // packets, remains deliverable with buffering off.
                            {
                                let mut session_stats = stats_recv.lock();
                                session_stats.packets_received += 1;
                                session_stats.bytes_received +=
                                    payload.len() as u64 + 12 + u64::from(padding_size);
                                session_stats.packets_lost = session_stats
                                    .packets_lost
                                    .saturating_sub(before.packets_lost)
                                    .saturating_add(after.packets_lost);
                                session_stats.packets_duplicated = session_stats
                                    .packets_duplicated
                                    .saturating_sub(before.duplicates)
                                    .saturating_add(after.duplicates);
                                session_stats.packets_out_of_order = session_stats
                                    .packets_out_of_order
                                    .saturating_sub(before.packets_out_of_order)
                                    .saturating_add(after.packets_out_of_order);
                                session_stats.jitter_ms = jitter_ms;
                                session_stats.remote_addr = Some(source);
                            }

                            (created, output)
                        };

                        // If this is a new stream, emit the NewStreamDetected event
                        if is_new_stream {
                            let _ = event_tx_recv
                                .send(RtpSessionEvent::NewStreamDetected { ssrc: packet_ssrc });
                        }

                        // Forward the packet
                        if let Some(output) = output_packet {
                            if receive_queue_enabled {
                                match receiver_tx.try_send(output.clone()) {
                                    Ok(()) => {}
                                    Err(mpsc::error::TrySendError::Full(_)) => {
                                        trace!(
                                            "RTP receive polling queue full; dropping duplicate packet"
                                        );
                                    }
                                    Err(mpsc::error::TrySendError::Closed(_)) => {
                                        error!(
                                            "Failed to forward RTP packet to receiver: channel closed"
                                        );
                                    }
                                }
                            }

                            // Broadcast packet received event
                            let _ = event_tx_recv.send(RtpSessionEvent::PacketReceived(output));
                        }
                    }
                    Ok(crate::traits::RtpEvent::Error(e)) => {
                        error!("Transport error: {}", e);
                        let _ = event_tx_recv.send(RtpSessionEvent::Error(e));
                    }
                    Ok(crate::traits::RtpEvent::DtmfEvent {
                        event,
                        end_of_event,
                        volume,
                        duration,
                        timestamp,
                        ssrc,
                        ..
                    }) => {
                        // RFC 4733: forward as a typed session event so
                        // media-core's RTP handler can bubble the digit
                        // up to session-core without re-parsing the
                        // 4-byte body.
                        let _ = event_tx_recv.send(RtpSessionEvent::DtmfReceived {
                            event,
                            end_of_event,
                            volume,
                            duration,
                            timestamp,
                            ssrc,
                        });
                    }
                    Err(e) => {
                        debug!("Transport event channel error: {}", e);
                    }
                }
            }
        });

        // Start RTCP sending task if we have a remote address and report generator
        if let (Some(remote_addr), Some(mut rtcp_generator)) =
            (self.config.remote_addr, self.rtcp_generator.take())
        {
            let transport = self.transport.clone();
            let ssrc = self.ssrc;
            let event_tx = self.event_tx.clone();
            let stats = self.stats.clone();
            let sender_octets = self.sender_octets.clone();
            let report_streams = self.streams.clone();
            let active_state = Arc::new(tokio::sync::Mutex::new(true));
            let _active_state_clone = active_state.clone();
            let bandwidth = self.bandwidth_bps;

            // Set bandwidth in the generator
            rtcp_generator.set_bandwidth(bandwidth);

            // Start the RTCP task
            let rtcp_task = spawn_memory_tracked("rtp_core.rtp_session.rtcp_task", async move {
                debug!("RTCP scheduling task started");

                // Initial interval calculation
                let mut interval = rtcp_generator.calculate_interval();
                debug!("Initial RTCP interval: {:?}", interval);

                while *active_state.lock().await {
                    // Wait for the calculated interval
                    tokio::time::sleep(interval).await;

                    // Check if we should continue
                    if !*active_state.lock().await {
                        break;
                    }

                    // Update RTP statistics before sending the report
                    let (sender_packet_count, sender_octet_count) =
                        sender_report_totals(&stats, &sender_octets);
                    rtcp_generator.set_sent_totals(sender_packet_count, sender_octet_count);

                    // Log the current stats for debugging
                    debug!(
                        "Current stats for RTCP report: packets={}, payload_octets={}",
                        sender_packet_count, sender_octet_count
                    );

                    // Send an RTCP report regardless of should_send_report logic for this example
                    // We'll send a compound packet with SR and SDES
                    debug!("Sending RTCP report");

                    // Generate sender report
                    let rtp_timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u32;

                    let mut sr = rtcp_generator.generate_sender_report(rtp_timestamp);
                    sr.report_blocks = take_rtcp_report_blocks(&report_streams);
                    let sdes = rtcp_generator.generate_sdes();

                    // Create compound packet
                    let mut compound = crate::packet::rtcp::RtcpCompoundPacket::new_with_sr(sr);
                    compound.add_sdes(sdes);

                    // Send the compound packet
                    if let Ok(data) = compound.serialize() {
                        if let Err(e) = transport.send_rtcp_bytes(&data, remote_addr).await {
                            if matches!(e, Error::UnsupportedFeature(_)) {
                                trace!(
                                    "Skipping RTCP report while authenticated SRTCP is unavailable"
                                );
                            } else {
                                warn!("Failed to send RTCP compound packet: {}", e);
                            }
                        } else {
                            info!("Sent RTCP compound packet of {} bytes", data.len());

                            // Emit SR event
                            if let Some(sr) = compound.get_sr() {
                                let _ = event_tx.send(RtpSessionEvent::RtcpSenderReport {
                                    ssrc,
                                    ntp_timestamp: sr.ntp_timestamp,
                                    rtp_timestamp: sr.rtp_timestamp,
                                    packet_count: sr.sender_packet_count,
                                    octet_count: sr.sender_octet_count,
                                    report_blocks: sr.report_blocks.clone(),
                                });
                            }
                        }
                    }

                    // Recalculate interval for next report
                    interval = rtcp_generator.calculate_interval();
                    debug!("Next RTCP report in {:?}", interval);
                }

                debug!("RTCP scheduling task ended");
            });

            self.rtcp_task = Some(rtcp_task);
        }

        self.recv_task = Some(recv_task);
        self.active = true;

        info!("Started RTP session with SSRC={:08x}", ssrc);
        Ok(())
    }

    /// Send an RTP packet with payload. Now `&self` — sequence
    /// numbers are managed by the ordered writer shared with the
    /// scheduler, so this no longer requires exclusive borrow. Lets
    /// concurrent callers (audio TX, DTMF transmitter, bridge
    /// forwarder) send without serialising on
    /// `Arc<Mutex<RtpSession>>`.
    pub async fn send_packet(
        &self,
        timestamp: RtpTimestamp,
        payload: Bytes,
        marker: bool,
    ) -> Result<()> {
        self.send_packet_with_pt(timestamp, payload, marker, self.config.payload_type)
            .await
    }

    /// Send an RTP packet overriding the configured payload type.
    ///
    /// Needed for RFC 4733 telephone-event (DTMF) transmission — the
    /// session's `config.payload_type` is the audio codec PT (0/8/etc),
    /// but DTMF rides on a distinct PT (typically 101). All other
    /// fields (SSRC, marker, timestamp) follow the same rules as
    /// [`send_packet`](Self::send_packet).
    pub async fn send_packet_with_pt(
        &self,
        timestamp: RtpTimestamp,
        payload: Bytes,
        marker: bool,
        payload_type: u8,
    ) -> Result<()> {
        // The caller controls PT + timestamp explicitly — RFC 4733
        // telephone-event needs every packet of a tone to share the
        // start timestamp. The common packet writer supplies the
        // session's one sequence space and wire-order authority.
        self.packet_sender
            .send_payload(timestamp, payload, marker, payload_type)
            .await
    }

    /// Get a lock-free send handle for this session.
    ///
    /// `RtpSendHandle` is `Send + Sync + Clone` and bypasses the
    /// outer `Arc<Mutex<RtpSession>>` that wraps this session in
    /// media-core. Every handle shares the session's ordered packet
    /// writer, so the wire-side sees one monotonic sequence space
    /// across audio, DTMF, bridge, and direct session sends.
    pub fn send_handle(&self) -> Option<RtpSendHandle> {
        Some(RtpSendHandle {
            packet_sender: self.packet_sender.clone(),
            default_payload_type: self.config.payload_type,
        })
    }

    /// Receive an RTP packet
    pub async fn receive_packet(&mut self) -> Result<RtpPacket> {
        self.receiver
            .recv()
            .await
            .ok_or_else(|| Error::SessionError("Receiver channel closed".to_string()))
    }

    /// Get the session statistics
    pub fn get_stats(&self) -> RtpSessionStats {
        self.stats.lock().clone()
    }

    /// Get current bounded-queue occupancy for leak/perf diagnostics.
    pub fn queue_diagnostics(&self) -> RtpSessionQueueDiagnostics {
        let (sender_queue_packets, sender_capacity_packets) =
            self.packet_sender.queue_diagnostics();
        let (receiver_queue_packets, receiver_capacity_packets) = if self.receive_queue_enabled {
            (self.receiver.len(), self.receiver.max_capacity())
        } else {
            (0, 0)
        };
        RtpSessionQueueDiagnostics {
            sender_queue_packets,
            sender_capacity_packets,
            receiver_queue_packets,
            receiver_capacity_packets,
            event_queue_events: self.event_tx.len(),
            event_receiver_count: self.event_tx.receiver_count(),
            #[cfg(feature = "memory-diagnostics")]
            stream_count: self.streams.len(),
        }
    }

    /// Set the remote address
    pub async fn set_remote_addr(&mut self, addr: SocketAddr) {
        self.config.remote_addr = Some(addr);

        // Update stats with remote address
        {
            let mut stats = self.stats.lock();
            stats.remote_addr = Some(addr);
        }

        // Update the transport's remote address
        self.packet_sender.set_remote_addr(addr);
        if let Some(t) = self.transport.as_any().downcast_ref::<UdpRtpTransport>() {
            t.set_remote_rtp_addr(addr).await;
        }
    }

    /// Get the local address
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.transport.local_rtp_addr()
    }

    /// Get the transport
    pub fn transport(&self) -> Arc<dyn RtpTransport> {
        self.transport.clone()
    }

    /// Close the session and clean up resources
    pub async fn close(&mut self) -> Result<()> {
        // Send BYE packet if we have a remote address
        if let Some(remote_addr) = self.config.remote_addr {
            // Create BYE packet
            let bye = crate::packet::rtcp::RtcpGoodbye::new_with_reason(
                self.ssrc,
                "Session closed".to_string(),
            );

            // Create RTCP packet
            let rtcp_packet = crate::packet::rtcp::RtcpPacket::Goodbye(bye);

            // Serialize and send
            match rtcp_packet.serialize() {
                Ok(data) => {
                    // Send using transport (through RTCP port if available)
                    if let Err(e) = self.transport.send_rtcp_bytes(&data, remote_addr).await {
                        if matches!(e, Error::UnsupportedFeature(_)) {
                            trace!("Skipping RTCP BYE while authenticated SRTCP is unavailable");
                        } else {
                            warn!("Failed to send RTCP BYE: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to serialize RTCP BYE: {}", e);
                }
            }
        }

        // Stop the scheduler if running
        if let Some(scheduler) = &mut self.scheduler {
            scheduler.stop().await;
        }

        // Stop the receive task
        if let Some(handle) = self.recv_task.take() {
            handle.abort();
            let _ = handle.await;
        }

        self.packet_sender.close();

        // Stop the RTCP task
        if let Some(handle) = self.rtcp_task.take() {
            handle.abort();
            let _ = handle.await;
        }

        // Close the transport
        let _ = self.transport.close().await;

        self.active = false;
        info!("Closed RTP session with SSRC={:08x}", self.ssrc);

        Ok(())
    }

    /// Get the current timestamp
    pub fn get_timestamp(&self) -> RtpTimestamp {
        if let Some(scheduler) = &self.scheduler {
            scheduler.get_timestamp()
        } else {
            // Generate based on uptime if no scheduler
            let now = std::time::SystemTime::now();
            let since_epoch = now
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_else(|_| Duration::from_secs(0));

            let secs = since_epoch.as_secs();
            let nanos = since_epoch.subsec_nanos();

            // Convert to timestamp units (samples)
            let timestamp_secs = secs * (self.config.clock_rate as u64);
            let timestamp_fraction =
                ((nanos as u64) * (self.config.clock_rate as u64)) / 1_000_000_000;

            (timestamp_secs + timestamp_fraction) as u32
        }
    }

    /// Current RTP timestamp cursor — the timestamp the next audio
    /// packet would carry. Coherent with the audio stream's SSRC per
    /// RFC 4733 §2.1: telephone-event packets share the start
    /// timestamp of the surrounding audio so receivers can align
    /// tones with the audio they overlay.
    ///
    /// The implementation derives the timestamp from wall-clock at
    /// the configured clock rate rather than reading the scheduler's
    /// internal `self.timestamp` field directly. This matters because:
    ///
    /// - When audio packets are flowing through the scheduler at the
    ///   audio rate, wall-clock and scheduler cursor stay in lockstep
    ///   (both advance at `clock_rate` Hz), so the returned value is
    ///   audio-anchored as RFC 4733 expects.
    /// - When no audio is flowing (e.g. the streampeer/dtmf example,
    ///   which exercises only RTP-control with PT 101 and never
    ///   pushes a PCMU audio source), the scheduler's `self.timestamp`
    ///   is frozen. A frozen timestamp would collapse successive DTMF
    ///   tones into one `(peer, ssrc, ts)` dedup key at the receiver,
    ///   silently dropping every digit after the first. Wall-clock
    ///   keeps successive tones distinct unconditionally.
    pub fn current_timestamp(&self) -> RtpTimestamp {
        let now = std::time::SystemTime::now();
        let since_epoch = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        let secs = since_epoch.as_secs();
        let nanos = since_epoch.subsec_nanos();
        let timestamp_secs = secs * (self.config.clock_rate as u64);
        let timestamp_fraction = ((nanos as u64) * (self.config.clock_rate as u64)) / 1_000_000_000;
        (timestamp_secs + timestamp_fraction) as u32
    }

    /// Get the SSRC of this session
    pub fn get_ssrc(&self) -> RtpSsrc {
        self.ssrc
    }

    /// Subscribe to session events
    pub fn subscribe(&self) -> broadcast::Receiver<RtpSessionEvent> {
        self.event_tx.subscribe()
    }

    /// Get the current payload type
    pub fn get_payload_type(&self) -> u8 {
        self.config.payload_type
    }

    /// Set the payload type
    pub fn set_payload_type(&mut self, payload_type: u8) {
        self.config.payload_type = payload_type;
    }

    /// Get a stream by SSRC, if it exists
    pub async fn get_stream(&self, ssrc: RtpSsrc) -> Option<RtpStreamStats> {
        self.streams.get(&ssrc).map(|stream| stream.get_stats())
    }

    /// Get a list of all current streams
    pub async fn get_all_streams(&self) -> Vec<RtpStreamStats> {
        self.streams
            .iter()
            .map(|entry| entry.value().get_stats())
            .collect()
    }

    /// Get the number of active streams
    pub async fn stream_count(&self) -> usize {
        self.streams.len()
    }

    /// Get a list of all SSRCs known to this session
    ///
    /// This returns all SSRCs that have been seen or explicitly precreated.
    pub async fn get_all_ssrcs(&self) -> Vec<RtpSsrc> {
        self.streams.iter().map(|entry| *entry.key()).collect()
    }

    /// Force creation of a stream for a specific SSRC
    ///
    /// This is useful when we want to ensure a stream exists for an SSRC
    /// even if no packets have been received yet.
    pub async fn create_stream_for_ssrc(&mut self, ssrc: RtpSsrc) -> bool {
        self.create_stream_for_ssrc_after(ssrc, std::future::ready(()))
            .await
    }

    async fn create_stream_for_ssrc_after<F>(&mut self, ssrc: RtpSsrc, before_insert: F) -> bool
    where
        F: std::future::Future<Output = ()>,
    {
        // Check if this SSRC already exists. The contains_key + insert
        // pair has a benign race (two callers may both decide "new" and
        // race the insert), but we only need a stable per-SSRC entry —
        // DashMap's `entry()` arbitrates.
        if self.streams.contains_key(&ssrc) {
            debug!("Stream for SSRC={:08x} already exists", ssrc);
            return false;
        }

        // Session ingress remains deliberately unbuffered in this correctness
        // repair. Enabling the legacy RtpStream jitter buffer here while the
        // first-packet path stays unbuffered makes precreated streams hold the
        // second sequential packet and can reverse later delivery.
        info!("Manually creating new RTP stream for SSRC={:08x}", ssrc);
        let stream = RtpStream::new(ssrc, self.config.clock_rate);

        // Production passes an immediately-ready future. Keeping the
        // insertion boundary explicit lets the race regression deliver an SR
        // after precreation has begun but before the stream becomes visible.
        before_insert.await;

        // The contains_key check above is racy w.r.t. the recv hot
        // path also inserting on first packet; `entry()` arbitrates.
        // The closure runs only on first insert, so `closure_ran`
        // tells us whether *we* created the entry or lost the race.
        let mut closure_ran = false;
        {
            let _entry = self.streams.entry(ssrc).or_insert_with(|| {
                closure_ran = true;
                stream
            });
        }
        if !closure_ran {
            return false;
        }

        // Reconcile retained SR state only after the stream is visible. This
        // closes both sides of the handoff with the RTCP task, which stores
        // the SR first and then looks up the stream: an SR arriving before
        // insertion is found here, while one arriving after insertion is
        // applied by the RTCP task. Lock the visible stream first so a newer
        // RTCP update waits behind reconciliation and cannot be overwritten
        // by an older retained snapshot.
        if let Some(mut stream) = self.streams.get_mut(&ssrc) {
            if let Some(sender_report) = self.received_sender_reports.get(&ssrc) {
                stream.update_last_sr_info(sender_report.lsr, sender_report.received_at);
            }
        }

        // Emit the new stream event
        debug!("Emitting NewStreamDetected event for SSRC={:08x}", ssrc);
        let _ = self
            .event_tx
            .send(RtpSessionEvent::NewStreamDetected { ssrc });

        true
    }

    /// Send an RTCP BYE packet to notify that we're leaving the session
    ///
    /// This can be used to notify other participants that we're leaving the session
    /// without closing the entire RtpSession. The BYE packet includes our SSRC and
    /// an optional reason string.
    ///
    /// Returns an error if serialization fails or if there's no remote address configured.
    pub async fn send_bye(&self, reason: Option<String>) -> Result<()> {
        // Check if we have a remote address
        let remote_addr = match self.config.remote_addr {
            Some(addr) => addr,
            None => {
                return Err(Error::SessionError(
                    "No remote address configured".to_string(),
                ))
            }
        };

        // Create BYE packet
        let bye = crate::packet::rtcp::RtcpGoodbye::new_with_reason(
            self.ssrc,
            reason.unwrap_or_else(|| "Session terminated".to_string()),
        );

        // Create RTCP packet
        let rtcp_packet = crate::packet::rtcp::RtcpPacket::Goodbye(bye);

        // Serialize and send
        match rtcp_packet.serialize() {
            Ok(data) => {
                // Send using transport
                self.transport.send_rtcp_bytes(&data, remote_addr).await
            }
            Err(e) => Err(Error::SerializationError(format!(
                "Failed to serialize RTCP BYE: {}",
                e
            ))),
        }
    }

    /// Send an RTCP Sender Report (SR) packet
    ///
    /// A Sender Report contains:
    /// - Our SSRC
    /// - Current NTP and RTP timestamps
    /// - Packet and octet counts
    /// - Optional report blocks with reception statistics about other sources
    ///
    /// This method generates an SR based on the current session statistics, which is useful
    /// for providing quality metrics to other participants.
    ///
    /// Returns an error if serialization fails or if there's no remote address configured.
    pub async fn send_sender_report(&self) -> Result<()> {
        // Check if we have a remote address
        let remote_addr = match self.config.remote_addr {
            Some(addr) => addr,
            None => {
                return Err(Error::SessionError(
                    "No remote address configured".to_string(),
                ))
            }
        };

        // Snapshot both sender totals while holding the same lock used by the
        // packet-send update so the report cannot mix two send generations.
        let (sender_packet_count, sender_octet_count) =
            sender_report_totals(&self.stats, &self.sender_octets);

        // Create a new SR packet
        let mut sr = crate::packet::rtcp::RtcpSenderReport::new(self.ssrc);

        // Set current NTP timestamp
        sr.ntp_timestamp = crate::packet::rtcp::NtpTimestamp::now();

        // Set current RTP timestamp (convert from NTP time)
        sr.rtp_timestamp = self.get_timestamp();

        // Set packet and octet count from session stats
        sr.sender_packet_count = sender_packet_count;
        sr.sender_octet_count = sender_octet_count;

        // Add report blocks for active streams (remote SSRCs we're receiving from)
        // Up to 31 streams per RTCP packet.
        for block in take_rtcp_report_blocks(&self.streams) {
            sr.add_report_block(block);
        }

        // **FIX: Update our own MediaSync context with the SR data we're sending**
        // This ensures our own timing data flows into MediaSync for API access
        if let Some(media_sync) = &self.media_sync {
            if let Ok(mut sync) = media_sync.write() {
                sync.update_from_sr(self.ssrc, sr.ntp_timestamp, sr.rtp_timestamp);
                debug!(
                    "Updated MediaSync with our own SR: SSRC={:08x}, NTP={:?}, RTP={}",
                    self.ssrc, sr.ntp_timestamp, sr.rtp_timestamp
                );
            }
        }

        // Create RTCP packet
        let rtcp_packet = crate::packet::rtcp::RtcpPacket::SenderReport(sr);

        // Serialize and send
        match rtcp_packet.serialize() {
            Ok(data) => self.transport.send_rtcp_bytes(&data, remote_addr).await,
            Err(e) => Err(Error::SerializationError(format!(
                "Failed to serialize RTCP SR: {}",
                e
            ))),
        }
    }

    /// Send an RTCP Receiver Report (RR) packet
    ///
    /// A Receiver Report contains:
    /// - Our SSRC
    /// - Report blocks with reception statistics about other sources
    ///
    /// This method generates an RR based on the current stream statistics, which is useful
    /// for providing quality metrics to other participants when we're receiving but not sending.
    ///
    /// Returns an error if serialization fails or if there's no remote address configured.
    pub async fn send_receiver_report(&self) -> Result<()> {
        // Check if we have a remote address
        let remote_addr = match self.config.remote_addr {
            Some(addr) => addr,
            None => {
                return Err(Error::SessionError(
                    "No remote address configured".to_string(),
                ))
            }
        };

        // Create a new RR packet
        let mut rr = crate::packet::rtcp::RtcpReceiverReport::new(self.ssrc);

        // Add report blocks for active streams (remote SSRCs we're receiving from)
        // Up to 31 streams per RTCP packet.
        for block in take_rtcp_report_blocks(&self.streams) {
            rr.add_report_block(block);
        }

        // Create RTCP packet
        let rtcp_packet = crate::packet::rtcp::RtcpPacket::ReceiverReport(rr);

        // Serialize and send
        match rtcp_packet.serialize() {
            Ok(data) => self.transport.send_rtcp_bytes(&data, remote_addr).await,
            Err(e) => Err(Error::SerializationError(format!(
                "Failed to serialize RTCP RR: {}",
                e
            ))),
        }
    }

    /// Enable media synchronization
    pub fn enable_media_sync(&mut self) -> Arc<std::sync::RwLock<crate::sync::MediaSync>> {
        let sync = Arc::new(std::sync::RwLock::new(crate::sync::MediaSync::new()));
        self.media_sync = Some(sync.clone());

        // Register our stream
        if let Ok(mut media_sync) = sync.write() {
            media_sync.register_stream(self.ssrc, self.config.clock_rate);
        }

        sync
    }

    /// Get the media synchronization context
    pub fn media_sync(&self) -> Option<Arc<std::sync::RwLock<crate::sync::MediaSync>>> {
        self.media_sync.clone()
    }

    /// Set the session bandwidth in bits per second
    ///
    /// This affects the RTCP report interval calculation.
    /// Higher bandwidth means more frequent RTCP packets.
    pub fn set_bandwidth(&mut self, bandwidth_bps: u32) {
        self.bandwidth_bps = bandwidth_bps;
    }

    /// Create a sender handle for this session
    ///
    /// This creates a lightweight handle that can be used to send RTP packets
    /// from another thread. This is useful when you need to send packets
    /// but don't want to clone the entire session.
    pub fn create_sender_handle(&self) -> RtpSessionSender {
        RtpSessionSender {
            packet_sender: self.packet_sender.clone(),
            payload_type: self.config.payload_type,
            clock_rate: self.config.clock_rate,
        }
    }

    /// Get the UDP socket handle from the transport
    ///
    /// This method is used to access the underlying UDP socket when needed for
    /// other protocols that need to share the same socket (e.g., DTLS).
    /// Reads and writes performed directly on the returned socket bypass RTP
    /// parsing and all SRTP authentication/encryption enforced by the transport.
    /// Callers must not use this raw handle for media when SRTP is configured;
    /// media must continue through the authenticated transport APIs.
    pub async fn get_socket_handle(&self) -> Result<Arc<UdpSocket>> {
        // Try to get the socket from the UdpRtpTransport
        if let Some(t) = self.transport.as_any().downcast_ref::<UdpRtpTransport>() {
            // Clone and return the RTP socket using the public method
            let socket = t.get_socket();
            return Ok(socket);
        }

        // If we get here, the transport is not UdpRtpTransport
        Err(Error::Transport(
            "Transport is not a UDP transport".to_string(),
        ))
    }
}

impl Drop for RtpSession {
    fn drop(&mut self) {
        // Cancellation may drop a session before the async `close` path can be
        // awaited. JoinHandle::abort is synchronous; dropping the transport
        // then aborts its UDP receive tasks as a second layer.
        if let Some(handle) = self.recv_task.take() {
            handle.abort();
        }
        self.packet_sender.close();
        if let Some(handle) = self.rtcp_task.take() {
            handle.abort();
        }
        self.active = false;
    }
}

/// A lightweight sender handle for an RTP session
///
/// This handle can be used to send RTP packets to the session
/// from another thread without having to clone the entire session.
#[derive(Clone)]
#[allow(dead_code)] // retained (liveness/Drop hold or reserved); not read
pub struct RtpSessionSender {
    /// Canonical ordered packet writer for this session.
    packet_sender: Arc<RtpPacketSender>,

    /// Payload type
    payload_type: u8,

    /// Clock rate for the payload type
    #[allow(dead_code)] // retained (liveness/Drop hold or reserved); not read
    clock_rate: u32,
}

impl RtpSessionSender {
    /// Send an RTP packet with payload
    pub async fn send_packet(
        &self,
        timestamp: RtpTimestamp,
        payload: Bytes,
        marker: bool,
    ) -> Result<()> {
        self.packet_sender
            .send_payload(timestamp, payload, marker, self.payload_type)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn next_packet_event(events: &mut broadcast::Receiver<RtpSessionEvent>) -> RtpPacket {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let RtpSessionEvent::PacketReceived(packet) = events.recv().await.unwrap() {
                    return packet;
                }
            }
        })
        .await
        .expect("timed out waiting for RTP packet event")
    }

    async fn send_raw_rtp(
        peer: &UdpSocket,
        destination: SocketAddr,
        sequence_number: u16,
        timestamp: u32,
        ssrc: u32,
    ) {
        let packet = RtpPacket::new(
            RtpHeader::new(96, sequence_number, timestamp, ssrc),
            Bytes::from_static(b"media"),
        );
        peer.send_to(&packet.serialize().unwrap(), destination)
            .await
            .unwrap();
    }

    fn parse_receiver_report(data: &[u8]) -> crate::packet::rtcp::RtcpReceiverReport {
        match crate::packet::rtcp::RtcpPacket::parse(data).unwrap() {
            crate::packet::rtcp::RtcpPacket::ReceiverReport(report) => report,
            packet => panic!("expected receiver report, got {packet:?}"),
        }
    }

    #[test]
    fn default_session_buffer_config_preserves_channel_capacities() {
        let config = RtpSessionConfig::default();

        assert_eq!(
            config.session_buffer_config.sender_channel_capacity,
            RTP_SESSION_CHANNEL_CAPACITY
        );
        assert_eq!(
            config.session_buffer_config.receiver_channel_capacity,
            RTP_SESSION_RECEIVE_QUEUE_CAPACITY
        );
        assert_eq!(
            config.session_buffer_config.event_channel_capacity,
            RTP_SESSION_CHANNEL_CAPACITY
        );
        assert_eq!(
            config.transport_buffer_config,
            RtpTransportBufferConfig::default()
        );
    }

    #[test]
    fn sender_report_totals_wait_for_a_complete_concurrent_update() {
        let stats = Arc::new(parking_lot::Mutex::new(RtpSessionStats::default()));
        let sender_octets = Arc::new(AtomicU64::new(0));
        let (packet_count_updated_tx, packet_count_updated_rx) = std::sync::mpsc::channel();
        let (finish_update_tx, finish_update_rx) = std::sync::mpsc::channel();

        let update_stats = stats.clone();
        let update_octets = sender_octets.clone();
        let updater = std::thread::spawn(move || {
            let mut stats = update_stats.lock();
            stats.packets_sent = 1;
            packet_count_updated_tx.send(()).unwrap();
            finish_update_rx.recv().unwrap();
            update_octets.store(5, Ordering::Relaxed);
        });

        packet_count_updated_rx.recv().unwrap();
        let (snapshot_started_tx, snapshot_started_rx) = std::sync::mpsc::channel();
        let snapshot_stats = stats.clone();
        let snapshot_octets = sender_octets.clone();
        let snapshot = std::thread::spawn(move || {
            snapshot_started_tx.send(()).unwrap();
            sender_report_totals(&snapshot_stats, &snapshot_octets)
        });

        snapshot_started_rx.recv().unwrap();
        finish_update_tx.send(()).unwrap();

        updater.join().unwrap();
        assert_eq!(snapshot.join().unwrap(), (1, 5));
    }

    #[tokio::test]
    async fn send_handles_share_one_ordered_writer_and_close_fence() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = peer.local_addr().unwrap();
        let mut config = RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer_addr),
            ssrc: Some(0x1020_3040),
            payload_type: 0,
            ..RtpSessionConfig::default()
        };
        config.session_buffer_config.sender_channel_capacity = 3;

        let mut session = RtpSession::new(config).await.unwrap();
        let handle = session.send_handle().unwrap();
        let second_handle = handle.clone();

        handle
            .send_packet(160, Bytes::from_static(&[0x11; 160]), true)
            .await
            .unwrap();
        second_handle
            .send_packet_with_pt(320, Bytes::from_static(&[0x22; 4]), false, 101)
            .await
            .unwrap();

        let mut buffer = [0u8; 2048];
        let (first_len, _) =
            tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        let first = RtpPacket::parse(&buffer[..first_len]).unwrap();
        let (second_len, _) =
            tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        let second = RtpPacket::parse(&buffer[..second_len]).unwrap();

        assert_eq!(first.header.ssrc, 0x1020_3040);
        assert_eq!(first.header.payload_type, 0);
        assert!(first.header.marker);
        assert_eq!(second.header.payload_type, 101);
        assert_eq!(
            second.header.sequence_number,
            first.header.sequence_number.wrapping_add(1)
        );
        assert_eq!(session.get_stats().packets_sent, 2);
        assert_eq!(session.queue_diagnostics().sender_capacity_packets, 3);
        assert_eq!(session.queue_diagnostics().sender_queue_packets, 0);

        session.close().await.unwrap();
        let error = handle
            .send_packet(480, Bytes::from_static(&[0x33; 4]), false)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::SessionError(_)));
    }

    #[tokio::test]
    async fn live_session_tracks_wrap_reordering_and_duplicates_without_buffering() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_ssrc = 0x0102_0304;
        let remote_ssrc = 0xa1a2_a3a4;
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: Some(local_ssrc),
            // The default requests a jitter buffer. Production receive
            // tracking intentionally remains unbuffered in this repair.
            enable_jitter_buffer: true,
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let destination = session.local_addr().unwrap();
        let mut events = session.subscribe();

        for (sequence, timestamp) in [(65535, 0), (1, 320), (1, 320), (0, 160)] {
            send_raw_rtp(&peer, destination, sequence, timestamp, remote_ssrc).await;
        }

        let mut received_sequences = Vec::new();
        for _ in 0..4 {
            let packet = next_packet_event(&mut events).await;
            assert_eq!(packet.header.ssrc, remote_ssrc);
            received_sequences.push(packet.header.sequence_number);
        }
        assert_eq!(received_sequences, vec![65535, 1, 1, 0]);

        let stream = session.get_stream(remote_ssrc).await.unwrap();
        assert_eq!(stream.highest_seq, 65_537);
        assert_eq!(stream.packets_lost, 0);
        assert_eq!(stream.duplicates, 1);
        assert_eq!(stream.packets_out_of_order, 1);

        let stats = session.get_stats();
        assert_eq!(stats.packets_received, 4);
        assert_eq!(stats.packets_lost, 0);
        assert_eq!(stats.packets_duplicated, 1);
        assert_eq!(stats.packets_out_of_order, 1);
    }

    #[tokio::test]
    async fn precreated_stream_delivers_sequential_packets_promptly_and_in_order() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let remote_ssrc = 0xb1b2_b3b4;
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            // Exercise the configuration that previously gave manually
            // precreated streams a broken legacy jitter buffer.
            enable_jitter_buffer: true,
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let destination = session.local_addr().unwrap();
        let mut events = session.subscribe();

        assert!(session.create_stream_for_ssrc(remote_ssrc).await);
        send_raw_rtp(&peer, destination, 100, 16_000, remote_ssrc).await;
        send_raw_rtp(&peer, destination, 101, 16_160, remote_ssrc).await;

        let first = next_packet_event(&mut events).await;
        let second = next_packet_event(&mut events).await;
        assert_eq!(first.header.sequence_number, 100);
        assert_eq!(second.header.sequence_number, 101);

        let stream = session.get_stream(remote_ssrc).await.unwrap();
        assert_eq!(stream.packets_received, 2);
        assert_eq!(stream.highest_seq, 101);
    }

    #[tokio::test]
    async fn precreated_stream_reconciles_sender_report_during_insert_handoff() {
        use crate::packet::rtcp::{NtpTimestamp, RtcpCompoundPacket, RtcpSenderReport};

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        session.set_remote_addr(peer.local_addr().unwrap()).await;
        let destination = session.local_addr().unwrap();
        let remote_ssrc = 0xc1c2_c3c4;
        let mut events = session.subscribe();

        let mut sender_report = RtcpSenderReport::new(remote_ssrc);
        sender_report.ntp_timestamp = NtpTimestamp {
            seconds: 0xaaaa_1234,
            fraction: 0x5678_bbbb,
        };
        let sender_report_wire = RtcpCompoundPacket::new_with_sr(sender_report)
            .serialize()
            .unwrap();

        // Pause production precreation immediately before insertion. The
        // receive task must retain the SR and observe that no stream exists;
        // precreation then inserts the stream and reconciles that retained SR.
        let report_during_handoff = async {
            peer.send_to(&sender_report_wire, destination)
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if matches!(
                        events.recv().await.unwrap(),
                        RtpSessionEvent::RtcpSenderReport { ssrc, .. } if ssrc == remote_ssrc
                    ) {
                        break;
                    }
                }
            })
            .await
            .expect("sender report was not processed during precreation");
        };
        assert!(
            session
                .create_stream_for_ssrc_after(remote_ssrc, report_during_handoff)
                .await
        );

        tokio::time::sleep(Duration::from_millis(10)).await;
        session.send_receiver_report().await.unwrap();
        let mut buffer = [0u8; 2048];
        let (size, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let report = parse_receiver_report(&buffer[..size]);
        assert_eq!(report.report_blocks.len(), 1);
        assert_eq!(report.report_blocks[0].ssrc, remote_ssrc);
        assert_eq!(report.report_blocks[0].last_sr, 0x1234_5678);
        assert!(report.report_blocks[0].delay_since_last_sr > 0);
    }

    #[tokio::test]
    async fn plain_udp_session_preserves_rtp_padding_and_remote_ssrc() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: Some(0x1111_1111),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let mut events = session.subscribe();

        let mut header = RtpHeader::new(96, 7, 960, 0x2222_2222);
        header.padding = true;
        let packet = RtpPacket {
            header,
            payload: Bytes::from_static(b"padded payload"),
            padding_size: 4,
        };
        let wire = packet.serialize().unwrap();
        peer.send_to(&wire, session.local_addr().unwrap())
            .await
            .unwrap();

        let received = next_packet_event(&mut events).await;
        assert_eq!(received.header.ssrc, 0x2222_2222);
        assert!(received.header.padding);
        assert_eq!(received.padding_size, 4);
        assert_eq!(received.payload, Bytes::from_static(b"padded payload"));
        assert_eq!(received.serialize().unwrap(), wire);
        assert_eq!(session.get_stats().bytes_received, wire.len() as u64);
    }

    #[tokio::test]
    async fn srtp_session_preserves_encrypted_rtp_padding() {
        use crate::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};

        fn context() -> SrtpContext {
            SrtpContext::new(
                SRTP_AES128_CM_SHA1_80,
                SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]),
            )
            .unwrap()
        }

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let transport = session.transport();
        let udp = transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .unwrap();
        udp.set_srtp_contexts(context(), context()).await.unwrap();
        let mut events = session.subscribe();

        let mut header = RtpHeader::new(96, 19, 3_040, 0x3333_3333);
        header.padding = true;
        let packet = RtpPacket {
            header,
            payload: Bytes::from_static(b"secret padded payload"),
            padding_size: 8,
        };
        let mut sender = context();
        let protected = sender.protect(&packet).unwrap().serialize().unwrap();
        peer.send_to(&protected, session.local_addr().unwrap())
            .await
            .unwrap();

        let received = next_packet_event(&mut events).await;
        assert_eq!(received.header.ssrc, 0x3333_3333);
        assert!(received.header.padding);
        assert_eq!(received.padding_size, 8);
        assert_eq!(received.payload, packet.payload);
        assert_eq!(received.serialize().unwrap(), packet.serialize().unwrap());
    }

    #[tokio::test]
    async fn production_rtcp_ingress_processes_members_around_unknown_packet() {
        use crate::packet::rtcp::{
            RtcpCompoundMember, RtcpGoodbye, RtcpPacket, RtcpReceiverReport, RtcpReportBlock,
            RtcpTolerantCompoundPacket, RtcpUnknownPacket,
        };

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: Some(0x4444_4444),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let mut events = session.subscribe();
        let remote_ssrc = 0x5555_5555;
        let mut receiver_report = RtcpReceiverReport::new(remote_ssrc);
        let mut outbound_loss = RtcpReportBlock::new(0x4444_4444);
        outbound_loss.cumulative_lost = 99;
        receiver_report.add_report_block(outbound_loss);
        let compound = RtcpTolerantCompoundPacket {
            packets: vec![
                RtcpCompoundMember::Known(RtcpPacket::ReceiverReport(receiver_report)),
                RtcpCompoundMember::Unknown(RtcpUnknownPacket {
                    packet_type: 205,
                    count: 1,
                    payload: Bytes::from_static(&[0xaa, 0xbb, 0xcc, 0xdd]),
                    padding: Bytes::new(),
                }),
                RtcpCompoundMember::Known(RtcpPacket::Goodbye(RtcpGoodbye::new_for_source(
                    remote_ssrc,
                ))),
            ],
        };
        peer.send_to(
            &compound.serialize().unwrap(),
            session.local_addr().unwrap(),
        )
        .await
        .unwrap();

        let (mut saw_report, mut saw_bye) = (false, false);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !(saw_report && saw_bye) {
                match events.recv().await.unwrap() {
                    RtpSessionEvent::RtcpReceiverReport { ssrc, .. } => {
                        assert_eq!(ssrc, remote_ssrc);
                        saw_report = true;
                    }
                    RtpSessionEvent::Bye { ssrc, .. } => {
                        assert_eq!(ssrc, remote_ssrc);
                        saw_bye = true;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("known RTCP members after an unknown member were not processed");
        assert_eq!(session.get_stats().packets_lost, 0);
    }

    #[tokio::test]
    async fn malformed_compound_rtcp_is_rejected_atomically_in_production() {
        async fn assert_no_event(data: &[u8]) {
            let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let session = RtpSession::new(RtpSessionConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                ..RtpSessionConfig::default()
            })
            .await
            .unwrap();
            let mut events = session.subscribe();
            peer.send_to(data, session.local_addr().unwrap())
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), events.recv())
                    .await
                    .is_err()
            );
        }

        let rr = [0x80, 201, 0, 1, 0x12, 0x34, 0x56, 0x78];

        let mut bad_trailing_version = rr.to_vec();
        bad_trailing_version.extend_from_slice(&[0x40, 205, 0, 1, 0, 0, 0, 0]);
        assert_no_event(&bad_trailing_version).await;

        let declared_overrun = [0x80, 201, 0, 10, 0x12, 0x34, 0x56, 0x78];
        assert_no_event(&declared_overrun).await;

        let mut non_final_padding = rr.to_vec();
        non_final_padding.extend_from_slice(&[0xa0, 205, 0, 1, 0, 0, 0, 4]);
        non_final_padding.extend_from_slice(&[0x80, 203, 0, 1, 0, 0, 0, 1]);
        assert_no_event(&non_final_padding).await;
    }

    #[tokio::test]
    async fn manual_reports_use_interval_loss_and_retain_cumulative_loss() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = peer.local_addr().unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        // Configure the destination after construction so this manual-report
        // test has no periodic RTCP task racing its assertions.
        session.set_remote_addr(peer_addr).await;
        let destination = session.local_addr().unwrap();
        let remote_ssrc = 0x6666_6666;
        let mut events = session.subscribe();

        for (sequence, timestamp) in [(10, 0), (12, 320)] {
            send_raw_rtp(&peer, destination, sequence, timestamp, remote_ssrc).await;
            next_packet_event(&mut events).await;
        }
        session.send_receiver_report().await.unwrap();
        let mut buffer = [0u8; 2048];
        let (size, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let first = parse_receiver_report(&buffer[..size]);
        assert_eq!(first.report_blocks[0].fraction_lost, 85);
        assert_eq!(first.report_blocks[0].cumulative_lost, 1);

        for (offset, sequence) in (13..=20).enumerate() {
            send_raw_rtp(
                &peer,
                destination,
                sequence,
                480 + offset as u32 * 160,
                remote_ssrc,
            )
            .await;
            next_packet_event(&mut events).await;
        }
        session.send_receiver_report().await.unwrap();
        let (size, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let second = parse_receiver_report(&buffer[..size]);
        assert_eq!(second.report_blocks[0].fraction_lost, 0);
        assert_eq!(second.report_blocks[0].cumulative_lost, 1);
    }

    #[tokio::test]
    async fn sr_before_rtp_populates_manual_and_periodic_lsr_dlsr() {
        use crate::packet::rtcp::{
            NtpTimestamp, RtcpCompoundMember, RtcpCompoundPacket, RtcpPacket, RtcpSenderReport,
        };

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = peer.local_addr().unwrap();
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer_addr),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let destination = session.local_addr().unwrap();
        let remote_ssrc = 0x7777_7777;
        let mut events = session.subscribe();

        let mut sender_report = RtcpSenderReport::new(remote_ssrc);
        sender_report.ntp_timestamp = NtpTimestamp {
            seconds: 0xaaaa_1234,
            fraction: 0x5678_bbbb,
        };
        let sender_report_wire = RtcpCompoundPacket::new_with_sr(sender_report)
            .serialize()
            .unwrap();
        peer.send_to(&sender_report_wire, destination)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if matches!(
                    events.recv().await.unwrap(),
                    RtpSessionEvent::RtcpSenderReport { ssrc, .. } if ssrc == remote_ssrc
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();

        send_raw_rtp(&peer, destination, 1, 160, remote_ssrc).await;
        next_packet_event(&mut events).await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        session.send_receiver_report().await.unwrap();
        let mut buffer = [0u8; 2048];
        let (size, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let manual = parse_receiver_report(&buffer[..size]);
        assert_eq!(manual.report_blocks[0].last_sr, 0x1234_5678);
        assert!(manual.report_blocks[0].delay_since_last_sr > 0);

        let (size, _) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut buffer))
            .await
            .expect("periodic RTCP report was not sent")
            .unwrap();
        let periodic = RtcpCompoundPacket::parse_tolerant(&buffer[..size]).unwrap();
        let periodic_sr = periodic
            .packets
            .iter()
            .find_map(|member| match member {
                RtcpCompoundMember::Known(RtcpPacket::SenderReport(report)) => Some(report),
                _ => None,
            })
            .expect("periodic compound packet did not contain a sender report");
        assert_eq!(periodic_sr.report_blocks[0].last_sr, 0x1234_5678);
        assert!(periodic_sr.report_blocks[0].delay_since_last_sr > 0);
    }

    #[tokio::test]
    async fn sender_report_octet_count_excludes_rtp_header() {
        use crate::packet::rtcp::{RtcpPacket, RtcpSenderReport};

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        session.set_remote_addr(peer.local_addr().unwrap()).await;
        session
            .send_packet(160, Bytes::from_static(b"12345"), false)
            .await
            .unwrap();

        let mut buffer = [0u8; 2048];
        tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        session.send_sender_report().await.unwrap();
        let (size, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let report: RtcpSenderReport = match RtcpPacket::parse(&buffer[..size]).unwrap() {
            RtcpPacket::SenderReport(report) => report,
            packet => panic!("expected sender report, got {packet:?}"),
        };
        assert_eq!(report.sender_packet_count, 1);
        assert_eq!(report.sender_octet_count, 5);
    }

    #[tokio::test]
    async fn srtp_session_rejects_manual_and_automatic_plaintext_rtcp() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let config = RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ssrc: Some(0x1020_3040),
            payload_type: 0,
            ..RtpSessionConfig::default()
        };
        let mut session = RtpSession::new(config).await.unwrap();
        let transport = session.transport();
        let udp = transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .unwrap();
        let key = vec![0x11; 16];
        let salt = vec![0x22; 14];
        let send = crate::srtp::SrtpContext::new(
            crate::srtp::SRTP_AES128_CM_SHA1_80,
            crate::srtp::SrtpCryptoKey::new(key.clone(), salt.clone()),
        )
        .unwrap();
        let recv = crate::srtp::SrtpContext::new(
            crate::srtp::SRTP_AES128_CM_SHA1_80,
            crate::srtp::SrtpCryptoKey::new(key, salt),
        )
        .unwrap();
        udp.set_srtp_contexts(send, recv).await.unwrap();

        assert!(matches!(
            session.send_sender_report().await,
            Err(Error::UnsupportedFeature(_))
        ));
        assert!(matches!(
            session.send_receiver_report().await,
            Err(Error::UnsupportedFeature(_))
        ));

        let mut wire = [0_u8; 2048];
        assert!(
            tokio::time::timeout(Duration::from_millis(1_200), peer.recv_from(&mut wire))
                .await
                .is_err()
        );
        session.close().await.unwrap();
    }
}
