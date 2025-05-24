//! Media Relay Module for Basic SIP Server
//!
//! This module provides basic RTP packet relay functionality for a SIP server.
//! It forwards RTP packets between two endpoints without transcoding, enabling
//! simple call routing through the server.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::error::{Error, Result};
use rvoip_rtp_core::{RtpSession, RtpPacket};

// Controller module for session-core integration
pub mod controller;
// Packet forwarding implementation
pub mod packet_forwarder;

// Re-export controller types for convenience
pub use controller::{
    MediaSessionController,
    MediaConfig,
    MediaSessionStatus,
    MediaSessionInfo,
    MediaSessionEvent,
    DialogId,
};

// Re-export packet forwarder types
pub use packet_forwarder::{
    PacketForwarder,
    ForwarderConfig,
    g711_passthrough::{G711PcmuCodec, G711PcmaCodec},
};

/// Unique identifier for a media relay session
pub type RelaySessionId = String;

/// Media relay statistics
#[derive(Debug, Clone)]
pub struct RelayStats {
    /// Total packets relayed
    pub packets_relayed: u64,
    /// Total bytes relayed
    pub bytes_relayed: u64,
    /// Packets dropped (errors)
    pub packets_dropped: u64,
    /// Session start time
    pub start_time: std::time::Instant,
}

impl Default for RelayStats {
    fn default() -> Self {
        Self {
            packets_relayed: 0,
            bytes_relayed: 0,
            packets_dropped: 0,
            start_time: std::time::Instant::now(),
        }
    }
}

/// Configuration for a relay session
#[derive(Debug, Clone)]
pub struct RelaySessionConfig {
    /// Session ID for endpoint A
    pub session_a_id: RelaySessionId,
    /// Session ID for endpoint B  
    pub session_b_id: RelaySessionId,
    /// Local RTP address for endpoint A
    pub local_addr_a: SocketAddr,
    /// Local RTP address for endpoint B
    pub local_addr_b: SocketAddr,
    /// Remote RTP address for endpoint A
    pub remote_addr_a: Option<SocketAddr>,
    /// Remote RTP address for endpoint B
    pub remote_addr_b: Option<SocketAddr>,
}

/// Represents a paired relay session between two endpoints
struct RelaySessionPair {
    /// Configuration
    config: RelaySessionConfig,
    /// RTP session for endpoint A
    rtp_session_a: Arc<RtpSession>,
    /// RTP session for endpoint B
    rtp_session_b: Arc<RtpSession>,
    /// Statistics
    stats: Arc<RwLock<RelayStats>>,
    /// Event sender for relay events
    event_tx: mpsc::UnboundedSender<RelayEvent>,
}

/// Events emitted by the media relay
#[derive(Debug, Clone)]
pub enum RelayEvent {
    /// Session pair created
    SessionPairCreated {
        session_a: RelaySessionId,
        session_b: RelaySessionId,
    },
    /// Session pair destroyed
    SessionPairDestroyed {
        session_a: RelaySessionId,
        session_b: RelaySessionId,
    },
    /// Packet relayed successfully
    PacketRelayed {
        from_session: RelaySessionId,
        to_session: RelaySessionId,
        packet_size: usize,
    },
    /// Error relaying packet
    RelayError {
        from_session: RelaySessionId,
        to_session: RelaySessionId,
        error: String,
    },
}

/// Main media relay for handling RTP packet forwarding
pub struct MediaRelay {
    /// Active RTP sessions indexed by session ID
    rtp_sessions: RwLock<HashMap<RelaySessionId, Arc<RtpSession>>>,
    /// Session pairs (A <-> B mapping)
    session_pairs: RwLock<HashMap<RelaySessionId, RelaySessionId>>,
    /// Session pair configurations and state
    relay_sessions: RwLock<HashMap<RelaySessionId, Arc<RelaySessionPair>>>,
    /// Event channel for relay events
    event_tx: mpsc::UnboundedSender<RelayEvent>,
    /// Event receiver (taken by the user)
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<RelayEvent>>>,
}

impl MediaRelay {
    /// Create a new media relay
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        Self {
            rtp_sessions: RwLock::new(HashMap::new()),
            session_pairs: RwLock::new(HashMap::new()),
            relay_sessions: RwLock::new(HashMap::new()),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
        }
    }
    
    /// Create a new relay session pair
    pub async fn create_session_pair(&self, config: RelaySessionConfig) -> Result<()> {
        info!("Creating relay session pair: {} <-> {}", 
              config.session_a_id, config.session_b_id);
        
        // Create RTP sessions for both endpoints
        let rtp_session_a = Arc::new(RtpSession::new());
        let rtp_session_b = Arc::new(RtpSession::new());
        
        // Configure RTP sessions with local addresses
        // Note: This is a simplified setup - in a real implementation,
        // you'd configure payload types, SSRCs, etc.
        
        // Create session pair
        let pair = Arc::new(RelaySessionPair {
            config: config.clone(),
            rtp_session_a: rtp_session_a.clone(),
            rtp_session_b: rtp_session_b.clone(),
            stats: Arc::new(RwLock::new(RelayStats {
                start_time: std::time::Instant::now(),
                ..Default::default()
            })),
            event_tx: self.event_tx.clone(),
        });
        
        // Store sessions and mappings
        {
            let mut sessions = self.rtp_sessions.write().await;
            sessions.insert(config.session_a_id.clone(), rtp_session_a);
            sessions.insert(config.session_b_id.clone(), rtp_session_b);
        }
        
        {
            let mut pairs = self.session_pairs.write().await;
            pairs.insert(config.session_a_id.clone(), config.session_b_id.clone());
            pairs.insert(config.session_b_id.clone(), config.session_a_id.clone());
        }
        
        {
            let mut relay_sessions = self.relay_sessions.write().await;
            relay_sessions.insert(config.session_a_id.clone(), pair.clone());
            relay_sessions.insert(config.session_b_id.clone(), pair);
        }
        
        // Emit event
        let _ = self.event_tx.send(RelayEvent::SessionPairCreated {
            session_a: config.session_a_id.clone(),
            session_b: config.session_b_id.clone(),
        });
        
        // Start packet forwarding tasks
        self.start_forwarding_tasks(&config).await?;
        
        Ok(())
    }
    
    /// Remove a session pair
    pub async fn remove_session_pair(&self, session_a_id: &str, session_b_id: &str) -> Result<()> {
        info!("Removing relay session pair: {} <-> {}", session_a_id, session_b_id);
        
        // Remove from all collections
        {
            let mut sessions = self.rtp_sessions.write().await;
            sessions.remove(session_a_id);
            sessions.remove(session_b_id);
        }
        
        {
            let mut pairs = self.session_pairs.write().await;
            pairs.remove(session_a_id);
            pairs.remove(session_b_id);
        }
        
        {
            let mut relay_sessions = self.relay_sessions.write().await;
            relay_sessions.remove(session_a_id);
            relay_sessions.remove(session_b_id);
        }
        
        // Emit event
        let _ = self.event_tx.send(RelayEvent::SessionPairDestroyed {
            session_a: session_a_id.to_string(),
            session_b: session_b_id.to_string(),
        });
        
        Ok(())
    }
    
    /// Get statistics for a session
    pub async fn get_session_stats(&self, session_id: &str) -> Option<RelayStats> {
        let relay_sessions = self.relay_sessions.read().await;
        if let Some(pair) = relay_sessions.get(session_id) {
            let stats = pair.stats.read().await;
            Some(stats.clone())
        } else {
            None
        }
    }
    
    /// Set remote address for a session
    pub async fn set_remote_address(&self, session_id: &str, remote_addr: SocketAddr) -> Result<()> {
        let sessions = self.rtp_sessions.read().await;
        if let Some(rtp_session) = sessions.get(session_id) {
            // TODO: Configure RTP session with remote address
            // This depends on the actual RTP core API
            debug!("Set remote address for session {}: {}", session_id, remote_addr);
            Ok(())
        } else {
            Err(Error::SessionNotFound(session_id.to_string()))
        }
    }
    
    /// Get event receiver (can only be called once)
    pub async fn take_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<RelayEvent>> {
        let mut event_rx = self.event_rx.write().await;
        event_rx.take()
    }
    
    /// Check if a session exists
    pub async fn has_session(&self, session_id: &str) -> bool {
        let sessions = self.rtp_sessions.read().await;
        sessions.contains_key(session_id)
    }
    
    /// Get the paired session ID for a given session
    pub async fn get_paired_session(&self, session_id: &str) -> Option<String> {
        let pairs = self.session_pairs.read().await;
        pairs.get(session_id).cloned()
    }
    
    /// Start packet forwarding tasks for a session pair
    async fn start_forwarding_tasks(&self, config: &RelaySessionConfig) -> Result<()> {
        // In a real implementation, we would:
        // 1. Set up packet listeners on both RTP sessions
        // 2. When a packet arrives on session A, forward it to session B
        // 3. Handle SSRC rewriting if needed
        // 4. Update statistics
        
        // For now, this is a placeholder
        debug!("Starting forwarding tasks for session pair: {} <-> {}", 
               config.session_a_id, config.session_b_id);
        
        // TODO: Implement actual packet forwarding using RTP core events
        // This would involve:
        // - Listening for RTP packets on each session
        // - Rewriting SSRC values if needed
        // - Forwarding packets to the paired session
        // - Updating relay statistics
        
        Ok(())
    }
    
    /// Forward a packet between sessions (called by forwarding tasks)
    async fn forward_packet(&self, from_session: &str, to_session: &str, mut packet: RtpPacket) -> Result<()> {
        // Get the target RTP session
        let sessions = self.rtp_sessions.read().await;
        let target_session = sessions.get(to_session)
            .ok_or_else(|| Error::SessionNotFound(to_session.to_string()))?;
        
        // Rewrite SSRC if needed (for call routing)
        // In a basic implementation, we might just pass through
        // packet.header.ssrc = generate_new_ssrc_for_session(to_session);
        
        // Send packet to target session
        // TODO: Use actual RTP session send API
        // target_session.send_packet(packet).await?;
        
        // Update statistics
        if let Some(pair) = {
            let relay_sessions = self.relay_sessions.read().await;
            relay_sessions.get(from_session).cloned()
        } {
            let mut stats = pair.stats.write().await;
            stats.packets_relayed += 1;
            stats.bytes_relayed += packet.payload.len() as u64;
        }
        
        // Emit event
        let _ = self.event_tx.send(RelayEvent::PacketRelayed {
            from_session: from_session.to_string(),
            to_session: to_session.to_string(),
            packet_size: packet.payload.len(),
        });
        
        Ok(())
    }
}

impl Default for MediaRelay {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to generate a unique session ID
pub fn generate_session_id() -> RelaySessionId {
    Uuid::new_v4().to_string()
}

/// Helper function to create a relay session config
pub fn create_relay_config(
    session_a_id: RelaySessionId,
    session_b_id: RelaySessionId,
    local_addr_a: SocketAddr,
    local_addr_b: SocketAddr,
) -> RelaySessionConfig {
    RelaySessionConfig {
        session_a_id,
        session_b_id,
        local_addr_a,
        local_addr_b,
        remote_addr_a: None,
        remote_addr_b: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    
    #[tokio::test]
    async fn test_create_session_pair() {
        let relay = MediaRelay::new();
        
        let config = create_relay_config(
            "session_a".to_string(),
            "session_b".to_string(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10000),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10002),
        );
        
        let result = relay.create_session_pair(config).await;
        assert!(result.is_ok());
        
        // Check that sessions exist
        assert!(relay.has_session("session_a").await);
        assert!(relay.has_session("session_b").await);
        
        // Check pairing
        assert_eq!(relay.get_paired_session("session_a").await, Some("session_b".to_string()));
        assert_eq!(relay.get_paired_session("session_b").await, Some("session_a".to_string()));
    }
    
    #[tokio::test]
    async fn test_remove_session_pair() {
        let relay = MediaRelay::new();
        
        let config = create_relay_config(
            "session_a".to_string(),
            "session_b".to_string(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10000),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10002),
        );
        
        relay.create_session_pair(config).await.unwrap();
        relay.remove_session_pair("session_a", "session_b").await.unwrap();
        
        // Check that sessions are removed
        assert!(!relay.has_session("session_a").await);
        assert!(!relay.has_session("session_b").await);
    }
} 