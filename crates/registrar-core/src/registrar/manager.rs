//! Registration manager for handling expiry and cleanup

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, info};
use crate::registrar::UserRegistry;

/// Manages registration lifecycle and expiry
pub struct RegistrationManager {
    registry: Arc<UserRegistry>,
    running: Arc<RwLock<bool>>,
    handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl RegistrationManager {
    pub fn new(registry: Arc<UserRegistry>) -> Self {
        Self {
            registry,
            running: Arc::new(RwLock::new(false)),
            handle: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Start the expiry management task
    pub async fn start(&self) {
        let mut running = self.running.write().await;
        if *running {
            debug!("Registration manager already running");
            return;
        }
        
        *running = true;
        let registry = self.registry.clone();
        let running_flag = self.running.clone();
        
        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(30));
            
            while *running_flag.read().await {
                ticker.tick().await;
                
                // Expire old registrations
                let expired = registry.expire_registrations().await;
                if !expired.is_empty() {
                    info!("Expired {} registrations", expired.len());
                }
            }
        });
        
        *self.handle.write().await = Some(handle);
        info!("Registration manager started");
    }
    
    /// Stop the expiry management task
    pub async fn stop(&self) {
        *self.running.write().await = false;
        
        if let Some(handle) = self.handle.write().await.take() {
            handle.abort();
            info!("Registration manager stopped");
        }
    }
}