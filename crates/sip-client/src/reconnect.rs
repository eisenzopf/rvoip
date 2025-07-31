//! Automatic reconnection logic for the SIP client
//!
//! This module provides automatic reconnection capabilities for various
//! failure scenarios including network disconnections, registration failures,
//! and call drops.

use crate::{
    error::{SipClientError, SipClientResult},
    events::{SipClientEvent, EventEmitter},
    recovery::{RecoveryManager, NetworkMetrics},
    types::CallId,
};
use std::{
    sync::Arc,
    time::Duration,
    collections::HashMap,
};
use tokio::{
    sync::{RwLock, Mutex},
    time::{sleep, timeout, Instant},
};
use tracing::{debug, error, info, warn};

/// Reconnection state for different connection types
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnectionType {
    /// SIP registration connection
    Registration,
    /// Media/RTP connection
    Media,
    /// Audio device connection
    AudioDevice,
    /// Individual call connection
    Call(CallId),
}

/// Reconnection handler for managing automatic reconnections
pub struct ReconnectionHandler {
    /// Recovery manager for error recovery
    recovery_manager: Arc<RecoveryManager>,
    
    /// Event emitter
    event_emitter: EventEmitter,
    
    /// Active reconnection tasks
    reconnection_tasks: Arc<Mutex<HashMap<ConnectionType, tokio::task::JoinHandle<()>>>>,
    
    /// Reconnection callbacks
    callbacks: Arc<RwLock<HashMap<ConnectionType, ReconnectionCallback>>>,
}

/// Callback for reconnection attempts
type ReconnectionCallback = Arc<dyn Fn() -> futures::future::BoxFuture<'static, SipClientResult<()>> + Send + Sync>;

impl ReconnectionHandler {
    /// Create a new reconnection handler
    pub fn new(
        recovery_manager: Arc<RecoveryManager>,
        event_emitter: EventEmitter,
    ) -> Self {
        Self {
            recovery_manager,
            event_emitter,
            reconnection_tasks: Arc::new(Mutex::new(HashMap::new())),
            callbacks: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Register a reconnection callback for a connection type
    pub async fn register_callback(
        &self,
        connection_type: ConnectionType,
        callback: impl Fn() -> futures::future::BoxFuture<'static, SipClientResult<()>> + Send + Sync + 'static,
    ) {
        let mut callbacks = self.callbacks.write().await;
        callbacks.insert(connection_type, Arc::new(callback));
    }
    
    /// Trigger reconnection for a specific connection type
    pub async fn trigger_reconnection(
        &self,
        connection_type: ConnectionType,
        error: SipClientError,
    ) -> SipClientResult<()> {
        info!("Triggering reconnection for {:?} due to: {}", connection_type, error);
        
        // Check if reconnection is already in progress
        let mut tasks = self.reconnection_tasks.lock().await;
        if tasks.contains_key(&connection_type) {
            debug!("Reconnection already in progress for {:?}", connection_type);
            return Ok(());
        }
        
        // Get the callback
        let callbacks = self.callbacks.read().await;
        let callback = callbacks.get(&connection_type)
            .ok_or_else(|| SipClientError::Internal {
                message: format!("No reconnection callback registered for {:?}", connection_type),
            })?
            .clone();
        
        // Create reconnection task
        let connection_type_clone = connection_type.clone();
        let recovery_manager = self.recovery_manager.clone();
        let event_emitter = self.event_emitter.clone();
        let tasks_arc = self.reconnection_tasks.clone();
        
        let task = tokio::spawn(async move {
            // Use recovery manager for the reconnection attempt
            let component_name = format!("reconnect_{:?}", connection_type_clone);
            let result = recovery_manager.handle_error(
                &component_name,
                &error,
                move || callback(),
            ).await;
            
            if let Err(e) = result {
                error!("Reconnection failed for {:?}: {}", connection_type_clone, e);
                event_emitter.emit(SipClientEvent::ReconnectionFailed {
                    connection_type: format!("{:?}", connection_type_clone),
                    error: e.to_string(),
                });
            } else {
                info!("Reconnection initiated for {:?}", connection_type_clone);
            }
            
            // Remove task from active tasks
            let mut tasks = tasks_arc.lock().await;
            tasks.remove(&connection_type_clone);
        });
        
        tasks.insert(connection_type, task);
        Ok(())
    }
    
    /// Stop all reconnection attempts
    pub async fn stop_all(&self) {
        let mut tasks = self.reconnection_tasks.lock().await;
        for (connection_type, task) in tasks.drain() {
            debug!("Stopping reconnection for {:?}", connection_type);
            task.abort();
        }
    }
    
    /// Check if reconnection is in progress for a connection type
    pub async fn is_reconnecting(&self, connection_type: &ConnectionType) -> bool {
        let tasks = self.reconnection_tasks.lock().await;
        tasks.contains_key(connection_type)
    }
}

/// Registration reconnection logic
pub struct RegistrationReconnector {
    /// Event emitter
    event_emitter: EventEmitter,
    
    /// Registration callback
    register_callback: Arc<dyn Fn() -> futures::future::BoxFuture<'static, SipClientResult<()>> + Send + Sync>,
}

impl RegistrationReconnector {
    /// Create a new registration reconnector
    pub fn new(
        event_emitter: EventEmitter,
        register_callback: impl Fn() -> futures::future::BoxFuture<'static, SipClientResult<()>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            event_emitter,
            register_callback: Arc::new(register_callback),
        }
    }
    
    /// Perform registration reconnection
    pub async fn reconnect(&self) -> SipClientResult<()> {
        info!("Attempting to re-register with SIP server");
        
        // Attempt registration with timeout
        match timeout(Duration::from_secs(30), (self.register_callback)()).await {
            Ok(Ok(())) => {
                info!("Re-registration successful");
                self.event_emitter.emit(SipClientEvent::RegistrationRestored);
                Ok(())
            }
            Ok(Err(e)) => {
                error!("Re-registration failed: {}", e);
                Err(e)
            }
            Err(_) => {
                error!("Re-registration timed out");
                Err(SipClientError::Timeout { seconds: 30 })
            }
        }
    }
}

/// Call reconnection logic
pub struct CallReconnector {
    /// Event emitter
    event_emitter: EventEmitter,
    
    /// Call ID
    call_id: CallId,
}

impl CallReconnector {
    /// Create a new call reconnector
    pub fn new(event_emitter: EventEmitter, call_id: CallId) -> Self {
        Self {
            event_emitter,
            call_id,
        }
    }
    
    /// Attempt to recover a dropped call
    pub async fn recover_call(
        &self,
        reinvite_callback: impl Fn() -> futures::future::BoxFuture<'static, SipClientResult<()>>,
    ) -> SipClientResult<()> {
        info!("Attempting to recover call: {}", self.call_id);
        
        // First, check if we can send a RE-INVITE
        match timeout(Duration::from_secs(10), reinvite_callback()).await {
            Ok(Ok(())) => {
                info!("Call recovery successful via RE-INVITE");
                self.event_emitter.emit(SipClientEvent::CallRecovered {
                    call_id: self.call_id,
                });
                Ok(())
            }
            Ok(Err(e)) => {
                warn!("Call recovery failed: {}", e);
                // Call is likely terminated, notify the application
                self.event_emitter.emit(SipClientEvent::CallLost {
                    call_id: self.call_id,
                    reason: e.to_string(),
                });
                Err(e)
            }
            Err(_) => {
                error!("Call recovery timed out");
                self.event_emitter.emit(SipClientEvent::CallLost {
                    call_id: self.call_id,
                    reason: "Recovery timeout".to_string(),
                });
                Err(SipClientError::Timeout { seconds: 10 })
            }
        }
    }
}

/// Audio device reconnection logic
pub struct AudioDeviceReconnector {
    /// Event emitter
    event_emitter: EventEmitter,
    
    /// Audio manager
    audio_manager: Arc<rvoip_audio_core::AudioDeviceManager>,
}

impl AudioDeviceReconnector {
    /// Create a new audio device reconnector
    pub fn new(
        event_emitter: EventEmitter,
        audio_manager: Arc<rvoip_audio_core::AudioDeviceManager>,
    ) -> Self {
        Self {
            event_emitter,
            audio_manager,
        }
    }
    
    /// Attempt to reconnect audio devices
    pub async fn reconnect_devices(&self) -> SipClientResult<()> {
        info!("Attempting to reconnect audio devices");
        
        // List available devices
        let input_devices = self.audio_manager
            .list_devices(rvoip_audio_core::AudioDirection::Input)
            .await
            .map_err(|e| SipClientError::AudioDevice {
                message: format!("Failed to list input devices: {}", e),
            })?;
            
        let output_devices = self.audio_manager
            .list_devices(rvoip_audio_core::AudioDirection::Output)
            .await
            .map_err(|e| SipClientError::AudioDevice {
                message: format!("Failed to list output devices: {}", e),
            })?;
        
        if input_devices.is_empty() || output_devices.is_empty() {
            warn!("No audio devices available");
            return Err(SipClientError::AudioDevice {
                message: "No audio devices available".to_string(),
            });
        }
        
        // Try to get default devices
        let default_input = self.audio_manager
            .get_default_device(rvoip_audio_core::AudioDirection::Input)
            .await
            .ok();
            
        let default_output = self.audio_manager
            .get_default_device(rvoip_audio_core::AudioDirection::Output)
            .await
            .ok();
        
        if default_input.is_some() && default_output.is_some() {
            info!("Audio devices reconnected successfully");
            self.event_emitter.emit(SipClientEvent::AudioDevicesRestored);
            Ok(())
        } else {
            Err(SipClientError::AudioDevice {
                message: "Failed to get default audio devices".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_connection_type_equality() {
        let call_id1 = CallId::new_v4();
        let call_id2 = CallId::new_v4();
        
        assert_eq!(ConnectionType::Registration, ConnectionType::Registration);
        assert_eq!(ConnectionType::Call(call_id1), ConnectionType::Call(call_id1));
        assert_ne!(ConnectionType::Call(call_id1), ConnectionType::Call(call_id2));
        assert_ne!(ConnectionType::Registration, ConnectionType::Media);
    }
    
    #[tokio::test]
    async fn test_reconnection_handler_creation() {
        let recovery_manager = Arc::new(RecoveryManager::new(
            crate::recovery::RecoveryConfig::default(),
            EventEmitter::default(),
        ));
        let event_emitter = EventEmitter::default();
        
        let handler = ReconnectionHandler::new(recovery_manager, event_emitter);
        
        // Test that no reconnections are in progress initially
        assert!(!handler.is_reconnecting(&ConnectionType::Registration).await);
        assert!(!handler.is_reconnecting(&ConnectionType::Media).await);
    }
}