//! P2P Heartbeat mechanism for direct peer presence
//!
//! Provides heartbeat-based presence detection for P2P scenarios
//! where peers communicate directly without a presence server.

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use tokio::sync::RwLock;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, info, warn};
use dashmap::DashMap;

use crate::coordinator::presence::{PresenceStatus, PresenceInfo};
use crate::errors::{Result, SessionError};

/// P2P presence heartbeat configuration
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// How often to send heartbeat messages
    pub send_interval: Duration,
    
    /// How long to wait before considering a peer offline
    pub offline_threshold: Duration,
    
    /// Whether to automatically mark offline peers as gone
    pub auto_cleanup: bool,
    
    /// Maximum number of peers to track
    pub max_peers: usize,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            send_interval: Duration::from_secs(30),
            offline_threshold: Duration::from_secs(90),
            auto_cleanup: true,
            max_peers: 1000,
        }
    }
}

/// Tracks heartbeat state for a single peer
#[derive(Debug, Clone)]
struct PeerHeartbeat {
    /// Peer identifier (SIP URI)
    peer_uri: String,
    
    /// Last time we received a heartbeat
    last_seen: Instant,
    
    /// Current presence status
    status: PresenceStatus,
    
    /// Optional presence note
    note: Option<String>,
    
    /// Number of missed heartbeats
    missed_count: u32,
}

impl PeerHeartbeat {
    fn new(peer_uri: String, status: PresenceStatus) -> Self {
        Self {
            peer_uri,
            last_seen: Instant::now(),
            status,
            note: None,
            missed_count: 0,
        }
    }
    
    fn is_online(&self, threshold: Duration) -> bool {
        self.last_seen.elapsed() < threshold
    }
    
    fn update(&mut self, status: PresenceStatus, note: Option<String>) {
        self.last_seen = Instant::now();
        self.status = status;
        self.note = note;
        self.missed_count = 0;
    }
    
    fn mark_missed(&mut self) {
        self.missed_count += 1;
    }
}

/// P2P Heartbeat manager for presence detection
pub struct P2PHeartbeatManager {
    /// Configuration
    config: HeartbeatConfig,
    
    /// Tracked peers
    peers: Arc<DashMap<String, PeerHeartbeat>>,
    
    /// Callbacks for presence changes
    presence_callbacks: Arc<RwLock<Vec<Box<dyn Fn(String, PresenceStatus) + Send + Sync>>>>,
    
    /// Whether the heartbeat task is running
    running: Arc<RwLock<bool>>,
}

impl P2PHeartbeatManager {
    /// Create a new P2P heartbeat manager
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            config,
            peers: Arc::new(DashMap::new()),
            presence_callbacks: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }
    
    /// Start the heartbeat monitoring task
    pub async fn start(&self) -> Result<()> {
        let mut running = self.running.write().await;
        if *running {
            return Ok(()); // Already running
        }
        *running = true;
        
        let peers = self.peers.clone();
        let config = self.config.clone();
        let callbacks = self.presence_callbacks.clone();
        let running_flag = self.running.clone();
        
        // Spawn monitoring task
        tokio::spawn(async move {
            let mut ticker = interval(config.send_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            
            while *running_flag.read().await {
                ticker.tick().await;
                
                // Check all peers for timeout
                let mut offline_peers = Vec::new();
                let mut peers_to_update = Vec::new();
                
                // First pass: identify peers that need updating
                for entry in peers.iter() {
                    let uri = entry.key().clone();
                    let peer = entry.value();
                    
                    if !peer.is_online(config.offline_threshold) {
                        peers_to_update.push(uri);
                    }
                }
                
                // Second pass: update the peers
                for uri in peers_to_update {
                    if let Some(mut peer) = peers.get_mut(&uri) {
                        peer.mark_missed();
                        
                        if peer.missed_count >= 3 {
                            // Mark as offline after 3 missed heartbeats
                            offline_peers.push(uri.clone());
                            
                            // Notify callbacks
                            let cbs = callbacks.read().await;
                            for callback in cbs.iter() {
                                callback(uri.clone(), PresenceStatus::Offline);
                            }
                        }
                    }
                }
                
                // Clean up offline peers if configured
                if config.auto_cleanup {
                    for uri in offline_peers {
                        peers.remove(&uri);
                        info!("Removed offline peer from P2P tracking: {}", uri);
                    }
                }
            }
            
            info!("P2P heartbeat monitoring stopped");
        });
        
        info!("P2P heartbeat monitoring started");
        Ok(())
    }
    
    /// Stop the heartbeat monitoring
    pub async fn stop(&self) -> Result<()> {
        let mut running = self.running.write().await;
        *running = false;
        info!("P2P heartbeat monitoring stopping");
        Ok(())
    }
    
    /// Process incoming heartbeat from a peer
    pub async fn receive_heartbeat(
        &self,
        peer_uri: &str,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        // Check peer limit
        if self.peers.len() >= self.config.max_peers && !self.peers.contains_key(peer_uri) {
            warn!("P2P peer limit reached, ignoring heartbeat from {}", peer_uri);
            return Err(SessionError::ResourceLimitExceeded(
                "Maximum P2P peers reached".to_string()
            ));
        }
        
        let is_new = !self.peers.contains_key(peer_uri);
        let prev_status = self.peers.get(peer_uri).map(|p| p.status.clone());
        
        // Update or insert peer
        self.peers
            .entry(peer_uri.to_string())
            .and_modify(|p| p.update(status.clone(), note.clone()))
            .or_insert_with(|| {
                let mut peer = PeerHeartbeat::new(peer_uri.to_string(), status.clone());
                peer.note = note.clone();
                peer
            });
        
        // Notify callbacks if status changed
        if is_new || prev_status != Some(status.clone()) {
            let callbacks = self.presence_callbacks.read().await;
            for callback in callbacks.iter() {
                callback(peer_uri.to_string(), status.clone());
            }
        }
        
        debug!("Received P2P heartbeat from {}: {:?}", peer_uri, status);
        Ok(())
    }
    
    /// Send heartbeat to a specific peer
    pub async fn send_heartbeat(
        &self,
        peer_uri: &str,
        my_status: PresenceStatus,
        my_note: Option<String>,
    ) -> Result<()> {
        // In a real implementation, this would send a SIP MESSAGE or OPTIONS
        // with presence information to the peer
        
        debug!("Sending P2P heartbeat to {}: {:?}", peer_uri, my_status);
        
        // TODO: Integrate with dialog-core to send actual SIP message
        // For now, this is a placeholder
        
        Ok(())
    }
    
    /// Broadcast heartbeat to all tracked peers
    pub async fn broadcast_heartbeat(
        &self,
        my_status: PresenceStatus,
        my_note: Option<String>,
    ) -> Result<()> {
        let peer_uris: Vec<String> = self.peers
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        
        for uri in peer_uris {
            if let Err(e) = self.send_heartbeat(&uri, my_status.clone(), my_note.clone()).await {
                warn!("Failed to send heartbeat to {}: {}", uri, e);
            }
        }
        
        Ok(())
    }
    
    /// Get current presence for a peer
    pub fn get_peer_presence(&self, peer_uri: &str) -> Option<PresenceInfo> {
        self.peers.get(peer_uri).map(|peer| {
            PresenceInfo::new(peer.peer_uri.clone(), peer.status.clone())
                .with_note(peer.note.clone())
        })
    }
    
    /// Get all tracked peers and their presence
    pub fn get_all_peers(&self) -> HashMap<String, PresenceInfo> {
        self.peers
            .iter()
            .map(|entry| {
                let peer = entry.value();
                (
                    entry.key().clone(),
                    PresenceInfo::new(peer.peer_uri.clone(), peer.status.clone())
                        .with_note(peer.note.clone()),
                )
            })
            .collect()
    }
    
    /// Add a callback for presence changes
    pub async fn add_presence_callback<F>(&self, callback: F)
    where
        F: Fn(String, PresenceStatus) + Send + Sync + 'static,
    {
        let mut callbacks = self.presence_callbacks.write().await;
        callbacks.push(Box::new(callback));
    }
    
    /// Remove a peer from tracking
    pub fn remove_peer(&self, peer_uri: &str) -> Option<PeerHeartbeat> {
        self.peers.remove(peer_uri).map(|(_, peer)| peer)
    }
    
    /// Clear all tracked peers
    pub fn clear_all_peers(&self) {
        self.peers.clear();
        info!("Cleared all P2P presence tracking");
    }
}

/// Integration with PresenceCoordinator
impl P2PHeartbeatManager {
    /// Sync P2P presence with the main presence coordinator
    pub async fn sync_with_coordinator(
        &self,
        presence_coordinator: &Arc<RwLock<crate::coordinator::presence::PresenceCoordinator>>,
    ) -> Result<()> {
        let coordinator = presence_coordinator.read().await;
        
        for entry in self.peers.iter() {
            let peer = entry.value();
            let presence_info = PresenceInfo::new(
                peer.peer_uri.clone(),
                peer.status.clone(),
            ).with_note(peer.note.clone());
            
            // Update coordinator with P2P presence
            coordinator.update_presence(
                peer.peer_uri.clone(),
                peer.status.clone(),
                peer.note.clone(),
            ).await?;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_heartbeat_tracking() {
        let manager = P2PHeartbeatManager::new(HeartbeatConfig::default());
        
        // Start monitoring
        manager.start().await.unwrap();
        
        // Receive heartbeat
        manager.receive_heartbeat(
            "sip:alice@example.com",
            PresenceStatus::Available,
            Some("Working".to_string()),
        ).await.unwrap();
        
        // Check presence
        let presence = manager.get_peer_presence("sip:alice@example.com");
        assert!(presence.is_some());
        assert_eq!(presence.unwrap().status, PresenceStatus::Available);
        
        // Stop monitoring
        manager.stop().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_offline_detection() {
        let mut config = HeartbeatConfig::default();
        config.offline_threshold = Duration::from_millis(100); // Short for testing
        
        let manager = P2PHeartbeatManager::new(config);
        
        // Add peer
        manager.receive_heartbeat(
            "sip:bob@example.com",
            PresenceStatus::Available,
            None,
        ).await.unwrap();
        
        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;
        
        // Check if marked offline
        let peer = manager.peers.get("sip:bob@example.com");
        assert!(peer.is_some());
        assert!(!peer.unwrap().is_online(Duration::from_millis(100)));
    }
    
    #[tokio::test]
    async fn test_peer_limit() {
        let mut config = HeartbeatConfig::default();
        config.max_peers = 2;
        
        let manager = P2PHeartbeatManager::new(config);
        
        // Add peers up to limit
        manager.receive_heartbeat("sip:peer1@example.com", PresenceStatus::Available, None)
            .await.unwrap();
        manager.receive_heartbeat("sip:peer2@example.com", PresenceStatus::Available, None)
            .await.unwrap();
        
        // Try to add beyond limit
        let result = manager.receive_heartbeat(
            "sip:peer3@example.com",
            PresenceStatus::Available,
            None,
        ).await;
        
        assert!(result.is_err());
    }
}