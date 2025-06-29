//! Call handling - Simple wrappers around client-core call management
//!
//! This module provides easy-to-use Call and IncomingCall handles that wrap
//! the underlying client-core call functionality.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, debug, warn};

use rvoip_client_core::{ClientManager, CallId, CallState, CallInfo};
use rvoip_client_core::call::CallDirection;

use crate::{Error, Result};

/// Handle for an active call (outgoing or answered incoming)
pub struct Call {
    /// Unique call identifier
    call_id: CallId,
    /// Remote party URI
    remote_uri: String,
    /// Reference to core client (for call operations)
    core: Arc<ClientManager>,
}

impl Call {
    /// Create a new call handle
    pub(crate) fn new(call_id: CallId, remote_uri: String, core: Arc<ClientManager>) -> Self {
        Self {
            call_id,
            remote_uri,
            core,
        }
    }

    /// Get the call ID
    pub fn id(&self) -> CallId {
        self.call_id
    }

    /// Get the remote party URI
    pub fn remote_uri(&self) -> &str {
        &self.remote_uri
    }

    /// Wait for the call to be answered
    pub async fn wait_for_answer(&self) -> Result<()> {
        info!("⏳ Waiting for call {} to be answered", self.call_id);
        
        // Poll call state until it's connected or fails
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        let timeout = std::time::Duration::from_secs(60); // 60 second timeout
        let start_time = std::time::Instant::now();
        
        loop {
            interval.tick().await;
            
            // Check for timeout
            if start_time.elapsed() > timeout {
                return Err(Error::Timeout("Call answer timeout".to_string()));
            }
            
            // Get current call state
            match self.state().await {
                Ok(state) => {
                    match state {
                        rvoip_client_core::call::CallState::Connected => {
                            info!("✅ Call {} answered", self.call_id);
                            return Ok(());
                        }
                        rvoip_client_core::call::CallState::Failed => {
                            return Err(Error::Call("Call failed".to_string()));
                        }
                        rvoip_client_core::call::CallState::Cancelled => {
                            return Err(Error::Call("Call was cancelled".to_string()));
                        }
                        rvoip_client_core::call::CallState::Terminated => {
                            return Err(Error::Call("Call was terminated".to_string()));
                        }
                        _ => {
                            // Still waiting (Initiating, Proceeding, Ringing, etc.)
                            debug!("Call {} state: {:?}, continuing to wait", self.call_id, state);
                        }
                    }
                }
                Err(e) => {
                    return Err(Error::Core(format!("Failed to get call state: {}", e)));
                }
            }
        }
    }

    /// Get current call state
    pub async fn state(&self) -> Result<CallState> {
        let call_info = self.core.get_call(&self.call_id).await
            .map_err(|e| Error::Core(e.to_string()))?;
        Ok(call_info.state)
    }

    /// Check if call is active (connected)
    pub async fn is_active(&self) -> bool {
        match self.state().await {
            Ok(state) => state.is_active(),
            Err(_) => false,
        }
    }

    /// Hang up the call
    pub async fn hangup(&self) -> Result<()> {
        info!("📴 Hanging up call {}", self.call_id);
        
        self.core.hangup_call(&self.call_id).await
            .map_err(|e| Error::Core(e.to_string()))?;
        
        info!("✅ Call {} hung up", self.call_id);
        Ok(())
    }

    /// Mute/unmute microphone
    pub async fn set_microphone_mute(&self, muted: bool) -> Result<()> {
        debug!("🎤 Setting microphone mute: {} for call {}", muted, self.call_id);
        
        self.core.set_microphone_mute(&self.call_id, muted).await
            .map_err(|e| Error::Core(e.to_string()))?;
        
        Ok(())
    }

    /// Mute/unmute speaker
    pub async fn set_speaker_mute(&self, muted: bool) -> Result<()> {
        debug!("🔊 Setting speaker mute: {} for call {}", muted, self.call_id);
        
        self.core.set_speaker_mute(&self.call_id, muted).await
            .map_err(|e| Error::Core(e.to_string()))?;
        
        Ok(())
    }

    /// Get call duration (if connected)
    pub async fn duration(&self) -> Option<Duration> {
        match self.info().await {
            Ok(call_info) => {
                if let Some(connected_at) = call_info.connected_at {
                    let now = std::time::SystemTime::now();
                    let now_utc = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(
                        now.duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs()
                    );
                    
                    if let Some(_ended_at) = call_info.ended_at {
                        // For now, return None as we can't easily calculate with chrono types
                        // TODO: Add proper duration calculation when chrono is available
                        None
                    } else {
                        // For now, return None as we can't easily calculate with chrono types
                        // TODO: Add proper duration calculation when chrono is available
                        None
                    }
                } else {
                    // Call never connected
                    None
                }
            }
            Err(_) => None,
        }
    }

    /// Get detailed call information
    pub async fn info(&self) -> Result<CallInfo> {
        self.core.get_call(&self.call_id).await
            .map_err(|e| Error::Core(e.to_string()))
    }
}

/// Handle for an incoming call (before answering)
pub struct IncomingCall {
    /// Unique call identifier
    call_id: CallId,
    /// Caller information
    caller_uri: String,
    /// Caller display name (if available)
    caller_name: Option<String>,
    /// Reference to core client (for call operations)
    core: Arc<ClientManager>,
}

impl IncomingCall {
    /// Create a new incoming call handle
    pub(crate) fn new(
        call_id: CallId,
        caller_uri: String,
        caller_name: Option<String>,
        core: Arc<ClientManager>,
    ) -> Self {
        Self {
            call_id,
            caller_uri,
            caller_name,
            core,
        }
    }

    /// Get the call ID
    pub fn id(&self) -> CallId {
        self.call_id
    }

    /// Get the caller's URI
    pub fn caller(&self) -> &str {
        &self.caller_uri
    }

    /// Get the caller's display name (if available)
    pub fn caller_name(&self) -> Option<&str> {
        self.caller_name.as_deref()
    }

    /// Get a friendly caller identifier (name or URI)
    pub fn caller_id(&self) -> &str {
        self.caller_name.as_deref().unwrap_or(&self.caller_uri)
    }

    /// Answer the incoming call
    pub async fn answer(&self) -> Result<Call> {
        info!("✅ Answering incoming call {} from {}", self.call_id, self.caller());
        
        self.core.answer_call(&self.call_id).await
            .map_err(|e| Error::Core(e.to_string()))?;
        
        // Return a Call handle for the now-active call
        Ok(Call::new(self.call_id, self.caller_uri.clone(), Arc::clone(&self.core)))
    }

    /// Reject the incoming call
    pub async fn reject(&self) -> Result<()> {
        info!("❌ Rejecting incoming call {} from {}", self.call_id, self.caller());
        
        self.core.reject_call(&self.call_id).await
            .map_err(|e| Error::Core(e.to_string()))?;
        
        info!("✅ Call {} rejected", self.call_id);
        Ok(())
    }

    /// Ignore the incoming call (let it time out)
    pub async fn ignore(&self) -> Result<()> {
        info!("🔇 Ignoring incoming call {} from {}", self.call_id, self.caller());
        
        // TODO: Implement ignore (might be same as reject, or could be no action)
        self.reject().await
    }
}

/// Helper function to convert call direction to a user-friendly string
pub fn call_direction_str(direction: &CallDirection) -> &'static str {
    match direction {
        CallDirection::Outgoing => "Outgoing",
        CallDirection::Incoming => "Incoming",
    }
} 