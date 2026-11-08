//! UDP transport for RTP/RTCP
//!
//! This module provides a UDP-based implementation of the RTP transport.

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use tokio::net::UdpSocket;
#[cfg(feature = "dtls-webrtc")]
use tokio::sync::mpsc;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace, warn};

/// RFC 4733 §2.5.1.3 — the sender emits up to three identical
/// end-of-event frames for loss resilience. Each shares
/// `(peer_addr, ssrc, rtp_timestamp)` with the first. A `DashMap`
/// keyed on that triple lets the UDP receive loop suppress the two
/// retransmits at the socket layer so downstream consumers (media-core,
/// session-core) see one logical digit per tone.
///
/// 500 ms covers the worst-case spacing between the three retransmits
/// while keeping the seen-set bounded under sustained DTMF traffic.
const DTMF_DEDUP_TTL: Duration = Duration::from_millis(500);
const RTP_DROP_LOG_INITIAL: u64 = 5;
const RTP_DROP_LOG_EVERY: u64 = 1_000;
const RTP_MALFORMED_WARN_EVERY: u64 = 10_000;

static SRTP_DIAGNOSTICS_ENABLED: AtomicBool = AtomicBool::new(false);
static RTP_DIAGNOSTICS_ENABLED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "memory-diagnostics")]
fn spawn_memory_tracked_on<F>(
    runtime: &tokio::runtime::Handle,
    kind: &'static str,
    future: F,
) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let guard = rvoip_infra_common::memory_diagnostics::ObjectGuard::new(kind, 0);
    runtime.spawn(async move {
        let _guard = guard;
        future.await
    })
}

#[cfg(not(feature = "memory-diagnostics"))]
fn spawn_memory_tracked_on<F>(
    runtime: &tokio::runtime::Handle,
    _: &'static str,
    future: F,
) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime.spawn(future)
}

/// The memory accounting of `spawn_memory_tracked_on`, for a receive loop
/// hosted on a lane poller instead of its own task.
#[cfg(feature = "memory-diagnostics")]
fn memory_tracked<F>(
    kind: &'static str,
    future: F,
) -> impl std::future::Future<Output = F::Output> + Send + 'static
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let guard = rvoip_infra_common::memory_diagnostics::ObjectGuard::new(kind, 0);
    async move {
        let _guard = guard;
        future.await
    }
}

#[cfg(not(feature = "memory-diagnostics"))]
fn memory_tracked<F>(_: &'static str, future: F) -> F
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    future
}

/// The transport's current remote RTP and RTCP addresses (signalled or
/// latched), readable without awaiting and without holding the transport.
#[derive(Clone)]
pub(crate) struct RemotePeerAddrs {
    rtp: Arc<ArcSwapOption<SocketAddr>>,
    rtcp: Arc<ArcSwapOption<SocketAddr>>,
}

impl RemotePeerAddrs {
    /// The current remote RTP address.
    pub(crate) fn rtp(&self) -> Option<SocketAddr> {
        self.rtp.load().as_deref().copied()
    }

    /// The current remote RTCP address.
    pub(crate) fn rtcp(&self) -> Option<SocketAddr> {
        self.rtcp.load().as_deref().copied()
    }
}

/// A transport receive loop: its own task (v1), or hosted on its lane's
/// poller (executor v2 with the lane poller). Only touched on lifecycle.
enum ReceiverTask {
    Task(JoinHandle<()>),
    Lane(crate::lane_poller::LaneTaskHandle),
}

impl ReceiverTask {
    fn abort(&self) {
        match self {
            Self::Task(task) => task.abort(),
            Self::Lane(hosted) => hosted.abort(),
        }
    }

    /// Wait until the loop's future has been dropped.
    async fn join(self) {
        match self {
            Self::Task(task) => {
                let _ = task.await;
            }
            Self::Lane(hosted) => hosted.join().await,
        }
    }
}

/// Move a bound socket's readiness registration onto `runtime`'s reactor.
///
/// Only the registration moves: the descriptor, its bound address and every
/// socket option are kept. A socket bound on the signaling runtime otherwise
/// stays on that runtime's reactor even though its receive task runs on a
/// media lane, so every media datagram would wake the signaling reactor.
fn register_socket_on_media(
    socket: UdpSocket,
    runtime: &tokio::runtime::Handle,
) -> Result<UdpSocket> {
    let socket = socket
        .into_std()
        .map_err(|error| Error::Transport(format!("Failed to deregister media socket: {error}")))?;
    let _entered = runtime.enter();
    UdpSocket::from_std(socket).map_err(|error| {
        Error::Transport(format!(
            "Failed to register media socket on its reactor: {error}"
        ))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RtpMuxPacketClass {
    Rtp,
    Rtcp,
    Stun,
    Dtls,
    Zrtp,
    TurnChannelData,
    TooSmall,
    UnknownReserved,
}

impl RtpMuxPacketClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rtp => "rtp",
            Self::Rtcp => "rtcp",
            Self::Stun => "stun",
            Self::Dtls => "dtls",
            Self::Zrtp => "zrtp",
            Self::TurnChannelData => "turn_channel_data",
            Self::TooSmall => "too_small",
            Self::UnknownReserved => "unknown_reserved",
        }
    }

    fn is_media(self) -> bool {
        matches!(self, Self::Rtp | Self::Rtcp)
    }
}

pub fn set_diagnostics(srtp_enabled: bool, rtp_enabled: bool) {
    SRTP_DIAGNOSTICS_ENABLED.store(srtp_enabled, Ordering::Relaxed);
    RTP_DIAGNOSTICS_ENABLED.store(rtp_enabled, Ordering::Relaxed);
}

fn srtp_diagnostics_enabled() -> bool {
    SRTP_DIAGNOSTICS_ENABLED.load(Ordering::Relaxed)
}

fn rtp_diagnostics_enabled() -> bool {
    RTP_DIAGNOSTICS_ENABLED.load(Ordering::Relaxed)
}

fn classify_rtp_mux_packet(buffer: &[u8]) -> RtpMuxPacketClass {
    if buffer.len() < 2 {
        return RtpMuxPacketClass::TooSmall;
    }

    match buffer[0] {
        // RFC 7983 demux ranges for packets sharing an RTP port.
        0..=3 => RtpMuxPacketClass::Stun,
        16..=19 => RtpMuxPacketClass::Zrtp,
        20..=63 => RtpMuxPacketClass::Dtls,
        64..=79 => RtpMuxPacketClass::TurnChannelData,
        128..=191 => {
            if is_rtcp_packet(buffer) {
                RtpMuxPacketClass::Rtcp
            } else if buffer.len() < crate::packet::header::RTP_MIN_HEADER_SIZE {
                RtpMuxPacketClass::TooSmall
            } else {
                RtpMuxPacketClass::Rtp
            }
        }
        _ => RtpMuxPacketClass::UnknownReserved,
    }
}

fn should_log_packet_drop(count: u64, diagnostics: bool) -> bool {
    diagnostics && (count <= RTP_DROP_LOG_INITIAL || count % RTP_DROP_LOG_EVERY == 0)
}

fn should_log_malformed_rtp(count: u64, diagnostics: bool) -> bool {
    if diagnostics {
        count <= RTP_DROP_LOG_INITIAL || count % RTP_DROP_LOG_EVERY == 0
    } else {
        count == 1 || count % RTP_MALFORMED_WARN_EVERY == 0
    }
}

fn packet_preview(buffer: &[u8]) -> String {
    const PREVIEW_BYTES: usize = 16;

    let visible = &buffer[..buffer.len().min(PREVIEW_BYTES)];
    let mut hex = String::new();
    for (idx, byte) in visible.iter().enumerate() {
        if idx > 0 {
            hex.push(' ');
        }
        let _ = write!(&mut hex, "{:02x}", byte);
    }

    let ascii: String = visible
        .iter()
        .map(|byte| {
            if (*byte).is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            }
        })
        .collect();

    format!(
        "first_bytes_hex=\"{}\" first_bytes_ascii=\"{}\"",
        hex, ascii
    )
}

fn log_dropped_non_rtp_packet(
    diagnostics: bool,
    drop_count: u64,
    local_addr: Option<SocketAddr>,
    source: SocketAddr,
    size: usize,
    class: RtpMuxPacketClass,
    buffer: &[u8],
) {
    if should_log_packet_drop(drop_count, diagnostics) {
        let line = format!(
            "RTP_DIAG dropped_non_rtp local={:?} source={} size={} class={} drops={} {}",
            local_addr,
            source,
            size,
            class.as_str(),
            drop_count,
            packet_preview(buffer)
        );
        eprintln!("{}", line);
        info!("{}", line);
    } else {
        trace!(
            "Dropped non-RTP datagram from {}: class={} size={}",
            source,
            class.as_str(),
            size
        );
    }
}

fn log_malformed_rtp_packet(
    diagnostics: bool,
    drop_count: u64,
    local_addr: Option<SocketAddr>,
    source: SocketAddr,
    size: usize,
    error: &Error,
    buffer: Option<&[u8]>,
) {
    if should_log_malformed_rtp(drop_count, diagnostics) {
        let preview = buffer
            .map(packet_preview)
            .unwrap_or_else(|| "first_bytes_unavailable=true".to_string());
        warn!(
            "Dropped malformed RTP packet local={:?} source={} size={} class=rtp drops={} error={} {}",
            local_addr,
            source,
            size,
            drop_count,
            error,
            preview
        );
    } else {
        debug!(
            "Dropped malformed RTP packet from {}: size={} error={}",
            source, size, error
        );
    }
}

use super::allocator::{GlobalPortAllocator, PairingStrategy, PortAllocator};
use super::rx_hot::{RxHot, RxHotVerdict};
use super::srtp_rx::{OwnerSrtpRx, PreviousRecv, SrtpRecvShared, SrtpRecvSlot};
use super::symmetric::{SymmetricRtpDecision, SymmetricRtpLearner};
use super::validation::PlatformSocketStrategy;
use super::{RtpTransport, RtpTransportConfig, SymmetricRtpDiagnostics, SymmetricRtpPolicy};
use crate::error::Error;
use crate::executor::{
    HandoffSlot, InlineOwnerCounters, InlineRtpIngress, InlineRtpPacket, OwnerTimer,
    INLINE_DRAIN_BUDGET,
};
use crate::hot_state::HotDoorbell;
use crate::packet::rtcp::RtcpPacket;
use crate::packet::RtpPacket;
use crate::traits::RtpEvent;
use crate::Result;

/// Executor v2 state owned by the socket's receive loop: the session's
/// inline ingress, the one owner timer that serves every inline deadline
/// (reorder hold, decoded pacing, a consumer's playout clock), and the
/// receive drain budget.
///
/// RTP datagrams are authenticated and decrypted in place in the receive
/// loop's own buffer; there is no second owner buffer.
struct InlineRx {
    ingress: Box<dyn InlineRtpIngress>,
    timer: OwnerTimer,
    /// Receives taken back-to-back without parking on the socket.
    drained: usize,
    counters: InlineOwnerCounters,
}

/// What woke the inline receive loop.
enum InlineWake {
    Received(std::io::Result<(usize, SocketAddr)>),
    /// An owner deadline was served; nothing was received.
    Timer,
}

impl InlineRx {
    fn new(ingress: Box<dyn InlineRtpIngress>) -> Self {
        Self {
            ingress,
            timer: OwnerTimer::new(),
            drained: 0,
            counters: InlineOwnerCounters::new(),
        }
    }

    /// Point the owner timer at the earliest inline deadline. The runtime
    /// timer is re-registered only when that deadline moved earlier or the
    /// timer fired (see `OwnerTimer`).
    fn rearm(&mut self, now: Instant) {
        let deadline = self.ingress.next_deadline(now);
        let outcome = self.timer.arm(deadline, now);
        self.counters.armed(outcome);
    }

    fn on_timer(&mut self) {
        // The lane pass reading when the pass clock is on (the owner timer
        // just judged its deadline due by that same reading).
        let now = crate::media_clock::now_std();
        self.timer.fired();
        self.counters.fire();
        self.ingress.on_deadline(now);
        // Hot call state: once per expiry, not per packet. The address the
        // lane clock prefetches when the next deadline falls due (0 unless
        // hot state is on).
        self.timer.set_prefetch_hint(self.ingress.prefetch_hint());
        self.rearm(now);
    }

    /// Wait for the next datagram or owner deadline.
    ///
    /// A ready datagram is taken with `try_recv_from` (no readiness
    /// re-registration, no cooperative budget), at most
    /// [`INLINE_DRAIN_BUDGET`] in a row; then the owner yields once and
    /// serves a deadline that came due meanwhile. Only an empty socket parks
    /// the loop, on the socket and the owner timer together. Receives win
    /// ties.
    async fn next_wake(&mut self, socket: &UdpSocket, buffer: &mut [u8]) -> InlineWake {
        if self.drained >= INLINE_DRAIN_BUDGET {
            self.drained = 0;
            self.counters.budget_yield();
            tokio::task::yield_now().await;
            if self.timer.is_due(crate::media_clock::now_std()) {
                self.on_timer();
                return InlineWake::Timer;
            }
        }
        match socket.try_recv_from(buffer) {
            Ok(received) => {
                self.drained += 1;
                return InlineWake::Received(Ok(received));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                self.drained = 0;
                return InlineWake::Received(Err(error));
            }
        }
        self.drained = 0;
        let received = match self.timer.armed_sleep() {
            Some(timer) => tokio::select! {
                biased;
                received = socket.recv_from(buffer) => Some(received),
                () = timer => None,
            },
            None => Some(socket.recv_from(buffer).await),
        };
        match received {
            Some(received) => {
                self.drained = 1;
                InlineWake::Received(received)
            }
            None => {
                self.on_timer();
                InlineWake::Timer
            }
        }
    }
}

/// Executor v2: adopt the session's inline ingress (offered once, before
/// media flows) at the first datagram it can be given.
#[inline]
fn adopt_inline_ingress(
    inline: &mut Option<InlineRx>,
    slot: &HandoffSlot<Box<dyn InlineRtpIngress>>,
) {
    if inline.is_none() && slot.is_dirty() {
        if let Some(ingress) = slot.take() {
            *inline = Some(InlineRx::new(ingress));
        }
    }
}

/// The inbound SRTP/SRTCP context for one datagram: held by the receive loop
/// itself ([`MediaExecutorConfig::owner_srtp`], no lock), or taken under the
/// transport's lock (v1, and every other user).
///
/// [`MediaExecutorConfig::owner_srtp`]: crate::executor::MediaExecutorConfig::owner_srtp
enum RecvContext<'a> {
    Owner(Option<&'a mut crate::srtp::SrtpContext>),
    Shared(parking_lot::MutexGuard<'a, SrtpRecvSlot>),
}

impl<'a> RecvContext<'a> {
    /// `check` false (hot call state): the transport's doorbell has not rung
    /// since the owner last checked the inbound SRTP epoch, so the epoch load
    /// is skipped (see `OwnerSrtpRx::context_gated`).
    #[inline]
    fn open(owner: &'a mut Option<OwnerSrtpRx>, shared: &'a SrtpRecvShared, check: bool) -> Self {
        match owner {
            Some(owner) => Self::Owner(owner.context_gated(check)),
            None => Self::Shared(shared.lock()),
        }
    }

    #[inline]
    fn get(&mut self) -> Option<&mut crate::srtp::SrtpContext> {
        match self {
            Self::Owner(context) => context.as_deref_mut(),
            Self::Shared(slot) => slot.home.as_mut(),
        }
    }
}

/// The receive loop's symmetric-RTP decision for one authenticated RTP
/// packet, shared by the v1 and inline paths: a new signalled destination
/// (generation) restarts the learner, then the learner observes the packet.
/// With hot call state (`rx_hot`) the generation is loaded only when the
/// transport's doorbell moved, and the latched SSRC is remembered.
#[allow(clippy::too_many_arguments)]
#[inline]
fn observe_symmetric_rtp(
    rx_hot: Option<&mut RxHot>,
    bell: u64,
    generation: &AtomicU64,
    learner: &mut SymmetricRtpLearner,
    observed_generation: &mut u64,
    source: SocketAddr,
    ssrc: u32,
    sequence: u16,
    now: Instant,
) -> RxHotVerdict {
    if let Some(hot) = rx_hot {
        return hot.observe(bell, generation, source, ssrc, sequence, now);
    }
    let current = generation.load(Ordering::Acquire);
    if current != *observed_generation {
        learner.reset();
        *observed_generation = current;
    }
    RxHotVerdict {
        decision: learner.observe(source, ssrc, sequence, now),
        latched_ssrc_known: false,
    }
}

/// The receive loop's symmetric-RTP handling of one authenticated RTP
/// packet, shared by the v1 and inline paths: remember the stream's SSRC,
/// latch or rebind the destination, count the decision. Returns whether the
/// packet is admitted. With hot call state, `verdict.latched_ssrc_known`
/// skips the latched-SSRC load (this loop is its only writer) and the
/// doorbell rings after a latch or rebind (the TX destination cache re-reads
/// the destination and the latch).
#[allow(clippy::too_many_arguments)]
fn admit_symmetric_rtp(
    verdict: RxHotVerdict,
    addr: SocketAddr,
    ssrc: u32,
    rtcp_mux: &AtomicBool,
    latched: &AtomicBool,
    latched_rtp_ssrc: &AtomicU64,
    remote_rtp_addr: &ArcSwapOption<SocketAddr>,
    remote_rtcp_addr: &ArcSwapOption<SocketAddr>,
    counters: &SymmetricRtpCounters,
    doorbell: Option<&HotDoorbell>,
) -> bool {
    let decision = verdict.decision;
    if matches!(
        decision,
        SymmetricRtpDecision::Accept
            | SymmetricRtpDecision::LatchInitial
            | SymmetricRtpDecision::Rebind
    ) && !verdict.latched_ssrc_known
        && latched_rtp_ssrc.load(Ordering::Relaxed) != u64::from(ssrc)
    {
        latched_rtp_ssrc.store(u64::from(ssrc), Ordering::Release);
    }
    match decision {
        SymmetricRtpDecision::Accept => true,
        SymmetricRtpDecision::LatchInitial => {
            // Publish the guard first so an overlapping outbound send cannot
            // restore its stale SDP destination after this store.
            latched.store(true, Ordering::Release);
            remote_rtp_addr.store(Some(Arc::new(addr)));
            if rtcp_mux.load(Ordering::Acquire) {
                remote_rtcp_addr.store(Some(Arc::new(addr)));
            }
            if let Some(doorbell) = doorbell {
                doorbell.ring();
            }
            counters.destination_learned.store(true, Ordering::Relaxed);
            true
        }
        SymmetricRtpDecision::Rebind => {
            remote_rtp_addr.store(Some(Arc::new(addr)));
            if rtcp_mux.load(Ordering::Acquire) {
                remote_rtcp_addr.store(Some(Arc::new(addr)));
            }
            if let Some(doorbell) = doorbell {
                doorbell.ring();
            }
            counters.accepted_rebindings.fetch_add(1, Ordering::Relaxed);
            true
        }
        SymmetricRtpDecision::Probation => {
            counters.probation_packets.fetch_add(1, Ordering::Relaxed);
            false
        }
        SymmetricRtpDecision::Reject => {
            counters.rejected_packets.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

/// Decode an RFC 4733 telephone-event body (the first four octets of
/// `payload`; RFC 4733 lets a sender append more) into a typed DTMF event,
/// with the RFC 4733 §2.5.1.3 retransmit dedup: a sender emits up to three
/// identical end-of-event frames sharing `(ssrc, rtp_timestamp)`. Entries are
/// keyed by `(peer_addr, ssrc, ts)` so two simultaneous DTMF streams from
/// different peers fire independently, and stale entries are pruned on every
/// end-of-event frame, so the map stays bounded. `None` is a suppressed
/// retransmit.
fn rfc4733_event(
    payload: &[u8],
    timestamp: u32,
    ssrc: u32,
    source: SocketAddr,
    seen: &DashMap<(SocketAddr, u32, u32), Instant>,
) -> Option<RtpEvent> {
    let p = &payload[..4];
    let event = p[0];
    let byte1 = p[1];
    let end_of_event = (byte1 & 0b1000_0000) != 0;
    let volume = byte1 & 0b0011_1111;
    let duration = u16::from_be_bytes([p[2], p[3]]);
    if end_of_event {
        let key = (source, ssrc, timestamp);
        let now = Instant::now();
        seen.retain(|_, seen_at| now.duration_since(*seen_at) < DTMF_DEDUP_TTL);
        if seen.insert(key, now).is_some() {
            return None;
        }
    }
    Some(RtpEvent::DtmfEvent {
        event,
        end_of_event,
        volume,
        duration,
        timestamp,
        source,
        ssrc,
    })
}

/// Publish a DTMF event, logging a failure only when someone listens.
fn publish_dtmf(event_tx: &broadcast::Sender<RtpEvent>, dtmf: RtpEvent) {
    if event_tx.receiver_count() > 0 {
        if let Err(e) = event_tx.send(dtmf) {
            warn!("Failed to send DTMF event: {}", e);
        }
    } else {
        let _ = event_tx.send(dtmf);
    }
}

/// UDP transport for RTP/RTCP
///
/// This implementation supports RTCP multiplexing as defined in RFC 5761,
/// allowing RTP and RTCP packets to be sent and received on the same port.
///
/// When RTCP multiplexing is enabled (via the `rtcp_mux` field in `RtpTransportConfig`),
/// both RTP and RTCP packets are sent and received on the RTP socket. The transport
/// automatically distinguishes between RTP and RTCP packets based on the payload type:
///
/// * RTCP packets have payload types 200-204 (as defined in RFC 5761)
/// * RTP packets use payload types 0-127
///
/// RTCP multiplexing is recommended for WebRTC and modern VoIP applications
/// as it simplifies NAT traversal and reduces the number of ports required.
#[derive(Default)]
struct SymmetricRtpCounters {
    destination_learned: AtomicBool,
    accepted_rebindings: AtomicU64,
    probation_packets: AtomicU64,
    rejected_packets: AtomicU64,
}

impl SymmetricRtpCounters {
    fn snapshot(&self) -> SymmetricRtpDiagnostics {
        SymmetricRtpDiagnostics {
            destination_learned: self.destination_learned.load(Ordering::Relaxed),
            accepted_rebindings: self.accepted_rebindings.load(Ordering::Relaxed),
            probation_packets: self.probation_packets.load(Ordering::Relaxed),
            rejected_packets: self.rejected_packets.load(Ordering::Relaxed),
        }
    }
}

/// Whether a failed RTP `sendto` refused only this datagram, on a socket that
/// stays usable: the local stack dropped it (`EPERM`/`EACCES` from a netfilter
/// rule or a full conntrack table, `ENOBUFS`, `ENOMEM`), or an ICMP error for
/// an earlier datagram was reported on this one (`ECONNREFUSED`, host or
/// network unreachable or down).
///
/// Such a datagram is lost, exactly as if the network had dropped it. Its
/// sequence number and SRTP index are consumed, so the far end sees a
/// one-packet gap and the stream goes on. Reported as send errors, these
/// ended a call's outbound audio at the first one: callers such as the SIP
/// outbound media pump stop sending on a failed send. Errors that mean the
/// socket itself is unusable (`EBADF`, `EINVAL`, `EMSGSIZE`, ...) still fail
/// the send.
fn datagram_refused(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    if matches!(
        error.kind(),
        ErrorKind::PermissionDenied
            | ErrorKind::ConnectionRefused
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkUnreachable
            | ErrorKind::NetworkDown
            | ErrorKind::OutOfMemory
    ) {
        return true;
    }
    #[cfg(unix)]
    {
        if matches!(error.raw_os_error(), Some(code) if code == libc::ENOBUFS || code == libc::EHOSTDOWN)
        {
            return true;
        }
    }
    false
}

/// Seed the pre-latch symmetric-RTP destination, storing only a changed one.
///
/// Before a validated inbound tuple latches, every outbound RTP and RTCP send
/// seeds the SDP destination. An unconditional `ArcSwapOption::store` costs a
/// fresh `Arc` plus `arc_swap`'s writer path (paying every thread's debt
/// slots) per packet, and a call that never receives media (signaling-only,
/// one-way, early teardown) pays it for every frame. Skipping an equal store
/// is observably identical. Returns whether it stored.
#[inline]
fn seed_prelatch_destination(slot: &ArcSwapOption<SocketAddr>, dest: SocketAddr) -> bool {
    if slot.load().as_deref() == Some(&dest) {
        return false;
    }
    slot.store(Some(Arc::new(dest)));
    true
}

/// Cancellation guard for the narrow interval between reserving allocator
/// state and handing ownership to a live transport. Tokio cancellation drops
/// the constructor future; spawning the idempotent release here prevents an
/// abandoned reservation from shrinking the bounded RTP pool forever.
struct PortAllocationCancellationGuard {
    allocator: Arc<PortAllocator>,
    session_id: Option<String>,
}

impl PortAllocationCancellationGuard {
    fn new(allocator: Arc<PortAllocator>, session_id: String) -> Self {
        Self {
            allocator,
            session_id: Some(session_id),
        }
    }

    fn disarm(&mut self) {
        self.session_id = None;
    }
}

impl Drop for PortAllocationCancellationGuard {
    fn drop(&mut self) {
        let Some(session_id) = self.session_id.take() else {
            return;
        };
        let allocator = self.allocator.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = allocator.release_session(&session_id).await;
            });
        }
    }
}

/// Move-only authority to restore the exact SRTP state replaced by one
/// successful context installation. Dropping it commits the replacement.
pub struct SrtpContextRollback {
    previous_send: Option<crate::srtp::SrtpContext>,
    /// The displaced inbound context, or the identity of the one the
    /// owning receive loop held, which it gives back unchanged.
    previous_recv: PreviousRecv,
    previous_secure_media_required: bool,
    installed_generation: u64,
}

/// `latched_rtp_ssrc` value before any RTP stream has latched. Outside the
/// 32-bit SSRC space, so it never equals a real sender.
const NO_LATCHED_SSRC: u64 = u64::MAX;

pub struct UdpRtpTransport {
    /// RTP socket
    rtp_socket: Arc<UdpSocket>,

    /// RTCP socket (if separate from RTP). Swappable so a session that
    /// negotiates RTP/RTCP multiplexing after creation can close it and hand
    /// its port back; see [`Self::release_rtcp_socket`].
    rtcp_socket: ArcSwapOption<UdpSocket>,

    /// Whether RTCP shares the RTP socket. Starts as `config.rtcp_mux`; a
    /// transport with a separate RTCP socket switches to the RTP socket when
    /// [`Self::set_rtcp_mux`] reports negotiated multiplexing.
    rtcp_mux: Arc<AtomicBool>,

    /// Receive task of the separate RTCP socket, kept apart from
    /// `receiver_tasks` so the socket can be closed on its own.
    rtcp_receiver_task: parking_lot::Mutex<Option<JoinHandle<()>>>,

    /// SSRC of the RTP stream from the latched source, or
    /// [`NO_LATCHED_SSRC`]. The separate RTCP socket learns its peer's RTCP
    /// source only from reports sent by that stream.
    latched_rtp_ssrc: Arc<AtomicU64>,

    /// Transport configuration
    config: RtpTransportConfig,

    /// Bounded source-learning policy. Kept outside `RtpTransportConfig` so
    /// existing public struct literals remain source-compatible.
    symmetric_rtp_policy: SymmetricRtpPolicy,

    /// Aggregate-only symmetric-RTP diagnostics.
    symmetric_rtp_counters: Arc<SymmetricRtpCounters>,

    /// Set before the receive task publishes an inbound-learned destination.
    /// Outbound sends may seed the destination before that first packet, but
    /// must never overwrite an established inbound latch.
    symmetric_rtp_latched: Arc<AtomicBool>,

    /// Incremented by explicit signaling-driven destination changes so the
    /// receive task starts a fresh latch for a re-INVITE or ICE-less endpoint
    /// update.
    symmetric_rtp_generation: Arc<AtomicU64>,

    /// Hot call state (`None` when off): rung after every store the receive
    /// loop or the TX destination cache must observe (`active`, the
    /// symmetric-RTP generation and latch, the remote RTP address, the
    /// inbound SRTP context). See [`crate::hot_state`].
    hot_doorbell: Option<Arc<HotDoorbell>>,

    /// Remote RTP address. `ArcSwapOption` so per-packet `send_rtp_bytes`
    /// can update the cached symmetric-RTP target with a single atomic
    /// store instead of an awaited `Mutex` guard.
    remote_rtp_addr: Arc<ArcSwapOption<SocketAddr>>,

    /// Remote RTCP address — same lock-free swap discipline as
    /// [`Self::remote_rtp_addr`].
    remote_rtcp_addr: Arc<ArcSwapOption<SocketAddr>>,

    /// Event broadcaster
    event_tx: broadcast::Sender<RtpEvent>,

    /// The media lane that owns this transport's socket registrations and
    /// receive tasks. Chosen once per transport, never per packet.
    media_runtime: tokio::runtime::Handle,

    /// Executor v2: the session's inline ingress, handed once to the
    /// receive loop. Never offered under v1.
    inline_ingress: Arc<HandoffSlot<Box<dyn InlineRtpIngress>>>,

    /// Receiver task. Only touched on lifecycle (start/stop) so a
    /// `tokio::Mutex` is fine — never on the per-packet path.
    receiver_tasks: Arc<Mutex<Vec<ReceiverTask>>>,

    /// Whether the transport is active. Polled once per recv-loop
    /// iteration, so an `AtomicBool` keeps that check off the
    /// scheduler.
    active: Arc<AtomicBool>,

    /// Irreversible secure-media policy latch. Once set, every RTP and RTCP
    /// operation must pass through an installed SRTP/SRTCP context.
    secure_media_required: Arc<AtomicBool>,

    /// Fences the move-only rollback token returned by a prepared context
    /// replacement so it cannot overwrite a later re-key.
    srtp_context_generation: Arc<AtomicU64>,

    /// Serializes SRTP context replacement and rollback. The guard protects
    /// only CPU-local validation and pointer swaps and is never held across
    /// network I/O or an `.await` point.
    srtp_context_update: Arc<parking_lot::Mutex<()>>,

    /// Makes allocator release idempotent across explicit close and Drop.
    allocator_release_started: AtomicBool,

    /// RTP datagrams the local network stack refused and that were counted
    /// as lost (see [`datagram_refused`]).
    refused_rtp_datagrams: AtomicU64,

    /// Outbound SRTP context (RFC 4568 / RFC 3711). When `Some`, every
    /// outbound RTP packet is wrapped with `SrtpContext::protect`
    /// before being sent on the wire. When `None`, plain RTP — keeps
    /// the existing UDP path unchanged. `parking_lot::Mutex` because
    /// `protect()` is `&mut self` + CPU-only (AES/HMAC); the lock is
    /// never held across `.await`, so the tokio scheduler overhead
    /// would be pure tax.
    srtp_send: Arc<parking_lot::Mutex<Option<crate::srtp::SrtpContext>>>,

    /// Inbound SRTP context. When installed, every received RTP datagram
    /// (non-RTCP) is fed through `SrtpContext::unprotect`; auth failures are
    /// silently dropped per RFC 3711 §3.4. Used under a `parking_lot::Mutex`
    /// (same rationale as [`Self::srtp_send`]), except that with
    /// `MediaExecutorConfig::owner_srtp` the receive loop checks it out and
    /// uses it without the lock (see `transport::srtp_rx`).
    srtp_recv: Arc<SrtpRecvShared>,

    /// RFC 4733 §2.5.1.3 retransmit dedup state, keyed on
    /// `(peer_addr, ssrc, rtp_timestamp)`. The first end-of-event
    /// frame per tone is forwarded; the two retransmits collide on
    /// this key and are suppressed before they reach `event_tx`. See
    /// [`DTMF_DEDUP_TTL`] for the per-entry lifetime. `peer_addr` is
    /// part of the key because rtp-core has no dialog scope at the
    /// socket layer — two simultaneous DTMF streams from different
    /// peers must each fire independently.
    dtmf_seen: Arc<DashMap<(SocketAddr, u32, u32), Instant>>,

    /// Sender half of the channel a DTLS-SRTP handshake reads from, when
    /// one is in progress. `None` means datagrams classified as DTLS by
    /// [`classify_rtp_mux_packet`] are just dropped (the pre-DTLS-SRTP
    /// behavior). Set by [`Self::dtls_conn_adapter`].
    #[cfg(feature = "dtls-webrtc")]
    dtls_tx: Arc<parking_lot::Mutex<Option<mpsc::Sender<Bytes>>>>,

    #[cfg(feature = "memory-diagnostics")]
    _memory_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard,
    #[cfg(feature = "memory-diagnostics")]
    _event_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard,
}

impl UdpRtpTransport {
    /// Create a new UDP transport for RTP
    pub async fn new(config: RtpTransportConfig) -> Result<Self> {
        let policy = if config.symmetric_rtp {
            SymmetricRtpPolicy::default()
        } else {
            SymmetricRtpPolicy::disabled()
        };
        Self::new_with_symmetric_rtp_policy(config, policy).await
    }

    /// Create a UDP transport with an explicit symmetric-RTP policy.
    ///
    /// This additive constructor leaves [`RtpTransportConfig`] unchanged for
    /// callers that construct it with a public struct literal.
    pub async fn new_with_symmetric_rtp_policy(
        mut config: RtpTransportConfig,
        mut policy: SymmetricRtpPolicy,
    ) -> Result<Self> {
        if !config.symmetric_rtp {
            policy = SymmetricRtpPolicy::disabled();
        }
        policy
            .validate()
            .map_err(|detail| Error::Transport(detail.to_string()))?;
        let buffer_config = config.buffer_config;
        let media_runtime = crate::task_runtime::next_media_io_runtime_handle();

        // Use platform-specific socket strategy
        let socket_strategy = PlatformSocketStrategy::for_current_platform();

        // Determine how to create the sockets based on config
        let (socket_rtp, socket_rtcp, _local_rtp_addr, _local_rtcp_addr) = if config
            .use_port_allocator
        {
            // Generate a session ID if not provided
            let session_id = config.session_id.clone().unwrap_or_else(|| {
                use rand::Rng;
                let random_suffix: u32 = rand::thread_rng().gen();
                format!("rtp-session-{}", random_suffix)
            });
            // Persist generated ownership so `close()` releases the exact
            // reservation made here.
            config.session_id = Some(session_id.clone());

            // Get the global port allocator
            let allocator = GlobalPortAllocator::instance().await;

            // Configure the pairing strategy based on rtcp_mux
            let pairing_strategy = if config.rtcp_mux {
                PairingStrategy::Muxed
            } else {
                PairingStrategy::Adjacent
            };

            // Determine IP from the provided address (keep the same IP, ignore port)
            let ip = config.local_rtp_addr.ip();

            // Allocate port(s)
            debug!("Allocating port(s) with strategy: {:?}", pairing_strategy);
            let (rtp_addr, rtcp_addr_opt) =
                allocator.allocate_port_pair(&session_id, Some(ip)).await?;
            let mut allocation_guard =
                PortAllocationCancellationGuard::new(allocator.clone(), session_id);

            debug!("Allocated RTP port: {}", rtp_addr);
            if let Some(rtcp_addr) = rtcp_addr_opt {
                debug!("Allocated RTCP port: {}", rtcp_addr);
            }

            // Create sockets with the allocated ports
            let socket_rtp = allocator.create_validated_socket(rtp_addr).await?;

            let socket_rtcp = if let Some(rtcp_addr) = rtcp_addr_opt {
                Some(allocator.create_validated_socket(rtcp_addr).await?)
            } else {
                None
            };

            // Keep the allocation guard armed through fallible registration.
            let socket_rtp = register_socket_on_media(socket_rtp, &media_runtime)?;
            let socket_rtcp = socket_rtcp
                .map(|socket| register_socket_on_media(socket, &media_runtime))
                .transpose()?;

            // The live transport now owns release through `config.session_id`.
            allocation_guard.disarm();

            (socket_rtp, socket_rtcp, rtp_addr, rtcp_addr_opt)
        } else {
            // Traditional socket binding without the allocator
            // Create RTP socket
            let socket_rtp = UdpSocket::bind(config.local_rtp_addr)
                .await
                .map_err(|e| Error::Transport(format!("Failed to bind RTP socket: {}", e)))?;

            // Apply platform-specific settings to the socket
            socket_strategy
                .apply_to_socket(&socket_rtp)
                .await
                .map_err(|e| Error::Transport(format!("Failed to configure RTP socket: {}", e)))?;

            // Get bound address
            let local_rtp_addr = socket_rtp
                .local_addr()
                .map_err(|e| Error::Transport(format!("Failed to get local RTP address: {}", e)))?;

            debug!("Bound RTP socket to {}", local_rtp_addr);

            // Create RTCP socket if not using RTCP-MUX
            let (socket_rtcp, local_rtcp_addr) = if !config.rtcp_mux {
                // Use the next port for RTCP (per convention)
                let local_rtcp_addr = match config.local_rtcp_addr {
                    Some(addr) => addr,
                    None => {
                        let rtcp_port = local_rtp_addr.port() + 1;
                        SocketAddr::new(local_rtp_addr.ip(), rtcp_port)
                    }
                };

                // Create RTCP socket
                let socket_rtcp = UdpSocket::bind(local_rtcp_addr)
                    .await
                    .map_err(|e| Error::Transport(format!("Failed to bind RTCP socket: {}", e)))?;

                // Apply platform-specific settings to the socket
                socket_strategy
                    .apply_to_socket(&socket_rtcp)
                    .await
                    .map_err(|e| {
                        Error::Transport(format!("Failed to configure RTCP socket: {}", e))
                    })?;

                // Get bound address
                let local_rtcp_addr = socket_rtcp.local_addr().map_err(|e| {
                    Error::Transport(format!("Failed to get local RTCP address: {}", e))
                })?;

                debug!("Bound RTCP socket to {}", local_rtcp_addr);

                (Some(socket_rtcp), Some(local_rtcp_addr))
            } else {
                debug!("Using RTCP-MUX - no separate RTCP socket");
                (None, None)
            };

            let socket_rtp = register_socket_on_media(socket_rtp, &media_runtime)?;
            let socket_rtcp = socket_rtcp
                .map(|socket| register_socket_on_media(socket, &media_runtime))
                .transpose()?;

            (socket_rtp, socket_rtcp, local_rtp_addr, local_rtcp_addr)
        };

        // Create broadcaster
        let (event_tx, _) = broadcast::channel(buffer_config.event_channel_capacity.max(1));

        let transport = Self {
            rtp_socket: Arc::new(socket_rtp),
            rtcp_socket: ArcSwapOption::from(socket_rtcp.map(Arc::new)),
            rtcp_mux: Arc::new(AtomicBool::new(config.rtcp_mux)),
            rtcp_receiver_task: parking_lot::Mutex::new(None),
            latched_rtp_ssrc: Arc::new(AtomicU64::new(NO_LATCHED_SSRC)),
            config,
            symmetric_rtp_policy: policy,
            symmetric_rtp_counters: Arc::new(SymmetricRtpCounters::default()),
            symmetric_rtp_latched: Arc::new(AtomicBool::new(false)),
            symmetric_rtp_generation: Arc::new(AtomicU64::new(0)),
            hot_doorbell: crate::executor::hot_state_enabled()
                .then(|| Arc::new(HotDoorbell::new())),
            remote_rtp_addr: Arc::new(ArcSwapOption::from(None)),
            remote_rtcp_addr: Arc::new(ArcSwapOption::from(None)),
            event_tx,
            media_runtime,
            inline_ingress: Arc::new(HandoffSlot::new()),
            receiver_tasks: Arc::new(Mutex::new(Vec::with_capacity(2))),
            active: Arc::new(AtomicBool::new(false)),
            secure_media_required: Arc::new(AtomicBool::new(false)),
            srtp_context_generation: Arc::new(AtomicU64::new(0)),
            srtp_context_update: Arc::new(parking_lot::Mutex::new(())),
            allocator_release_started: AtomicBool::new(false),
            refused_rtp_datagrams: AtomicU64::new(0),
            srtp_send: Arc::new(parking_lot::Mutex::new(None)),
            srtp_recv: Arc::new(SrtpRecvShared::new()),
            dtmf_seen: Arc::new(DashMap::with_shard_amount(
                crate::session::PER_SESSION_MAP_SHARDS,
            )),
            #[cfg(feature = "dtls-webrtc")]
            dtls_tx: Arc::new(parking_lot::Mutex::new(None)),
            #[cfg(feature = "memory-diagnostics")]
            _memory_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "rtp_core.udp_transport",
                std::mem::size_of::<Self>(),
            ),
            #[cfg(feature = "memory-diagnostics")]
            _event_channel_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "rtp_core.udp_transport.event_broadcast_capacity",
                buffer_config.event_channel_capacity * std::mem::size_of::<RtpEvent>(),
            ),
        };

        // Start the receiver task
        transport.start_receiver().await?;

        Ok(transport)
    }

    /// Spawn one of this transport's receive tasks on its media lane, beside
    /// the reactor that serves its sockets.
    fn spawn_on_lane<F>(&self, kind: &'static str, future: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        spawn_memory_tracked_on(&self.media_runtime, kind, future)
    }

    /// Executor v2: whether no inline ingress has been offered yet.
    pub(crate) fn inline_ingress_unclaimed(&self) -> bool {
        !self.inline_ingress.is_claimed()
    }

    /// Executor v2: hand RTP media to `ingress` inline in the receive loop
    /// instead of publishing `RtpEvent::MediaReceived`. Once per transport.
    /// The loop adopts it before the next RTP packet; RTCP, DTMF, STUN, DTLS
    /// and errors keep using the broadcast.
    pub(crate) fn attach_inline_ingress(&self, ingress: Box<dyn InlineRtpIngress>) -> bool {
        self.inline_ingress.offer(ingress).is_ok()
    }

    /// Start the packet receiver task
    async fn start_receiver(&self) -> Result<()> {
        // Single CAS guards both the early-return and the activation —
        // if another caller already set `active`, we return without
        // spawning a second receiver pair.
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }

        // Start RTP receiver
        let rtp_socket = self.rtp_socket.clone();
        let event_tx = self.event_tx.clone();
        let active_state = self.active.clone();
        let srtp_recv = self.srtp_recv.clone();
        let secure_media_required = self.secure_media_required.clone();
        let dtmf_seen = self.dtmf_seen.clone();
        let symmetric_rtp_policy = self.symmetric_rtp_policy;
        let symmetric_rtp_counters = self.symmetric_rtp_counters.clone();
        let symmetric_rtp_latched = self.symmetric_rtp_latched.clone();
        let symmetric_rtp_generation = self.symmetric_rtp_generation.clone();
        let remote_rtp_addr = self.remote_rtp_addr.clone();
        let remote_rtcp_addr = self.remote_rtcp_addr.clone();
        let rtcp_mux = self.rtcp_mux.clone();
        let latched_rtp_ssrc = self.latched_rtp_ssrc.clone();
        // Hot call state: the doorbell gates the per-datagram control loads.
        let hot_doorbell = self.hot_doorbell.clone();
        #[cfg(feature = "dtls-webrtc")]
        let dtls_tx = self.dtls_tx.clone();
        let srtp_diagnostics = srtp_diagnostics_enabled();
        let rtp_diagnostics = rtp_diagnostics_enabled();
        let local_rtp_addr = rtp_socket.local_addr().ok();
        let recv_buffer_size = self.config.buffer_config.recv_buffer_size;
        let inline_slot = self.inline_ingress.clone();
        // Executor v2: the loop can adopt an inline ingress, and with the lane
        // poller it runs inside its lane's poller task (one task wakeup
        // serves every socket that became ready together).
        let on_lane = crate::executor::lane_poller_enabled();
        // The loop owns the inbound SRTP context when it is that context's
        // only datagram user: no separate RTCP socket, whose receive task
        // unprotects SRTCP with the same context. A transport only ever loses
        // its RTCP socket, so this cannot become wrong later.
        let owner_srtp = crate::executor::owner_srtp_enabled() && self.rtcp_socket.load().is_none();

        let rtp_receiver_future = memory_tracked(
            "rtp_core.udp_transport.rtp_receiver_task",
            async move {
                let mut first_inbound_rtp_logged = false;
                let mut srtp_unprotect_failures = 0_u64;
                let mut non_rtp_drop_count = 0_u64;
                let mut malformed_rtp_drop_count = 0_u64;
                let mut symmetric_rtp = SymmetricRtpLearner::new(symmetric_rtp_policy);
                let mut observed_symmetric_generation =
                    symmetric_rtp_generation.load(Ordering::Acquire);
                debug!("UDP receive loop started on {:?}", rtp_socket.local_addr());
                // Executor v2 (None under v1): see `InlineRx`.
                let mut inline: Option<InlineRx> = None;
                // The inbound SRTP context this loop holds (`None` under v1
                // or without `owner_srtp`). Dropped with the loop, which gives
                // the context back to the transport.
                let mut owner_rx: Option<OwnerSrtpRx> =
                    owner_srtp.then(|| OwnerSrtpRx::new(srtp_recv.clone()));
                // Hot call state (`None` when off): the per-check doorbell
                // snapshots and the symmetric-RTP learner, in the lane's slab
                // when this loop runs on a dedicated current-thread lane.
                // Placed on the first poll, on the thread that polls the loop.
                let mut rx_hot: Option<RxHot> = hot_doorbell
                    .as_ref()
                    .map(|_| RxHot::new(symmetric_rtp_policy));
                // One receive buffer serves the whole loop. The inline path
                // authenticates and decrypts RTP in place in it; on the v1
                // path every event copies the bytes it keeps, and each
                // iteration reads only the `size` bytes recv_from returned.
                let mut storage = vec![0u8; recv_buffer_size];

                loop {
                    // Check if we should continue running (hot call state:
                    // only when the doorbell rang since the last check).
                    let check_active = match (rx_hot.as_mut(), hot_doorbell.as_deref()) {
                        (Some(hot), Some(doorbell)) => hot.gate_top(doorbell.read()),
                        _ => true,
                    };
                    if check_active && !active_state.load(Ordering::Acquire) {
                        break;
                    }

                    let buffer: &mut [u8] = &mut storage;

                    // Receive packet
                    let received = match inline.as_mut() {
                        // The drain budget plus the one owner timer that
                        // serves every inline deadline of the call.
                        Some(rx) => match rx.next_wake(&rtp_socket, buffer).await {
                            InlineWake::Received(received) => received,
                            InlineWake::Timer => continue,
                        },
                        None => rtp_socket.recv_from(buffer).await,
                    };
                    match received {
                        Ok((size, addr)) => {
                            if on_lane {
                                crate::lane_poller::note_lane_datagram();
                            }
                            // Hot call state: one doorbell read per datagram,
                            // after it was received; each gated check below
                            // compares it with its own snapshot.
                            let bell = hot_doorbell.as_deref().map_or(0, HotDoorbell::read);
                            trace!("UDP recv_from returned {} bytes from {}", size, addr);

                            let packet_class = classify_rtp_mux_packet(&buffer[..size]);
                            if packet_class == RtpMuxPacketClass::Stun {
                                // ICE connectivity checks share the media
                                // port by design. Forward them to whoever
                                // subscribed (the ICE agent's pump); with no
                                // subscriber this send fails silently, which
                                // is exactly the old drop behavior.
                                let Some(local) = local_rtp_addr else {
                                    continue;
                                };
                                let _ = event_tx.send(RtpEvent::StunPacket {
                                    local,
                                    source: addr,
                                    payload: Bytes::copy_from_slice(&buffer[..size]),
                                });
                                continue;
                            }

                            #[cfg(feature = "dtls-webrtc")]
                            if packet_class == RtpMuxPacketClass::Dtls {
                                let forwarded = dtls_tx
                                    .lock()
                                    .as_ref()
                                    .map(|tx| {
                                        tx.try_send(Bytes::copy_from_slice(&buffer[..size])).is_ok()
                                    })
                                    .unwrap_or(false);
                                if !forwarded {
                                    // No handshake in flight (or its
                                    // receiver is backed up) to hand this
                                    // to; nothing else can use it either.
                                    non_rtp_drop_count = non_rtp_drop_count.saturating_add(1);
                                    log_dropped_non_rtp_packet(
                                        rtp_diagnostics,
                                        non_rtp_drop_count,
                                        local_rtp_addr,
                                        addr,
                                        size,
                                        packet_class,
                                        &buffer[..size],
                                    );
                                }
                                continue;
                            }

                            if !packet_class.is_media() {
                                non_rtp_drop_count = non_rtp_drop_count.saturating_add(1);
                                log_dropped_non_rtp_packet(
                                    rtp_diagnostics,
                                    non_rtp_drop_count,
                                    local_rtp_addr,
                                    addr,
                                    size,
                                    packet_class,
                                    &buffer[..size],
                                );
                                continue;
                            }

                            // Check if it's RTCP according to RFC 5761
                            if packet_class == RtpMuxPacketClass::Rtcp {
                                // Once an RTP tuple is latched, multiplexed
                                // RTCP must arrive on that tuple too. A new
                                // tuple earns trust through RTP probation,
                                // never through a single unauthenticated RTCP
                                // packet.
                                if rtcp_mux.load(Ordering::Acquire)
                                    && symmetric_rtp_policy.enabled
                                    && symmetric_rtp_latched.load(Ordering::Acquire)
                                    && remote_rtp_addr
                                        .load()
                                        .as_deref()
                                        .is_some_and(|expected| *expected != addr)
                                {
                                    symmetric_rtp_counters
                                        .rejected_packets
                                        .fetch_add(1, Ordering::Relaxed);
                                    continue;
                                }
                                debug!("Received RTCP packet, type: {}", buffer[1] & 0x7F);
                                let rtcp_data = if secure_media_required.load(Ordering::Acquire) {
                                    let mut recv = RecvContext::open(
                                        &mut owner_rx,
                                        &srtp_recv,
                                        rx_hot.as_mut().is_none_or(|hot| hot.gate_srtp(bell)),
                                    );
                                    let Some(context) = recv.get() else {
                                        trace!("SRTCP is required but no receive context is installed; dropping packet");
                                        continue;
                                    };
                                    match context.unprotect_rtcp(&buffer[..size]) {
                                        Ok(plaintext) => plaintext,
                                        Err(error) => {
                                            trace!(
                                                "SRTCP unprotect failed; dropping packet: {error}"
                                            );
                                            continue;
                                        }
                                    }
                                } else {
                                    Bytes::copy_from_slice(&buffer[..size])
                                };
                                // Executor v2: the session's inline ingress
                                // may process RTCP here (`owner_rtcp`).
                                adopt_inline_ingress(&mut inline, &inline_slot);
                                if let Some(rx) = inline.as_mut() {
                                    if rx.ingress.on_rtcp(&rtcp_data, addr, Instant::now()) {
                                        continue;
                                    }
                                }
                                let event = RtpEvent::RtcpReceived {
                                    data: rtcp_data,
                                    source: addr,
                                };

                                // Only log errors if there are receivers
                                if event_tx.receiver_count() > 0 {
                                    if let Err(e) = event_tx.send(event) {
                                        warn!("Failed to send RTCP event: {}", e);
                                    }
                                } else {
                                    // Still send the event but ignore errors if no one is listening
                                    let _ = event_tx.send(event);
                                }
                            } else {
                                // Executor v2: adopt the session's inline
                                // ingress before the first RTP packet it can
                                // own.
                                adopt_inline_ingress(&mut inline, &inline_slot);
                                if let Some(rx) = inline.as_mut() {
                                    // Authenticate and decrypt in place in the
                                    // receive buffer: no copy and no
                                    // per-packet allocation.
                                    let data = &mut buffer[..size];
                                    let parsed = {
                                        let mut srtp_guard = RecvContext::open(
                                            &mut owner_rx,
                                            &srtp_recv,
                                            rx_hot.as_mut().is_none_or(|hot| hot.gate_srtp(bell)),
                                        );
                                        if srtp_diagnostics && !first_inbound_rtp_logged {
                                            info!(
                                                "SRTP_DIAG inbound_rtp_first local={:?} source={} size={} srtp_context={}",
                                                local_rtp_addr,
                                                addr,
                                                size,
                                                srtp_guard.get().is_some()
                                            );
                                            first_inbound_rtp_logged = true;
                                        }
                                        if let Some(ctx) = srtp_guard.get() {
                                            match ctx.unprotect_in_place(data) {
                                                Ok(bounds) => Ok(bounds),
                                                Err(_) => {
                                                    srtp_unprotect_failures += 1;
                                                    if srtp_diagnostics
                                                        && (srtp_unprotect_failures <= 5
                                                            || srtp_unprotect_failures % 50 == 0)
                                                    {
                                                        info!(
                                                            "SRTP_DIAG unprotect_failed local={:?} source={} size={} failures={}",
                                                            local_rtp_addr,
                                                            addr,
                                                            size,
                                                            srtp_unprotect_failures
                                                        );
                                                    }
                                                    trace!(
                                                        "SRTP unprotect failed; dropping packet"
                                                    );
                                                    continue;
                                                }
                                            }
                                        } else if secure_media_required.load(Ordering::Acquire) {
                                            trace!("SRTP is required but no receive context is installed; dropping packet");
                                            continue;
                                        } else {
                                            RtpPacket::parse_bounds(data)
                                        }
                                    };
                                    let bounds = match parsed {
                                        Ok(bounds) => bounds,
                                        Err(e) => {
                                            malformed_rtp_drop_count =
                                                malformed_rtp_drop_count.saturating_add(1);
                                            log_malformed_rtp_packet(
                                                rtp_diagnostics,
                                                malformed_rtp_drop_count,
                                                local_rtp_addr,
                                                addr,
                                                size,
                                                &e,
                                                Some(&*data),
                                            );
                                            continue;
                                        }
                                    };
                                    // One pass reading for the datagrams of
                                    // a lane pass (bounded staleness; see
                                    // `lane_clock`), else a clock read.
                                    let now = crate::media_clock::now_std();
                                    let verdict = observe_symmetric_rtp(
                                        rx_hot.as_mut(),
                                        bell,
                                        &symmetric_rtp_generation,
                                        &mut symmetric_rtp,
                                        &mut observed_symmetric_generation,
                                        addr,
                                        bounds.header.ssrc,
                                        bounds.header.sequence_number,
                                        now,
                                    );
                                    if !admit_symmetric_rtp(
                                        verdict,
                                        addr,
                                        bounds.header.ssrc,
                                        &rtcp_mux,
                                        &symmetric_rtp_latched,
                                        &latched_rtp_ssrc,
                                        &remote_rtp_addr,
                                        &remote_rtcp_addr,
                                        &symmetric_rtp_counters,
                                        hot_doorbell.as_deref(),
                                    ) {
                                        continue;
                                    }
                                    let payload = &data[bounds.payload.clone()];
                                    if bounds.header.payload_type == 101 && payload.len() >= 4 {
                                        // DTMF stays on the broadcast path.
                                        if let Some(dtmf) = rfc4733_event(
                                            payload,
                                            bounds.header.timestamp,
                                            bounds.header.ssrc,
                                            addr,
                                            &dtmf_seen,
                                        ) {
                                            publish_dtmf(&event_tx, dtmf);
                                        }
                                        continue;
                                    }
                                    let packet = InlineRtpPacket {
                                        payload_type: bounds.header.payload_type,
                                        sequence_number: bounds.header.sequence_number,
                                        timestamp: bounds.header.timestamp,
                                        marker: bounds.header.marker,
                                        ssrc: bounds.header.ssrc,
                                        padding_size: bounds.padding_size,
                                        payload,
                                        source: addr,
                                    };
                                    rx.ingress.on_rtp(&packet, now);
                                    rx.rearm(now);
                                    continue;
                                }
                                // SRTP unprotect (RFC 3711 §3.4) when an
                                // inbound SrtpContext is configured. Auth
                                // failures MUST be silently dropped — no
                                // event, no warn-level log — to avoid
                                // leaking timing or distinguishing failure
                                // modes to a network attacker.
                                let mut srtp_guard = RecvContext::open(
                                    &mut owner_rx,
                                    &srtp_recv,
                                    rx_hot.as_mut().is_none_or(|hot| hot.gate_srtp(bell)),
                                );
                                if srtp_diagnostics && !first_inbound_rtp_logged {
                                    info!(
                                    "SRTP_DIAG inbound_rtp_first local={:?} source={} size={} srtp_context={}",
                                    local_rtp_addr,
                                    addr,
                                    size,
                                    srtp_guard.get().is_some()
                                );
                                    first_inbound_rtp_logged = true;
                                }
                                let parse_result: Result<RtpPacket> = if let Some(ctx) =
                                    srtp_guard.get()
                                {
                                    match ctx.unprotect(&buffer[0..size]) {
                                        Ok(packet) => Ok(packet),
                                        Err(_) => {
                                            srtp_unprotect_failures += 1;
                                            if srtp_diagnostics
                                                && (srtp_unprotect_failures <= 5
                                                    || srtp_unprotect_failures % 50 == 0)
                                            {
                                                info!(
                                                    "SRTP_DIAG unprotect_failed local={:?} source={} size={} failures={}",
                                                    local_rtp_addr,
                                                    addr,
                                                    size,
                                                    srtp_unprotect_failures
                                                );
                                            }
                                            trace!("SRTP unprotect failed; dropping packet");
                                            drop(srtp_guard);
                                            continue;
                                        }
                                    }
                                } else if secure_media_required.load(Ordering::Acquire) {
                                    trace!("SRTP is required but no receive context is installed; dropping packet");
                                    drop(srtp_guard);
                                    continue;
                                } else {
                                    RtpPacket::parse(&buffer[..size])
                                };
                                drop(srtp_guard);
                                match parse_result {
                                    Ok(packet) => {
                                        let verdict = observe_symmetric_rtp(
                                            rx_hot.as_mut(),
                                            bell,
                                            &symmetric_rtp_generation,
                                            &mut symmetric_rtp,
                                            &mut observed_symmetric_generation,
                                            addr,
                                            packet.header.ssrc,
                                            packet.header.sequence_number,
                                            Instant::now(),
                                        );
                                        if !admit_symmetric_rtp(
                                            verdict,
                                            addr,
                                            packet.header.ssrc,
                                            &rtcp_mux,
                                            &symmetric_rtp_latched,
                                            &latched_rtp_ssrc,
                                            &remote_rtp_addr,
                                            &remote_rtcp_addr,
                                            &symmetric_rtp_counters,
                                            hot_doorbell.as_deref(),
                                        ) {
                                            continue;
                                        }

                                        // Log packet reception at transport level (debug only)
                                        debug!(
                                        "Transport received packet with SSRC={:08x}, seq={}, ts={}",
                                        packet.header.ssrc,
                                        packet.header.sequence_number,
                                        packet.header.timestamp
                                    );

                                        // Debug: Log SSRC demultiplexing info
                                        debug!("SSRC demultiplexing: Forwarding packet with SSRC={:08x}, seq={}, payload size={} bytes",
                                           packet.header.ssrc, packet.header.sequence_number, packet.payload.len());

                                        // RFC 4733: PT 101 (by default) is `telephone-event` —
                                        // DTMF tones carried as RTP events rather than audio
                                        // samples. Decode the 4-byte body inline and emit a
                                        // typed `DtmfEvent` instead of a generic
                                        // `MediaReceived`, so the media layer doesn't have
                                        // to re-parse and doesn't try to feed the bytes to
                                        // a PCMU/PCMA/Opus decoder. Oversized payloads are
                                        // tolerated per RFC 4733's forward-compat clause
                                        // (read only first 4 bytes).
                                        if packet.header.payload_type == 101
                                            && packet.payload.len() >= 4
                                        {
                                            if let Some(dtmf) = rfc4733_event(
                                                &packet.payload,
                                                packet.header.timestamp,
                                                packet.header.ssrc,
                                                addr,
                                                &dtmf_seen,
                                            ) {
                                                publish_dtmf(&event_tx, dtmf);
                                            }
                                            continue;
                                        }

                                        // Create RTP event
                                        let event = RtpEvent::MediaReceived {
                                            payload_type: packet.header.payload_type,
                                            sequence_number: packet.header.sequence_number,
                                            timestamp: packet.header.timestamp,
                                            marker: packet.header.marker,
                                            payload: packet.payload.clone(), // Use the parsed payload
                                            padding_size: packet.padding_size,
                                            source: addr,
                                            ssrc: packet.header.ssrc, // Include the SSRC from the parsed packet
                                        };

                                        // Only log errors if there are receivers
                                        if event_tx.receiver_count() > 0 {
                                            if let Err(e) = event_tx.send(event) {
                                                warn!("Failed to send RTP event: {}", e);
                                            }
                                        } else {
                                            // Still send the event but ignore errors if no one is listening
                                            let _ = event_tx.send(event);
                                        }
                                    }
                                    Err(e) => {
                                        malformed_rtp_drop_count =
                                            malformed_rtp_drop_count.saturating_add(1);
                                        log_malformed_rtp_packet(
                                            rtp_diagnostics,
                                            malformed_rtp_drop_count,
                                            local_rtp_addr,
                                            addr,
                                            size,
                                            &e,
                                            Some(&buffer[..size]),
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Error receiving packet: {}", e);

                            // Send error event
                            let err_event =
                                RtpEvent::Error(Error::Transport(format!("Socket error: {}", e)));
                            if event_tx.receiver_count() > 0 {
                                let _ = event_tx.send(err_event);
                            }

                            // Short delay before retrying
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        }
                    }
                }
            },
        );
        let rtp_receiver = if on_lane {
            match crate::lane_poller::host_on_lane(&self.media_runtime, rtp_receiver_future) {
                Ok(hosted) => ReceiverTask::Lane(hosted),
                // The lane's runtime is shutting down: run it as v1 does.
                Err(future) => ReceiverTask::Task(self.media_runtime.spawn(future)),
            }
        } else {
            ReceiverTask::Task(self.media_runtime.spawn(rtp_receiver_future))
        };

        // Store task handle
        let mut receiver_tasks = self.receiver_tasks.lock().await;
        receiver_tasks.push(rtp_receiver);

        drop(receiver_tasks);

        // If we have a separate RTCP socket, start that receiver too
        if let Some(rtcp_socket) = self.rtcp_socket.load_full() {
            let task = self.spawn_rtcp_receiver(rtcp_socket);
            *self.rtcp_receiver_task.lock() = Some(task);
        }

        info!("Started UDP transport receiver tasks");
        Ok(())
    }

    /// Receive loop for the separate RTCP socket.
    ///
    /// Symmetric RTCP (the RTCP counterpart of symmetric RTP): once inbound
    /// RTP has latched a source, a report from that stream's SSRC arriving
    /// from the same IP address (any IP when the policy allows IP changes)
    /// moves the RTCP destination to the report's source. A peer behind NAT
    /// whose RTCP leaves from a mapped port therefore gets its reports back
    /// through the mapping. The destination never moves onto the peer's RTP
    /// address, moves at most `1 + max_rebindings` times per signalled
    /// destination, and does not move at all with the policy disabled.
    /// Every datagram still goes to the session, which drops reports that
    /// are not from its peer.
    fn spawn_rtcp_receiver(&self, rtcp_socket: Arc<UdpSocket>) -> JoinHandle<()> {
        let event_tx = self.event_tx.clone();
        let active_state = self.active.clone();
        let srtp_recv = self.srtp_recv.clone();
        let secure_media_required = self.secure_media_required.clone();
        let rtcp_recv_buffer_size = self.config.buffer_config.rtcp_recv_buffer_size;
        let policy = self.symmetric_rtp_policy;
        let symmetric_rtp_latched = self.symmetric_rtp_latched.clone();
        let symmetric_rtp_generation = self.symmetric_rtp_generation.clone();
        let remote_rtp_addr = self.remote_rtp_addr.clone();
        let remote_rtcp_addr = self.remote_rtcp_addr.clone();
        let latched_rtp_ssrc = self.latched_rtp_ssrc.clone();

        self.spawn_on_lane("rtp_core.udp_transport.rtcp_receiver_task", async move {
            let mut observed_generation = symmetric_rtp_generation.load(Ordering::Acquire);
            let mut rtcp_moves = 0_u8;
            // Reused across datagrams, as in the RTP receive loop.
            let mut buffer = vec![0u8; rtcp_recv_buffer_size];
            loop {
                // Check if we should continue running
                if !active_state.load(Ordering::Acquire) {
                    break;
                }

                // Receive packet
                match rtcp_socket.recv_from(&mut buffer).await {
                    Ok((size, addr)) => {
                        let rtcp_data = if secure_media_required.load(Ordering::Acquire) {
                            let mut guard = srtp_recv.lock();
                            let Some(context) = guard.home.as_mut() else {
                                trace!("SRTCP is required but no receive context is installed; dropping packet");
                                continue;
                            };
                            match context.unprotect_rtcp(&buffer[..size]) {
                                Ok(plaintext) => plaintext,
                                Err(error) => {
                                    trace!("SRTCP unprotect failed; dropping packet: {error}");
                                    continue;
                                }
                            }
                        } else {
                            Bytes::copy_from_slice(&buffer[..size])
                        };

                        if policy.enabled {
                            let generation = symmetric_rtp_generation.load(Ordering::Acquire);
                            if generation != observed_generation {
                                observed_generation = generation;
                                rtcp_moves = 0;
                            }
                            let rtp_peer = remote_rtp_addr.load().as_deref().copied();
                            let sender_ssrc = rtcp_data
                                .get(4..8)
                                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                                .map(|bytes| u64::from(u32::from_be_bytes(bytes)));
                            let learn = remote_rtcp_addr.load().as_deref().copied() != Some(addr)
                                && symmetric_rtp_latched.load(Ordering::Acquire)
                                && sender_ssrc.is_some_and(|ssrc| {
                                    ssrc == latched_rtp_ssrc.load(Ordering::Acquire)
                                })
                                && rtp_peer.is_some_and(|peer| {
                                    peer != addr
                                        && (policy.allow_ip_change || peer.ip() == addr.ip())
                                })
                                && rtcp_moves <= policy.max_rebindings;
                            if learn {
                                rtcp_moves = rtcp_moves.saturating_add(1);
                                remote_rtcp_addr.store(Some(Arc::new(addr)));
                                debug!("Symmetric RTCP learned the peer's RTCP source");
                            }
                        }

                        let event = RtpEvent::RtcpReceived {
                            data: rtcp_data,
                            source: addr,
                        };
                        let _ = event_tx.send(event);
                    }
                    Err(e) => {
                        error!("Error receiving RTCP packet: {}", e);

                        // Send error event
                        let err_event =
                            RtpEvent::Error(Error::Transport(format!("RTCP socket error: {}", e)));
                        if event_tx.receiver_count() > 0 {
                            let _ = event_tx.send(err_event);
                        }

                        // Short delay before retrying
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                }
            }
        })
    }

    /// Whether RTCP currently travels on the RTP socket: the transport was
    /// built for RTP/RTCP multiplexing, multiplexing was negotiated later, or
    /// the separate RTCP socket was released.
    pub fn rtcp_mux(&self) -> bool {
        self.rtcp_mux.load(Ordering::Acquire)
    }

    /// Report whether RTP/RTCP multiplexing (RFC 5761) is negotiated.
    ///
    /// Only a transport that owns a separate RTCP socket changes behaviour:
    /// with `true` it sends and accepts RTCP on the RTP socket, with `false`
    /// it sends RTCP from the RTCP socket. A transport built for
    /// multiplexing ignores the call.
    pub fn set_rtcp_mux(&self, negotiated: bool) {
        if self.rtcp_socket.load().is_some() {
            self.rtcp_mux.store(negotiated, Ordering::Release);
        }
    }

    /// Close the separate RTCP socket and stop its receive task, switching
    /// RTCP to the RTP socket for good. Returns the address the socket was
    /// bound to, so a caller that reserved the port can hand it back; `None`
    /// when there was no separate socket. The socket is closed when this
    /// returns.
    pub async fn release_rtcp_socket(&self) -> Option<SocketAddr> {
        self.rtcp_mux.store(true, Ordering::Release);
        let socket = self.rtcp_socket.swap(None)?;
        let task = self.rtcp_receiver_task.lock().take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        let local = socket.local_addr().ok();
        drop(socket);
        local
    }

    /// Local address of the separate RTCP socket, if one is open.
    pub fn local_rtcp_socket_addr(&self) -> Option<SocketAddr> {
        self.rtcp_socket
            .load()
            .as_deref()
            .and_then(|socket| socket.local_addr().ok())
    }

    /// Stop the receiver task
    pub async fn stop_receiver(&self) -> Result<()> {
        // Set inactive state
        self.active.store(false, Ordering::Release);
        self.ring_hot_doorbell();

        let rtcp_task = self.rtcp_receiver_task.lock().take();
        if let Some(task) = rtcp_task {
            task.abort();
            let _ = task.await;
        }

        // Wait for receiver task to complete
        let mut receiver_tasks = self.receiver_tasks.lock().await;
        for task in receiver_tasks.drain(..) {
            task.abort();
            task.join().await;
        }

        Ok(())
    }

    /// Set the remote RTP address
    pub async fn set_remote_rtp_addr(&self, addr: SocketAddr) {
        self.symmetric_rtp_latched.store(false, Ordering::Release);
        self.remote_rtp_addr.store(Some(Arc::new(addr)));
        self.symmetric_rtp_generation.fetch_add(1, Ordering::AcqRel);
        self.ring_hot_doorbell();
    }

    /// Hot call state: ring the doorbell after a store the receive loop or
    /// the TX destination cache must observe (no-op when hot state is off).
    #[inline]
    fn ring_hot_doorbell(&self) {
        if let Some(doorbell) = self.hot_doorbell.as_deref() {
            doorbell.ring();
        }
    }

    /// Set the remote RTCP address
    pub async fn set_remote_rtcp_addr(&self, addr: SocketAddr) {
        self.remote_rtcp_addr.store(Some(Arc::new(addr)));
    }

    /// The current (signalled or latched) remote RTP address, without
    /// awaiting; the load is atomic.
    pub(crate) fn current_remote_rtp_addr(&self) -> Option<SocketAddr> {
        self.remote_rtp_addr.load().as_deref().copied()
    }

    /// Test seam: (the receive loop holds the installed inbound context, a
    /// context is installed at home).
    #[cfg(test)]
    pub(crate) fn srtp_recv_location(&self) -> (bool, bool) {
        let slot = self.srtp_recv.lock();
        (slot.owned_by_receiver(), slot.home.is_some())
    }

    /// The remote RTP and RTCP addresses as they change, for a reader that
    /// must not keep the transport alive.
    pub(crate) fn remote_peer_addrs(&self) -> RemotePeerAddrs {
        RemotePeerAddrs {
            rtp: self.remote_rtp_addr.clone(),
            rtcp: self.remote_rtcp_addr.clone(),
        }
    }

    /// Set the remote RTCP address without awaiting; the store is atomic.
    pub(crate) fn store_remote_rtcp_addr(&self, addr: SocketAddr) {
        self.remote_rtcp_addr.store(Some(Arc::new(addr)));
    }

    /// Get the remote RTP address
    pub async fn remote_rtp_addr(&self) -> Option<SocketAddr> {
        self.remote_rtp_addr.load().as_deref().copied()
    }

    /// Build a [`Conn`](webrtc_util::conn::Conn) adapter for running a
    /// DTLS-SRTP handshake over this transport's RTP socket, sharing the
    /// port with RTP/RTCP. [`Self::set_remote_rtp_addr`] must be called
    /// first — the adapter needs a fixed peer to send handshake bytes to.
    ///
    /// From this point on, incoming datagrams the receive loop classifies
    /// as DTLS (RFC 7983) are routed to the returned adapter instead of
    /// being dropped. Only one handshake can be in flight at a time per
    /// transport; calling this again replaces the previous channel.
    #[cfg(feature = "dtls-webrtc")]
    pub fn dtls_conn_adapter(
        &self,
    ) -> Result<Arc<crate::transport::dtls_bridge::DtlsUdpConnAdapter>> {
        let remote = self
            .remote_rtp_addr
            .load()
            .as_deref()
            .copied()
            .ok_or_else(|| {
                crate::error::Error::Transport(
                    "remote RTP address must be set before starting a DTLS handshake".to_string(),
                )
            })?;

        let (tx, rx) = mpsc::channel(64);
        *self.dtls_tx.lock() = Some(tx);

        Ok(Arc::new(
            crate::transport::dtls_bridge::DtlsUdpConnAdapter::new(
                self.rtp_socket.clone(),
                remote,
                rx,
            ),
        ))
    }

    /// Get the remote RTCP address
    pub async fn remote_rtcp_addr(&self) -> Option<SocketAddr> {
        self.remote_rtcp_addr.load().as_deref().copied()
    }

    /// Aggregate-only symmetric-RTP diagnostics. Peer addresses and SSRCs are
    /// intentionally omitted.
    pub fn symmetric_rtp_diagnostics(&self) -> SymmetricRtpDiagnostics {
        self.symmetric_rtp_counters.snapshot()
    }

    /// RTP datagrams the local network stack refused, each treated as a
    /// packet lost on the wire rather than a failed send (a firewall rule
    /// that drops egress, a full conntrack table, `ENOBUFS`, or an ICMP error
    /// reported on a later send).
    pub fn refused_rtp_datagrams(&self) -> u64 {
        self.refused_rtp_datagrams.load(Ordering::Relaxed)
    }

    /// Subscribe to transport events
    pub fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        self.event_tx.subscribe()
    }

    /// Get a clone of the raw RTP socket.
    ///
    /// This low-level interoperability escape is not security-aware: direct
    /// `UdpSocket` reads and writes bypass this transport's SRTP policy,
    /// authentication, replay handling, and event pipeline. Possession of this
    /// handle must never be treated as evidence of a secure media path.
    pub fn get_socket(&self) -> Arc<UdpSocket> {
        self.rtp_socket.clone()
    }

    /// Get the separate raw RTCP socket, when RTCP mux is disabled.
    pub(crate) fn get_rtcp_socket(&self) -> Option<Arc<UdpSocket>> {
        self.rtcp_socket.load_full()
    }

    /// Install per-direction SRTP contexts (RFC 4568 §6.1, RFC 3711).
    ///
    /// `send` protects outbound RTP and RTCP. `recv` authenticates and
    /// unprotects inbound RTP and RTCP. The latch is set before validation so
    /// a rejected installation can never leave a plaintext fallback open.
    ///
    /// Calling this method irreversibly requires secure media, even when
    /// validation fails, so ignoring its error cannot downgrade subsequent
    /// traffic to plaintext. Only enabled contexts using one of the four exact
    /// reviewed AES-CM/HMAC suites are installed. A successful second call
    /// replaces the contexts (used today only in tests; mid-call rekeying is
    /// out of scope for this step).
    pub async fn set_srtp_contexts(
        &self,
        send: crate::srtp::SrtpContext,
        recv: crate::srtp::SrtpContext,
    ) -> Result<()> {
        let _rollback = self.replace_srtp_contexts(send, recv).await?;
        Ok(())
    }

    /// Install a validated directional pair and return move-only authority to
    /// restore the immediately preceding pair. This is used across a SIP
    /// zero-wire commit boundary; dropping the token commits the replacement.
    pub async fn replace_srtp_contexts(
        &self,
        send: crate::srtp::SrtpContext,
        recv: crate::srtp::SrtpContext,
    ) -> Result<SrtpContextRollback> {
        let _update_guard = self.srtp_context_update.lock();
        // Latch before inspecting caller-provided contexts. A rejected
        // installation must fail closed even when its error is ignored; the
        // exact rollback token may restore the previous policy only after a
        // fully validated pair has actually been installed.
        let previous_secure_media_required =
            self.secure_media_required.swap(true, Ordering::AcqRel);
        if let Err(error) = send
            .validate_for_secure_transport()
            .and_then(|()| recv.validate_for_secure_transport())
        {
            // A rejected secure-media installation is itself a newer policy
            // decision. Retire any earlier rollback token so it cannot later
            // reopen the plaintext path by restoring an older latch value.
            let current_generation = self.srtp_context_generation.load(Ordering::Acquire);
            self.srtp_context_generation
                .store(current_generation.saturating_add(1), Ordering::Release);
            return Err(error);
        }
        let (previous_send, previous_recv, installed_generation) = {
            let mut send_guard = self.srtp_send.lock();
            let mut recv_slot = self.srtp_recv.lock();
            let current_generation = self.srtp_context_generation.load(Ordering::Acquire);
            if current_generation.checked_add(2).is_none() {
                // As with validation failure, an exhausted replacement must
                // invalidate older rollback authority while remaining
                // permanently fail closed.
                self.srtp_context_generation
                    .store(current_generation.saturating_add(1), Ordering::Release);
                return Err(Error::InvalidState(
                    "SRTP context generation exhausted".to_string(),
                ));
            }
            let installed_generation = current_generation + 1;
            let previous_send = std::mem::replace(&mut *send_guard, Some(send));
            // An owning receive loop adopts the new context before its next
            // datagram and gives the old one back unchanged.
            let previous_recv = self.srtp_recv.install(&mut recv_slot, recv);
            // Hot call state: after the epoch moved, so the receive loop
            // re-checks it.
            self.ring_hot_doorbell();
            self.srtp_context_generation
                .store(installed_generation, Ordering::Release);
            (previous_send, previous_recv, installed_generation)
        };
        if srtp_diagnostics_enabled() {
            info!(
                "SRTP_DIAG contexts_installed local={:?}",
                self.rtp_socket.local_addr().ok()
            );
        }
        Ok(SrtpContextRollback {
            previous_send,
            previous_recv,
            previous_secure_media_required,
            installed_generation,
        })
    }

    /// Consume an exact replacement token and restore its former contexts.
    /// A later replacement invalidates the token instead of allowing stale
    /// rollback to overwrite newer keys.
    pub async fn rollback_srtp_contexts(&self, rollback: SrtpContextRollback) -> Result<()> {
        let _update_guard = self.srtp_context_update.lock();
        let mut send_guard = self.srtp_send.lock();
        let mut recv_slot = self.srtp_recv.lock();
        if self.srtp_context_generation.load(Ordering::Acquire) != rollback.installed_generation {
            return Err(Error::InvalidState(
                "SRTP rollback token was superseded by a later context replacement".to_string(),
            ));
        }
        let retired_generation = rollback
            .installed_generation
            .checked_add(1)
            .ok_or_else(|| Error::InvalidState("SRTP context generation exhausted".to_string()))?;
        if !self
            .srtp_recv
            .can_restore(&recv_slot, &rollback.previous_recv)
        {
            // Unreachable for a current token (the owning receive loop keeps
            // or gives back the context it held); fail without changing state.
            return Err(Error::InvalidState(
                "SRTP rollback cannot restore the receive context the receive loop held"
                    .to_string(),
            ));
        }
        *send_guard = rollback.previous_send;
        self.srtp_recv
            .restore(&mut recv_slot, rollback.previous_recv);
        self.ring_hot_doorbell();
        self.srtp_context_generation
            .store(retired_generation, Ordering::Release);
        self.secure_media_required
            .store(rollback.previous_secure_media_required, Ordering::Release);
        Ok(())
    }

    /// Whether valid, enabled SRTP contexts are installed in both directions.
    /// This remains false for a secure-media policy latch without ready
    /// contexts, including after a rejected installation attempt.
    pub async fn srtp_enabled(&self) -> bool {
        if !self.secure_media_required.load(Ordering::Acquire) {
            return false;
        }
        self.srtp_send
            .lock()
            .as_ref()
            .is_some_and(|context| context.validate_for_secure_transport().is_ok())
            && self.srtp_recv.lock().installed_valid()
    }

    /// Irreversibly require authenticated RTP and RTCP on this transport.
    ///
    /// DTLS-SRTP callers use this before starting the handshake so media
    /// cannot leak as plaintext while key agreement is still in flight.
    pub fn require_secure_media(&self) {
        self.secure_media_required.store(true, Ordering::Release);
    }

    pub(crate) fn require_srtp(&self) {
        self.require_secure_media();
    }

    /// Send one STUN datagram on the RTP socket.
    ///
    /// ICE checks share the media port and sit *below* SRTP, so this is the
    /// one legitimate plaintext send on a secured transport. The payload
    /// must actually classify as STUN — the check is what keeps the
    /// irreversible secure-media latch meaningful: this door only fits
    /// packets SRTP was never going to cover.
    ///
    /// # Errors
    ///
    /// Refuses payloads that do not classify as STUN, and surfaces socket
    /// errors.
    pub async fn send_stun_bytes(&self, payload: &[u8], destination: SocketAddr) -> Result<()> {
        if classify_rtp_mux_packet(payload) != RtpMuxPacketClass::Stun {
            return Err(Error::InvalidPacket(
                "send_stun_bytes only accepts STUN datagrams".to_string(),
            ));
        }
        self.rtp_socket
            .send_to(payload, destination)
            .await
            .map_err(|error| Error::Transport(format!("stun send failed: {error}")))?;
        Ok(())
    }

    /// Send bytes that have already passed the transport's SRTP protection
    /// path. This is crate-internal so public raw-byte callers cannot bypass
    /// the irreversible secure-media requirement.
    pub(crate) async fn send_protected_rtp_bytes(
        &self,
        bytes: &[u8],
        dest: SocketAddr,
    ) -> Result<()> {
        if !self.secure_media_required.load(Ordering::Acquire) {
            return Err(Error::InvalidState(
                "protected RTP wire send requires secure media mode".to_string(),
            ));
        }
        self.send_rtp_wire_bytes_unchecked(bytes, dest).await
    }

    /// Send an already protected SRTCP datagram from a crate-owned security
    /// wrapper. Public raw-byte callers cannot reach this bypass.
    pub(crate) async fn send_protected_rtcp_bytes(
        &self,
        bytes: &[u8],
        dest: SocketAddr,
    ) -> Result<()> {
        if !self.secure_media_required.load(Ordering::Acquire) {
            return Err(Error::InvalidState(
                "protected SRTCP wire send requires secure media mode".to_string(),
            ));
        }
        self.send_rtcp_wire_bytes_unchecked(bytes, dest).await
    }

    /// Lowest-level RTP datagram write. All public and crate-internal callers
    /// must enforce their security policy before reaching this helper.
    async fn send_rtp_wire_bytes_unchecked(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.config.symmetric_rtp && !self.symmetric_rtp_latched.load(Ordering::Acquire) {
            // Seed the pre-latch target for callers that send before media is
            // received. Once a validated inbound tuple is latched, outbound
            // calls must not restore a stale SDP destination.
            if seed_prelatch_destination(&self.remote_rtp_addr, dest) {
                // Hot call state: only a changed destination rings.
                self.ring_hot_doorbell();
            }
        }

        let sent_bytes = match self.rtp_socket.send_to(bytes, dest).await {
            Ok(sent_bytes) => sent_bytes,
            // The sequence number and SRTP index are already consumed, so the
            // far end sees exactly one lost packet, as on the wire.
            Err(error) if datagram_refused(&error) => {
                self.refused_rtp_datagrams.fetch_add(1, Ordering::Relaxed);
                debug!(%error, %dest, "RTP datagram refused by the local network stack; counted as lost");
                return Ok(());
            }
            Err(e) => {
                return Err(Error::Transport(format!(
                    "Failed to send RTP packet: {}",
                    e
                )))
            }
        };

        debug!("UDP send_to sent {} bytes to {}", sent_bytes, dest);
        Ok(())
    }

    /// Protect an outbound RTCP compound as `send_rtcp_bytes` does: `None`
    /// when it leaves as plaintext.
    fn protect_outbound_rtcp(&self, bytes: &[u8]) -> Result<Option<Bytes>> {
        if !self.secure_media_required.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut guard = self.srtp_send.lock();
        let context = guard.as_mut().ok_or_else(|| {
            Error::InvalidState("SRTCP is required but no send context is installed".to_string())
        })?;
        Ok(Some(context.protect_rtcp(bytes)?))
    }

    /// The socket RTCP leaves from now (the separate RTCP socket while
    /// multiplexing is off, else `None` for the RTP socket), after seeding
    /// the pre-latch RTCP destination.
    fn rtcp_wire_socket(&self, dest: SocketAddr) -> Option<Arc<UdpSocket>> {
        if self.config.symmetric_rtp && !self.symmetric_rtp_latched.load(Ordering::Acquire) {
            seed_prelatch_destination(&self.remote_rtcp_addr, dest);
        }
        if self.rtcp_mux.load(Ordering::Acquire) {
            None
        } else {
            self.rtcp_socket.load_full()
        }
    }

    async fn send_rtcp_wire_bytes_unchecked(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        let rtcp_socket = self.rtcp_wire_socket(dest);
        rtcp_socket
            .as_deref()
            .unwrap_or(&self.rtp_socket)
            .send_to(bytes, dest)
            .await
            .map_err(|e| Error::Transport(format!("Failed to send RTCP packet: {}", e)))?;
        Ok(())
    }

    /// [`RtpTransport::send_rtcp_bytes`] with the datagram written by
    /// `try_send_to` from the caller's poll (no readiness registration, no
    /// await) when the socket takes it. Protection, the secure-media policy,
    /// the pre-latch seed, the socket choice (RTP socket with multiplexing,
    /// the separate RTCP socket otherwise) and errors are those of
    /// `send_rtcp_bytes`. A datagram the socket would not take now goes
    /// through the same awaited `send_to`, already protected, so its SRTCP
    /// index is consumed exactly once.
    pub(crate) async fn send_rtcp_bytes_now(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        let protected = self.protect_outbound_rtcp(bytes)?;
        let wire = protected.as_deref().unwrap_or(bytes);
        let rtcp_socket = self.rtcp_wire_socket(dest);
        let socket = rtcp_socket.as_deref().unwrap_or(&self.rtp_socket);
        match socket.try_send_to(wire, dest) {
            Ok(_) => {
                crate::executor::record_rtcp_send(true);
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                crate::executor::record_rtcp_send(false);
                socket
                    .send_to(wire, dest)
                    .await
                    .map(|_| ())
                    .map_err(|e| Error::Transport(format!("Failed to send RTCP packet: {}", e)))
            }
            Err(error) => Err(Error::Transport(format!(
                "Failed to send RTCP packet: {}",
                error
            ))),
        }
    }

    /// Send an RTP packet using caller-provided scratch storage for
    /// serialization. Plain RTP writes directly into the scratch buffer;
    /// SRTP keeps crypto-owned output semantics but combines protected
    /// packet bytes and auth tag in one reusable buffer.
    pub async fn send_rtp_with_buffer(
        &self,
        packet: &RtpPacket,
        dest: SocketAddr,
        buffer: &mut BytesMut,
    ) -> Result<()> {
        let protected = {
            let mut srtp_guard = self.srtp_send.lock();
            if let Some(ctx) = srtp_guard.as_mut() {
                Some(ctx.protect(packet)?)
            } else {
                None
            }
        };

        if let Some(protected) = protected {
            let data = protected.serialize_into(buffer)?;
            self.send_rtp_wire_bytes_unchecked(&data, dest).await
        } else if self.secure_media_required.load(Ordering::Acquire) {
            Err(Error::InvalidState(
                "SRTP is required but no send context is installed".to_string(),
            ))
        } else {
            buffer.clear();
            let data = packet.serialize_into(buffer)?;
            self.send_rtp_wire_bytes_unchecked(&data, dest).await
        }
    }
}

/// One RTP payload a caller wants on the wire now (synchronous TX path).
/// The header is the minimal one `RtpPacketSender` builds (`RtpHeader::new`
/// plus the marker), with consecutive sequence numbers assigned by the
/// sender.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TxPayload<'a> {
    pub(crate) payload_type: u8,
    pub(crate) marker: bool,
    pub(crate) timestamp: u32,
    pub(crate) payload: &'a [u8],
}

/// What [`UdpRtpTransport::try_send_rtp_now`] did.
#[derive(Debug, Default)]
pub(crate) struct TxNowOutcome {
    /// Leading packets that left: accepted by the kernel, or refused by the
    /// local stack and lost as on the wire (see [`datagram_refused`]). Their
    /// sequence numbers and SRTP indexes are consumed.
    pub(crate) sent: usize,
    /// Of `sent`, the datagrams the local stack refused.
    pub(crate) refused: u32,
    /// The packet at index `sent` failed after consuming its sequence
    /// number (`send_payload` returned this error for it). Packets after it
    /// were not consumed.
    pub(crate) error: Option<Error>,
    /// Packets after `sent` (and after a failed one) were not consumed: the
    /// socket would block. The caller sends them through the async path,
    /// which assigns them the same sequence numbers and SRTP indexes.
    pub(crate) would_block: bool,
    /// `sendmmsg` calls that carried more than one packet, and those packets.
    pub(crate) sendmmsg_calls: u32,
    pub(crate) sendmmsg_packets: u32,
    /// Some packets went one `sendto` at a time after a short (or
    /// unavailable) `sendmmsg`.
    pub(crate) fallback: bool,
}

impl TxNowOutcome {
    /// Sequence numbers consumed: every sent packet, plus a failed one.
    pub(crate) fn consumed(&self) -> usize {
        self.sent + usize::from(self.error.is_some())
    }
}

/// Where the synchronous TX loop writes. The socket in production; a
/// scripted fake in tests, so the `sendmmsg` fallback rules are
/// deterministic.
pub(crate) trait DatagramWriter {
    /// One non-blocking `sendto`.
    fn send_one(&self, datagram: &[u8], dest: SocketAddr) -> std::io::Result<usize>;
    /// One non-blocking `sendmmsg` of all `datagrams`; `None` where the
    /// platform has none.
    fn send_many(&self, datagrams: &[&[u8]], dest: SocketAddr) -> Option<std::io::Result<usize>>;
}

impl DatagramWriter for UdpSocket {
    fn send_one(&self, datagram: &[u8], dest: SocketAddr) -> std::io::Result<usize> {
        self.try_send_to(datagram, dest)
    }

    #[cfg(target_os = "linux")]
    fn send_many(&self, datagrams: &[&[u8]], dest: SocketAddr) -> Option<std::io::Result<usize>> {
        use std::os::fd::AsRawFd;
        let fd = self.as_raw_fd();
        // `try_io` only runs the closure while Tokio believes the socket is
        // writable, and clears that readiness if it returns `WouldBlock`,
        // exactly as `try_send_to` does.
        Some(self.try_io(tokio::io::Interest::WRITABLE, || {
            crate::sys_mmsg::send_to_many(fd, datagrams, dest)
        }))
    }

    #[cfg(not(target_os = "linux"))]
    fn send_many(&self, _datagrams: &[&[u8]], _dest: SocketAddr) -> Option<std::io::Result<usize>> {
        None
    }
}

/// Send `datagrams` (one call's packets, in wire order) to `dest` without
/// blocking. With `batch` and more than one datagram, one `sendmmsg` first;
/// whatever it did not take goes one `sendto` at a time. Stops at the first
/// `WouldBlock` (the rest are reported unsent) or the first error other than
/// a per-datagram refusal (reported for that datagram). A refused datagram
/// is lost and the rest go on, as on the async path. Order is never changed.
pub(crate) fn write_datagrams_now<W: DatagramWriter + ?Sized>(
    writer: &W,
    datagrams: &[&[u8]],
    dest: SocketAddr,
    batch: bool,
) -> (TxNowOutcome, Option<std::io::Error>) {
    let mut outcome = TxNowOutcome::default();
    let mut next = 0usize;
    if batch && datagrams.len() > 1 {
        let limit = datagrams.len().min(MAX_TX_BATCH);
        match writer.send_many(&datagrams[..limit], dest) {
            Some(Ok(accepted)) => {
                let accepted = accepted.min(limit);
                if accepted > 1 {
                    outcome.sendmmsg_calls = 1;
                    outcome.sendmmsg_packets = accepted as u32;
                }
                next = accepted;
                if next < datagrams.len() {
                    outcome.fallback = true;
                }
            }
            Some(Err(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                outcome.would_block = true;
                return (outcome, None);
            }
            // A hard error from `sendmmsg` belongs to its first datagram;
            // let the per-packet path report it precisely.
            Some(Err(_)) | None => outcome.fallback = true,
        }
    }
    while next < datagrams.len() {
        match writer.send_one(datagrams[next], dest) {
            Ok(_) => next += 1,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                outcome.would_block = true;
                break;
            }
            Err(error) if datagram_refused(&error) => {
                // Consumed and lost, like a network drop; the rest go on.
                debug!(%error, %dest, "RTP datagram refused by the local network stack; counted as lost");
                outcome.refused += 1;
                next += 1;
            }
            Err(error) => {
                outcome.sent = next;
                return (outcome, Some(error));
            }
        }
    }
    outcome.sent = next;
    (outcome, None)
}

/// Most packets one synchronous call protects and sends.
pub(crate) const MAX_TX_BATCH: usize = 16;

/// Hot call state: a packet writer's cached remote RTP address and latch
/// state, valid while the transport's doorbell still reads `bell`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TxDestCache {
    bell: u64,
    dest: Option<SocketAddr>,
    latched: bool,
}

impl TxDestCache {
    pub(crate) fn new() -> Self {
        Self {
            bell: crate::hot_state::NEVER_SEEN,
            dest: None,
            latched: false,
        }
    }
}

impl Default for TxDestCache {
    fn default() -> Self {
        Self::new()
    }
}

impl UdpRtpTransport {
    /// The remote RTP address ([`Self::current_remote_rtp_addr`]) and, with
    /// hot call state, the symmetric-RTP latch state, served from `cache`
    /// while the doorbell has not rung since the cache was filled (the
    /// address's two pointer hops and the latch's own line saved per frame).
    /// Every writer of the address or the latch rings after its store. With
    /// hot state off: the plain address and no latch hint.
    pub(crate) fn remote_rtp_now_cached(
        &self,
        cache: &mut TxDestCache,
    ) -> (Option<SocketAddr>, Option<bool>) {
        let Some(doorbell) = self.hot_doorbell.as_deref() else {
            return (self.current_remote_rtp_addr(), None);
        };
        let bell = doorbell.read();
        if cache.bell == bell {
            if let Some(dest) = cache.dest {
                return (Some(dest), Some(cache.latched));
            }
        }
        let dest = self.current_remote_rtp_addr();
        let latched = self.symmetric_rtp_latched.load(Ordering::Acquire);
        *cache = TxDestCache {
            bell,
            dest,
            latched,
        };
        (dest, Some(latched))
    }

    /// Test seam: whether this transport has a doorbell (hot call state).
    #[cfg(test)]
    pub(crate) fn hot_state_enabled(&self) -> bool {
        self.hot_doorbell.is_some()
    }
}

impl UdpRtpTransport {
    /// Protect and send `packets` (one stream, consecutive sequence numbers
    /// from `first_sequence`, in order) from the caller's thread, without
    /// awaiting: one `try_send_to` per packet, or one `sendmmsg` for a batch
    /// on Linux.
    ///
    /// The caller holds the stream's packet-writer lock, so sequence numbers
    /// and wire order are exactly those of `send_payload`. SRTP state is
    /// committed only for packets that consumed their sequence number (sent,
    /// refused, or failed after protection as the async path fails): a
    /// packet that would block is left unconsumed for the async path. The
    /// `srtp_send` lock is held across the non-blocking system call so no
    /// other protect can take an index in between.
    ///
    /// `None`: this transport cannot send synchronously; nothing was
    /// consumed.
    pub(crate) fn try_send_rtp_now(
        &self,
        packets: &[TxPayload<'_>],
        ssrc: u32,
        first_sequence: u16,
        dest: SocketAddr,
        arena: &mut Vec<u8>,
        batch: bool,
        latched_hint: Option<bool>,
    ) -> Option<TxNowOutcome> {
        let packets = &packets[..packets.len().min(MAX_TX_BATCH)];
        if packets.is_empty() {
            return Some(TxNowOutcome::default());
        }
        // Hot call state: `latched_hint` is the latch state the caller's
        // destination cache read under an unchanged doorbell
        // (`remote_rtp_now_cached`).
        if self.config.symmetric_rtp
            && !latched_hint.unwrap_or_else(|| self.symmetric_rtp_latched.load(Ordering::Acquire))
        {
            // As `send_rtp_wire_bytes_unchecked`: seed the pre-latch target.
            if seed_prelatch_destination(&self.remote_rtp_addr, dest) {
                self.ring_hot_doorbell();
            }
        }
        let mut srtp_guard = self.srtp_send.lock();
        arena.clear();
        let mut ranges: [(usize, usize); MAX_TX_BATCH] = [(0, 0); MAX_TX_BATCH];
        let mut commits = [crate::srtp::OutboundRtpCommit::default(); MAX_TX_BATCH];
        let mut protected = 0usize;
        let mut protect_error = None;
        for (index, packet) in packets.iter().enumerate() {
            let sequence = first_sequence.wrapping_add(index as u16);
            let start = arena.len();
            if let Some(context) = srtp_guard.as_mut() {
                match context.protect_rtp_into(
                    packet.payload_type,
                    packet.marker,
                    sequence,
                    packet.timestamp,
                    ssrc,
                    packet.payload,
                    &commits[..protected],
                    arena,
                ) {
                    Ok(commit) => commits[protected] = commit,
                    Err(error) => {
                        protect_error = Some(error);
                        break;
                    }
                }
            } else if self.secure_media_required.load(Ordering::Acquire) {
                protect_error = Some(Error::InvalidState(
                    "SRTP is required but no send context is installed".to_string(),
                ));
                break;
            } else {
                crate::srtp::write_minimal_rtp_header(
                    arena,
                    packet.payload_type,
                    packet.marker,
                    sequence,
                    packet.timestamp,
                    ssrc,
                );
                arena.extend_from_slice(packet.payload);
            }
            ranges[index] = (start, arena.len());
            protected += 1;
        }
        let mut views: [&[u8]; MAX_TX_BATCH] = [&[]; MAX_TX_BATCH];
        for (view, (start, end)) in views.iter_mut().zip(ranges.iter()).take(protected) {
            *view = &arena[*start..*end];
        }
        let (mut outcome, io_error) =
            write_datagrams_now(&*self.rtp_socket, &views[..protected], dest, batch);
        if outcome.refused > 0 {
            self.refused_rtp_datagrams
                .fetch_add(u64::from(outcome.refused), Ordering::Relaxed);
        }
        if let Some(error) = io_error {
            outcome.error = Some(Error::Transport(format!(
                "Failed to send RTP packet: {}",
                error
            )));
        } else if !outcome.would_block {
            // Everything protected left; a protect failure (the async path:
            // the frame's `send_rtp_with_buffer` error, after its sequence
            // number was taken) is reported for the next packet.
            if let Some(error) = protect_error {
                outcome.error = Some(error);
            }
        }
        // SRTP state: every packet that consumed its sequence number and was
        // protected. A protect failure never committed (the async path
        // neither).
        if let Some(context) = srtp_guard.as_mut() {
            let committed = outcome.consumed().min(protected);
            context.commit_outbound(&commits[..committed]);
        }
        Some(outcome)
    }
}

#[async_trait]
impl RtpTransport for UdpRtpTransport {
    fn media_runtime_handle(&self) -> Option<tokio::runtime::Handle> {
        Some(self.media_runtime.clone())
    }

    async fn send_stun_bytes(&self, payload: &[u8], destination: SocketAddr) -> Result<()> {
        UdpRtpTransport::send_stun_bytes(self, payload, destination).await
    }

    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        self.rtp_socket
            .local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get local RTP address: {}", e)))
    }

    /// Get the local RTCP address
    /// The bound RTCP socket's address while one is open; otherwise the
    /// configured `local_rtcp_addr`, as before separate sockets could close.
    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>> {
        Ok(self
            .local_rtcp_socket_addr()
            .or(self.config.local_rtcp_addr))
    }

    async fn send_rtp(&self, packet: &RtpPacket, dest: SocketAddr) -> Result<()> {
        let mut buffer = BytesMut::with_capacity(packet.size() + 16);
        self.send_rtp_with_buffer(packet, dest, &mut buffer).await
    }

    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.secure_media_required.load(Ordering::Acquire) {
            return Err(Error::InvalidState(
                "raw RTP bytes cannot bypass a secure-media transport".to_string(),
            ));
        }
        self.send_rtp_wire_bytes_unchecked(bytes, dest).await
    }

    async fn send_rtcp(&self, packet: &RtcpPacket, dest: SocketAddr) -> Result<()> {
        // Serialize the packet
        let data = packet.serialize()?;

        // Send the serialized bytes
        self.send_rtcp_bytes(&data, dest).await
    }

    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        match self.protect_outbound_rtcp(bytes)? {
            Some(protected) => self.send_rtcp_wire_bytes_unchecked(&protected, dest).await,
            None => self.send_rtcp_wire_bytes_unchecked(bytes, dest).await,
        }
    }

    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        // Receive data from the RTP socket
        let (size, addr) = self
            .rtp_socket
            .recv_from(buffer)
            .await
            .map_err(|e| Error::Transport(format!("Failed to receive packet: {}", e)))?;

        if !self.secure_media_required.load(Ordering::Acquire) {
            return Ok((size, addr));
        }

        let plaintext = {
            let mut guard = self.srtp_recv.lock();
            if guard.owned_by_receiver() {
                // The transport's own receive loop holds the inbound context
                // (and reads this socket itself).
                return Err(Error::InvalidState(
                    "the inbound SRTP context is owned by the transport's receive loop".to_string(),
                ));
            }
            let context = guard.home.as_mut().ok_or_else(|| {
                Error::InvalidState(
                    "secure media is required but no receive context is installed".to_string(),
                )
            })?;
            match classify_rtp_mux_packet(&buffer[..size]) {
                RtpMuxPacketClass::Rtcp => context.unprotect_rtcp(&buffer[..size])?,
                RtpMuxPacketClass::Rtp => context.unprotect(&buffer[..size])?.serialize()?,
                RtpMuxPacketClass::Stun => {
                    // ICE sits below SRTP: connectivity checks are never
                    // SRTP-wrapped, so a STUN datagram on a secured socket
                    // is forwarded to the agent, not treated as an attack.
                    drop(guard);
                    let _ = self.event_tx.send(RtpEvent::StunPacket {
                        local: self.local_rtp_addr()?,
                        source: addr,
                        payload: Bytes::copy_from_slice(&buffer[..size]),
                    });
                    return Err(Error::InvalidState(
                        "stun datagram forwarded; no media packet".to_string(),
                    ));
                }
                class => {
                    return Err(Error::InvalidPacket(format!(
                        "secure media receive rejected {} datagram",
                        class.as_str()
                    )))
                }
            }
        };
        if plaintext.len() > buffer.len() {
            return Err(Error::BufferTooSmall {
                required: plaintext.len(),
                available: buffer.len(),
            });
        }
        buffer[..plaintext.len()].copy_from_slice(&plaintext);
        Ok((plaintext.len(), addr))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        self.event_tx.subscribe()
    }

    async fn close(&self) -> Result<()> {
        // Stop the receiver task
        self.stop_receiver().await?;

        // If we used the port allocator, release the ports
        if self.config.use_port_allocator
            && self
                .allocator_release_started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            if let Some(session_id) = &self.config.session_id {
                // Get the global allocator
                let allocator = GlobalPortAllocator::instance().await;

                // Release all ports associated with this session
                if let Err(e) = allocator.release_session(session_id).await {
                    warn!("Failed to release ports for session {}: {}", session_id, e);
                } else {
                    debug!("Released all ports for session {}", session_id);
                }
            }
        }

        // UDP sockets don't need explicit closing
        Ok(())
    }
}

impl Drop for UdpRtpTransport {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        self.ring_hot_doorbell();
        if let Ok(mut tasks) = self.receiver_tasks.try_lock() {
            for task in tasks.drain(..) {
                task.abort();
            }
        }
        if let Some(task) = self.rtcp_receiver_task.get_mut().take() {
            task.abort();
        }

        if self.config.use_port_allocator
            && self
                .allocator_release_started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            if let (Some(session_id), Ok(runtime)) = (
                self.config.session_id.clone(),
                tokio::runtime::Handle::try_current(),
            ) {
                runtime.spawn(async move {
                    let allocator = GlobalPortAllocator::instance().await;
                    let _ = allocator.release_session(&session_id).await;
                });
            }
        }
    }
}

/// Determine if a packet is RTCP according to RFC 5761
///
/// RFC 5761 specifies that RTP/RTCP multiplexing uses the following rules to
/// distinguish RTCP from RTP packets:
///
/// 1. Packets with payload types in the range 64-95 could be either RTP or RTCP.
/// 2. The complete RTCP packet-type range 192-223 is reserved from RTP
///    payload-type assignment on a muxed session, including packet types this
///    crate does not interpret yet.
/// 3. All other packets in the range 64-95 are RTP.
/// 4. All packets with payload types in the range 0-63 and 96-127 are RTP.
///
/// See RFC 5761 section 4 for more details.
pub(crate) fn is_rtcp_packet(buffer: &[u8]) -> bool {
    if buffer.len() < 2 {
        return false;
    }

    let first_byte = buffer[0];
    let second_byte = buffer[1];

    let version = (first_byte >> 6) & 0x03;
    // For RTP, payload type is in the lower 7 bits of the second byte
    // For RTCP, packet type is the full second byte value

    // RFC 5761 section 4 reserves the full 192-223 range for RTCP demux.
    if version == 2 && (192..=223).contains(&second_byte) {
        debug!(
            "Identified RTCP packet: version={}, PT={}",
            version, second_byte
        );
        return true;
    }

    // Second check: For ambiguous range (64-95), we need to do additional checks
    let rtp_payload_type = second_byte & 0x7F; // Strip marker bit
    if version == 2 && (rtp_payload_type >= 64 && rtp_payload_type <= 95) {
        // Additional checks could be implemented here
        // For example, checking packet structure specific to RTCP

        // For now, we'll conservatively treat this as RTP
        debug!(
            "Ambiguous packet in PT range 64-95: {}, treating as RTP",
            rtp_payload_type
        );
        return false;
    }

    // If neither condition is met, it's an RTP packet
    debug!(
        "Identified as RTP packet: version={}, PT={}",
        version, rtp_payload_type
    );
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::RtpHeader;
    use bytes::Bytes;

    #[cfg(target_os = "linux")]
    #[test]
    fn media_socket_moves_to_an_independent_epoll_reactor_without_rebinding() {
        use std::os::fd::AsRawFd;
        fn epoll_owners(socket: i32) -> std::collections::BTreeSet<String> {
            // fdinfo keeps the descriptor number used at registration. That
            // number can be reused while a duplicated old socket lives, so
            // match the socket inode too and never count another socket.
            let target = std::fs::read_link(format!("/proc/self/fd/{socket}")).unwrap();
            let target = target.to_string_lossy();
            let inode = target
                .strip_prefix("socket:[")
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|inode| inode.parse::<u64>().ok())
                .unwrap();
            std::fs::read_dir("/proc/self/fdinfo")
                .unwrap()
                .filter_map(|entry| {
                    let info = std::fs::read_to_string(entry.ok()?.path()).ok()?;
                    let owns_socket = info.lines().any(|line| {
                        let words = line.split_whitespace().collect::<Vec<_>>();
                        words.first() == Some(&"tfd:")
                            && words.get(1).and_then(|word| word.parse::<i32>().ok())
                                == Some(socket)
                            && line
                                .split_once("ino:")
                                .and_then(|(_, tail)| tail.split_whitespace().next())
                                .and_then(|value| u64::from_str_radix(value, 16).ok())
                                == Some(inode)
                    });
                    // Mio clones its registry by duplicating the epoll
                    // descriptor, and both aliases show the same kernel watch
                    // list: deduplicate the list, not the descriptor numbers.
                    owns_socket.then(|| {
                        let mut watches = info
                            .lines()
                            .filter(|line| line.starts_with("tfd:"))
                            .collect::<Vec<_>>();
                        watches.sort_unstable();
                        watches.join("\n")
                    })
                })
                .collect()
        }

        let signaling = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let lane = || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap()
        };
        let (lane_a, lane_b) = (lane(), lane());
        signaling.block_on(async {
            let a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            a.set_ttl(47).unwrap();
            a.set_broadcast(true).unwrap();
            let (address, descriptor) = (a.local_addr().unwrap(), a.as_raw_fd());
            let initial = epoll_owners(descriptor);
            assert_eq!(initial.len(), 1);

            let a = register_socket_on_media(a, lane_a.handle()).unwrap();
            let b = register_socket_on_media(b, lane_b.handle()).unwrap();
            assert_eq!(a.local_addr().unwrap(), address);
            assert_eq!(a.as_raw_fd(), descriptor);
            assert_eq!(a.ttl().unwrap(), 47);
            assert!(a.broadcast().unwrap());
            let a_owner = epoll_owners(a.as_raw_fd());
            let b_owner = epoll_owners(b.as_raw_fd());
            assert_eq!(a_owner.len(), 1);
            assert_eq!(b_owner.len(), 1);
            assert_ne!(
                a_owner, initial,
                "registration must leave the signaling reactor"
            );
            assert_ne!(
                a_owner, b_owner,
                "independent lanes need independent reactors"
            );

            tokio::time::timeout(Duration::from_secs(1), async {
                for _ in 0..32 {
                    a.send_to(b"mux-media", b.local_addr().unwrap())
                        .await
                        .unwrap();
                    let mut payload = [0; 32];
                    let (length, source) = b.recv_from(&mut payload).await.unwrap();
                    assert_eq!(&payload[..length], b"mux-media");
                    assert_eq!(source, address);
                }
            })
            .await
            .unwrap();
        });
    }

    #[test]
    fn transport_sockets_and_receivers_outlive_the_creating_runtime() {
        // The transport's sockets are served by its media lane, so shutting
        // down the (signaling) runtime that created it must not silence its
        // receive tasks, for RTP or for a separate RTCP socket.
        let signaling = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let transport = signaling.block_on(async {
            UdpRtpTransport::new(RtpTransportConfig {
                local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
                local_rtcp_addr: None,
                symmetric_rtp: false,
                rtcp_mux: false,
                session_id: Some("lane_owned_sockets".to_string()),
                use_port_allocator: false,
                buffer_config: Default::default(),
            })
            .await
            .unwrap()
        });
        drop(signaling);

        let rtp_addr = transport.local_rtp_addr().unwrap();
        let rtcp_addr = transport
            .local_rtcp_socket_addr()
            .expect("separate RTCP socket");
        let mut events = transport.subscribe();
        let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let rtp = RtpPacket::new(
            RtpHeader::new(0, 7, 160, 0x1234_5678),
            Bytes::from_static(b"lane"),
        );
        peer.send_to(&rtp.serialize().unwrap(), rtp_addr).unwrap();
        let rtcp = [0x80, 201, 0x00, 0x01, 0x11, 0x22, 0x33, 0x44];
        peer.send_to(&rtcp, rtcp_addr).unwrap();

        let observer = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let received = observer.block_on(async {
            let (mut media, mut control) = (false, false);
            // One deadline for both: a receive loop whose socket lost its
            // reactor reports errors instead of packets.
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                while !(media && control) {
                    match events.recv().await.expect("event") {
                        RtpEvent::MediaReceived { payload, .. } => {
                            assert_eq!(&payload[..], b"lane");
                            media = true;
                        }
                        RtpEvent::RtcpReceived { data, .. } => {
                            assert_eq!(&data[..], &rtcp);
                            control = true;
                        }
                        _ => {}
                    }
                }
            })
            .await;
            (media, control)
        });
        assert_eq!(
            received,
            (true, true),
            "RTP and RTCP keep arriving after the creating runtime stopped"
        );
        drop(transport);
    }

    #[test]
    fn classify_rtp_mux_packet_demuxes_known_ranges() {
        let rtp = RtpPacket::new(
            RtpHeader::new(0, 1, 160, 0x1122_3344),
            Bytes::from_static(b"payload"),
        )
        .serialize()
        .expect("rtp serializes");
        assert_eq!(classify_rtp_mux_packet(&rtp), RtpMuxPacketClass::Rtp);

        let rtcp_sr = [
            0x80, 200, 0x00, 0x06, 0x11, 0x22, 0x33, 0x44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(classify_rtp_mux_packet(&rtcp_sr), RtpMuxPacketClass::Rtcp);

        let rtcp_rr = [0x80, 201, 0x00, 0x01, 0x11, 0x22, 0x33, 0x44];
        assert_eq!(classify_rtp_mux_packet(&rtcp_rr), RtpMuxPacketClass::Rtcp);

        let stun_binding = [
            0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xa4, 0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(
            classify_rtp_mux_packet(&stun_binding),
            RtpMuxPacketClass::Stun
        );

        assert_eq!(
            classify_rtp_mux_packet(&[22, 0xfe, 0xfd, 0, 0]),
            RtpMuxPacketClass::Dtls
        );
        assert_eq!(
            classify_rtp_mux_packet(&[0x10, 0, 0, 0]),
            RtpMuxPacketClass::Zrtp
        );
        assert_eq!(
            classify_rtp_mux_packet(&[0x40, 0, 0, 0]),
            RtpMuxPacketClass::TurnChannelData
        );
        assert_eq!(
            classify_rtp_mux_packet(&[0x80]),
            RtpMuxPacketClass::TooSmall
        );
        assert_eq!(
            classify_rtp_mux_packet(&[0x50, 0]),
            RtpMuxPacketClass::UnknownReserved
        );
    }

    #[test]
    fn classify_ascii_sip_methods_as_non_rtp() {
        for method in [
            b"INVITE sip:bob@example.com SIP/2.0\r\n".as_slice(),
            b"ACK sip:bob@example.com SIP/2.0\r\n".as_slice(),
            b"BYE sip:bob@example.com SIP/2.0\r\n".as_slice(),
        ] {
            assert_eq!(
                classify_rtp_mux_packet(method),
                RtpMuxPacketClass::TurnChannelData,
                "ASCII SIP methods begin in the RFC 7983 TURN ChannelData range and must not be parsed as RTP"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn only_per_datagram_refusals_count_as_loss() {
        for refused in [
            std::io::Error::from_raw_os_error(libc::EPERM),
            std::io::Error::from_raw_os_error(libc::EACCES),
            std::io::Error::from_raw_os_error(libc::ENOBUFS),
            std::io::Error::from_raw_os_error(libc::ECONNREFUSED),
            std::io::Error::from_raw_os_error(libc::EHOSTUNREACH),
            std::io::Error::from_raw_os_error(libc::ENETUNREACH),
            std::io::Error::from_raw_os_error(libc::EHOSTDOWN),
        ] {
            assert!(datagram_refused(&refused), "{refused:?}");
        }
        for fatal in [
            std::io::Error::from_raw_os_error(libc::EBADF),
            std::io::Error::from_raw_os_error(libc::EINVAL),
            std::io::Error::from_raw_os_error(libc::EMSGSIZE),
            std::io::Error::from(std::io::ErrorKind::WouldBlock),
        ] {
            assert!(!datagram_refused(&fatal), "{fatal:?}");
        }
    }

    #[tokio::test]
    async fn a_refused_rtp_datagram_is_lost_not_a_send_error() {
        // The local stack refuses this one datagram. A broadcast destination
        // without SO_BROADCAST is refused with EACCES, the same errno a
        // netfilter DROP rule in OUTPUT returns. Its sequence number is
        // already consumed, so the far end sees one lost packet; failing the
        // send instead ended the caller's outbound audio for the rest of the
        // call. Skip where the host does not refuse the probe.
        let refused: SocketAddr = "255.255.255.255:9".parse().unwrap();
        let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        match probe.send_to(b"probe", refused) {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
            other => {
                eprintln!("skipping: this host does not refuse the broadcast probe ({other:?})");
                return;
            }
        }
        let config = RtpTransportConfig {
            local_rtp_addr: "0.0.0.0:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: false,
            rtcp_mux: true,
            session_id: Some("refused-datagram".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let transport = UdpRtpTransport::new(config).await.unwrap();
        transport
            .send_rtp_bytes(b"\x80\x00\x00\x01payload", refused)
            .await
            .expect("a refused datagram is lost on the wire, not a failed send");
        assert_eq!(transport.refused_rtp_datagrams(), 1);
        // The stream goes on: the next datagram leaves normally.
        let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        transport
            .send_rtp_bytes(b"\x80\x00\x00\x02payload", sink.local_addr().unwrap())
            .await
            .unwrap();
        let mut received = [0u8; 64];
        let (len, _) = tokio::time::timeout(Duration::from_secs(1), sink.recv_from(&mut received))
            .await
            .expect("the next datagram arrives")
            .unwrap();
        assert_eq!(&received[..len], b"\x80\x00\x00\x02payload");
    }

    #[test]
    fn prelatch_seed_stores_only_a_changed_destination() {
        let slot = ArcSwapOption::<SocketAddr>::from(None);
        let a: SocketAddr = "192.0.2.10:4000".parse().unwrap();
        let b: SocketAddr = "192.0.2.11:4002".parse().unwrap();
        assert!(seed_prelatch_destination(&slot, a));
        let first = slot.load_full().expect("seeded");
        assert!(!seed_prelatch_destination(&slot, a));
        // The equal re-seed kept the very same allocation: no store happened.
        assert!(Arc::ptr_eq(&first, &slot.load_full().expect("kept")));
        assert!(seed_prelatch_destination(&slot, b));
        assert_eq!(slot.load().as_deref(), Some(&b));
    }

    #[tokio::test]
    async fn unlatched_sends_do_not_restore_an_equal_destination() {
        // Before a validated inbound tuple latches, every send seeds the
        // pre-latch destination. Re-storing an equal value cost a fresh Arc
        // and arc_swap's writer path per packet, for the whole call when the
        // peer never sends media.
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("prelatch-seed".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let transport = UdpRtpTransport::new(config).await.unwrap();
        let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let dest = sink.local_addr().unwrap();
        transport.set_remote_rtp_addr(dest).await;
        transport.set_remote_rtcp_addr(dest).await;
        let seeded_rtp = transport
            .remote_rtp_addr
            .load_full()
            .expect("SDP destination");
        let seeded_rtcp = transport
            .remote_rtcp_addr
            .load_full()
            .expect("SDP destination");
        for _ in 0..8 {
            transport
                .send_rtp_bytes(b"\x80\x00\x00\x01payload", dest)
                .await
                .unwrap();
            transport
                .send_rtcp_bytes(&[0x80, 201, 0x00, 0x01, 0x11, 0x22, 0x33, 0x44], dest)
                .await
                .unwrap();
        }
        assert!(
            Arc::ptr_eq(&seeded_rtp, &transport.remote_rtp_addr.load_full().unwrap()),
            "an unlatched RTP send to the same destination must not re-store it"
        );
        assert!(
            Arc::ptr_eq(
                &seeded_rtcp,
                &transport.remote_rtcp_addr.load_full().unwrap()
            ),
            "an unlatched RTCP send to the same destination must not re-store it"
        );
        // A send to another destination still seeds it, as before.
        let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let other_addr = other.local_addr().unwrap();
        transport
            .send_rtp_bytes(b"\x80\x00\x00\x02payload", other_addr)
            .await
            .unwrap();
        assert_eq!(transport.remote_rtp_addr().await, Some(other_addr));
    }

    fn hot_config(session: &str) -> RtpTransportConfig {
        RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some(session.to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        }
    }

    /// Hot call state: a transport has a doorbell only when it was built
    /// with hot state on (v2), and every control-plane store the receive loop
    /// or the TX destination cache checks rings it after the store; a
    /// pre-latch seed of an unchanged destination does not.
    #[tokio::test]
    async fn every_control_plane_writer_rings_the_hot_state_doorbell() {
        for config in [
            crate::executor::MediaExecutorConfig::v1().with_hot_state(true),
            crate::executor::MediaExecutorConfig::v2().with_hot_state(false),
        ] {
            let _config = crate::executor::override_media_executor_config_on_this_thread(config);
            let transport = UdpRtpTransport::new(hot_config("hot-off")).await.unwrap();
            assert!(!transport.hot_state_enabled());
        }
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        let transport = UdpRtpTransport::new(hot_config("hot-doorbell"))
            .await
            .unwrap();
        assert!(transport.hot_state_enabled());
        let bell = || transport.hot_doorbell.as_deref().unwrap().read();
        let mut last = bell();
        let mut rang = |what: &str, expected: bool| {
            let now = bell();
            assert_eq!(now > last, expected, "{what}");
            last = now;
        };
        let signalled = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let a = signalled.local_addr().unwrap();
        let b = other.local_addr().unwrap();
        transport.set_remote_rtp_addr(a).await;
        rang("a signalled destination", true);
        transport
            .send_rtp_bytes(b"\x80\x00\x00\x01payload", a)
            .await
            .unwrap();
        rang("a pre-latch seed of the same destination", false);
        transport
            .send_rtp_bytes(b"\x80\x00\x00\x02payload", b)
            .await
            .unwrap();
        rang("a pre-latch seed of a new destination", true);
        // The receive loop latches an inbound source.
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let packet = RtpPacket::new(
            crate::packet::RtpHeader::new(0, 1, 160, 0x0bad_f00d),
            Bytes::from_static(b"latch me"),
        );
        peer.send_to(
            &packet.serialize().unwrap(),
            transport.local_rtp_addr().unwrap(),
        )
        .await
        .unwrap();
        let latched = peer.local_addr().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while transport.current_remote_rtp_addr() != Some(latched) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the receive loop latches the inbound source");
        rang("a latch", true);
        transport
            .send_rtp_bytes(b"\x80\x00\x00\x03payload", b)
            .await
            .unwrap();
        rang("a send after the latch seeds nothing", false);
        let (send, recv) = make_srtp_ctx_pair();
        let rollback = transport.replace_srtp_contexts(send, recv).await.unwrap();
        rang("an SRTP installation", true);
        transport.rollback_srtp_contexts(rollback).await.unwrap();
        rang("an SRTP rollback", true);
        transport.stop_receiver().await.unwrap();
        rang("stopping the receiver", true);
    }

    /// Hot call state: on a current-thread runtime (a dedicated media lane)
    /// the receive loop keeps its control entry in the lane's slab, and
    /// closing the transport releases it.
    #[tokio::test]
    async fn a_hot_receive_loop_keeps_its_entry_in_the_lane_slab_until_close() {
        let _local = crate::task_runtime::MediaTasksOnCurrentRuntime::enter();
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        let before = super::super::rx_hot::lane_rx_entries();
        let transport = UdpRtpTransport::new(hot_config("hot-lane-entry"))
            .await
            .unwrap();
        let mut events = transport.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let packet = RtpPacket::new(
            crate::packet::RtpHeader::new(0, 7, 1120, 0x0102_0304),
            Bytes::from_static(b"lane payload"),
        );
        peer.send_to(
            &packet.serialize().unwrap(),
            transport.local_rtp_addr().unwrap(),
        )
        .await
        .unwrap();
        match tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
            Ok(Ok(RtpEvent::MediaReceived {
                sequence_number, ..
            })) => assert_eq!(sequence_number, 7),
            other => panic!("expected MediaReceived, got {other:?}"),
        }
        assert_eq!(
            super::super::rx_hot::lane_rx_entries(),
            before + 1,
            "the loop's entry is in this lane's slab"
        );
        transport.close().await.unwrap();
        assert_eq!(
            super::super::rx_hot::lane_rx_entries(),
            before,
            "close released the entry"
        );
    }

    #[tokio::test]
    async fn test_udp_transport_creation() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: true,
            rtcp_mux: false, // Disable RTCP-MUX for this test
            session_id: Some("test_creation".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport = UdpRtpTransport::new(config).await;
        assert!(transport.is_ok());

        let transport = transport.unwrap();
        let rtp_addr = transport.local_rtp_addr().unwrap();

        // For non-muxed connections, we should get assigned a real RTCP socket
        assert_ne!(rtp_addr.port(), 0);
        assert!(
            transport.rtcp_socket.load().is_some(),
            "RTCP socket should exist when rtcp_mux is false"
        );

        // Check the actual RTCP socket address, not just the config value
        if let Some(rtcp_socket) = transport.rtcp_socket.load_full() {
            let rtcp_addr = rtcp_socket.local_addr().unwrap();
            assert_ne!(rtcp_addr.port(), 0);
            assert_ne!(rtp_addr.port(), rtcp_addr.port());
        }
    }

    #[tokio::test]
    async fn test_udp_transport_with_rtcp_mux() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()), // This should be ignored
            symmetric_rtp: true,
            rtcp_mux: true, // Enable RTCP-MUX
            session_id: Some("test_rtcp_mux".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport = UdpRtpTransport::new(config).await;
        assert!(transport.is_ok());

        let transport = transport.unwrap();
        let rtp_addr = transport.local_rtp_addr().unwrap();

        assert_ne!(rtp_addr.port(), 0, "RTP port should not be 0");

        // With RTCP-MUX, no separate RTCP socket should be created
        assert!(
            transport.rtcp_socket.load().is_none(),
            "RTCP socket should be None with rtcp_mux enabled"
        );

        // The config should retain the original RTCP address - it doesn't matter
        // what this is with RTCP-MUX as it's not used
        let rtcp_addr_option = transport.local_rtcp_addr().unwrap();
        assert!(
            rtcp_addr_option.is_some(),
            "RTCP address should be available in the config"
        );
    }

    #[tokio::test]
    async fn test_rtcp_packet_detection() {
        // Test RTCP detection with SR packet (PT=200)
        let mut sr_packet = vec![0x80, 200, 0, 0]; // Version=2, PT=200 (SR)
        sr_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&sr_packet));

        // Test RTCP detection with RR packet (PT=201)
        let mut rr_packet = vec![0x80, 201, 0, 0]; // Version=2, PT=201 (RR)
        rr_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&rr_packet));

        // Test RTCP detection with SDES packet (PT=202)
        let mut sdes_packet = vec![0x80, 202, 0, 0]; // Version=2, PT=202 (SDES)
        sdes_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&sdes_packet));

        // Test RTCP detection with BYE packet (PT=203)
        let mut bye_packet = vec![0x80, 203, 0, 0]; // Version=2, PT=203 (BYE)
        bye_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&bye_packet));

        // Test RTCP detection with APP packet (PT=204)
        let mut app_packet = vec![0x80, 204, 0, 0]; // Version=2, PT=204 (APP)
        app_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&app_packet));

        // Unknown RTCP types in the RFC 5761 reserved range must still route
        // through the tolerant RTCP/SRTCP parser.
        assert!(is_rtcp_packet(&[0x80, 192, 0, 1, 0, 0, 0, 1]));
        assert!(is_rtcp_packet(&[0x80, 208, 0, 1, 0, 0, 0, 1]));
        assert!(is_rtcp_packet(&[0x80, 223, 0, 1, 0, 0, 0, 1]));

        // Test regular RTP packet (PT=0)
        let mut rtp_packet = vec![0x80, 0, 0, 0]; // Version=2, PT=0 (PCMU)
        rtp_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(!is_rtcp_packet(&rtp_packet));

        // Test regular RTP packet with marker bit (PT=0, M=1)
        let mut rtp_packet_with_marker = vec![0x80, 0x80, 0, 0]; // Version=2, PT=0, M=1
        rtp_packet_with_marker.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(!is_rtcp_packet(&rtp_packet_with_marker));

        // Test regular RTP packet (PT=96, common for dynamic codecs)
        let mut rtp_dynamic_packet = vec![0x80, 96, 0, 0]; // Version=2, PT=96
        rtp_dynamic_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(!is_rtcp_packet(&rtp_dynamic_packet));
        assert!(!is_rtcp_packet(&[0x80, 224, 0, 1, 0, 0, 0, 1]));
    }

    #[tokio::test]
    async fn test_udp_transport_packet_send() {
        // Create two transport instances
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_send1".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_send2".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let transport2 = UdpRtpTransport::new(config2).await.unwrap();

        // Create a test packet
        let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new(header, payload);

        // Send from transport1 to transport2
        let addr2 = transport2.local_rtp_addr().unwrap();
        let result = transport1.send_rtp(&packet, addr2).await;
        assert!(result.is_ok());

        // Check if remote address was updated in transport1
        let remote_addr = transport1.remote_rtp_addr().await;
        assert_eq!(remote_addr, Some(addr2));
    }

    #[tokio::test]
    async fn symmetric_rtp_latches_valid_source_and_bounds_rebinding() {
        async fn send_packet(socket: &UdpSocket, destination: SocketAddr, seq: u16, ssrc: u32) {
            let packet = RtpPacket::new(
                RtpHeader::new(0, seq, u32::from(seq) * 160, ssrc),
                Bytes::from_static(b"audio"),
            )
            .serialize()
            .expect("serialize RTP");
            socket
                .send_to(&packet, destination)
                .await
                .expect("send RTP");
        }

        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("symmetric-rebind".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let policy = SymmetricRtpPolicy {
            probation_packets: 3,
            max_rebindings: 1,
            ..SymmetricRtpPolicy::default()
        };
        let transport = UdpRtpTransport::new_with_symmetric_rtp_policy(config, policy)
            .await
            .unwrap();
        let destination = transport.local_rtp_addr().unwrap();
        let original = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rebound = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rejected = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let original_addr = original.local_addr().unwrap();
        let rebound_addr = rebound.local_addr().unwrap();
        let mut events = transport.subscribe();

        // SDP seeds an address, but the first parsed RTP packet is the
        // authoritative symmetric tuple.
        transport.set_remote_rtp_addr(original_addr).await;
        send_packet(&original, destination, 100, 0x1020_3040).await;
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("initial RTP timeout")
            .expect("initial RTP event");
        assert_eq!(transport.remote_rtp_addr().await, Some(original_addr));

        // Two packets cannot redirect the call; they remain in probation and
        // are not delivered as media.
        send_packet(&rebound, destination, 101, 0x1020_3040).await;
        send_packet(&rebound, destination, 102, 0x1020_3040).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(80), events.recv())
                .await
                .is_err()
        );
        assert_eq!(transport.remote_rtp_addr().await, Some(original_addr));

        // The third consecutive, sequence-plausible packet completes the
        // bounded same-IP rebind.
        send_packet(&rebound, destination, 103, 0x1020_3040).await;
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("rebound RTP timeout")
            .expect("rebound RTP event");
        assert_eq!(transport.remote_rtp_addr().await, Some(rebound_addr));

        // max_rebindings=1: a later source cannot move the established tuple.
        send_packet(&rejected, destination, 104, 0x1020_3040).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(80), events.recv())
                .await
                .is_err()
        );
        assert_eq!(transport.remote_rtp_addr().await, Some(rebound_addr));

        let diagnostics = transport.symmetric_rtp_diagnostics();
        assert!(diagnostics.destination_learned);
        assert_eq!(diagnostics.accepted_rebindings, 1);
        assert_eq!(diagnostics.probation_packets, 2);
        assert_eq!(diagnostics.rejected_packets, 1);
        let rendered = format!("{diagnostics:?}");
        assert!(!rendered.contains("127.0.0.1"));
        assert!(!rendered.contains("10203040"));
    }

    #[tokio::test]
    async fn close_aborts_both_rtp_and_separate_rtcp_receivers() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("receiver-drain".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let transport = UdpRtpTransport::new(config).await.unwrap();
        assert_eq!(transport.receiver_tasks.lock().await.len(), 1);
        assert!(transport.rtcp_receiver_task.lock().is_some());

        transport.close().await.unwrap();

        assert!(transport.receiver_tasks.lock().await.is_empty());
        assert!(transport.rtcp_receiver_task.lock().is_none());
        assert!(!transport.active.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn test_udp_transport_event_subscription() {
        // Create two transport instances
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_event1".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_event2".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let transport2 = UdpRtpTransport::new(config2).await.unwrap();

        // Subscribe to events on transport2
        let mut events = transport2.subscribe();

        // Create a test packet
        let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"test payload");
        let mut packet = RtpPacket::new(header, payload.clone());
        packet.set_padding(4);

        // Send from transport1 to transport2
        let addr2 = transport2.local_rtp_addr().unwrap();
        transport1.send_rtp(&packet, addr2).await.unwrap();

        // Give some time for the packet to be processed
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Try to receive the event
        match tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await {
            Ok(Ok(event)) => match event {
                RtpEvent::MediaReceived {
                    payload_type,
                    timestamp,
                    marker,
                    payload: received_payload,
                    source,
                    ..
                } => {
                    assert_eq!(payload_type, 96);
                    assert_eq!(timestamp, 12345);
                    assert_eq!(marker, false);
                    assert_eq!(&received_payload[..], &payload[..]);
                    assert_eq!(source, transport1.local_rtp_addr().unwrap());
                }
                _ => panic!("Unexpected event type: {:?}", event),
            },
            Ok(Err(e)) => panic!("Failed to receive event: {}", e),
            Err(_) => panic!("Timeout waiting for event"),
        }
    }

    #[tokio::test]
    async fn udp_transport_drops_non_rtp_and_malformed_rtp_without_media_event() {
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("drop_sender".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("drop_receiver".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let transport2 = UdpRtpTransport::new(config2).await.unwrap();
        let receiver_addr = transport2.local_rtp_addr().unwrap();
        let mut events = transport2.subscribe();

        let raw_sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        raw_sender
            .send_to(b"INVITE sip:bob@example.com SIP/2.0\r\n", receiver_addr)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(tokio::time::Duration::from_millis(150), events.recv())
                .await
                .is_err(),
            "ASCII SIP datagrams on an RTP socket must be dropped, not emitted as media"
        );

        let malformed_rtp = [0x8f, 0x00, 0x00, 0x01, 0, 0, 0, 1, 0x12, 0x34, 0x56, 0x78];
        raw_sender
            .send_to(&malformed_rtp, receiver_addr)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(tokio::time::Duration::from_millis(150), events.recv())
                .await
                .is_err(),
            "RTP-looking malformed datagrams must be dropped, not emitted as fallback media"
        );

        let header = RtpHeader::new(0, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"valid payload");
        let packet = RtpPacket::new(header, payload.clone());
        transport1.send_rtp(&packet, receiver_addr).await.unwrap();

        match tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await {
            Ok(Ok(RtpEvent::MediaReceived {
                payload_type,
                timestamp,
                payload: received_payload,
                ..
            })) => {
                assert_eq!(payload_type, 0);
                assert_eq!(timestamp, 12345);
                assert_eq!(&received_payload[..], &payload[..]);
            }
            other => panic!("expected valid RTP after drops, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_pt101_dispatch_as_dtmf_event() {
        // RFC 4733: payload-type 101 should surface as `RtpEvent::DtmfEvent`
        // rather than a generic `MediaReceived`, so media-core doesn't
        // try to feed DTMF bytes through a PCMU/Opus decoder.
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("dtmf_sender".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("dtmf_receiver".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let sender = UdpRtpTransport::new(config1).await.unwrap();
        let receiver = UdpRtpTransport::new(config2).await.unwrap();

        let mut events = receiver.subscribe();

        // Build a 4-byte RFC 4733 telephone-event payload encoding
        // digit '5' (event=5), end-of-event=true, volume=10, duration=800.
        let payload = Bytes::from_static(&[
            0x05,        // event=5 ('5')
            0x80 | 0x0A, // E=1 | volume=10 (R bit = 0)
            0x03,
            0x20, // duration=800
        ]);
        let header = RtpHeader::new(101, 1000, 0xAABBCCDD, 0x12345678);
        let packet = RtpPacket::new(header, payload);

        let recv_addr = receiver.local_rtp_addr().unwrap();
        sender.send_rtp(&packet, recv_addr).await.unwrap();

        // Expect DtmfEvent, not MediaReceived.
        let evt = tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv())
            .await
            .expect("event must arrive")
            .expect("broadcast channel open");

        match evt {
            RtpEvent::DtmfEvent {
                event,
                end_of_event,
                volume,
                duration,
                timestamp,
                ssrc,
                ..
            } => {
                assert_eq!(event, 5);
                assert!(end_of_event, "E bit set");
                assert_eq!(volume, 10);
                assert_eq!(duration, 800);
                assert_eq!(timestamp, 0xAABBCCDD);
                assert_eq!(ssrc, 0x12345678);
            }
            other => panic!("expected DtmfEvent, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_separate_rtcp_socket_creation() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("test1".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport = UdpRtpTransport::new(config).await.unwrap();

        let rtp_addr = transport.local_rtp_addr().unwrap();
        assert_ne!(rtp_addr.port(), 0, "RTP port should not be 0");

        // Check that a separate RTCP socket was created
        assert!(
            transport.rtcp_socket.load().is_some(),
            "RTCP socket should be created"
        );

        // Check the actual RTCP socket address, not just the config value
        if let Some(rtcp_socket) = transport.rtcp_socket.load_full() {
            let rtcp_addr = rtcp_socket.local_addr().unwrap();
            assert_ne!(rtcp_addr.port(), 0, "RTCP port should not be 0");
            assert_ne!(
                rtp_addr.port(),
                rtcp_addr.port(),
                "RTP and RTCP ports should be different"
            );
        }
    }

    #[tokio::test]
    async fn separate_rtcp_socket_receive_emits_pooled_bytes_event() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("test_rtcp_receive".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport = UdpRtpTransport::new(config).await.unwrap();
        let rtcp_addr = transport
            .rtcp_socket
            .load_full()
            .expect("separate RTCP socket")
            .local_addr()
            .unwrap();
        let mut events = transport.subscribe();
        transport.start_receiver().await.unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let packet = Bytes::from_static(&[0x80, 201, 0x00, 0x01, 0x11, 0x22, 0x33, 0x44]);
        sender.send_to(&packet, rtcp_addr).await.unwrap();

        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("rtcp event timeout")
            .expect("rtcp event");

        match event {
            RtpEvent::RtcpReceived { data, source } => {
                assert_eq!(data, packet);
                assert_eq!(source, sender.local_addr().unwrap());
            }
            other => panic!("expected RtcpReceived, got {other:?}"),
        }

        transport.stop_receiver().await.unwrap();
    }

    #[tokio::test]
    async fn test_rtcp_mux_socket_creation() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None, // Should be ignored with mux
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test2".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport = UdpRtpTransport::new(config).await.unwrap();

        let rtp_addr = transport.local_rtp_addr().unwrap();
        assert_ne!(rtp_addr.port(), 0, "RTP port should not be 0");

        // With RTCP mux, no separate RTCP socket should be created
        assert!(
            transport.rtcp_socket.load().is_none(),
            "No RTCP socket should be created with rtcp_mux"
        );

        // For RTCP mux, the config does not need to have an RTCP address since it uses the RTP address
        // As long as this doesn't panic, this is sufficient
        let _rtcp_addr_option = transport.local_rtcp_addr();
    }

    #[tokio::test]
    async fn test_separate_socket_bind_conflicts() {
        // First transport
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("test_conflict1".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let rtp_addr1 = transport1.local_rtp_addr().unwrap();
        let rtcp_addr1 = transport1
            .local_rtcp_addr()
            .unwrap()
            .expect("RTCP address should be available");

        // Second transport with specific ports
        let config2 = RtpTransportConfig {
            // Try to bind to the same ports as the first transport
            local_rtp_addr: SocketAddr::new(rtp_addr1.ip(), rtp_addr1.port()),
            local_rtcp_addr: Some(SocketAddr::new(rtcp_addr1.ip(), rtcp_addr1.port())),
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("test_conflict2".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        // This should fail because the ports are already in use
        let result = UdpRtpTransport::new(config2).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_muxed_socket_bind_conflicts() {
        // First transport with RTCP mux
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_mux_conflict1".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let rtp_addr1 = transport1.local_rtp_addr().unwrap();

        // Second transport trying to use the same port
        let config2 = RtpTransportConfig {
            local_rtp_addr: rtp_addr1,
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_mux_conflict2".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };

        // This should fail because the port is already in use
        let result = UdpRtpTransport::new(config2).await;
        assert!(result.is_err());
    }

    // -------- SRTP wrapping (Step 2B.2) ---------------------------------

    /// Build a matched pair of `SrtpContext`s from a single shared
    /// master key. Each side has its own context (mutates per-packet
    /// state independently) but they derive identical keystreams from
    /// the same master + per-packet inputs (RFC 3711 §4.3) so what
    /// one encrypts the other can decrypt.
    fn make_srtp_ctx_pair() -> (crate::srtp::SrtpContext, crate::srtp::SrtpContext) {
        use crate::srtp::{SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
        let key = vec![1u8; 16];
        let salt = vec![2u8; 14];
        let a = crate::srtp::SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(key.clone(), salt.clone()),
        )
        .expect("ctx A");
        let b =
            crate::srtp::SrtpContext::new(SRTP_AES128_CM_SHA1_80, SrtpCryptoKey::new(key, salt))
                .expect("ctx B");
        (a, b)
    }

    fn make_aes256_srtp_ctx_pair() -> (crate::srtp::SrtpContext, crate::srtp::SrtpContext) {
        use crate::srtp::{SrtpCryptoKey, SRTP_AES256_CM_SHA1_80};
        let key = vec![3u8; 32];
        let salt = vec![4u8; 14];
        let a = crate::srtp::SrtpContext::new(
            SRTP_AES256_CM_SHA1_80,
            SrtpCryptoKey::new(key.clone(), salt.clone()),
        )
        .expect("AES-256 ctx A");
        let b =
            crate::srtp::SrtpContext::new(SRTP_AES256_CM_SHA1_80, SrtpCryptoKey::new(key, salt))
                .expect("AES-256 ctx B");
        (a, b)
    }

    fn test_receiver_report(ssrc: u32) -> RtcpPacket {
        RtcpPacket::ReceiverReport(crate::packet::rtcp::RtcpReceiverReport::new(ssrc))
    }

    async fn make_srtp_rollback_transport(session_id: &str) -> UdpRtpTransport {
        UdpRtpTransport::new(RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: false,
            rtcp_mux: true,
            session_id: Some(session_id.to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        })
        .await
        .unwrap()
    }

    /// Owner RTCP: the synchronous report send puts exactly the bytes on
    /// the wire that the async `send_rtcp_bytes` does (plain RTCP, and SRTCP
    /// with consecutive indexes), in the same order and from the same socket:
    /// the RTP socket with multiplexing, the separate RTCP socket without.
    #[tokio::test]
    async fn synchronous_rtcp_send_is_byte_identical_to_the_async_send() {
        let before = crate::executor::inline_executor_counters();
        let report = crate::packet::rtcp::RtcpCompoundPacket::new_with_rr(
            crate::packet::rtcp::RtcpReceiverReport::new(0x0102_0304),
        )
        .serialize()
        .unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = peer.local_addr().unwrap();
        // ((separate RTCP socket, synchronous, secure), datagram)
        let mut wires: Vec<((bool, bool, bool), Vec<u8>)> = Vec::new();
        for separate in [false, true] {
            for synchronous in [false, true] {
                for secure in [false, true] {
                    let transport = UdpRtpTransport::new(RtpTransportConfig {
                        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
                        local_rtcp_addr: separate.then(|| "127.0.0.1:0".parse().unwrap()),
                        symmetric_rtp: false,
                        rtcp_mux: !separate,
                        session_id: Some("rtcp-now".to_string()),
                        use_port_allocator: false,
                        buffer_config: Default::default(),
                    })
                    .await
                    .unwrap();
                    let expected_source = if separate {
                        transport.local_rtcp_socket_addr().unwrap()
                    } else {
                        transport.local_rtp_addr().unwrap()
                    };
                    if secure {
                        let (send, recv) = make_srtp_ctx_pair();
                        transport.set_srtp_contexts(send, recv).await.unwrap();
                    }
                    for _ in 0..2 {
                        if synchronous {
                            transport
                                .send_rtcp_bytes_now(&report, destination)
                                .await
                                .unwrap();
                        } else {
                            RtpTransport::send_rtcp_bytes(&transport, &report, destination)
                                .await
                                .unwrap();
                        }
                        let mut buffer = [0u8; 512];
                        let (size, source) = tokio::time::timeout(
                            Duration::from_secs(1),
                            peer.recv_from(&mut buffer),
                        )
                        .await
                        .expect("report on the wire")
                        .unwrap();
                        assert_eq!(source, expected_source, "separate={separate}");
                        wires.push(((separate, synchronous, secure), buffer[..size].to_vec()));
                    }
                    transport.close().await.unwrap();
                }
            }
        }
        for separate in [false, true] {
            for secure in [false, true] {
                let pick = |synchronous: bool| -> Vec<Vec<u8>> {
                    wires
                        .iter()
                        .filter(|(key, _)| *key == (separate, synchronous, secure))
                        .map(|(_, wire)| wire.clone())
                        .collect()
                };
                let (asynchronous, synchronous) = (pick(false), pick(true));
                assert_eq!(asynchronous.len(), 2);
                assert_eq!(
                    asynchronous, synchronous,
                    "separate={separate} secure={secure}"
                );
                if secure {
                    assert_ne!(
                        synchronous[0], synchronous[1],
                        "each report takes its own SRTCP index"
                    );
                    assert_ne!(&synchronous[0][..], &report[..]);
                } else {
                    assert_eq!(&synchronous[0][..], &report[..]);
                }
            }
        }
        let after = crate::executor::inline_executor_counters();
        assert!(
            after.rtcp_sync_sends + after.rtcp_sync_deferred
                >= before.rtcp_sync_sends + before.rtcp_sync_deferred + 8
        );
    }

    #[tokio::test]
    async fn exact_srtp_replacement_token_restores_plaintext_state() {
        let transport = make_srtp_rollback_transport("srtp-exact-rollback").await;
        assert!(!transport.srtp_enabled().await);

        let (send, recv) = make_srtp_ctx_pair();
        let rollback = transport.replace_srtp_contexts(send, recv).await.unwrap();
        assert!(transport.srtp_enabled().await);

        transport.rollback_srtp_contexts(rollback).await.unwrap();
        assert!(!transport.srtp_enabled().await);
    }

    #[tokio::test]
    async fn stale_srtp_replacement_token_cannot_overwrite_newer_keys() {
        let transport = make_srtp_rollback_transport("srtp-stale-rollback").await;
        let (send_a, recv_a) = make_srtp_ctx_pair();
        let stale = transport
            .replace_srtp_contexts(send_a, recv_a)
            .await
            .unwrap();
        let (send_b, recv_b) = make_aes256_srtp_ctx_pair();
        let current = transport
            .replace_srtp_contexts(send_b, recv_b)
            .await
            .unwrap();

        let error = transport
            .rollback_srtp_contexts(stale)
            .await
            .expect_err("a superseded rollback token must be rejected");
        assert!(matches!(error, Error::InvalidState(_)));
        assert!(transport.srtp_enabled().await);

        transport.rollback_srtp_contexts(current).await.unwrap();
        assert!(transport.srtp_enabled().await);
    }

    #[tokio::test]
    async fn rejected_srtp_replacement_retires_earlier_plaintext_rollback_authority() {
        let transport = make_srtp_rollback_transport("srtp-rejected-replacement").await;
        let (send, recv) = make_srtp_ctx_pair();
        let plaintext_rollback = transport.replace_srtp_contexts(send, recv).await.unwrap();

        let (mut disabled_send, mut disabled_recv) = make_srtp_ctx_pair();
        disabled_send.set_enabled(false);
        disabled_recv.set_enabled(false);
        let error = match transport
            .replace_srtp_contexts(disabled_send, disabled_recv)
            .await
        {
            Ok(_) => panic!("disabled contexts must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, Error::InvalidState(_)));

        let rollback_error = transport
            .rollback_srtp_contexts(plaintext_rollback)
            .await
            .expect_err("a rejected replacement must retire older rollback authority");
        assert!(matches!(rollback_error, Error::InvalidState(_)));
        assert!(transport.secure_media_required.load(Ordering::Acquire));
        assert!(transport.srtp_enabled().await);
    }

    #[tokio::test]
    async fn exhausted_srtp_context_generation_fails_without_mutating_security_state() {
        let transport = make_srtp_rollback_transport("srtp-generation-exhaustion").await;
        transport
            .srtp_context_generation
            .store(u64::MAX - 1, Ordering::Release);
        let (send, recv) = make_srtp_ctx_pair();

        let error = match transport.replace_srtp_contexts(send, recv).await {
            Ok(_) => panic!("generation exhaustion must fail closed"),
            Err(error) => error,
        };
        assert!(matches!(error, Error::InvalidState(_)));
        assert!(!transport.srtp_enabled().await);
        assert!(transport.srtp_send.lock().is_none());
        assert!(transport.srtp_recv.lock().home.is_none());
    }

    #[tokio::test]
    async fn public_secure_receive_never_returns_unauthenticated_rtp() {
        let transport = UdpRtpTransport::new(RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: false,
            rtcp_mux: true,
            session_id: Some("secure-public-receive".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        })
        .await
        .unwrap();
        transport.stop_receiver().await.unwrap();

        let (transport_send, transport_recv) = make_srtp_ctx_pair();
        transport
            .set_srtp_contexts(transport_send, transport_recv)
            .await
            .unwrap();
        let destination = transport.local_rtp_addr().unwrap();
        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut receive_buffer = [0_u8; 256];

        let plaintext = RtpPacket::new(
            RtpHeader::new(0, 10, 1_600, 0x1122_3344),
            Bytes::from_static(b"plaintext must fail"),
        )
        .serialize()
        .unwrap();
        sender.send_to(&plaintext, destination).await.unwrap();
        assert!(tokio::time::timeout(
            Duration::from_secs(1),
            transport.receive_packet(&mut receive_buffer)
        )
        .await
        .unwrap()
        .is_err());

        let mut wrong_key = crate::srtp::SrtpContext::new(
            crate::srtp::SRTP_AES128_CM_SHA1_80,
            crate::srtp::SrtpCryptoKey::new(vec![0xa5; 16], vec![0x5a; 14]),
        )
        .unwrap();
        let wrong_packet = RtpPacket::new(
            RtpHeader::new(0, 11, 1_760, 0x1122_3344),
            Bytes::from_static(b"wrong authentication key"),
        );
        let wrong_wire = wrong_key
            .protect(&wrong_packet)
            .unwrap()
            .serialize()
            .unwrap();
        sender.send_to(&wrong_wire, destination).await.unwrap();
        assert!(tokio::time::timeout(
            Duration::from_secs(1),
            transport.receive_packet(&mut receive_buffer)
        )
        .await
        .unwrap()
        .is_err());

        let (mut matching_sender, _) = make_srtp_ctx_pair();
        let valid_packet = RtpPacket::new(
            RtpHeader::new(0, 12, 1_920, 0x1122_3344),
            Bytes::from_static(b"authenticated plaintext"),
        );
        let valid_wire = matching_sender
            .protect(&valid_packet)
            .unwrap()
            .serialize()
            .unwrap();
        sender.send_to(&valid_wire, destination).await.unwrap();
        let (size, source) = tokio::time::timeout(
            Duration::from_secs(1),
            transport.receive_packet(&mut receive_buffer),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(source, sender.local_addr().unwrap());
        assert_ne!(&receive_buffer[..size], valid_wire.as_ref());
        let recovered = RtpPacket::parse(&receive_buffer[..size]).unwrap();
        assert_eq!(recovered.payload, valid_packet.payload);

        transport.close().await.unwrap();
    }

    #[tokio::test]
    async fn receive_scratch_reuse_preserves_owned_packets_and_lengths() {
        for rtcp_mux in [true, false] {
            let receiver = UdpRtpTransport::new(RtpTransportConfig {
                local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
                local_rtcp_addr: (!rtcp_mux).then(|| "127.0.0.1:0".parse().unwrap()),
                symmetric_rtp: false,
                rtcp_mux,
                session_id: Some(format!("scratch-reuse-{rtcp_mux}")),
                use_port_allocator: false,
                buffer_config: Default::default(),
            })
            .await
            .unwrap();
            let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let rtp_destination = receiver.local_rtp_addr().unwrap();
            let rtcp_destination = if rtcp_mux {
                rtp_destination
            } else {
                receiver
                    .rtcp_socket
                    .load_full()
                    .unwrap()
                    .local_addr()
                    .unwrap()
            };
            let mut events = receiver.subscribe();
            let mut retained = Vec::new();
            // Long datagrams followed by short ones: a reused buffer must
            // never leak the tail of an earlier, longer datagram.
            for (seq, len) in [1024, 1, 160, 3].into_iter().enumerate() {
                let payload = Bytes::from(vec![seq as u8 + 1; len]);
                let packet = RtpPacket::new(
                    RtpHeader::new(0, seq as u16, seq as u32 * 160, 123),
                    payload.clone(),
                )
                .serialize()
                .unwrap();
                sender.send_to(&packet, rtp_destination).await.unwrap();
                match tokio::time::timeout(Duration::from_secs(2), events.recv())
                    .await
                    .unwrap()
                    .unwrap()
                {
                    RtpEvent::MediaReceived {
                        payload: actual, ..
                    } => retained.push((actual, payload)),
                    other => panic!("unexpected {other:?}"),
                }
                // Valid RTCP APP packets of alternating length share the RTP
                // buffer with rtcp-mux and use the RTCP buffer without it.
                let words = if seq % 2 == 0 { 128 } else { 3 };
                let mut rtcp = vec![seq as u8 + 10; words * 4];
                rtcp[..4].copy_from_slice(&[0x80, 204, 0, (words - 1) as u8]);
                sender.send_to(&rtcp, rtcp_destination).await.unwrap();
                match tokio::time::timeout(Duration::from_secs(2), events.recv())
                    .await
                    .unwrap()
                    .unwrap()
                {
                    RtpEvent::RtcpReceived { data, .. } => retained.push((data, Bytes::from(rtcp))),
                    other => panic!("unexpected {other:?}"),
                }
            }
            for (actual, expected) in retained {
                assert_eq!(actual, expected);
            }
            receiver.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn srtcp_round_trip_uses_muxed_and_separate_udp_paths() {
        for rtcp_mux in [true, false] {
            let config = |name: &str| RtpTransportConfig {
                local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
                local_rtcp_addr: (!rtcp_mux).then(|| "127.0.0.1:0".parse().unwrap()),
                symmetric_rtp: false,
                rtcp_mux,
                session_id: Some(format!("srtcp-{name}-{rtcp_mux}")),
                use_port_allocator: false,
                buffer_config: Default::default(),
            };
            let transport_a = UdpRtpTransport::new(config("a")).await.unwrap();
            let transport_b = UdpRtpTransport::new(config("b")).await.unwrap();
            let (a_send, b_recv) = make_srtp_ctx_pair();
            let (b_send, a_recv) = make_srtp_ctx_pair();
            transport_a.set_srtp_contexts(a_send, a_recv).await.unwrap();
            transport_b.set_srtp_contexts(b_send, b_recv).await.unwrap();

            let destination = if rtcp_mux {
                transport_b.local_rtp_addr().unwrap()
            } else {
                transport_b
                    .rtcp_socket
                    .load_full()
                    .unwrap()
                    .local_addr()
                    .unwrap()
            };
            let report = test_receiver_report(0x1122_3344);
            let expected = report.serialize().unwrap();
            let mut events = transport_b.subscribe();
            transport_a.send_rtcp(&report, destination).await.unwrap();

            match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
                Ok(Ok(RtpEvent::RtcpReceived { data, .. })) => assert_eq!(data, expected),
                other => {
                    panic!("expected authenticated RTCP event for mux={rtcp_mux}, got {other:?}")
                }
            }

            let unknown = [0x80, 208, 0, 1, 0x11, 0x22, 0x33, 0x44];
            transport_a
                .send_rtcp_bytes(&unknown, destination)
                .await
                .unwrap();
            match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
                Ok(Ok(RtpEvent::RtcpReceived { data, .. })) => {
                    assert_eq!(data.as_ref(), unknown)
                }
                other => panic!("expected unknown authenticated RTCP event, got {other:?}"),
            }
            transport_a.close().await.unwrap();
            transport_b.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn secure_udp_paths_drop_plaintext_rtcp_then_accept_valid_srtcp() {
        for rtcp_mux in [true, false] {
            let config = |name: &str| RtpTransportConfig {
                local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
                local_rtcp_addr: (!rtcp_mux).then(|| "127.0.0.1:0".parse().unwrap()),
                symmetric_rtp: false,
                rtcp_mux,
                session_id: Some(format!("srtcp-drop-{name}-{rtcp_mux}")),
                use_port_allocator: false,
                buffer_config: Default::default(),
            };
            let transport_a = UdpRtpTransport::new(config("a")).await.unwrap();
            let transport_b = UdpRtpTransport::new(config("b")).await.unwrap();
            let (a_send, b_recv) = make_srtp_ctx_pair();
            let (b_send, a_recv) = make_srtp_ctx_pair();
            transport_a.set_srtp_contexts(a_send, a_recv).await.unwrap();
            transport_b.set_srtp_contexts(b_send, b_recv).await.unwrap();

            let destination = if rtcp_mux {
                transport_b.local_rtp_addr().unwrap()
            } else {
                transport_b
                    .rtcp_socket
                    .load_full()
                    .unwrap()
                    .local_addr()
                    .unwrap()
            };
            let report = test_receiver_report(0x5566_7788);
            let plaintext = report.serialize().unwrap();
            let attacker = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let mut events = transport_b.subscribe();
            attacker.send_to(&plaintext, destination).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), events.recv())
                    .await
                    .is_err()
            );

            transport_a.send_rtcp(&report, destination).await.unwrap();
            match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
                Ok(Ok(RtpEvent::RtcpReceived { data, .. })) => assert_eq!(data, plaintext),
                other => panic!("expected valid SRTCP after plaintext drop, got {other:?}"),
            }
            transport_a.close().await.unwrap();
            transport_b.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn srtp_round_trip_through_real_udp_sockets() {
        // Two transports, both with SRTP enabled (matched key pair).
        // A sends an RTP packet; B receives it. The wire bytes are
        // encrypted; B's `MediaReceived` event must surface the
        // original payload after `unprotect()`.
        let cfg_a = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("srtp-a".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let cfg_b = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("srtp-b".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let transport_a = UdpRtpTransport::new(cfg_a).await.unwrap();
        let transport_b = UdpRtpTransport::new(cfg_b).await.unwrap();

        // Two pairs needed: A.send/B.recv (A→B direction) plus
        // A.recv/B.send (B→A direction). For this one-shot test we
        // only need A→B; we still install a matched receive context
        // on the other side.
        let (a_send, b_recv) = make_srtp_ctx_pair();
        let (b_send, a_recv) = make_srtp_ctx_pair();
        transport_a.set_srtp_contexts(a_send, a_recv).await.unwrap();
        transport_b.set_srtp_contexts(b_send, b_recv).await.unwrap();
        assert!(transport_a.srtp_enabled().await);
        assert!(transport_b.srtp_enabled().await);

        let mut events = transport_b.subscribe();

        // RTP payload type 0 (PCMU) so the test mirrors the real
        // call-path; SRTP is codec-agnostic.
        let header = RtpHeader::new(0, 1, 12345, 0xdead_beef);
        let payload = Bytes::from_static(b"hello srtp wire");
        let packet = RtpPacket::new(header, payload.clone());

        let addr_b = transport_b.local_rtp_addr().unwrap();
        transport_a.send_rtp(&packet, addr_b).await.unwrap();

        match tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await {
            Ok(Ok(RtpEvent::MediaReceived {
                payload: received_payload,
                payload_type,
                timestamp,
                ..
            })) => {
                assert_eq!(payload_type, 0);
                assert_eq!(timestamp, 12345);
                assert_eq!(
                    &received_payload[..],
                    &payload[..],
                    "B should see the decrypted plaintext payload"
                );
            }
            other => panic!("expected MediaReceived, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn srtp_aes256_round_trip_through_real_udp_sockets() {
        let cfg_a = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("srtp-aes256-a".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let cfg_b = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("srtp-aes256-b".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let transport_a = UdpRtpTransport::new(cfg_a).await.unwrap();
        let transport_b = UdpRtpTransport::new(cfg_b).await.unwrap();

        let (a_send, b_recv) = make_aes256_srtp_ctx_pair();
        let (b_send, a_recv) = make_aes256_srtp_ctx_pair();
        transport_a.set_srtp_contexts(a_send, a_recv).await.unwrap();
        transport_b.set_srtp_contexts(b_send, b_recv).await.unwrap();

        let mut events = transport_b.subscribe();
        let header = RtpHeader::new(0, 1, 12345, 0xdead_beef);
        let payload = Bytes::from_static(b"hello aes-256 srtp wire");
        let packet = RtpPacket::new(header, payload.clone());

        let addr_b = transport_b.local_rtp_addr().unwrap();
        transport_a.send_rtp(&packet, addr_b).await.unwrap();

        match tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await {
            Ok(Ok(RtpEvent::MediaReceived {
                payload: received_payload,
                payload_type,
                timestamp,
                ..
            })) => {
                assert_eq!(payload_type, 0);
                assert_eq!(timestamp, 12345);
                assert_eq!(&received_payload[..], &payload[..]);
            }
            other => panic!("expected MediaReceived, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn srtp_silent_drop_on_auth_failure() {
        // RFC 3711 §3.4 — auth failures MUST be silently dropped.
        // A sends with one key, B is configured with a different
        // (mismatched) recv key — `unprotect` will fail. B's event
        // stream must produce nothing, and the receive task must keep
        // running (so subsequent valid packets would still be
        // processed).
        let cfg_a = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("srtp-drop-a".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let cfg_b = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("srtp-drop-b".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let transport_a = UdpRtpTransport::new(cfg_a).await.unwrap();
        let transport_b = UdpRtpTransport::new(cfg_b).await.unwrap();

        // A: matched pair with itself (key 1).
        let (a_send, _a_unused) = make_srtp_ctx_pair();
        let (_a2, a_recv) = make_srtp_ctx_pair();
        transport_a.set_srtp_contexts(a_send, a_recv).await.unwrap();

        // B: DIFFERENT key — set up a separate pair so unprotect
        // can't authenticate A's packets.
        use crate::srtp::{SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
        let mismatch_key = vec![0xffu8; 16];
        let mismatch_salt = vec![0xeeu8; 14];
        let b_recv_mismatch = crate::srtp::SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(mismatch_key.clone(), mismatch_salt.clone()),
        )
        .unwrap();
        let b_send_mismatch = crate::srtp::SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(mismatch_key, mismatch_salt),
        )
        .unwrap();
        transport_b
            .set_srtp_contexts(b_send_mismatch, b_recv_mismatch)
            .await
            .unwrap();

        let mut events = transport_b.subscribe();

        let header = RtpHeader::new(0, 1, 12345, 0xdead_beef);
        let payload = Bytes::from_static(b"this should be dropped");
        let packet = RtpPacket::new(header, payload);

        let addr_b = transport_b.local_rtp_addr().unwrap();
        transport_a.send_rtp(&packet, addr_b).await.unwrap();

        // Wait long enough that any forwarded event would have
        // arrived; assert nothing came through.
        let waited =
            tokio::time::timeout(tokio::time::Duration::from_millis(200), events.recv()).await;
        assert!(
            waited.is_err(),
            "auth-failed packet must be silently dropped (got event {:?})",
            waited
        );
    }

    /// Build a 4-byte RFC 4733 telephone-event wire payload.
    /// `event_code` is 0-15 for DTMF; `end_of_event` is the E bit;
    /// `duration` is in 8 kHz timestamp units.
    fn rfc4733_payload(event_code: u8, end_of_event: bool, volume: u8, duration: u16) -> Bytes {
        let e_bit = if end_of_event { 0b1000_0000 } else { 0 };
        let byte1 = e_bit | (volume & 0b0011_1111);
        let dur = duration.to_be_bytes();
        Bytes::copy_from_slice(&[event_code, byte1, dur[0], dur[1]])
    }

    /// RFC 4733 §2.5.1.3 retransmit dedup. Sender emits three identical
    /// E=1 frames sharing `(ssrc, rtp_timestamp)` for loss resilience.
    /// The transport must collapse them into one `RtpEvent::DtmfEvent`
    /// before downstream consumers see them.
    #[tokio::test]
    async fn test_pt101_three_end_of_event_retransmits_dedup_to_one_event() {
        let cfg1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("dtmf-dedup-sender".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let cfg2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("dtmf-dedup-receiver".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let sender = UdpRtpTransport::new(cfg1).await.unwrap();
        let receiver = UdpRtpTransport::new(cfg2).await.unwrap();

        let mut events = receiver.subscribe();
        let receiver_addr = receiver.local_rtp_addr().unwrap();

        // Three identical E=1 packets, same ssrc + ts (RFC 4733 §2.5.1.3
        // retransmit shape).
        let payload = rfc4733_payload(
            /*'1'*/ 1, /*end*/ true, /*vol*/ 10, /*dur*/ 800,
        );
        let header = RtpHeader::new(
            /*PT*/ 101,
            /*seq*/ 1,
            /*ts*/ 12345,
            /*ssrc*/ 0xdead_beef,
        );
        for _ in 0..3 {
            let packet = RtpPacket::new(header.clone(), payload.clone());
            sender.send_rtp(&packet, receiver_addr).await.unwrap();
        }

        // First packet → one DtmfEvent.
        match tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await {
            Ok(Ok(RtpEvent::DtmfEvent {
                event,
                end_of_event,
                ..
            })) => {
                assert_eq!(event, 1);
                assert!(end_of_event);
            }
            other => panic!("expected first DtmfEvent, got {:?}", other),
        }

        // Subsequent retransmits must be suppressed — no further event.
        let leftover =
            tokio::time::timeout(tokio::time::Duration::from_millis(150), events.recv()).await;
        assert!(
            leftover.is_err(),
            "RFC 4733 retransmit must be deduped, got extra event {:?}",
            leftover
        );
    }

    /// Dedup keys on `(peer_addr, ssrc, ts)` because rtp-core has no
    /// dialog scope. Two simultaneous DTMF streams from different peers
    /// that happen to collide on `(ssrc, ts)` must each fire — they're
    /// not retransmits of the same tone.
    #[tokio::test]
    async fn test_pt101_dedup_scoped_by_peer_addr() {
        let mk_cfg = |session_id: &str| RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some(session_id.to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let peer_a = UdpRtpTransport::new(mk_cfg("dtmf-peer-a")).await.unwrap();
        let peer_b = UdpRtpTransport::new(mk_cfg("dtmf-peer-b")).await.unwrap();
        // This test intentionally admits two independent peers to one socket
        // so it can isolate the peer component of the DTMF dedup key. A
        // symmetric-RTP receiver is single-peer by design: after the first
        // packet latches its source, another source must pass rebinding
        // probation and an identical sequence number is rejected.
        // Peer-admission behavior is covered by the dedicated symmetric-RTP
        // tests.
        let mut receiver_config = mk_cfg("dtmf-multi-peer-rx");
        receiver_config.symmetric_rtp = false;
        let receiver = UdpRtpTransport::new(receiver_config).await.unwrap();

        let mut events = receiver.subscribe();
        let receiver_addr = receiver.local_rtp_addr().unwrap();

        // Same `(ssrc, ts)` from two distinct senders. Receiver must
        // emit two events — once per peer.
        let payload = rfc4733_payload(2, true, 10, 800);
        let header = RtpHeader::new(101, 1, 22222, 0xdead_beef);
        let pkt = RtpPacket::new(header.clone(), payload.clone());

        peer_a.send_rtp(&pkt, receiver_addr).await.unwrap();
        peer_b.send_rtp(&pkt, receiver_addr).await.unwrap();

        let first =
            tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await;
        let second =
            tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await;
        match (first, second) {
            (
                Ok(Ok(RtpEvent::DtmfEvent {
                    source: addr_first, ..
                })),
                Ok(Ok(RtpEvent::DtmfEvent {
                    source: addr_second,
                    ..
                })),
            ) => {
                assert_ne!(
                    addr_first, addr_second,
                    "expected two DtmfEvents from distinct peer addresses"
                );
            }
            other => panic!("expected two DtmfEvents (one per peer), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn plain_rtp_path_unaffected_when_srtp_unset() {
        // Regression: verify the no-SRTP path still works after the
        // wrapping fields were added. Mirrors the existing
        // `test_udp_transport_event_subscription` shape.
        let cfg1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("plain1".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let cfg2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("plain2".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        };
        let t1 = UdpRtpTransport::new(cfg1).await.unwrap();
        let t2 = UdpRtpTransport::new(cfg2).await.unwrap();
        // Neither side touches set_srtp_contexts — both must report
        // SRTP disabled.
        assert!(!t1.srtp_enabled().await);
        assert!(!t2.srtp_enabled().await);

        let mut events = t2.subscribe();
        let header = RtpHeader::new(0, 1, 12345, 0xdead_beef);
        let payload = Bytes::from_static(b"plain rtp payload");
        let packet = RtpPacket::new(header, payload.clone());
        let addr_b = t2.local_rtp_addr().unwrap();
        t1.send_rtp(&packet, addr_b).await.unwrap();
        match tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await {
            Ok(Ok(RtpEvent::MediaReceived { payload: rcv, .. })) => {
                assert_eq!(&rcv[..], &payload[..]);
            }
            other => panic!("expected plaintext MediaReceived, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod inline_rx_tests {
    //! Executor v2: the socket owner's drain budget and its one
    //! owner timer, exercised directly on `InlineRx`.
    use super::*;
    use crate::executor::inline_executor_counters;
    use std::sync::atomic::AtomicUsize;

    /// Counts deadlines; reports one pending deadline until it is served.
    struct DeadlineIngress {
        next: Option<Instant>,
        served: Arc<AtomicUsize>,
    }

    impl InlineRtpIngress for DeadlineIngress {
        fn on_rtp(&mut self, _packet: &InlineRtpPacket<'_>, _now: Instant) {}

        fn next_deadline(&mut self, _now: Instant) -> Option<Instant> {
            self.next
        }

        fn on_deadline(&mut self, now: Instant) {
            if self.next.is_some_and(|deadline| deadline <= now) {
                self.next = None;
                self.served.fetch_add(1, Ordering::AcqRel);
            }
        }
    }

    fn inline_rx(next: Option<Instant>) -> (InlineRx, Arc<AtomicUsize>) {
        let served = Arc::new(AtomicUsize::new(0));
        let rx = InlineRx::new(Box::new(DeadlineIngress {
            next,
            served: Arc::clone(&served),
        }));
        (rx, served)
    }

    #[tokio::test]
    async fn drain_budget_yields_every_fifty_back_to_back_receives_and_keeps_order() {
        let before = inline_executor_counters();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = socket.local_addr().unwrap();
        const BURST: usize = 2 * INLINE_DRAIN_BUDGET + 20;
        for index in 0..BURST {
            peer.send_to(&(index as u32).to_be_bytes(), destination)
                .await
                .unwrap();
        }
        // Let every datagram land in the receive queue before draining.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (mut rx, _) = inline_rx(None);
        let mut buffer = vec![0u8; 64];
        for index in 0..BURST {
            let wake =
                tokio::time::timeout(Duration::from_secs(2), rx.next_wake(&socket, &mut buffer))
                    .await
                    .expect("queued datagrams are received");
            match wake {
                InlineWake::Received(Ok((size, source))) => {
                    assert_eq!(size, 4);
                    assert_eq!(source, peer.local_addr().unwrap());
                    assert_eq!(&buffer[..4], &(index as u32).to_be_bytes());
                }
                InlineWake::Received(Err(error)) => panic!("receive failed: {error}"),
                InlineWake::Timer => panic!("no deadline is armed"),
            }
            assert!(rx.drained <= INLINE_DRAIN_BUDGET);
        }
        drop(rx);
        let after = inline_executor_counters();
        assert!(
            after.drain_budget_yields >= before.drain_budget_yields + 2,
            "a {BURST}-datagram burst crosses the budget twice"
        );
    }

    #[tokio::test]
    async fn the_owner_timer_serves_the_ingress_deadline_on_an_idle_socket() {
        let before = inline_executor_counters();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let start = Instant::now();
        let deadline = start + Duration::from_millis(5);
        let (mut rx, served) = inline_rx(Some(deadline));
        rx.rearm(start);
        assert_eq!(rx.timer.armed(), Some(deadline));
        rx.rearm(start);
        let mut buffer = vec![0u8; 64];
        let wake = tokio::time::timeout(Duration::from_secs(2), rx.next_wake(&socket, &mut buffer))
            .await
            .expect("the owner timer fires");
        assert!(matches!(wake, InlineWake::Timer));
        assert!(Instant::now() >= deadline, "never early");
        assert_eq!(served.load(Ordering::Acquire), 1);
        assert_eq!(rx.timer.armed(), None, "nothing left to arm");
        drop(rx);
        let after = inline_executor_counters();
        assert!(after.owner_timer_fires > before.owner_timer_fires);
        assert!(after.owner_timer_resets > before.owner_timer_resets);
    }

    #[tokio::test]
    async fn a_deadline_that_comes_due_during_a_burst_is_served_at_the_budget_yield() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = socket.local_addr().unwrap();
        for index in 0..(INLINE_DRAIN_BUDGET + 5) {
            peer.send_to(&(index as u32).to_be_bytes(), destination)
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Already due: receives win ties while the socket is readable, and
        // the budget yield is where the owner serves it.
        let (mut rx, served) = inline_rx(Some(Instant::now()));
        rx.rearm(Instant::now());
        let mut buffer = vec![0u8; 64];
        let mut received = 0;
        let mut timer_at = None;
        for wake_index in 0..(INLINE_DRAIN_BUDGET + 6) {
            match tokio::time::timeout(Duration::from_secs(2), rx.next_wake(&socket, &mut buffer))
                .await
                .expect("wakes")
            {
                InlineWake::Received(Ok(_)) => received += 1,
                InlineWake::Received(Err(error)) => panic!("receive failed: {error}"),
                InlineWake::Timer => timer_at = Some(wake_index),
            }
        }
        assert_eq!(received, INLINE_DRAIN_BUDGET + 5);
        assert_eq!(served.load(Ordering::Acquire), 1);
        let timer_at = timer_at.expect("the deadline was served");
        assert!(
            timer_at <= INLINE_DRAIN_BUDGET,
            "served no later than the first budget yield"
        );
    }

    /// Several deadlines falling due at the same instant (on
    /// fire, serve all due work and re-arm once).
    struct ManyDeadlines {
        pending: Vec<Instant>,
        served: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
    }

    impl InlineRtpIngress for ManyDeadlines {
        fn on_rtp(&mut self, _packet: &InlineRtpPacket<'_>, _now: Instant) {}

        fn next_deadline(&mut self, _now: Instant) -> Option<Instant> {
            self.pending.iter().copied().min()
        }

        fn on_deadline(&mut self, now: Instant) {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let before = self.pending.len();
            self.pending.retain(|deadline| *deadline > now);
            self.served
                .fetch_add(before - self.pending.len(), Ordering::AcqRel);
        }
    }

    #[tokio::test]
    async fn one_fire_serves_every_due_deadline_and_rearms_once() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let start = Instant::now();
        let due = start + Duration::from_millis(5);
        let next = start + Duration::from_millis(400);
        let served = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut rx = InlineRx::new(Box::new(ManyDeadlines {
            pending: vec![due, due, due, next],
            served: Arc::clone(&served),
            calls: Arc::clone(&calls),
        }));
        rx.rearm(start);
        let mut buffer = vec![0u8; 64];
        let wake = tokio::time::timeout(Duration::from_secs(2), rx.next_wake(&socket, &mut buffer))
            .await
            .expect("the owner timer fires");
        assert!(matches!(wake, InlineWake::Timer));
        assert_eq!(
            served.load(Ordering::Acquire),
            3,
            "every due source served by one fire"
        );
        assert_eq!(calls.load(Ordering::Acquire), 1, "one service pass");
        assert_eq!(
            rx.timer.armed(),
            Some(next),
            "re-armed once, for the next deadline"
        );
        // A later deadline from the same owner extends rather than re-registers.
        let later = next + Duration::from_millis(100);
        assert_eq!(
            rx.timer.arm(Some(later), Instant::now()),
            crate::executor::ArmOutcome::Extended
        );
    }

    /// The v2 receive loop's shape: wait with `next_wake`, log each datagram.
    fn logging_loop(
        id: usize,
        socket: UdpSocket,
        mut rx: InlineRx,
        log: Arc<parking_lot::Mutex<Vec<(usize, u32)>>>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        async move {
            let mut buffer = vec![0u8; 64];
            loop {
                match rx.next_wake(&socket, &mut buffer).await {
                    InlineWake::Received(Ok((size, _))) => {
                        assert_eq!(size, 4);
                        let value =
                            u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
                        log.lock().push((id, value));
                    }
                    // Never expected on loopback; ending the loop fails the
                    // test's datagram count instead.
                    InlineWake::Received(Err(_)) => break,
                    InlineWake::Timer => {}
                }
            }
        }
    }

    #[tokio::test]
    async fn a_lane_poller_serves_quiet_sockets_while_one_socket_bursts() {
        // Socket 0 has three drain budgets queued; sockets 1 and 2 have one
        // datagram each. All three loops share one lane poller task.
        const BURST: usize = 3 * INLINE_DRAIN_BUDGET;
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut sockets = Vec::new();
        for _ in 0..3 {
            sockets.push(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        }
        for index in 0..BURST {
            peer.send_to(
                &(index as u32).to_be_bytes(),
                sockets[0].local_addr().unwrap(),
            )
            .await
            .unwrap();
        }
        for (id, socket) in sockets.iter().enumerate().skip(1) {
            peer.send_to(
                &(1000 + id as u32).to_be_bytes(),
                socket.local_addr().unwrap(),
            )
            .await
            .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let poller = crate::lane_poller::LanePoller::start(&tokio::runtime::Handle::current());
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        for (id, socket) in sockets.into_iter().enumerate() {
            let (rx, _) = inline_rx(None);
            handles.push(
                poller
                    .host(logging_loop(id, socket, rx, Arc::clone(&log)))
                    .ok()
                    .expect("poller is running"),
            );
        }
        let expected = BURST + 2;
        for _ in 0..200 {
            if log.lock().len() >= expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let log = log.lock().clone();
        assert_eq!(log.len(), expected, "every datagram was received");
        let burst: Vec<u32> = log
            .iter()
            .filter(|(id, _)| *id == 0)
            .map(|(_, v)| *v)
            .collect();
        assert_eq!(
            burst,
            (0..BURST as u32).collect::<Vec<_>>(),
            "per-socket order kept"
        );
        let position = |wanted: usize| log.iter().position(|(id, _)| *id == wanted).unwrap();
        let first_after_budget = log
            .iter()
            .enumerate()
            .filter(|(_, (id, _))| *id == 0)
            .nth(INLINE_DRAIN_BUDGET)
            .map(|(index, _)| index)
            .unwrap();
        for quiet in [1, 2] {
            assert!(
                position(quiet) < first_after_budget,
                "socket {quiet} waited behind more than one drain budget of the burst"
            );
        }
        for handle in handles {
            handle.abort();
            tokio::time::timeout(Duration::from_secs(2), handle.join())
                .await
                .expect("aborted loops join");
        }
    }

    #[tokio::test]
    async fn a_hosted_owner_serves_its_deadline_from_the_lane_poller() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let start = Instant::now();
        let (mut rx, served) = inline_rx(Some(start + Duration::from_millis(5)));
        rx.rearm(start);
        let poller = crate::lane_poller::LanePoller::start(&tokio::runtime::Handle::current());
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let handle = poller
            .host(logging_loop(0, socket, rx, log))
            .ok()
            .expect("poller is running");
        for _ in 0..200 {
            if served.load(Ordering::Acquire) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(served.load(Ordering::Acquire), 1);
        assert!(Instant::now() >= start + Duration::from_millis(5));
        handle.abort();
        handle.join().await;
    }

    #[tokio::test]
    async fn a_v2_transport_receives_on_its_lane_poller_and_close_releases_it() {
        let before = inline_executor_counters();
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        let transport = UdpRtpTransport::new(RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: false,
            rtcp_mux: true,
            session_id: Some("lane-hosted".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        })
        .await
        .unwrap();
        {
            let tasks = transport.receiver_tasks.lock().await;
            assert_eq!(tasks.len(), 1);
            assert!(
                matches!(tasks[0], ReceiverTask::Lane(_)),
                "v2 hosts the loop on its lane"
            );
        }
        let mut events = transport.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = transport.local_rtp_addr().unwrap();
        let packet = RtpPacket::new(
            crate::packet::RtpHeader::new(0, 7, 1120, 0x0102_0304),
            Bytes::from_static(b"lane payload"),
        );
        peer.send_to(&packet.serialize().unwrap(), destination)
            .await
            .unwrap();
        match tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
            Ok(Ok(RtpEvent::MediaReceived {
                sequence_number,
                payload,
                ..
            })) => {
                assert_eq!(sequence_number, 7);
                assert_eq!(&payload[..], b"lane payload");
            }
            other => panic!("expected MediaReceived from the hosted loop, got {other:?}"),
        }
        transport.close().await.unwrap();
        assert!(transport.receiver_tasks.lock().await.is_empty());
        assert!(!transport.active.load(Ordering::Acquire));
        // The hosted loop is gone: nothing is received any more.
        peer.send_to(&packet.serialize().unwrap(), destination)
            .await
            .ok();
        let late = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;
        assert!(
            !matches!(late, Ok(Ok(RtpEvent::MediaReceived { .. }))),
            "{late:?}"
        );
        let after = inline_executor_counters();
        assert!(after.lane_sockets > before.lane_sockets);
    }

    #[tokio::test]
    async fn a_v2_receive_loop_is_hosted_on_its_transports_own_lane() {
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        // Several transports, placed round robin on the media lanes: each
        // loop is hosted by a poller of the lane its socket is registered on.
        let mut transports = Vec::new();
        for index in 0..4 {
            let transport = UdpRtpTransport::new(RtpTransportConfig {
                local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
                local_rtcp_addr: None,
                symmetric_rtp: false,
                rtcp_mux: true,
                session_id: Some(format!("lane-placement-{index}")),
                use_port_allocator: false,
                buffer_config: Default::default(),
            })
            .await
            .unwrap();
            assert!(
                matches!(
                    transport.receiver_tasks.lock().await[0],
                    ReceiverTask::Lane(_)
                ),
                "hosted on a lane poller"
            );
            let lane = transport
                .media_runtime_handle()
                .expect("a UDP transport has a lane");
            assert_eq!(lane.id(), transport.media_runtime.id());
            assert!(
                crate::lane_poller::hosts_pollers_on(&lane),
                "the poller runs on the transport's own lane"
            );
            transports.push(transport);
        }
        for transport in transports {
            transport.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_v1_transport_keeps_its_own_receive_task() {
        let _v1 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v1(),
        );
        let transport = UdpRtpTransport::new(RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: false,
            rtcp_mux: true,
            session_id: Some("v1-task".to_string()),
            use_port_allocator: false,
            buffer_config: Default::default(),
        })
        .await
        .unwrap();
        {
            let tasks = transport.receiver_tasks.lock().await;
            assert!(matches!(tasks[0], ReceiverTask::Task(_)));
        }
        transport.close().await.unwrap();
    }
}

#[cfg(test)]
mod sync_tx_writer_tests {
    //! The `sendmmsg` fallback and refusal rules of the synchronous TX loop,
    //! against a scripted writer.
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io;

    enum Many {
        Unsupported,
        Accept(usize),
        Fail(io::ErrorKind),
    }

    struct Scripted {
        many: Many,
        one: RefCell<VecDeque<io::Result<usize>>>,
        wire: RefCell<Vec<Vec<u8>>>,
    }

    impl Scripted {
        fn new(many: Many, one: Vec<io::Result<usize>>) -> Self {
            Self {
                many,
                one: RefCell::new(one.into()),
                wire: RefCell::new(Vec::new()),
            }
        }
    }

    impl DatagramWriter for Scripted {
        fn send_one(&self, datagram: &[u8], _dest: SocketAddr) -> io::Result<usize> {
            let result = self
                .one
                .borrow_mut()
                .pop_front()
                .unwrap_or(Ok(datagram.len()));
            if result.is_ok() {
                self.wire.borrow_mut().push(datagram.to_vec());
            }
            result
        }

        fn send_many(&self, datagrams: &[&[u8]], _dest: SocketAddr) -> Option<io::Result<usize>> {
            match self.many {
                Many::Unsupported => None,
                Many::Fail(kind) => Some(Err(io::Error::from(kind))),
                Many::Accept(count) => {
                    let count = count.min(datagrams.len());
                    self.wire
                        .borrow_mut()
                        .extend(datagrams[..count].iter().map(|datagram| datagram.to_vec()));
                    Some(Ok(count))
                }
            }
        }
    }

    fn datagrams() -> Vec<Vec<u8>> {
        (0..5u8).map(|i| vec![i; 3]).collect()
    }

    fn run(writer: &Scripted, batch: bool) -> (TxNowOutcome, Option<io::Error>) {
        let owned = datagrams();
        let views: Vec<&[u8]> = owned.iter().map(Vec::as_slice).collect();
        write_datagrams_now(writer, &views, "127.0.0.1:9".parse().unwrap(), batch)
    }

    #[test]
    fn a_full_sendmmsg_sends_everything_in_one_call() {
        let writer = Scripted::new(Many::Accept(5), vec![]);
        let (outcome, error) = run(&writer, true);
        assert!(error.is_none());
        assert_eq!(
            (
                outcome.sent,
                outcome.sendmmsg_calls,
                outcome.sendmmsg_packets
            ),
            (5, 1, 4 + 1)
        );
        assert!(!outcome.fallback && !outcome.would_block);
        assert_eq!(*writer.wire.borrow(), datagrams());
    }

    #[test]
    fn a_short_sendmmsg_falls_back_per_packet_in_order() {
        let writer = Scripted::new(Many::Accept(2), vec![]);
        let (outcome, error) = run(&writer, true);
        assert!(error.is_none());
        assert_eq!(outcome.sent, 5);
        assert!(outcome.fallback);
        assert_eq!((outcome.sendmmsg_calls, outcome.sendmmsg_packets), (1, 2));
        assert_eq!(
            *writer.wire.borrow(),
            datagrams(),
            "order kept across the fallback"
        );
    }

    #[test]
    fn would_block_stops_and_leaves_the_rest_unsent() {
        let writer = Scripted::new(
            Many::Accept(1),
            vec![Ok(3), Err(io::Error::from(io::ErrorKind::WouldBlock))],
        );
        let (outcome, error) = run(&writer, true);
        assert!(error.is_none());
        assert_eq!(outcome.sent, 2);
        assert!(outcome.would_block);
        assert_eq!(outcome.consumed(), 2);
        assert_eq!(
            outcome.sendmmsg_calls, 0,
            "a one-packet sendmmsg is not a batch"
        );
    }

    #[test]
    fn a_blocked_sendmmsg_sends_nothing() {
        let writer = Scripted::new(Many::Fail(io::ErrorKind::WouldBlock), vec![]);
        let (outcome, error) = run(&writer, true);
        assert!(error.is_none());
        assert_eq!((outcome.sent, outcome.would_block), (0, true));
        assert!(writer.wire.borrow().is_empty());
    }

    #[test]
    fn a_hard_error_is_reported_for_its_packet() {
        let writer = Scripted::new(
            Many::Fail(io::ErrorKind::PermissionDenied),
            vec![Ok(3), Err(io::Error::from(io::ErrorKind::InvalidInput))],
        );
        let (outcome, error) = run(&writer, true);
        assert_eq!(outcome.sent, 1);
        assert_eq!(
            outcome.consumed(),
            1,
            "a hard error ends the send before the failed packet consumes its number"
        );
        assert!(outcome.fallback);
        assert_eq!(
            error.map(|error| error.kind()),
            Some(io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn a_refused_datagram_is_lost_and_the_rest_still_leave() {
        // A host firewall's egress drop rule (`iptables -j DROP` in
        // OUTPUT) makes `sendto` return EPERM for one datagram. Before the
        // fix that was the call's last packet: the send failed and every
        // consumer ended its outbound audio.
        for refusal in [
            io::Error::from_raw_os_error(libc::EPERM),
            io::Error::from_raw_os_error(libc::ENOBUFS),
            io::Error::from(io::ErrorKind::ConnectionRefused),
            io::Error::from(io::ErrorKind::HostUnreachable),
        ] {
            let kind = refusal.kind();
            let writer = Scripted::new(Many::Unsupported, vec![Ok(3), Err(refusal), Ok(3)]);
            let (outcome, error) = run(&writer, true);
            assert!(error.is_none(), "{kind:?}");
            assert_eq!(outcome.sent, 5, "{kind:?}");
            assert_eq!(
                outcome.consumed(),
                5,
                "every packet consumed its number once"
            );
            let mut expected = datagrams();
            expected.remove(1);
            assert_eq!(
                *writer.wire.borrow(),
                expected,
                "only the refused one is missing"
            );
        }
    }

    #[test]
    fn without_batching_or_sendmmsg_every_packet_is_a_sendto() {
        for (many, batch) in [(Many::Accept(5), false), (Many::Unsupported, true)] {
            let writer = Scripted::new(many, vec![]);
            let (outcome, error) = run(&writer, batch);
            assert!(error.is_none());
            assert_eq!((outcome.sent, outcome.sendmmsg_calls), (5, 0));
            assert_eq!(*writer.wire.borrow(), datagrams());
        }
    }
}
