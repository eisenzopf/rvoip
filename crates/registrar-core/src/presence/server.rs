//! Presence server implementation

use std::sync::Arc;
use tracing::{debug, info};
use crate::types::{PresenceStatus, BuddyInfo};
use crate::error::Result;
use super::{PresenceStore, SubscriptionManager};

/// Presence server handling status updates and notifications
pub struct PresenceServer {
    store: Arc<PresenceStore>,
    subscriptions: Arc<SubscriptionManager>,
}

impl PresenceServer {
    pub fn new(
        store: Arc<PresenceStore>,
        subscriptions: Arc<SubscriptionManager>,
    ) -> Self {
        Self {
            store,
            subscriptions,
        }
    }
    
    /// Update user's presence and notify subscribers
    pub async fn update_presence(
        &self,
        user_id: &str,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        // Update store
        self.store.update_status(user_id, status, note).await?;
        
        // Notify subscribers
        let subscribers = self.subscriptions.get_subscribers(user_id).await?;
        for subscriber in subscribers {
            debug!("Notifying {} of {}'s presence change", subscriber, user_id);
            // Notification would be sent through session-core
        }
        
        info!("Updated presence for {}", user_id);
        Ok(())
    }
    
    /// Get buddy list with presence info
    pub async fn get_buddy_list(&self, user_id: &str) -> Result<Vec<BuddyInfo>> {
        // Get subscriptions for this user
        let watching = self.subscriptions.get_subscriptions(user_id).await?;
        let mut buddies = Vec::new();
        
        for buddy_id in watching {
            if let Ok(presence) = self.store.get_status(&buddy_id).await {
                buddies.push(BuddyInfo {
                    user_id: buddy_id.clone(),
                    display_name: Some(buddy_id.clone()),
                    status: presence.extended_status
                        .map(|s| PresenceStatus::from(s))
                        .unwrap_or(PresenceStatus::Offline),
                    note: presence.note,
                    last_updated: presence.last_updated,
                    is_online: presence.devices.len() > 0,
                    active_devices: presence.devices.len(),
                });
            }
        }
        
        Ok(buddies)
    }
}