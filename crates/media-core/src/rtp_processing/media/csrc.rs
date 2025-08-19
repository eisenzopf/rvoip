//! CSRC management functionality (moved from rtp-core)
//!
//! This module handles Contributing Source (CSRC) identifier management
//! for media processing and conferencing scenarios.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::api::error::MediaError;

/// CSRC identifier type
pub type RtpSsrc = u32;
pub type RtpCsrc = u32;

/// CSRC mapping information
#[derive(Debug, Clone, PartialEq)]
pub struct CsrcMapping {
    /// Original SSRC
    pub original_ssrc: RtpSsrc,
    /// Mapped CSRC
    pub csrc: RtpCsrc,
    /// Canonical name (CNAME)
    pub cname: Option<String>,
    /// Display name
    pub display_name: Option<String>,
    /// Media type
    pub media_type: Option<String>,
}

impl CsrcMapping {
    /// Create a new CSRC mapping
    pub fn new(original_ssrc: RtpSsrc, csrc: RtpCsrc) -> Self {
        Self {
            original_ssrc,
            csrc,
            cname: None,
            display_name: None,
            media_type: None,
        }
    }
    
    /// Set the CNAME for this mapping
    pub fn with_cname(mut self, cname: String) -> Self {
        self.cname = Some(cname);
        self
    }
    
    /// Set the display name for this mapping
    pub fn with_display_name(mut self, name: String) -> Self {
        self.display_name = Some(name);
        self
    }
    
    /// Set the media type for this mapping
    pub fn with_media_type(mut self, media_type: String) -> Self {
        self.media_type = Some(media_type);
        self
    }
}

/// CSRC manager for handling contributing source mappings
pub struct CsrcManager {
    /// SSRC to CSRC mappings
    mappings: HashMap<RtpSsrc, CsrcMapping>,
    /// Next available CSRC identifier
    next_csrc: RtpCsrc,
}

impl CsrcManager {
    /// Create a new CSRC manager
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
            next_csrc: 0x10000000, // Start from a high value to avoid conflicts
        }
    }
    
    /// Add a new CSRC mapping
    pub fn add_mapping(&mut self, mapping: CsrcMapping) {
        debug!("Adding CSRC mapping: {:08x} -> {:08x}", mapping.original_ssrc, mapping.csrc);
        self.mappings.insert(mapping.original_ssrc, mapping);
    }
    
    /// Add a simple SSRC to CSRC mapping
    pub fn add_simple_mapping(&mut self, original_ssrc: RtpSsrc, csrc: RtpCsrc) {
        let mapping = CsrcMapping::new(original_ssrc, csrc);
        self.add_mapping(mapping);
    }
    
    /// Add a mapping with automatic CSRC generation
    pub fn add_auto_mapping(&mut self, original_ssrc: RtpSsrc) -> RtpCsrc {
        let csrc = self.next_csrc;
        self.next_csrc = self.next_csrc.wrapping_add(1);
        
        let mapping = CsrcMapping::new(original_ssrc, csrc);
        self.add_mapping(mapping);
        
        csrc
    }
    
    /// Remove a mapping by original SSRC
    pub fn remove_by_ssrc(&mut self, original_ssrc: RtpSsrc) -> Option<CsrcMapping> {
        let removed = self.mappings.remove(&original_ssrc);
        if removed.is_some() {
            debug!("Removed CSRC mapping for SSRC: {:08x}", original_ssrc);
        }
        removed
    }
    
    /// Get a mapping by original SSRC
    pub fn get_by_ssrc(&self, original_ssrc: RtpSsrc) -> Option<&CsrcMapping> {
        self.mappings.get(&original_ssrc)
    }
    
    /// Get all mappings
    pub fn get_all_mappings(&self) -> Vec<CsrcMapping> {
        self.mappings.values().cloned().collect()
    }
    
    /// Update CNAME for a mapping
    pub fn update_cname(&mut self, original_ssrc: RtpSsrc, cname: String) -> bool {
        if let Some(mapping) = self.mappings.get_mut(&original_ssrc) {
            mapping.cname = Some(cname);
            true
        } else {
            false
        }
    }
    
    /// Update display name for a mapping
    pub fn update_display_name(&mut self, original_ssrc: RtpSsrc, name: String) -> bool {
        if let Some(mapping) = self.mappings.get_mut(&original_ssrc) {
            mapping.display_name = Some(name);
            true
        } else {
            false
        }
    }
    
    /// Get CSRC values for active sources
    pub fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc]) -> Vec<RtpCsrc> {
        active_ssrcs
            .iter()
            .filter_map(|&ssrc| self.get_by_ssrc(ssrc).map(|mapping| mapping.csrc))
            .collect()
    }
    
    /// Get CSRC for a specific SSRC
    pub fn get_csrc_for_ssrc(&self, ssrc: RtpSsrc) -> Option<RtpCsrc> {
        self.get_by_ssrc(ssrc).map(|mapping| mapping.csrc)
    }
    
    /// Check if an SSRC has a mapping
    pub fn has_mapping(&self, original_ssrc: RtpSsrc) -> bool {
        self.mappings.contains_key(&original_ssrc)
    }
    
    /// Clear all mappings
    pub fn clear(&mut self) {
        debug!("Clearing all CSRC mappings");
        self.mappings.clear();
    }
    
    /// Get the number of mappings
    pub fn len(&self) -> usize {
        self.mappings.len()
    }
    
    /// Check if the manager has no mappings
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
}

impl Default for CsrcManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Media CSRC service for managing CSRC operations
pub struct MediaCsrcService {
    /// CSRC manager
    manager: Arc<RwLock<CsrcManager>>,
    /// Whether CSRC management is enabled
    enabled: Arc<RwLock<bool>>,
}

impl MediaCsrcService {
    /// Create a new media CSRC service
    pub fn new() -> Self {
        Self {
            manager: Arc::new(RwLock::new(CsrcManager::new())),
            enabled: Arc::new(RwLock::new(false)),
        }
    }
    
    /// Enable CSRC management
    pub async fn enable(&self) -> Result<(), MediaError> {
        let mut enabled = self.enabled.write().await;
        *enabled = true;
        debug!("Enabled media CSRC management");
        Ok(())
    }
    
    /// Disable CSRC management
    pub async fn disable(&self) -> Result<(), MediaError> {
        let mut enabled = self.enabled.write().await;
        *enabled = false;
        debug!("Disabled media CSRC management");
        Ok(())
    }
    
    /// Check if CSRC management is enabled
    pub async fn is_enabled(&self) -> bool {
        *self.enabled.read().await
    }
    
    /// Add a new CSRC mapping
    pub async fn add_mapping(&self, mapping: CsrcMapping) -> Result<(), MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let mut manager = self.manager.write().await;
        manager.add_mapping(mapping);
        Ok(())
    }
    
    /// Add a simple SSRC to CSRC mapping
    pub async fn add_simple_mapping(&self, original_ssrc: RtpSsrc, csrc: RtpCsrc) -> Result<(), MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let mut manager = self.manager.write().await;
        manager.add_simple_mapping(original_ssrc, csrc);
        Ok(())
    }
    
    /// Add a mapping with automatic CSRC generation
    pub async fn add_auto_mapping(&self, original_ssrc: RtpSsrc) -> Result<RtpCsrc, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let mut manager = self.manager.write().await;
        let csrc = manager.add_auto_mapping(original_ssrc);
        Ok(csrc)
    }
    
    /// Remove a mapping by original SSRC
    pub async fn remove_mapping(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let mut manager = self.manager.write().await;
        Ok(manager.remove_by_ssrc(original_ssrc))
    }
    
    /// Get a mapping by original SSRC
    pub async fn get_mapping(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let manager = self.manager.read().await;
        Ok(manager.get_by_ssrc(original_ssrc).cloned())
    }
    
    /// Get all mappings
    pub async fn get_all_mappings(&self) -> Result<Vec<CsrcMapping>, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let manager = self.manager.read().await;
        Ok(manager.get_all_mappings())
    }
    
    /// Update CNAME for a mapping
    pub async fn update_cname(&self, original_ssrc: RtpSsrc, cname: String) -> Result<bool, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let mut manager = self.manager.write().await;
        Ok(manager.update_cname(original_ssrc, cname))
    }
    
    /// Update display name for a mapping
    pub async fn update_display_name(&self, original_ssrc: RtpSsrc, name: String) -> Result<bool, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let mut manager = self.manager.write().await;
        Ok(manager.update_display_name(original_ssrc, name))
    }
    
    /// Get CSRC values for active sources
    pub async fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc]) -> Result<Vec<RtpCsrc>, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let manager = self.manager.read().await;
        Ok(manager.get_active_csrcs(active_ssrcs))
    }
    
    /// Get CSRC for a specific SSRC
    pub async fn get_csrc_for_ssrc(&self, ssrc: RtpSsrc) -> Result<Option<RtpCsrc>, MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let manager = self.manager.read().await;
        Ok(manager.get_csrc_for_ssrc(ssrc))
    }
    
    /// Clear all mappings
    pub async fn clear(&self) -> Result<(), MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        let mut manager = self.manager.write().await;
        manager.clear();
        Ok(())
    }
}

impl Default for MediaCsrcService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_csrc_mapping() {
        let mapping = CsrcMapping::new(0x12345678, 0x87654321)
            .with_cname("test@example.com".to_string())
            .with_display_name("Test User".to_string())
            .with_media_type("audio".to_string());
        
        assert_eq!(mapping.original_ssrc, 0x12345678);
        assert_eq!(mapping.csrc, 0x87654321);
        assert_eq!(mapping.cname, Some("test@example.com".to_string()));
        assert_eq!(mapping.display_name, Some("Test User".to_string()));
        assert_eq!(mapping.media_type, Some("audio".to_string()));
    }
    
    #[test]
    fn test_csrc_manager() {
        let mut manager = CsrcManager::new();
        
        // Add mapping
        let mapping = CsrcMapping::new(0x12345678, 0x87654321);
        manager.add_mapping(mapping.clone());
        
        // Test retrieval
        assert!(manager.has_mapping(0x12345678));
        assert_eq!(manager.get_by_ssrc(0x12345678), Some(&mapping));
        assert_eq!(manager.get_csrc_for_ssrc(0x12345678), Some(0x87654321));
        
        // Test auto mapping
        let csrc = manager.add_auto_mapping(0x11111111);
        assert!(manager.has_mapping(0x11111111));
        assert_eq!(manager.get_csrc_for_ssrc(0x11111111), Some(csrc));
        
        // Test active CSRCs
        let active_ssrcs = vec![0x12345678, 0x11111111];
        let active_csrcs = manager.get_active_csrcs(&active_ssrcs);
        assert_eq!(active_csrcs.len(), 2);
        assert!(active_csrcs.contains(&0x87654321));
        assert!(active_csrcs.contains(&csrc));
        
        // Test removal
        let removed = manager.remove_by_ssrc(0x12345678);
        assert!(removed.is_some());
        assert!(!manager.has_mapping(0x12345678));
    }
    
    #[tokio::test]
    async fn test_media_csrc_service() {
        let service = MediaCsrcService::new();
        
        // Initially disabled
        assert!(!service.is_enabled().await);
        
        // Enable service
        service.enable().await.unwrap();
        assert!(service.is_enabled().await);
        
        // Add mapping
        let mapping = CsrcMapping::new(0x12345678, 0x87654321);
        service.add_mapping(mapping.clone()).await.unwrap();
        
        // Test retrieval
        let retrieved = service.get_mapping(0x12345678).await.unwrap();
        assert_eq!(retrieved, Some(mapping));
        
        // Test auto mapping
        let csrc = service.add_auto_mapping(0x11111111).await.unwrap();
        let retrieved_csrc = service.get_csrc_for_ssrc(0x11111111).await.unwrap();
        assert_eq!(retrieved_csrc, Some(csrc));
        
        // Test active CSRCs
        let active_ssrcs = vec![0x12345678, 0x11111111];
        let active_csrcs = service.get_active_csrcs(&active_ssrcs).await.unwrap();
        assert_eq!(active_csrcs.len(), 2);
    }
}