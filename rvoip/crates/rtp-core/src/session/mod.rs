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
}

/// RTP session for sending and receiving RTP packets
pub struct RtpSession {
    /// Session configuration
    config: RtpSessionConfig,
    
    /// SSRC for this session
    ssrc: RtpSsrc,
    
    /// Transport for sending/receiving packets
    transport: Arc<dyn RtpTransport>,
    
    /// Map of received streams by SSRC
    streams: HashMap<RtpSsrc, RtpStream>,
    
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
    
    /// Whether the session is active
    active: bool,
}

impl RtpSession {
    /// Create a new RTP session
    pub async fn new(config: RtpSessionConfig) -> Result<Self> {
        // Generate SSRC if not provided
        let ssrc = config.ssrc.unwrap_or_else(|| {
            let mut rng = rand::thread_rng();
            rng.gen::<u32>()
        });
        
        // Create transport
        let transport_config = RtpTransportConfig {
            local_rtp_addr: config.local_addr,
            local_rtcp_addr: None, // RTCP on same port for now
            symmetric_rtp: true,
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
        
        let mut session = Self {
            config,
            ssrc,
            transport,
            streams: HashMap::new(),
            scheduler,
            receiver: receiver_rx,
            sender: sender_tx,
            event_tx,
            recv_task: None,
            send_task: None,
            stats: Arc::new(Mutex::new(RtpSessionStats::default())),
            active: false,
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
                // Get destination address
                let dest = if let Some(addr) = last_remote_addr {
                    addr
                } else {
                    // No destination address, can't send
                    warn!("No destination address for RTP packet, dropping");
                    continue;
                };
                
                // Send the packet
                if let Err(e) = send_transport.send_rtp(&packet, dest).await {
                    error!("Failed to send RTP packet: {}", e);
                    
                    // Broadcast error event
                    let _ = event_tx_send.send(RtpSessionEvent::Error(e));
                    continue;
                }
                
                // Update stats
                if let Ok(mut session_stats) = stats_send.lock() {
                    session_stats.packets_sent += 1;
                    session_stats.bytes_sent += packet.size() as u64;
                }
            }
        });
        
        // Start receiving task
        let recv_transport = transport.clone();
        let recv_jitter_buffer = self.config.enable_jitter_buffer;
        let jitter_size = self.config.jitter_buffer_size.unwrap_or(50);
        let max_age_ms = self.config.max_packet_age_ms.unwrap_or(200);
        
        let recv_task = tokio::spawn(async move {
            let mut buffer = vec![0u8; DEFAULT_MAX_PACKET_SIZE];
            
            loop {
                // We'll use recv_from for now and process one packet at a time
                let sock_addr = recv_transport.local_rtp_addr().unwrap_or_else(|_| {
                    "0.0.0.0:0".parse().unwrap()
                });
                
                // This is a placeholder for now - we need to implement the receiving logic
                // in the transport trait properly
                let udp_socket = UdpSocket::bind(sock_addr).await.unwrap();
                
                let result = udp_socket.recv_from(&mut buffer).await;
                match result {
                    Ok((size, addr)) => {
                        if size < RTP_MIN_HEADER_SIZE {
                            warn!("Received packet too small to be RTP: {} bytes", size);
                            continue;
                        }
                        
                        // Parse RTP packet
                        match RtpPacket::parse(&buffer[..size]) {
                            Ok(packet) => {
                                // Update stats
                                if let Ok(mut session_stats) = stats_recv.lock() {
                                    session_stats.packets_received += 1;
                                    session_stats.bytes_received += size as u64;
                                    session_stats.remote_addr = Some(addr);
                                }
                                
                                // Process stream for this SSRC
                                let ssrc = packet.header.ssrc;
                                let mut streams_map = HashMap::new(); // This is a placeholder - we need a proper streams map
                                
                                let stream = streams_map.entry(ssrc).or_insert_with(|| {
                                    if recv_jitter_buffer {
                                        RtpStream::with_jitter_buffer(ssrc, clock_rate, jitter_size, max_age_ms as u64)
                                    } else {
                                        RtpStream::new(ssrc, clock_rate)
                                    }
                                });
                                
                                // Process the packet and get the output packet (if any)
                                if let Some(output_packet) = stream.process_packet(packet) {
                                    // Update jitter stats
                                    if let Ok(mut session_stats) = stats_recv.lock() {
                                        session_stats.jitter_ms = stream.get_jitter_ms();
                                        
                                        // Update other stream-specific stats
                                        let stream_stats = stream.get_stats();
                                        session_stats.packets_lost += stream_stats.packets_lost;
                                        session_stats.packets_duplicated += stream_stats.duplicates;
                                    }
                                    
                                    // Forward the packet to the receiver channel
                                    if let Err(e) = receiver_tx.send(output_packet.clone()).await {
                                        error!("Failed to forward RTP packet to receiver: {}", e);
                                    }
                                    
                                    // Broadcast packet received event
                                    let _ = event_tx_recv.send(RtpSessionEvent::PacketReceived(output_packet));
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse RTP packet: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error receiving RTP packet: {}", e);
                    }
                }
            }
        });
        
        self.recv_task = Some(recv_task);
        self.send_task = Some(send_task);
        self.active = true;
        
        info!("Started RTP session with SSRC={:08x}", self.ssrc);
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
    pub fn set_remote_addr(&mut self, addr: SocketAddr) {
        self.config.remote_addr = Some(addr);
        
        // Update stats with remote address
        if let Ok(mut stats) = self.stats.lock() {
            stats.remote_addr = Some(addr);
        }
    }
    
    /// Get the local address
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.transport.local_rtp_addr()
    }
    
    /// Close the session and clean up resources
    pub async fn close(&mut self) {
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
        
        // Close the transport
        let _ = self.transport.close().await;
        
        self.active = false;
        info!("Closed RTP session with SSRC={:08x}", self.ssrc);
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
} 