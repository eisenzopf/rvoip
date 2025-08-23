//! User registration and location services

use std::sync::Arc;
use crate::types::{ContactInfo, UserRegistration};
use crate::error::Result;

pub mod registry;
pub mod location;
pub mod manager;

pub use registry::UserRegistry;
pub use location::LocationService;
pub use manager::RegistrationManager;

/// Main registrar interface combining all registration functionality
pub struct Registrar {
    registry: Arc<UserRegistry>,
    location: Arc<LocationService>,
    manager: Arc<RegistrationManager>,
}

impl Registrar {
    /// Create a new registrar instance
    pub fn new() -> Self {
        let registry = Arc::new(UserRegistry::new());
        let location = Arc::new(LocationService::new());
        let manager = Arc::new(RegistrationManager::new(registry.clone()));
        
        Self {
            registry,
            location,
            manager,
        }
    }
    
    /// Register a user with contact information
    pub async fn register_user(
        &self,
        user_id: &str,
        contact: ContactInfo,
        expires: u32,
    ) -> Result<()> {
        self.registry.register(user_id, contact, expires).await
    }
    
    /// Unregister a user completely
    pub async fn unregister_user(&self, user_id: &str) -> Result<()> {
        self.registry.unregister(user_id).await
    }
    
    /// Unregister a specific contact
    pub async fn unregister_contact(&self, user_id: &str, contact_uri: &str) -> Result<()> {
        self.registry.remove_contact(user_id, contact_uri).await
    }
    
    /// Lookup user's contact locations
    pub async fn lookup_user(&self, user_id: &str) -> Result<Vec<ContactInfo>> {
        self.location.find_contacts(user_id).await
    }
    
    /// Refresh registration (update expiry)
    pub async fn refresh_registration(
        &self,
        user_id: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<()> {
        self.registry.refresh(user_id, contact_uri, expires).await
    }
    
    /// Get all registered users
    pub async fn list_users(&self) -> Vec<String> {
        self.registry.list_all_users().await
    }
    
    /// Check if a user is registered
    pub async fn is_registered(&self, user_id: &str) -> bool {
        self.registry.is_registered(user_id).await
    }
    
    /// Get registration info for a user
    pub async fn get_registration(&self, user_id: &str) -> Result<UserRegistration> {
        self.registry.get_registration(user_id).await
    }
    
    /// Start background expiry management
    pub async fn start_expiry_manager(&self) {
        self.manager.start().await
    }
    
    /// Stop background expiry management
    pub async fn stop_expiry_manager(&self) {
        self.manager.stop().await
    }
}