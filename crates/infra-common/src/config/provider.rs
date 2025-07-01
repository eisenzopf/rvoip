use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;
use crate::errors::types::{Error, Result};
use serde::de::DeserializeOwned;
use serde::{Serialize, Deserialize};

/// Source of configuration data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfigSource {
    /// Configuration from default values
    Default,
    /// Configuration from file
    File,
    /// Configuration from environment variables
    Environment,
    /// Configuration from command line arguments
    CommandLine,
    /// Configuration from an API call
    Api,
    /// Configuration from a database
    Database,
    /// Other configuration source
    Other,
}

/// Trait for configuration providers
pub trait ConfigProvider: Send + Sync + Debug {
    /// Get the name of this configuration provider
    fn name(&self) -> &str;
    
    /// Get the source of this configuration
    fn source(&self) -> ConfigSource;
    
    /// Get configuration as a specific type
    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T>;
    
    /// Get configuration as a raw value
    fn get_raw(&self, key: &str) -> Result<Box<dyn Any>>;
    
    /// Check if a configuration key exists
    fn has(&self, key: &str) -> bool;
    
    /// List available configuration keys
    fn keys(&self) -> Vec<String>;
    
    /// Reload configuration from source
    fn reload(&self) -> Result<()>;
}

/// A basic implementation of ConfigProvider that wraps a configuration object
#[derive(Debug)]
pub struct BasicConfigProvider<T: 'static> {
    name: String,
    source: ConfigSource,
    config: Arc<T>,
}

impl<T: DeserializeOwned + Send + Sync + 'static> BasicConfigProvider<T> {
    /// Create a new basic config provider
    pub fn new<S: Into<String>>(name: S, source: ConfigSource, config: T) -> Self {
        BasicConfigProvider {
            name: name.into(),
            source,
            config: Arc::new(config),
        }
    }
    
    /// Get the wrapped configuration object
    pub fn config(&self) -> Arc<T> {
        self.config.clone()
    }
}

impl<T: DeserializeOwned + Send + Sync + Debug + serde::Serialize + 'static> ConfigProvider for BasicConfigProvider<T> {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn source(&self) -> ConfigSource {
        self.source
    }
    
    fn get<U: DeserializeOwned>(&self, _key: &str) -> Result<U> {
        // This basic implementation doesn't support nested keys
        // Instead it tries to convert the entire config object
        let value = serde_json::to_value(&*self.config)
            .map_err(|e| Error::Config(format!("Failed to serialize config: {}", e)))?;
            
        serde_json::from_value(value)
            .map_err(|e| Error::Config(format!("Failed to deserialize config: {}", e)))
    }
    
    fn get_raw(&self, _key: &str) -> Result<Box<dyn Any>> {
        Ok(Box::new(self.config.clone()))
    }
    
    fn has(&self, _key: &str) -> bool {
        true // The basic provider only has the root object
    }
    
    fn keys(&self) -> Vec<String> {
        vec![] // This basic implementation doesn't expose nested keys
    }
    
    fn reload(&self) -> Result<()> {
        // Basic provider doesn't support reloading
        Err(Error::Config("Reload not supported".to_string()))
    }
} 