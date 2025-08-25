//! Presence aggregation for multiple devices
//!
//! Handles presence state aggregation when a user has multiple devices
//! or instances registered with different presence states.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};
use dashmap::DashMap;

use crate::coordinator::presence::{PresenceStatus, PresenceInfo};
use crate::errors::{Result, SessionError};

/// Represents a single device/instance for a user
#[derive(Debug, Clone)]
pub struct DevicePresence {
    /// Unique device/instance identifier
    pub device_id: String,
    
    /// Device type (e.g., "mobile", "desktop", "web")
    pub device_type: Option<String>,
    
    /// Current presence status for this device
    pub status: PresenceStatus,
    
    /// Optional presence note
    pub note: Option<String>,
    
    /// Device capabilities (e.g., ["audio", "video", "chat"])
    pub capabilities: Vec<String>,
    
    /// Last update time
    pub last_updated: DateTime<Utc>,
    
    /// Priority for this device (higher wins in aggregation)
    pub priority: i32,
}

impl DevicePresence {
    /// Create a new device presence entry
    pub fn new(device_id: String, status: PresenceStatus) -> Self {
        Self {
            device_id,
            device_type: None,
            status,
            note: None,
            capabilities: vec!["audio".to_string()],
            last_updated: Utc::now(),
            priority: 0,
        }
    }
    
    /// Set the device type
    pub fn with_type(mut self, device_type: String) -> Self {
        self.device_type = Some(device_type);
        self
    }
    
    /// Set the priority
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
    
    /// Update the presence status
    pub fn update(&mut self, status: PresenceStatus, note: Option<String>) {
        self.status = status;
        self.note = note;
        self.last_updated = Utc::now();
    }
}

/// Aggregation strategy for multiple device presence
#[derive(Debug, Clone, PartialEq)]
pub enum AggregationStrategy {
    /// Most available wins (Available > Away > Busy > DND > Offline)
    MostAvailable,
    
    /// Most recent update wins
    MostRecent,
    
    /// Highest priority device wins
    HighestPriority,
    
    /// Custom rules-based aggregation
    Custom,
}

impl Default for AggregationStrategy {
    fn default() -> Self {
        Self::MostAvailable
    }
}

/// Rules for custom aggregation
#[derive(Debug, Clone)]
pub struct AggregationRules {
    /// Prefer specific device types (in order)
    pub preferred_device_types: Vec<String>,
    
    /// Ignore devices offline for longer than this duration
    pub offline_timeout: chrono::Duration,
    
    /// Merge notes from all devices
    pub merge_notes: bool,
    
    /// Combine capabilities from all online devices
    pub combine_capabilities: bool,
}

impl Default for AggregationRules {
    fn default() -> Self {
        Self {
            preferred_device_types: vec!["desktop".to_string(), "mobile".to_string(), "web".to_string()],
            offline_timeout: chrono::Duration::hours(1),
            merge_notes: false,
            combine_capabilities: true,
        }
    }
}

/// Manages presence aggregation for multiple devices
pub struct PresenceAggregator {
    /// User devices: user_uri -> (device_id -> DevicePresence)
    user_devices: Arc<DashMap<String, HashMap<String, DevicePresence>>>,
    
    /// Aggregation strategy
    strategy: AggregationStrategy,
    
    /// Custom aggregation rules
    rules: AggregationRules,
    
    /// Cached aggregated presence for each user
    aggregated_cache: Arc<DashMap<String, PresenceInfo>>,
}

impl PresenceAggregator {
    /// Create a new presence aggregator
    pub fn new(strategy: AggregationStrategy) -> Self {
        Self {
            user_devices: Arc::new(DashMap::new()),
            strategy,
            rules: AggregationRules::default(),
            aggregated_cache: Arc::new(DashMap::new()),
        }
    }
    
    /// Set custom aggregation rules
    pub fn with_rules(mut self, rules: AggregationRules) -> Self {
        self.rules = rules;
        self
    }
    
    /// Update presence for a specific device
    pub fn update_device_presence(
        &self,
        user_uri: &str,
        device_id: &str,
        status: PresenceStatus,
        note: Option<String>,
        device_type: Option<String>,
    ) -> Result<()> {
        let mut devices = self.user_devices
            .entry(user_uri.to_string())
            .or_insert_with(HashMap::new);
        
        if let Some(device) = devices.get_mut(device_id) {
            device.update(status.clone(), note);
        } else {
            let mut device = DevicePresence::new(device_id.to_string(), status.clone());
            device.note = note;
            device.device_type = device_type;
            devices.insert(device_id.to_string(), device);
        }
        
        // Invalidate cache
        self.aggregated_cache.remove(user_uri);
        
        debug!("Updated presence for {}:{} to {:?}", user_uri, device_id, status);
        Ok(())
    }
    
    /// Remove a device from tracking
    pub fn remove_device(&self, user_uri: &str, device_id: &str) -> Result<()> {
        if let Some(mut devices) = self.user_devices.get_mut(user_uri) {
            devices.remove(device_id);
            
            // If no devices left, remove user entry
            if devices.is_empty() {
                drop(devices);
                self.user_devices.remove(user_uri);
            }
            
            // Invalidate cache
            self.aggregated_cache.remove(user_uri);
            
            info!("Removed device {} for user {}", device_id, user_uri);
        }
        
        Ok(())
    }
    
    /// Get aggregated presence for a user
    pub fn get_aggregated_presence(&self, user_uri: &str) -> Option<PresenceInfo> {
        // Check cache first
        if let Some(cached) = self.aggregated_cache.get(user_uri) {
            return Some(cached.clone());
        }
        
        // Get all devices for user
        let devices = self.user_devices.get(user_uri)?;
        if devices.is_empty() {
            return None;
        }
        
        // Aggregate based on strategy
        let aggregated = match self.strategy {
            AggregationStrategy::MostAvailable => self.aggregate_most_available(&devices),
            AggregationStrategy::MostRecent => self.aggregate_most_recent(&devices),
            AggregationStrategy::HighestPriority => self.aggregate_highest_priority(&devices),
            AggregationStrategy::Custom => self.aggregate_custom(&devices),
        };
        
        // Cache the result
        self.aggregated_cache.insert(user_uri.to_string(), aggregated.clone());
        
        Some(aggregated)
    }
    
    /// Aggregate using "most available" strategy
    fn aggregate_most_available(&self, devices: &HashMap<String, DevicePresence>) -> PresenceInfo {
        let mut best_status = PresenceStatus::Offline;
        let mut best_note = None;
        let mut all_capabilities = HashSet::new();
        
        for device in devices.values() {
            // Skip old offline devices
            if device.status == PresenceStatus::Offline {
                let age = Utc::now() - device.last_updated;
                if age > self.rules.offline_timeout {
                    continue;
                }
            }
            
            // Update best status (Available > Away > Busy > DND > Offline)
            best_status = match (&best_status, &device.status) {
                // If current best is Offline, take any new status
                (PresenceStatus::Offline, s) => s.clone(),
                // Available is the best - always take it
                (_, PresenceStatus::Available) => PresenceStatus::Available,
                // If we already have Available, keep it
                (PresenceStatus::Available, _) => PresenceStatus::Available,
                // Away is better than Busy, DND, or Offline
                (PresenceStatus::Busy | PresenceStatus::DoNotDisturb, PresenceStatus::Away) => PresenceStatus::Away,
                (_, PresenceStatus::Away) if best_status != PresenceStatus::Available => PresenceStatus::Away,
                // Busy is better than DND or Offline
                (PresenceStatus::DoNotDisturb, PresenceStatus::Busy) => PresenceStatus::Busy,
                (_, PresenceStatus::Busy) if best_status != PresenceStatus::Available && best_status != PresenceStatus::Away => PresenceStatus::Busy,
                // Otherwise keep current best
                _ => best_status,
            };
            
            // Collect notes
            if device.note.is_some() && best_note.is_none() {
                best_note = device.note.clone();
            }
            
            // Collect capabilities
            if self.rules.combine_capabilities && device.status != PresenceStatus::Offline {
                all_capabilities.extend(device.capabilities.iter().cloned());
            }
        }
        
        let mut info = PresenceInfo::new(String::new(), best_status);
        if let Some(note) = best_note {
            info = info.with_note(Some(note));
        }
        
        info
    }
    
    /// Aggregate using "most recent" strategy
    fn aggregate_most_recent(&self, devices: &HashMap<String, DevicePresence>) -> PresenceInfo {
        let most_recent = devices
            .values()
            .max_by_key(|d| d.last_updated)
            .expect("Should have at least one device");
        
        PresenceInfo::new(String::new(), most_recent.status.clone())
            .with_note(most_recent.note.clone())
    }
    
    /// Aggregate using "highest priority" strategy
    fn aggregate_highest_priority(&self, devices: &HashMap<String, DevicePresence>) -> PresenceInfo {
        let highest_priority = devices
            .values()
            .max_by_key(|d| d.priority)
            .expect("Should have at least one device");
        
        PresenceInfo::new(String::new(), highest_priority.status.clone())
            .with_note(highest_priority.note.clone())
    }
    
    /// Aggregate using custom rules
    fn aggregate_custom(&self, devices: &HashMap<String, DevicePresence>) -> PresenceInfo {
        // Find preferred device type
        for preferred_type in &self.rules.preferred_device_types {
            if let Some(device) = devices.values()
                .find(|d| d.device_type.as_ref() == Some(preferred_type)) {
                
                // Skip if too old and offline
                if device.status == PresenceStatus::Offline {
                    let age = Utc::now() - device.last_updated;
                    if age > self.rules.offline_timeout {
                        continue;
                    }
                }
                
                return PresenceInfo::new(String::new(), device.status.clone())
                    .with_note(device.note.clone());
            }
        }
        
        // Fall back to most available
        self.aggregate_most_available(devices)
    }
    
    /// Get all devices for a user
    pub fn get_user_devices(&self, user_uri: &str) -> Vec<DevicePresence> {
        self.user_devices
            .get(user_uri)
            .map(|devices| devices.values().cloned().collect())
            .unwrap_or_default()
    }
    
    /// Get device count for a user
    pub fn get_device_count(&self, user_uri: &str) -> usize {
        self.user_devices
            .get(user_uri)
            .map(|devices| devices.len())
            .unwrap_or(0)
    }
    
    /// Clear all devices for a user
    pub fn clear_user_devices(&self, user_uri: &str) {
        self.user_devices.remove(user_uri);
        self.aggregated_cache.remove(user_uri);
        info!("Cleared all devices for user {}", user_uri);
    }
    
    /// Get statistics about aggregation
    pub fn get_stats(&self) -> AggregationStats {
        let total_users = self.user_devices.len();
        let mut total_devices = 0;
        let mut online_devices = 0;
        
        for devices in self.user_devices.iter() {
            total_devices += devices.len();
            online_devices += devices
                .values()
                .filter(|d| d.status != PresenceStatus::Offline)
                .count();
        }
        
        AggregationStats {
            total_users,
            total_devices,
            online_devices,
            cache_size: self.aggregated_cache.len(),
        }
    }
}

/// Statistics about presence aggregation
#[derive(Debug, Clone)]
pub struct AggregationStats {
    /// Total number of users being tracked
    pub total_users: usize,
    
    /// Total number of devices across all users
    pub total_devices: usize,
    
    /// Number of online devices
    pub online_devices: usize,
    
    /// Size of aggregation cache
    pub cache_size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_most_available_aggregation() {
        let aggregator = PresenceAggregator::new(AggregationStrategy::MostAvailable);
        
        // Add devices with different statuses
        aggregator.update_device_presence(
            "sip:alice@example.com",
            "mobile",
            PresenceStatus::Busy,
            Some("In meeting".to_string()),
            Some("mobile".to_string()),
        ).unwrap();
        
        aggregator.update_device_presence(
            "sip:alice@example.com",
            "desktop",
            PresenceStatus::Available,
            Some("At desk".to_string()),
            Some("desktop".to_string()),
        ).unwrap();
        
        // Should aggregate to Available (most available)
        let presence = aggregator.get_aggregated_presence("sip:alice@example.com").unwrap();
        assert_eq!(presence.status, PresenceStatus::Available);
    }
    
    #[test]
    fn test_device_removal() {
        let aggregator = PresenceAggregator::new(AggregationStrategy::MostRecent);
        
        // Add two devices
        aggregator.update_device_presence(
            "sip:bob@example.com",
            "device1",
            PresenceStatus::Available,
            None,
            None,
        ).unwrap();
        
        aggregator.update_device_presence(
            "sip:bob@example.com",
            "device2",
            PresenceStatus::Busy,
            None,
            None,
        ).unwrap();
        
        assert_eq!(aggregator.get_device_count("sip:bob@example.com"), 2);
        
        // Remove one device
        aggregator.remove_device("sip:bob@example.com", "device1").unwrap();
        assert_eq!(aggregator.get_device_count("sip:bob@example.com"), 1);
    }
    
    #[test]
    fn test_priority_aggregation() {
        let aggregator = PresenceAggregator::new(AggregationStrategy::HighestPriority);
        
        // Add device with low priority
        let mut devices = HashMap::new();
        devices.insert(
            "low".to_string(),
            DevicePresence::new("low".to_string(), PresenceStatus::Available)
                .with_priority(1),
        );
        
        // Add device with high priority
        devices.insert(
            "high".to_string(),
            DevicePresence::new("high".to_string(), PresenceStatus::Busy)
                .with_priority(10),
        );
        
        aggregator.user_devices.insert("user".to_string(), devices);
        
        // Should use high priority device status (Busy)
        let presence = aggregator.get_aggregated_presence("user").unwrap();
        assert_eq!(presence.status, PresenceStatus::Busy);
    }
}