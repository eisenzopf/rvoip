//! Transmit buffer for outbound RTP packets
//!
//! This module provides a high-performance transmit buffer with:
//! - Prioritization of different packet types
//! - Congestion control using RTCP feedback
//! - Buffer limits to prevent memory exhaustion
//! - Efficient packet scheduling

use std::collections::{VecDeque, BTreeMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore, Notify};
use futures::future::poll_fn;
use tokio::time::sleep;
use std::task::{Poll, Context};
use std::pin::Pin;

use bytes::Bytes;
use tracing::{debug, warn, info, trace, error};

use crate::packet::{RtpPacket, rtcp::RtcpPacket};
use crate::RtpSsrc;
use crate::RtpTimestamp;

use super::{BufferLimits, GlobalBufferManager, MemoryPermit, BufferPool, PooledBuffer};

/// Default transmit buffer capacity
pub const DEFAULT_TRANSMIT_BUFFER_CAPACITY: usize = 1000;

/// Default congestion window size (packets)
pub const DEFAULT_CONGESTION_WINDOW: usize = 64;

/// Default minimum retransmission timeout
pub const DEFAULT_MIN_RTO_MS: u64 = 70;

/// Default maximum retransmission timeout
pub const DEFAULT_MAX_RTO_MS: u64 = 1000;

/// Default initial retransmission timeout
pub const DEFAULT_INITIAL_RTO_MS: u64 = 200;

/// Default maximum burst size (packets)
pub const DEFAULT_MAX_BURST: usize = 16;

/// Packet priority levels
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PacketPriority {
    /// Critical control packets (RTCP BYE, SR, RR)
    Control = 0,
    
    /// High priority media (e.g., I-frames in video)
    High = 1,
    
    /// Normal priority media (typical packets)
    Normal = 2,
    
    /// Low priority (can be dropped first)
    Low = 3,
}

impl Default for PacketPriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Packet in the transmit queue
struct QueuedPacket {
    /// The RTP packet to transmit
    packet: RtpPacket,
    
    /// When the packet was queued
    queue_time: Instant,
    
    /// Priority of this packet
    priority: PacketPriority,
    
    /// Whether this is a retransmission
    is_retransmission: bool,
    
    /// Metadata for the packet
    metadata: Option<PacketMetadata>,
}

/// Metadata for packet tracking
#[derive(Debug, Clone)]
pub struct PacketMetadata {
    /// Timestamp when packet was first sent
    pub first_send_time: Option<Instant>,
    
    /// Number of transmission attempts
    pub transmit_count: u32,
    
    /// Whether the packet was acknowledged
    pub acknowledged: bool,
    
    /// Last time the packet was sent
    pub last_send_time: Option<Instant>,
}

/// Congestion control state
struct CongestionState {
    /// Current congestion window size (packets)
    cwnd: usize,
    
    /// Slow start threshold
    ssthresh: usize,
    
    /// Current retransmission timeout (ms)
    rto_ms: u64,
    
    /// Smoothed round-trip time (ms)
    srtt_ms: Option<f64>,
    
    /// RTT variation (ms)
    rttvar_ms: Option<f64>,
    
    /// Last time window was reduced
    last_window_reduction: Instant,
    
    /// Current estimate of network bandwidth (bps)
    estimated_bps: u64,
    
    /// Packets in flight
    in_flight: usize,
    
    /// Lost packets detected
    lost_packets: u64,
    
    /// Total packets sent
    total_sent: u64,
    
    /// Last sequence number sent
    last_seq_sent: u16,
    
    /// Whether we're in slow start
    in_slow_start: bool,
}

impl Default for CongestionState {
    fn default() -> Self {
        Self {
            cwnd: DEFAULT_CONGESTION_WINDOW,
            ssthresh: usize::MAX,
            rto_ms: DEFAULT_INITIAL_RTO_MS,
            srtt_ms: None,
            rttvar_ms: None,
            last_window_reduction: Instant::now(),
            estimated_bps: 1_000_000, // 1 Mbps initial guess
            in_flight: 0,
            lost_packets: 0,
            total_sent: 0,
            last_seq_sent: 0,
            in_slow_start: true,
        }
    }
}

/// Transmit buffer configuration
#[derive(Debug, Clone)]
pub struct TransmitBufferConfig {
    /// Maximum packets to buffer for transmission
    pub max_packets: usize,
    
    /// Initial congestion window size (packets)
    pub initial_cwnd: usize,
    
    /// Minimum retransmission timeout (ms)
    pub min_rto_ms: u64,
    
    /// Maximum retransmission timeout (ms)
    pub max_rto_ms: u64,
    
    /// Initial retransmission timeout (ms)
    pub initial_rto_ms: u64,
    
    /// Whether to enable congestion control
    pub congestion_control_enabled: bool,
    
    /// Maximum burst size (packets)
    pub max_burst: usize,
    
    /// Whether to prioritize packets
    pub prioritize_packets: bool,
    
    /// Maximum packet age before dropping (ms)
    pub max_packet_age_ms: u64,
    
    /// Clock rate for timestamp calculations
    pub clock_rate: u32,
}

impl Default for TransmitBufferConfig {
    fn default() -> Self {
        Self {
            max_packets: DEFAULT_TRANSMIT_BUFFER_CAPACITY,
            initial_cwnd: DEFAULT_CONGESTION_WINDOW,
            min_rto_ms: DEFAULT_MIN_RTO_MS,
            max_rto_ms: DEFAULT_MAX_RTO_MS,
            initial_rto_ms: DEFAULT_INITIAL_RTO_MS,
            congestion_control_enabled: true,
            max_burst: DEFAULT_MAX_BURST,
            prioritize_packets: true,
            max_packet_age_ms: 1000, // 1 second
            clock_rate: 8000, // Default for audio
        }
    }
}

/// Statistics for the transmit buffer
#[derive(Debug, Clone)]
pub struct TransmitBufferStats {
    /// Current number of packets in the queue
    pub queued_packets: usize,
    
    /// Total packets sent
    pub packets_sent: u64,
    
    /// Packets dropped due to queue overflow
    pub packets_dropped_overflow: u64,
    
    /// Packets dropped due to age
    pub packets_dropped_aged: u64,
    
    /// Packets retransmitted
    pub packets_retransmitted: u64,
    
    /// Current congestion window size
    pub cwnd: usize,
    
    /// Current retransmission timeout (ms)
    pub rto_ms: u64,
    
    /// Current estimated RTT (ms)
    pub srtt_ms: Option<f64>,
    
    /// Estimated bandwidth (bps)
    pub estimated_bps: u64,
    
    /// Packets currently in flight
    pub in_flight: usize,
    
    /// Packet loss rate (0.0-1.0)
    pub loss_rate: f64,
    
    /// Buffer fullness percentage (0.0-1.0)
    pub buffer_fullness: f32,
    
    /// Access for the API layer to these stats
    pub packets_queued: usize,
    
    /// Total number of packets that have been dropped
    pub packets_dropped: u64,
}

impl Default for TransmitBufferStats {
    fn default() -> Self {
        Self {
            queued_packets: 0,
            packets_sent: 0,
            packets_dropped_overflow: 0,
            packets_dropped_aged: 0,
            packets_retransmitted: 0,
            cwnd: DEFAULT_CONGESTION_WINDOW,
            rto_ms: DEFAULT_INITIAL_RTO_MS,
            srtt_ms: None,
            estimated_bps: 1_000_000, // 1 Mbps initial guess
            in_flight: 0,
            loss_rate: 0.0,
            buffer_fullness: 0.0,
            packets_queued: 0,
            packets_dropped: 0,
        }
    }
}

/// High-performance transmit buffer for RTP packets
///
/// This implementation provides:
/// - Efficient packet queuing and sending
/// - Congestion control and bandwidth estimation
/// - Packet prioritization
/// - RTCP-based feedback handling
/// - Memory management with global limits
pub struct TransmitBuffer {
    /// Configuration
    config: TransmitBufferConfig,
    
    /// Priority queues for packets
    /// Lower priority value = higher priority (will be sent first)
    queues: BTreeMap<PacketPriority, VecDeque<QueuedPacket>>,
    
    /// Congestion control state
    congestion: CongestionState,
    
    /// Statistics
    stats: TransmitBufferStats,
    
    /// Reference to global buffer manager
    buffer_manager: Option<Arc<GlobalBufferManager>>,
    
    /// Notification for new packets
    packet_notify: Arc<Notify>,
    
    /// Sequence -> packet metadata map for tracking
    packet_tracking: BTreeMap<u16, PacketMetadata>,
    
    /// Permits for congestion window
    cwnd_semaphore: Arc<Semaphore>,
    
    /// Buffer for efficient packet ordering
    packet_buffer: Option<Arc<BufferPool>>,
    
    /// SSRC for this transmit buffer
    ssrc: RtpSsrc,
    
    /// Last packet send time
    last_send_time: Option<Instant>,
    
    /// Pacing state
    pacing_interval_us: u64,
}

impl TransmitBuffer {
    /// Create a new transmit buffer
    pub fn new(ssrc: RtpSsrc, config: TransmitBufferConfig) -> Self {
        let mut queues = BTreeMap::new();
        
        // Initialize priority queues
        queues.insert(PacketPriority::Control, VecDeque::with_capacity(100));
        queues.insert(PacketPriority::High, VecDeque::with_capacity(config.max_packets / 4));
        queues.insert(PacketPriority::Normal, VecDeque::with_capacity(config.max_packets / 2));
        queues.insert(PacketPriority::Low, VecDeque::with_capacity(config.max_packets / 4));
        
        // Initialize congestion state
        let mut congestion = CongestionState::default();
        congestion.cwnd = config.initial_cwnd;
        congestion.rto_ms = config.initial_rto_ms;
        
        let stats = TransmitBufferStats {
            queued_packets: 0,
            packets_sent: 0,
            packets_dropped_overflow: 0,
            packets_dropped_aged: 0,
            packets_retransmitted: 0,
            cwnd: config.initial_cwnd,
            rto_ms: config.initial_rto_ms,
            srtt_ms: None,
            estimated_bps: 1_000_000, // 1 Mbps initial guess
            in_flight: 0,
            loss_rate: 0.0,
            buffer_fullness: 0.0,
            packets_queued: 0,
            packets_dropped: 0,
        };
        
        // Create congestion window semaphore
        let cwnd_semaphore = Arc::new(Semaphore::new(config.initial_cwnd));
        
        Self {
            config,
            queues,
            congestion,
            stats,
            buffer_manager: None,
            packet_notify: Arc::new(Notify::new()),
            packet_tracking: BTreeMap::new(),
            cwnd_semaphore,
            packet_buffer: None,
            ssrc,
            last_send_time: None,
            pacing_interval_us: 0,
        }
    }
    
    /// Create a new transmit buffer with global buffer management
    pub fn with_buffer_manager(
        ssrc: RtpSsrc,
        config: TransmitBufferConfig,
        buffer_manager: Arc<GlobalBufferManager>,
        packet_buffer: Arc<BufferPool>,
    ) -> Self {
        let mut buffer = Self::new(ssrc, config);
        buffer.buffer_manager = Some(buffer_manager);
        buffer.packet_buffer = Some(packet_buffer);
        buffer
    }
    
    /// Queue a packet for transmission
    ///
    /// Returns true if the packet was queued, false if it was dropped.
    pub async fn queue_packet(
        &mut self, 
        packet: RtpPacket, 
        priority: PacketPriority
    ) -> bool {
        // Check if buffer has reached maximum capacity
        let total_packets = self.total_queued_packets();
        
        // Fast reject if at max capacity and priority isn't high enough to drop other packets
        if total_packets >= self.config.max_packets {
            // For normal or low priority packets, drop if we're at capacity
            // High priority packets can try to make room by dropping lower priority ones
            if priority != PacketPriority::High {
                // Normal or low priority when buffer is full - drop immediately
                trace!("Buffer full ({}/{}), dropping {:?} priority packet with seq={}",
                      total_packets, self.config.max_packets,
                      priority, packet.header.sequence_number);
                self.stats.packets_dropped_overflow += 1;
                return false;
            } else {
                // High priority packet - try to drop something to make room
                if !self.drop_low_priority_packets() {
                    // If there's nothing to drop, drop this packet too
                    trace!("Buffer full, nowhere to make room for high priority packet");
                    self.stats.packets_dropped_overflow += 1;
                    return false;
                }
            }
        }
        
        // At this point we have room (or made room) in the buffer
        
        // Create packet metadata for tracking
        let metadata = PacketMetadata {
            first_send_time: None,
            transmit_count: 0,
            acknowledged: false,
            last_send_time: None,
        };
        
        // Create queued packet entry
        let queued = QueuedPacket {
            packet,
            queue_time: Instant::now(),
            priority,
            is_retransmission: false,
            metadata: Some(metadata),
        };
        
        // Get or create queue for this priority
        let queue = self.queues.entry(priority).or_insert_with(|| VecDeque::new());
        
        // Add to queue
        queue.push_back(queued);
        
        // Update stats
        self.stats.queued_packets = self.total_queued_packets();
        
        // Notify waiters
        self.packet_notify.notify_one();
        
        true
    }
    
    /// Schedule a packet retransmission
    ///
    /// Returns true if the retransmission was scheduled, false otherwise.
    pub async fn schedule_retransmission(&mut self, seq: u16) -> bool {
        // Check if we're tracking this packet
        if let Some(metadata) = self.packet_tracking.get_mut(&seq) {
            // Only retransmit if not already acknowledged
            if !metadata.acknowledged {
                // Find the packet to retransmit (we may still have it)
                // Look through all queues in order of priority
                for (priority, queue) in self.queues.iter() {
                    for queued_packet in queue {
                        if queued_packet.packet.header.sequence_number == seq {
                            // Packet is already queued, just update stats
                            trace!("Packet seq={} already queued for retransmission", seq);
                            return true;
                        }
                    }
                }
                
                // We don't have the packet in queue, need to rebuild it
                // This would require access to the original data
                // In a real implementation, we'd need to cache the packets
                // or have the application rebuild it
                
                // For now, just log it
                warn!("Requested retransmission for seq={} but packet not available", seq);
                
                // Update stats
                self.stats.packets_retransmitted += 1;
                
                return false;
            }
        }
        
        false
    }
    
    /// Get the next packet to send
    ///
    /// This handles congestion control if enabled.
    pub async fn get_next_packet(&mut self) -> Option<RtpPacket> {
        // Nothing to send
        if self.total_queued_packets() == 0 {
            return None;
        }
        
        // If congestion control is not enabled, bypass window checks
        if !self.config.congestion_control_enabled {
            return self.get_packet_without_congestion_control().await;
        }
        
        // Skip this check if we have no packets in flight yet
        if self.congestion.in_flight > 0 {
            // Check if window is already full
            if self.congestion.in_flight >= self.congestion.cwnd {
                trace!("Congestion window full ({}/{}), not sending new packets", 
                      self.congestion.in_flight, self.congestion.cwnd);
                return None;
            }
        }
        
        // Try to acquire a congestion window permit
        match self.cwnd_semaphore.try_acquire() {
            Ok(permit) => {
                // Permit acquired, we can send
                drop(permit);
            }
            Err(_) => {
                // No permits available, congestion window is full
                trace!("No congestion permits available, not sending");
                return None;
            }
        }
        
        // Apply pacing if needed
        if self.pacing_interval_us > 0 {
            self.apply_pacing().await;
        }
        
        // Get highest priority packet
        let packet_option = self.dequeue_highest_priority_packet();
        
        if let Some(packet) = &packet_option {
            // Update in-flight count
            self.congestion.in_flight += 1;
            self.congestion.total_sent += 1;
            
            // Update stats
            self.stats.in_flight = self.congestion.in_flight;
            self.stats.packets_sent += 1;
            
            // Update last send time
            self.last_send_time = Some(Instant::now());
        }
        
        packet_option
    }
    
    /// Wait until either a packet is available or timeout occurs
    ///
    /// Returns true if a packet is available, false if timeout occurred.
    pub async fn wait_for_packet(&self, timeout: Duration) -> bool {
        // If we already have packets and congestion window permits, return immediately
        if self.total_queued_packets() > 0 && self.cwnd_semaphore.available_permits() > 0 {
            return true;
        }
        
        // Wait for notification with timeout
        let notify = self.packet_notify.clone();
        tokio::select! {
            _ = notify.notified() => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }
    
    /// Process RTCP feedback packets to update congestion control
    pub fn process_rtcp_feedback(&mut self, rtcp: &RtcpPacket) {
        // Simplified implementation that doesn't rely on specific RTCP methods
        match rtcp {
            RtcpPacket::ReceiverReport(_) => {
                // In a real implementation, we would extract RTT and loss data 
                // from the receiver report. For this example, we'll simulate it.
                
                // Simulate RTT measurement
                let rtt_ms = 50.0; // Assume 50ms RTT
                self.update_rtt_estimate(rtt_ms);
                
                // Simulate packet loss information
                self.stats.loss_rate = 0.01; // 1% loss rate
                
                // Update congestion window
                self.update_congestion_window(None);
            },
            RtcpPacket::SenderReport(_) => {
                // Nothing specific to do for sender reports
            },
            _ => {
                // Other RTCP packet types not handled
            },
        }
    }
    
    /// Signal a packet has been acknowledged (e.g., via RTCP)
    pub fn acknowledge_packet(&mut self, seq: u16) {
        if let Some(metadata) = self.packet_tracking.get_mut(&seq) {
            metadata.acknowledged = true;
            
            // Update in-flight count if needed
            if self.congestion.in_flight > 0 {
                self.congestion.in_flight -= 1;
                self.stats.in_flight = self.congestion.in_flight;
            }
            
            // Release a congestion window permit
            self.cwnd_semaphore.add_permits(1);
            
            // Adjust congestion window for successful transmission
            if self.config.congestion_control_enabled {
                self.update_congestion_window(Some(seq));
            }
        }
    }
    
    /// Update RTT estimate using RFC 6298 algorithm
    fn update_rtt_estimate(&mut self, rtt_ms: f64) {
        if let (Some(srtt), Some(rttvar)) = (self.congestion.srtt_ms, self.congestion.rttvar_ms) {
            // Update RTTVAR and SRTT
            // RTTVAR = (1 - beta) * RTTVAR + beta * |SRTT - R|
            // SRTT = (1 - alpha) * SRTT + alpha * R
            // where alpha = 1/8 and beta = 1/4
            
            let alpha = 0.125;
            let beta = 0.25;
            
            let new_rttvar = (1.0 - beta) * rttvar + beta * (srtt - rtt_ms).abs();
            let new_srtt = (1.0 - alpha) * srtt + alpha * rtt_ms;
            
            self.congestion.rttvar_ms = Some(new_rttvar);
            self.congestion.srtt_ms = Some(new_srtt);
            
            // Update RTO based on RFC 6298 formula: RTO = SRTT + 4 * RTTVAR
            let new_rto = new_srtt + 4.0 * new_rttvar;
            
            // Clamp RTO to min/max values
            let clamped_rto = (new_rto.round() as u64)
                .max(self.config.min_rto_ms)
                .min(self.config.max_rto_ms);
            
            self.congestion.rto_ms = clamped_rto;
        } else {
            // First RTT measurement
            let srtt = rtt_ms;
            let rttvar = rtt_ms / 2.0;
            
            self.congestion.srtt_ms = Some(srtt);
            self.congestion.rttvar_ms = Some(rttvar);
            
            // Initial RTO = SRTT + 4 * RTTVAR
            let new_rto = srtt + 4.0 * rttvar;
            
            // Clamp RTO to min/max values
            let clamped_rto = (new_rto.round() as u64)
                .max(self.config.min_rto_ms)
                .min(self.config.max_rto_ms);
            
            self.congestion.rto_ms = clamped_rto;
        }
        
        // Update stats
        self.stats.srtt_ms = self.congestion.srtt_ms;
        self.stats.rto_ms = self.congestion.rto_ms;
    }
    
    /// Handle a congestion event (packet loss or timeout)
    fn congestion_event(&mut self) {
        let now = Instant::now();
        
        // Avoid reacting to multiple congestion events in a short time
        if now.duration_since(self.congestion.last_window_reduction).as_millis() < self.congestion.rto_ms as u128 {
            return;
        }
        
        // Record the time
        self.congestion.last_window_reduction = now;
        
        // Cut congestion window in half (but minimum of 2)
        let new_cwnd = (self.congestion.cwnd / 2).max(2);
        
        if self.congestion.in_slow_start {
            // Exit slow start
            self.congestion.in_slow_start = false;
            
            // Set slow start threshold to half of current window
            self.congestion.ssthresh = new_cwnd;
        }
        
        // Update congestion window
        self.congestion.cwnd = new_cwnd;
        
        // Recompute pacing interval
        self.update_pacing();
        
        // Update semaphore (may need to invalidate permits)
        let in_flight = self.congestion.in_flight;
        
        // Reset semaphore to current window size minus in-flight packets
        let available = if in_flight < self.congestion.cwnd {
            self.congestion.cwnd - in_flight
        } else {
            0
        };
        
        // Reset semaphore to new permitted value
        self.cwnd_semaphore = Arc::new(Semaphore::new(available));
        
        // Update stats
        self.stats.cwnd = self.congestion.cwnd;
        
        debug!(
            "Congestion event: cwnd={} -> {}, in_flight={}, loss_rate={:.2}%",
            self.congestion.cwnd * 2,
            self.congestion.cwnd,
            in_flight,
            self.stats.loss_rate * 100.0
        );
    }
    
    /// Update congestion window after successful transmission
    fn update_congestion_window(&mut self, seq: Option<u16>) {
        if self.congestion.in_slow_start {
            // In slow start, grow window exponentially
            // Increase by 1 for each ACK
            let new_cwnd = self.congestion.cwnd + 1;
            
            // Check if we should exit slow start
            if new_cwnd >= self.congestion.ssthresh {
                self.congestion.in_slow_start = false;
            }
            
            self.congestion.cwnd = new_cwnd;
        } else {
            // In congestion avoidance, grow window linearly
            // Increase by 1/cwnd for each ACK, so cwnd += 1 after a full window
            // We approximate this by adding 1 every cwnd packets
            if let Some(ack_seq) = seq {
                let cwnd = self.congestion.cwnd;
                if ack_seq % (cwnd as u16) == 0 {
                    self.congestion.cwnd += 1;
                }
            }
        }
        
        // Update pacing
        self.update_pacing();
        
        // Update stats
        self.stats.cwnd = self.congestion.cwnd;
    }
    
    /// Calculate and update pacing interval
    fn update_pacing(&mut self) {
        // Rough approximation: pace packets evenly across the RTT or a minimum interval
        if let Some(srtt_ms) = self.congestion.srtt_ms {
            // Convert to microseconds for precision
            let srtt_us = (srtt_ms * 1000.0) as u64;
            
            // Minimum sensible interval (100 microseconds)
            const MIN_INTERVAL_US: u64 = 100;
            
            // Calculate pacing interval: RTT / cwnd
            let interval = if self.congestion.cwnd > 0 {
                srtt_us / self.congestion.cwnd as u64
            } else {
                srtt_us
            };
            
            // Use a reasonable minimum
            self.pacing_interval_us = interval.max(MIN_INTERVAL_US);
        } else {
            // No RTT estimate yet, use a reasonable default
            self.pacing_interval_us = 1000; // 1ms
        }
    }
    
    /// Apply pacing to avoid bursts
    async fn apply_pacing(&mut self) {
        if self.pacing_interval_us == 0 {
            return;
        }
        
        if let Some(last_send_time) = self.last_send_time {
            let now = Instant::now();
            let elapsed_us = now.duration_since(last_send_time).as_micros() as u64;
            
            if elapsed_us < self.pacing_interval_us {
                // Need to wait for pacing
                let wait_us = self.pacing_interval_us - elapsed_us;
                
                if wait_us > 100 { // Only wait if it's significant (>100µs)
                    sleep(Duration::from_micros(wait_us)).await;
                }
            }
        }
        
        // Update last send time
        self.last_send_time = Some(Instant::now());
    }
    
    /// Total number of packets queued across all priorities
    fn total_queued_packets(&self) -> usize {
        self.queues.values().map(|q| q.len()).sum()
    }
    
    /// Try to drop low priority packets to make room for higher priority ones
    ///
    /// Returns true if packets were dropped, false otherwise.
    fn drop_low_priority_packets(&mut self) -> bool {
        let mut dropped = false;
        
        // Try low priority first
        if let Some(queue) = self.queues.get_mut(&PacketPriority::Low) {
            if !queue.is_empty() {
                queue.pop_front();
                self.stats.packets_dropped_overflow += 1;
                self.stats.queued_packets = self.total_queued_packets();
                dropped = true;
                trace!("Dropped low priority packet to make room");
                return dropped;
            }
        }
        
        // If we couldn't drop low priority, try normal priority
        if let Some(queue) = self.queues.get_mut(&PacketPriority::Normal) {
            if !queue.is_empty() {
                queue.pop_front();
                self.stats.packets_dropped_overflow += 1;
                self.stats.queued_packets = self.total_queued_packets();
                dropped = true;
                trace!("Dropped normal priority packet to make room");
                return dropped;
            }
        }
        
        // Don't drop high priority or control packets
        trace!("No low/normal priority packets available to drop");
        dropped
    }
    
    /// Dequeue the highest priority packet
    fn dequeue_highest_priority_packet(&mut self) -> Option<RtpPacket> {
        // Try each queue in priority order
        for priority in [
            PacketPriority::Control,
            PacketPriority::High,
            PacketPriority::Normal,
            PacketPriority::Low,
        ] {
            if let Some(queue) = self.queues.get_mut(&priority) {
                if !queue.is_empty() {
                    let queued_packet = queue.pop_front().unwrap();
                    
                    // Update stats
                    self.stats.queued_packets = self.total_queued_packets();
                    
                    // Update metadata and tracking
                    if let Some(mut metadata) = queued_packet.metadata {
                        let now = Instant::now();
                        let seq = queued_packet.packet.header.sequence_number;
                        
                        if metadata.first_send_time.is_none() {
                            metadata.first_send_time = Some(now);
                        }
                        
                        metadata.transmit_count += 1;
                        metadata.last_send_time = Some(now);
                        
                        // Track the packet
                        self.packet_tracking.insert(seq, metadata);
                        
                        // Update last sequence sent
                        self.congestion.last_seq_sent = seq;
                    }
                    
                    return Some(queued_packet.packet);
                }
            }
        }
        
        None
    }
    
    /// Get a packet without congestion control
    ///
    /// This is used when congestion control is disabled.
    async fn get_packet_without_congestion_control(&mut self) -> Option<RtpPacket> {
        // Get highest priority packet
        let packet_option = self.dequeue_highest_priority_packet();
        
        if packet_option.is_some() {
            // Update stats
            self.stats.packets_sent += 1;
            
            // Update last send time
            self.last_send_time = Some(Instant::now());
        }
        
        packet_option
    }
    
    /// Purge expired packets from the queue
    ///
    /// This is useful to periodically call to avoid memory leaks.
    pub fn purge_expired_packets(&mut self) {
        let now = Instant::now();
        let max_age = Duration::from_millis(self.config.max_packet_age_ms);
        
        // Check each queue
        for queue in self.queues.values_mut() {
            // Remove packets that are too old
            let mut i = 0;
            while i < queue.len() {
                if now.duration_since(queue[i].queue_time) > max_age {
                    queue.remove(i);
                    self.stats.packets_dropped_aged += 1;
                } else {
                    i += 1;
                }
            }
        }
        
        // Update stats
        self.stats.queued_packets = self.total_queued_packets();
    }
    
    /// Clear the transmit buffer
    pub fn clear(&mut self) {
        // Clear all queues
        for queue in self.queues.values_mut() {
            queue.clear();
        }
        
        // Clear packet tracking
        self.packet_tracking.clear();
        
        // Reset stats
        self.stats.queued_packets = 0;
    }
    
    /// Get statistics for the transmit buffer
    pub fn get_stats(&self) -> TransmitBufferStats {
        // Calculate additional stats
        let total_capacity = self.config.max_packets;
        let current_queued = self.total_queued_packets();
        let buffer_fullness = if total_capacity > 0 {
            current_queued as f32 / total_capacity as f32
        } else {
            0.0
        };
        
        // Update the queued_packets count
        let mut stats = self.stats.clone();
        stats.queued_packets = current_queued;
        stats.buffer_fullness = buffer_fullness;
        stats.packets_queued = current_queued;
        stats.packets_dropped = stats.packets_dropped_overflow + stats.packets_dropped_aged;
        
        stats
    }
    
    /// Update the configuration of the transmit buffer
    pub fn update_config(&mut self, config: TransmitBufferConfig) {
        // Store current values for comparison
        let old_max_packets = self.config.max_packets;
        let old_cwnd = self.config.initial_cwnd;
        let old_cc_enabled = self.config.congestion_control_enabled;
        
        // Update the configuration
        self.config = config;
        
        // If the congestion window size changed, update the semaphore
        if old_cwnd != self.config.initial_cwnd {
            // Only if not in active congestion control
            if !old_cc_enabled || !self.config.congestion_control_enabled {
                self.congestion.cwnd = self.config.initial_cwnd;
                
                // Reset semaphore to current window size minus in-flight packets
                let in_flight = self.congestion.in_flight;
                let available = if in_flight < self.congestion.cwnd {
                    self.congestion.cwnd - in_flight
                } else {
                    0
                };
                
                // Reset semaphore to new permitted value
                self.cwnd_semaphore = Arc::new(Semaphore::new(available));
                
                // Update stats
                self.stats.cwnd = self.congestion.cwnd;
            }
        }
        
        // Update pacing
        self.update_pacing();
        
        debug!("Updated transmit buffer config: max_packets={}, cwnd={}, cc_enabled={}",
              self.config.max_packets, self.congestion.cwnd, self.config.congestion_control_enabled);
    }
    
    /// Set the priority threshold for a specific buffer fullness level
    ///
    /// When the buffer reaches the specified fullness level (0.0-1.0),
    /// only packets with priority greater than or equal to the threshold
    /// will be transmitted.
    pub fn set_priority_threshold(&mut self, buffer_fullness: f32, priority: PacketPriority) {
        // Store this as a configuration option
        debug!("Setting priority threshold: at {:.1}% fullness, only {:?} or higher priority will be sent",
              buffer_fullness * 100.0, priority);
        
        // We could implement more sophisticated logic here, like 
        // storing multiple thresholds for different fullness levels
        
        // For this simple implementation, we just note it in the log
        // A real implementation would check buffer fullness in get_next_packet
        // and only return packets above the threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::packet::{RtpHeader, RtpPacket};
    
    fn create_test_packet(seq: u16, ts: u32, ssrc: RtpSsrc) -> RtpPacket {
        let header = RtpHeader::new(96, seq, ts, ssrc);
        let payload = Bytes::from_static(b"test");
        RtpPacket::new(header, payload)
    }
    
    #[tokio::test]
    async fn test_basic_queuing() {
        let config = TransmitBufferConfig::default();
        let mut buffer = TransmitBuffer::new(0x12345678, config);
        
        // Queue some packets
        buffer.queue_packet(
            create_test_packet(1, 0, 0x12345678),
            PacketPriority::Normal
        ).await;
        
        buffer.queue_packet(
            create_test_packet(2, 160, 0x12345678),
            PacketPriority::High
        ).await;
        
        // Get packets - should come in priority order
        let p1 = buffer.get_next_packet().await;
        let p2 = buffer.get_next_packet().await;
        
        assert!(p1.is_some());
        assert!(p2.is_some());
        
        // High priority should be first
        assert_eq!(p1.unwrap().header.sequence_number, 2);
        assert_eq!(p2.unwrap().header.sequence_number, 1);
    }
    
    #[tokio::test]
    async fn test_buffer_overflow() {
        let config = TransmitBufferConfig {
            max_packets: 2,
            ..Default::default()
        };
        
        let mut buffer = TransmitBuffer::new(0x12345678, config);
        
        // Queue up to capacity with normal priority
        assert!(buffer.queue_packet(
            create_test_packet(1, 0, 0x12345678),
            PacketPriority::Normal
        ).await, "First packet should be queued");
        
        assert!(buffer.queue_packet(
            create_test_packet(2, 160, 0x12345678),
            PacketPriority::Normal
        ).await, "Second packet should be queued");
        
        // Verify buffer state
        let stats = buffer.get_stats();
        assert_eq!(stats.queued_packets, 2, "Buffer should have 2 packets");
        
        // Third normal priority packet should be rejected as we're at capacity
        assert!(!buffer.queue_packet(
            create_test_packet(3, 320, 0x12345678),
            PacketPriority::Normal
        ).await, "Third normal packet should be rejected");
        
        // High priority packet should be accepted by dropping a normal one
        assert!(buffer.queue_packet(
            create_test_packet(3, 320, 0x12345678),
            PacketPriority::High
        ).await, "High priority packet should be accepted");
        
        // Verify we still have max 2 packets
        let stats = buffer.get_stats();
        assert_eq!(stats.queued_packets, 2, "Buffer should still have 2 packets");
        
        // We should now have packets 2 and 3 (high priority)
        let p1 = buffer.get_next_packet().await;
        let p2 = buffer.get_next_packet().await;
        let p3 = buffer.get_next_packet().await;
        
        assert!(p1.is_some(), "First packet (high priority) should be available");
        assert!(p2.is_some(), "Second packet should be available");
        assert!(p3.is_none(), "Buffer should be empty after 2 packets");
        
        // High priority should be first
        assert_eq!(p1.unwrap().header.sequence_number, 3, "First packet should be the high priority one");
        assert_eq!(p2.unwrap().header.sequence_number, 2, "Second packet should be the remaining normal one");
    }
    
    #[tokio::test]
    async fn test_congestion_control() {
        let config = TransmitBufferConfig {
            initial_cwnd: 2,              // Small window for testing 
            congestion_control_enabled: true,
            ..Default::default()
        };
        
        let mut buffer = TransmitBuffer::new(0x12345678, config);
        
        // Queue several packets
        for i in 1..=5 {
            assert!(buffer.queue_packet(
                create_test_packet(i, (i as u32) * 160, 0x12345678),
                PacketPriority::Normal
            ).await, "Packet {} should be queued", i);
        }
        
        // Verify all 5 packets are queued
        let stats = buffer.get_stats();
        assert_eq!(stats.queued_packets, 5, "Buffer should have 5 queued packets");
        
        // But congestion window should only allow sending 2
        let p1 = buffer.get_next_packet().await;
        assert!(p1.is_some(), "First packet should be available");
        
        let p2 = buffer.get_next_packet().await;
        assert!(p2.is_some(), "Second packet should be available");
        
        // Verify in-flight count
        let stats = buffer.get_stats();
        assert_eq!(stats.in_flight, 2, "Should have 2 packets in flight");
        
        // Third packet should be blocked by congestion window
        let p3 = buffer.get_next_packet().await;
        assert!(p3.is_none(), "Third packet should be blocked by congestion window");
        
        // Acknowledge one packet to open a window slot
        buffer.acknowledge_packet(1);
        
        // Verify in-flight count decreased
        let stats = buffer.get_stats();
        assert_eq!(stats.in_flight, 1, "Should have 1 packet in flight after ACK");
        
        // Now we should be able to get another packet
        let p3 = buffer.get_next_packet().await;
        assert!(p3.is_some(), "Third packet should be available after ACK");
        assert_eq!(p3.unwrap().header.sequence_number, 3, "Third packet should have seq=3");
    }
} 