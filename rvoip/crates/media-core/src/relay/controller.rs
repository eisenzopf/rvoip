//! Media Session Controller for Session-Core Integration
//!
//! This module provides the high-level interface for session-core to control
//! media sessions. It manages the lifecycle of media sessions tied to SIP dialogs.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use super::{MediaRelay, RelaySessionConfig, RelayEvent, RelayStats, generate_session_id, create_relay_config};

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
    /// Session statistics
    pub stats: Option<RelayStats>,
    /// Creation time
    pub created_at: std::time::Instant,
}

/// Events emitted by the media session controller
#[derive(Debug, Clone)]
pub enum MediaSessionEvent {
    /// Media session started
    SessionStarted {
        dialog_id: DialogId,
        local_addr: SocketAddr,
    },
    /// Media session ended
    SessionEnded {
        dialog_id: DialogId,
        reason: String,
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
    /// Underlying media relay
    relay: Arc<MediaRelay>,
    /// Active media sessions indexed by dialog ID
    sessions: RwLock<HashMap<DialogId, MediaSessionInfo>>,
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
    
    fn allocate(&mut self, dialog_id: &str) -> Option<u16> {
        // Find next available even port (RTP uses even ports)
        while self.next_port <= self.max_port {
            let port = self.next_port;
            self.next_port += 2; // Skip odd port (reserved for RTCP)
            
            if !self.allocated.values().any(|&p| p == port) {
                self.allocated.insert(dialog_id.to_string(), port);
                return Some(port);
            }
        }
        None
    }
    
    fn release(&mut self, dialog_id: &str) {
        self.allocated.remove(dialog_id);
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
            relay: Arc::new(MediaRelay::new()),
            sessions: RwLock::new(HashMap::new()),
            port_allocator: RwLock::new(PortAllocator::new(10000, 20000)),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
        }
    }
    
    /// Create a new media session controller with custom port range
    pub fn with_port_range(base_port: u16, max_port: u16) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        Self {
            relay: Arc::new(MediaRelay::new()),
            sessions: RwLock::new(HashMap::new()),
            port_allocator: RwLock::new(PortAllocator::new(base_port, max_port)),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
        }
    }
    
    /// Start a media session for a dialog
    pub async fn start_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        info!("Starting media session for dialog: {}", dialog_id);
        
        // Check if session already exists
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&dialog_id) {
                return Err(Error::InvalidState(format!("Media session already exists for dialog: {}", dialog_id)));
            }
        }
        
        // Allocate a local port
        let local_port = {
            let mut allocator = self.port_allocator.write().await;
            allocator.allocate(&dialog_id)
                .ok_or_else(|| Error::Other("No available ports for media session".to_string()))?
        };
        
        // Create local address
        let local_addr = SocketAddr::new(config.local_addr.ip(), local_port);
        
        // Create session info
        let session_info = MediaSessionInfo {
            dialog_id: dialog_id.clone(),
            relay_session_ids: None, // Will be set when we have a peer
            status: MediaSessionStatus::Creating,
            config: MediaConfig {
                local_addr,
                ..config
            },
            stats: None,
            created_at: std::time::Instant::now(),
        };
        
        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(dialog_id.clone(), session_info);
        }
        
        // Emit event
        let _ = self.event_tx.send(MediaSessionEvent::SessionStarted {
            dialog_id: dialog_id.clone(),
            local_addr,
        });
        
        info!("Media session started for dialog {} on {}", dialog_id, local_addr);
        Ok(())
    }
    
    /// Stop a media session
    pub async fn stop_media(&self, dialog_id: DialogId) -> Result<()> {
        info!("Stopping media session for dialog: {}", dialog_id);
        
        // Get session info
        let session_info = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(&dialog_id)
                .ok_or_else(|| Error::SessionNotFound(dialog_id.clone()))?
        };
        
        // Remove relay session if it exists
        if let Some((session_a, session_b)) = session_info.relay_session_ids {
            self.relay.remove_session_pair(&session_a, &session_b).await?;
        }
        
        // Release port
        {
            let mut allocator = self.port_allocator.write().await;
            allocator.release(&dialog_id);
        }
        
        // Emit event
        let _ = self.event_tx.send(MediaSessionEvent::SessionEnded {
            dialog_id: dialog_id.clone(),
            reason: "Session stopped".to_string(),
        });
        
        info!("Media session stopped for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Update media configuration (e.g., when remote address becomes known)
    pub async fn update_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        debug!("Updating media session for dialog: {}", dialog_id);
        
        let mut sessions = self.sessions.write().await;
        let session_info = sessions.get_mut(&dialog_id)
            .ok_or_else(|| Error::SessionNotFound(dialog_id.clone()))?;
        
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
    
    /// Create a media relay between two dialogs (for call routing)
    pub async fn create_relay(&self, dialog_a: DialogId, dialog_b: DialogId) -> Result<()> {
        info!("Creating media relay between dialogs: {} <-> {}", dialog_a, dialog_b);
        
        let mut sessions = self.sessions.write().await;
        
        // Get both session infos
        let session_a = sessions.get(&dialog_a)
            .ok_or_else(|| Error::SessionNotFound(dialog_a.clone()))?;
        let session_b = sessions.get(&dialog_b)
            .ok_or_else(|| Error::SessionNotFound(dialog_b.clone()))?;
        
        // Generate relay session IDs
        let relay_session_a = generate_session_id();
        let relay_session_b = generate_session_id();
        
        // Create relay configuration
        let relay_config = create_relay_config(
            relay_session_a.clone(),
            relay_session_b.clone(),
            session_a.config.local_addr,
            session_b.config.local_addr,
        );
        
        // Create the relay session pair
        self.relay.create_session_pair(relay_config).await?;
        
        // Update session infos with relay session IDs
        if let Some(session_a) = sessions.get_mut(&dialog_a) {
            session_a.relay_session_ids = Some((relay_session_a.clone(), relay_session_b.clone()));
            session_a.status = MediaSessionStatus::Active;
        }
        
        if let Some(session_b) = sessions.get_mut(&dialog_b) {
            session_b.relay_session_ids = Some((relay_session_b, relay_session_a));
            session_b.status = MediaSessionStatus::Active;
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
    pub fn relay(&self) -> &Arc<MediaRelay> {
        &self.relay
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