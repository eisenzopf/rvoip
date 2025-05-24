//! RTP Session Management
//!
//! This module provides functionality for managing RTP sessions, including
//! configuration, packet sending/receiving, and jitter buffer management.

mod stream;
mod scheduling;

pub use stream::{RtpStream, RtpStreamStats};
pub use scheduling::{RtpScheduler, RtpSchedulerStats};

use bytes::Bytes;
use rand::Rng;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, broadcast};
use tokio::task::JoinHandle;
use tracing::{error, warn, debug, trace, info};

use crate::error::Error;
use crate::packet::{RtpHeader, RtpPacket};
use crate::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
use crate::{Result, RtpSequenceNumber, RtpSsrc, RtpTimestamp, DEFAULT_MAX_PACKET_SIZE};

// Define the constant locally since it's not publicly exported
const RTP_MIN_HEADER_SIZE: usize = 12;

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
        }
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
    
    /// Map of received streams by SSRC
    streams: Arc<Mutex<HashMap<RtpSsrc, RtpStream>>>,
    
    /// Packet scheduler for sending packets
    scheduler: Option<RtpScheduler>,
    
    /// Channel for receiving packets
    receiver: mpsc::Receiver<RtpPacket>,
    
    /// Channel for sending packets
    sender: mpsc::Sender<RtpPacket>,
    
    /// Event broadcaster
    event_tx: broadcast::Sender<RtpSessionEvent>,
    
    /// Receiving task handle
    recv_task: Option<JoinHandle<()>>,
    
    /// Sending task handle
    send_task: Option<JoinHandle<()>>,
    
    /// Session statistics
    stats: Arc<Mutex<RtpSessionStats>>,
    
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
}

impl RtpSession {
    /// Create a new RTP session
    pub async fn new(config: RtpSessionConfig) -> Result<Self> {
        // Generate SSRC if not provided
        let ssrc = config.ssrc.unwrap_or_else(|| {
            let mut rng = rand::thread_rng();
            rng.gen::<u32>()
        });
        
        // Create transport config
        let transport_config = RtpTransportConfig {
            local_rtp_addr: config.local_addr,
            local_rtcp_addr: None, // RTCP on same port for now
            symmetric_rtp: true,
            rtcp_mux: true, // Enable RTCP multiplexing by default
            session_id: Some(format!("rtp-session-{}", ssrc)),
            use_port_allocator: true,
        };
        
        // Create UDP transport
        let transport = Arc::new(UdpRtpTransport::new(transport_config).await?);
        
        // Create channels for internal communication
        let (sender_tx, sender_rx) = mpsc::channel(100);
        let (receiver_tx, receiver_rx) = mpsc::channel(100);
        let (event_tx, _) = broadcast::channel(100);
        
        // Create scheduler if needed
        let scheduler = Some(RtpScheduler::new(
            config.clock_rate,
            rand::thread_rng().gen::<u16>(), // Random starting sequence
            rand::thread_rng().gen::<u32>(), // Random starting timestamp
        ));
        
        // Create RTCP report generator
        let hostname = hostname::get().unwrap_or_else(|_| "unknown".into());
        let hostname_str = hostname.to_string_lossy();
        let cname = format!("{}@{}", std::env::var("USER").unwrap_or_else(|_| "user".to_string()), hostname_str);
        let rtcp_generator = crate::stats::reports::RtcpReportGenerator::new(ssrc, cname);
        
        let mut session = Self {
            config,
            ssrc,
            transport,
            streams: Arc::new(Mutex::new(HashMap::new())),
            scheduler,
            receiver: receiver_rx,
            sender: sender_tx,
            event_tx,
            recv_task: None,
            send_task: None,
            stats: Arc::new(Mutex::new(RtpSessionStats::default())),
            media_sync: None,
            active: false,
            rtcp_generator: Some(rtcp_generator),
            rtcp_task: None,
            bandwidth_bps: 64000, // Default bandwidth: 64 kbps
        };
        
        // Start the session
        session.start(sender_rx, receiver_tx).await?;
        
        Ok(session)
    }
    
    /// Start the session tasks
    async fn start(
        &mut self,
        mut sender_rx: mpsc::Receiver<RtpPacket>,
        receiver_tx: mpsc::Sender<RtpPacket>,
    ) -> Result<()> {
        if self.active {
            return Ok(());
        }
        
        let transport = self.transport.clone();
        let stats_send = self.stats.clone();
        let stats_recv = self.stats.clone();
        let remote_addr = self.config.remote_addr;
        let event_tx_send = self.event_tx.clone();
        let event_tx_recv = self.event_tx.clone();
        let clock_rate = self.config.clock_rate;
        let payload_type = self.config.payload_type;
        let ssrc = self.ssrc;
        let streams_map = self.streams.clone();
        let jitter_buffer_enabled = self.config.enable_jitter_buffer;
        let jitter_size = self.config.jitter_buffer_size.unwrap_or(50);
        let max_age_ms = self.config.max_packet_age_ms.unwrap_or(200);
        
        let media_sync = self.media_sync.clone();
        
        // If we have a remote address, set it on the transport
        if let Some(addr) = remote_addr {
            // Set the remote RTP address on the UDP transport
            if let Some(t) = transport.as_any().downcast_ref::<UdpRtpTransport>() {
                t.set_remote_rtp_addr(addr).await;
            }
        }
        
        // Start the scheduler if available
        if let Some(scheduler) = &mut self.scheduler {
            let sender_tx = self.sender.clone();
            scheduler.set_sender(sender_tx);
            
            // Set appropriate timestamp increment based on packet interval
            let interval_ms = 20; // Default 20ms packet interval
            let samples_per_packet = (clock_rate as f64 * (interval_ms as f64 / 1000.0)) as u32;
            scheduler.set_interval(interval_ms, samples_per_packet);
            
            scheduler.start()?;
        }
        
        // Start sending task
        let send_transport = transport.clone();
        let send_task = tokio::spawn(async move {
            let mut last_remote_addr = remote_addr;
            
            while let Some(packet) = sender_rx.recv().await {
                // Try to get the remote address from different sources
                let dest = if let Some(addr) = last_remote_addr {
                    addr
                } else {
                    // Try to get from transport if available
                    if let Some(t) = send_transport.as_any().downcast_ref::<UdpRtpTransport>() {
                        match t.remote_rtp_addr().await {
                            Some(addr) => addr,
                            None => {
                    // No destination address, can't send
                    warn!("No destination address for RTP packet, dropping");
                    continue;
                            }
                        }
                    } else {
                        // No destination address, can't send
                        warn!("No destination address for RTP packet, dropping");
                        continue;
                    }
                };
                
                // Send the packet
                if let Err(e) = send_transport.send_rtp(&packet, dest).await {
                    error!("Failed to send RTP packet: {}", e);
                    
                    // Broadcast error event
                    let _ = event_tx_send.send(RtpSessionEvent::Error(e));
                    continue;
                }
                
                // Update local copy of remote address
                last_remote_addr = Some(dest);
                
                // Update stats
                if let Ok(mut session_stats) = stats_send.lock() {
                    session_stats.packets_sent += 1;
                    session_stats.bytes_sent += packet.size() as u64;
                }
            }
        });
        
        // Start receiving task
        let recv_transport = transport.clone();
        
        // Subscribe to transport events to handle RTCP packets
        let mut transport_events = recv_transport.subscribe();
        
        let recv_task = tokio::spawn(async move {
            let mut buffer = vec![0u8; DEFAULT_MAX_PACKET_SIZE];
            
            // Create a basic select loop to handle both direct packet reception
            // and events from the transport
            loop {
                tokio::select! {
                    // Handle direct packet reception from the transport
                    result = recv_transport.receive_packet(&mut buffer) => {
                match result {
                    Ok((size, addr)) => {
                                // Process RTP packet
                        if size < RTP_MIN_HEADER_SIZE {
                            warn!("Received packet too small to be RTP: {} bytes", size);
                            continue;
                        }
                        
                        // Parse RTP packet
                        match RtpPacket::parse(&buffer[..size]) {
                            Ok(packet) => {
                                        // Extract the SSRC from the packet
                                        let packet_ssrc = packet.header.ssrc;
                                        debug!("Received packet with SSRC={:08x}, seq={}, timestamp={}",
                                               packet_ssrc, packet.header.sequence_number, packet.header.timestamp);
                                        
                                // Update stats
                                if let Ok(mut session_stats) = stats_recv.lock() {
                                    session_stats.packets_received += 1;
                                    session_stats.bytes_received += size as u64;
                                    session_stats.remote_addr = Some(addr);
                                }
                                
                                        // Get or create the stream for this SSRC
                                        let (is_new_stream, output_packet) = {
                                            let mut streams = match streams_map.lock() {
                                                Ok(streams) => streams,
                                                Err(e) => {
                                                    error!("Failed to lock streams map: {}", e);
                                                    continue;
                                                }
                                            };
                                            
                                            // Check if this is a new stream
                                            let is_new = !streams.contains_key(&packet_ssrc);
                                            if is_new {
                                                debug!("Creating new stream for SSRC={:08x}", packet_ssrc);
                                    } else {
                                                debug!("Found existing stream for SSRC={:08x}", packet_ssrc);
                                            }
                                            
                                            // Get or create the stream for this SSRC
                                            let stream = streams.entry(packet_ssrc).or_insert_with(|| {
                                                info!("New RTP stream detected with SSRC={:08x}", packet_ssrc);
                                                if jitter_buffer_enabled {
                                                    debug!("Creating stream with jitter buffer for SSRC={:08x}", packet_ssrc);
                                                    RtpStream::with_jitter_buffer(
                                                        packet_ssrc, 
                                                        clock_rate, 
                                                        jitter_size, 
                                                        max_age_ms as u64
                                                    )
                                                } else {
                                                    debug!("Creating stream without jitter buffer for SSRC={:08x}", packet_ssrc);
                                                    RtpStream::new(packet_ssrc, clock_rate)
                                                }
                                            });
                                            
                                            // Make sure the stream is initialized with the first sequence number
                                            // This ensures all streams are properly initialized even if their packets
                                            // are being held in the jitter buffer or discarded for some reason
                                            if is_new {
                                                stream.ensure_initialized(packet.header.sequence_number);
                                                debug!("Initialized new stream for SSRC={:08x} with seq={}", 
                                                       packet_ssrc, packet.header.sequence_number);
                                            }
                                
                                // Process the packet and get the output packet (if any)
                                            let output = stream.process_packet(packet.clone());
                                            if output.is_some() {
                                                debug!("Stream {:08x} returned a packet for delivery", packet_ssrc);
                                            } else {
                                                debug!("Stream {:08x} held the packet in jitter buffer", packet_ssrc);
                                            }
                                            
                                            (is_new, output)
                                        };
                                        
                                        // If this is a new stream, emit the NewStreamDetected event
                                        if is_new_stream {
                                            // Broadcast new stream event
                                            debug!("Emitting NewStreamDetected event for SSRC={:08x}", packet_ssrc);
                                            let _ = event_tx_recv.send(RtpSessionEvent::NewStreamDetected {
                                                ssrc: packet_ssrc,
                                            });
                                        }
                                        
                                        // If we have an output packet, forward it
                                        if let Some(output) = output_packet {
                                            // Update jitter stats - get the stream again
                                            if let Ok(streams) = streams_map.lock() {
                                                if let Some(stream) = streams.get(&packet_ssrc) {
                                    if let Ok(mut session_stats) = stats_recv.lock() {
                                        session_stats.jitter_ms = stream.get_jitter_ms();
                                        
                                        // Update other stream-specific stats
                                        let stream_stats = stream.get_stats();
                                                        session_stats.packets_lost = stream_stats.packets_lost;
                                                        session_stats.packets_duplicated = stream_stats.duplicates;
                                                        session_stats.packets_out_of_order = 0; // TODO: Track this
                                                    }
                                                }
                                    }
                                    
                                    // Forward the packet to the receiver channel
                                            if let Err(e) = receiver_tx.send(output.clone()).await {
                                        error!("Failed to forward RTP packet to receiver: {}", e);
                                    }
                                    
                                    // Broadcast packet received event
                                            debug!("Emitting PacketReceived event for SSRC={:08x}", packet_ssrc);
                                            let _ = event_tx_recv.send(RtpSessionEvent::PacketReceived(output));
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse RTP packet: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                                error!("Error receiving from transport: {}", e);
                                continue;
                            }
                        }
                    }
                    
                    // Handle events from the transport (like RTCP packets)
                    event = transport_events.recv() => {
                        match event {
                            Ok(crate::traits::RtpEvent::RtcpReceived { data, source }) => {
                                // Try to parse the RTCP packet
                                if let Ok(rtcp_packet) = crate::packet::rtcp::RtcpPacket::parse(&data) {
                                    // Handle the RTCP packet based on its type
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
                                                
                                                info!("Received RTCP BYE from SSRC={:08x}", source_ssrc);
                                            }
                                        },
                                        crate::packet::rtcp::RtcpPacket::SenderReport(sr) => {
                                            // Process sender report
                                            let report_ssrc = sr.ssrc;
                                            
                                            debug!("Received RTCP SR from SSRC={:08x}", report_ssrc);
                                            
                                            // Update stream statistics if this stream exists
                                            if let Ok(mut streams) = streams_map.lock() {
                                                if let Some(stream) = streams.get_mut(&report_ssrc) {
                                                    // Update the stream's RTCP SR info
                                                    // This will be used for calculating round-trip time
                                                    stream.update_last_sr_info(
                                                        sr.ntp_timestamp.to_u32(),
                                                        std::time::Instant::now(),
                                                    );
                                                    
                                                    debug!("Updated RTCP SR info for stream SSRC={:08x}", report_ssrc);
                                                }
                                            }
                                            
                                            // If media sync is enabled, update it
                                            if let Some(sync) = &media_sync {
                                                if let Ok(mut media_sync) = sync.write() {
                                                    // Update synchronization data
                                                    media_sync.update_from_sr(report_ssrc, sr.ntp_timestamp, sr.rtp_timestamp);
                                                }
                                            }
                                            
                                            // Emit SR event for external processing
                                            let _ = event_tx_recv.send(RtpSessionEvent::RtcpSenderReport { 
                                                ssrc: report_ssrc,
                                                ntp_timestamp: sr.ntp_timestamp,
                                                rtp_timestamp: sr.rtp_timestamp,
                                                packet_count: sr.sender_packet_count,
                                                octet_count: sr.sender_octet_count,
                                                report_blocks: sr.report_blocks,
                                            });
                                        },
                                        crate::packet::rtcp::RtcpPacket::ReceiverReport(rr) => {
                                            // Process receiver report
                                            let report_ssrc = rr.ssrc;
                                            
                                            debug!("Received RTCP RR from SSRC={:08x} with {} report blocks", 
                                                  report_ssrc, rr.report_blocks.len());
                                            
                                            // If there's a report block about our SSRC, process it
                                            for block in &rr.report_blocks {
                                                if block.ssrc == ssrc {
                                                    debug!("Processing report block about our SSRC={:08x}", ssrc);
                                                    
                                                    // Update session stats with packet loss info
                                                    if let Ok(mut stats) = stats_recv.lock() {
                                                        stats.packets_lost = block.cumulative_lost as u64;
                                                        
                                                        // Calculate packet loss percentage
                                                        let fraction_lost = block.fraction_lost as f64 / 256.0;
                                                        debug!("Packet loss: {}% (fraction={})", 
                                                               fraction_lost * 100.0, block.fraction_lost);
                                                    }
                                                }
                                            }
                                            
                                            // Emit RR event for external processing
                                            let _ = event_tx_recv.send(RtpSessionEvent::RtcpReceiverReport { 
                                                ssrc: report_ssrc,
                                                report_blocks: rr.report_blocks,
                                            });
                                        },
                                        // Handle other RTCP packet types as needed
                                        _ => {
                                            // For now, we're just logging other packet types
                                            trace!("Received RTCP packet: {:?}", rtcp_packet);
                                        }
                                    }
                                } else {
                                    warn!("Failed to parse RTCP packet");
                                }
                            }
                            Ok(crate::traits::RtpEvent::MediaReceived { .. }) => {
                                // This is handled by the direct packet reception above
                            }
                            Ok(crate::traits::RtpEvent::Error(e)) => {
                                error!("Transport error: {}", e);
                                let _ = event_tx_recv.send(RtpSessionEvent::Error(e));
                            }
                            Err(e) => {
                                debug!("Transport event channel error: {}", e);
                            }
                        }
                    }
                }
            }
        });
        
        // Start RTCP sending task if we have a remote address and report generator
        if let (Some(remote_addr), Some(mut rtcp_generator)) = (self.config.remote_addr, self.rtcp_generator.take()) {
            let transport = self.transport.clone();
            let ssrc = self.ssrc;
            let event_tx = self.event_tx.clone();
            let stats = self.stats.clone();
            let active_state = Arc::new(tokio::sync::Mutex::new(true));
            let active_state_clone = active_state.clone();
            let bandwidth = self.bandwidth_bps;
            
            // Set bandwidth in the generator
            rtcp_generator.set_bandwidth(bandwidth);
            
            // Start the RTCP task
            let rtcp_task = tokio::spawn(async move {
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
                    if let Ok(session_stats) = stats.lock() {
                        rtcp_generator.update_sent_stats(
                            session_stats.packets_sent as u32,
                            session_stats.bytes_sent as u32
                        );
                        
                        // Log the current stats for debugging
                        debug!("Current stats for RTCP report: packets={}, bytes={}", 
                               session_stats.packets_sent, session_stats.bytes_sent);
                    }
                    
                    // Send an RTCP report regardless of should_send_report logic for this example
                    // We'll send a compound packet with SR and SDES
                    debug!("Sending RTCP report");
                    
                    // Generate sender report
                    let rtp_timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u32;
                        
                    let sr = rtcp_generator.generate_sender_report(rtp_timestamp);
                    let sdes = rtcp_generator.generate_sdes();
                    
                    // Create compound packet
                    let mut compound = crate::packet::rtcp::RtcpCompoundPacket::new_with_sr(sr);
                    compound.add_sdes(sdes);
                    
                    // Send the compound packet
                    if let Ok(data) = compound.serialize() {
                        if let Err(e) = transport.send_rtcp_bytes(&data, remote_addr).await {
                            warn!("Failed to send RTCP compound packet: {}", e);
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
        self.send_task = Some(send_task);
        self.active = true;
        
        info!("Started RTP session with SSRC={:08x}", ssrc);
        Ok(())
    }
    
    /// Send an RTP packet with payload
    pub async fn send_packet(&mut self, timestamp: RtpTimestamp, payload: Bytes, marker: bool) -> Result<()> {
        // Create RTP header
        let mut header = RtpHeader::new(
            self.config.payload_type,
            0, // Sequence number will be set by scheduler
            timestamp,
            self.ssrc,
        );
        
        // Set marker bit if needed
        header.marker = marker;
        
        // Create packet
        let packet = RtpPacket::new(header, payload);
        
        // If using scheduler, schedule the packet
        if let Some(scheduler) = &mut self.scheduler {
            scheduler.schedule_packet(packet)
        } else {
            // Otherwise send directly
            self.sender.send(packet)
                .await
                .map_err(|_| Error::SessionError("Failed to send packet".to_string()))
        }
    }
    
    /// Receive an RTP packet
    pub async fn receive_packet(&mut self) -> Result<RtpPacket> {
        self.receiver.recv()
            .await
            .ok_or_else(|| Error::SessionError("Receiver channel closed".to_string()))
    }
    
    /// Get the session statistics
    pub fn get_stats(&self) -> RtpSessionStats {
        if let Ok(stats) = self.stats.lock() {
            stats.clone()
        } else {
            RtpSessionStats::default()
        }
    }
    
    /// Set the remote address
    pub async fn set_remote_addr(&mut self, addr: SocketAddr) {
        self.config.remote_addr = Some(addr);
        
        // Update stats with remote address
        if let Ok(mut stats) = self.stats.lock() {
            stats.remote_addr = Some(addr);
        }
        
        // Update the transport's remote address
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
                        warn!("Failed to send RTCP BYE: {}", e);
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
        }
        
        // Stop the send task
        if let Some(handle) = self.send_task.take() {
            handle.abort();
        }
        
        // Stop the RTCP task
        if let Some(handle) = self.rtcp_task.take() {
            handle.abort();
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
            let since_epoch = now.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_else(|_| Duration::from_secs(0));
            
            let secs = since_epoch.as_secs();
            let nanos = since_epoch.subsec_nanos();
            
            // Convert to timestamp units (samples)
            let timestamp_secs = secs * (self.config.clock_rate as u64);
            let timestamp_fraction = ((nanos as u64) * (self.config.clock_rate as u64)) / 1_000_000_000;
            
            (timestamp_secs + timestamp_fraction) as u32
        }
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
        let streams = match self.streams.lock() {
            Ok(streams) => streams,
            Err(_) => return None,
        };
        
        streams.get(&ssrc).map(|stream| stream.get_stats())
    }
    
    /// Get a list of all current streams
    pub async fn get_all_streams(&self) -> Vec<RtpStreamStats> {
        let streams = match self.streams.lock() {
            Ok(streams) => streams,
            Err(_) => return Vec::new(),
        };
        
        streams.values().map(|stream| stream.get_stats()).collect()
    }
    
    /// Get the number of active streams
    pub async fn stream_count(&self) -> usize {
        let streams = match self.streams.lock() {
            Ok(streams) => streams,
            Err(_) => return 0,
        };
        
        streams.len()
    }
    
    /// Get a list of all SSRCs known to this session
    /// 
    /// This returns all SSRCs that have been seen, even if their streams
    /// haven't released any packets from their jitter buffers yet.
    pub async fn get_all_ssrcs(&self) -> Vec<RtpSsrc> {
        if let Ok(streams) = self.streams.lock() {
            streams.keys().copied().collect()
        } else {
            Vec::new()
        }
    }
    
    /// Force creation of a stream for a specific SSRC
    /// 
    /// This is useful when we want to ensure a stream exists for an SSRC
    /// even if no packets have been received yet.
    pub async fn create_stream_for_ssrc(&mut self, ssrc: RtpSsrc) -> bool {
        let mut streams = match self.streams.lock() {
            Ok(streams) => streams,
            Err(e) => {
                error!("Failed to lock streams map: {}", e);
                return false;
            }
        };
        
        // Check if this SSRC already exists
        if streams.contains_key(&ssrc) {
            debug!("Stream for SSRC={:08x} already exists", ssrc);
            return false;
        }
        
        // Create the stream
        info!("Manually creating new RTP stream for SSRC={:08x}", ssrc);
        let stream = if self.config.enable_jitter_buffer {
            debug!("Creating stream with jitter buffer for SSRC={:08x}", ssrc);
            RtpStream::with_jitter_buffer(
                ssrc, 
                self.config.clock_rate, 
                self.config.jitter_buffer_size.unwrap_or(50), 
                self.config.max_packet_age_ms.unwrap_or(200) as u64
            )
        } else {
            debug!("Creating stream without jitter buffer for SSRC={:08x}", ssrc);
            RtpStream::new(ssrc, self.config.clock_rate)
        };
        
        // Add the stream
        streams.insert(ssrc, stream);
        
        // Emit the new stream event
        debug!("Emitting NewStreamDetected event for SSRC={:08x}", ssrc);
        let _ = self.event_tx.send(RtpSessionEvent::NewStreamDetected {
            ssrc,
        });
        
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
            None => return Err(Error::SessionError("No remote address configured".to_string())),
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
            Err(e) => {
                Err(Error::SerializationError(format!("Failed to serialize RTCP BYE: {}", e)))
            }
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
            None => return Err(Error::SessionError("No remote address configured".to_string())),
        };
        
        // Get session stats
        let session_stats = if let Ok(stats) = self.stats.lock() {
            stats.clone()
        } else {
            RtpSessionStats::default()
        };
        
        // Create a new SR packet
        let mut sr = crate::packet::rtcp::RtcpSenderReport::new(self.ssrc);
        
        // Set current NTP timestamp
        sr.ntp_timestamp = crate::packet::rtcp::NtpTimestamp::now();
        
        // Set current RTP timestamp (convert from NTP time)
        sr.rtp_timestamp = self.get_timestamp();
        
        // Set packet and octet count from session stats
        sr.sender_packet_count = session_stats.packets_sent as u32;
        sr.sender_octet_count = session_stats.bytes_sent as u32;
        
        // Add report blocks for active streams (remote SSRCs we're receiving from)
        if let Ok(streams) = self.streams.lock() {
            // Add report blocks for up to 31 streams (max allowed by RTCP)
            for (ssrc, stream) in streams.iter().take(31) {
                let stream_stats = stream.get_stats();
                
                // Create a report block for this source
                let mut block = crate::packet::rtcp::RtcpReportBlock::new(*ssrc);
                
                // Set statistics
                let expected_packets = stream_stats.highest_seq - stream_stats.first_seq + 1;
                let (fraction_lost, cumulative_lost) = 
                    block.calculate_packet_loss(expected_packets, stream_stats.received);
                
                block.fraction_lost = fraction_lost;
                block.cumulative_lost = cumulative_lost as u32;
                block.highest_seq = stream_stats.highest_seq;
                block.jitter = stream_stats.jitter;
                
                // TODO: Set last_sr and delay_since_last_sr when we process incoming SRs
                
                // Add the block to the SR
                sr.add_report_block(block);
            }
        }
        
        // **FIX: Update our own MediaSync context with the SR data we're sending**
        // This ensures our own timing data flows into MediaSync for API access
        if let Some(media_sync) = &self.media_sync {
            if let Ok(mut sync) = media_sync.write() {
                sync.update_from_sr(self.ssrc, sr.ntp_timestamp, sr.rtp_timestamp);
                debug!("Updated MediaSync with our own SR: SSRC={:08x}, NTP={:?}, RTP={}", 
                       self.ssrc, sr.ntp_timestamp, sr.rtp_timestamp);
            }
        }
        
        // Create RTCP packet
        let rtcp_packet = crate::packet::rtcp::RtcpPacket::SenderReport(sr);
        
        // Serialize and send
        match rtcp_packet.serialize() {
            Ok(data) => {
                self.transport.send_rtcp_bytes(&data, remote_addr).await
            }
            Err(e) => {
                Err(Error::SerializationError(format!("Failed to serialize RTCP SR: {}", e)))
            }
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
            None => return Err(Error::SessionError("No remote address configured".to_string())),
        };
        
        // Create a new RR packet
        let mut rr = crate::packet::rtcp::RtcpReceiverReport::new(self.ssrc);
        
        // Add report blocks for active streams (remote SSRCs we're receiving from)
        if let Ok(streams) = self.streams.lock() {
            // Add report blocks for up to 31 streams (max allowed by RTCP)
            for (ssrc, stream) in streams.iter().take(31) {
                let stream_stats = stream.get_stats();
                
                // Create a report block for this source
                let mut block = crate::packet::rtcp::RtcpReportBlock::new(*ssrc);
                
                // Set statistics
                let expected_packets = stream_stats.highest_seq - stream_stats.first_seq + 1;
                let (fraction_lost, cumulative_lost) = 
                    block.calculate_packet_loss(expected_packets, stream_stats.received);
                
                block.fraction_lost = fraction_lost;
                block.cumulative_lost = cumulative_lost as u32;
                block.highest_seq = stream_stats.highest_seq;
                block.jitter = stream_stats.jitter;
                
                // TODO: Set last_sr and delay_since_last_sr when we process incoming SRs
                
                // Add the block to the RR
                rr.add_report_block(block);
            }
        }
        
        // Create RTCP packet
        let rtcp_packet = crate::packet::rtcp::RtcpPacket::ReceiverReport(rr);
        
        // Serialize and send
        match rtcp_packet.serialize() {
            Ok(data) => {
                self.transport.send_rtcp_bytes(&data, remote_addr).await
            }
            Err(e) => {
                Err(Error::SerializationError(format!("Failed to serialize RTCP RR: {}", e)))
            }
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
            sender: self.sender.clone(),
            ssrc: self.ssrc,
            payload_type: self.config.payload_type,
            clock_rate: self.config.clock_rate,
        }
    }
    
    /// Get the UDP socket handle from the transport
    /// 
    /// This method is used to access the underlying UDP socket when needed for
    /// other protocols that need to share the same socket (e.g., DTLS).
    pub async fn get_socket_handle(&self) -> Result<Arc<UdpSocket>> {
        // Try to get the socket from the UdpRtpTransport
        if let Some(t) = self.transport.as_any().downcast_ref::<UdpRtpTransport>() {
            // Clone and return the RTP socket using the public method
            let socket = t.get_socket();
            return Ok(socket);
        }
        
        // If we get here, the transport is not UdpRtpTransport
        Err(Error::Transport("Transport is not a UDP transport".to_string()))
    }
}

/// A lightweight sender handle for an RTP session
///
/// This handle can be used to send RTP packets to the session
/// from another thread without having to clone the entire session.
#[derive(Clone)]
pub struct RtpSessionSender {
    /// Channel for sending packets
    sender: mpsc::Sender<RtpPacket>,
    
    /// SSRC for this session
    ssrc: RtpSsrc,
    
    /// Payload type
    payload_type: u8,
    
    /// Clock rate for the payload type
    clock_rate: u32,
}

impl RtpSessionSender {
    /// Send an RTP packet with payload
    pub async fn send_packet(&self, timestamp: RtpTimestamp, payload: Bytes, marker: bool) -> Result<()> {
        // Create RTP header
        let mut header = RtpHeader::new(
            self.payload_type,
            0, // Sequence number will be set by scheduler
            timestamp,
            self.ssrc,
        );
        
        // Set marker bit if needed
        header.marker = marker;
        
        // Create packet
        let packet = RtpPacket::new(header, payload);
        
        // Send the packet
        self.sender.send(packet)
            .await
            .map_err(|_| Error::SessionError("Failed to send packet".to_string()))
    }
} 