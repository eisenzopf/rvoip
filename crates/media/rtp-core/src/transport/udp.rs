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
fn spawn_memory_tracked<F>(kind: &'static str, future: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    rvoip_infra_common::memory_diagnostics::spawn_tracked(kind, future)
}

#[cfg(not(feature = "memory-diagnostics"))]
fn spawn_memory_tracked<F>(_: &'static str, future: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
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
use super::symmetric::{SymmetricRtpDecision, SymmetricRtpLearner};
use super::validation::PlatformSocketStrategy;
use super::{RtpTransport, RtpTransportConfig, SymmetricRtpDiagnostics, SymmetricRtpPolicy};
use crate::error::Error;
use crate::packet::rtcp::RtcpPacket;
use crate::packet::RtpPacket;
use crate::traits::RtpEvent;
use crate::Result;

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

pub struct UdpRtpTransport {
    /// RTP socket
    rtp_socket: Arc<UdpSocket>,

    /// RTCP socket (if separate from RTP)
    rtcp_socket: Option<Arc<UdpSocket>>,

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

    /// Remote RTP address. `ArcSwapOption` so per-packet `send_rtp_bytes`
    /// can update the cached symmetric-RTP target with a single atomic
    /// store instead of an awaited `Mutex` guard.
    remote_rtp_addr: Arc<ArcSwapOption<SocketAddr>>,

    /// Remote RTCP address — same lock-free swap discipline as
    /// [`Self::remote_rtp_addr`].
    remote_rtcp_addr: Arc<ArcSwapOption<SocketAddr>>,

    /// Event broadcaster
    event_tx: broadcast::Sender<RtpEvent>,

    /// Receiver task. Only touched on lifecycle (start/stop) so a
    /// `tokio::Mutex` is fine — never on the per-packet path.
    receiver_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,

    /// Whether the transport is active. Polled once per recv-loop
    /// iteration, so an `AtomicBool` keeps that check off the
    /// scheduler.
    active: Arc<AtomicBool>,

    /// Makes allocator release idempotent across explicit close and Drop.
    allocator_release_started: AtomicBool,

    /// Outbound SRTP context (RFC 4568 / RFC 3711). When `Some`, every
    /// outbound RTP packet is wrapped with `SrtpContext::protect`
    /// before being sent on the wire. When `None`, plain RTP — keeps
    /// the existing UDP path unchanged. `parking_lot::Mutex` because
    /// `protect()` is `&mut self` + CPU-only (AES/HMAC); the lock is
    /// never held across `.await`, so the tokio scheduler overhead
    /// would be pure tax.
    srtp_send: Arc<parking_lot::Mutex<Option<crate::srtp::SrtpContext>>>,

    /// Inbound SRTP context. When `Some`, every received RTP datagram
    /// (non-RTCP) is fed through `SrtpContext::unprotect`; auth
    /// failures are silently dropped per RFC 3711 §3.4. Same
    /// `parking_lot::Mutex` rationale as [`Self::srtp_send`].
    srtp_recv: Arc<parking_lot::Mutex<Option<crate::srtp::SrtpContext>>>,

    /// RFC 4733 §2.5.1.3 retransmit dedup state, keyed on
    /// `(peer_addr, ssrc, rtp_timestamp)`. The first end-of-event
    /// frame per tone is forwarded; the two retransmits collide on
    /// this key and are suppressed before they reach `event_tx`. See
    /// [`DTMF_DEDUP_TTL`] for the per-entry lifetime. `peer_addr` is
    /// part of the key because rtp-core has no dialog scope at the
    /// socket layer — two simultaneous DTMF streams from different
    /// peers must each fire independently.
    dtmf_seen: Arc<DashMap<(SocketAddr, u32, u32), Instant>>,

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

            (socket_rtp, socket_rtcp, local_rtp_addr, local_rtcp_addr)
        };

        // Create broadcaster
        let (event_tx, _) = broadcast::channel(buffer_config.event_channel_capacity.max(1));

        let transport = Self {
            rtp_socket: Arc::new(socket_rtp),
            rtcp_socket: socket_rtcp.map(Arc::new),
            config,
            symmetric_rtp_policy: policy,
            symmetric_rtp_counters: Arc::new(SymmetricRtpCounters::default()),
            symmetric_rtp_latched: Arc::new(AtomicBool::new(false)),
            symmetric_rtp_generation: Arc::new(AtomicU64::new(0)),
            remote_rtp_addr: Arc::new(ArcSwapOption::from(None)),
            remote_rtcp_addr: Arc::new(ArcSwapOption::from(None)),
            event_tx,
            receiver_tasks: Arc::new(Mutex::new(Vec::with_capacity(2))),
            active: Arc::new(AtomicBool::new(false)),
            allocator_release_started: AtomicBool::new(false),
            srtp_send: Arc::new(parking_lot::Mutex::new(None)),
            srtp_recv: Arc::new(parking_lot::Mutex::new(None)),
            dtmf_seen: Arc::new(DashMap::new()),
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
        let dtmf_seen = self.dtmf_seen.clone();
        let symmetric_rtp_policy = self.symmetric_rtp_policy;
        let symmetric_rtp_counters = self.symmetric_rtp_counters.clone();
        let symmetric_rtp_latched = self.symmetric_rtp_latched.clone();
        let symmetric_rtp_generation = self.symmetric_rtp_generation.clone();
        let remote_rtp_addr = self.remote_rtp_addr.clone();
        let remote_rtcp_addr = self.remote_rtcp_addr.clone();
        let rtcp_mux = self.config.rtcp_mux;
        let srtp_diagnostics = srtp_diagnostics_enabled();
        let rtp_diagnostics = rtp_diagnostics_enabled();
        let local_rtp_addr = rtp_socket.local_addr().ok();
        let recv_buffer_size = self.config.buffer_config.recv_buffer_size;

        let rtp_receiver = spawn_memory_tracked(
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

                loop {
                    // Check if we should continue running
                    if !active_state.load(Ordering::Acquire) {
                        break;
                    }

                    let mut buffer = vec![0u8; recv_buffer_size];

                    // Receive packet
                    match rtp_socket.recv_from(&mut buffer).await {
                        Ok((size, addr)) => {
                            info!("🔵 UDP recv_from returned {} bytes from {}", size, addr);

                            let packet_class = classify_rtp_mux_packet(&buffer[..size]);
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
                                if rtcp_mux
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
                                let rtcp_data = Bytes::copy_from_slice(&buffer[..size]);
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
                                // SRTP unprotect (RFC 3711 §3.4) when an
                                // inbound SrtpContext is configured. Auth
                                // failures MUST be silently dropped — no
                                // event, no warn-level log — to avoid
                                // leaking timing or distinguishing failure
                                // modes to a network attacker.
                                let mut srtp_guard = srtp_recv.lock();
                                if srtp_diagnostics && !first_inbound_rtp_logged {
                                    info!(
                                    "SRTP_DIAG inbound_rtp_first local={:?} source={} size={} srtp_context={}",
                                    local_rtp_addr,
                                    addr,
                                    size,
                                    srtp_guard.is_some()
                                );
                                    first_inbound_rtp_logged = true;
                                }
                                let parse_result: Result<RtpPacket> = if let Some(ctx) =
                                    srtp_guard.as_mut()
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
                                } else {
                                    RtpPacket::parse(&buffer[..size])
                                };
                                drop(srtp_guard);
                                match parse_result {
                                    Ok(packet) => {
                                        let generation =
                                            symmetric_rtp_generation.load(Ordering::Acquire);
                                        if generation != observed_symmetric_generation {
                                            symmetric_rtp.reset();
                                            observed_symmetric_generation = generation;
                                        }
                                        match symmetric_rtp.observe(
                                            addr,
                                            packet.header.ssrc,
                                            packet.header.sequence_number,
                                            Instant::now(),
                                        ) {
                                            SymmetricRtpDecision::Accept => {}
                                            SymmetricRtpDecision::LatchInitial => {
                                                // Publish the guard first so an
                                                // overlapping outbound send
                                                // cannot restore its stale SDP
                                                // destination after this store.
                                                symmetric_rtp_latched
                                                    .store(true, Ordering::Release);
                                                remote_rtp_addr.store(Some(Arc::new(addr)));
                                                if rtcp_mux {
                                                    remote_rtcp_addr.store(Some(Arc::new(addr)));
                                                }
                                                symmetric_rtp_counters
                                                    .destination_learned
                                                    .store(true, Ordering::Relaxed);
                                            }
                                            SymmetricRtpDecision::Rebind => {
                                                remote_rtp_addr.store(Some(Arc::new(addr)));
                                                if rtcp_mux {
                                                    remote_rtcp_addr.store(Some(Arc::new(addr)));
                                                }
                                                symmetric_rtp_counters
                                                    .accepted_rebindings
                                                    .fetch_add(1, Ordering::Relaxed);
                                            }
                                            SymmetricRtpDecision::Probation => {
                                                symmetric_rtp_counters
                                                    .probation_packets
                                                    .fetch_add(1, Ordering::Relaxed);
                                                continue;
                                            }
                                            SymmetricRtpDecision::Reject => {
                                                symmetric_rtp_counters
                                                    .rejected_packets
                                                    .fetch_add(1, Ordering::Relaxed);
                                                continue;
                                            }
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
                                            let p = &packet.payload[..4];
                                            let event = p[0];
                                            let byte1 = p[1];
                                            let end_of_event = (byte1 & 0b1000_0000) != 0;
                                            let volume = byte1 & 0b0011_1111;
                                            let duration = u16::from_be_bytes([p[2], p[3]]);

                                            // RFC 4733 §2.5.1.3 retransmit dedup. The
                                            // sender emits up to three identical E=1
                                            // frames sharing `(ssrc, rtp_timestamp)`.
                                            // Keyed by `(peer_addr, ssrc, ts)` so two
                                            // simultaneous DTMF streams from
                                            // different peers fire independently.
                                            // Inline retain prunes stale entries on
                                            // every fire — at one PT 101 frame per
                                            // ~20 ms per active tone, this stays
                                            // bounded.
                                            if end_of_event {
                                                let key = (
                                                    addr,
                                                    packet.header.ssrc,
                                                    packet.header.timestamp,
                                                );
                                                let now = Instant::now();
                                                dtmf_seen.retain(|_, seen_at| {
                                                    now.duration_since(*seen_at) < DTMF_DEDUP_TTL
                                                });
                                                if dtmf_seen.insert(key, now).is_some() {
                                                    continue; // retransmit — suppress
                                                }
                                            }

                                            let dtmf = RtpEvent::DtmfEvent {
                                                event,
                                                end_of_event,
                                                volume,
                                                duration,
                                                timestamp: packet.header.timestamp,
                                                source: addr,
                                                ssrc: packet.header.ssrc,
                                            };
                                            if event_tx.receiver_count() > 0 {
                                                if let Err(e) = event_tx.send(dtmf) {
                                                    warn!("Failed to send DTMF event: {}", e);
                                                }
                                            } else {
                                                let _ = event_tx.send(dtmf);
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

        // Store task handle
        let mut receiver_tasks = self.receiver_tasks.lock().await;
        receiver_tasks.push(rtp_receiver);

        // If we have a separate RTCP socket, start that receiver too
        if let Some(rtcp_socket) = &self.rtcp_socket {
            let rtcp_socket = rtcp_socket.clone();
            let event_tx = self.event_tx.clone();
            let active_state = self.active.clone();
            let rtcp_recv_buffer_size = self.config.buffer_config.rtcp_recv_buffer_size;

            let rtcp_receiver =
                spawn_memory_tracked("rtp_core.udp_transport.rtcp_receiver_task", async move {
                    loop {
                        // Check if we should continue running
                        if !active_state.load(Ordering::Acquire) {
                            break;
                        }

                        let mut buffer = vec![0u8; rtcp_recv_buffer_size];

                        // Receive packet
                        match rtcp_socket.recv_from(&mut buffer).await {
                            Ok((size, addr)) => {
                                // Create RTCP event
                                let rtcp_data = Bytes::copy_from_slice(&buffer[..size]);
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
                            }
                            Err(e) => {
                                error!("Error receiving RTCP packet: {}", e);

                                // Send error event
                                let err_event = RtpEvent::Error(Error::Transport(format!(
                                    "RTCP socket error: {}",
                                    e
                                )));
                                if event_tx.receiver_count() > 0 {
                                    let _ = event_tx.send(err_event);
                                }

                                // Short delay before retrying
                                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                            }
                        }
                    }
                });

            receiver_tasks.push(rtcp_receiver);
        }

        info!("Started UDP transport receiver tasks");
        Ok(())
    }

    /// Stop the receiver task
    pub async fn stop_receiver(&self) -> Result<()> {
        // Set inactive state
        self.active.store(false, Ordering::Release);

        // Wait for receiver task to complete
        let mut receiver_tasks = self.receiver_tasks.lock().await;
        for task in receiver_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }

        Ok(())
    }

    /// Set the remote RTP address
    pub async fn set_remote_rtp_addr(&self, addr: SocketAddr) {
        self.symmetric_rtp_latched.store(false, Ordering::Release);
        self.remote_rtp_addr.store(Some(Arc::new(addr)));
        self.symmetric_rtp_generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Set the remote RTCP address
    pub async fn set_remote_rtcp_addr(&self, addr: SocketAddr) {
        self.remote_rtcp_addr.store(Some(Arc::new(addr)));
    }

    /// Get the remote RTP address
    pub async fn remote_rtp_addr(&self) -> Option<SocketAddr> {
        self.remote_rtp_addr.load().as_deref().copied()
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

    /// Subscribe to transport events
    pub fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        self.event_tx.subscribe()
    }

    /// Get a clone of the RTP socket
    /// This is used when sharing the same socket with other protocols (e.g., DTLS)
    pub fn get_socket(&self) -> Arc<UdpSocket> {
        self.rtp_socket.clone()
    }

    /// Install per-direction SRTP contexts (RFC 4568 §6.1, RFC 3711).
    ///
    /// `send` is consumed by `send_rtp` to wrap every outbound RTP
    /// packet with `protect()`; `recv` is consumed by the receive
    /// loop to `unprotect()` every inbound RTP datagram (RTCP is not
    /// SRTP-protected — SRTCP support is a separate feature).
    ///
    /// Setting both contexts is the *only* way to switch this
    /// transport from plain RTP to SRTP. Calling this method is
    /// idempotent: a second call replaces the contexts (used today
    /// only in tests; mid-call rekeying is out of scope for this
    /// step).
    pub async fn set_srtp_contexts(
        &self,
        send: crate::srtp::SrtpContext,
        recv: crate::srtp::SrtpContext,
    ) {
        *self.srtp_send.lock() = Some(send);
        *self.srtp_recv.lock() = Some(recv);
        if srtp_diagnostics_enabled() {
            info!(
                "SRTP_DIAG contexts_installed local={:?}",
                self.rtp_socket.local_addr().ok()
            );
        }
    }

    /// Whether SRTP is currently configured on this transport. Used by
    /// tests + diagnostic introspection; the send/receive paths
    /// branch internally on the same `Option`.
    pub async fn srtp_enabled(&self) -> bool {
        self.srtp_send.lock().is_some() || self.srtp_recv.lock().is_some()
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
            self.send_rtp_bytes(&data, dest).await
        } else {
            buffer.clear();
            packet.header.serialize(buffer)?;
            buffer.extend_from_slice(&packet.payload);
            self.send_rtp_bytes(buffer, dest).await
        }
    }
}

#[async_trait]
impl RtpTransport for UdpRtpTransport {
    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        self.rtp_socket
            .local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get local RTP address: {}", e)))
    }

    /// Get the local RTCP address
    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>> {
        Ok(self.config.local_rtcp_addr)
    }

    async fn send_rtp(&self, packet: &RtpPacket, dest: SocketAddr) -> Result<()> {
        let mut buffer = BytesMut::with_capacity(packet.size() + 16);
        self.send_rtp_with_buffer(packet, dest, &mut buffer).await
    }

    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.config.symmetric_rtp && !self.symmetric_rtp_latched.load(Ordering::Acquire) {
            // Seed the pre-latch target for callers that send before media is
            // received. Once a validated inbound tuple is latched, outbound
            // calls must not restore a stale SDP destination.
            self.remote_rtp_addr.store(Some(Arc::new(dest)));
        }

        // Send the data
        let sent_bytes = self
            .rtp_socket
            .send_to(bytes, dest)
            .await
            .map_err(|e| Error::Transport(format!("Failed to send RTP packet: {}", e)))?;

        debug!("UDP send_to sent {} bytes to {}", sent_bytes, dest);
        Ok(())
    }

    async fn send_rtcp(&self, packet: &RtcpPacket, dest: SocketAddr) -> Result<()> {
        // Serialize the packet
        let data = packet.serialize()?;

        // Send the serialized bytes
        self.send_rtcp_bytes(&data, dest).await
    }

    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.config.symmetric_rtp && !self.symmetric_rtp_latched.load(Ordering::Acquire) {
            // Same pre-latch seeding rule as RTP.
            self.remote_rtcp_addr.store(Some(Arc::new(dest)));
        }

        // Use the appropriate socket for sending RTCP
        let socket = if self.config.rtcp_mux {
            // If RTCP-MUX is enabled, use the RTP socket for RTCP packets
            &self.rtp_socket
        } else if let Some(rtcp_socket) = &self.rtcp_socket {
            // If a separate RTCP socket exists, use it
            rtcp_socket
        } else {
            // Fallback to RTP socket if no RTCP socket is available
            &self.rtp_socket
        };

        // Send the data
        socket
            .send_to(bytes, dest)
            .await
            .map_err(|e| Error::Transport(format!("Failed to send RTCP packet: {}", e)))?;

        Ok(())
    }

    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        // Receive data from the RTP socket
        self.rtp_socket
            .recv_from(buffer)
            .await
            .map_err(|e| Error::Transport(format!("Failed to receive packet: {}", e)))
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
        if let Ok(mut tasks) = self.receiver_tasks.try_lock() {
            for task in tasks.drain(..) {
                task.abort();
            }
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
/// 2. For these ambiguous payload types, a packet is RTCP if
///    the second byte is a known RTCP packet type (SR=200, RR=201,
///    SDES=202, BYE=203, APP=204, RTPFB=205, PSFB=206, XR=207).
/// 3. All other packets in the range 64-95 are RTP.
/// 4. All packets with payload types in the range 0-63 and 96-127 are RTP.
///
/// See RFC 5761 section 4 for more details.
fn is_rtcp_packet(buffer: &[u8]) -> bool {
    if buffer.len() < 2 {
        return false;
    }

    let first_byte = buffer[0];
    let second_byte = buffer[1];

    let version = (first_byte >> 6) & 0x03;
    // For RTP, payload type is in the lower 7 bits of the second byte
    // For RTCP, packet type is the full second byte value

    // First check: If the packet type is between 200-207, it's RTCP.
    if version == 2 && (second_byte >= 200 && second_byte <= 207) {
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
            transport.rtcp_socket.is_some(),
            "RTCP socket should exist when rtcp_mux is false"
        );

        // Check the actual RTCP socket address, not just the config value
        if let Some(rtcp_socket) = &transport.rtcp_socket {
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
            transport.rtcp_socket.is_none(),
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
        assert_eq!(transport.receiver_tasks.lock().await.len(), 2);

        transport.close().await.unwrap();

        assert!(transport.receiver_tasks.lock().await.is_empty());
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
        let packet = RtpPacket::new(header, payload.clone());

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
            transport.rtcp_socket.is_some(),
            "RTCP socket should be created"
        );

        // Check the actual RTCP socket address, not just the config value
        if let Some(rtcp_socket) = &transport.rtcp_socket {
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
            .as_ref()
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
            transport.rtcp_socket.is_none(),
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
        transport_a.set_srtp_contexts(a_send, a_recv).await;
        transport_b.set_srtp_contexts(b_send, b_recv).await;
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
        transport_a.set_srtp_contexts(a_send, a_recv).await;
        transport_b.set_srtp_contexts(b_send, b_recv).await;

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
        transport_a.set_srtp_contexts(a_send, a_recv).await;

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
            .await;

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
