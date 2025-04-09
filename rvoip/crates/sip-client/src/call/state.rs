use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, warn};

use crate::error::{Error, Result};

use super::call_struct::Call;
use super::types::CallState;
use super::events::CallEvent;
use super::utils::is_valid_state_transition;

impl Call {
    /// Get the current call state
    pub async fn state(&self) -> CallState {
        *self.state_ref().read().await
    }
    
    /// Get the call registry
    pub async fn registry(&self) -> Option<Arc<dyn super::registry_interface::CallRegistryInterface + Send + Sync>> {
        self.registry_ref().read().await.clone()
    }
    
    /// Set the call registry
    pub async fn set_registry(&self, registry: Arc<dyn super::registry_interface::CallRegistryInterface + Send + Sync>) {
        *self.registry_ref().write().await = Some(registry);
    }
    
    /// Store the INVITE transaction ID
    pub async fn store_invite_transaction_id(&self, transaction_id: String) -> Result<()> {
        *self.invite_transaction_id_ref().write().await = Some(transaction_id);
        Ok(())
    }
    
    /// Update the call state
    pub async fn update_state(&self, new_state: CallState) -> Result<()> {
        let current_state = *self.state_ref().read().await;
        
        if !is_valid_state_transition(current_state, new_state) {
            return Err(Error::Call(format!(
                "Invalid state transition from {} to {}",
                current_state, new_state
            )));
        }
        
        // Update state
        *self.state_ref().write().await = new_state;
        
        // Update state watcher
        if let Err(e) = self.state_sender_ref().send(new_state) {
            warn!("Failed to update state watcher: {}", e);
        }
        
        // Handle state-specific actions
        match new_state {
            CallState::Connecting => {
                // Set connect time when transitioning to Connecting state
                *self.connect_time_ref().write().await = Some(Instant::now());
            }
            CallState::Terminated | CallState::Failed => {
                // Set end time when the call is terminated or failed
                *self.end_time_ref().write().await = Some(Instant::now());
            }
            _ => {}
        }
        
        // Send state change event
        if let Err(e) = self.event_tx_ref().send(CallEvent::StateChanged {
            call: Arc::new(self.clone()),
            previous: current_state,
            current: new_state,
        }).await {
            warn!("Failed to send state changed event: {}", e);
        }
        
        Ok(())
    }
    
    /// Simple state transition, just forwards to update_state
    pub async fn transition_to(&self, new_state: CallState) -> Result<()> {
        let current_state = *self.state_ref().read().await;
        
        if !is_valid_state_transition(current_state, new_state) {
            return Err(Error::Call(format!(
                "Invalid state transition from {} to {}",
                current_state, new_state
            )));
        }
        
        *self.state_ref().write().await = new_state;
        
        // Notify state watchers 
        if let Err(e) = self.state_sender_ref().send(new_state) {
            warn!("Failed to send state change notification: {}", e);
        }
        
        // Handle state-specific actions
        match new_state {
            CallState::Connecting => {
                // Set connect time when transitioning to Connecting state
                *self.connect_time_ref().write().await = Some(Instant::now());
            }
            CallState::Terminated | CallState::Failed => {
                // Set end time when the call is terminated or failed
                *self.end_time_ref().write().await = Some(Instant::now());
            }
            _ => {}
        }
        
        // Send event
        if let Err(e) = self.event_tx_ref().send(CallEvent::StateChanged {
            call: Arc::new(self.clone()),
            previous: current_state,
            current: new_state,
        }).await {
            warn!("Failed to send state changed event: {}", e);
        }
        
        Ok(())
    }
    
    /// Store the original INVITE request
    pub async fn store_invite_request(&self, request: rvoip_sip_core::Request) -> Result<()> {
        *self.original_invite_ref().write().await = Some(request);
        Ok(())
    }
    
    /// Store the last response received
    pub async fn store_last_response(&self, response: rvoip_sip_core::Response) -> Result<()> {
        *self.last_response_ref().write().await = Some(response);
        Ok(())
    }
    
    /// Get the remote tag
    pub async fn remote_tag(&self) -> Option<String> {
        self.remote_tag_ref().read().await.clone()
    }
    
    /// Set the remote tag
    pub async fn set_remote_tag(&self, tag: String) {
        *self.remote_tag_ref().write().await = Some(tag);
    }
    
    /// Get the active media sessions
    pub async fn media_sessions(&self) -> Vec<crate::media::MediaSession> {
        self.media_sessions_ref().read().await.clone()
    }
    
    /// Get the SIP dialog
    pub async fn dialog(&self) -> Option<rvoip_session_core::dialog::Dialog> {
        self.dialog_ref().read().await.clone()
    }
} 