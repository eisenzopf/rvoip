use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tracing::{debug, warn};

use crate::error::{Error, Result};
use crate::codec::traits::{Codec, CodecCapability, CodecFactory, MediaType};

/// Registry for managing all available codecs
#[derive(Debug, Default)]
pub struct CodecRegistry {
    /// Factories for creating codecs
    factories: RwLock<HashMap<String, Arc<dyn CodecFactory>>>,
}

impl CodecRegistry {
    /// Create a new empty codec registry
    pub fn new() -> Self {
        Self {
            factories: RwLock::new(HashMap::new()),
        }
    }
    
    /// Register a codec factory
    pub fn register_factory(&self, factory: Arc<dyn CodecFactory>) -> Result<()> {
        let id = factory.id().to_string();
        let mut factories = self.factories.write().map_err(|_| Error::LockError)?;
        
        if factories.contains_key(&id) {
            warn!("Overwriting existing codec factory for {}", id);
        }
        
        factories.insert(id.clone(), factory);
        debug!("Registered codec factory: {}", id);
        
        Ok(())
    }
    
    /// Create a codec instance by ID
    pub fn create(&self, id: &str) -> Result<Box<dyn Codec>> {
        let factories = self.factories.read().map_err(|_| Error::LockError)?;
        
        if let Some(factory) = factories.get(id) {
            debug!("Creating codec: {}", id);
            factory.create_default()
        } else {
            Err(Error::CodecNotFound(id.to_string()))
        }
    }
    
    /// Create a codec instance with parameters
    pub fn create_with_params(&self, id: &str, params: &[u8]) -> Result<Box<dyn Codec>> {
        let factories = self.factories.read().map_err(|_| Error::LockError)?;
        
        if let Some(factory) = factories.get(id) {
            debug!("Creating codec with params: {}", id);
            factory.create_with_params(params)
        } else {
            Err(Error::CodecNotFound(id.to_string()))
        }
    }
    
    /// Get a list of all available codec capabilities
    pub fn list_capabilities(&self) -> Result<Vec<CodecCapability>> {
        let factories = self.factories.read().map_err(|_| Error::LockError)?;
        
        let mut capabilities = Vec::new();
        for factory in factories.values() {
            capabilities.extend(factory.capabilities());
        }
        
        Ok(capabilities)
    }
    
    /// Get capabilities filtered by media type
    pub fn list_capabilities_by_type(&self, media_type: MediaType) -> Result<Vec<CodecCapability>> {
        let all_caps = self.list_capabilities()?;
        
        Ok(all_caps
            .into_iter()
            .filter(|cap| cap.media_type == media_type)
            .collect())
    }
    
    /// Check if a codec is available
    pub fn has_codec(&self, id: &str) -> bool {
        if let Ok(factories) = self.factories.read() {
            factories.contains_key(id)
        } else {
            false
        }
    }
    
    /// Get factory for a codec
    pub fn get_factory(&self, id: &str) -> Option<Arc<dyn CodecFactory>> {
        if let Ok(factories) = self.factories.read() {
            factories.get(id).cloned()
        } else {
            None
        }
    }
    
    /// Unregister a codec factory
    pub fn unregister_factory(&self, id: &str) -> Result<()> {
        let mut factories = self.factories.write().map_err(|_| Error::LockError)?;
        
        if factories.remove(id).is_some() {
            debug!("Unregistered codec factory: {}", id);
            Ok(())
        } else {
            Err(Error::CodecNotFound(id.to_string()))
        }
    }
    
    /// Get count of registered codecs
    pub fn count(&self) -> usize {
        if let Ok(factories) = self.factories.read() {
            factories.len()
        } else {
            0
        }
    }
    
    /// Create a singleton codec registry
    pub fn global() -> &'static CodecRegistry {
        static INSTANCE: std::sync::OnceLock<CodecRegistry> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(|| {
            debug!("Creating global codec registry");
            CodecRegistry::new()
        })
    }
}

/// Codec negotiation helper functions
pub mod negotiation {
    use super::*;
    
    /// Find best matching codec for a given capability
    pub fn find_matching_codec(
        registry: &CodecRegistry,
        remote_cap: &CodecCapability,
    ) -> Result<Box<dyn Codec>> {
        // First try exact ID match
        if registry.has_codec(&remote_cap.id) {
            return registry.create(&remote_cap.id);
        }
        
        // Then try by payload type for well-known codecs
        if let Some(pt) = remote_cap.payload_type {
            let caps = registry.list_capabilities()?;
            for cap in caps {
                if let Some(local_pt) = cap.payload_type {
                    if local_pt == pt && cap.media_type == remote_cap.media_type {
                        return registry.create(&cap.id);
                    }
                }
            }
        }
        
        // Finally try by MIME type
        let caps = registry.list_capabilities()?;
        for cap in caps {
            if cap.mime_type == remote_cap.mime_type && cap.media_type == remote_cap.media_type {
                return registry.create(&cap.id);
            }
        }
        
        Err(Error::CodecNotFound(format!(
            "No matching codec for {}", remote_cap.name
        )))
    }
    
    /// Find best codec from a list of candidates
    pub fn select_best_codec(
        registry: &CodecRegistry,
        candidates: &[CodecCapability],
        media_type: MediaType,
    ) -> Result<Box<dyn Codec>> {
        if candidates.is_empty() {
            return Err(Error::CodecNotFound("No codec candidates provided".to_string()));
        }
        
        // Filter by media type
        let candidates: Vec<_> = candidates
            .iter()
            .filter(|c| c.media_type == media_type)
            .collect();
        
        if candidates.is_empty() {
            return Err(Error::CodecNotFound(format!(
                "No codec candidates for media type {:?}", media_type
            )));
        }
        
        // Priority order: Opus > G722 > G711a > G711u > others
        // This is a simple heuristic for voice quality
        let priority_order = ["opus", "g722", "g711a", "g711u"];
        
        for &codec_id in &priority_order {
            for cap in &candidates {
                if cap.id.to_lowercase().contains(codec_id) {
                    if let Ok(codec) = registry.create(&cap.id) {
                        return Ok(codec);
                    }
                }
            }
        }
        
        // Fall back to first available candidate
        if let Some(first) = candidates.first() {
            registry.create(&first.id)
        } else {
            Err(Error::CodecNotFound("No suitable codec found".to_string()))
        }
    }
} 