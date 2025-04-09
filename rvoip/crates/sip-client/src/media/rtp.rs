use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use bytes::Bytes;
use tracing::{debug, error, info, warn};

use rvoip_rtp_core::{
    RtpPacket, RtpHeader, RtpTimestamp, RtpSession as CoreRtpSession, 
    RtpSessionConfig as CoreRtpSessionConfig
};
use crate::config::CodecType;
use crate::error::{Error, Result};

/// RTP session wrapper for the SIP client
pub struct RtpSession {
    /// Core RTP session
    core_session: CoreRtpSession,

    /// Local RTP socket address
    local_addr: SocketAddr,

    /// Remote RTP destination address
    remote_addr: SocketAddr,

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

    /// Channel for received packets
    packet_rx: Option<mpsc::Receiver<RtpPacket>>,

    /// Last time a packet was sent
    last_send_time: Arc<tokio::sync::Mutex<Option<Instant>>>,

    /// Last time a packet was received
    last_receive_time: Arc<tokio::sync::Mutex<Option<Instant>>>,
}

impl RtpSession {
    /// Create a new RTP session
    pub async fn new(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        codec: CodecType,
    ) -> Result<Self> {
        // Set up codec-specific parameters
        let (payload_type, sampling_rate, samples_per_packet) = match codec {
            CodecType::PCMU => (0, 8000, 160),   // G.711 μ-law, 20ms at 8kHz
            CodecType::PCMA => (8, 8000, 160),   // G.711 A-law, 20ms at 8kHz
            CodecType::G722 => (9, 16000, 320),  // G.722, 20ms at 16kHz
            CodecType::G729 => (18, 8000, 160),  // G.729, 20ms at 8kHz
            CodecType::OPUS => (111, 48000, 960), // Opus, 20ms at 48kHz
        };
        
        // Configure core RTP session
        let config = CoreRtpSessionConfig {
            local_addr,
            remote_addr: Some(remote_addr),
            payload_type,
            clock_rate: sampling_rate,
            // Use default values for other parameters
            ..Default::default()
        };
        
        // Create the core RTP session
        let core_session = CoreRtpSession::new(config)
            .await
            .map_err(|e| Error::Media(format!("Failed to create RTP session: {}", e)))?;
        
        // Get receiver channel
        let packet_rx = Some(core_session.get_receiver_channel());
        
        debug!("RTP session created - local: {}, remote: {}", local_addr, remote_addr);
        
        Ok(Self {
            core_session,
            local_addr,
            remote_addr,
            codec,
            payload_type,
            sampling_rate,
            samples_per_packet,
            marker: true, // First packet has marker bit set
            packet_rx,
            last_send_time: Arc::new(tokio::sync::Mutex::new(None)),
            last_receive_time: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }
    
    /// Start the RTP session
    pub async fn start(&mut self) -> Result<()> {
        // The core session is already started when created
        Ok(())
    }
    
    /// Stop the RTP session
    pub async fn stop(&mut self) -> Result<()> {
        self.core_session.close().await.map_err(|e| Error::Media(e.to_string()))?;
        Ok(())
    }
    
    /// Send an RTP packet
    pub async fn send_packet(&mut self, payload: Bytes) -> Result<()> {
        // Get current timestamp based on samples per packet
        let timestamp = self.core_session.get_timestamp();
        
        // Send the packet
        self.core_session.send_packet(
            timestamp, 
            payload,
            self.marker, // Use current marker value
        )
        .await
        .map_err(|e| Error::Media(e.to_string()))?;
        
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
                Some(packet) => {
                    // Update last receive time
                    *self.last_receive_time.lock().await = Some(Instant::now());
                    Ok(packet)
                },
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
        self.core_session.get_ssrc()
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
        
        // Update core session
        self.core_session.set_remote_addr(remote_addr);
        
        Ok(())
    }
} 