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
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering};
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

fn compact_ntp_rtt_ms(last_sender_report: u32, delay_since_last_report: u32) -> Option<f64> {
    if last_sender_report == 0 {
        return None;
    }
    let now = crate::packet::rtcp::NtpTimestamp::now().to_u32();
    let elapsed = now
        .wrapping_sub(last_sender_report)
        .wrapping_sub(delay_since_last_report);
    // A negative 16.16 fixed-point interval appears with the high bit set.
    // Ignore it rather than publishing a wrap-sized RTT.
    if elapsed & 0x8000_0000 != 0 {
        return None;
    }
    Some(f64::from(elapsed) * 1_000.0 / 65_536.0)
}

/// RTP timestamp of "now" derived from the wall clock. Only used when a
/// sender report must be produced before any media packet anchored the
/// session's RTP clock.
fn wallclock_rtp_timestamp(clock_rate: u32) -> RtpTimestamp {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = since_epoch.as_secs() * u64::from(clock_rate)
        + u64::from(since_epoch.subsec_nanos()) * u64::from(clock_rate) / 1_000_000_000;
    ticks as RtpTimestamp
}

/// A fresh RFC 7022 §4.2 short-term persistent CNAME: 96 random bits,
/// base64-encoded. It identifies the session without revealing the local
/// user or host name, and stays fixed for the session's lifetime.
fn random_cname() -> String {
    use base64::Engine;
    let bytes: [u8; 12] = rand::thread_rng().gen();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// SSRC of the first packet in a compound RTCP datagram: the report or
/// description's sender.
fn rtcp_sender_ssrc(data: &[u8]) -> Option<RtpSsrc> {
    let bytes: [u8; 4] = data.get(4..8)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

/// Where this session's RTCP goes: the transport's (possibly latched) peer,
/// falling back to the signalled address.
async fn rtcp_destination(
    transport: &Arc<dyn RtpTransport>,
    packet_sender: &RtpPacketSender,
) -> Option<SocketAddr> {
    if let Some(udp) = transport.as_any().downcast_ref::<UdpRtpTransport>() {
        udp.remote_rtcp_addr()
            .await
            .or_else(|| *packet_sender.remote_addr.read())
    } else {
        *packet_sender.remote_addr.read()
    }
}

/// Where periodic reports and the close-time BYE go, or `None` when they
/// may not be sent.
///
/// With multiplexing negotiated they share the RTP socket and go to the
/// peer's (latched) RTP address. Without it they go only from a separate
/// RTCP socket to the peer's RTCP address, and never to an address the RTP
/// stream uses: RFC 5761 §5.1.1 forbids multiplexing a peer did not agree
/// to. A session without a separate socket sends nothing until mux is
/// negotiated.
async fn rtcp_report_destination(
    transport: &Arc<dyn RtpTransport>,
    packet_sender: &RtpPacketSender,
    rtcp_mux: bool,
) -> Option<SocketAddr> {
    if rtcp_mux {
        return rtcp_destination(transport, packet_sender).await;
    }
    let udp = transport.as_any().downcast_ref::<UdpRtpTransport>()?;
    if udp.rtcp_mux() || udp.local_rtcp_socket_addr().is_none() {
        return None;
    }
    let destination = udp.remote_rtcp_addr().await?;
    if Some(destination) == udp.remote_rtp_addr().await
        || Some(destination) == *packet_sender.remote_addr.read()
    {
        return None;
    }
    Some(destination)
}

/// Whether inbound RTCP from `source` belongs to this session.
///
/// Only the session's expected peer may feed its reports, statistics, and
/// BYE handling: the (latched or signalled) remote address, or a sender SSRC
/// this session already receives RTP from. With consecutive port allocation
/// a peer of a neighbouring call that does not multiplex sends its RTCP to
/// its RTP port plus one, which is this session's RTP port; that traffic
/// matches neither and is dropped.
async fn rtcp_source_is_expected(
    transport: &Arc<dyn RtpTransport>,
    packet_sender: &RtpPacketSender,
    streams: &DashMap<RtpSsrc, RtpStream>,
    source: SocketAddr,
    data: &[u8],
) -> bool {
    if *packet_sender.remote_addr.read() == Some(source) {
        return true;
    }
    if let Some(udp) = transport.as_any().downcast_ref::<UdpRtpTransport>() {
        if udp.remote_rtp_addr().await == Some(source)
            || udp.remote_rtcp_addr().await == Some(source)
        {
            return true;
        }
    }
    rtcp_sender_ssrc(data).is_some_and(|ssrc| streams.contains_key(&ssrc))
}

fn build_voip_metrics_xr(
    sender_ssrc: RtpSsrc,
    report_blocks: &[crate::packet::rtcp::RtcpReportBlock],
    stats: &RtpSessionStats,
    clock_rate: u32,
) -> Option<crate::packet::rtcp::RtcpExtendedReport> {
    if report_blocks.is_empty() {
        return None;
    }
    let mut xr = crate::packet::rtcp::RtcpExtendedReport::new(sender_ssrc);
    let discard_rate = if stats.packets_received == 0 {
        0
    } else {
        ((stats.packets_discarded_by_jitter.saturating_mul(256) / stats.packets_received).min(255))
            as u8
    };
    let rtt_ms = stats.rtt_ms.unwrap_or(0.0).clamp(0.0, f64::from(u16::MAX)) as u16;
    for block in report_blocks {
        let jitter_ms = if clock_rate == 0 {
            0.0
        } else {
            block.jitter as f32 * 1_000.0 / clock_rate as f32
        };
        let loss_percent = block.fraction_lost as f32 * 100.0 / 256.0;
        let mut metrics = crate::packet::rtcp::VoipMetricsBlock::new(block.ssrc);
        metrics.loss_rate = block.fraction_lost;
        metrics.discard_rate = discard_rate;
        metrics.round_trip_delay = rtt_ms;
        metrics.calculate_r_factor(loss_percent, rtt_ms, jitter_ms);
        xr.add_voip_metrics(metrics);
    }
    Some(xr)
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

    /// Compound RTCP packets accepted from this session's expected peer.
    pub rtcp_packets_received: u64,

    /// Compound RTCP packets dropped because they came from neither the
    /// session's expected remote address nor a remote SSRC it receives RTP
    /// from — for example a neighbouring call's peer sending RTCP to its RTP
    /// port plus one.
    pub rtcp_packets_rejected: u64,

    /// Current jitter estimate (in milliseconds)
    pub jitter_ms: f64,

    /// Most recently measured round-trip time from an RTCP report block.
    /// `None` until the peer reflects one of this session's sender reports.
    pub rtt_ms: Option<f64>,

    /// Remote address of the most recent packet
    pub remote_addr: Option<SocketAddr>,

    /// The most recent RTCP reception report a peer sent about *this*
    /// session's outbound stream (an SR or RR report block whose SSRC is
    /// ours). `None` until such a block arrives — in particular, always
    /// `None` when RTCP never reaches this session (for example when
    /// rtcp-mux was not negotiated and nothing listens on the RTCP port).
    ///
    /// When several remote sources report on us, the most recently received
    /// block wins; [`PeerReceptionReport::reporter_ssrc`] says which source
    /// sent it. Blocks about other SSRCs are ignored.
    pub peer_report: Option<PeerReceptionReport>,

    /// Set when the peer announced it is leaving with an RTCP BYE.
    pub peer_bye: Option<PeerRtcpBye>,
}

/// One RTCP reception report block (RFC 3550 §6.4.1) that a remote source
/// sent about this session's outbound stream, as retained by
/// [`RtpSessionStats::peer_report`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeerReceptionReport {
    /// SSRC of the remote source that sent the SR/RR carrying this block.
    pub reporter_ssrc: RtpSsrc,
    /// Fraction of our packets the peer lost since its previous report,
    /// as the raw RFC 3550 8-bit fixed-point value (`fraction / 256`).
    pub fraction_lost: u8,
    /// Cumulative number of our packets the peer lost, sign-extended from
    /// the 24-bit wire field. Negative when duplicates outnumber losses.
    pub cumulative_lost: i32,
    /// Extended highest sequence number the peer received from us.
    pub extended_highest_sequence: u32,
    /// Interarrival jitter the peer measured on our stream, in RTP
    /// timestamp units.
    pub jitter: u32,
    /// [`Self::jitter`] converted to milliseconds with the session's RTP
    /// clock rate at the time the report arrived.
    pub jitter_ms: f64,
    /// Round-trip time computed from this block's LSR/DLSR, when the peer
    /// reflected one of our sender reports (`None` when LSR is zero).
    pub rtt_ms: Option<f64>,
    /// When this block was received.
    pub received_at: Instant,
}

impl PeerReceptionReport {
    /// Build the retained view of a report block about our SSRC.
    pub fn from_report_block(
        reporter_ssrc: RtpSsrc,
        block: &crate::packet::rtcp::RtcpReportBlock,
        clock_rate: u32,
        received_at: Instant,
    ) -> Self {
        // The wire field is a signed 24-bit integer.
        let cumulative_lost = (((block.cumulative_lost & 0x00ff_ffff) << 8) as i32) >> 8;
        let jitter_ms = if clock_rate == 0 {
            0.0
        } else {
            f64::from(block.jitter) * 1_000.0 / f64::from(clock_rate)
        };
        Self {
            reporter_ssrc,
            fraction_lost: block.fraction_lost,
            cumulative_lost,
            extended_highest_sequence: block.highest_seq,
            jitter: block.jitter,
            jitter_ms,
            rtt_ms: compact_ntp_rtt_ms(block.last_sr, block.delay_since_last_sr),
            received_at,
        }
    }

    /// [`Self::fraction_lost`] as a fraction in `0.0..=1.0`.
    pub fn fraction_lost_ratio(&self) -> f64 {
        f64::from(self.fraction_lost) / 256.0
    }
}

/// An RTCP BYE received from a remote source, retained by
/// [`RtpSessionStats::peer_bye`].
#[derive(Debug, Clone, PartialEq)]
pub struct PeerRtcpBye {
    /// First SSRC listed in the BYE.
    pub ssrc: RtpSsrc,
    /// Optional reason text carried by the BYE.
    pub reason: Option<String>,
    /// When the BYE was received.
    pub received_at: Instant,
}

/// Retain a report block about our own SSRC and its RTT measurement.
fn record_peer_reception_report(
    stats: &parking_lot::Mutex<RtpSessionStats>,
    reporter_ssrc: RtpSsrc,
    block: &crate::packet::rtcp::RtcpReportBlock,
    clock_rate: u32,
) {
    let report =
        PeerReceptionReport::from_report_block(reporter_ssrc, block, clock_rate, Instant::now());
    let mut stats = stats.lock();
    if let Some(rtt_ms) = report.rtt_ms {
        stats.rtt_ms = Some(rtt_ms);
    }
    stats.peer_report = Some(report);
}

/// Cadence for RFC 3611 VoIP-metrics reports emitted by an RTP session.
///
/// The report rides the existing compound RTCP schedule, so it does not add a
/// timer or a detached task per call. A value of one emits XR beside every
/// regular report; larger values reduce reporting bandwidth deterministically.
///
/// XR is off unless asked for: a session built with
/// [`RtpSession::new_event_driven_with_quality_reporting`] starts with it on,
/// and [`RtpSession::set_rtcp_xr_enabled`] switches it per session, for
/// example once SDP shows the peer wants `a=rtcp-xr` (RFC 3611 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcpXrQualityConfig {
    pub every_n_rtcp_reports: NonZeroU32,
}

impl Default for RtcpXrQualityConfig {
    fn default() -> Self {
        Self {
            every_n_rtcp_reports: NonZeroU32::new(1).expect("one is non-zero"),
        }
    }
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
    /// The session's media payload type. Only media (and comfort noise)
    /// timestamps advance with the sampling clock; RFC 4733 events hold
    /// their start timestamp for the whole tone.
    media_payload_type: AtomicU8,
    /// Live RTP clock rate of the media stream.
    clock_rate: Arc<AtomicU32>,
    /// RTP timestamp and wall time of the last media packet actually sent,
    /// the anchor a sender report extrapolates from (RFC 3550 §6.4.1).
    send_clock: parking_lot::Mutex<Option<SendClockAnchor>>,
}

#[derive(Debug, Clone, Copy)]
struct SendClockAnchor {
    rtp_timestamp: RtpTimestamp,
    sent_at: Instant,
    clock_rate: u32,
}

/// Static comfort-noise payload type (RFC 3389); it shares the media clock.
const COMFORT_NOISE_PAYLOAD_TYPE: u8 = 13;

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
        media_payload_type: u8,
        clock_rate: Arc<AtomicU32>,
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
            media_payload_type: AtomicU8::new(media_payload_type),
            clock_rate,
            send_clock: parking_lot::Mutex::new(None),
        }
    }

    /// The RTP timestamp that corresponds to wall time `at` on this session's
    /// media clock: the last sent media timestamp advanced by the elapsed
    /// time at the stream clock rate (RFC 3550 §6.4.1). `None` until a media
    /// packet has been sent.
    fn sr_rtp_timestamp(&self, at: Instant) -> Option<RtpTimestamp> {
        let anchor = (*self.send_clock.lock())?;
        let elapsed = at.saturating_duration_since(anchor.sent_at);
        let ticks = (elapsed.as_secs_f64() * f64::from(anchor.clock_rate)).round() as u64;
        Some(anchor.rtp_timestamp.wrapping_add(ticks as u32))
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
                if payload_type == self.media_payload_type.load(Ordering::Relaxed)
                    || payload_type == COMFORT_NOISE_PAYLOAD_TYPE
                {
                    *self.send_clock.lock() = Some(SendClockAnchor {
                        rtp_timestamp: timestamp,
                        sent_at: Instant::now(),
                        clock_rate: self.clock_rate.load(Ordering::Relaxed),
                    });
                }
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

/// Upper bound on remote SSRCs remembered from RTCP alone, for the
/// member count in the report interval.
const MAX_RTCP_ONLY_MEMBERS: usize = 64;

/// RTCP state shared by the periodic report task, manual reports, and the
/// close-time BYE: interval timing, sender state, CNAME, and XR policy.
struct RtcpReporter {
    ssrc: RtpSsrc,
    generator: parking_lot::Mutex<crate::stats::reports::RtcpReportGenerator>,
    stats: Arc<parking_lot::Mutex<RtpSessionStats>>,
    sender_octets: Arc<AtomicU64>,
    streams: Arc<DashMap<RtpSsrc, RtpStream>>,
    packet_sender: Arc<RtpPacketSender>,
    clock_rate: Arc<AtomicU32>,
    /// Remote SSRCs seen only through accepted RTCP (receive-only peers).
    rtcp_members: DashMap<RtpSsrc, ()>,
    xr_enabled: AtomicBool,
    xr_quality: RtcpXrQualityConfig,
    periodic_reports: AtomicU64,
    /// Wakes the report task to redraw a pending interval after the
    /// bandwidth or minimum-interval policy changed.
    reschedule: tokio::sync::Notify,
}

impl RtcpReporter {
    /// Refresh sender totals and session membership, then draw the next
    /// RFC 3550 report interval.
    fn next_interval(&self) -> Duration {
        let (packets, octets) = sender_report_totals(&self.stats, &self.sender_octets);
        let remote_senders = self.streams.len() as u32;
        let rtcp_only = self
            .rtcp_members
            .iter()
            .filter(|entry| !self.streams.contains_key(entry.key()))
            .count() as u32;
        let mut generator = self.generator.lock();
        generator.set_sent_totals(packets, octets);
        let senders = remote_senders + u32::from(generator.we_sent());
        let members = 1 + remote_senders + rtcp_only;
        generator.update_members(senders, members - senders);
        generator.calculate_interval()
    }

    /// Note an accepted inbound compound RTCP datagram.
    fn observe_received(&self, data: &[u8]) {
        self.generator.lock().on_report_received(data.len());
        if let Some(ssrc) = rtcp_sender_ssrc(data) {
            if ssrc != self.ssrc && self.rtcp_members.len() < MAX_RTCP_ONLY_MEMBERS {
                self.rtcp_members.insert(ssrc, ());
            }
        }
    }

    /// Build one compound report: SR when this session sent RTP in the last
    /// two report intervals and RR otherwise (RFC 3550 §6.4), always with an
    /// SDES CNAME (§6.1), then XR when enabled on a periodic report, then an
    /// optional BYE last.
    fn build(
        &self,
        bye: Option<crate::packet::rtcp::RtcpGoodbye>,
        periodic: bool,
    ) -> crate::packet::rtcp::RtcpCompoundPacket {
        use crate::packet::rtcp::{
            NtpTimestamp, RtcpCompoundPacket, RtcpReceiverReport, RtcpSenderReport,
        };

        let (packets, octets) = sender_report_totals(&self.stats, &self.sender_octets);
        let report_blocks = take_rtcp_report_blocks(&self.streams);
        let mut generator = self.generator.lock();
        generator.set_sent_totals(packets, octets);
        let mut compound = if generator.we_sent() {
            // Capture the wall clock once so the NTP and RTP timestamps
            // describe the same instant.
            let at = Instant::now();
            let ntp_timestamp = NtpTimestamp::now();
            let rtp_timestamp = self.packet_sender.sr_rtp_timestamp(at).unwrap_or_else(|| {
                wallclock_rtp_timestamp(self.clock_rate.load(Ordering::Relaxed))
            });
            RtcpCompoundPacket::new_with_sr(RtcpSenderReport {
                ssrc: self.ssrc,
                ntp_timestamp,
                rtp_timestamp,
                sender_packet_count: packets,
                sender_octet_count: octets,
                report_blocks: report_blocks.clone(),
            })
        } else {
            RtcpCompoundPacket::new_with_rr(RtcpReceiverReport {
                ssrc: self.ssrc,
                report_blocks: report_blocks.clone(),
            })
        };
        compound.add_sdes(generator.generate_sdes());
        drop(generator);

        if periodic {
            let index = self
                .periodic_reports
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            if self.xr_enabled.load(Ordering::Relaxed)
                && index % u64::from(self.xr_quality.every_n_rtcp_reports.get()) == 0
            {
                let stats_snapshot = self.stats.lock().clone();
                if let Some(xr) = build_voip_metrics_xr(
                    self.ssrc,
                    &report_blocks,
                    &stats_snapshot,
                    self.clock_rate.load(Ordering::Relaxed),
                ) {
                    compound.add_xr(xr);
                }
            }
        }
        if let Some(bye) = bye {
            compound.add_bye(bye);
        }
        compound
    }

    /// Serialize and send a compound report, recording it for the interval
    /// and sender-state calculation when it leaves.
    async fn send(
        &self,
        transport: &Arc<dyn RtpTransport>,
        compound: &crate::packet::rtcp::RtcpCompoundPacket,
        destination: SocketAddr,
    ) -> Result<()> {
        let data = compound.serialize()?;
        transport.send_rtcp_bytes(&data, destination).await?;
        self.generator.lock().on_report_sent(data.len());
        Ok(())
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

    /// Live RTP clock used when receive streams are created after a codec
    /// renegotiation.
    clock_rate: Arc<AtomicU32>,

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

    /// RTCP report state shared with the periodic report task.
    rtcp_reporter: Arc<RtcpReporter>,

    /// RTCP sender task
    rtcp_task: Option<JoinHandle<()>>,

    /// Whether [`Self::set_bandwidth`] fixed the session bandwidth; otherwise
    /// it follows the payload type.
    bandwidth_explicit: bool,

    /// Whether RTP/RTCP multiplexing is in force, so reports share the RTP
    /// port; see [`Self::set_rtcp_mux`].
    rtcp_mux: Arc<AtomicBool>,

    /// The peer's RTCP address from SDP `a=rtcp:` (RFC 3605), used instead
    /// of RTP port + 1 when reports leave from a separate RTCP socket.
    signalled_remote_rtcp_addr: parking_lot::Mutex<Option<SocketAddr>>,

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
        Self::new_with_receive_queue(config, true, SymmetricRtpPolicy::default(), None).await
    }

    /// Create a new RTP session with an explicit symmetric-RTP policy.
    pub async fn new_with_symmetric_rtp_policy(
        config: RtpSessionConfig,
        policy: SymmetricRtpPolicy,
    ) -> Result<Self> {
        Self::new_with_receive_queue(config, true, policy, None).await
    }

    /// Create a new RTP session for event-driven consumers.
    ///
    /// Packets are still emitted through [`RtpSessionEvent::PacketReceived`],
    /// but they are not duplicated into the polling queue used by
    /// [`RtpSession::receive_packet`].
    pub async fn new_event_driven(config: RtpSessionConfig) -> Result<Self> {
        Self::new_with_receive_queue(config, false, SymmetricRtpPolicy::default(), None).await
    }

    /// Create an event-driven RTP session with an explicit symmetric-RTP
    /// learning/rebinding policy.
    pub async fn new_event_driven_with_symmetric_rtp_policy(
        config: RtpSessionConfig,
        policy: SymmetricRtpPolicy,
    ) -> Result<Self> {
        Self::new_with_receive_queue(config, false, policy, None).await
    }

    /// Create an event-driven session with explicit symmetric-RTP and RTCP XR
    /// quality-reporting policy. RFC 3611 VoIP-metrics reports start enabled;
    /// [`Self::set_rtcp_xr_enabled`] can switch them off again.
    pub async fn new_event_driven_with_quality_reporting(
        config: RtpSessionConfig,
        policy: SymmetricRtpPolicy,
        xr_quality: RtcpXrQualityConfig,
    ) -> Result<Self> {
        Self::new_with_receive_queue(config, false, policy, Some(xr_quality)).await
    }

    /// Create an event-driven session that also binds a separate RTCP
    /// socket on `local_rtcp_addr`, for peers that do not multiplex RTP and
    /// RTCP (RFC 3550 §11 puts it on the RTP port + 1).
    ///
    /// Until [`Self::set_rtcp_mux`] reports negotiated multiplexing, periodic
    /// reports and the close-time BYE leave from this socket to the peer's
    /// RTCP address: [`Self::set_remote_rtcp_addr`] when SDP named one with
    /// `a=rtcp:` (RFC 3605), otherwise the peer's RTP port + 1. Reports
    /// arriving on the socket pass the same peer filter as multiplexed ones,
    /// and SRTCP protects both directions once SRTP contexts are installed.
    /// When multiplexing is negotiated instead, [`Self::release_rtcp_socket`]
    /// closes the socket so its port can be reused.
    pub async fn new_event_driven_with_rtcp_socket(
        config: RtpSessionConfig,
        policy: SymmetricRtpPolicy,
        local_rtcp_addr: SocketAddr,
    ) -> Result<Self> {
        Self::new_session(config, false, policy, None, Some(local_rtcp_addr)).await
    }

    async fn new_with_receive_queue(
        config: RtpSessionConfig,
        receive_queue_enabled: bool,
        symmetric_rtp_policy: SymmetricRtpPolicy,
        xr_quality: Option<RtcpXrQualityConfig>,
    ) -> Result<Self> {
        Self::new_session(
            config,
            receive_queue_enabled,
            symmetric_rtp_policy,
            xr_quality,
            None,
        )
        .await
    }

    async fn new_session(
        config: RtpSessionConfig,
        receive_queue_enabled: bool,
        symmetric_rtp_policy: SymmetricRtpPolicy,
        xr_quality: Option<RtcpXrQualityConfig>,
        local_rtcp_addr: Option<SocketAddr>,
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
            // A separate RTCP socket only when the caller reserved a port
            // for it; otherwise RTCP shares the RTP socket.
            local_rtcp_addr,
            symmetric_rtp: true,
            rtcp_mux: local_rtcp_addr.is_none(),
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
        let clock_rate = Arc::new(AtomicU32::new(config.clock_rate));
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
            config.payload_type,
            clock_rate.clone(),
        ));

        // RTCP reporting state. The CNAME is random per session (RFC 7022)
        // so reports never disclose the local user or host name.
        let mut rtcp_generator =
            crate::stats::reports::RtcpReportGenerator::new(ssrc, random_cname());
        rtcp_generator.set_bandwidth(
            crate::stats::reports::session_bandwidth_for_payload_type(config.payload_type)
                .unwrap_or(crate::stats::reports::DEFAULT_SESSION_BANDWIDTH_BPS),
        );
        let streams = Arc::new(DashMap::new());
        let rtcp_reporter = Arc::new(RtcpReporter {
            ssrc,
            generator: parking_lot::Mutex::new(rtcp_generator),
            stats: stats.clone(),
            sender_octets: sender_octets.clone(),
            streams: Arc::clone(&streams),
            packet_sender: packet_sender.clone(),
            clock_rate: clock_rate.clone(),
            rtcp_members: DashMap::new(),
            xr_enabled: AtomicBool::new(xr_quality.is_some()),
            xr_quality: xr_quality.unwrap_or_default(),
            periodic_reports: AtomicU64::new(0),
            reschedule: tokio::sync::Notify::new(),
        });

        // A session with a separate RTCP socket was built for a peer that may
        // not multiplex, so it never assumes multiplexing.
        let rtcp_mux = Arc::new(AtomicBool::new(
            config.remote_addr.is_some() && local_rtcp_addr.is_none(),
        ));
        let mut session = Self {
            config,
            clock_rate,
            ssrc,
            transport,
            streams,
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
            rtcp_reporter,
            rtcp_task: None,
            bandwidth_explicit: false,
            rtcp_mux,
            signalled_remote_rtcp_addr: parking_lot::Mutex::new(None),
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
        let clock_rate = self.clock_rate.clone();
        let _payload_type = self.config.payload_type;
        let ssrc = self.ssrc;
        let streams_map = self.streams.clone();
        let received_sender_reports = self.received_sender_reports.clone();
        let _jitter_buffer_enabled = self.config.enable_jitter_buffer;
        let _jitter_size = self.config.jitter_buffer_size.unwrap_or(50);
        let _max_age_ms = self.config.max_packet_age_ms.unwrap_or(200);
        let receive_queue_enabled = self.receive_queue_enabled;
        let rtcp_ingress_transport = self.transport.clone();
        let rtcp_ingress_sender = self.packet_sender.clone();
        let rtcp_ingress_reporter = self.rtcp_reporter.clone();

        let media_sync = self.media_sync.clone();

        // If we have a remote address, set it on the transport
        if let Some(addr) = remote_addr {
            // Set the remote RTP address on the UDP transport
            if let Some(t) = transport.as_any().downcast_ref::<UdpRtpTransport>() {
                t.set_remote_rtp_addr(addr).await;
            }
        }
        self.refresh_remote_rtcp_addr();

        // Prepare the scheduler's timestamp state, but do not start its
        // millisecond polling task. Session sends use the single ordered
        // packet writer above; no production code uses the scheduler queue.
        // Starting one 1 ms timer per call was measurable CPU load under
        // SIPp fan-out.
        if let Some(scheduler) = &mut self.scheduler {
            // Set appropriate timestamp increment based on packet interval
            let interval_ms = 20; // Default 20ms packet interval
            let samples_per_packet = (f64::from(clock_rate.load(Ordering::Relaxed))
                * (interval_ms as f64 / 1000.0)) as u32;
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
                    // STUN belongs to the ICE agent subscribed on the same
                    // bus, not to the RTP session.
                    Ok(crate::traits::RtpEvent::StunPacket { .. }) => continue,
                    Ok(crate::traits::RtpEvent::RtcpReceived { data, source }) => {
                        if !rtcp_source_is_expected(
                            &rtcp_ingress_transport,
                            &rtcp_ingress_sender,
                            &streams_map,
                            source,
                            &data,
                        )
                        .await
                        {
                            stats_recv.lock().rtcp_packets_rejected += 1;
                            debug!(
                                "Dropping RTCP from {} that is not this session's peer",
                                source
                            );
                            continue;
                        }
                        // Parse the complete compound packet. Unknown but
                        // well-formed members are retained by the tolerant
                        // parser and ignored by the production handler.
                        match crate::packet::rtcp::RtcpCompoundPacket::parse_tolerant(&data) {
                            Ok(compound) => {
                                stats_recv.lock().rtcp_packets_received += 1;
                                rtcp_ingress_reporter.observe_received(&data);
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

                                                stats_recv.lock().peer_bye = Some(PeerRtcpBye {
                                                    ssrc: source_ssrc,
                                                    reason: bye.reason.clone(),
                                                    received_at: Instant::now(),
                                                });

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

                                            // An SR may carry the same report blocks as an RR.
                                            // Measure RTT from either packet type; otherwise a
                                            // bidirectional peer that sends SRs never populates
                                            // quality RTT despite reflecting our timestamps.
                                            for block in &sr.report_blocks {
                                                if block.ssrc == ssrc {
                                                    record_peer_reception_report(
                                                        &stats_recv,
                                                        report_ssrc,
                                                        block,
                                                        clock_rate.load(Ordering::Relaxed),
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
                                                    record_peer_reception_report(
                                                        &stats_recv,
                                                        report_ssrc,
                                                        block,
                                                        clock_rate.load(Ordering::Relaxed),
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
                                let mut stream =
                                    RtpStream::new(packet_ssrc, clock_rate.load(Ordering::Relaxed));
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

        // Outbound offer/answer sessions start before their SDP peer is known.
        // Keep one report task alive and resolve the current destination at each
        // tick; taking the generator only when a peer existed stranded late SDP.
        {
            let transport = self.transport.clone();
            let ssrc = self.ssrc;
            let event_tx = self.event_tx.clone();
            let reporter = self.rtcp_reporter.clone();
            let rtcp_mux = self.rtcp_mux.clone();

            let rtcp_task = spawn_memory_tracked("rtp_core.rtp_session.rtcp_task", async move {
                debug!("RTCP scheduling task started");

                let mut previous = tokio::time::Instant::now();
                loop {
                    // RFC 3550 §6.3: the interval follows session bandwidth
                    // and membership, starts at half the minimum until the
                    // first report leaves, and is randomised every time.
                    let interval = reporter.next_interval();
                    trace!("Next RTCP report in {:?}", interval);
                    let deadline = previous + interval;
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => {}
                        // A policy change redraws the pending interval from
                        // the same starting point.
                        _ = reporter.reschedule.notified() => continue,
                    }
                    previous = deadline;

                    // RFC 5761 §5.1.1: reports share the RTP socket only with
                    // negotiated multiplexing; otherwise they need a separate
                    // RTCP socket, or are not sent at all.
                    let Some(remote_addr) = rtcp_report_destination(
                        &transport,
                        &reporter.packet_sender,
                        rtcp_mux.load(Ordering::Acquire),
                    )
                    .await
                    else {
                        continue; // No peer yet, or no RTCP path to it.
                    };

                    let compound = reporter.build(None, true);
                    match reporter.send(&transport, &compound, remote_addr).await {
                        Ok(()) => {
                            debug!("Sent periodic RTCP compound report to {}", remote_addr);
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
                        Err(Error::UnsupportedFeature(_)) => {
                            trace!("Skipping RTCP report while authenticated SRTCP is unavailable");
                        }
                        Err(e) => warn!("Failed to send RTCP compound packet: {}", e),
                    }
                }
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
        self.refresh_remote_rtcp_addr();
    }

    /// Set the peer's RTCP address from SDP `a=rtcp:` (RFC 3605), or clear
    /// it with `None` to fall back to the peer's RTP port + 1. Only a
    /// session with a separate RTCP socket that has not negotiated
    /// multiplexing sends there; a multiplexing session reports to the
    /// peer's RTP address whatever this says.
    pub fn set_remote_rtcp_addr(&self, addr: Option<SocketAddr>) {
        *self.signalled_remote_rtcp_addr.lock() = addr;
        self.refresh_remote_rtcp_addr();
    }

    /// Local address of the separate RTCP socket, while one is open.
    pub fn local_rtcp_addr(&self) -> Option<SocketAddr> {
        self.transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .and_then(UdpRtpTransport::local_rtcp_socket_addr)
    }

    /// Close the separate RTCP socket, once RTP/RTCP multiplexing makes it
    /// unnecessary, and return the address it was bound to so the caller can
    /// hand the port back. RTCP uses the RTP socket from then on. `None`
    /// when the session had no separate socket.
    pub async fn release_rtcp_socket(&self) -> Option<SocketAddr> {
        let udp = self.transport.as_any().downcast_ref::<UdpRtpTransport>()?;
        let released = udp.release_rtcp_socket().await;
        self.refresh_remote_rtcp_addr();
        released
    }

    /// Point the transport's RTCP destination at the right place for the
    /// current multiplexing state: the peer's RTP address when RTCP shares
    /// the RTP socket, otherwise the signalled `a=rtcp:` address or the
    /// peer's RTP port + 1 (RFC 3550 §11).
    fn refresh_remote_rtcp_addr(&self) {
        let Some(udp) = self.transport.as_any().downcast_ref::<UdpRtpTransport>() else {
            return;
        };
        let Some(rtp_peer) = self.config.remote_addr else {
            return;
        };
        let separate = udp.local_rtcp_socket_addr().is_some() && !udp.rtcp_mux();
        let destination = if separate {
            let signalled = *self.signalled_remote_rtcp_addr.lock();
            match signalled {
                Some(addr) => addr,
                None => match rtp_peer.port().checked_add(1) {
                    Some(port) => SocketAddr::new(rtp_peer.ip(), port),
                    None => return,
                },
            }
        } else {
            // Multiplexed RTCP follows the RTP destination, latched or not.
            udp.current_remote_rtp_addr().unwrap_or(rtp_peer)
        };
        udp.store_remote_rtcp_addr(destination);
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
        // Send BYE packet if we have a remote address. Like periodic reports
        // it shares the RTP socket only with negotiated rtcp-mux, and
        // otherwise needs a separate RTCP socket (RFC 5761 §5.1.1).
        let bye_destination = if self.config.remote_addr.is_some() {
            rtcp_report_destination(&self.transport, &self.packet_sender, self.rtcp_mux()).await
        } else {
            None
        };
        if let Some(remote_addr) = bye_destination {
            // Create BYE packet
            let bye = crate::packet::rtcp::RtcpGoodbye::new_with_reason(
                self.ssrc,
                "Session closed".to_string(),
            );

            // RFC 3550 §6.1: a BYE travels in a compound packet that starts
            // with SR or RR and carries the SDES CNAME. A standalone BYE is
            // reduced-size RTCP, which needs RFC 5506 negotiation this
            // session does not do.
            let compound = self.rtcp_reporter.build(Some(bye), false);
            match self
                .rtcp_reporter
                .send(&self.transport, &compound, remote_addr)
                .await
            {
                Ok(()) => {}
                Err(Error::UnsupportedFeature(_)) => {
                    trace!("Skipping RTCP BYE while authenticated SRTCP is unavailable");
                }
                Err(e) => warn!("Failed to send RTCP BYE: {}", e),
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
        self.packet_sender
            .media_payload_type
            .store(payload_type, Ordering::Relaxed);
        if !self.bandwidth_explicit {
            self.rtcp_reporter.generator.lock().set_bandwidth(
                crate::stats::reports::session_bandwidth_for_payload_type(payload_type)
                    .unwrap_or(crate::stats::reports::DEFAULT_SESSION_BANDWIDTH_BPS),
            );
            self.rtcp_reporter.reschedule.notify_one();
        }
    }

    /// Set the RTP clock after a completed codec renegotiation.
    /// Existing receive-stream timing state belongs to the preceding codec
    /// generation and is discarded so the next packet starts a fresh stream.
    pub fn set_clock_rate(&mut self, clock_rate: u32) {
        self.config.clock_rate = clock_rate;
        self.clock_rate.store(clock_rate, Ordering::Release);
        self.streams.clear();
        self.stats.lock().jitter_ms = 0.0;
        if let Some(scheduler) = &mut self.scheduler {
            scheduler.set_clock_rate(clock_rate);
        }
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

        // Send BYE as standards-compliant compound RTCP: SR or RR, SDES
        // CNAME, then BYE (RFC 3550 §6.1). A standalone BYE is reduced-size
        // RTCP and requires separate RFC 5506 negotiation.
        let destination = rtcp_destination(&self.transport, &self.packet_sender)
            .await
            .unwrap_or(remote_addr);
        let compound = self.rtcp_reporter.build(Some(bye), false);
        self.rtcp_reporter
            .send(&self.transport, &compound, destination)
            .await
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

        // RFC 3550 §6.4.1: the RTP timestamp names the same instant as the
        // NTP timestamp, on the media clock — the last sent media timestamp
        // extrapolated to now. Capture the wall clock once for both.
        let at = Instant::now();
        sr.ntp_timestamp = crate::packet::rtcp::NtpTimestamp::now();
        sr.rtp_timestamp = self
            .packet_sender
            .sr_rtp_timestamp(at)
            .unwrap_or_else(|| self.get_timestamp());

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

    /// Allow or stop periodic RTCP reports.
    ///
    /// This session sends RTCP from its single RTP socket to the peer's RTP
    /// address, which RFC 5761 §5.1.1 permits only after `a=rtcp-mux` was
    /// both offered and answered. A session constructed with its peer already
    /// known reports by default, as it always has. A session whose peer
    /// arrives later through SDP stays silent until the signalling layer
    /// confirms multiplexing here.
    ///
    /// A session built with [`Self::new_event_driven_with_rtcp_socket`]
    /// sends from its separate RTCP socket to the peer's RTCP address while
    /// multiplexing is not negotiated, and moves RTCP onto the RTP socket
    /// when it is.
    pub fn set_rtcp_mux(&self, negotiated: bool) {
        self.rtcp_mux.store(negotiated, Ordering::Release);
        // Only a separate RTCP socket changes where reports go; a
        // single-socket session keeps its (possibly latched) destination.
        if let Some(udp) = self.transport.as_any().downcast_ref::<UdpRtpTransport>() {
            if udp.local_rtcp_socket_addr().is_some() {
                udp.set_rtcp_mux(negotiated);
                self.refresh_remote_rtcp_addr();
            }
        }
    }

    /// Whether periodic RTCP reports may currently be sent.
    pub fn rtcp_mux(&self) -> bool {
        self.rtcp_mux.load(Ordering::Acquire)
    }

    /// Set the session bandwidth in bits per second
    ///
    /// This is the RFC 3550 §6.2 session bandwidth — the codec bitrate plus
    /// IP/UDP/RTP header overhead — and drives the RTCP report interval: 5%
    /// of it is shared by the session's members. Without a call here the
    /// session derives it from a static payload type, or assumes
    /// [`DEFAULT_SESSION_BANDWIDTH_BPS`](crate::stats::reports::DEFAULT_SESSION_BANDWIDTH_BPS).
    /// A pending report interval is redrawn with the new value.
    pub fn set_bandwidth(&mut self, bandwidth_bps: u32) {
        self.bandwidth_explicit = true;
        self.rtcp_reporter
            .generator
            .lock()
            .set_bandwidth(bandwidth_bps);
        self.rtcp_reporter.reschedule.notify_one();
    }

    /// Session bandwidth in bits per second used for the RTCP interval.
    pub fn bandwidth(&self) -> u32 {
        self.rtcp_reporter.generator.lock().bandwidth()
    }

    /// Use the RFC 3550 §6.2 reduced minimum RTCP interval — 360 divided by
    /// the session bandwidth in kbit/s, when that is under five seconds —
    /// instead of the fixed five-second minimum. Off by default. A pending
    /// report interval is redrawn with the new minimum.
    pub fn set_rtcp_reduced_minimum(&self, enabled: bool) {
        self.rtcp_reporter
            .generator
            .lock()
            .set_reduced_minimum(enabled);
        self.rtcp_reporter.reschedule.notify_one();
    }

    /// Whether the reduced minimum RTCP interval is in use.
    pub fn rtcp_reduced_minimum(&self) -> bool {
        self.rtcp_reporter.generator.lock().reduced_minimum()
    }

    /// Append RFC 3611 VoIP-metrics XR blocks to periodic reports.
    ///
    /// Off by default: XR is an extension the peer signals interest in with
    /// SDP `a=rtcp-xr` (RFC 3611 §5.1), so the signalling layer enables it
    /// per session once negotiation shows it.
    pub fn set_rtcp_xr_enabled(&self, enabled: bool) {
        self.rtcp_reporter
            .xr_enabled
            .store(enabled, Ordering::Relaxed);
    }

    /// Whether periodic reports carry RFC 3611 VoIP-metrics XR blocks.
    pub fn rtcp_xr_enabled(&self) -> bool {
        self.rtcp_reporter.xr_enabled.load(Ordering::Relaxed)
    }

    /// The SDES CNAME this session reports: random per session (RFC 7022
    /// §4.2) and stable for its lifetime.
    pub fn cname(&self) -> String {
        self.rtcp_reporter.generator.lock().cname().to_string()
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

    #[tokio::test]
    async fn periodic_rtcp_follows_late_sdp_peer_changes_and_stops_on_close() {
        let first = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let second = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: None,
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        fast_rtcp(&mut session);
        let mut bytes = [0u8; 2048];
        assert!(
            tokio::time::timeout(Duration::from_millis(1100), first.recv_from(&mut bytes))
                .await
                .is_err()
        );
        // The signalling layer reports a negotiated a=rtcp-mux.
        session.set_rtcp_mux(true);
        session.set_remote_addr(first.local_addr().unwrap()).await;
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), first.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let report = crate::packet::rtcp::RtcpCompoundPacket::parse(&bytes[..n]).unwrap();
        assert_eq!(report.get_rr().unwrap().ssrc, session.ssrc);
        session.set_remote_addr(second.local_addr().unwrap()).await;
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), second.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let report = crate::packet::rtcp::RtcpCompoundPacket::parse(&bytes[..n]).unwrap();
        assert_eq!(report.get_rr().unwrap().ssrc, session.ssrc);
        session.close().await.unwrap();
        // Drain any report already in flight and the synchronous compound
        // BYE, then require the report task to stop.
        let bye = loop {
            let (n, _) = tokio::time::timeout(Duration::from_secs(1), second.recv_from(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            let compound = crate::packet::rtcp::RtcpCompoundPacket::parse(&bytes[..n]).unwrap();
            if compound
                .packets
                .iter()
                .any(|p| matches!(p, crate::packet::rtcp::RtcpPacket::Goodbye(_)))
            {
                break bytes[..n].to_vec();
            }
        };
        assert_compound_bye(&bye, session.ssrc, "Session closed");
        assert!(
            tokio::time::timeout(Duration::from_secs(1), second.recv_from(&mut bytes))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn late_sdp_peer_gets_no_periodic_rtcp_until_mux_is_negotiated() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: None,
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        assert!(!session.rtcp_mux());
        fast_rtcp(&mut session);
        session.set_remote_addr(peer.local_addr().unwrap()).await;
        let mut bytes = [0u8; 2048];
        // Several report intervals pass; nothing may reach the RTP port.
        assert!(
            tokio::time::timeout(Duration::from_millis(1500), peer.recv_from(&mut bytes))
                .await
                .is_err(),
            "periodic RTCP reached a peer that never agreed to rtcp-mux"
        );
        session.set_rtcp_mux(true);
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let report = crate::packet::rtcp::RtcpCompoundPacket::parse(&bytes[..n]).unwrap();
        assert_eq!(report.get_rr().unwrap().ssrc, session.ssrc);
        session.close().await.unwrap();
    }

    /// Report fast for wall-clock tests: the RFC 3550 §6.2 reduced minimum at
    /// 2 Mbit/s is 0.18 s, so intervals fall in about [0.07, 0.22] s.
    fn fast_rtcp(session: &mut RtpSession) {
        session.set_bandwidth(2_000_000);
        session.set_rtcp_reduced_minimum(true);
    }

    #[tokio::test]
    async fn close_sends_no_rtcp_bye_to_a_peer_without_rtcp_mux() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: None,
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        session.set_remote_addr(peer.local_addr().unwrap()).await;
        assert!(!session.rtcp_mux());
        session.close().await.unwrap();
        let mut bytes = [0u8; 2048];
        assert!(
            tokio::time::timeout(Duration::from_millis(500), peer.recv_from(&mut bytes))
                .await
                .is_err(),
            "RTCP BYE reached the RTP port of a peer that never agreed to rtcp-mux"
        );
    }

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

    /// Assert an RFC 3550 §6.1 compound BYE: SR or RR first, then SDES with
    /// the CNAME, then BYE. Returns the CNAME.
    fn assert_compound_bye(data: &[u8], expected_ssrc: RtpSsrc, expected_reason: &str) -> String {
        use crate::packet::rtcp::{RtcpCompoundPacket, RtcpPacket};

        let compound = RtcpCompoundPacket::parse(data).expect("BYE must be valid compound RTCP");
        assert_eq!(compound.packets.len(), 3, "{compound:?}");
        match &compound.packets[0] {
            RtcpPacket::ReceiverReport(report) => assert_eq!(report.ssrc, expected_ssrc),
            RtcpPacket::SenderReport(report) => assert_eq!(report.ssrc, expected_ssrc),
            packet => panic!("expected SR or RR before BYE, got {packet:?}"),
        }
        let cname = match &compound.packets[1] {
            RtcpPacket::SourceDescription(sdes) => sdes
                .find_cname(expected_ssrc)
                .expect("SDES carries the CNAME")
                .to_string(),
            packet => panic!("expected SDES before BYE, got {packet:?}"),
        };
        match &compound.packets[2] {
            RtcpPacket::Goodbye(bye) => {
                assert_eq!(bye.sources, vec![expected_ssrc]);
                assert_eq!(bye.reason.as_deref(), Some(expected_reason));
            }
            packet => panic!("expected BYE last, got {packet:?}"),
        }
        cname
    }

    #[tokio::test]
    async fn send_bye_uses_compound_rtcp_without_reduced_size_negotiation() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let expected_ssrc = 0x1020_3040;
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ssrc: Some(expected_ssrc),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();

        session
            .send_bye(Some("test complete".to_string()))
            .await
            .unwrap();

        let mut buffer = [0u8; 1500];
        let (size, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .expect("timed out waiting for compound RTCP BYE")
            .unwrap();
        assert_compound_bye(&buffer[..size], expected_ssrc, "test complete");
    }

    #[tokio::test]
    async fn close_uses_compound_rtcp_without_reduced_size_negotiation() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let expected_ssrc = 0x5060_7080;
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ssrc: Some(expected_ssrc),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();

        session.close().await.unwrap();

        let mut buffer = [0u8; 1500];
        let (size, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
            .await
            .expect("timed out waiting for close RTCP BYE")
            .unwrap();
        assert_compound_bye(&buffer[..size], expected_ssrc, "Session closed");
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
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: Some(0x4444_4444),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        // Only the session's peer may feed its RTCP handling.
        session.set_remote_addr(peer.local_addr().unwrap()).await;
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
    async fn peer_reception_reports_about_our_ssrc_and_bye_are_retained_as_state() {
        use crate::packet::rtcp::{
            RtcpCompoundPacket, RtcpGoodbye, RtcpPacket, RtcpReceiverReport, RtcpReportBlock,
        };

        let local_ssrc = 0x4444_4444;
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: Some(local_ssrc),
            clock_rate: 8_000,
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        // RTCP is accepted only from the call's signalled peer, as in a call.
        session.set_remote_addr(peer.local_addr().unwrap()).await;
        let mut events = session.subscribe();
        let local_addr = session.local_addr().unwrap();
        assert!(session.get_stats().peer_report.is_none());
        assert!(session.get_stats().peer_bye.is_none());

        async fn send_and_wait(
            peer: &UdpSocket,
            to: SocketAddr,
            events: &mut broadcast::Receiver<RtpSessionEvent>,
            packets: Vec<RtcpPacket>,
        ) {
            let want_bye = matches!(packets.last(), Some(RtcpPacket::Goodbye(_)));
            let compound = RtcpCompoundPacket { packets };
            peer.send_to(&compound.serialize().unwrap(), to)
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    match events.recv().await.unwrap() {
                        RtpSessionEvent::Bye { .. } if want_bye => return,
                        RtpSessionEvent::RtcpReceiverReport { .. } if !want_bye => return,
                        _ => {}
                    }
                }
            })
            .await
            .expect("RTCP was not processed");
        }

        // One reporter describes our stream and an unrelated source. Only
        // the block about our SSRC is retained.
        let first_reporter = 0x5555_5555;
        let mut rr = RtcpReceiverReport::new(first_reporter);
        let mut about_other = RtcpReportBlock::new(0x9999_9999);
        about_other.fraction_lost = 200;
        rr.add_report_block(about_other);
        let mut about_us = RtcpReportBlock::new(local_ssrc);
        about_us.fraction_lost = 64; // 25 percent
        about_us.cumulative_lost = 0x00ff_fffe; // -2 as a signed 24-bit value
        about_us.highest_seq = 0x0001_0010;
        about_us.jitter = 160; // 20 ms at 8 kHz
        rr.add_report_block(about_us);
        send_and_wait(
            &peer,
            local_addr,
            &mut events,
            vec![RtcpPacket::ReceiverReport(rr)],
        )
        .await;

        let report = session
            .get_stats()
            .peer_report
            .expect("block about our SSRC is retained");
        assert_eq!(report.reporter_ssrc, first_reporter);
        assert_eq!(report.fraction_lost, 64);
        assert!((report.fraction_lost_ratio() - 0.25).abs() < f64::EPSILON);
        assert_eq!(report.cumulative_lost, -2);
        assert_eq!(report.extended_highest_sequence, 0x0001_0010);
        assert_eq!(report.jitter, 160);
        assert!((report.jitter_ms - 20.0).abs() < 1e-9);
        // LSR zero: the peer has not seen one of our SRs, so no RTT.
        assert_eq!(report.rtt_ms, None);
        assert_eq!(session.get_stats().rtt_ms, None);
        // A remote-reported loss never inflates our local inbound loss.
        assert_eq!(session.get_stats().packets_lost, 0);

        // A second reporter's newer block replaces the first one.
        let second_reporter = 0x6666_6666;
        let mut rr = RtcpReceiverReport::new(second_reporter);
        let mut about_us = RtcpReportBlock::new(local_ssrc);
        about_us.fraction_lost = 0;
        about_us.cumulative_lost = 3;
        rr.add_report_block(about_us);
        send_and_wait(
            &peer,
            local_addr,
            &mut events,
            vec![RtcpPacket::ReceiverReport(rr)],
        )
        .await;
        let report = session.get_stats().peer_report.unwrap();
        assert_eq!(report.reporter_ssrc, second_reporter);
        assert_eq!(report.cumulative_lost, 3);

        let mut bye = RtcpGoodbye::new_for_source(second_reporter);
        bye.reason = Some("done".to_string());
        send_and_wait(
            &peer,
            local_addr,
            &mut events,
            vec![
                // A compound packet must lead with a report.
                RtcpPacket::ReceiverReport(RtcpReceiverReport::new(second_reporter)),
                RtcpPacket::Goodbye(bye),
            ],
        )
        .await;
        let bye = session.get_stats().peer_bye.expect("BYE is retained");
        assert_eq!(bye.ssrc, second_reporter);
        assert_eq!(bye.reason.as_deref(), Some("done"));
        // The last reception report survives the BYE.
        assert!(session.get_stats().peer_report.is_some());
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

        // The first report waits half the RFC 3550 minimum, randomised:
        // at most 2.5 s * 1.5 / (e - 3/2), about 3.1 s.
        let (size, _) = tokio::time::timeout(Duration::from_secs(4), peer.recv_from(&mut buffer))
            .await
            .expect("periodic RTCP report was not sent")
            .unwrap();
        let periodic = RtcpCompoundPacket::parse_tolerant(&buffer[..size]).unwrap();
        // Nothing was sent, so the periodic report is an RR (RFC 3550 §6.4).
        let periodic_blocks = periodic
            .packets
            .iter()
            .find_map(|member| match member {
                RtcpCompoundMember::Known(RtcpPacket::ReceiverReport(report)) => {
                    Some(report.report_blocks.clone())
                }
                RtcpCompoundMember::Known(RtcpPacket::SenderReport(report)) => {
                    Some(report.report_blocks.clone())
                }
                _ => None,
            })
            .expect("periodic compound packet did not contain a report");
        assert_eq!(periodic_blocks[0].last_sr, 0x1234_5678);
        assert!(periodic_blocks[0].delay_since_last_sr > 0);
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
    async fn srtp_session_emits_authenticated_manual_rtcp() {
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
        let peer_key = crate::srtp::SrtpCryptoKey::new(key.clone(), salt.clone());
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

        session.send_sender_report().await.unwrap();
        session.send_receiver_report().await.unwrap();

        let mut wire = [0_u8; 2048];
        let mut peer_receive =
            crate::srtp::SrtpContext::new(crate::srtp::SRTP_AES128_CM_SHA1_80, peer_key).unwrap();
        for expected_type in [
            crate::packet::rtcp::RtcpPacketType::SenderReport,
            crate::packet::rtcp::RtcpPacketType::ReceiverReport,
        ] {
            let (length, _) =
                tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut wire))
                    .await
                    .unwrap()
                    .unwrap();
            let plaintext = peer_receive.unprotect_rtcp(&wire[..length]).unwrap();
            let packet = crate::packet::rtcp::RtcpPacket::parse(&plaintext).unwrap();
            assert_eq!(packet.packet_type(), expected_type);
            assert_ne!(&wire[..length], plaintext.as_ref());
        }
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn clock_change_resets_session_jitter_diagnostics() {
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        session.stats.lock().jitter_ms = 37.5;

        session.set_clock_rate(48_000);

        assert_eq!(session.config.clock_rate, 48_000);
        assert_eq!(session.clock_rate.load(Ordering::Acquire), 48_000);
        assert_eq!(session.stats.lock().jitter_ms, 0.0);
        session.close().await.unwrap();
    }

    #[test]
    fn measured_quality_builds_rtcp_xr_voip_metrics() {
        let mut report = crate::packet::rtcp::RtcpReportBlock::new(0x5566_7788);
        report.fraction_lost = 8; // 3.125 percent in RFC 3550 fixed-point form.
        report.jitter = 160; // 20 ms at an 8 kHz RTP clock.
        let stats = RtpSessionStats {
            packets_received: 100,
            packets_discarded_by_jitter: 5,
            jitter_ms: 20.0,
            rtt_ms: Some(123.0),
            ..RtpSessionStats::default()
        };

        let xr = build_voip_metrics_xr(0x1122_3344, &[report], &stats, 8_000)
            .expect("one measured source produces an XR packet");
        assert_eq!(xr.ssrc, 0x1122_3344);
        assert_eq!(xr.blocks.len(), 1);
        let crate::packet::rtcp::RtcpXrBlock::VoipMetrics(metrics) = &xr.blocks[0] else {
            panic!("expected VoIP metrics block")
        };
        assert_eq!(metrics.ssrc, 0x5566_7788);
        assert_eq!(metrics.loss_rate, 8);
        assert_eq!(metrics.discard_rate, 12);
        assert_eq!(metrics.round_trip_delay, 123);
        assert_ne!(metrics.mos_lq, 0);
        assert_ne!(metrics.r_factor, 0);
    }

    #[test]
    fn rtcp_xr_is_not_invented_before_a_remote_stream_exists() {
        assert!(
            build_voip_metrics_xr(0x1122_3344, &[], &RtpSessionStats::default(), 8_000,).is_none()
        );
    }

    // ---- RFC 3550 sender/receiver report correctness ----------------------

    /// Receive datagrams at `peer` until a compound RTCP packet arrives,
    /// skipping RTP. `None` when `window` passes first.
    async fn next_rtcp(
        peer: &UdpSocket,
        window: Duration,
    ) -> Option<crate::packet::rtcp::RtcpCompoundPacket> {
        let mut bytes = [0u8; 2048];
        tokio::time::timeout(window, async {
            loop {
                let (n, _) = peer.recv_from(&mut bytes).await.unwrap();
                if n >= 2 && (200..=207).contains(&bytes[1]) {
                    return crate::packet::rtcp::RtcpCompoundPacket::parse(&bytes[..n])
                        .expect("valid compound RTCP");
                }
            }
        })
        .await
        .ok()
    }

    fn ntp_secs(ntp: crate::packet::rtcp::NtpTimestamp) -> f64 {
        ntp.to_duration_since_unix_epoch().as_secs_f64()
    }

    /// The SR's RTP timestamp must be the last sent media timestamp advanced
    /// by the time between that send and the SR's own NTP timestamp at the
    /// 8 kHz media clock (RFC 3550 §6.4.1). `sent_window` brackets the send.
    fn assert_sr_extrapolates(
        sr: &crate::packet::rtcp::RtcpSenderReport,
        media_timestamp: u32,
        sent_window: (f64, f64),
    ) {
        let ticks = f64::from(sr.rtp_timestamp.wrapping_sub(media_timestamp));
        let sr_time = ntp_secs(sr.ntp_timestamp);
        let earliest = (sr_time - sent_window.1) * 8_000.0;
        let latest = (sr_time - sent_window.0) * 8_000.0;
        // One tick of rounding either side.
        assert!(
            ticks >= earliest - 1.0 && ticks <= latest + 1.0,
            "SR RTP timestamp is {ticks} ticks past the last media packet; \
             the sender clock says {earliest:.1}..={latest:.1}"
        );
    }

    async fn send_media_bracketed(session: &RtpSession, timestamp: u32) -> (f64, f64) {
        let before = ntp_secs(crate::packet::rtcp::NtpTimestamp::now());
        session
            .send_packet(timestamp, Bytes::from_static(&[0xff; 160]), false)
            .await
            .unwrap();
        let after = ntp_secs(crate::packet::rtcp::NtpTimestamp::now());
        (before, after)
    }

    #[tokio::test]
    async fn manual_sender_report_extrapolates_the_last_sent_media_timestamp() {
        use crate::packet::rtcp::RtcpPacket;

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        session.set_remote_addr(peer.local_addr().unwrap()).await;

        // Near the wrap point, so the extrapolation must wrap too.
        let media_timestamp = 0xffff_ff00;
        let window = send_media_bracketed(&session, media_timestamp).await;
        // A telephone-event packet keeps its tone's start timestamp; it must
        // not move the media clock anchor.
        session
            .send_packet_with_pt(
                0x1234_5678,
                Bytes::from_static(&[1, 0x8a, 0, 160]),
                true,
                101,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;

        session.send_sender_report().await.unwrap();
        let mut buffer = [0u8; 2048];
        let sr = loop {
            let (n, _) = tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            if buffer[1] == 200 {
                match RtcpPacket::parse(&buffer[..n]).unwrap() {
                    RtcpPacket::SenderReport(report) => break report,
                    packet => panic!("expected SR, got {packet:?}"),
                }
            }
        };
        assert_sr_extrapolates(&sr, media_timestamp, window);
        // About 2000 ticks for 250 ms; nowhere near the event timestamp.
        assert!(sr.rtp_timestamp.wrapping_sub(media_timestamp) >= 1_900);
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn periodic_sender_report_extrapolates_the_last_sent_media_timestamp() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let media_timestamp = 0x0102_0304;
        let window = send_media_bracketed(&session, media_timestamp).await;

        // First report: half the five-second minimum, randomised, < 3.1 s.
        let report = next_rtcp(&peer, Duration::from_secs(4))
            .await
            .expect("periodic report");
        let sr = report.get_sr().expect("a session that sent RTP reports SR");
        assert_eq!(sr.sender_packet_count, 1);
        assert_sr_extrapolates(sr, media_timestamp, window);
    }

    #[tokio::test]
    async fn silent_session_reports_rr_and_sender_state_lasts_two_intervals() {
        use crate::packet::rtcp::RtcpPacket;

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        fast_rtcp(&mut session);
        let window = Duration::from_secs(2);

        // Nothing sent yet: RR + SDES, never an SR with zero counts.
        let first = next_rtcp(&peer, window).await.expect("first report");
        assert!(first.get_sr().is_none(), "silent session sent {first:?}");
        assert_eq!(first.get_rr().unwrap().ssrc, session.get_ssrc());
        assert!(matches!(first.packets[1], RtcpPacket::SourceDescription(_)));

        session
            .send_packet(160, Bytes::from_static(&[0xff; 160]), false)
            .await
            .unwrap();
        // Sent during this interval, and the one before the next report.
        for _ in 0..2 {
            let report = next_rtcp(&peer, window).await.expect("sender report");
            assert_eq!(
                report
                    .get_sr()
                    .expect("SR after sending")
                    .sender_packet_count,
                1
            );
        }
        // Two whole intervals without RTP: a receiver again.
        let report = next_rtcp(&peer, window).await.expect("receiver report");
        assert!(report.get_sr().is_none(), "{report:?}");
        assert!(report.get_rr().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn periodic_report_interval_follows_rfc3550_under_paused_time() {
        let _local = crate::task_runtime::MediaTasksOnCurrentRuntime::enter();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let started = tokio::time::Instant::now();
        let ssrc = session.get_ssrc();
        let mut events = session.subscribe();
        // Keep the session a sender so every report is an SR, which the
        // session also announces locally the moment it leaves.
        let sender = session.send_handle().unwrap();
        let pump = tokio::spawn(async move {
            let mut timestamp = 0_u32;
            loop {
                let _ = sender
                    .send_packet(timestamp, Bytes::from_static(&[0xff; 160]), false)
                    .await;
                timestamp = timestamp.wrapping_add(4_000);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });

        let mut sent_at = Vec::new();
        while sent_at.len() < 16 {
            if let RtpSessionEvent::RtcpSenderReport { ssrc: from, .. } =
                events.recv().await.unwrap()
            {
                if from == ssrc {
                    sent_at.push(tokio::time::Instant::now());
                }
            }
        }
        pump.abort();

        let compensation = std::f64::consts::E - 1.5;
        // First report: the 5 s minimum halved, then [0.5, 1.5] / (e - 3/2).
        let first = (sent_at[0] - started).as_secs_f64();
        assert!(
            first >= 2.5 * 0.5 / compensation - 0.01 && first <= 2.5 * 1.5 / compensation + 0.01,
            "first report after {first} s"
        );
        let gaps: Vec<f64> = sent_at
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).as_secs_f64())
            .collect();
        let (lower, upper) = (5.0 * 0.5 / compensation, 5.0 * 1.5 / compensation);
        for gap in &gaps {
            assert!(
                *gap >= lower - 0.01 && *gap <= upper + 0.01,
                "report gap {gap} s outside [{lower}, {upper}]"
            );
        }
        // Mean 5 / (e - 3/2) = 4.10 s; 15 uniform draws keep it within ±1 s.
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        assert!((mean - 5.0 / compensation).abs() < 1.0, "mean gap {mean} s");
        // Randomised, not a fixed period.
        let spread = gaps.iter().cloned().fold(0.0, f64::max)
            - gaps.iter().cloned().fold(f64::INFINITY, f64::min);
        assert!(spread > 0.5, "report gaps barely vary: {gaps:?}");
    }

    #[tokio::test]
    async fn reduced_minimum_interval_is_a_per_session_opt_in() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        // PCMU: 64 kbit/s plus 16 kbit/s of headers.
        assert_eq!(session.bandwidth(), 80_000);
        assert!(!session.rtcp_reduced_minimum());
        let started = tokio::time::Instant::now();
        session.set_bandwidth(1_000_000);
        session.set_rtcp_reduced_minimum(true);
        assert!(session.rtcp_reduced_minimum());
        // 360 / 1000 kbit/s = 0.36 s minimum, halved for the first report:
        // five reports within 0.22 + 4 * 0.44 s. The five-second floor would
        // need at least 1.03 + 4 * 2.05 s.
        for _ in 0..5 {
            next_rtcp(&peer, Duration::from_secs(2)).await.unwrap();
        }
        let elapsed = started.elapsed().as_secs_f64();
        assert!(elapsed < 2.5, "five reports took {elapsed} s");
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn close_bye_carries_the_sdes_cname() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let cname = session.cname();
        let ssrc = session.get_ssrc();
        session.close().await.unwrap();
        let report = next_rtcp(&peer, Duration::from_secs(1))
            .await
            .expect("close sends a compound BYE");
        let wire = report.serialize().unwrap();
        assert_eq!(assert_compound_bye(&wire, ssrc, "Session closed"), cname);
    }

    #[tokio::test]
    async fn cname_is_random_per_session_and_names_no_user_or_host() {
        use crate::packet::rtcp::RtcpPacket;
        use base64::Engine;

        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        fast_rtcp(&mut session);
        let other = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();

        let mut wire_cnames = Vec::new();
        for _ in 0..2 {
            let report = next_rtcp(&peer, Duration::from_secs(2)).await.unwrap();
            let cname = report
                .packets
                .iter()
                .find_map(|packet| match packet {
                    RtcpPacket::SourceDescription(sdes) => {
                        sdes.find_cname(session.get_ssrc()).map(str::to_string)
                    }
                    _ => None,
                })
                .expect("every report carries SDES CNAME");
            wire_cnames.push(cname);
        }
        // Stable for the session, and what the session says it is.
        assert_eq!(wire_cnames[0], wire_cnames[1]);
        assert_eq!(wire_cnames[0], session.cname());
        // RFC 7022 §4.2: 96 random bits, base64.
        let cname = &wire_cnames[0];
        assert_eq!(cname.len(), 16);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(cname)
            .expect("base64 CNAME");
        assert_eq!(decoded.len(), 12);
        assert!(!cname.contains('@'), "CNAME {cname} looks like user@host");
        if let Ok(user) = std::env::var("USER") {
            assert!(user.is_empty() || !cname.contains(&user));
        }
        assert_ne!(session.cname(), other.cname());
    }

    #[tokio::test]
    async fn rtcp_xr_is_absent_by_default_and_follows_the_session_switch() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        fast_rtcp(&mut session);
        let destination = session.local_addr().unwrap();
        let mut events = session.subscribe();
        let has_xr = |report: &crate::packet::rtcp::RtcpCompoundPacket| {
            report
                .packets
                .iter()
                .any(|p| matches!(p, crate::packet::rtcp::RtcpPacket::ExtendedReport(_)))
        };
        assert!(!session.rtcp_xr_enabled());

        // Receive media so reports carry a block XR could describe.
        send_raw_rtp(&peer, destination, 1, 160, 0x9999_0001).await;
        next_packet_event(&mut events).await;
        // A report may already be in flight at each switch, so judge a few
        // reports at each step.
        let window = Duration::from_secs(2);
        let mut with_blocks = 0;
        for _ in 0..3 {
            let report = next_rtcp(&peer, window).await.unwrap();
            with_blocks += report.get_rr().unwrap().report_blocks.len();
            assert!(!has_xr(&report), "XR sent without being negotiated");
        }
        assert!(
            with_blocks > 0,
            "reports never described the received stream"
        );

        session.set_rtcp_xr_enabled(true);
        let mut xr_seen = false;
        for _ in 0..2 {
            xr_seen |= has_xr(&next_rtcp(&peer, window).await.unwrap());
        }
        assert!(xr_seen, "XR missing once enabled");

        session.set_rtcp_xr_enabled(false);
        next_rtcp(&peer, window).await.unwrap();
        for _ in 0..2 {
            assert!(!has_xr(&next_rtcp(&peer, window).await.unwrap()));
        }
    }

    // ---- Inbound RTCP hygiene --------------------------------------------

    async fn assert_no_rtcp_event(events: &mut broadcast::Receiver<RtpSessionEvent>) {
        let outcome = tokio::time::timeout(Duration::from_millis(300), async {
            loop {
                match events.recv().await {
                    Ok(event @ RtpSessionEvent::RtcpSenderReport { .. })
                    | Ok(event @ RtpSessionEvent::RtcpReceiverReport { .. })
                    | Ok(event @ RtpSessionEvent::Bye { .. }) => return event,
                    Ok(_) => {}
                    Err(error) => panic!("event stream failed: {error}"),
                }
            }
        })
        .await;
        if let Ok(event) = outcome {
            panic!("RTCP from a stranger reached the session: {event:?}");
        }
    }

    #[tokio::test]
    async fn neighbouring_calls_rtcp_is_dropped_and_leaves_stats_untouched() {
        use crate::packet::rtcp::{
            NtpTimestamp, RtcpCompoundPacket, RtcpGoodbye, RtcpReportBlock, RtcpSenderReport,
        };

        // Our call's peer, and the peer of the call on the neighbouring port
        // pair. Without rtcp-mux that neighbour sends RTCP to its RTP port
        // plus one: our RTP port.
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let neighbour = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_ssrc = 0x0a0b_0c0d;
        let mut session = RtpSession::new(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: Some(local_ssrc),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        session.set_remote_addr(peer.local_addr().unwrap()).await;
        let destination = session.local_addr().unwrap();
        let mut events = session.subscribe();

        let compound = |sender: RtpSsrc| {
            let mut sr = RtcpSenderReport::new(sender);
            sr.ntp_timestamp = NtpTimestamp::now();
            // A block about our SSRC that would produce an RTT sample.
            let mut block = RtcpReportBlock::new(local_ssrc);
            block.last_sr = NtpTimestamp::now().to_u32().wrapping_sub(0x0001_0000);
            block.fraction_lost = 200;
            sr.report_blocks.push(block);
            let mut compound = RtcpCompoundPacket::new_with_sr(sr);
            compound.add_bye(RtcpGoodbye::new_for_source(sender));
            compound.serialize().unwrap()
        };

        neighbour
            .send_to(&compound(0x0e0e_0e0e), destination)
            .await
            .unwrap();
        assert_no_rtcp_event(&mut events).await;
        let stats = session.get_stats();
        assert_eq!(stats.rtt_ms, None, "a stranger's report set our RTT");
        assert_eq!(stats.rtcp_packets_received, 0);
        assert_eq!(stats.rtcp_packets_rejected, 1);
        assert!(session.received_sender_reports.is_empty());

        // The call's own peer is still heard.
        peer.send_to(&compound(0x0f0f_0f0f), destination)
            .await
            .unwrap();
        let saw_peer_report = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let RtpSessionEvent::RtcpSenderReport { ssrc, .. } = events.recv().await.unwrap()
                {
                    break ssrc;
                }
            }
        })
        .await
        .expect("the peer's RTCP was dropped");
        assert_eq!(saw_peer_report, 0x0f0f_0f0f);
        let stats = session.get_stats();
        assert_eq!(stats.rtcp_packets_received, 1);
        assert_eq!(stats.rtcp_packets_rejected, 1);
        assert!(stats.rtt_ms.is_some());
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn rtcp_with_a_known_remote_ssrc_is_accepted_from_another_port() {
        // Symmetric RTP off: nothing is latched, and the peer's RTCP leaves
        // from a different port than its RTP. Its SSRC identifies it.
        let rtp_peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rtcp_peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let session = RtpSession::new_with_symmetric_rtp_policy(
            RtpSessionConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                ..RtpSessionConfig::default()
            },
            SymmetricRtpPolicy::disabled(),
        )
        .await
        .unwrap();
        let destination = session.local_addr().unwrap();
        let mut events = session.subscribe();
        let remote_ssrc = 0x1357_9bdf;
        send_raw_rtp(&rtp_peer, destination, 1, 160, remote_ssrc).await;
        next_packet_event(&mut events).await;

        let rr = |ssrc| {
            crate::packet::rtcp::RtcpCompoundPacket::new_with_rr(
                crate::packet::rtcp::RtcpReceiverReport::new(ssrc),
            )
            .serialize()
            .unwrap()
        };
        stranger
            .send_to(&rr(0x2468_ace0), destination)
            .await
            .unwrap();
        assert_no_rtcp_event(&mut events).await;
        rtcp_peer
            .send_to(&rr(remote_ssrc), destination)
            .await
            .unwrap();
        let ssrc = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let RtpSessionEvent::RtcpReceiverReport { ssrc, .. } =
                    events.recv().await.unwrap()
                {
                    break ssrc;
                }
            }
        })
        .await
        .expect("RTCP from a known remote SSRC was dropped");
        assert_eq!(ssrc, remote_ssrc);
        let stats = session.get_stats();
        assert_eq!(stats.rtcp_packets_received, 1);
        assert_eq!(stats.rtcp_packets_rejected, 1);
    }

    /// Bind an RTP socket and the RTCP socket on the port above it, the way
    /// a peer that does not multiplex lays out its ports (RFC 3550 §11).
    async fn bind_port_pair() -> (UdpSocket, UdpSocket) {
        for _ in 0..64 {
            let rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let Some(next) = rtp.local_addr().unwrap().port().checked_add(1) else {
                continue;
            };
            if let Ok(rtcp) = UdpSocket::bind(("127.0.0.1", next)).await {
                return (rtp, rtcp);
            }
        }
        panic!("no adjacent UDP port pair available");
    }

    async fn non_mux_session(policy: SymmetricRtpPolicy, ssrc: RtpSsrc) -> RtpSession {
        let mut session = RtpSession::new_event_driven_with_rtcp_socket(
            RtpSessionConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                ssrc: Some(ssrc),
                ..RtpSessionConfig::default()
            },
            policy,
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        fast_rtcp(&mut session);
        session
    }

    /// Wait for one compound RTCP datagram, returning it and its source.
    async fn recv_rtcp(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
        let mut bytes = [0u8; 2048];
        let (n, source) =
            tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut bytes))
                .await
                .expect("no RTCP arrived")
                .unwrap();
        (bytes[..n].to_vec(), source)
    }

    async fn assert_silent(socket: &UdpSocket, what: &str) {
        let mut bytes = [0u8; 2048];
        assert!(
            tokio::time::timeout(Duration::from_millis(800), socket.recv_from(&mut bytes))
                .await
                .is_err(),
            "{what}"
        );
    }

    /// An SR from `sender` with a report block about `about` that yields an
    /// RTT sample.
    fn sender_report_about(sender: RtpSsrc, about: RtpSsrc) -> Vec<u8> {
        use crate::packet::rtcp::{
            NtpTimestamp, RtcpCompoundPacket, RtcpReportBlock, RtcpSenderReport,
        };
        let mut sr = RtcpSenderReport::new(sender);
        sr.ntp_timestamp = NtpTimestamp::now();
        let mut block = RtcpReportBlock::new(about);
        block.last_sr = NtpTimestamp::now().to_u32().wrapping_sub(0x0001_0000);
        sr.report_blocks.push(block);
        RtcpCompoundPacket::new_with_sr(sr)
            .serialize()
            .unwrap()
            .to_vec()
    }

    async fn next_sender_report(events: &mut broadcast::Receiver<RtpSessionEvent>) -> RtpSsrc {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let RtpSessionEvent::RtcpSenderReport { ssrc, .. } = events.recv().await.unwrap()
                {
                    break ssrc;
                }
            }
        })
        .await
        .expect("the peer's RTCP was dropped")
    }

    #[tokio::test]
    async fn non_mux_rtcp_uses_the_separate_socket_and_the_peers_rtp_port_plus_one() {
        let (peer_rtp, peer_rtcp) = bind_port_pair().await;
        let local_ssrc = 0x5151_5151;
        let mut session = non_mux_session(SymmetricRtpPolicy::default(), local_ssrc).await;
        let local_rtcp = session.local_rtcp_addr().expect("separate RTCP socket");
        assert_ne!(local_rtcp, session.local_addr().unwrap());
        session
            .set_remote_addr(peer_rtp.local_addr().unwrap())
            .await;
        assert!(!session.rtcp_mux());

        // Periodic reports reach RTP + 1 from our RTCP socket.
        let (report, source) = recv_rtcp(&peer_rtcp).await;
        assert_eq!(source, local_rtcp);
        let report = crate::packet::rtcp::RtcpCompoundPacket::parse(&report).unwrap();
        assert_eq!(report.get_rr().unwrap().ssrc, local_ssrc);

        // The peer's RTCP to our RTCP port is parsed and yields an RTT.
        let mut events = session.subscribe();
        peer_rtcp
            .send_to(&sender_report_about(0x6262_6262, local_ssrc), local_rtcp)
            .await
            .unwrap();
        assert_eq!(next_sender_report(&mut events).await, 0x6262_6262);
        let stats = session.get_stats();
        assert!(stats.rtt_ms.is_some(), "no RTT from the peer's report");
        assert_eq!(stats.rtcp_packets_received, 1);

        // Nothing ever reached the RTP port.
        assert_silent(&peer_rtp, "RTCP reached the RTP port of a non-mux peer").await;

        session.close().await.unwrap();
        let bye = loop {
            let (data, source) = recv_rtcp(&peer_rtcp).await;
            assert_eq!(source, local_rtcp);
            let compound = crate::packet::rtcp::RtcpCompoundPacket::parse(&data).unwrap();
            if compound
                .packets
                .iter()
                .any(|p| matches!(p, crate::packet::rtcp::RtcpPacket::Goodbye(_)))
            {
                break data;
            }
        };
        assert_compound_bye(&bye, local_ssrc, "Session closed");
        assert_silent(&peer_rtp, "the close-time BYE reached the RTP port").await;
    }

    #[tokio::test]
    async fn non_mux_rtcp_follows_the_peers_a_rtcp_address() {
        let peer_rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_rtcp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = non_mux_session(SymmetricRtpPolicy::default(), 0x0102_0304).await;
        session
            .set_remote_addr(peer_rtp.local_addr().unwrap())
            .await;
        session.set_remote_rtcp_addr(Some(peer_rtcp.local_addr().unwrap()));
        let (_, source) = recv_rtcp(&peer_rtcp).await;
        assert_eq!(Some(source), session.local_rtcp_addr());
        assert_silent(&peer_rtp, "RTCP reached the RTP port of a non-mux peer").await;

        // An a=rtcp: naming the RTP port itself would be multiplexing the
        // peer never agreed to; nothing is sent.
        session.set_remote_rtcp_addr(Some(peer_rtp.local_addr().unwrap()));
        let mut bytes = [0u8; 2048];
        while tokio::time::timeout(Duration::from_millis(300), peer_rtcp.recv_from(&mut bytes))
            .await
            .is_ok()
        {}
        assert_silent(&peer_rtp, "RTCP was sent to an a=rtcp: naming the RTP port").await;
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn negotiated_mux_moves_rtcp_to_the_rtp_socket_and_frees_the_rtcp_port() {
        let (peer_rtp, peer_rtcp) = bind_port_pair().await;
        let session = non_mux_session(SymmetricRtpPolicy::default(), 0x0a0a_0a0a).await;
        let local_rtcp = session.local_rtcp_addr().unwrap();
        let local_rtp = session.local_addr().unwrap();
        let mut session = session;
        session
            .set_remote_addr(peer_rtp.local_addr().unwrap())
            .await;
        session.set_rtcp_mux(true);
        assert_eq!(session.release_rtcp_socket().await, Some(local_rtcp));
        assert_eq!(session.local_rtcp_addr(), None);
        assert_eq!(session.release_rtcp_socket().await, None);
        // The port is closed: it can be bound again at once.
        drop(
            UdpSocket::bind(local_rtcp)
                .await
                .expect("RTCP port was not released"),
        );

        let mut bytes = [0u8; 2048];
        // Drain anything sent to RTP + 1 before the switch.
        while tokio::time::timeout(Duration::from_millis(50), peer_rtcp.recv_from(&mut bytes))
            .await
            .is_ok()
        {}
        let (_, source) = recv_rtcp(&peer_rtp).await;
        assert_eq!(source, local_rtp);
        assert_silent(&peer_rtcp, "RTCP still reached RTP + 1 after mux").await;
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn neighbouring_rtcp_on_the_separate_rtcp_socket_is_rejected() {
        let (peer_rtp, _peer_rtcp) = bind_port_pair().await;
        let neighbour = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_ssrc = 0x7373_7373;
        let mut session = non_mux_session(SymmetricRtpPolicy::default(), local_ssrc).await;
        session
            .set_remote_addr(peer_rtp.local_addr().unwrap())
            .await;
        let local_rtcp = session.local_rtcp_addr().unwrap();
        let mut events = session.subscribe();
        neighbour
            .send_to(&sender_report_about(0x0e0e_0e0e, local_ssrc), local_rtcp)
            .await
            .unwrap();
        assert_no_rtcp_event(&mut events).await;
        let stats = session.get_stats();
        assert_eq!(stats.rtt_ms, None, "a stranger's report set our RTT");
        assert_eq!(stats.rtcp_packets_received, 0);
        assert_eq!(stats.rtcp_packets_rejected, 1);
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn symmetric_rtcp_latches_the_peers_mapped_rtcp_source_only() {
        // A peer behind NAT: its RTP arrives from the signalled address, its
        // RTCP from a mapped port that is not RTP + 1.
        let (peer_rtp, peer_rtcp_signalled) = bind_port_pair().await;
        let peer_rtcp_mapped = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_ssrc = 0x3434_3434;
        let remote_ssrc = 0x4545_4545;
        let mut session = non_mux_session(SymmetricRtpPolicy::default(), local_ssrc).await;
        session
            .set_remote_addr(peer_rtp.local_addr().unwrap())
            .await;
        let local_rtcp = session.local_rtcp_addr().unwrap();
        let mut events = session.subscribe();
        send_raw_rtp(
            &peer_rtp,
            session.local_addr().unwrap(),
            1,
            160,
            remote_ssrc,
        )
        .await;
        next_packet_event(&mut events).await;

        // A stranger's report does not move the destination.
        stranger
            .send_to(&sender_report_about(0x0e0e_0e0e, local_ssrc), local_rtcp)
            .await
            .unwrap();
        assert_no_rtcp_event(&mut events).await;

        peer_rtcp_mapped
            .send_to(&sender_report_about(remote_ssrc, local_ssrc), local_rtcp)
            .await
            .unwrap();
        assert_eq!(next_sender_report(&mut events).await, remote_ssrc);
        let mut bytes = [0u8; 2048];
        while tokio::time::timeout(
            Duration::from_millis(50),
            peer_rtcp_signalled.recv_from(&mut bytes),
        )
        .await
        .is_ok()
        {}
        let (_, source) = recv_rtcp(&peer_rtcp_mapped).await;
        assert_eq!(source, local_rtcp);
        assert_silent(&peer_rtcp_signalled, "RTCP kept going to RTP + 1").await;
        assert_silent(&stranger, "RTCP went to a stranger").await;
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn non_mux_rtcp_is_srtcp_protected_both_ways() {
        let (peer_rtp, peer_rtcp) = bind_port_pair().await;
        let local_ssrc = 0x1919_1919;
        let mut session = non_mux_session(SymmetricRtpPolicy::default(), local_ssrc).await;
        let transport = session.transport();
        let udp = transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .unwrap();
        let context = || {
            crate::srtp::SrtpContext::new(
                crate::srtp::SRTP_AES128_CM_SHA1_80,
                crate::srtp::SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]),
            )
            .unwrap()
        };
        udp.set_srtp_contexts(context(), context()).await.unwrap();
        session
            .set_remote_addr(peer_rtp.local_addr().unwrap())
            .await;

        let (wire, _) = recv_rtcp(&peer_rtcp).await;
        let plaintext = context().unprotect_rtcp(&wire).unwrap();
        assert_ne!(wire, plaintext.as_ref());
        let report = crate::packet::rtcp::RtcpCompoundPacket::parse(&plaintext).unwrap();
        assert_eq!(report.get_rr().unwrap().ssrc, local_ssrc);

        let mut events = session.subscribe();
        let local_rtcp = session.local_rtcp_addr().unwrap();
        // Plain RTCP is refused on a secure session; SRTCP is accepted.
        peer_rtcp
            .send_to(&sender_report_about(0x2727_2727, local_ssrc), local_rtcp)
            .await
            .unwrap();
        assert_no_rtcp_event(&mut events).await;
        let protected = context()
            .protect_rtcp(&sender_report_about(0x2727_2727, local_ssrc))
            .unwrap();
        peer_rtcp.send_to(&protected, local_rtcp).await.unwrap();
        assert_eq!(next_sender_report(&mut events).await, 0x2727_2727);
        assert!(session.get_stats().rtt_ms.is_some());
        assert_silent(&peer_rtp, "SRTCP reached the RTP port of a non-mux peer").await;
        session.close().await.unwrap();
    }
}
