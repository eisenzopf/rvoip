use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, RwLock, Mutex};
use tokio::time::Instant;
use bytes::Bytes;
use tracing::{debug, error, info, warn};

use rvoip_rtp_core::{RtpPacket, RtpHeader, RtpTimestamp};
use crate::config::CodecType;
use crate::error::{Error, Result};

/// RTP session for sending and receiving media
pub struct RtpSession {
    /// Local RTP socket address
    local_addr: SocketAddr,

    /// Remote RTP destination address
    remote_addr: SocketAddr,

    /// UDP socket for RTP
    socket: Arc<UdpSocket>,

    /// Sequence number for outgoing RTP packets
    sequence: u16,

    /// Timestamp for outgoing RTP packets
    timestamp: RtpTimestamp,

    /// SSRC for this session
    ssrc: u32,

    /// Codec being used
    codec: CodecType,

    /// Payload type
    payload_type: u8,

    /// Sampling rate in Hz
    sampling_rate: u32,

    /// Samples per packet
    samples_per_packet: u32,

    /// Marker bit for first packet after silence
    marker: bool,

    /// Buffer for receiving packets
    receive_buffer: Vec<u8>,

    /// Task handle for the receive loop
    receive_task: Option<tokio::task::JoinHandle<()>>,

    /// Channel for received packets
    packet_rx: Option<mpsc::Receiver<RtpPacket>>,

    /// Sender for the packet channel
    packet_tx: Option<mpsc::Sender<RtpPacket>>,

    /// Is the session active
    is_active: Arc<RwLock<bool>>,

    /// Last time a packet was sent
    last_send_time: Arc<Mutex<Option<Instant>>>,

    /// Last time a packet was received
    last_receive_time: Arc<Mutex<Option<Instant>>>,
}

impl RtpSession {
    /// Create a new RTP session
    pub async fn new(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        codec: CodecType,
    ) -> Result<Self> {
        // Create UDP socket
        let socket = UdpSocket::bind(local_addr).await
            .map_err(|e| Error::Network(e))?;
        
        // Connect the socket to the remote address
        socket.connect(remote_addr).await
            .map_err(|e| Error::Network(e))?;
        
        debug!("RTP session created - local: {}, remote: {}", local_addr, remote_addr);
        
        // Create random SSRC
        let ssrc = rand::random::<u32>();
        
        // Create channel for received packets
        let (packet_tx, packet_rx) = mpsc::channel(32);
        
        // Set up codec-specific parameters
        let (payload_type, sampling_rate, samples_per_packet) = match codec {
            CodecType::PCMU => (0, 8000, 160),  // G.711 μ-law, 20ms at 8kHz
            CodecType::PCMA => (8, 8000, 160),  // G.711 A-law, 20ms at 8kHz
            CodecType::G722 => (9, 16000, 320), // G.722, 20ms at 16kHz
            CodecType::G729 => (18, 8000, 160), // G.729, 20ms at 8kHz
            CodecType::OPUS => (111, 48000, 960), // Opus, 20ms at 48kHz
        };
        
        Ok(Self {
            local_addr,
            remote_addr,
            socket: Arc::new(socket),
            sequence: rand::random::<u16>(),
            timestamp: rand::random::<u32>(),
            ssrc,
            codec,
            payload_type,
            sampling_rate,
            samples_per_packet,
            marker: true, // First packet has marker bit set
            receive_buffer: vec![0u8; 2048],
            receive_task: None,
            packet_rx: Some(packet_rx),
            packet_tx: Some(packet_tx),
            is_active: Arc::new(RwLock::new(false)),
            last_send_time: Arc::new(Mutex::new(None)),
            last_receive_time: Arc::new(Mutex::new(None)),
        })
    }
    
    /// Start the RTP session
    pub async fn start(&mut self) -> Result<()> {
        if *self.is_active.read().await {
            return Ok(());
        }
        
        // Set active flag
        *self.is_active.write().await = true;
        
        // Start receive task
        let socket = self.socket.clone();
        let packet_tx = self.packet_tx.take()
            .ok_or_else(|| Error::Media("RTP session already started".into()))?;
        let is_active = self.is_active.clone();
        let last_receive_time = self.last_receive_time.clone();
        let mut receive_buffer = vec![0u8; 2048];
        
        let receive_task = tokio::spawn(async move {
            debug!("RTP receive task started");
            
            while *is_active.read().await {
                // Read packet with timeout
                let result = tokio::time::timeout(
                    Duration::from_secs(1),
                    socket.recv(&mut receive_buffer)
                ).await;
                
                match result {
                    Ok(Ok(len)) => {
                        // Parse RTP packet
                        match RtpPacket::parse(&receive_buffer[..len]) {
                            Ok(packet) => {
                                // Update last receive time
                                *last_receive_time.lock().await = Some(Instant::now());
                                
                                // Send packet to channel
                                if packet_tx.send(packet).await.is_err() {
                                    error!("Failed to send RTP packet to channel");
                                    break;
                                }
                            },
                            Err(e) => {
                                warn!("Failed to parse RTP packet: {}", e);
                            }
                        }
                    },
                    Ok(Err(e)) => {
                        error!("Error receiving RTP packet: {}", e);
                        break;
                    },
                    Err(_) => {
                        // Timeout, continue
                    }
                }
            }
            
            debug!("RTP receive task ended");
        });
        
        self.receive_task = Some(receive_task);
        
        Ok(())
    }
    
    /// Stop the RTP session
    pub async fn stop(&mut self) -> Result<()> {
        if !*self.is_active.read().await {
            return Ok(());
        }
        
        // Set inactive flag
        *self.is_active.write().await = false;
        
        // Wait for receive task to end
        if let Some(task) = self.receive_task.take() {
            task.abort();
            let _ = tokio::time::timeout(Duration::from_millis(100), task).await;
        }
        
        // Reset packet channel
        let (packet_tx, packet_rx) = mpsc::channel(32);
        self.packet_tx = Some(packet_tx);
        self.packet_rx = Some(packet_rx);
        
        Ok(())
    }
    
    /// Send an RTP packet
    pub async fn send_packet(&mut self, payload: Bytes) -> Result<()> {
        // Create RTP header
        let header = RtpHeader {
            version: 2,
            padding: false,
            extension: false,
            cc: 0,
            marker: self.marker,
            payload_type: self.payload_type,
            sequence_number: self.sequence,
            timestamp: self.timestamp,
            ssrc: self.ssrc,
            csrc: Vec::new(),
            extension_id: None,
            extension_data: None,
        };
        
        // Create RTP packet
        let packet = RtpPacket {
            header,
            payload: payload.clone(),
        };
        
        // Serialize packet
        let data = packet.serialize()
            .map_err(|e| Error::Media(format!("Failed to serialize RTP packet: {}", e)))?;
        
        // Send packet
        self.socket.send(&data).await
            .map_err(|e| Error::Network(e))?;
        
        // Update sequence number
        self.sequence = self.sequence.wrapping_add(1);
        
        // Update timestamp
        self.timestamp = self.timestamp.wrapping_add(self.samples_per_packet);
        
        // Clear marker after first packet
        self.marker = false;
        
        // Update last send time
        *self.last_send_time.lock().await = Some(Instant::now());
        
        Ok(())
    }
    
    /// Receive the next RTP packet
    pub async fn receive_packet(&mut self) -> Result<RtpPacket> {
        if let Some(ref mut rx) = self.packet_rx {
            match rx.recv().await {
                Some(packet) => Ok(packet),
                None => Err(Error::Media("RTP packet channel closed".into())),
            }
        } else {
            Err(Error::Media("RTP session not started".into()))
        }
    }
    
    /// Get local address
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
    
    /// Get remote address
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
    
    /// Get SSRC
    pub fn ssrc(&self) -> u32 {
        self.ssrc
    }
    
    /// Get codec
    pub fn codec(&self) -> CodecType {
        self.codec
    }
    
    /// Get last send time
    pub async fn last_send_time(&self) -> Option<Instant> {
        *self.last_send_time.lock().await
    }
    
    /// Get last receive time
    pub async fn last_receive_time(&self) -> Option<Instant> {
        *self.last_receive_time.lock().await
    }
    
    /// Set marker bit for next packet
    pub fn set_marker(&mut self, marker: bool) {
        self.marker = marker;
    }
    
    /// Set new remote address
    pub async fn set_remote_addr(&mut self, remote_addr: SocketAddr) -> Result<()> {
        if remote_addr == self.remote_addr {
            return Ok(());
        }
        
        // Update remote address
        self.remote_addr = remote_addr;
        
        // Reconnect socket
        self.socket.connect(remote_addr).await
            .map_err(|e| Error::Network(e))?;
        
        Ok(())
    }
} 