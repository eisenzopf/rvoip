//! Presence data storage

use dashmap::DashMap;
use std::sync::Arc;
use chrono::Utc;
use crate::types::{PresenceState, PresenceStatus, ExtendedStatus, DevicePresence, BasicStatus};
use crate::error::{RegistrarError, Result};

/// In-memory presence store
pub struct PresenceStore {
    /// Map of user_id to presence state
    presence: Arc<DashMap<String, PresenceState>>,
}

impl PresenceStore {
    pub fn new() -> Self {
        Self {
            presence: Arc::new(DashMap::new()),
        }
    }
    
    /// Update user's presence status
    pub async fn update_status(
        &self,
        user_id: &str,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        let extended = match status {
            PresenceStatus::Available => ExtendedStatus::Available,
            PresenceStatus::Away => ExtendedStatus::Away,
            PresenceStatus::Busy => ExtendedStatus::Busy,
            PresenceStatus::DoNotDisturb => ExtendedStatus::DoNotDisturb,
            PresenceStatus::InCall => ExtendedStatus::OnThePhone,
            PresenceStatus::Offline => ExtendedStatus::Offline,
            PresenceStatus::Custom(s) => ExtendedStatus::Custom(s),
        };
        
        self.presence
            .entry(user_id.to_string())
            .and_modify(|state| {
                state.extended_status = Some(extended.clone());
                state.note = note.clone();
                state.last_updated = Utc::now();
            })
            .or_insert(PresenceState {
                user_id: user_id.to_string(),
                basic_status: BasicStatus::Open,
                extended_status: Some(extended),
                note,
                activities: Vec::new(),
                devices: Vec::new(),
                last_updated: Utc::now(),
                expires: None,
                priority: 0,
            });
        
        Ok(())
    }
    
    /// Get user's presence state
    pub async fn get_status(&self, user_id: &str) -> Result<PresenceState> {
        self.presence
            .get(user_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| RegistrarError::UserNotFound(user_id.to_string()))
    }
    
    /// Add device to user's presence
    pub async fn add_device(&self, user_id: &str, device: DevicePresence) -> Result<()> {
        self.presence
            .entry(user_id.to_string())
            .and_modify(|state| {
                // Remove duplicate device if exists
                state.devices.retain(|d| d.instance_id != device.instance_id);
                state.devices.push(device.clone());
                state.last_updated = Utc::now();
            })
            .or_insert(PresenceState {
                user_id: user_id.to_string(),
                basic_status: BasicStatus::Open,
                extended_status: Some(ExtendedStatus::Available),
                note: None,
                activities: Vec::new(),
                devices: vec![device],
                last_updated: Utc::now(),
                expires: None,
                priority: 0,
            });
        
        Ok(())
    }
    
    /// Remove device from user's presence
    pub async fn remove_device(&self, user_id: &str, device_id: &str) -> Result<()> {
        if let Some(mut entry) = self.presence.get_mut(user_id) {
            entry.devices.retain(|d| d.instance_id != device_id);
            entry.last_updated = Utc::now();
            
            // If no devices left, mark as offline
            if entry.devices.is_empty() {
                entry.extended_status = Some(ExtendedStatus::Offline);
                entry.basic_status = BasicStatus::Closed;
            }
        }
        Ok(())
    }
    
    /// Clear all presence data for a user
    pub async fn clear_presence(&self, user_id: &str) -> Result<()> {
        self.presence.remove(user_id);
        Ok(())
    }
    
    /// List all users with presence
    pub async fn list_users_with_presence(&self) -> Vec<String> {
        self.presence
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
}