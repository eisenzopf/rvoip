//! Call control operations for the client-core library
//! 
//! This module provides comprehensive call control functionality for managing active VoIP calls.
//! It includes operations for call hold/resume, DTMF transmission, call transfer (both blind
//! and attended), and capability management.
//!
//! # Call Control Features
//!
//! ## Hold and Resume Operations
//! - **Hold Call**: Put a call on hold (mute audio, send hold indication)
//! - **Resume Call**: Resume a call from hold state
//! - **Hold Status**: Check if a call is currently on hold
//!
//! ## DTMF (Dual-Tone Multi-Frequency) Support
//! - **Send DTMF**: Transmit dial tones during active calls
//! - **Validation**: Ensure DTMF digits are valid (0-9, A-D, *, #)
//! - **History Tracking**: Maintain DTMF transmission history
//!
//! ## Call Transfer Operations
//! - **Blind Transfer**: Transfer call directly to destination without consultation
//! - **Attended Transfer**: Consultation-based transfer with hold and release
//! - **URI Validation**: Ensure transfer targets are valid SIP or TEL URIs
//!
//! ## Capability Management
//! - **Dynamic Capabilities**: Determine available operations based on call state
//! - **State-Aware**: Operations adapt to current call conditions
//! - **Permission Checking**: Validate operations before execution
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────┐
//! │   Application Layer     │
//! └───────────┬─────────────┘
//!             │
//! ┌───────────▼─────────────┐
//! │   Call Controls         │ ◄── This Module
//! │ ┌─────────────────────┐ │
//! │ │ Hold/Resume        │ │  • State management
//! │ │ DTMF Transmission  │ │  • Session coordination
//! │ │ Call Transfer      │ │  • Event notification
//! │ │ Capabilities       │ │  • Error handling
//! │ └─────────────────────┘ │
//! └───────────┬─────────────┘
//!             │
//! ┌───────────▼─────────────┐
//! │    session-core         │
//! │  SessionControl API     │
//! └─────────────────────────┘
//! ```
//!
//! # Usage Examples
//!
//! ## Basic Call Hold and Resume
//!
//! ```rust
//! use rvoip_client_core::{ClientManager, ClientConfig, CallId};
//! 
//! async fn hold_resume_example() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ClientConfig::new()
//!         .with_sip_addr("127.0.0.1:5060".parse()?);
//!     let client = ClientManager::new(config).await?;
//!     client.start().await?;
//!     
//!     // Assume we have an active call
//!     let call_id = CallId::new_v4();
//!     
//!     // Check capabilities first
//!     if let Ok(caps) = client.get_call_capabilities(&call_id).await {
//!         if caps.can_hold {
//!             // Put call on hold
//!             if let Err(e) = client.hold_call(&call_id).await {
//!                 println!("Hold failed: {}", e);
//!             }
//!             
//!             // Check hold status
//!             if let Ok(on_hold) = client.is_call_on_hold(&call_id).await {
//!                 println!("Call on hold: {}", on_hold);
//!             }
//!             
//!             // Resume call
//!             if let Err(e) = client.resume_call(&call_id).await {
//!                 println!("Resume failed: {}", e);
//!             }
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## DTMF Transmission
//!
//! ```rust
//! use rvoip_client_core::{ClientManager, ClientConfig, CallId};
//! 
//! async fn dtmf_example() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ClientConfig::new()
//!         .with_sip_addr("127.0.0.1:5061".parse()?);
//!     let client = ClientManager::new(config).await?;
//!     client.start().await?;
//!     
//!     let call_id = CallId::new_v4();
//!     
//!     // Send individual DTMF digits
//!     if let Err(e) = client.send_dtmf(&call_id, "1").await {
//!         println!("DTMF failed: {}", e);
//!     }
//!     
//!     // Send multiple digits
//!     if let Err(e) = client.send_dtmf(&call_id, "123*456#").await {
//!         println!("DTMF sequence failed: {}", e);
//!     }
//!     
//!     // Send extended DTMF (including A-D)
//!     if let Err(e) = client.send_dtmf(&call_id, "123A456B").await {
//!         println!("Extended DTMF failed: {}", e);
//!     }
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Call Transfer Operations
//!
//! ```rust
//! use rvoip_client_core::{ClientManager, ClientConfig, CallId};
//! 
//! async fn transfer_example() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ClientConfig::new()
//!         .with_sip_addr("127.0.0.1:5062".parse()?);
//!     let client = ClientManager::new(config).await?;
//!     client.start().await?;
//!     
//!     let call_id1 = CallId::new_v4();
//!     let call_id2 = CallId::new_v4();
//!     
//!     // Blind transfer to SIP URI
//!     if let Err(e) = client.transfer_call(&call_id1, "sip:transfer@example.com").await {
//!         println!("Blind transfer failed: {}", e);
//!     }
//!     
//!     // Attended transfer between two calls
//!     if let Err(e) = client.attended_transfer(&call_id1, &call_id2).await {
//!         println!("Attended transfer failed: {}", e);
//!     }
//!     
//!     Ok(())
//! }
//! ```

use chrono::Utc;

// Import session-core APIs
use rvoip_session_core::api::{
    SessionControl,
};

// Import client-core types
use crate::{
    ClientResult, ClientError,
    call::CallId,
};

use crate::client::types::*;

/// Call control operations implementation for ClientManager
impl super::manager::ClientManager {
    // ===== PRIORITY 3.2: CALL CONTROL OPERATIONS =====
    
    /// Put an active call on hold
    /// 
    /// This method places a call in hold state, which typically mutes the audio stream
    /// and may play hold music to the remote party. The call remains connected but
    /// media transmission is suspended until the call is resumed.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to put on hold
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the call was successfully placed on hold.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// * `ClientError::InvalidCallState` - If the call is not in a holdable state
    /// * `ClientError::CallSetupFailed` - If the hold operation fails
    /// 
    /// # State Requirements
    /// 
    /// The call must be in the `Connected` state to be placed on hold. Calls in
    /// other states (such as `Ringing`, `Terminated`, etc.) cannot be held.
    /// 
    /// # Examples
    /// 
    /// ## Basic Hold Operation
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn hold_active_call() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5060".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Put call on hold
    ///     match client.hold_call(&call_id).await {
    ///         Ok(()) => {
    ///             println!("Call {} successfully placed on hold", call_id);
    ///             
    ///             // Verify hold status
    ///             if let Ok(on_hold) = client.is_call_on_hold(&call_id).await {
    ///                 assert!(on_hold);
    ///             }
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Failed to hold call: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Hold with Capability Check
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn safe_hold_call() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5061".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Check if call can be held before attempting
    ///     if let Ok(capabilities) = client.get_call_capabilities(&call_id).await {
    ///         if capabilities.can_hold {
    ///             client.hold_call(&call_id).await?;
    ///             println!("Call successfully held");
    ///         } else {
    ///             println!("Call cannot be held in current state");
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Error Handling
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId, ClientError};
    /// 
    /// async fn hold_with_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5062".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     match client.hold_call(&call_id).await {
    ///         Ok(()) => {
    ///             println!("✅ Call placed on hold successfully");
    ///         }
    ///         Err(ClientError::CallNotFound { .. }) => {
    ///             println!("❌ Call not found - may have been terminated");
    ///         }
    ///         Err(ClientError::InvalidCallState { current_state, .. }) => {
    ///             println!("❌ Cannot hold call in state: {:?}", current_state);
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Hold operation failed: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with hold status and timestamp
    /// - Triggers media events for hold state change
    /// - May play hold music to the remote party (depending on server configuration)
    /// - Audio transmission is suspended for the local party
    pub async fn hold_call(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to hold
                }
                crate::call::CallState::Terminated | 
                crate::call::CallState::Failed | 
                crate::call::CallState::Cancelled => {
                    return Err(ClientError::InvalidCallState { 
                        call_id: *call_id, 
                        current_state: call_info.state.clone() 
                    });
                }
                _ => {
                    return Err(ClientError::InvalidCallStateGeneric { 
                        expected: "Connected".to_string(),
                        actual: format!("{:?}", call_info.state)
                    });
                }
            }
        }
            
        // Use session-core hold functionality
        SessionControl::hold_session(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to hold call: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("on_hold".to_string(), "true".to_string());
            call_info.metadata.insert("hold_initiated_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent for hold state change
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::HoldStateChanged { on_hold: true },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Put call {} on hold", call_id);
        Ok(())
    }
    
    /// Resume a call from hold state
    /// 
    /// This method resumes a previously held call, restoring audio transmission
    /// and returning the call to its active connected state. The call must have
    /// been previously placed on hold using `hold_call()`.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to resume
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the call was successfully resumed from hold.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// * `ClientError::CallSetupFailed` - If the resume operation fails
    /// 
    /// # State Requirements
    /// 
    /// The call should be in a held state to be resumed. However, this method
    /// will attempt to resume any call that exists, as the session-core layer
    /// handles the actual state validation.
    /// 
    /// # Examples
    /// 
    /// ## Basic Resume Operation
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn resume_held_call() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5063".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // First put call on hold (would normally be done earlier)
    ///     if let Err(e) = client.hold_call(&call_id).await {
    ///         println!("Hold failed: {}", e);
    ///         return Ok(());
    ///     }
    ///     
    ///     // Verify call is on hold
    ///     if let Ok(on_hold) = client.is_call_on_hold(&call_id).await {
    ///         println!("Call on hold: {}", on_hold);
    ///     }
    ///     
    ///     // Resume the call
    ///     match client.resume_call(&call_id).await {
    ///         Ok(()) => {
    ///             println!("Call {} successfully resumed", call_id);
    ///             
    ///             // Verify call is no longer on hold
    ///             if let Ok(on_hold) = client.is_call_on_hold(&call_id).await {
    ///                 assert!(!on_hold);
    ///             }
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Failed to resume call: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Resume with Capability Check
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn safe_resume_call() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5064".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Check if call can be resumed before attempting
    ///     if let Ok(capabilities) = client.get_call_capabilities(&call_id).await {
    ///         if capabilities.can_resume {
    ///             client.resume_call(&call_id).await?;
    ///             println!("Call successfully resumed");
    ///         } else {
    ///             println!("Call cannot be resumed (not on hold or wrong state)");
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Hold/Resume Cycle
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// use tokio::time::{sleep, Duration};
    /// 
    /// async fn hold_resume_cycle() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5065".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Put call on hold
    ///     if client.hold_call(&call_id).await.is_ok() {
    ///         println!("Call placed on hold");
    ///         
    ///         // Wait briefly (in real app, this might be much longer)
    ///         sleep(Duration::from_millis(100)).await;
    ///         
    ///         // Resume the call
    ///         if client.resume_call(&call_id).await.is_ok() {
    ///             println!("Call resumed from hold");
    ///             
    ///             // Verify final state
    ///             if let Ok(on_hold) = client.is_call_on_hold(&call_id).await {
    ///                 println!("Final hold state: {}", on_hold);
    ///             }
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata to remove hold status and add resume timestamp
    /// - Triggers media events for hold state change (on_hold: false)
    /// - Resumes audio transmission between parties
    /// - May stop hold music playback (if configured)
    pub async fn resume_call(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core resume functionality
        SessionControl::resume_session(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to resume call: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("on_hold".to_string(), "false".to_string());
            call_info.metadata.insert("resumed_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent for hold state change
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::HoldStateChanged { on_hold: false },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Resumed call {} from hold", call_id);
        Ok(())
    }
    
    /// Check if a call is currently on hold
    /// 
    /// This method queries the hold status of a call by examining its metadata.
    /// It returns `true` if the call is currently on hold, `false` if active,
    /// or an error if the call doesn't exist.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to check
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(true)` if the call is on hold, `Ok(false)` if active.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// 
    /// # Examples
    /// 
    /// ## Basic Hold Status Check
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn check_hold_status() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5066".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Check initial status
    ///     match client.is_call_on_hold(&call_id).await {
    ///         Ok(on_hold) => {
    ///             println!("Call on hold: {}", on_hold);
    ///             assert!(!on_hold); // Should be false initially
    ///         }
    ///         Err(e) => {
    ///             println!("Error checking hold status: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Hold Status Monitoring
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn monitor_hold_status() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5067".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Check status before hold
    ///     if let Ok(status) = client.is_call_on_hold(&call_id).await {
    ///         println!("Before hold: {}", status);
    ///     }
    ///     
    ///     // Put call on hold (ignore errors for doc test)
    ///     let _ = client.hold_call(&call_id).await;
    ///     
    ///     // Check status after hold
    ///     if let Ok(status) = client.is_call_on_hold(&call_id).await {
    ///         println!("After hold: {}", status);
    ///         if status {
    ///             println!("✅ Call is now on hold");
    ///         }
    ///     }
    ///     
    ///     // Resume call (ignore errors for doc test)
    ///     let _ = client.resume_call(&call_id).await;
    ///     
    ///     // Check status after resume
    ///     if let Ok(status) = client.is_call_on_hold(&call_id).await {
    ///         println!("After resume: {}", status);
    ///         if !status {
    ///             println!("✅ Call is now active");
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Conditional Operations Based on Hold Status
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn conditional_operations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5068".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Perform different actions based on hold status
    ///     match client.is_call_on_hold(&call_id).await {
    ///         Ok(true) => {
    ///             println!("Call is on hold - offering to resume");
    ///             // Could resume the call
    ///             // client.resume_call(&call_id).await?;
    ///         }
    ///         Ok(false) => {
    ///             println!("Call is active - offering to hold");
    ///             // Could put call on hold
    ///             // client.hold_call(&call_id).await?;
    ///         }
    ///         Err(e) => {
    ///             println!("Cannot check hold status: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Implementation Notes
    /// 
    /// This method checks the call's metadata for an "on_hold" field that is
    /// set to "true" when a call is placed on hold and "false" when resumed.
    /// If the metadata field doesn't exist, the call is considered not on hold.
    pub async fn is_call_on_hold(&self, call_id: &CallId) -> ClientResult<bool> {
        if let Some(call_info) = self.call_info.get(call_id) {
            // Check metadata for hold status
            let on_hold = call_info.metadata.get("on_hold")
                .map(|s| s == "true")
                .unwrap_or(false);
            Ok(on_hold)
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }
    
    /// Send DTMF (Dual-Tone Multi-Frequency) digits during an active call
    /// 
    /// This method transmits DTMF tones to the remote party during a connected call.
    /// DTMF is commonly used for navigating phone menus, entering PINs, or other
    /// interactive voice response (IVR) interactions.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to send DTMF to
    /// * `digits` - A string containing valid DTMF characters to transmit
    /// 
    /// # Valid DTMF Characters
    /// 
    /// - **Digits**: `0-9` (standard numeric keypad)
    /// - **Letters**: `A-D` (extended DTMF for special applications)
    /// - **Symbols**: `*` (star) and `#` (pound/hash)
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the DTMF digits were successfully transmitted.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// * `ClientError::InvalidCallState` - If the call is not in a connected state
    /// * `ClientError::InvalidConfiguration` - If digits are empty or contain invalid characters
    /// * `ClientError::CallSetupFailed` - If the DTMF transmission fails
    /// 
    /// # State Requirements
    /// 
    /// The call must be in the `Connected` state to send DTMF. Calls that are
    /// ringing, terminated, or in other states cannot transmit DTMF tones.
    /// 
    /// # Examples
    /// 
    /// ## Basic DTMF Transmission
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn send_basic_dtmf() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5069".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Send individual digit
    ///     match client.send_dtmf(&call_id, "1").await {
    ///         Ok(()) => println!("✅ Sent DTMF digit '1'"),
    ///         Err(e) => println!("❌ DTMF failed: {}", e),
    ///     }
    ///     
    ///     // Send multiple digits
    ///     if client.send_dtmf(&call_id, "123").await.is_ok() {
    ///         println!("✅ Sent DTMF sequence '123'");
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Interactive Menu Navigation
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// use tokio::time::{sleep, Duration};
    /// 
    /// async fn navigate_menu() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5070".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Navigate through a typical phone menu
    ///     let menu_sequence = [
    ///         ("1", "Select English"),
    ///         ("2", "Customer Service"),
    ///         ("3", "Account Information"),
    ///         ("*", "Return to previous menu"),
    ///         ("#", "End menu navigation"),
    ///     ];
    ///     
    ///     for (digit, description) in menu_sequence {
    ///         if client.send_dtmf(&call_id, digit).await.is_ok() {
    ///             println!("📞 Sent '{}' - {}", digit, description);
    ///             
    ///             // Wait between menu selections
    ///             sleep(Duration::from_millis(50)).await;
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## PIN Entry with Validation
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn enter_pin() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5071".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Example PIN entry
    ///     let pin = "1234";
    ///     
    ///     // Validate PIN before sending
    ///     for ch in pin.chars() {
    ///         if !ch.is_ascii_digit() {
    ///             println!("❌ Invalid PIN character: {}", ch);
    ///             return Ok(());
    ///         }
    ///     }
    ///     
    ///     // Send PIN digits
    ///     match client.send_dtmf(&call_id, pin).await {
    ///         Ok(()) => {
    ///             println!("✅ PIN entered successfully");
    ///             
    ///             // Send confirmation tone
    ///             if client.send_dtmf(&call_id, "#").await.is_ok() {
    ///                 println!("✅ PIN confirmed with #");
    ///             }
    ///         }
    ///         Err(e) => {
    ///             println!("❌ PIN entry failed: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Extended DTMF Usage
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn extended_dtmf() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5072".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Use extended DTMF characters (A-D)
    ///     let extended_sequence = "123A456B789C0*#D";
    ///     
    ///     match client.send_dtmf(&call_id, extended_sequence).await {
    ///         Ok(()) => {
    ///             println!("✅ Extended DTMF sequence sent");
    ///             println!("Sequence: {}", extended_sequence);
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Extended DTMF failed: {}", e);
    ///         }
    ///     }
    ///     
    ///     // Test individual extended characters
    ///     for digit in ['A', 'B', 'C', 'D'] {
    ///         let digit_str = digit.to_string();
    ///         if client.send_dtmf(&call_id, &digit_str).await.is_ok() {
    ///             println!("✅ Sent extended DTMF: {}", digit);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with DTMF history and timestamps
    /// - Triggers media events for DTMF transmission
    /// - Transmits actual audio tones to the remote party
    /// - Maintains a history of all DTMF transmissions for the call
    /// 
    /// # Implementation Notes
    /// 
    /// The method validates DTMF characters before transmission and maintains
    /// a history of all DTMF sequences sent during the call. Both uppercase
    /// and lowercase letters (A-D, a-d) are accepted and normalized.
    pub async fn send_dtmf(&self, call_id: &CallId, digits: &str) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to send DTMF
                }
                crate::call::CallState::Terminated | 
                crate::call::CallState::Failed | 
                crate::call::CallState::Cancelled => {
                    return Err(ClientError::InvalidCallState { 
                        call_id: *call_id, 
                        current_state: call_info.state.clone() 
                    });
                }
                _ => {
                    return Err(ClientError::InvalidCallStateGeneric { 
                        expected: "Connected".to_string(),
                        actual: format!("{:?}", call_info.state)
                    });
                }
            }
        }
        
        // Validate DTMF digits
        if digits.is_empty() {
            return Err(ClientError::InvalidConfiguration { 
                field: "dtmf_digits".to_string(),
                reason: "DTMF digits cannot be empty".to_string() 
            });
        }
        
        // Check for valid DTMF characters (0-9, A-D, *, #)
        for ch in digits.chars() {
            if !matches!(ch, '0'..='9' | 'A'..='D' | 'a'..='d' | '*' | '#') {
                return Err(ClientError::InvalidConfiguration { 
                    field: "dtmf_digits".to_string(),
                    reason: format!("Invalid DTMF character: {}", ch) 
                });
            }
        }
            
        // Use session-core DTMF functionality
        SessionControl::send_dtmf(&self.coordinator, &session_id, digits)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to send DTMF: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            let dtmf_history = call_info.metadata.entry("dtmf_history".to_string())
                .or_insert_with(String::new);
            if !dtmf_history.is_empty() {
                dtmf_history.push(',');
            }
            dtmf_history.push_str(&format!("{}@{}", digits, Utc::now().to_rfc3339()));
            
            call_info.metadata.insert("last_dtmf_sent".to_string(), digits.to_string());
            call_info.metadata.insert("last_dtmf_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent for DTMF
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::DtmfSent { digits: digits.to_string() },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Sent DTMF '{}' to call {}", digits, call_id);
        Ok(())
    }
    
    /// Transfer a call to another destination (blind transfer)
    /// 
    /// This method performs a blind transfer, which immediately transfers the call
    /// to the specified destination without consultation. The original caller is
    /// connected directly to the transfer target, and the transferring party is
    /// removed from the call.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to transfer
    /// * `target` - The SIP or TEL URI of the transfer destination
    /// 
    /// # Valid Target Formats
    /// 
    /// - **SIP URI**: `sip:user@domain.com` or `sip:user@192.168.1.100:5060`
    /// - **TEL URI**: `tel:+15551234567` (for PSTN numbers)
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the transfer was successfully initiated.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// * `ClientError::InvalidCallState` - If the call is not in a transferable state
    /// * `ClientError::InvalidConfiguration` - If the target URI is empty or invalid
    /// * `ClientError::CallSetupFailed` - If the transfer operation fails
    /// 
    /// # State Requirements
    /// 
    /// The call must be in the `Connected` state to be transferred. Calls in
    /// other states (ringing, terminated, etc.) cannot be transferred.
    /// 
    /// # Examples
    /// 
    /// ## Basic Blind Transfer
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn perform_blind_transfer() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5073".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Transfer to another SIP user
    ///     let transfer_target = "sip:support@example.com";
    ///     
    ///     match client.transfer_call(&call_id, transfer_target).await {
    ///         Ok(()) => {
    ///             println!("✅ Call {} transferred to {}", call_id, transfer_target);
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Transfer failed: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Transfer to PSTN Number
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn transfer_to_pstn() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5074".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Transfer to external phone number
    ///     let phone_number = "tel:+15551234567";
    ///     
    ///     if client.transfer_call(&call_id, phone_number).await.is_ok() {
    ///         println!("✅ Call transferred to phone: {}", phone_number);
    ///     } else {
    ///         println!("❌ PSTN transfer failed");
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Transfer with Validation
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn validated_transfer() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5075".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     let target = "sip:manager@company.com";
    ///     
    ///     // Check if call can be transferred before attempting
    ///     if let Ok(capabilities) = client.get_call_capabilities(&call_id).await {
    ///         if capabilities.can_transfer {
    ///             // Validate target format
    ///             if target.starts_with("sip:") || target.starts_with("tel:") {
    ///                 match client.transfer_call(&call_id, target).await {
    ///                     Ok(()) => {
    ///                         println!("✅ Transfer completed successfully");
    ///                     }
    ///                     Err(e) => {
    ///                         println!("❌ Transfer failed: {}", e);
    ///                     }
    ///                 }
    ///             } else {
    ///                 println!("❌ Invalid target URI format");
    ///             }
    ///         } else {
    ///             println!("❌ Call cannot be transferred in current state");
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Multiple Transfer Destinations
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn try_multiple_transfers() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5076".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Try multiple transfer destinations in order
    ///     let transfer_options = [
    ///         ("sip:primary@support.com", "Primary Support"),
    ///         ("sip:backup@support.com", "Backup Support"),
    ///         ("tel:+15551234567", "Emergency Line"),
    ///     ];
    ///     
    ///     for (target, description) in transfer_options {
    ///         match client.transfer_call(&call_id, target).await {
    ///             Ok(()) => {
    ///                 println!("✅ Successfully transferred to {} ({})", target, description);
    ///                 break; // Stop after first successful transfer
    ///             }
    ///             Err(e) => {
    ///                 println!("❌ Failed to transfer to {}: {}", description, e);
    ///                 // Continue to next option
    ///             }
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Transfer Types
    /// 
    /// This method performs a **blind transfer** (also called unattended transfer):
    /// - The call is immediately transferred without consultation
    /// - The transferring party does not speak to the transfer target first
    /// - The original caller is connected directly to the transfer destination
    /// - The transferring party is removed from the call immediately
    /// 
    /// For **attended transfers** (with consultation), use the `attended_transfer()` method.
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with transfer information and timestamp
    /// - Triggers media events for transfer initiation
    /// - The local party is immediately disconnected from the call
    /// - The remote party is connected to the transfer target
    /// 
    /// # SIP Protocol Notes
    /// 
    /// This method uses SIP REFER requests to perform the transfer, which is
    /// the standard mechanism defined in RFC 3515. The transfer target must
    /// be reachable and accept the incoming call for the transfer to succeed.
    pub async fn transfer_call(&self, call_id: &CallId, target: &str) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to transfer
                }
                crate::call::CallState::Terminated | 
                crate::call::CallState::Failed | 
                crate::call::CallState::Cancelled => {
                    return Err(ClientError::InvalidCallState { 
                        call_id: *call_id, 
                        current_state: call_info.state.clone() 
                    });
                }
                _ => {
                    return Err(ClientError::InvalidCallStateGeneric { 
                        expected: "Connected".to_string(),
                        actual: format!("{:?}", call_info.state)
                    });
                }
            }
        }
        
        // Validate target URI
        if target.is_empty() {
            return Err(ClientError::InvalidConfiguration { 
                field: "transfer_target".to_string(),
                reason: "Transfer target cannot be empty".to_string() 
            });
        }
        
        if !target.starts_with("sip:") && !target.starts_with("tel:") {
            return Err(ClientError::InvalidConfiguration { 
                field: "transfer_target".to_string(),
                reason: "Transfer target must be a valid SIP or TEL URI".to_string() 
            });
        }
            
        // Use session-core transfer functionality
        SessionControl::transfer_session(&self.coordinator, &session_id, target)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to transfer call: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("transfer_target".to_string(), target.to_string());
            call_info.metadata.insert("transfer_initiated_at".to_string(), Utc::now().to_rfc3339());
            call_info.metadata.insert("transfer_type".to_string(), "blind".to_string());
        }
        
        // Emit MediaEvent for transfer initiation
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::TransferInitiated { 
                    target: target.to_string(), 
                    transfer_type: "blind".to_string() 
                },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Initiated blind transfer of call {} to {}", call_id, target);
        Ok(())
    }
    
    /// Perform an attended transfer (consultative transfer)
    /// 
    /// This method performs an attended transfer, which connects two existing calls
    /// together. The typical scenario is having one call on hold while establishing
    /// a consultation call with the transfer target, then connecting the original
    /// caller directly to the transfer target.
    /// 
    /// # Arguments
    /// 
    /// * `call_id1` - The primary call to be transferred (usually the original call)
    /// * `call_id2` - The consultation call (the transfer target)
    /// 
    /// # Transfer Process
    /// 
    /// 1. **Hold**: The primary call (`call_id1`) is placed on hold
    /// 2. **Consultation**: The agent speaks with the transfer target (`call_id2`)
    /// 3. **Transfer**: The primary caller is connected to the transfer target
    /// 4. **Cleanup**: The consultation call is terminated
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the attended transfer was successfully completed.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If either call ID doesn't exist
    /// * `ClientError::InvalidCallState` - If either call is not in a transferable state
    /// * `ClientError::CallSetupFailed` - If any step of the transfer process fails
    /// 
    /// # State Requirements
    /// 
    /// Both calls must be in the `Connected` state to perform an attended transfer.
    /// 
    /// # Examples
    /// 
    /// ## Basic Attended Transfer
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn perform_attended_transfer() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5077".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Assume we have two active calls
    ///     let primary_call = CallId::new_v4();    // Original caller
    ///     let consultation_call = CallId::new_v4(); // Transfer target
    ///     
    ///     match client.attended_transfer(&primary_call, &consultation_call).await {
    ///         Ok(()) => {
    ///             println!("✅ Attended transfer completed successfully");
    ///             println!("Primary caller connected to transfer target");
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Attended transfer failed: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Step-by-Step Transfer Workflow
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// use tokio::time::{sleep, Duration};
    /// 
    /// async fn transfer_workflow() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5078".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let customer_call = CallId::new_v4();
    ///     let manager_call = CallId::new_v4();
    ///     
    ///     // Step 1: Answer customer call (would be done earlier)
    ///     println!("📞 Customer call in progress...");
    ///     
    ///     // Step 2: Put customer on hold to make consultation call
    ///     println!("⏸️  Putting customer on hold...");
    ///     if client.hold_call(&customer_call).await.is_ok() {
    ///         println!("✅ Customer on hold");
    ///     }
    ///     
    ///     // Step 3: Make consultation call to manager (would be done earlier)
    ///     println!("📞 Calling manager for consultation...");
    ///     // client.make_call("sip:manager@company.com").await?;
    ///     
    ///     // Step 4: Brief consultation (simulated)
    ///     sleep(Duration::from_millis(100)).await;
    ///     println!("💬 Consultation complete - transferring call");
    ///     
    ///     // Step 5: Perform the attended transfer
    ///     match client.attended_transfer(&customer_call, &manager_call).await {
    ///         Ok(()) => {
    ///             println!("✅ Transfer complete - customer now speaking with manager");
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Transfer failed: {}", e);
    ///             // Would typically resume customer call here
    ///             let _ = client.resume_call(&customer_call).await;
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Error Recovery Attended Transfer
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId, ClientError};
    /// 
    /// async fn robust_attended_transfer() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5079".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let caller_id = CallId::new_v4();
    ///     let target_id = CallId::new_v4();
    ///     
    ///     // Check capabilities before attempting transfer
    ///     let can_transfer_caller = client.get_call_capabilities(&caller_id).await
    ///         .map(|caps| caps.can_transfer)
    ///         .unwrap_or(false);
    ///     
    ///     let can_transfer_target = client.get_call_capabilities(&target_id).await
    ///         .map(|caps| caps.can_transfer)
    ///         .unwrap_or(false);
    ///     
    ///     if !can_transfer_caller || !can_transfer_target {
    ///         println!("❌ One or both calls cannot be transferred");
    ///         return Ok(());
    ///     }
    ///     
    ///     match client.attended_transfer(&caller_id, &target_id).await {
    ///         Ok(()) => {
    ///             println!("✅ Attended transfer successful");
    ///         }
    ///         Err(ClientError::CallNotFound { call_id }) => {
    ///             println!("❌ Call {} no longer exists", call_id);
    ///         }
    ///         Err(ClientError::InvalidCallState { call_id, current_state }) => {
    ///             println!("❌ Call {} in invalid state: {:?}", call_id, current_state);
    ///             
    ///             // Try to recover by resuming the original call
    ///             if let Err(e) = client.resume_call(&caller_id).await {
    ///                 println!("❌ Failed to resume original call: {}", e);
    ///             }
    ///         }
    ///         Err(e) => {
    ///             println!("❌ Transfer failed: {}", e);
    ///             
    ///             // General recovery - try to resume original call
    ///             let _ = client.resume_call(&caller_id).await;
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Comparison with Blind Transfer
    /// 
    /// | Feature | Blind Transfer | Attended Transfer |
    /// |---------|----------------|-------------------|
    /// | **Consultation** | No | Yes |
    /// | **Agent Control** | Immediate | Full control |
    /// | **Success Rate** | Lower | Higher |
    /// | **User Experience** | Basic | Professional |
    /// | **Call Setup** | Single call | Two calls |
    /// 
    /// # Side Effects
    /// 
    /// - The primary call is placed on hold during the process
    /// - Call metadata is updated with transfer type "attended"
    /// - Media events are triggered for transfer completion
    /// - The consultation call is automatically terminated
    /// - The transferring agent is removed from both calls
    /// 
    /// # Best Practices
    /// 
    /// 1. **Always verify both calls exist** before attempting transfer
    /// 2. **Check call capabilities** to ensure transfer is possible
    /// 3. **Implement error recovery** to handle failed transfers gracefully
    /// 4. **Inform the customer** when placing them on hold for consultation
    /// 5. **Have a fallback plan** if the transfer target is unavailable
    pub async fn attended_transfer(&self, call_id1: &CallId, call_id2: &CallId) -> ClientResult<()> {
        // Get session IDs for both calls (for validation, though not directly used below)
        let _session_id1 = self.session_mapping.get(call_id1)
            .ok_or(ClientError::CallNotFound { call_id: *call_id1 })?
            .clone();
        let _session_id2 = self.session_mapping.get(call_id2)
            .ok_or(ClientError::CallNotFound { call_id: *call_id2 })?
            .clone();
            
        // Validate both calls are in connected state
        for call_id in [call_id1, call_id2] {
            if let Some(call_info) = self.call_info.get(call_id) {
                match call_info.state {
                    crate::call::CallState::Connected => {
                        // OK to transfer
                    }
                    crate::call::CallState::Terminated | 
                    crate::call::CallState::Failed | 
                    crate::call::CallState::Cancelled => {
                        return Err(ClientError::InvalidCallState { 
                            call_id: *call_id, 
                            current_state: call_info.state.clone() 
                        });
                    }
                    _ => {
                        return Err(ClientError::InvalidCallStateGeneric { 
                            expected: "Connected".to_string(),
                            actual: format!("{:?}", call_info.state)
                        });
                    }
                }
            }
        }
        
        // For attended transfer, we typically would:
        // 1. Put the first call on hold
        // 2. Establish a consultation call with the transfer target
        // 3. Complete the transfer connecting the original caller to the transfer target
        // 
        // Since session-core doesn't have a specific attended transfer API,
        // we'll simulate it with available operations
        
        // Put first call on hold
        self.hold_call(call_id1).await?;
        
        // Get remote URI from second call to use as transfer target
        let target_uri = if let Some(call_info2) = self.call_info.get(call_id2) {
            call_info2.remote_uri.clone()
        } else {
            return Err(ClientError::CallNotFound { call_id: *call_id2 });
        };
        
        // Transfer the first call to the target of the second call
        self.transfer_call(call_id1, &target_uri).await?;
        
        // Hang up the consultation call since transfer is completing
        self.hangup_call(call_id2).await?;
        
        // Update metadata for attended transfer
        if let Some(mut call_info) = self.call_info.get_mut(call_id1) {
            call_info.metadata.insert("transfer_type".to_string(), "attended".to_string());
            call_info.metadata.insert("consultation_call_id".to_string(), call_id2.to_string());
            call_info.metadata.insert("attended_transfer_completed_at".to_string(), Utc::now().to_rfc3339());
        }
        
        tracing::info!("Completed attended transfer: call {} transferred to target of call {}", call_id1, call_id2);
        Ok(())
    }
    
    /// Get call control capabilities for a specific call
    /// 
    /// This method returns the available call control operations for a call based on
    /// its current state. Different call states support different operations, and this
    /// method helps applications determine what actions are available before attempting them.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to query
    /// 
    /// # Returns
    /// 
    /// Returns a `CallCapabilities` struct indicating which operations are available:
    /// 
    /// - `can_hold` - Whether the call can be placed on hold
    /// - `can_resume` - Whether the call can be resumed from hold
    /// - `can_transfer` - Whether the call can be transferred
    /// - `can_send_dtmf` - Whether DTMF digits can be sent
    /// - `can_mute` - Whether the call can be muted
    /// - `can_hangup` - Whether the call can be terminated
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// 
    /// # Capability Matrix by Call State
    /// 
    /// | State | Hold | Resume | Transfer | DTMF | Mute | Hangup |
    /// |-------|------|--------|----------|------|------|--------|
    /// | **Connected** | ✅ | ⚡ | ✅ | ✅ | ✅ | ✅ |
    /// | **Ringing** | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
    /// | **Initiating** | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
    /// | **Proceeding** | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
    /// | **Terminated** | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
    /// 
    /// ⚡ = Available only if call is currently on hold
    /// 
    /// # Examples
    /// 
    /// ## Basic Capability Check
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn check_capabilities() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5080".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     match client.get_call_capabilities(&call_id).await {
    ///         Ok(capabilities) => {
    ///             println!("Call capabilities:");
    ///             println!("  Hold: {}", capabilities.can_hold);
    ///             println!("  Resume: {}", capabilities.can_resume);
    ///             println!("  Transfer: {}", capabilities.can_transfer);
    ///             println!("  DTMF: {}", capabilities.can_send_dtmf);
    ///             println!("  Mute: {}", capabilities.can_mute);
    ///             println!("  Hangup: {}", capabilities.can_hangup);
    ///         }
    ///         Err(e) => {
    ///             println!("Failed to get capabilities: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Conditional Operation Execution
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn conditional_operations() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5081".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     if let Ok(caps) = client.get_call_capabilities(&call_id).await {
    ///         // Only attempt operations that are available
    ///         if caps.can_hold {
    ///             println!("✅ Hold operation available");
    ///             // client.hold_call(&call_id).await?;
    ///         } else {
    ///             println!("❌ Cannot hold call in current state");
    ///         }
    ///         
    ///         if caps.can_send_dtmf {
    ///             println!("✅ DTMF available - can send digits");
    ///             // client.send_dtmf(&call_id, "123").await?;
    ///         }
    ///         
    ///         if caps.can_transfer {
    ///             println!("✅ Transfer available");
    ///             // client.transfer_call(&call_id, "sip:target@example.com").await?;
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Dynamic UI Updates
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn update_ui_based_on_capabilities() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5082".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     if let Ok(capabilities) = client.get_call_capabilities(&call_id).await {
    ///         // Simulate UI button states
    ///         let buttons = vec![
    ///             ("Hold", capabilities.can_hold),
    ///             ("Resume", capabilities.can_resume),
    ///             ("Transfer", capabilities.can_transfer),
    ///             ("DTMF", capabilities.can_send_dtmf),
    ///             ("Mute", capabilities.can_mute),
    ///             ("Hangup", capabilities.can_hangup),
    ///         ];
    ///         
    ///         println!("UI Button States:");
    ///         for (button_name, enabled) in buttons {
    ///             let status = if enabled { "ENABLED" } else { "DISABLED" };
    ///             println!("  [{}] {}", status, button_name);
    ///         }
    ///         
    ///         // Special logic for hold/resume button
    ///         if capabilities.can_hold && !capabilities.can_resume {
    ///             println!("💡 Show 'Hold' button");
    ///         } else if !capabilities.can_hold && capabilities.can_resume {
    ///             println!("💡 Show 'Resume' button");
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// ## Capability Monitoring
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// use tokio::time::{sleep, Duration};
    /// 
    /// async fn monitor_capability_changes() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5083".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = CallId::new_v4();
    ///     
    ///     // Monitor capabilities over time (e.g., during state changes)
    ///     for i in 0..3 {
    ///         if let Ok(caps) = client.get_call_capabilities(&call_id).await {
    ///             println!("Check {}: Hold={}, Resume={}, Transfer={}", 
    ///                 i + 1, caps.can_hold, caps.can_resume, caps.can_transfer);
    ///         }
    ///         
    ///         sleep(Duration::from_millis(50)).await;
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Implementation Notes
    /// 
    /// The capabilities are determined based on the call's current state:
    /// 
    /// - **Connected calls** have the most capabilities available
    /// - **Ringing calls** can only be answered or rejected (hangup)
    /// - **Initiating calls** can only be cancelled (hangup)
    /// - **Terminated calls** have no available operations
    /// 
    /// The `can_resume` capability is dynamically determined by checking if
    /// the call is currently on hold using the call's metadata.
    /// 
    /// # Best Practices
    /// 
    /// 1. **Always check capabilities** before attempting operations
    /// 2. **Update UI dynamically** based on capability changes
    /// 3. **Handle capability changes** during call state transitions
    /// 4. **Provide user feedback** when operations are not available
    /// 5. **Cache capabilities briefly** to avoid excessive queries
    pub async fn get_call_capabilities(&self, call_id: &CallId) -> ClientResult<CallCapabilities> {
        let call_info = self.get_call(call_id).await?;
        
        let capabilities = match call_info.state {
            crate::call::CallState::Connected => CallCapabilities {
                can_hold: true,
                can_resume: self.is_call_on_hold(call_id).await.unwrap_or(false),
                can_transfer: true,
                can_send_dtmf: true,
                can_mute: true,
                can_hangup: true,
            },
            crate::call::CallState::Ringing | crate::call::CallState::IncomingPending => CallCapabilities {
                can_hold: false,
                can_resume: false,
                can_transfer: false,
                can_send_dtmf: false,
                can_mute: false,
                can_hangup: true, // Can reject
            },
            crate::call::CallState::Initiating | crate::call::CallState::Proceeding => CallCapabilities {
                can_hold: false,
                can_resume: false,
                can_transfer: false,
                can_send_dtmf: false,
                can_mute: false,
                can_hangup: true, // Can cancel
            },
            _ => CallCapabilities::default(), // Terminated states have no capabilities
        };
        
        Ok(capabilities)
    }
} 