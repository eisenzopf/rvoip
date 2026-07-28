//! Presence server implementation

use super::{PresenceStore, SubscriptionManager};
use crate::error::Result;
use crate::types::{BuddyInfo, PresenceStatus};
use std::sync::Arc;
use tracing::{debug, info};

/// Presence server handling status updates and notifications
pub struct PresenceServer {
    store: Arc<PresenceStore>,
    subscriptions: Arc<SubscriptionManager>,
}

impl PresenceServer {
    pub fn new(store: Arc<PresenceStore>, subscriptions: Arc<SubscriptionManager>) -> Self {
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
            debug!(
                stage = "presence-notify",
                subscriber_present = !subscriber.is_empty(),
                subscriber_bytes = subscriber.len(),
                target_present = !user_id.is_empty(),
                target_bytes = user_id.len(),
                "Presence notification scheduled"
            );
            // Notification would be sent through session-core
        }

        info!(
            stage = "presence-update",
            user_present = !user_id.is_empty(),
            user_bytes = user_id.len(),
            "Presence state updated"
        );
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
                    status: presence
                        .extended_status
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

#[cfg(test)]
mod diagnostic_source_tests {
    #[test]
    fn presence_server_logs_only_structural_identity_metadata() {
        let source = include_str!("server.rs");

        for fragments in [
            ["Notifying {}", " of {}'s presence change"],
            ["Updated presence", " for {}"],
        ] {
            let forbidden = fragments.concat();
            assert!(
                !source.contains(&forbidden),
                "presence server regained value-bearing diagnostic: {forbidden}"
            );
        }

        for required in [
            "stage = \"presence-notify\"",
            "stage = \"presence-update\"",
            "subscriber_present",
            "subscriber_bytes",
            "target_present",
            "target_bytes",
            "user_present",
            "user_bytes",
        ] {
            assert!(
                source.contains(required),
                "presence diagnostic lost structural field: {required}"
            );
        }
    }
}
