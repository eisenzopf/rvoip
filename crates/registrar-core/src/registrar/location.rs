//! Location service for mapping users to contact addresses

use dashmap::DashMap;
use std::sync::Arc;
use crate::types::ContactInfo;
use crate::error::{RegistrarError, Result};

/// Location service for finding where users can be reached
pub struct LocationService {
    /// Map of user_id to list of contacts
    locations: Arc<DashMap<String, Vec<ContactInfo>>>,
}

impl LocationService {
    pub fn new() -> Self {
        Self {
            locations: Arc::new(DashMap::new()),
        }
    }
    
    /// Add a binding between user and contact
    pub async fn add_binding(&self, user_id: &str, contact: ContactInfo) -> Result<()> {
        self.locations
            .entry(user_id.to_string())
            .and_modify(|contacts| {
                // Remove duplicate URI if exists
                contacts.retain(|c| c.uri != contact.uri);
                contacts.push(contact.clone());
            })
            .or_insert(vec![contact]);
        
        Ok(())
    }
    
    /// Remove a binding
    pub async fn remove_binding(&self, user_id: &str, contact_uri: &str) -> Result<()> {
        if let Some(mut entry) = self.locations.get_mut(user_id) {
            entry.retain(|c| c.uri != contact_uri);
            if entry.is_empty() {
                drop(entry);
                self.locations.remove(user_id);
            }
            Ok(())
        } else {
            Err(RegistrarError::UserNotFound(user_id.to_string()))
        }
    }
    
    /// Find all contacts for a user
    pub async fn find_contacts(&self, user_id: &str) -> Result<Vec<ContactInfo>> {
        self.locations
            .get(user_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| RegistrarError::UserNotFound(user_id.to_string()))
    }
    
    /// Reverse lookup - find user by contact URI
    pub async fn find_user(&self, contact_uri: &str) -> Option<String> {
        for entry in self.locations.iter() {
            if entry.value().iter().any(|c| c.uri == contact_uri) {
                return Some(entry.key().clone());
            }
        }
        None
    }
    
    /// Clear all locations for a user
    pub async fn clear_user(&self, user_id: &str) -> Result<()> {
        self.locations.remove(user_id);
        Ok(())
    }
}