//! High-level presence API for SimplePeer
//!
//! This module provides simple, developer-friendly presence functionality
//! that works transparently in both P2P and B2BUA scenarios.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::api::SimplePeer;
use crate::coordinator::presence::{PresenceStatus, PresenceInfo};
use crate::errors::{Result, SessionError};
use crate::manager::events::SessionEvent;

/// Builder for setting presence status
pub struct PresenceBuilder<'a> {
    peer: &'a SimplePeer,
    status: PresenceStatus,
    note: Option<String>,
    device: Option<String>,
    capabilities: Vec<String>,
}

impl<'a> PresenceBuilder<'a> {
    /// Create a new presence builder
    pub(crate) fn new(peer: &'a SimplePeer, status: PresenceStatus) -> Self {
        Self {
            peer,
            status,
            note: None,
            device: None,
            capabilities: vec!["audio".to_string()],
        }
    }
    
    /// Add a custom note to the presence
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
    
    /// Set the device identifier
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
        self
    }
    
    /// Set capabilities (e.g., ["audio", "video", "chat"])
    pub fn capabilities(mut self, caps: Vec<String>) -> Self {
        self.capabilities = caps;
        self
    }
    
    /// Add a single capability
    pub fn with_capability(mut self, cap: impl Into<String>) -> Self {
        self.capabilities.push(cap.into());
        self
    }
    
    /// Publish the presence status
    pub async fn publish(self) -> Result<()> {
        // Get the user's SIP URI
        let local_addr = format!("{}:{}", self.peer.local_addr, self.peer.port);
        let entity = format!("sip:{}@{}", 
            self.peer.identity,
            local_addr
        );
        
        // Update presence in the coordinator
        let presence_coordinator = self.peer.coordinator.presence_coordinator.read().await;
        presence_coordinator.update_presence(
            entity.clone(),
            self.status.clone(),
            self.note,
        ).await?;
        
        // TODO: Send PUBLISH if connected to a presence server
        info!("Presence updated for {}: {:?}", entity, self.status);
        
        Ok(())
    }
}

// Implement IntoFuture for PresenceBuilder to allow await directly
impl<'a> std::future::IntoFuture for PresenceBuilder<'a> {
    type Output = Result<()>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.publish())
    }
}

/// Watches for presence updates of another user
pub struct PresenceWatcher {
    /// The entity being watched
    entity: String,
    
    /// Channel for receiving presence updates
    rx: mpsc::Receiver<PresenceInfo>,
    
    /// Subscription dialog ID (if subscribed)
    dialog_id: Option<rvoip_dialog_core::DialogId>,
    
    /// Reference to coordinator for cleanup
    coordinator: Arc<crate::coordinator::SessionCoordinator>,
}

impl PresenceWatcher {
    /// Create a new presence watcher
    pub(crate) fn new(
        entity: String,
        rx: mpsc::Receiver<PresenceInfo>,
        coordinator: Arc<crate::coordinator::SessionCoordinator>,
    ) -> Self {
        Self {
            entity,
            rx,
            dialog_id: None,
            coordinator,
        }
    }
    
    /// Set the subscription dialog ID
    pub(crate) fn with_dialog(mut self, dialog_id: rvoip_dialog_core::DialogId) -> Self {
        self.dialog_id = Some(dialog_id);
        self
    }
    
    /// Receive the next presence update
    pub async fn recv(&mut self) -> Option<PresenceInfo> {
        self.rx.recv().await
    }
    
    /// Get the current presence state
    pub async fn current(&self) -> Option<PresenceInfo> {
        let presence_coordinator = self.coordinator.presence_coordinator.read().await;
        presence_coordinator.get_presence(&self.entity)
    }
    
    /// Stop watching (unsubscribe)
    pub async fn stop(self) -> Result<()> {
        if let Some(dialog_id) = self.dialog_id {
            // Terminate the subscription
            let presence_coordinator = self.coordinator.presence_coordinator.read().await;
            presence_coordinator.terminate_subscription(
                dialog_id,
                Some("Watcher stopped".to_string()),
            ).await?;
        }
        
        info!("Stopped watching presence for {}", self.entity);
        Ok(())
    }
}

/// Buddy list for managing multiple presence subscriptions
pub struct BuddyList {
    /// List of buddies and their watchers
    buddies: Vec<(String, PresenceWatcher)>,
    
    /// Reference to SimplePeer
    peer: Arc<SimplePeer>,
}

impl BuddyList {
    /// Create a new buddy list
    pub fn new(peer: Arc<SimplePeer>) -> Self {
        Self {
            buddies: Vec::new(),
            peer,
        }
    }
    
    /// Add a buddy to watch
    pub async fn add(&mut self, uri: &str) -> Result<()> {
        let watcher = self.peer.watch(uri).await?;
        self.buddies.push((uri.to_string(), watcher));
        Ok(())
    }
    
    /// Remove a buddy
    pub async fn remove(&mut self, uri: &str) -> Result<()> {
        if let Some(pos) = self.buddies.iter().position(|(u, _)| u == uri) {
            let (_, watcher) = self.buddies.remove(pos);
            watcher.stop().await?;
        }
        Ok(())
    }
    
    /// Get all buddy statuses
    pub async fn get_all(&self) -> Vec<(String, Option<PresenceInfo>)> {
        let mut statuses = Vec::new();
        for (uri, watcher) in &self.buddies {
            let status = watcher.current().await;
            statuses.push((uri.clone(), status));
        }
        statuses
    }
    
    /// Clear all buddies
    pub async fn clear(&mut self) -> Result<()> {
        for (_, watcher) in self.buddies.drain(..) {
            watcher.stop().await?;
        }
        Ok(())
    }
}

/// Extensions to SimplePeer for presence
impl SimplePeer {
    /// Set my presence status
    pub fn presence(&self, status: PresenceStatus) -> PresenceBuilder {
        PresenceBuilder::new(self, status)
    }
    
    /// Set status to available
    pub fn available(&self) -> PresenceBuilder {
        self.presence(PresenceStatus::Available)
    }
    
    /// Set status to busy
    pub fn busy(&self) -> PresenceBuilder {
        self.presence(PresenceStatus::Busy)
    }
    
    /// Set status to away
    pub fn away(&self) -> PresenceBuilder {
        self.presence(PresenceStatus::Away)
    }
    
    /// Set status to do not disturb
    pub fn dnd(&self) -> PresenceBuilder {
        self.presence(PresenceStatus::DoNotDisturb)
    }
    
    /// Set status to offline
    pub fn offline(&self) -> PresenceBuilder {
        self.presence(PresenceStatus::Offline)
    }
    
    /// Watch another user's presence
    pub async fn watch(&self, target: &str) -> Result<PresenceWatcher> {
        // Parse target URI
        let target_uri = if target.starts_with("sip:") {
            target.to_string()
        } else {
            format!("sip:{}", target)
        };
        
        // Create a channel for presence updates
        let (tx, rx) = mpsc::channel(10);
        
        // Create watcher
        let mut watcher = PresenceWatcher::new(
            target_uri.clone(),
            rx,
            self.coordinator.clone(),
        );
        
        // Send SUBSCRIBE to the target
        // TODO: Implement SUBSCRIBE sending through dialog-core
        // For now, we just set up the watcher
        info!("Watching presence for {}", target_uri);
        
        // In P2P mode, we might poll the target periodically
        // In B2BUA mode, we would send SUBSCRIBE to the server
        
        Ok(watcher)
    }
    
    /// Create a buddy list
    pub fn buddy_list(self: Arc<Self>) -> BuddyList {
        BuddyList::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_presence_builder() {
        // This would need a full SimplePeer setup to test properly
        // For now, just test the builder pattern
        
        // The actual test would look like:
        // let peer = SimplePeer::new("alice", 5060).await.unwrap();
        // peer.available()
        //     .note("Working from home")
        //     .device("laptop")
        //     .with_capability("video")
        //     .await
        //     .unwrap();
    }
}