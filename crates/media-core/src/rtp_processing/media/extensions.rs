//! Header extensions functionality (moved from rtp-core)
//!
//! This module handles RTP header extensions for media processing,
//! including audio level detection and other media-related extensions.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::api::error::MediaError;

/// Header extension format types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionFormat {
    /// One-byte header extension format
    OneByte,
    /// Two-byte header extension format
    TwoByte,
    /// Legacy format
    Legacy,
}

/// Header extension data
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderExtension {
    /// Extension identifier
    pub id: u8,
    /// Extension URI (for mapping)
    pub uri: String,
    /// Extension data
    pub data: Vec<u8>,
}

impl HeaderExtension {
    /// Create a new header extension
    pub fn new(id: u8, uri: String, data: Vec<u8>) -> Self {
        Self { id, uri, data }
    }
    
    /// Create an audio level extension
    pub fn audio_level(id: u8, voice_activity: bool, level: u8) -> Self {
        // Clamp level to 0-127
        let level = level.min(127);
        
        // For audio level, the format is:
        // - Bit 7 (MSB): Voice activity flag (0 = active, 1 = inactive)
        // - Bits 0-6: Level (0-127 dB)
        let data = if voice_activity {
            // For active voice, the MSB is 0
            vec![level]
        } else {
            // For inactive voice, the MSB is 1
            vec![level | 0x80]
        };
        
        Self {
            id,
            uri: "urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string(),
            data,
        }
    }
    
    /// Parse audio level from extension data
    pub fn parse_audio_level(&self) -> Option<(bool, u8)> {
        if self.uri != "urn:ietf:params:rtp-hdrext:ssrc-audio-level" || self.data.is_empty() {
            return None;
        }
        
        let byte = self.data[0];
        
        // Extract voice activity flag (0 = active, 1 = inactive)
        let voice_activity = (byte & 0x80) == 0;
        
        // Extract level (0-127 dB)
        let level = byte & 0x7F;
        
        Some((voice_activity, level))
    }
}

/// Header extension manager
pub struct HeaderExtensionManager {
    /// Whether header extensions are enabled
    enabled: bool,
    /// Extension format
    format: ExtensionFormat,
    /// Extension ID to URI mappings
    id_mappings: HashMap<u8, String>,
    /// URI to extension ID mappings
    uri_mappings: HashMap<String, u8>,
    /// Next available extension ID
    next_id: u8,
}

impl HeaderExtensionManager {
    /// Create a new header extension manager
    pub fn new() -> Self {
        Self {
            enabled: false,
            format: ExtensionFormat::OneByte,
            id_mappings: HashMap::new(),
            uri_mappings: HashMap::new(),
            next_id: 1,
        }
    }
    
    /// Enable header extensions with the specified format
    pub fn enable(&mut self, format: ExtensionFormat) -> Result<(), MediaError> {
        self.format = format;
        self.enabled = true;
        debug!("Enabled header extensions with format: {:?}", format);
        Ok(())
    }
    
    /// Disable header extensions
    pub fn disable(&mut self) {
        self.enabled = false;
        debug!("Disabled header extensions");
    }
    
    /// Check if header extensions are enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    /// Get the current extension format
    pub fn get_format(&self) -> ExtensionFormat {
        self.format
    }
    
    /// Configure a header extension mapping
    pub fn configure_extension(&mut self, id: u8, uri: String) -> Result<(), MediaError> {
        if !self.enabled {
            return Err(MediaError::ConfigError("Header extensions are not enabled".to_string()));
        }
        
        // Check if ID is valid for the current format
        match self.format {
            ExtensionFormat::OneByte => {
                if id < 1 || id > 14 {
                    return Err(MediaError::ConfigError(
                        format!("Invalid extension ID for one-byte format: {} (must be 1-14)", id)
                    ));
                }
            },
            ExtensionFormat::TwoByte => {
                if id < 1 || id > 255 {
                    return Err(MediaError::ConfigError(
                        format!("Invalid extension ID for two-byte format: {} (must be 1-255)", id)
                    ));
                }
            },
            ExtensionFormat::Legacy => {
                // No specific validation for legacy format
            },
        }
        
        // Remove old mapping for this URI if it exists
        if let Some(old_id) = self.uri_mappings.remove(&uri) {
            self.id_mappings.remove(&old_id);
        }
        
        // Add new mapping
        self.id_mappings.insert(id, uri.clone());
        self.uri_mappings.insert(uri.clone(), id);
        
        debug!("Configured header extension ID {} -> URI: {}", id, uri);
        Ok(())
    }
    
    /// Configure multiple header extension mappings
    pub fn configure_extensions(&mut self, mappings: HashMap<u8, String>) -> Result<(), MediaError> {
        for (id, uri) in mappings {
            self.configure_extension(id, uri)?;
        }
        Ok(())
    }
    
    /// Auto-configure an extension with the next available ID
    pub fn auto_configure_extension(&mut self, uri: String) -> Result<u8, MediaError> {
        if !self.enabled {
            return Err(MediaError::ConfigError("Header extensions are not enabled".to_string()));
        }
        
        // Find next available ID
        let max_id = match self.format {
            ExtensionFormat::OneByte => 14,
            ExtensionFormat::TwoByte => 255,
            ExtensionFormat::Legacy => 255,
        };
        
        while self.next_id <= max_id && self.id_mappings.contains_key(&self.next_id) {
            self.next_id += 1;
        }
        
        if self.next_id > max_id {
            return Err(MediaError::ConfigError("No available extension IDs".to_string()));
        }
        
        let id = self.next_id;
        self.configure_extension(id, uri)?;
        self.next_id += 1;
        
        Ok(id)
    }
    
    /// Get extension ID for a URI
    pub fn get_id_for_uri(&self, uri: &str) -> Option<u8> {
        self.uri_mappings.get(uri).copied()
    }
    
    /// Get URI for an extension ID
    pub fn get_uri_for_id(&self, id: u8) -> Option<&String> {
        self.id_mappings.get(&id)
    }
    
    /// Get all configured mappings
    pub fn get_all_mappings(&self) -> HashMap<u8, String> {
        self.id_mappings.clone()
    }
    
    /// Clear all extension mappings
    pub fn clear_mappings(&mut self) {
        self.id_mappings.clear();
        self.uri_mappings.clear();
        self.next_id = 1;
        debug!("Cleared all header extension mappings");
    }
    
    /// Create an audio level extension
    pub fn create_audio_level_extension(&self, voice_activity: bool, level: u8) -> Result<HeaderExtension, MediaError> {
        let id = self.get_id_for_uri("urn:ietf:params:rtp-hdrext:ssrc-audio-level")
            .unwrap_or(1); // Default to ID 1 if not configured
        
        Ok(HeaderExtension::audio_level(id, voice_activity, level))
    }
}

impl Default for HeaderExtensionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Media header extension service
pub struct MediaHeaderExtensionService {
    /// Extension manager
    manager: Arc<RwLock<HeaderExtensionManager>>,
    /// Pending extensions per client
    pending_extensions: Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    /// Received extensions per client
    received_extensions: Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
}

impl MediaHeaderExtensionService {
    /// Create a new media header extension service
    pub fn new() -> Self {
        Self {
            manager: Arc::new(RwLock::new(HeaderExtensionManager::new())),
            pending_extensions: Arc::new(RwLock::new(HashMap::new())),
            received_extensions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Enable header extensions
    pub async fn enable(&self, format: ExtensionFormat) -> Result<(), MediaError> {
        let mut manager = self.manager.write().await;
        manager.enable(format)
    }
    
    /// Disable header extensions
    pub async fn disable(&self) -> Result<(), MediaError> {
        let mut manager = self.manager.write().await;
        manager.disable();
        Ok(())
    }
    
    /// Check if header extensions are enabled
    pub async fn is_enabled(&self) -> bool {
        let manager = self.manager.read().await;
        manager.is_enabled()
    }
    
    /// Configure extension mappings
    pub async fn configure_extensions(&self, mappings: HashMap<u8, String>) -> Result<(), MediaError> {
        let mut manager = self.manager.write().await;
        manager.configure_extensions(mappings)
    }
    
    /// Configure audio level extension
    pub async fn configure_audio_level_extension(&self, id: Option<u8>) -> Result<u8, MediaError> {
        let mut manager = self.manager.write().await;
        
        if let Some(id) = id {
            manager.configure_extension(id, "urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string())?;
            Ok(id)
        } else {
            manager.auto_configure_extension("urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string())
        }
    }
    
    /// Add header extension for a client
    pub async fn add_extension_for_client(&self, client_id: &str, extension: HeaderExtension) -> Result<(), MediaError> {
        if !self.is_enabled().await {
            return Err(MediaError::ConfigError("Header extensions are not enabled".to_string()));
        }
        
        let mut pending = self.pending_extensions.write().await;
        let client_pending = pending.entry(client_id.to_string()).or_insert_with(Vec::new);
        client_pending.push(extension);
        
        debug!("Added header extension for client: {}", client_id);
        Ok(())
    }
    
    /// Add audio level extension for a client
    pub async fn add_audio_level_for_client(&self, client_id: &str, voice_activity: bool, level: u8) -> Result<(), MediaError> {
        let manager = self.manager.read().await;
        let extension = manager.create_audio_level_extension(voice_activity, level)?;
        drop(manager);
        
        self.add_extension_for_client(client_id, extension).await
    }
    
    /// Get pending extensions for a client
    pub async fn get_pending_extensions(&self, client_id: &str) -> Result<Vec<HeaderExtension>, MediaError> {
        let pending = self.pending_extensions.read().await;
        Ok(pending.get(client_id).cloned().unwrap_or_default())
    }
    
    /// Clear pending extensions for a client
    pub async fn clear_pending_extensions(&self, client_id: &str) -> Result<(), MediaError> {
        let mut pending = self.pending_extensions.write().await;
        pending.remove(client_id);
        Ok(())
    }
    
    /// Record received extension from a client
    pub async fn record_received_extension(&self, client_id: &str, extension: HeaderExtension) -> Result<(), MediaError> {
        let mut received = self.received_extensions.write().await;
        let client_received = received.entry(client_id.to_string()).or_insert_with(Vec::new);
        client_received.push(extension);
        Ok(())
    }
    
    /// Get received extensions for a client
    pub async fn get_received_extensions(&self, client_id: &str) -> Result<Vec<HeaderExtension>, MediaError> {
        let received = self.received_extensions.read().await;
        Ok(received.get(client_id).cloned().unwrap_or_default())
    }
    
    /// Get received audio level for a client
    pub async fn get_received_audio_level(&self, client_id: &str) -> Result<Option<(bool, u8)>, MediaError> {
        let received = self.received_extensions.read().await;
        
        if let Some(extensions) = received.get(client_id) {
            // Look for the most recent audio level extension
            for extension in extensions.iter().rev() {
                if let Some(audio_level) = extension.parse_audio_level() {
                    return Ok(Some(audio_level));
                }
            }
        }
        
        Ok(None)
    }
    
    /// Clear received extensions for a client
    pub async fn clear_received_extensions(&self, client_id: &str) -> Result<(), MediaError> {
        let mut received = self.received_extensions.write().await;
        received.remove(client_id);
        Ok(())
    }
    
    /// Clear all extensions
    pub async fn clear_all_extensions(&self) -> Result<(), MediaError> {
        let mut pending = self.pending_extensions.write().await;
        let mut received = self.received_extensions.write().await;
        
        pending.clear();
        received.clear();
        
        debug!("Cleared all header extensions");
        Ok(())
    }
}

impl Default for MediaHeaderExtensionService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_audio_level_extension() {
        // Test active voice
        let ext = HeaderExtension::audio_level(1, true, 50);
        assert_eq!(ext.id, 1);
        assert_eq!(ext.data, vec![50]);
        
        let parsed = ext.parse_audio_level().unwrap();
        assert_eq!(parsed, (true, 50));
        
        // Test inactive voice
        let ext = HeaderExtension::audio_level(1, false, 30);
        assert_eq!(ext.data, vec![30 | 0x80]);
        
        let parsed = ext.parse_audio_level().unwrap();
        assert_eq!(parsed, (false, 30));
    }
    
    #[test]
    fn test_header_extension_manager() {
        let mut manager = HeaderExtensionManager::new();
        
        // Initially disabled
        assert!(!manager.is_enabled());
        
        // Enable
        manager.enable(ExtensionFormat::OneByte).unwrap();
        assert!(manager.is_enabled());
        assert_eq!(manager.get_format(), ExtensionFormat::OneByte);
        
        // Configure extension
        manager.configure_extension(1, "urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string()).unwrap();
        assert_eq!(manager.get_id_for_uri("urn:ietf:params:rtp-hdrext:ssrc-audio-level"), Some(1));
        assert_eq!(manager.get_uri_for_id(1), Some(&"urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string()));
        
        // Auto-configure extension
        let id = manager.auto_configure_extension("urn:ietf:params:rtp-hdrext:toffset".to_string()).unwrap();
        assert_eq!(id, 2);
        assert_eq!(manager.get_id_for_uri("urn:ietf:params:rtp-hdrext:toffset"), Some(2));
        
        // Create audio level extension
        let ext = manager.create_audio_level_extension(true, 75).unwrap();
        assert_eq!(ext.id, 1);
        assert_eq!(ext.parse_audio_level(), Some((true, 75)));
    }
    
    #[tokio::test]
    async fn test_media_header_extension_service() {
        let service = MediaHeaderExtensionService::new();
        
        // Initially disabled
        assert!(!service.is_enabled().await);
        
        // Enable
        service.enable(ExtensionFormat::OneByte).await.unwrap();
        assert!(service.is_enabled().await);
        
        // Configure audio level extension
        let id = service.configure_audio_level_extension(Some(1)).await.unwrap();
        assert_eq!(id, 1);
        
        // Add audio level for client
        service.add_audio_level_for_client("client1", true, 60).await.unwrap();
        
        // Get pending extensions
        let pending = service.get_pending_extensions("client1").await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].parse_audio_level(), Some((true, 60)));
        
        // Record received extension
        let received_ext = HeaderExtension::audio_level(1, false, 40);
        service.record_received_extension("client1", received_ext).await.unwrap();
        
        // Get received audio level
        let audio_level = service.get_received_audio_level("client1").await.unwrap();
        assert_eq!(audio_level, Some((false, 40)));
    }
}