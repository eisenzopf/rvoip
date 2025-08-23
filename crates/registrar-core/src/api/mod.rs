//! High-level API for session-core integration

use std::sync::Arc;
use tracing::{info, debug, warn};
#[allow(unused_imports)] // EventPublisher trait is needed for .publish() method
use infra_common::events::api::{EventSystem as EventSystemTrait, EventPublisher as _};

use crate::registrar::Registrar;
use crate::presence::Presence;
use crate::types::{
    ContactInfo, PresenceStatus, PresenceState, BuddyInfo, 
    RegistrarConfig,
};
use crate::error::Result;
use crate::events::{RegistrarEvent, PresenceEvent};

/// High-level registrar service for session-core integration
pub struct RegistrarService {
    /// User registration management
    registrar: Arc<Registrar>,
    
    /// Presence management
    presence: Arc<Presence>,
    
    /// Configuration
    config: Arc<RegistrarConfig>,
    
    /// Event bus for publishing events
    event_bus: Option<Arc<infra_common::events::system::EventSystem>>,
    
    /// Service mode
    mode: ServiceMode,
}

/// Service operation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceMode {
    /// P2P mode - minimal features, no auto-buddy lists
    P2P,
    /// B2BUA mode - full features with auto-buddy lists
    B2BUA,
}

impl RegistrarService {
    /// Create a new registrar service for P2P mode
    pub async fn new_p2p() -> Result<Self> {
        Self::new_with_mode(ServiceMode::P2P, RegistrarConfig::default()).await
    }
    
    /// Create a new registrar service for B2BUA mode
    pub async fn new_b2bua() -> Result<Self> {
        let mut config = RegistrarConfig::default();
        config.auto_buddy_lists = true;
        config.default_presence_enabled = true;
        
        Self::new_with_mode(ServiceMode::B2BUA, config).await
    }
    
    /// Create with specific mode and configuration
    pub async fn new_with_mode(mode: ServiceMode, config: RegistrarConfig) -> Result<Self> {
        let registrar = Arc::new(Registrar::new());
        let presence = Arc::new(Presence::new());
        
        // Start background tasks
        registrar.start_expiry_manager().await;
        
        info!("RegistrarService started in {:?} mode", mode);
        
        Ok(Self {
            registrar,
            presence,
            config: Arc::new(config),
            event_bus: None,
            mode,
        })
    }
    
    /// Set the event bus for publishing events
    pub fn set_event_bus(&mut self, event_bus: Arc<infra_common::events::system::EventSystem>) {
        self.event_bus = Some(event_bus);
    }
    
    // ========== Registration Methods ==========
    
    /// Register a user with contact information
    /// 
    /// Called when session-core receives a REGISTER request
    pub async fn register_user(
        &self,
        user_id: &str,
        contact: ContactInfo,
        expires: Option<u32>,
    ) -> Result<()> {
        let expires = expires.unwrap_or(self.config.default_expires);
        
        // Register the user
        self.registrar.register_user(user_id, contact.clone(), expires).await?;
        
        // In B2BUA mode, set up automatic buddy lists
        if self.mode == ServiceMode::B2BUA && self.config.auto_buddy_lists {
            self.setup_auto_buddy_list(user_id).await?;
        }
        
        // Publish event
        self.publish_event(RegistrarEvent::UserRegistered {
            user: user_id.to_string(),
            contact,
        }).await;
        
        info!("User {} registered", user_id);
        Ok(())
    }
    
    /// Unregister a user
    /// 
    /// Called when session-core receives REGISTER with Expires: 0
    pub async fn unregister_user(&self, user_id: &str) -> Result<()> {
        // Clear presence
        self.presence.clear_presence(user_id).await?;
        
        // Remove registrations
        self.registrar.unregister_user(user_id).await?;
        
        // Publish event
        self.publish_event(RegistrarEvent::UserUnregistered {
            user: user_id.to_string(),
        }).await;
        
        info!("User {} unregistered", user_id);
        Ok(())
    }
    
    /// Lookup where a user can be reached
    /// 
    /// Called when session-core needs to route an INVITE
    pub async fn lookup_user(&self, user_id: &str) -> Result<Vec<ContactInfo>> {
        self.registrar.lookup_user(user_id).await
    }
    
    /// Get all registered users
    pub async fn list_registered_users(&self) -> Vec<String> {
        self.registrar.list_users().await
    }
    
    /// Check if a user is registered
    pub async fn is_registered(&self, user_id: &str) -> bool {
        self.registrar.is_registered(user_id).await
    }
    
    // ========== Presence Methods ==========
    
    /// Update user's presence status
    /// 
    /// Called when session-core receives a PUBLISH request
    pub async fn update_presence(
        &self,
        user_id: &str,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        // Update presence
        self.presence.update_presence(user_id, status.clone(), note.clone()).await?;
        
        // Notify watchers
        let notified = self.presence.notify_subscribers(user_id).await?;
        
        // Publish event
        self.publish_event(PresenceEvent::Updated {
            user: user_id.to_string(),
            status,
            note,
            watchers_notified: notified.len(),
        }).await;
        
        debug!("Presence updated for {} ({} watchers notified)", user_id, notified.len());
        Ok(())
    }
    
    /// Get user's current presence
    pub async fn get_presence(&self, user_id: &str) -> Result<PresenceState> {
        self.presence.get_presence(user_id).await
    }
    
    /// Subscribe to a user's presence
    /// 
    /// Called when session-core receives a SUBSCRIBE request
    pub async fn subscribe_presence(
        &self,
        subscriber: &str,
        target: &str,
        expires: Option<u32>,
    ) -> Result<String> {
        let expires = expires.unwrap_or(self.config.default_expires);
        
        let subscription_id = self.presence.subscribe(subscriber, target, expires).await?;
        
        // Publish event
        self.publish_event(PresenceEvent::Subscribed {
            subscriber: subscriber.to_string(),
            target: target.to_string(),
            subscription_id: subscription_id.clone(),
        }).await;
        
        debug!("{} subscribed to {}'s presence", subscriber, target);
        Ok(subscription_id)
    }
    
    /// Unsubscribe from presence
    pub async fn unsubscribe_presence(&self, subscription_id: &str) -> Result<()> {
        self.presence.unsubscribe(subscription_id).await?;
        
        // Publish event
        self.publish_event(PresenceEvent::Unsubscribed {
            subscription_id: subscription_id.to_string(),
        }).await;
        
        Ok(())
    }
    
    /// Get buddy list for a user
    /// 
    /// In B2BUA mode, returns all registered users with their presence
    pub async fn get_buddy_list(&self, user_id: &str) -> Result<Vec<BuddyInfo>> {
        if self.mode == ServiceMode::B2BUA {
            // In B2BUA mode, all registered users are buddies
            let users = self.registrar.list_users().await;
            let mut buddies = Vec::new();
            
            for buddy_id in users {
                if buddy_id != user_id {
                    // Get presence if available
                    let presence = self.presence.get_presence(&buddy_id).await.ok();
                    
                    buddies.push(BuddyInfo {
                        user_id: buddy_id.clone(),
                        display_name: Some(buddy_id.clone()),
                        status: presence.as_ref()
                            .and_then(|p| p.extended_status.as_ref())
                            .map(|s| PresenceStatus::from(s.clone()))
                            .unwrap_or(PresenceStatus::Offline),
                        note: presence.as_ref().and_then(|p| p.note.clone()),
                        last_updated: presence.as_ref()
                            .map(|p| p.last_updated)
                            .unwrap_or_else(chrono::Utc::now),
                        is_online: presence.is_some(),
                        active_devices: presence.as_ref()
                            .map(|p| p.devices.len())
                            .unwrap_or(0),
                    });
                }
            }
            
            Ok(buddies)
        } else {
            // In P2P mode, use explicit buddy list
            self.presence.get_buddy_list(user_id).await
        }
    }
    
    /// Generate PIDF XML for a user's presence
    /// 
    /// Used when session-core needs to send NOTIFY
    pub async fn generate_pidf(&self, user_id: &str) -> Result<String> {
        self.presence.generate_pidf(user_id).await
    }
    
    /// Parse PIDF XML
    /// 
    /// Used when session-core receives PUBLISH
    pub async fn parse_pidf(&self, xml: &str) -> Result<PresenceState> {
        self.presence.parse_pidf(xml).await
    }
    
    // ========== Internal Methods ==========
    
    /// Set up automatic buddy list for a newly registered user
    async fn setup_auto_buddy_list(&self, user_id: &str) -> Result<()> {
        if self.mode != ServiceMode::B2BUA || !self.config.auto_buddy_lists {
            return Ok(());
        }
        
        // Get all other registered users
        let all_users = self.registrar.list_users().await;
        
        for other_user in all_users {
            if other_user != user_id {
                // Create bidirectional subscriptions
                let _ = self.presence.subscribe(user_id, &other_user, self.config.default_expires).await;
                let _ = self.presence.subscribe(&other_user, user_id, self.config.default_expires).await;
            }
        }
        
        debug!("Auto buddy list set up for {}", user_id);
        Ok(())
    }
    
    /// Publish an event to the event bus
    async fn publish_event<E>(&self, event: E) 
    where
        E: infra_common::events::types::Event + std::fmt::Debug + 'static,
    {
        if let Some(bus) = &self.event_bus {
            let publisher = bus.create_publisher::<E>();
            if let Err(e) = publisher.publish(event).await {
                warn!("Failed to publish event: {:?}", e);
            }
        }
    }
    
    /// Shutdown the service
    pub async fn shutdown(&self) -> Result<()> {
        self.registrar.stop_expiry_manager().await;
        info!("RegistrarService shutdown");
        Ok(())
    }
}

// Conversion helpers
impl From<ExtendedStatus> for PresenceStatus {
    fn from(status: ExtendedStatus) -> Self {
        use crate::types::ExtendedStatus;
        match status {
            ExtendedStatus::Available => PresenceStatus::Available,
            ExtendedStatus::Away => PresenceStatus::Away,
            ExtendedStatus::Busy => PresenceStatus::Busy,
            ExtendedStatus::DoNotDisturb => PresenceStatus::DoNotDisturb,
            ExtendedStatus::OnThePhone => PresenceStatus::InCall,
            ExtendedStatus::Offline => PresenceStatus::Offline,
            ExtendedStatus::InMeeting => PresenceStatus::Busy,
            ExtendedStatus::Custom(s) => PresenceStatus::Custom(s),
        }
    }
}

use crate::types::ExtendedStatus;