//! Media Session Controller for Session-Core Integration
//!
//! This module provides the high-level interface for session-core to control
//! media sessions. It manages the lifecycle of media sessions tied to SIP dialogs.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};
use rand::Rng;

use crate::error::{Error, Result};
use super::{MediaRelay, RelaySessionConfig, RelayEvent, RelayStats, generate_session_id, create_relay_config};

// Import RTP session capabilities
use rvoip_rtp_core::{RtpSession, RtpSessionConfig};

/// Represents a SIP Dialog ID (from session-core)
pub type DialogId = String;

/// Media configuration for a session
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// Local RTP address
    pub local_addr: SocketAddr,
    /// Remote RTP address (if known)
    pub remote_addr: Option<SocketAddr>,
    /// Preferred codec (for future implementation)
    pub preferred_codec: Option<String>,
    /// Additional media parameters
    pub parameters: HashMap<String, String>,
}

/// Media session status
#[derive(Debug, Clone)]
pub enum MediaSessionStatus {
    /// Session is being created
    Creating,
    /// Session is active and relaying media
    Active,
    /// Session is on hold
    OnHold,
    /// Session has ended
    Ended,
    /// Session failed
    Failed(String),
}

/// Information about an active media session
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    /// Dialog ID this session is associated with
    pub dialog_id: DialogId,
    /// Media relay session IDs (if this is a relay session)
    pub relay_session_ids: Option<(String, String)>,
    /// Current status
    pub status: MediaSessionStatus,
    /// Media configuration
    pub config: MediaConfig,
    /// RTP port allocated for this session
    pub rtp_port: Option<u16>,
    /// Session statistics
    pub stats: Option<RelayStats>,
    /// Creation time
    pub created_at: std::time::Instant,
}

/// RTP session wrapper for MediaSessionController
struct RtpSessionWrapper {
    /// The actual RTP session
    session: Arc<tokio::sync::Mutex<RtpSession>>,
    /// Local RTP address
    local_addr: SocketAddr,
    /// Remote RTP address (if known)
    remote_addr: Option<SocketAddr>,
    /// Session creation time
    created_at: std::time::Instant,
}

impl Default for MediaSessionInfo {
    fn default() -> Self {
        Self {
            dialog_id: String::new(),
            relay_session_ids: None,
            status: MediaSessionStatus::Creating,
            config: MediaConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            },
            rtp_port: None,
            stats: None,
            created_at: std::time::Instant::now(),
        }
    }
}

/// Events emitted by the media session controller
#[derive(Debug, Clone)]
pub enum MediaSessionEvent {
    /// Media session created
    SessionCreated {
        dialog_id: DialogId,
        session_id: DialogId,
    },
    /// Media session destroyed
    SessionDestroyed {
        dialog_id: DialogId,
        session_id: DialogId,
    },
    /// Media session failed
    SessionFailed {
        dialog_id: DialogId,
        error: String,
    },
    /// Remote address updated
    RemoteAddressUpdated {
        dialog_id: DialogId,
        remote_addr: SocketAddr,
    },
}

/// Media Session Controller for managing media sessions
pub struct MediaSessionController {
    /// Underlying media relay (optional)
    relay: Option<Arc<MediaRelay>>,
    /// Active media sessions indexed by dialog ID
    sessions: RwLock<HashMap<DialogId, MediaSessionInfo>>,
    /// Active RTP sessions indexed by dialog ID
    rtp_sessions: RwLock<HashMap<DialogId, RtpSessionWrapper>>,
    /// Port allocator for media sessions
    port_allocator: RwLock<PortAllocator>,
    /// Event channel for media session events
    event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
    /// Event receiver (taken by the user)
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<MediaSessionEvent>>>,
}

/// Simple port allocator for RTP sessions
struct PortAllocator {
    /// Base port for allocation
    base_port: u16,
    /// Next available port
    next_port: u16,
    /// Maximum port
    max_port: u16,
    /// Allocated ports
    allocated: HashMap<DialogId, u16>,
}

impl PortAllocator {
    fn new(base_port: u16, max_port: u16) -> Self {
        Self {
            base_port,
            next_port: base_port,
            max_port,
            allocated: HashMap::new(),
        }
    }
    
    fn allocate_port(&mut self) -> Option<u16> {
        // Find next available even port (RTP uses even ports)
        while self.next_port <= self.max_port {
            let port = self.next_port;
            self.next_port += 2; // Skip odd port (reserved for RTCP)
            
            if !self.allocated.values().any(|&p| p == port) {
                return Some(port);
            }
        }
        None
    }
    
    fn release_port(&mut self, port: u16) {
        self.allocated.retain(|_, &mut p| p != port);
    }
    
    fn assign_port(&mut self, dialog_id: &str, port: u16) {
        self.allocated.insert(dialog_id.to_string(), port);
    }
    
    fn get_port(&self, dialog_id: &str) -> Option<u16> {
        self.allocated.get(dialog_id).copied()
    }
}

impl MediaSessionController {
    /// Create a new media session controller
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        Self {
            relay: None,
            sessions: RwLock::new(HashMap::new()),
            rtp_sessions: RwLock::new(HashMap::new()),
            port_allocator: RwLock::new(PortAllocator::new(10000, 20000)),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
        }
    }
    
    /// Create a new media session controller with custom port range
    pub fn with_port_range(base_port: u16, max_port: u16) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        Self {
            relay: None,
            sessions: RwLock::new(HashMap::new()),
            rtp_sessions: RwLock::new(HashMap::new()),
            port_allocator: RwLock::new(PortAllocator::new(base_port, max_port)),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
        }
    }
    
    /// Start a media session for a dialog
    pub async fn start_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        info!("Starting media session for dialog: {}", dialog_id);
        
        // Check if media session already exists for this dialog
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&dialog_id) {
                return Err(Error::config(format!("Media session already exists for dialog: {}", dialog_id)));
            }
        }

        // Allocate RTP ports
        let rtp_port = {
            let mut allocator = self.port_allocator.write().await;
            allocator.allocate_port()
                .ok_or_else(|| Error::config("No available ports for media session"))?
        };
        
        // Create local RTP address with allocated port
        let local_rtp_addr = SocketAddr::new("127.0.0.1".parse().unwrap(), rtp_port);
        
        // Create RTP session configuration
        let rtp_config = RtpSessionConfig {
            local_addr: local_rtp_addr,
            remote_addr: config.remote_addr,
            ssrc: Some(rand::random()), // Generate random SSRC
            payload_type: 0, // Default to PCMU
            clock_rate: 8000, // Default to 8kHz
            jitter_buffer_size: Some(50),
            max_packet_age_ms: Some(200),
            enable_jitter_buffer: true,
        };
        
        // Create actual RTP session
        let rtp_session = RtpSession::new(rtp_config).await
            .map_err(|e| Error::config(format!("Failed to create RTP session: {}", e)))?;
        
        // Wrap RTP session
        let rtp_wrapper = RtpSessionWrapper {
            session: Arc::new(tokio::sync::Mutex::new(rtp_session)),
            local_addr: local_rtp_addr,
            remote_addr: config.remote_addr,
            created_at: std::time::Instant::now(),
        };
        
        // Create media session info
        let session_info = MediaSessionInfo {
            dialog_id: dialog_id.clone(),
            status: MediaSessionStatus::Active,
            config: config.clone(),
            rtp_port: Some(rtp_port),
            relay_session_ids: None,
            stats: None,
            created_at: std::time::Instant::now(),
        };

        // Assign port to dialog
        {
            let mut allocator = self.port_allocator.write().await;
            allocator.assign_port(&dialog_id, rtp_port);
        }

        // Store session and RTP session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(dialog_id.clone(), session_info);
        }
        
        {
            let mut rtp_sessions = self.rtp_sessions.write().await;
            rtp_sessions.insert(dialog_id.clone(), rtp_wrapper);
        }

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionCreated {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        info!("✅ Created media session with REAL RTP session: {} (port: {})", dialog_id, rtp_port);
        Ok(())
    }
    
    /// Stop media session for a dialog
    pub async fn stop_media(&self, dialog_id: String) -> Result<()> {
        info!("Stopping media session for dialog: {}", dialog_id);

        // Remove session and get info for cleanup
        let session_info = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(&dialog_id)
                .ok_or_else(|| Error::session_not_found(dialog_id.clone()))?
        };
        
        // Stop and remove RTP session
        {
            let mut rtp_sessions = self.rtp_sessions.write().await;
            if let Some(rtp_wrapper) = rtp_sessions.remove(&dialog_id) {
                // Close the RTP session
                let mut rtp_session = rtp_wrapper.session.lock().await;
                rtp_session.close().await;
                info!("✅ Stopped RTP session for dialog: {}", dialog_id);
            }
        }

        // Clean up relay if exists
        if let Some((session_a, session_b)) = &session_info.relay_session_ids {
            if let Some(relay) = &self.relay {
                let _ = relay.remove_session_pair(session_a, session_b).await;
            }
        }

        // Release port
        if let Some(port) = session_info.rtp_port {
            let mut allocator = self.port_allocator.write().await;
            allocator.release_port(port);
        }

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionDestroyed {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        Ok(())
    }
    
    /// Update media configuration (e.g., when remote address becomes known)
    pub async fn update_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        debug!("Updating media session for dialog: {}", dialog_id);
        
        let mut sessions = self.sessions.write().await;
        let session_info = sessions.get_mut(&dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.clone()))?;
        
        // Update configuration
        let old_remote = session_info.config.remote_addr;
        session_info.config = config.clone();
        
        // If remote address was set/changed, emit event
        if config.remote_addr != old_remote {
            if let Some(remote_addr) = config.remote_addr {
                let _ = self.event_tx.send(MediaSessionEvent::RemoteAddressUpdated {
                    dialog_id: dialog_id.clone(),
                    remote_addr,
                });
            }
        }
        
        debug!("Media session updated for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Create relay between two dialogs
    pub async fn create_relay(&self, dialog_a: String, dialog_b: String) -> Result<()> {
        info!("Creating relay between dialogs: {} <-> {}", dialog_a, dialog_b);

        // Verify both sessions exist and get their configs
        let (session_a_config, session_b_config) = {
            let sessions = self.sessions.read().await;
            let session_a = sessions.get(&dialog_a)
                .ok_or_else(|| Error::session_not_found(dialog_a.clone()))?;
            let session_b = sessions.get(&dialog_b)
                .ok_or_else(|| Error::session_not_found(dialog_b.clone()))?;
            (session_a.config.clone(), session_b.config.clone())
        };
        
        // Generate relay session IDs
        let relay_session_a = generate_session_id();
        let relay_session_b = generate_session_id();
        
        // Create relay configuration
        let relay_config = create_relay_config(
            relay_session_a.clone(),
            relay_session_b.clone(),
            session_a_config.local_addr,
            session_b_config.local_addr,
        );
        
        // Create the relay session pair if relay is available
        if let Some(relay) = &self.relay {
            relay.create_session_pair(relay_config).await?;
        }
        
        // Update session infos with relay session IDs
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session_a_info) = sessions.get_mut(&dialog_a) {
                session_a_info.relay_session_ids = Some((relay_session_a.clone(), relay_session_b.clone()));
                session_a_info.status = MediaSessionStatus::Active;
            }
            if let Some(session_b_info) = sessions.get_mut(&dialog_b) {
                session_b_info.relay_session_ids = Some((relay_session_b, relay_session_a));
                session_b_info.status = MediaSessionStatus::Active;
            }
        }
        
        info!("Media relay created between dialogs: {} <-> {}", dialog_a, dialog_b);
        Ok(())
    }
    
    /// Get session information for a dialog
    pub async fn get_session_info(&self, dialog_id: &str) -> Option<MediaSessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.get(dialog_id).cloned()
    }
    
    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<MediaSessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }
    
    /// Get event receiver (can only be called once)
    pub async fn take_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<MediaSessionEvent>> {
        let mut event_rx = self.event_rx.write().await;
        event_rx.take()
    }
    
    /// Get media relay reference (for advanced usage)
    pub fn relay(&self) -> Option<&Arc<MediaRelay>> {
        self.relay.as_ref()
    }
    
    /// Get RTP session for a dialog (for packet transmission)
    pub async fn get_rtp_session(&self, dialog_id: &str) -> Option<Arc<tokio::sync::Mutex<RtpSession>>> {
        let rtp_sessions = self.rtp_sessions.read().await;
        rtp_sessions.get(dialog_id).map(|wrapper| wrapper.session.clone())
    }
    
    /// Send RTP packet for a dialog
    pub async fn send_rtp_packet(&self, dialog_id: &str, payload: Vec<u8>, timestamp: u32) -> Result<()> {
        let rtp_session = self.get_rtp_session(dialog_id).await
            .ok_or_else(|| Error::session_not_found(dialog_id.to_string()))?;
        
        let mut session = rtp_session.lock().await;
        session.send_packet(timestamp, bytes::Bytes::from(payload), false).await
            .map_err(|e| Error::config(format!("Failed to send RTP packet: {}", e)))?;
        
        debug!("✅ Sent RTP packet for dialog: {} (timestamp: {})", dialog_id, timestamp);
        Ok(())
    }
    
    /// Update remote address for RTP session
    pub async fn update_rtp_remote_addr(&self, dialog_id: &str, remote_addr: SocketAddr) -> Result<()> {
        let rtp_session = self.get_rtp_session(dialog_id).await
            .ok_or_else(|| Error::session_not_found(dialog_id.to_string()))?;
        
        let mut session = rtp_session.lock().await;
        session.set_remote_addr(remote_addr);
        
        // Update wrapper info
        {
            let mut rtp_sessions = self.rtp_sessions.write().await;
            if let Some(wrapper) = rtp_sessions.get_mut(dialog_id) {
                wrapper.remote_addr = Some(remote_addr);
            }
        }
        
        info!("✅ Updated RTP remote address for dialog: {} -> {}", dialog_id, remote_addr);
        Ok(())
    }
    
    /// Get RTP session statistics
    pub async fn get_rtp_stats(&self, dialog_id: &str) -> Option<String> {
        let rtp_session = self.get_rtp_session(dialog_id).await?;
        let session = rtp_session.lock().await;
        
        // Get basic session info
        let local_addr = session.local_addr().ok()?;
        let ssrc = session.get_ssrc();
        
        Some(format!("RTP Session - Local: {}, SSRC: 0x{:08x}", local_addr, ssrc))
    }
}

impl Default for MediaSessionController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    
    #[tokio::test]
    async fn test_start_stop_session() {
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        // Start session
        let result = controller.start_media("dialog1".to_string(), config).await;
        assert!(result.is_ok());
        
        // Check session exists
        let session_info = controller.get_session_info("dialog1").await;
        assert!(session_info.is_some());
        
        // Stop session
        let result = controller.stop_media("dialog1".to_string()).await;
        assert!(result.is_ok());
        
        // Check session is removed
        let session_info = controller.get_session_info("dialog1").await;
        assert!(session_info.is_none());
    }
    
    #[tokio::test]
    async fn test_create_relay() {
        let controller = MediaSessionController::new();
        
        let config_a = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 5060)),
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        let config_b = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)), 5060)),
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        // Start both sessions
        controller.start_media("dialog_a".to_string(), config_a).await.unwrap();
        controller.start_media("dialog_b".to_string(), config_b).await.unwrap();
        
        // Create relay
        let result = controller.create_relay("dialog_a".to_string(), "dialog_b".to_string()).await;
        assert!(result.is_ok());
        
        // Check that both sessions now have relay session IDs
        let session_a = controller.get_session_info("dialog_a").await.unwrap();
        let session_b = controller.get_session_info("dialog_b").await.unwrap();
        
        assert!(session_a.relay_session_ids.is_some());
        assert!(session_b.relay_session_ids.is_some());
        assert!(matches!(session_a.status, MediaSessionStatus::Active));
        assert!(matches!(session_b.status, MediaSessionStatus::Active));
    }
} 