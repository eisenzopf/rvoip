use crate::errors::types::{Error, Result};
use crate::config::provider::{ConfigProvider, ConfigSource};
use crate::events::bus::EventBus;
use std::fmt::Debug;
use std::sync::{Arc, RwLock};
use std::any::Any;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use std::time::Duration;
use tokio::time::interval;
use tokio::sync::mpsc;

/// Configuration change event
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigChangedEvent {
    /// Name of the configuration that changed
    pub name: String,
    /// Source of the configuration
    pub source: ConfigSource,
    /// Timestamp when the change occurred (in seconds since epoch)
    #[serde(default)]
    pub timestamp_secs: u64,
}

impl ConfigChangedEvent {
    /// Create a new configuration change event with current time
    pub fn new(name: String, source: ConfigSource) -> Self {
        ConfigChangedEvent {
            name,
            source,
            timestamp_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

impl crate::events::types::Event for ConfigChangedEvent {
    fn event_type() -> &'static str {
        "config_changed"
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Configuration that can be updated at runtime
pub struct DynamicConfig<T: 'static> {
    name: String,
    source: ConfigSource,
    config: Arc<RwLock<T>>,
    event_bus: Option<EventBus>,
    last_update: Arc<RwLock<u64>>,
}

impl<T: DeserializeOwned + Send + Sync + Clone + Debug + 'static> DynamicConfig<T> {
    /// Create a new dynamic configuration
    pub fn new<S: Into<String>>(name: S, source: ConfigSource, initial_config: T) -> Self {
        DynamicConfig {
            name: name.into(),
            source,
            config: Arc::new(RwLock::new(initial_config)),
            event_bus: None,
            last_update: Arc::new(RwLock::new(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs())),
        }
    }
    
    /// Create a new dynamic configuration with an event bus
    pub fn with_events<S: Into<String>>(
        name: S,
        source: ConfigSource,
        initial_config: T,
        event_bus: EventBus,
    ) -> Self {
        DynamicConfig {
            name: name.into(),
            source,
            config: Arc::new(RwLock::new(initial_config)),
            event_bus: Some(event_bus),
            last_update: Arc::new(RwLock::new(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs())),
        }
    }
    
    /// Get a clone of the current configuration
    pub fn get_config(&self) -> T {
        self.config.read().unwrap().clone()
    }
    
    /// Update the configuration
    pub fn update(&self, new_config: T) -> Result<()> {
        let mut config = self.config.write().unwrap();
        *config = new_config;
        *self.last_update.write().unwrap() = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        // Publish event if an event bus is available
        if let Some(event_bus) = &self.event_bus {
            let event = ConfigChangedEvent::new(self.name.clone(), self.source);
            
            // Fire and forget
            let event_bus = event_bus.clone();
            tokio::spawn(async move {
                let _ = event_bus.publish(event).await;
            });
        }
        
        Ok(())
    }
    
    /// Set up automatic refresh from a loader function
    pub fn auto_refresh<F>(&self, refresh_interval: Duration, loader: F) -> mpsc::Sender<()>
    where
        F: Fn() -> Result<T> + Send + 'static
    {
        let (tx, mut rx) = mpsc::channel::<()>(1);
        let config = self.clone();
        
        tokio::spawn(async move {
            let mut interval = interval(refresh_interval);
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Ok(new_config) = loader() {
                            let _ = config.update(new_config);
                        }
                    }
                    _ = rx.recv() => {
                        // Channel closed, stop refreshing
                        break;
                    }
                }
            }
        });
        
        tx
    }
}

impl<T: DeserializeOwned + Send + Sync + Clone + Debug + 'static> Clone for DynamicConfig<T> {
    fn clone(&self) -> Self {
        DynamicConfig {
            name: self.name.clone(),
            source: self.source,
            config: Arc::clone(&self.config),
            event_bus: self.event_bus.clone(),
            last_update: Arc::clone(&self.last_update),
        }
    }
}

impl<T: DeserializeOwned + Send + Sync + Clone + Debug + Serialize + 'static> ConfigProvider for DynamicConfig<T> {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn source(&self) -> ConfigSource {
        self.source
    }
    
    fn get<U: DeserializeOwned>(&self, _key: &str) -> Result<U> {
        let config = self.config.read().unwrap();
        let value = serde_json::to_value(&*config)
            .map_err(|e| Error::Config(format!("Failed to serialize config: {}", e)))?;
            
        serde_json::from_value(value)
            .map_err(|e| Error::Config(format!("Failed to deserialize config: {}", e)))
    }
    
    fn get_raw(&self, _key: &str) -> Result<Box<dyn Any>> {
        let config = self.config.read().unwrap().clone();
        Ok(Box::new(config))
    }
    
    fn has(&self, _key: &str) -> bool {
        true // The dynamic provider doesn't support keys
    }
    
    fn keys(&self) -> Vec<String> {
        vec![] // The dynamic provider doesn't expose keys
    }
    
    fn reload(&self) -> Result<()> {
        // This basic implementation doesn't know how to reload itself
        // A real implementation would track its data source and reload from there
        Err(Error::Config("Reload not supported without a reload function".to_string()))
    }
}

impl<T: Debug + 'static> Debug for DynamicConfig<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicConfig")
            .field("name", &self.name)
            .field("source", &self.source)
            .field("config", &self.config)
            .field("last_update", &self.last_update)
            // Skip event_bus which doesn't implement Debug
            .finish()
    }
} 