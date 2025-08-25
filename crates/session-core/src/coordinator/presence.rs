//! Presence Coordinator
//!
//! This module manages SIP presence functionality including:
//! - Presence state storage and management
//! - Subscription tracking
//! - NOTIFY generation
//! - PUBLISH handling
//! - Watcher lists

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use dashmap::DashMap;
use tracing::{debug, info, warn, error};
use chrono::{DateTime, Utc};

use rvoip_dialog_core::DialogId;
use rvoip_sip_core::{
    Method, Request, Response, StatusCode,
    types::pidf::{Presence, BasicStatus, Status, Tuple},
    builder::SimpleRequestBuilder,
};

use crate::errors::{Result, SessionError};
use crate::dialog::DialogManager;

/// Presence status for a user
#[derive(Debug, Clone, PartialEq)]
pub enum PresenceStatus {
    /// User is available
    Available,
    /// User is busy
    Busy,
    /// User is away
    Away,
    /// User doesn't want to be disturbed
    DoNotDisturb,
    /// User is offline
    Offline,
    /// User is in a call
    InCall,
    /// Custom status
    Custom(String),
}

impl PresenceStatus {
    /// Convert to PIDF basic status
    pub fn to_basic_status(&self) -> BasicStatus {
        match self {
            PresenceStatus::Available | PresenceStatus::InCall => BasicStatus::Open,
            _ => BasicStatus::Closed,
        }
    }
    
    /// Get a human-readable note for this status
    pub fn to_note(&self) -> Option<String> {
        match self {
            PresenceStatus::Available => Some("Available".to_string()),
            PresenceStatus::Busy => Some("Busy".to_string()),
            PresenceStatus::Away => Some("Away".to_string()),
            PresenceStatus::DoNotDisturb => Some("Do not disturb".to_string()),
            PresenceStatus::Offline => Some("Offline".to_string()),
            PresenceStatus::InCall => Some("In a call".to_string()),
            PresenceStatus::Custom(note) => Some(note.clone()),
        }
    }
}

/// Rich presence information for a user
#[derive(Debug, Clone)]
pub struct PresenceInfo {
    /// Current presence status
    pub status: PresenceStatus,
    /// Optional status note
    pub note: Option<String>,
    /// Device identifier
    pub device: Option<String>,
    /// Location information
    pub location: Option<String>,
    /// Capabilities (e.g., ["audio", "video", "chat"])
    pub capabilities: Vec<String>,
    /// Last update timestamp
    pub last_updated: DateTime<Utc>,
    /// Entity URI (e.g., "sip:alice@example.com")
    pub entity: String,
}

impl PresenceInfo {
    /// Create new presence info
    pub fn new(entity: String, status: PresenceStatus) -> Self {
        Self {
            status,
            note: None,
            device: None,
            location: None,
            capabilities: vec!["audio".to_string()],
            last_updated: Utc::now(),
            entity,
        }
    }
    
    /// Set the note for this presence
    pub fn with_note(mut self, note: Option<String>) -> Self {
        self.note = note;
        self
    }
    
    /// Convert to PIDF format
    pub fn to_pidf(&self) -> Presence {
        // Create a tuple for this presence state
        let tuple_id = self.device.as_deref().unwrap_or("default");
        let mut tuple = Tuple::new(
            tuple_id.to_string(),
            Status::new(self.status.to_basic_status()),
        );
        
        // Set contact if we have device info
        if let Some(device) = &self.device {
            tuple.contact = Some(format!("sip:{}@{}", 
                self.entity.split('@').next().unwrap_or("unknown"),
                device
            ));
        }
        
        tuple.timestamp = Some(self.last_updated);
        
        // Build the presence document using the builder pattern
        let mut presence = Presence::new(self.entity.clone())
            .add_tuple(tuple);
        
        // Add note if present
        let note = self.note.clone().or_else(|| self.status.to_note());
        if let Some(n) = note {
            presence = presence.add_note(n);
        }
        
        presence
    }
}

/// Information about an active subscription
#[derive(Debug, Clone)]
pub struct SubscriptionInfo {
    /// Dialog ID for this subscription
    pub dialog_id: DialogId,
    /// Subscriber URI (watcher)
    pub watcher: String,
    /// Presentity URI (watched entity)
    pub presentity: String,
    /// Event package (usually "presence")
    pub event_package: String,
    /// Subscription expiry time
    pub expires_at: DateTime<Utc>,
    /// Subscription state
    pub state: SubscriptionState,
}

/// Subscription state
#[derive(Debug, Clone, PartialEq)]
pub enum SubscriptionState {
    /// Waiting for initial NOTIFY
    Pending,
    /// Active subscription
    Active,
    /// Subscription is terminating
    Terminating,
    /// Subscription has terminated
    Terminated,
}

/// Manages presence state and subscriptions
pub struct PresenceCoordinator {
    /// Presence states by entity URI
    presence_states: Arc<DashMap<String, PresenceInfo>>,
    
    /// Active subscriptions by dialog ID
    subscriptions: Arc<DashMap<DialogId, SubscriptionInfo>>,
    
    /// Watcher lists: presentity URI -> list of dialog IDs
    watchers: Arc<DashMap<String, Vec<DialogId>>>,
    
    /// Dialog manager for sending NOTIFY
    dialog_manager: Option<Arc<DialogManager>>,
    
    /// Default subscription duration
    default_expires: Duration,
}

impl PresenceCoordinator {
    /// Create a new presence coordinator
    pub fn new() -> Self {
        Self {
            presence_states: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            watchers: Arc::new(DashMap::new()),
            dialog_manager: None,
            default_expires: Duration::from_secs(3600),
        }
    }
    
    /// Set the dialog manager for NOTIFY sending
    pub fn set_dialog_manager(&mut self, dialog_manager: Arc<DialogManager>) {
        self.dialog_manager = Some(dialog_manager);
    }
    
    /// Update presence for an entity
    pub async fn update_presence(
        &self,
        entity: String,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        info!("Updating presence for {}: {:?}", entity, status);
        
        // Update or create presence info
        let mut presence = self.presence_states.entry(entity.clone())
            .or_insert_with(|| PresenceInfo::new(entity.clone(), status.clone()));
        
        presence.status = status;
        presence.note = note;
        presence.last_updated = Utc::now();
        
        // Notify all watchers
        self.notify_watchers(&entity).await?;
        
        Ok(())
    }
    
    /// Handle new subscription
    pub async fn handle_subscription(
        &self,
        dialog_id: DialogId,
        watcher: String,
        presentity: String,
        event_package: String,
        expires: Duration,
    ) -> Result<()> {
        info!("New subscription: {} watching {}", watcher, presentity);
        
        // Create subscription info
        let subscription = SubscriptionInfo {
            dialog_id: dialog_id.clone(),
            watcher,
            presentity: presentity.clone(),
            event_package,
            expires_at: Utc::now() + chrono::Duration::from_std(expires).unwrap(),
            state: SubscriptionState::Pending,
        };
        
        // Store subscription
        self.subscriptions.insert(dialog_id.clone(), subscription);
        
        // Add to watcher list
        self.watchers.entry(presentity.clone())
            .or_insert_with(Vec::new)
            .push(dialog_id.clone());
        
        // Send initial NOTIFY
        self.send_initial_notify(dialog_id).await?;
        
        Ok(())
    }
    
    /// Send initial NOTIFY for a subscription
    async fn send_initial_notify(&self, dialog_id: DialogId) -> Result<()> {
        let subscription = self.subscriptions.get(&dialog_id)
            .ok_or_else(|| SessionError::SessionNotFound("Subscription not found".to_string()))?;
        
        // Get current presence state
        let presence = self.presence_states.get(&subscription.presentity)
            .map(|p| p.clone())
            .unwrap_or_else(|| {
                PresenceInfo::new(subscription.presentity.clone(), PresenceStatus::Offline)
            });
        
        // Send NOTIFY with current state
        self.send_notify(dialog_id.clone(), presence, false).await?;
        
        // Mark subscription as active
        if let Some(mut sub) = self.subscriptions.get_mut(&dialog_id) {
            sub.state = SubscriptionState::Active;
        }
        
        Ok(())
    }
    
    /// Notify all watchers of a presentity
    async fn notify_watchers(&self, presentity: &str) -> Result<()> {
        if let Some(watchers) = self.watchers.get(presentity) {
            let presence = self.presence_states.get(presentity)
                .map(|p| p.clone())
                .unwrap_or_else(|| {
                    PresenceInfo::new(presentity.to_string(), PresenceStatus::Offline)
                });
            
            for dialog_id in watchers.iter() {
                if let Err(e) = self.send_notify(dialog_id.clone(), presence.clone(), false).await {
                    warn!("Failed to notify watcher {}: {}", dialog_id, e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Send NOTIFY message
    async fn send_notify(
        &self,
        dialog_id: DialogId,
        presence: PresenceInfo,
        is_terminating: bool,
    ) -> Result<()> {
        debug!("Sending NOTIFY for dialog {} (terminating: {})", dialog_id, is_terminating);
        
        // We need the dialog manager to send NOTIFY
        let dialog_manager = self.dialog_manager.as_ref()
            .ok_or_else(|| SessionError::ConfigError("Dialog manager not set".to_string()))?;
        
        // Generate PIDF body
        let pidf = presence.to_pidf();
        let body = pidf.to_xml();
        
        // Build subscription state
        let subscription_state = if is_terminating {
            "terminated".to_string()
        } else {
            // For active subscriptions, include expiry time
            if let Some(subscription) = self.subscriptions.get(&dialog_id) {
                let remaining = subscription.expires_at.signed_duration_since(Utc::now());
                format!("active;expires={}", remaining.num_seconds().max(0))
            } else {
                "active".to_string()
            }
        };
        
        // Send NOTIFY through dialog manager
        dialog_manager.send_notify(
            &dialog_id,
            "presence".to_string(),
            subscription_state,
            Some(body),
        ).await?;
        
        info!("Sent NOTIFY for dialog {} (terminating: {})", dialog_id, is_terminating);
        
        Ok(())
    }
    
    /// Terminate a subscription
    pub async fn terminate_subscription(
        &self,
        dialog_id: DialogId,
        reason: Option<String>,
    ) -> Result<()> {
        info!("Terminating subscription {}: {:?}", dialog_id, reason);
        
        if let Some(subscription) = self.subscriptions.get(&dialog_id) {
            // Get presence for final NOTIFY
            let presence = self.presence_states.get(&subscription.presentity)
                .map(|p| p.clone())
                .unwrap_or_else(|| {
                    PresenceInfo::new(subscription.presentity.clone(), PresenceStatus::Offline)
                });
            
            // Send terminating NOTIFY
            self.send_notify(dialog_id.clone(), presence, true).await?;
            
            // Remove from watcher list
            if let Some(mut watchers) = self.watchers.get_mut(&subscription.presentity) {
                watchers.retain(|id| id != &dialog_id);
            }
            
            // Remove subscription
            self.subscriptions.remove(&dialog_id);
        }
        
        Ok(())
    }
    
    /// Get current presence for an entity
    pub fn get_presence(&self, entity: &str) -> Option<PresenceInfo> {
        self.presence_states.get(entity).map(|p| p.clone())
    }
    
    /// Get all active subscriptions
    pub fn get_subscriptions(&self) -> Vec<SubscriptionInfo> {
        self.subscriptions.iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get all presence states
    pub fn get_all_presence(&self) -> Vec<PresenceInfo> {
        self.presence_states.iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Clean up expired subscriptions
    pub async fn cleanup_expired_subscriptions(&self) -> Result<()> {
        let now = Utc::now();
        let expired: Vec<DialogId> = self.subscriptions.iter()
            .filter(|entry| entry.expires_at < now)
            .map(|entry| entry.key().clone())
            .collect();
        
        for dialog_id in expired {
            self.terminate_subscription(dialog_id, Some("Subscription expired".to_string())).await?;
        }
        
        Ok(())
    }
}

impl Default for PresenceCoordinator {
    fn default() -> Self {
        Self::new()
    }
}