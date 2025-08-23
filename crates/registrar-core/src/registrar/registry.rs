//! User registry for managing registrations

use std::sync::Arc;
use dashmap::DashMap;
use chrono::{DateTime, Duration, Utc};
use tracing::{debug, info, warn};
use crate::types::{UserRegistration, ContactInfo};
use crate::error::{RegistrarError, Result};

/// Thread-safe user registry
pub struct UserRegistry {
    /// Map of user_id to registration information
    users: Arc<DashMap<String, UserRegistration>>,
    
    /// Configuration
    config: RegistryConfig,
}

#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub max_contacts_per_user: usize,
    pub default_expires: u32,
    pub max_expires: u32,
    pub min_expires: u32,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            max_contacts_per_user: 10,
            default_expires: 3600,
            max_expires: 86400,
            min_expires: 60,
        }
    }
}

impl UserRegistry {
    /// Create a new user registry
    pub fn new() -> Self {
        Self::with_config(RegistryConfig::default())
    }
    
    /// Create with custom configuration
    pub fn with_config(config: RegistryConfig) -> Self {
        Self {
            users: Arc::new(DashMap::new()),
            config,
        }
    }
    
    /// Register a user with contact information
    pub async fn register(
        &self,
        user_id: &str,
        contact: ContactInfo,
        expires: u32,
    ) -> Result<()> {
        let expires = self.validate_expires(expires)?;
        let expires_at = Utc::now() + Duration::seconds(expires as i64);
        
        let contact_uri = contact.uri.clone();
        
        self.users
            .entry(user_id.to_string())
            .and_modify(|reg| {
                // Update existing registration
                self.update_contact(reg, contact.clone(), expires_at);
            })
            .or_insert_with(|| {
                // Create new registration
                UserRegistration {
                    user_id: user_id.to_string(),
                    contacts: vec![contact],
                    expires: expires_at,
                    presence_enabled: true,
                    capabilities: vec!["presence".to_string()],
                    registered_at: Utc::now(),
                    attributes: Default::default(),
                }
            });
        
        info!("User {} registered with contact {}", user_id, contact_uri);
        Ok(())
    }
    
    /// Unregister a user completely
    pub async fn unregister(&self, user_id: &str) -> Result<()> {
        if self.users.remove(user_id).is_some() {
            info!("User {} unregistered", user_id);
            Ok(())
        } else {
            Err(RegistrarError::UserNotFound(user_id.to_string()))
        }
    }
    
    /// Remove a specific contact for a user
    pub async fn remove_contact(&self, user_id: &str, contact_uri: &str) -> Result<()> {
        let mut entry = self.users
            .get_mut(user_id)
            .ok_or_else(|| RegistrarError::UserNotFound(user_id.to_string()))?;
        
        let initial_count = entry.contacts.len();
        entry.contacts.retain(|c| c.uri != contact_uri);
        
        if entry.contacts.len() == initial_count {
            return Err(RegistrarError::ContactNotFound {
                user: user_id.to_string(),
                uri: contact_uri.to_string(),
            });
        }
        
        // If no contacts left, remove the user
        if entry.contacts.is_empty() {
            drop(entry);
            self.users.remove(user_id);
            info!("User {} unregistered (no contacts remaining)", user_id);
        }
        
        Ok(())
    }
    
    /// Refresh registration expiry
    pub async fn refresh(
        &self,
        user_id: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<()> {
        let expires = self.validate_expires(expires)?;
        let expires_at = Utc::now() + Duration::seconds(expires as i64);
        
        let mut entry = self.users
            .get_mut(user_id)
            .ok_or_else(|| RegistrarError::UserNotFound(user_id.to_string()))?;
        
        let contact = entry.contacts
            .iter_mut()
            .find(|c| c.uri == contact_uri)
            .ok_or_else(|| RegistrarError::ContactNotFound {
                user: user_id.to_string(),
                uri: contact_uri.to_string(),
            })?;
        
        contact.expires = expires_at;
        entry.expires = expires_at;
        
        debug!("Refreshed registration for {}:{}", user_id, contact_uri);
        Ok(())
    }
    
    /// Get registration information for a user
    pub async fn get_registration(&self, user_id: &str) -> Result<UserRegistration> {
        self.users
            .get(user_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| RegistrarError::UserNotFound(user_id.to_string()))
    }
    
    /// Check if a user is registered
    pub async fn is_registered(&self, user_id: &str) -> bool {
        self.users.contains_key(user_id)
    }
    
    /// List all registered users
    pub async fn list_all_users(&self) -> Vec<String> {
        self.users
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
    
    /// Get all registrations (for admin/debugging)
    pub async fn get_all_registrations(&self) -> Vec<UserRegistration> {
        self.users
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Remove expired registrations
    pub async fn expire_registrations(&self) -> Vec<String> {
        let now = Utc::now();
        let mut expired_users = Vec::new();
        
        // Find expired registrations
        let to_remove: Vec<String> = self.users
            .iter()
            .filter(|entry| entry.expires < now)
            .map(|entry| entry.key().clone())
            .collect();
        
        // Remove them
        for user_id in to_remove {
            if self.users.remove(&user_id).is_some() {
                warn!("Registration expired for user: {}", user_id);
                expired_users.push(user_id);
            }
        }
        
        expired_users
    }
    
    /// Update or add a contact to existing registration
    fn update_contact(
        &self,
        registration: &mut UserRegistration,
        contact: ContactInfo,
        expires_at: DateTime<Utc>,
    ) {
        // Remove existing contact with same URI
        registration.contacts.retain(|c| c.uri != contact.uri);
        
        // Add new/updated contact
        registration.contacts.push(contact);
        
        // Update overall expiry to latest
        if expires_at > registration.expires {
            registration.expires = expires_at;
        }
    }
    
    /// Validate and normalize expires value
    fn validate_expires(&self, expires: u32) -> Result<u32> {
        if expires == 0 {
            // 0 means unregister
            return Ok(0);
        }
        
        if expires < self.config.min_expires {
            return Ok(self.config.min_expires);
        }
        
        if expires > self.config.max_expires {
            return Ok(self.config.max_expires);
        }
        
        Ok(expires)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_user_registration() {
        let registry = UserRegistry::new();
        
        let contact = ContactInfo {
            uri: "sip:alice@192.168.1.100:5060".to_string(),
            instance_id: "device-1".to_string(),
            transport: crate::types::Transport::UDP,
            user_agent: "Test UA".to_string(),
            expires: Utc::now() + Duration::hours(1),
            q_value: 1.0,
            received: None,
            path: vec![],
            methods: vec!["INVITE".to_string(), "MESSAGE".to_string()],
        };
        
        // Register user
        registry.register("alice", contact.clone(), 3600).await.unwrap();
        
        // Check registration
        assert!(registry.is_registered("alice").await);
        
        let reg = registry.get_registration("alice").await.unwrap();
        assert_eq!(reg.user_id, "alice");
        assert_eq!(reg.contacts.len(), 1);
        assert_eq!(reg.contacts[0].uri, contact.uri);
    }
    
    #[tokio::test]
    async fn test_unregister() {
        let registry = UserRegistry::new();
        
        let contact = ContactInfo {
            uri: "sip:bob@example.com".to_string(),
            instance_id: "device-1".to_string(),
            transport: crate::types::Transport::TCP,
            user_agent: "Test".to_string(),
            expires: Utc::now() + Duration::hours(1),
            q_value: 1.0,
            received: None,
            path: vec![],
            methods: vec![],
        };
        
        registry.register("bob", contact, 3600).await.unwrap();
        assert!(registry.is_registered("bob").await);
        
        registry.unregister("bob").await.unwrap();
        assert!(!registry.is_registered("bob").await);
    }
}