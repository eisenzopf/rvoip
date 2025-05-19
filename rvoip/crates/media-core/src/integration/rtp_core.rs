use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::error::Result;
use crate::rtp::session::{RtpSession, RtpSessionConfig, RtpSessionEvent};

/// RTP manager configuration
#[derive(Debug, Clone)]
pub struct RtpManagerConfig {
    /// Default bind address for RTP
    pub bind_address: String,
    /// Port range for RTP (start, end)
    pub port_range: (u16, u16),
    /// RTCP multiplexing enabled
    pub rtcp_mux: bool,
    /// SRTP enabled
    pub srtp_enabled: bool,
    /// Maximum packet size
    pub max_packet_size: usize,
    /// Send buffer size
    pub send_buffer_size: usize,
    /// Receive buffer size
    pub recv_buffer_size: usize,
}

impl Default for RtpManagerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            port_range: (10000, 20000),
            rtcp_mux: true,
            srtp_enabled: true,
            max_packet_size: 1500,
            send_buffer_size: 65536,
            recv_buffer_size: 65536,
        }
    }
}

/// RTP session information
#[derive(Debug, Clone)]
pub struct RtpSessionInfo {
    /// Session ID
    pub session_id: String,
    /// Local address
    pub local_addr: SocketAddr,
    /// Remote address
    pub remote_addr: Option<SocketAddr>,
    /// SSRC
    pub ssrc: u32,
    /// Payload type
    pub payload_type: u8,
    /// Clock rate
    pub clock_rate: u32,
    /// RTCP interval in milliseconds
    pub rtcp_interval_ms: u32,
    /// Whether SRTP is enabled
    pub srtp_enabled: bool,
    /// Total packets sent
    pub packets_sent: u64,
    /// Total packets received
    pub packets_received: u64,
    /// Lost packets
    pub packets_lost: u64,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Round-trip time in milliseconds
    pub rtt_ms: Option<f32>,
}

/// RTP manager for integration with rtp-core
pub struct RtpManager {
    /// Configuration
    config: RtpManagerConfig,
    /// Active RTP sessions
    sessions: Mutex<Vec<Arc<RtpSession>>>,
    /// Port allocator
    port_allocator: Mutex<PortAllocator>,
}

/// Port allocator for RTP sessions
struct PortAllocator {
    /// Available ports
    available_ports: Vec<u16>,
    /// Used ports
    used_ports: Vec<u16>,
}

impl PortAllocator {
    /// Create a new port allocator
    fn new(port_range: (u16, u16)) -> Self {
        let mut available_ports = Vec::new();
        
        // Only use even ports for RTP (RTCP will use the next odd port)
        for port in (port_range.0..=port_range.1).step_by(2) {
            available_ports.push(port);
        }
        
        Self {
            available_ports,
            used_ports: Vec::new(),
        }
    }
    
    /// Allocate a port
    fn allocate(&mut self) -> Option<u16> {
        if let Some(port) = self.available_ports.pop() {
            self.used_ports.push(port);
            Some(port)
        } else {
            None
        }
    }
    
    /// Release a port
    fn release(&mut self, port: u16) {
        if let Some(index) = self.used_ports.iter().position(|&p| p == port) {
            self.used_ports.remove(index);
            self.available_ports.push(port);
            self.available_ports.sort_unstable();
        }
    }
}

impl RtpManager {
    /// Create a new RTP manager
    pub fn new(config: RtpManagerConfig) -> Self {
        Self {
            config: config.clone(),
            sessions: Mutex::new(Vec::new()),
            port_allocator: Mutex::new(PortAllocator::new(config.port_range)),
        }
    }
    
    /// Create a new RTP session
    pub async fn create_session(&self, session_id: &str) -> Result<(Arc<RtpSession>, mpsc::Receiver<RtpSessionEvent>)> {
        // Allocate a port
        let port = {
            let mut allocator = self.port_allocator.lock().unwrap();
            allocator.allocate().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    "No available ports for RTP session"
                )
            })?
        };
        
        // Create socket address
        let local_addr = format!("{}:{}", self.config.bind_address, port).parse()?;
        
        // Create RTP session config
        let config = RtpSessionConfig {
            local_addr,
            remote_addr: None,
            ssrc: rand::random(),
            payload_type: 0, // Will be set later
            clock_rate: 8000, // Default, will be set later
            rtcp_interval_ms: 5000,
            rtcp_mux: self.config.rtcp_mux,
            srtp_enabled: self.config.srtp_enabled,
            max_packet_size: self.config.max_packet_size,
            send_buffer_size: self.config.send_buffer_size,
            recv_buffer_size: self.config.recv_buffer_size,
        };
        
        // Create RTP session
        let (session, events) = RtpSession::new(config).await?;
        let session = Arc::new(session);
        
        // Store the session
        {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.push(session.clone());
        }
        
        info!("Created RTP session: id={}, local_addr={}", session_id, local_addr);
        
        Ok((session, events))
    }
    
    /// Close an RTP session
    pub async fn close_session(&self, session: &RtpSession) -> Result<()> {
        // Stop the session
        session.stop().await?;
        
        // Get the local port to release
        let local_port = session.local_addr().port();
        
        // Release the port
        {
            let mut allocator = self.port_allocator.lock().unwrap();
            allocator.release(local_port);
        }
        
        // Remove from sessions
        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(index) = sessions.iter().position(|s| Arc::ptr_eq(s, &Arc::new(session.clone()))) {
                sessions.remove(index);
            }
        }
        
        info!("Closed RTP session: local_addr={}", session.local_addr());
        
        Ok(())
    }
    
    /// Get session information
    pub fn get_session_info(&self, session: &RtpSession) -> RtpSessionInfo {
        // Get session statistics
        let stats = session.get_statistics();
        
        RtpSessionInfo {
            session_id: "".to_string(), // Not tracked by RtpSession
            local_addr: session.local_addr(),
            remote_addr: session.remote_addr(),
            ssrc: session.ssrc(),
            payload_type: session.payload_type(),
            clock_rate: session.clock_rate(),
            rtcp_interval_ms: session.rtcp_interval_ms(),
            srtp_enabled: session.srtp_enabled(),
            packets_sent: stats.packets_sent,
            packets_received: stats.packets_received,
            packets_lost: stats.packets_lost,
            jitter_ms: stats.jitter_ms,
            rtt_ms: stats.rtt_ms,
        }
    }
    
    /// Get all active sessions
    pub fn get_all_sessions(&self) -> Vec<Arc<RtpSession>> {
        let sessions = self.sessions.lock().unwrap();
        sessions.clone()
    }
    
    /// Set SRTP key material
    pub async fn set_srtp_keys(&self, session: &RtpSession, local_key: &[u8], remote_key: &[u8]) -> Result<()> {
        session.set_srtp_keys(local_key, remote_key).await
    }
    
    /// Configure session with codecs
    pub fn configure_session(&self, session: &RtpSession, payload_type: u8, clock_rate: u32) -> Result<()> {
        session.set_payload_type(payload_type)?;
        session.set_clock_rate(clock_rate)?;
        Ok(())
    }
    
    /// Set remote address for a session
    pub fn set_remote_address(&self, session: &RtpSession, remote_addr: SocketAddr) -> Result<()> {
        session.set_remote_addr(remote_addr)
    }
} 