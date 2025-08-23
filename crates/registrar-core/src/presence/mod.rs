//! Presence management and subscription handling

use std::sync::Arc;
use crate::types::{PresenceState, PresenceStatus, BuddyInfo};
use crate::error::Result;

pub mod server;
pub mod store;
pub mod subscription;
pub mod pidf;

pub use server::PresenceServer;
pub use store::PresenceStore;
pub use subscription::SubscriptionManager;
pub use pidf::PidfGenerator;

/// Main presence interface combining all presence functionality
pub struct Presence {
    server: Arc<PresenceServer>,
    store: Arc<PresenceStore>,
    subscriptions: Arc<SubscriptionManager>,
    pidf: Arc<PidfGenerator>,
}

impl Presence {
    /// Create a new presence instance
    pub fn new() -> Self {
        let store = Arc::new(PresenceStore::new());
        let subscriptions = Arc::new(SubscriptionManager::new());
        let pidf = Arc::new(PidfGenerator::new());
        let server = Arc::new(PresenceServer::new(
            store.clone(),
            subscriptions.clone(),
        ));
        
        Self {
            server,
            store,
            subscriptions,
            pidf,
        }
    }
    
    /// Update user's presence status
    pub async fn update_presence(
        &self,
        user_id: &str,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        self.server.update_presence(user_id, status, note).await
    }
    
    /// Get user's current presence
    pub async fn get_presence(&self, user_id: &str) -> Result<PresenceState> {
        self.store.get_status(user_id).await
    }
    
    /// Subscribe to a user's presence
    pub async fn subscribe(
        &self,
        subscriber: &str,
        target: &str,
        expires: u32,
    ) -> Result<String> {
        self.subscriptions.add_subscription(subscriber, target, expires).await
    }
    
    /// Unsubscribe from presence
    pub async fn unsubscribe(&self, subscription_id: &str) -> Result<()> {
        self.subscriptions.remove_subscription(subscription_id).await
    }
    
    /// Get all subscribers watching a user
    pub async fn get_watchers(&self, user_id: &str) -> Result<Vec<String>> {
        self.subscriptions.get_subscribers(user_id).await
    }
    
    /// Get all users that a subscriber is watching
    pub async fn get_watching(&self, subscriber: &str) -> Result<Vec<String>> {
        self.subscriptions.get_subscriptions(subscriber).await
    }
    
    /// Get buddy list for a user
    pub async fn get_buddy_list(&self, user_id: &str) -> Result<Vec<BuddyInfo>> {
        self.server.get_buddy_list(user_id).await
    }
    
    /// Generate PIDF document for user's presence
    pub async fn generate_pidf(&self, user_id: &str) -> Result<String> {
        let presence = self.store.get_status(user_id).await?;
        self.pidf.create_pidf(&presence).await
    }
    
    /// Parse PIDF document
    pub async fn parse_pidf(&self, xml: &str) -> Result<PresenceState> {
        self.pidf.parse_pidf(xml).await
    }
    
    /// Notify all subscribers of presence change
    pub async fn notify_subscribers(&self, user_id: &str) -> Result<Vec<String>> {
        self.subscriptions.notify_subscribers(user_id).await
    }
    
    /// Remove all presence data for a user
    pub async fn clear_presence(&self, user_id: &str) -> Result<()> {
        self.store.clear_presence(user_id).await
    }
    
    /// Expire old subscriptions
    pub async fn expire_subscriptions(&self) -> Vec<String> {
        self.subscriptions.expire_subscriptions().await
    }
}