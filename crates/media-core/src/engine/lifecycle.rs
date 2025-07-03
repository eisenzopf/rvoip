//! MediaEngine lifecycle management
//!
//! This module handles the lifecycle of the MediaEngine including startup,
//! shutdown, and state management.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};
use crate::error::{Result, Error};

/// MediaEngine state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    /// Engine is not initialized
    Uninitialized,
    /// Engine is starting up
    Starting,
    /// Engine is running and ready
    Running,
    /// Engine is shutting down
    Stopping,
    /// Engine has stopped
    Stopped,
    /// Engine encountered an error
    Error,
}

impl Default for EngineState {
    fn default() -> Self {
        Self::Uninitialized
    }
}

impl std::fmt::Display for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineState::Uninitialized => write!(f, "uninitialized"),
            EngineState::Starting => write!(f, "starting"),
            EngineState::Running => write!(f, "running"),
            EngineState::Stopping => write!(f, "stopping"),
            EngineState::Stopped => write!(f, "stopped"),
            EngineState::Error => write!(f, "error"),
        }
    }
}

/// Lifecycle manager for MediaEngine
#[derive(Debug)]
pub struct LifecycleManager {
    /// Current engine state
    state: Arc<RwLock<EngineState>>,
    /// Shutdown signal sender (using Mutex for interior mutability)
    shutdown_tx: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// Shutdown signal receiver
    shutdown_rx: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl LifecycleManager {
    /// Create a new lifecycle manager
    pub fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        
        Self {
            state: Arc::new(RwLock::new(EngineState::Uninitialized)),
            shutdown_tx: Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx))),
            shutdown_rx: tokio::sync::Mutex::new(Some(shutdown_rx)),
        }
    }
    
    /// Get the current engine state
    pub async fn state(&self) -> EngineState {
        *self.state.read().await
    }
    
    /// Check if the engine is running
    pub async fn is_running(&self) -> bool {
        matches!(self.state().await, EngineState::Running)
    }
    
    /// Check if the engine can be started
    pub async fn can_start(&self) -> bool {
        matches!(
            self.state().await,
            EngineState::Uninitialized | EngineState::Stopped
        )
    }
    
    /// Check if the engine can be stopped
    pub async fn can_stop(&self) -> bool {
        matches!(self.state().await, EngineState::Running)
    }
    
    /// Start the engine lifecycle
    pub async fn start(&self) -> Result<()> {
        let mut state = self.state.write().await;
        
        match *state {
            EngineState::Uninitialized | EngineState::Stopped => {
                info!("Starting MediaEngine...");
                *state = EngineState::Starting;
                
                // Perform startup operations
                match self.perform_startup().await {
                    Ok(()) => {
                        *state = EngineState::Running;
                        info!("MediaEngine started successfully");
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to start MediaEngine: {}", e);
                        *state = EngineState::Error;
                        Err(e)
                    }
                }
            }
            current_state => {
                warn!("Cannot start MediaEngine in state: {}", current_state);
                Err(Error::config(format!(
                    "Cannot start engine in state: {}",
                    current_state
                )))
            }
        }
    }
    
    /// Stop the engine lifecycle
    pub async fn stop(&self) -> Result<()> {
        let mut state = self.state.write().await;
        
        match *state {
            EngineState::Running => {
                info!("Stopping MediaEngine...");
                *state = EngineState::Stopping;
                
                // Perform shutdown operations
                match self.perform_shutdown().await {
                    Ok(()) => {
                        *state = EngineState::Stopped;
                        info!("MediaEngine stopped successfully");
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to stop MediaEngine cleanly: {}", e);
                        *state = EngineState::Error;
                        Err(e)
                    }
                }
            }
            current_state => {
                warn!("Cannot stop MediaEngine in state: {}", current_state);
                Err(Error::config(format!(
                    "Cannot stop engine in state: {}",
                    current_state
                )))
            }
        }
    }
    
    /// Force shutdown (used in emergency situations)
    pub async fn force_shutdown(&self) {
        warn!("Force shutdown requested for MediaEngine");
        let mut state = self.state.write().await;
        *state = EngineState::Stopped;
        
        // Send shutdown signal if available
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(());
        }
    }
    
    /// Wait for shutdown signal
    pub async fn wait_for_shutdown(&mut self) -> Result<()> {
        if let Some(rx) = self.shutdown_rx.lock().await.take() {
            match rx.await {
                Ok(()) => {
                    debug!("Shutdown signal received");
                    Ok(())
                }
                Err(_) => {
                    warn!("Shutdown signal sender dropped");
                    Ok(()) // Not an error, just means no signal will come
                }
            }
        } else {
            warn!("Shutdown receiver already taken");
            Ok(())
        }
    }
    
    /// Perform startup operations
    async fn perform_startup(&self) -> Result<()> {
        debug!("Performing MediaEngine startup operations");
        
        // TODO: Initialize components in order:
        // 1. Audio processing components
        // 2. Codec registry
        // 3. Quality monitoring
        // 4. Integration bridges
        // 5. Worker threads
        
        // For now, just simulate startup
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        
        debug!("MediaEngine startup operations completed");
        Ok(())
    }
    
    /// Perform shutdown operations
    async fn perform_shutdown(&self) -> Result<()> {
        debug!("Performing MediaEngine shutdown operations");
        
        // TODO: Shutdown components in reverse order:
        // 1. Stop accepting new sessions
        // 2. Close existing sessions gracefully
        // 3. Stop worker threads
        // 4. Shutdown integration bridges
        // 5. Clean up resources
        
        // For now, just simulate shutdown
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        
        debug!("MediaEngine shutdown operations completed");
        Ok(())
    }
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new()
    }
} 