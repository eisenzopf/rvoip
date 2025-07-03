use std::path::{Path, PathBuf};
use std::fmt::Debug;

use crate::errors::types::{Error, Result};
use crate::config::provider::{ConfigSource, BasicConfigProvider};
use serde::de::DeserializeOwned;
use config::{Config, Environment, File};

/// Handles loading configuration from various sources
#[derive(Debug)]
pub struct ConfigLoader {
    base_path: PathBuf,
    environment: String,
}

impl ConfigLoader {
    /// Create a new configuration loader
    pub fn new<P: AsRef<Path>>(base_path: P, environment: &str) -> Self {
        ConfigLoader {
            base_path: base_path.as_ref().to_path_buf(),
            environment: environment.to_string(),
        }
    }
    
    /// Load configuration from a file
    pub fn load_from_file<T, P>(&self, file_path: P) -> Result<BasicConfigProvider<T>>
    where
        T: DeserializeOwned + Send + Sync + Debug + 'static,
        P: AsRef<Path>,
    {
        let full_path = self.resolve_path(file_path);
        
        if !full_path.exists() {
            return Err(Error::Config(format!("Config file not found: {:?}", full_path)));
        }
        
        let config = self.build_config(Some(&full_path))?;
        let typed_config: T = config.try_deserialize()
            .map_err(|e| Error::Config(format!("Failed to deserialize config: {}", e)))?;
        
        Ok(BasicConfigProvider::new(
            full_path.to_string_lossy().to_string(),
            ConfigSource::File,
            typed_config,
        ))
    }
    
    /// Load configuration with environment overrides
    pub fn load_with_env<T>(&self, name: &str) -> Result<BasicConfigProvider<T>>
    where
        T: DeserializeOwned + Send + Sync + Debug + 'static,
    {
        let base_file = format!("{}.toml", name);
        let env_file = format!("{}.{}.toml", name, self.environment);
        
        let base_path = self.resolve_path(&base_file);
        let env_path = self.resolve_path(&env_file);
        
        let mut config = Config::default();
        
        // Load base config if it exists
        if base_path.exists() {
            config.merge(File::from(base_path.clone()))
                .map_err(|e| Error::Config(format!("Failed to load base config: {}", e)))?;
        }
        
        // Load environment-specific config if it exists
        if env_path.exists() {
            config.merge(File::from(env_path))
                .map_err(|e| Error::Config(format!("Failed to load env config: {}", e)))?;
        }
        
        // Add environment variable overrides
        let env_prefix = name.to_uppercase();
        config.merge(Environment::with_prefix(&env_prefix).separator("__"))
            .map_err(|e| Error::Config(format!("Failed to add env overrides: {}", e)))?;
        
        let typed_config: T = config.try_deserialize()
            .map_err(|e| Error::Config(format!("Failed to deserialize config: {}", e)))?;
        
        Ok(BasicConfigProvider::new(
            name.to_string(),
            ConfigSource::File,
            typed_config,
        ))
    }
    
    /// Load configuration from environment variables only
    pub fn load_from_env<T>(&self, prefix: &str) -> Result<BasicConfigProvider<T>>
    where
        T: DeserializeOwned + Send + Sync + Debug + 'static,
    {
        let mut config = Config::default();
        
        // Add environment variable configuration
        let env_prefix = prefix.to_uppercase();
        config.merge(Environment::with_prefix(&env_prefix).separator("__"))
            .map_err(|e| Error::Config(format!("Failed to load env config: {}", e)))?;
        
        let typed_config: T = config.try_deserialize()
            .map_err(|e| Error::Config(format!("Failed to deserialize env config: {}", e)))?;
        
        Ok(BasicConfigProvider::new(
            format!("{}_env", prefix),
            ConfigSource::Environment,
            typed_config,
        ))
    }
    
    // Helper to build a Config object from a path
    fn build_config(&self, path: Option<&PathBuf>) -> Result<Config> {
        let mut config = Config::default();
        
        if let Some(p) = path {
            config.merge(File::from(p.clone()))
                .map_err(|e| Error::Config(format!("Failed to load config file: {}", e)))?;
        }
        
        Ok(config)
    }
    
    // Helper to resolve a path relative to the base path
    fn resolve_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.base_path.join(path)
    }
} 