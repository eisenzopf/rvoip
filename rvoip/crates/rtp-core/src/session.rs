use bytes::Bytes;
use rand::Rng;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, warn, debug};

use crate::error::Error;
use crate::packet::{RtpHeader, RtpPacket, RTP_MIN_HEADER_SIZE};
use crate::{Result, RtpSequenceNumber, RtpSsrc, RtpTimestamp, DEFAULT_MAX_PACKET_SIZE};

/// Default size for the jitter buffer (packets)
const DEFAULT_JITTER_BUFFER_SIZE: usize = 50;

/// Default maximum age for packets in the jitter buffer (ms)
const DEFAULT_MAX_PACKET_AGE_MS: u32 = 200;

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
}

/// Simple jitter buffer implementation
#[derive(Clone)]
struct JitterBuffer {
    /// Buffer size in packets
    size: usize,
    
    /// Maximum age of packets in milliseconds
    max_age_ms: u32,
    
    /// Packet buffer (sorted by sequence number)
    packets: VecDeque<(RtpSequenceNumber, RtpTimestamp, Bytes)>,
    
    /// Expected next sequence number
    expected_seq: Option<RtpSequenceNumber>,
    
    /// Clock rate for the payload type (to calculate timing)
    clock_rate: u32,
}

impl JitterBuffer {
    /// Create a new jitter buffer
    fn new(size: usize, max_age_ms: u32, clock_rate: u32) -> Self {
        Self {
            size,
            max_age_ms,
            packets: VecDeque::with_capacity(size),
            expected_seq: None,
            clock_rate,
        }
    }
    
    /// Add a packet to the jitter buffer
    fn add_packet(&mut self, header: &RtpHeader, payload: Bytes) -> bool {
        // Initialize expected sequence if not set
        if self.expected_seq.is_none() {
            self.expected_seq = Some(header.sequence_number);
            self.packets.push_back((header.sequence_number, header.timestamp, payload));
            return true;
        }

        // Check for duplicate packet (already in buffer)
        for (seq, _, _) in &self.packets {
            if *seq == header.sequence_number {
                // Duplicate packet, discard
                return false;
            }
        }
        
        // Handle sequence wrapping
        let expected = self.expected_seq.unwrap();
        let seq_diff_val = calculate_seq_diff(header.sequence_number, expected);
        
        // If the packet is very old, discard it
        if seq_diff_val < -(self.size as i32 / 2) {
            return false;
        }
        
        // If the packet is very new (far in the future), it might indicate packet loss
        // In this case, we accept it and adjust our expected sequence
        if seq_diff_val > (self.size as i32 / 2) {
            // Major jump in sequence - possible reset or significant packet loss
            self.expected_seq = Some(header.sequence_number.wrapping_add(1));
            self.packets.clear();
            self.packets.push_back((header.sequence_number, header.timestamp, payload));
            return true;
        }
        
        // Insert packet in sorted order by sequence number
        let mut inserted = false;
        for i in 0..self.packets.len() {
            let curr_seq = self.packets[i].0;
            
            // If found insert position (current packet has higher sequence)
            if calculate_seq_diff(curr_seq, header.sequence_number) > 0 {
                self.packets.insert(i, (header.sequence_number, header.timestamp, payload.clone()));
                inserted = true;
                break;
            }
        }
        
        // If not inserted, add to the end
        if !inserted {
            self.packets.push_back((header.sequence_number, header.timestamp, payload));
        }
        
        // If we've exceeded buffer size, remove the oldest packet
        while self.packets.len() > self.size {
            self.packets.pop_front();
        }
        
        // Update expected sequence if this is the next packet we're expecting
        if header.sequence_number == expected {
            self.expected_seq = Some(header.sequence_number.wrapping_add(1));
        }
        
        true
    }
    
    /// Get the next packet in sequence
    fn get_next_packet(&mut self) -> Option<(RtpSequenceNumber, RtpTimestamp, Bytes)> {
        // If buffer is empty, return None
        if self.packets.is_empty() {
            return None;
        }
        
        // Check if the first packet is ready to be played out
        let expected = self.expected_seq.unwrap_or(self.packets[0].0);
        
        if self.packets[0].0 == expected {
            let packet = self.packets.pop_front().unwrap();
            self.expected_seq = Some(packet.0.wrapping_add(1));
            Some(packet)
        } else {
            None
        }
    }
    
    /// Check if there are packets ready to be played out
    fn has_packets(&self) -> bool {
        !self.packets.is_empty()
    }
    
    /// Clear the buffer
    fn clear(&mut self) {
        self.packets.clear();
        self.expected_seq = None;
    }
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
            jitter_buffer_size: None,
            max_packet_age_ms: None,
            enable_jitter_buffer: true,
        }
    }
}

/// Event from the RTP session
#[derive(Debug)]
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
    
    /// UDP socket for sending/receiving packets
    socket: Arc<UdpSocket>,
    
    /// Event sender channel
    event_tx: mpsc::Sender<RtpSessionEvent>,
    
    /// Event receiver channel
    event_rx: mpsc::Receiver<RtpSessionEvent>,
    
    /// Current sequence number for outgoing packets
    sequence_number: RtpSequenceNumber,
    
    /// Session statistics
    stats: Arc<Mutex<RtpSessionStats>>,
    
    /// Jitter buffer (if enabled)
    jitter_buffer: Option<Arc<Mutex<JitterBuffer>>>,
    
    /// Receiver task handle
    receiver_handle: Option<JoinHandle<()>>,
}

impl RtpSession {
    /// Create a new RTP session
    pub async fn new(config: RtpSessionConfig) -> Result<Self> {
        // Generate SSRC if not provided
        let _ssrc = config.ssrc.unwrap_or_else(|| rand::thread_rng().gen());
        
        // Generate initial sequence number
        let sequence_number = rand::thread_rng().gen();
        
        // Create UDP socket
        let socket = UdpSocket::bind(config.local_addr).await
            .map_err(|e| Error::IoError(e))?;
        
        // Connect socket to remote address if provided
        // This will help validate that packets are sent/received from the expected address
        if let Some(remote_addr) = config.remote_addr {
            socket.connect(remote_addr).await
                .map_err(|e| Error::IoError(e))?;
            
            debug!("Socket connected to remote address: {}", remote_addr);
        }
        
        let socket = Arc::new(socket);
        
        // Create event channels with sufficient capacity to avoid dropping packets
        let (event_tx, event_rx) = mpsc::channel(1000);
        
        // Create jitter buffer if enabled
        let jitter_buffer = if config.enable_jitter_buffer {
            Some(Arc::new(Mutex::new(JitterBuffer::new(
                config.jitter_buffer_size.unwrap_or(DEFAULT_JITTER_BUFFER_SIZE),
                config.max_packet_age_ms.unwrap_or(DEFAULT_MAX_PACKET_AGE_MS),
                config.clock_rate,
            ))))
        } else {
            None
        };
        
        let mut session = Self {
            config,
            socket,
            event_tx,
            event_rx,
            sequence_number,
            stats: Arc::new(Mutex::new(RtpSessionStats::default())),
            jitter_buffer,
            receiver_handle: None,
        };
        
        // Start receiver task
        session.start_receiver();
        
        Ok(session)
    }
    
    /// Send an RTP packet with payload
    pub async fn send_packet(&mut self, timestamp: RtpTimestamp, payload: Bytes, marker: bool) -> Result<()> {
        // Create RTP header
        let mut header = RtpHeader::new(
            self.config.payload_type,
            self.sequence_number,
            timestamp,
            self.config.ssrc.unwrap_or(0),
        );
        header.marker = marker;
        
        // Create RTP packet
        let packet = RtpPacket::new(header, payload);
        
        // Serialize packet
        let data = packet.serialize()?;
        
        // Send packet using the appropriate method (connected or unconnected socket)
        if self.socket.peer_addr().is_ok() {
            // Socket is connected, use send()
            self.socket.send(&data).await
                .map_err(|e| Error::IoError(e))?;
        } else if let Some(remote_addr) = self.config.remote_addr {
            // Socket is not connected, use send_to() with the remote address
            self.socket.send_to(&data, remote_addr).await
                .map_err(|e| Error::IoError(e))?;
        } else {
            return Err(Error::SessionError("Remote address not set".to_string()));
        }
        
        // Update sequence number
        self.sequence_number = self.sequence_number.wrapping_add(1);
        
        // Update stats
        if let Ok(mut stats) = self.stats.lock() {
            stats.packets_sent += 1;
            stats.bytes_sent += data.len() as u64;
        }
        
        Ok(())
    }
    
    /// Receive an RTP packet (blocks until a packet is available)
    pub async fn receive_packet(&mut self) -> Result<RtpPacket> {
        loop {
            match self.event_rx.recv().await {
                Some(RtpSessionEvent::PacketReceived(packet)) => {
                    return Ok(packet);
                }
                Some(RtpSessionEvent::Error(e)) => {
                    return Err(e);
                }
                None => {
                    return Err(Error::SessionError("Event channel closed".to_string()));
                }
            }
        }
    }
    
    /// Get the session statistics
    pub fn get_stats(&self) -> RtpSessionStats {
        self.stats.lock().unwrap().clone()
    }
    
    /// Set the remote address
    pub fn set_remote_addr(&mut self, addr: SocketAddr) {
        self.config.remote_addr = Some(addr);
    }
    
    /// Get the local address
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket.local_addr().map_err(|e| Error::IoError(e))
    }
    
    /// Start the receiver task
    fn start_receiver(&mut self) {
        let socket = self.socket.clone();
        let event_tx = self.event_tx.clone();
        let stats = self.stats.clone();
        let jitter_buffer = self.jitter_buffer.clone();
        let payload_type = self.config.payload_type;
        
        let handle = tokio::spawn(async move {
            let mut buf = vec![0u8; DEFAULT_MAX_PACKET_SIZE];
            
            loop {
                let result = if socket.peer_addr().is_ok() {
                    // Socket is connected, use recv()
                    socket.recv(&mut buf).await.map(|len| (len, socket.peer_addr().unwrap()))
                } else {
                    // Socket is not connected, use recv_from()
                    socket.recv_from(&mut buf).await
                };
                
                match result {
                    Ok((len, addr)) => {
                        debug!("Received {} bytes from {}", len, addr);
                        
                        // Make sure we have enough data for a valid RTP packet
                        if len < RTP_MIN_HEADER_SIZE {
                            error!("Received data too small for RTP packet: {} bytes, min header size is {}", 
                                  len, RTP_MIN_HEADER_SIZE);
                            
                            if len > 0 {
                                let prefix = if len > 16 { 16 } else { len };
                                let hex_data = hex_dump(&buf[..prefix]);
                                error!("Invalid packet data prefix: {}", hex_data);
                            }
                            
                            continue;
                        }
                    
                        // Ensure we only use the actual received bytes
                        let packet_data = &buf[..len];
                        
                        // Process received packet
                        match RtpPacket::parse(packet_data) {
                            Ok(packet) => {
                                debug!("Successfully parsed RTP packet: seq={}, ts={}, pt={}, len={}",
                                      packet.header.sequence_number, 
                                      packet.header.timestamp,
                                      packet.header.payload_type,
                                      packet.payload.len());
                                
                                // Update stats
                                if let Ok(mut stats) = stats.lock() {
                                    stats.packets_received += 1;
                                    stats.bytes_received += len as u64;
                                }
                                
                                // Check if packet payload type matches expected
                                if packet.header.payload_type != payload_type {
                                    warn!("Received packet with unexpected payload type: {}, expected: {}", 
                                          packet.header.payload_type, payload_type);
                                    continue;
                                }
                                
                                // Add to jitter buffer if enabled
                                if let Some(jitter_buffer) = &jitter_buffer {
                                    if let Ok(mut buffer) = jitter_buffer.lock() {
                                        let added = buffer.add_packet(&packet.header, packet.payload.clone());
                                        
                                        if !added {
                                            // Packet discarded by jitter buffer
                                            if let Ok(mut stats) = stats.lock() {
                                                stats.packets_discarded_by_jitter += 1;
                                            }
                                            continue;
                                        }
                                    }
                                }
                                
                                // Forward packet to event handler
                                if event_tx.send(RtpSessionEvent::PacketReceived(packet)).await.is_err() {
                                    error!("Failed to send RTP packet event, channel closed");
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Failed to parse RTP packet ({} bytes): {}", len, e);
                                
                                // Log more details about the packet for debugging
                                if len > 0 {
                                    let prefix = if len > 16 { 16 } else { len };
                                    let hex_data = hex_dump(&packet_data[..prefix]);
                                    error!("Packet data prefix: {}", hex_data);
                                }
                                
                                if event_tx.send(RtpSessionEvent::Error(e)).await.is_err() {
                                    error!("Failed to send RTP error event, channel closed");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to receive from socket: {}", e);
                        if event_tx.send(RtpSessionEvent::Error(Error::IoError(e))).await.is_err() {
                            error!("Failed to send RTP error event, channel closed");
                            break;
                        }
                    }
                }
            }
        });
        
        self.receiver_handle = Some(handle);
    }
    
    /// Close the session and clean up resources
    pub async fn close(&mut self) {
        // Abort receiver task if running
        if let Some(handle) = self.receiver_handle.take() {
            handle.abort();
        }
    }
    
    /// Get the current timestamp
    pub fn get_timestamp(&self) -> RtpTimestamp {
        // Current timestamp is based on the sequence number, samples per packet, and clock rate
        // For simplicity, return the current value plus one packet's worth of samples
        let base_timestamp = self.config.clock_rate / 50; // 20ms worth of samples
        base_timestamp
    }
    
    /// Get the SSRC of this session
    pub fn get_ssrc(&self) -> RtpSsrc {
        self.config.ssrc.unwrap_or(0)
    }
    
    /// Get the receiver channel for incoming packets
    pub fn get_receiver_channel(&self) -> mpsc::Receiver<RtpPacket> {
        // Create a new channel
        let (tx, rx) = mpsc::channel(100);
        
        // Clone the event receiver
        let mut event_rx = self.event_rx.clone();
        
        // Forward events to the new channel
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                match event {
                    RtpSessionEvent::PacketReceived(packet) => {
                        if tx.send(packet).await.is_err() {
                            break;
                        }
                    },
                    _ => {
                        // Ignore other events
                    }
                }
            }
        });
        
        rx
    }
}

/// Calculate difference between two sequence numbers, accounting for wrapping
fn calculate_seq_diff(a: u16, b: u16) -> i32 {
    let diff = (a as i32) - (b as i32);
    
    if diff > 32767 {
        diff - 65536
    } else if diff < -32768 {
        diff + 65536
    } else {
        diff
    }
}

/// Calculate difference between two timestamps, accounting for wrapping
fn timestamp_diff(a: u32, b: u32) -> u32 {
    a.wrapping_sub(b)
}

/// Utility function to generate a hex dump of data for debugging
fn hex_dump(data: &[u8]) -> String {
    let mut output = String::new();
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            output.push(' ');
        }
        output.push_str(&format!("{:02x}", byte));
    }
    output
} 