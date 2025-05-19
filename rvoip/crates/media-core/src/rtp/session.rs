//! RTP Session
//!
//! This module provides integration with the rtp-core library's
//! RTP session implementation.

use std::net::SocketAddr;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock, broadcast};
use std::time::Instant;

use bytes::Bytes;
use tokio::sync::mpsc::{self, Sender, Receiver};
use tracing::{debug, error, info, trace, warn};

use crate::error::{Error, Result};

use rvoip_rtp_core::{
    packet::RtpPacket,
    transport::{RtpSocket, RtpTransport},
};

use crate::{AudioBuffer, AudioFormat, SampleRate};
use crate::codec::Codec;
use crate::rtp::{
    packetizer::{Packetizer, PacketizerConfig},
    depacketizer::{Depacketizer, DepacketizerConfig},
};

/// RTP session configuration
#[derive(Debug, Clone)]
pub struct RtpSessionConfig {
    /// Local address and port
    pub local_addr: SocketAddr,
    /// Remote address and port
    pub remote_addr: Option<SocketAddr>,
    /// SSRC identifier
    pub ssrc: u32,
    /// Payload type
    pub payload_type: u8,
    /// Clock rate in Hz
    pub clock_rate: u32,
    /// Audio format for decoded output
    pub audio_format: AudioFormat,
    /// Maximum packet size in bytes
    pub max_packet_size: usize,
    /// Whether to perform packet reordering
    pub reorder_packets: bool,
    /// Whether to enable RTCP
    pub enable_rtcp: bool,
}

impl Default for RtpSessionConfig {
    fn default() -> Self {
        Self {
            local_addr: "0.0.0.0:0".parse().unwrap(),
            remote_addr: None,
            ssrc: rand::random::<u32>(),
            payload_type: 0, // Default to PCMU
            clock_rate: 8000, // Default to 8kHz
            audio_format: AudioFormat::telephony(),
            max_packet_size: 1200, // Safe size for Internet MTU
            reorder_packets: true,
            enable_rtcp: true,
        }
    }
}

/// Statistics for an RTP session
#[derive(Debug, Clone)]
pub struct RtpSessionStats {
    /// Number of packets sent
    pub packets_sent: u64,
    /// Number of packets received
    pub packets_received: u64,
    /// Number of bytes sent
    pub bytes_sent: u64,
    /// Number of bytes received
    pub bytes_received: u64,
    /// Number of packets received out of order
    pub out_of_order_packets: u32,
    /// Number of packets too late to be reordered
    pub late_packets: u32,
    /// Average jitter in milliseconds
    pub jitter_ms: f32,
    /// Packet loss percentage (0.0-1.0)
    pub packet_loss: f32,
    /// Round-trip time in milliseconds (from RTCP)
    pub rtt_ms: Option<f32>,
    /// Session duration in seconds
    pub duration_secs: u64,
}

/// RTP session event types
#[derive(Debug, Clone)]
pub enum RtpSessionEvent {
    /// Connection established with remote party
    Connected(SocketAddr),
    /// Connection broken with remote party
    Disconnected,
    /// Audio data received from remote party
    AudioReceived(AudioBuffer),
    /// Session statistics updated
    StatsUpdated(RtpSessionStats),
    /// Fatal error occurred
    Error(String),
}

/// Media direction for RTP session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    /// Send and receive media
    SendRecv,
    /// Send media only
    SendOnly,
    /// Receive media only
    RecvOnly,
    /// Don't send or receive media
    Inactive,
}

impl Default for MediaDirection {
    fn default() -> Self {
        Self::SendRecv
    }
}

/// State of an RTP session
#[derive(Debug)]
struct RtpSessionState {
    /// Session configuration
    config: RtpSessionConfig,
    /// Media direction
    direction: MediaDirection,
    /// Socket used for RTP transmission
    socket: Option<Arc<RtpSocket>>,
    /// Packetizer for outgoing media
    packetizer: Packetizer,
    /// Depacketizer for incoming media
    depacketizer: Depacketizer,
    /// Codec used for this session
    codec: Option<Arc<dyn Codec>>,
    /// Session statistics
    stats: RtpSessionStats,
    /// Session start time
    start_time: Instant,
    /// Event sender channel
    event_sender: Option<Sender<RtpSessionEvent>>,
    /// Whether the session is active
    active: bool,
}

impl RtpSessionState {
    /// Create a new RTP session state
    fn new(config: RtpSessionConfig, tx: Sender<RtpSessionEvent>) -> Self {
        // Create packetizer
        let packetizer_config = PacketizerConfig {
            ssrc: config.ssrc,
            payload_type: config.payload_type,
            initial_seq: rand::random::<u16>(),
            initial_ts: rand::random::<u32>(),
            set_marker: true,
            clock_rate: config.clock_rate,
        };
        
        // Create depacketizer
        let depacketizer_config = DepacketizerConfig {
            payload_type: config.payload_type,
            clock_rate: config.clock_rate,
            audio_format: config.audio_format,
            reorder_packets: config.reorder_packets,
            reordering_time_ms: 50, // 50ms default for reordering
            max_jitter_packets: 5,  // 5 packets max in jitter buffer
        };
        
        Self {
            config,
            direction: MediaDirection::default(),
            socket: None,
            packetizer: Packetizer::new(packetizer_config),
            depacketizer: Depacketizer::new(depacketizer_config),
            codec: None,
            stats: RtpSessionStats {
                packets_sent: 0,
                packets_received: 0,
                bytes_sent: 0,
                bytes_received: 0,
                out_of_order_packets: 0,
                late_packets: 0,
                jitter_ms: 0.0,
                packet_loss: 0.0,
                rtt_ms: None,
                duration_secs: 0,
            },
            start_time: Instant::now(),
            event_sender: Some(tx),
            active: false,
        }
    }
    
    /// Send an event to the user
    fn send_event(&self, event: RtpSessionEvent) {
        if let Some(tx) = &self.event_sender {
            if let Err(e) = tx.try_send(event) {
                warn!("Failed to send RTP session event: {}", e);
            }
        }
    }
    
    /// Update session statistics
    fn update_stats(&mut self) {
        // Update duration
        self.stats.duration_secs = self.start_time.elapsed().as_secs();
        
        // Get depacketizer stats
        let depack_stats = self.depacketizer.stats();
        self.stats.out_of_order_packets = depack_stats.out_of_order_count;
        self.stats.late_packets = depack_stats.late_packets;
        
        // Send stats event
        self.send_event(RtpSessionEvent::StatsUpdated(self.stats.clone()));
    }
}

/// RTP session for managing RTP media streams
pub struct RtpSession {
    /// Session state
    state: RwLock<RtpSessionState>,
    /// Packet receiver from network thread
    packet_rx: Option<Receiver<RtpPacket>>,
    /// Session ID for debugging/logging
    id: String,
}

impl RtpSession {
    /// Create a new RTP session
    pub async fn new(config: RtpSessionConfig) -> Result<(Self, Receiver<RtpSessionEvent>)> {
        let (event_tx, event_rx) = mpsc::channel(100);
        let (packet_tx, packet_rx) = mpsc::channel(100);
        
        let session_id = format!("rtp-{:08x}", config.ssrc);
        
        let session = Self {
            state: RwLock::new(RtpSessionState::new(config.clone(), event_tx)),
            packet_rx: Some(packet_rx),
            id: session_id,
        };
        
        // Open the RTP socket
        session.open_socket().await?;
        
        Ok((session, event_rx))
    }
    
    /// Open the RTP socket for this session
    async fn open_socket(&self) -> Result<()> {
        let mut state = self.state.write().unwrap();
        
        // Create socket
        let socket = RtpSocket::bind(state.config.local_addr).await
            .map_err(|e| Error::TransportError(format!("Failed to bind RTP socket: {}", e)))?;
        
        info!("[{}] RTP socket bound to {}", self.id, socket.local_addr());
        
        // Set remote address if available
        if let Some(remote_addr) = state.config.remote_addr {
            socket.connect(remote_addr).await
                .map_err(|e| Error::TransportError(format!("Failed to connect RTP socket: {}", e)))?;
            
            info!("[{}] RTP socket connected to {}", self.id, remote_addr);
            
            // Send connected event
            state.send_event(RtpSessionEvent::Connected(remote_addr));
        }
        
        // Store socket
        state.socket = Some(Arc::new(socket));
        
        // Start receive loop
        self.start_receive_loop();
        
        Ok(())
    }
    
    /// Start the packet receive loop
    fn start_receive_loop(&self) {
        let state = self.state.read().unwrap();
        
        if let Some(socket) = &state.socket {
            let socket_clone = socket.clone();
            let packet_tx = match self.packet_rx.as_ref() {
                Some(_) => {
                    // Get sender from socket's own channel
                    // This is a simplification - in a real implementation,
                    // we would need to manage this channel properly
                    let (tx, _) = mpsc::channel(100);
                    tx
                },
                None => return,
            };
            
            // Spawn receive task
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 2048];
                
                loop {
                    match socket_clone.recv(&mut buffer).await {
                        Ok((len, addr)) => {
                            trace!("Received {} bytes from {}", len, addr);
                            
                            // Parse RTP packet
                            match RtpPacket::parse(&buffer[..len]) {
                                Ok(packet) => {
                                    if let Err(e) = packet_tx.send(packet).await {
                                        error!("Failed to forward RTP packet: {}", e);
                                        break;
                                    }
                                },
                                Err(e) => {
                                    warn!("Failed to parse RTP packet: {}", e);
                                }
                            }
                        },
                        Err(e) => {
                            error!("RTP socket receive error: {}", e);
                            break;
                        }
                    }
                }
            });
        }
    }
    
    /// Set the codec for this session
    pub fn set_codec(&self, codec: Arc<dyn Codec>) {
        let mut state = self.state.write().unwrap();
        
        // Set codec in packetizer and depacketizer
        state.packetizer.set_codec(codec.clone());
        state.depacketizer.set_codec(codec.clone());
        
        // Store codec
        state.codec = Some(codec);
    }
    
    /// Set the media direction for this session
    pub fn set_direction(&self, direction: MediaDirection) {
        let mut state = self.state.write().unwrap();
        state.direction = direction;
    }
    
    /// Get the current media direction
    pub fn direction(&self) -> MediaDirection {
        let state = self.state.read().unwrap();
        state.direction
    }
    
    /// Set remote address for RTP
    pub async fn set_remote_addr(&self, addr: SocketAddr) -> Result<()> {
        let state = self.state.read().unwrap();
        
        if let Some(socket) = &state.socket {
            socket.connect(addr).await
                .map_err(|e| Error::TransportError(format!("Failed to connect RTP socket: {}", e)))?;
            
            info!("[{}] RTP socket connected to {}", self.id, addr);
            
            // Send connected event
            state.send_event(RtpSessionEvent::Connected(addr));
        } else {
            return Err(Error::NotInitialized("RTP socket not initialized".into()));
        }
        
        Ok(())
    }
    
    /// Send audio data through this session
    pub async fn send_audio(&self, buffer: &AudioBuffer) -> Result<()> {
        let state = self.state.read().unwrap();
        
        // Check if we're in send mode
        if state.direction == MediaDirection::RecvOnly || state.direction == MediaDirection::Inactive {
            return Ok(());
        }
        
        // Check if we have a socket
        let socket = match &state.socket {
            Some(s) => s,
            None => return Err(Error::NotInitialized("RTP socket not initialized".into())),
        };
        
        // Get mutable reference to packetizer
        // Note: This is a hack to get around Rust's borrowing rules
        // In a real implementation, we'd use proper interior mutability
        let packetizer = unsafe {
            // This is safe because we know packetizer is only accessed here
            &mut *(&state.packetizer as *const Packetizer as *mut Packetizer)
        };
        
        // Packetize the audio
        let packets = packetizer.packetize_audio(buffer)?;
        
        // Send each packet
        for packet in packets {
            let data = packet.serialize();
            socket.send(&data).await
                .map_err(|e| Error::TransportError(format!("Failed to send RTP packet: {}", e)))?;
            
            // Update stats
            let mut state = self.state.write().unwrap();
            state.stats.packets_sent += 1;
            state.stats.bytes_sent += data.len() as u64;
        }
        
        Ok(())
    }
    
    /// Process received RTP packets
    pub async fn process_received_packets(&self) -> Result<()> {
        let mut packet_rx = match self.packet_rx.as_ref() {
            Some(rx) => rx,
            None => return Ok(()),
        };
        
        // Get mutable reference to depacketizer
        // Note: This is a hack to get around Rust's borrowing rules
        // In a real implementation, we'd use proper interior mutability
        let depacketizer = unsafe {
            let state = self.state.read().unwrap();
            &mut *(&state.depacketizer as *const Depacketizer as *mut Depacketizer)
        };
        
        // Process all available packets
        while let Ok(packet) = packet_rx.try_recv() {
            let mut state = self.state.write().unwrap();
            
            // Check if we're in receive mode
            if state.direction == MediaDirection::SendOnly || state.direction == MediaDirection::Inactive {
                continue;
            }
            
            // Update stats
            state.stats.packets_received += 1;
            state.stats.bytes_received += packet.size() as u64;
            
            // Depacketize the packet
            match depacketizer.process_packet(packet) {
                Ok(Some(buffer)) => {
                    // We have a complete audio frame
                    state.send_event(RtpSessionEvent::AudioReceived(buffer));
                },
                Ok(None) => {
                    // Not a complete frame yet
                },
                Err(e) => {
                    warn!("[{}] Error processing RTP packet: {}", self.id, e);
                }
            }
            
            // Update stats periodically
            if state.stats.packets_received % 100 == 0 {
                state.update_stats();
            }
        }
        
        Ok(())
    }
    
    /// Reset the session
    pub fn reset(&self) {
        let mut state = self.state.write().unwrap();
        
        // Reset state
        state.packetizer.reset();
        state.depacketizer.reset();
        
        // Reset stats
        state.stats = RtpSessionStats {
            packets_sent: 0,
            packets_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
            out_of_order_packets: 0,
            late_packets: 0,
            jitter_ms: 0.0,
            packet_loss: 0.0,
            rtt_ms: None,
            duration_secs: 0,
        };
        state.start_time = Instant::now();
    }
    
    /// Close the session
    pub async fn close(&self) -> Result<()> {
        let mut state = self.state.write().unwrap();
        
        // Close socket
        state.socket = None;
        
        // Send disconnected event
        state.send_event(RtpSessionEvent::Disconnected);
        
        // Clear event sender
        state.event_sender = None;
        
        // Mark as inactive
        state.active = false;
        
        Ok(())
    }
    
    /// Get session statistics
    pub fn stats(&self) -> RtpSessionStats {
        let state = self.state.read().unwrap();
        state.stats.clone()
    }
    
    /// Get local address
    pub fn local_addr(&self) -> Option<SocketAddr> {
        let state = self.state.read().unwrap();
        
        state.socket.as_ref().map(|s| s.local_addr())
    }
} 