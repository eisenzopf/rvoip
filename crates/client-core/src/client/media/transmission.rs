//! Audio transmission start/stop, custom audio, and tone generation operations

use std::collections::HashMap;
use chrono::{DateTime, Utc};

// Import session-core APIs
use rvoip_session_core::api::{
    SessionControl,
    MediaControl,
    SessionCoordinator,
    SessionId,
    MediaInfo as SessionMediaInfo,
    CallStatistics,
    MediaSessionStats,
    RtpSessionStats,
    QualityMetrics,
    QualityThresholds,
};

// Import client-core types
use crate::{
    ClientResult, ClientError,
    call::CallId,
    events::MediaEventInfo,
};

use super::super::types::*;

/// Audio transmission operations implementation for ClientManager
impl super::super::manager::ClientManager {
    /// Start audio transmission for a call in pass-through mode (default)
    /// 
    /// Starts audio transmission for the specified call using the default pass-through mode,
    /// which allows RTP audio packets to flow between endpoints without automatic audio
    /// generation. This is the recommended mode for most production use cases.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to start audio transmission for
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success, or a `ClientError` if:
    /// - The call is not found
    /// - The call is not in the Connected state
    /// - The underlying media session fails to start transmission
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Start audio transmission
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would start audio transmission for call {}", call_id);
    /// println!("RTP audio packets would begin flowing");
    /// # }
    /// ```
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with transmission status and timestamp
    /// - Emits a `MediaEventType::AudioStarted` event
    /// - Begins RTP packet transmission through session-core
    /// 
    /// # State Requirements
    /// 
    /// The call must be in `Connected` state. Calls that are terminated, failed,
    /// or cancelled cannot have audio transmission started.
    pub async fn start_audio_transmission(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to start transmission
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
            
        // Use session-core to start audio transmission in pass-through mode
        MediaControl::start_audio_transmission(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to start audio transmission: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("audio_transmission_active".to_string(), "true".to_string());
            call_info.metadata.insert("audio_transmission_mode".to_string(), "pass_through".to_string());
            call_info.metadata.insert("transmission_started_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::AudioStarted,
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata.insert("mode".to_string(), "pass_through".to_string());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Started audio transmission (pass-through mode) for call {}", call_id);
        Ok(())
    }
    
    /// Start audio transmission for a call with tone generation
    /// 
    /// Starts audio transmission for the specified call using tone generation mode,
    /// which generates a 440Hz sine wave for testing purposes. This is useful for
    /// testing audio connectivity without requiring external audio sources.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to start audio transmission for
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success, or a `ClientError` if:
    /// - The call is not found
    /// - The call is not in the Connected state
    /// - The underlying media session fails to start transmission
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Start audio transmission with test tone
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would start 440Hz test tone for call {}", call_id);
    /// # }
    /// ```
    pub async fn start_audio_transmission_with_tone(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        self.validate_call_state_for_audio(call_id)?;
            
        // Use session-core to start audio transmission with tone
        MediaControl::start_audio_transmission_with_tone(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to start audio transmission with tone: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("audio_transmission_active".to_string(), "true".to_string());
            call_info.metadata.insert("audio_transmission_mode".to_string(), "tone_generation".to_string());
            call_info.metadata.insert("transmission_started_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::AudioStarted,
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata.insert("mode".to_string(), "tone_generation".to_string());
                    metadata.insert("frequency".to_string(), "440".to_string());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Started audio transmission with tone generation for call {}", call_id);
        Ok(())
    }
    
    /// Start audio transmission for a call with custom audio samples
    /// 
    /// Starts audio transmission for the specified call using custom audio samples.
    /// The samples must be in G.711 μ-law format (8-bit samples at 8kHz).
    /// This allows playing back custom audio files or any audio data during the call.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to start audio transmission for
    /// * `samples` - The audio samples in G.711 μ-law format
    /// * `repeat` - Whether to repeat the audio samples when they finish
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success, or a `ClientError` if:
    /// - The call is not found
    /// - The call is not in the Connected state
    /// - The samples vector is empty
    /// - The underlying media session fails to start transmission
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Start audio transmission with custom audio
    /// let call_id: CallId = Uuid::new_v4();
    /// let audio_samples = vec![0x7F, 0x80, 0x7F, 0x80]; // Example μ-law samples
    /// println!("Would start custom audio transmission for call {} ({} samples)", 
    ///          call_id, audio_samples.len());
    /// # }
    /// ```
    pub async fn start_audio_transmission_with_custom_audio(&self, call_id: &CallId, samples: Vec<u8>, repeat: bool) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        self.validate_call_state_for_audio(call_id)?;
        
        // Validate samples
        if samples.is_empty() {
            return Err(ClientError::InvalidConfiguration { 
                field: "samples".to_string(),
                reason: "Audio samples cannot be empty".to_string() 
            });
        }
            
        // Use session-core to start audio transmission with custom audio
        MediaControl::start_audio_transmission_with_custom_audio(&self.coordinator, &session_id, samples.clone(), repeat)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to start audio transmission with custom audio: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("audio_transmission_active".to_string(), "true".to_string());
            call_info.metadata.insert("audio_transmission_mode".to_string(), "custom_audio".to_string());
            call_info.metadata.insert("custom_audio_samples".to_string(), samples.len().to_string());
            call_info.metadata.insert("custom_audio_repeat".to_string(), repeat.to_string());
            call_info.metadata.insert("transmission_started_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::AudioStarted,
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata.insert("mode".to_string(), "custom_audio".to_string());
                    metadata.insert("samples_count".to_string(), samples.len().to_string());
                    metadata.insert("repeat".to_string(), repeat.to_string());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Started audio transmission with custom audio for call {} ({} samples, repeat: {})", 
                      call_id, samples.len(), repeat);
        Ok(())
    }
    
    /// Set custom audio samples for an active transmission session
    /// 
    /// Updates the audio samples for an already active transmission session.
    /// This allows changing the audio content during an ongoing call without 
    /// stopping and restarting the transmission.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call
    /// * `samples` - The new audio samples in G.711 μ-law format
    /// * `repeat` - Whether to repeat the audio samples when they finish
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success, or a `ClientError` if:
    /// - The call is not found
    /// - Audio transmission is not active for this call
    /// - The samples vector is empty
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Switch to different audio during call
    /// let call_id: CallId = Uuid::new_v4();
    /// let new_samples = vec![0x7F, 0x80, 0x7F, 0x80]; // New audio content
    /// println!("Would update audio for call {} with {} new samples", 
    ///          call_id, new_samples.len());
    /// # }
    /// ```
    pub async fn set_custom_audio(&self, call_id: &CallId, samples: Vec<u8>, repeat: bool) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate samples
        if samples.is_empty() {
            return Err(ClientError::InvalidConfiguration { 
                field: "samples".to_string(),
                reason: "Audio samples cannot be empty".to_string() 
            });
        }
        
        // Check if audio transmission is active
        if !self.is_audio_transmission_active(call_id).await? {
            return Err(ClientError::InvalidCallStateGeneric { 
                expected: "Active audio transmission".to_string(),
                actual: "Audio transmission not active".to_string()
            });
        }
            
        // Use session-core to set custom audio
        MediaControl::set_custom_audio(&self.coordinator, &session_id, samples.clone(), repeat)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to set custom audio: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("audio_transmission_mode".to_string(), "custom_audio".to_string());
            call_info.metadata.insert("custom_audio_samples".to_string(), samples.len().to_string());
            call_info.metadata.insert("custom_audio_repeat".to_string(), repeat.to_string());
            call_info.metadata.insert("audio_updated_at".to_string(), Utc::now().to_rfc3339());
        }
        
        tracing::info!("Set custom audio for call {} ({} samples, repeat: {})", 
                      call_id, samples.len(), repeat);
        Ok(())
    }
    
    /// Set tone generation parameters for an active transmission session
    /// 
    /// Updates the tone generation parameters for an already active transmission session.
    /// This allows changing from custom audio or pass-through mode to tone generation
    /// during an ongoing call.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call
    /// * `frequency` - The tone frequency in Hz (e.g., 440.0 for A4)
    /// * `amplitude` - The tone amplitude (0.0 to 1.0)
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success, or a `ClientError` if:
    /// - The call is not found
    /// - Audio transmission is not active for this call
    /// - Invalid frequency or amplitude values
    pub async fn set_tone_generation(&self, call_id: &CallId, frequency: f64, amplitude: f64) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate parameters
        if frequency <= 0.0 || frequency > 20000.0 {
            return Err(ClientError::InvalidConfiguration { 
                field: "frequency".to_string(),
                reason: "Frequency must be between 0 and 20000 Hz".to_string() 
            });
        }
        
        if amplitude < 0.0 || amplitude > 1.0 {
            return Err(ClientError::InvalidConfiguration { 
                field: "amplitude".to_string(),
                reason: "Amplitude must be between 0.0 and 1.0".to_string() 
            });
        }
        
        // Check if audio transmission is active
        if !self.is_audio_transmission_active(call_id).await? {
            return Err(ClientError::InvalidCallStateGeneric { 
                expected: "Active audio transmission".to_string(),
                actual: "Audio transmission not active".to_string()
            });
        }
            
        // Use session-core to set tone generation
        MediaControl::set_tone_generation(&self.coordinator, &session_id, frequency, amplitude)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to set tone generation: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("audio_transmission_mode".to_string(), "tone_generation".to_string());
            call_info.metadata.insert("tone_frequency".to_string(), frequency.to_string());
            call_info.metadata.insert("tone_amplitude".to_string(), amplitude.to_string());
            call_info.metadata.insert("audio_updated_at".to_string(), Utc::now().to_rfc3339());
        }
        
        tracing::info!("Set tone generation for call {} ({}Hz, amplitude: {})", 
                      call_id, frequency, amplitude);
        Ok(())
    }
    
    /// Enable pass-through mode for an active transmission session
    /// 
    /// Switches an active transmission session to pass-through mode, which stops
    /// any audio generation (tones or custom audio) and allows normal RTP audio
    /// flow between endpoints.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success, or a `ClientError` if:
    /// - The call is not found
    /// - Audio transmission is not active for this call
    pub async fn set_pass_through_mode(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Check if audio transmission is active
        if !self.is_audio_transmission_active(call_id).await? {
            return Err(ClientError::InvalidCallStateGeneric { 
                expected: "Active audio transmission".to_string(),
                actual: "Audio transmission not active".to_string()
            });
        }
            
        // Use session-core to set pass-through mode
        MediaControl::set_pass_through_mode(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to set pass-through mode: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("audio_transmission_mode".to_string(), "pass_through".to_string());
            call_info.metadata.insert("audio_updated_at".to_string(), Utc::now().to_rfc3339());
        }
        
        tracing::info!("Set pass-through mode for call {}", call_id);
        Ok(())
    }
    
    /// Helper method to validate call state for audio operations
    fn validate_call_state_for_audio(&self, call_id: &CallId) -> ClientResult<()> {
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => Ok(()),
                crate::call::CallState::Terminated | 
                crate::call::CallState::Failed | 
                crate::call::CallState::Cancelled => {
                    Err(ClientError::InvalidCallState { 
                        call_id: *call_id, 
                        current_state: call_info.state.clone() 
                    })
                }
                _ => {
                    Err(ClientError::InvalidCallStateGeneric { 
                        expected: "Connected".to_string(),
                        actual: format!("{:?}", call_info.state)
                    })
                }
            }
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }
    
    /// Stop audio transmission for a call
    /// 
    /// Stops audio transmission for the specified call, halting the flow of
    /// RTP audio packets between the local client and the remote endpoint. This
    /// is typically used when putting a call on hold or during call termination.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to stop audio transmission for
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success, or a `ClientError` if:
    /// - The call is not found
    /// - The underlying media session fails to stop transmission
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Stop audio transmission
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would stop audio transmission for call {}", call_id);
    /// println!("RTP audio packets would stop flowing");
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Put call on hold
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Putting call {} on hold", call_id);
    /// println!("Audio transmission would be stopped");
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Emergency stop
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Emergency stop of audio for call {}", call_id);
    /// println!("Immediate halt of RTP transmission");
    /// # }
    /// ```
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with transmission status and timestamp
    /// - Emits a `MediaEventType::AudioStopped` event
    /// - Stops RTP packet transmission through session-core
    /// 
    /// # Use Cases
    /// 
    /// - Putting calls on hold
    /// - Call termination procedures
    /// - Emergency audio cutoff
    /// - Bandwidth conservation
    pub async fn stop_audio_transmission(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to stop audio transmission
        MediaControl::stop_audio_transmission(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to stop audio transmission: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("audio_transmission_active".to_string(), "false".to_string());
            call_info.metadata.insert("transmission_stopped_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::AudioStopped,
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Stopped audio transmission for call {}", call_id);
        Ok(())
    }
    
    /// Check if audio transmission is active for a call
    /// 
    /// Determines whether audio transmission is currently active for the specified call.
    /// This status is tracked in the call's metadata and reflects the current state
    /// of RTP audio packet transmission.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to check
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(true)` if audio transmission is active, `Ok(false)` if inactive,
    /// or `ClientError::CallNotFound` if the call doesn't exist.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Check transmission status
    /// let call_id: CallId = Uuid::new_v4();
    /// let is_active = true; // Simulated state
    /// 
    /// if is_active {
    ///     println!("Call {} has active audio transmission", call_id);
    /// } else {
    ///     println!("Call {} audio transmission is stopped", call_id);
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Conditional UI display
    /// let call_id: CallId = Uuid::new_v4();
    /// let transmission_active = false; // Simulated
    /// 
    /// let status_icon = if transmission_active { "🔊" } else { "⏸️" };
    /// println!("Audio status for call {}: {}", call_id, status_icon);
    /// # }
    /// ```
    pub async fn is_audio_transmission_active(&self, call_id: &CallId) -> ClientResult<bool> {
        if let Some(call_info) = self.call_info.get(call_id) {
            let active = call_info.metadata.get("audio_transmission_active")
                .map(|s| s == "true")
                .unwrap_or(false);
            Ok(active)
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }
}
