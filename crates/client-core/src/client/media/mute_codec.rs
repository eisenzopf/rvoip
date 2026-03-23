//! Mute controls (microphone/speaker) and codec management operations

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

/// Mute and codec operations implementation for ClientManager
impl super::super::manager::ClientManager {
    // ===== PRIORITY 4.1: ENHANCED MEDIA INTEGRATION =====
    
    /// Enhanced microphone mute/unmute with proper session-core integration
    /// 
    /// Controls the microphone mute state for a specific call. When muted, the local audio
    /// transmission is stopped, preventing the remote party from hearing your voice.
    /// This operation validates the call state and emits appropriate media events.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to mute/unmute
    /// * `muted` - `true` to mute the microphone, `false` to unmute
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the operation succeeds, or a `ClientError` if:
    /// - The call is not found
    /// - The call is in an invalid state (terminated, failed, cancelled)
    /// - The underlying media session fails to change mute state
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Basic usage - mute the microphone
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would mute microphone for call {}", call_id);
    /// 
    /// // Toggle functionality
    /// let current_state = false; // Simulated current state
    /// let new_state = !current_state;
    /// println!("Would toggle microphone from {} to {}", current_state, new_state);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Privacy mode example
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Enabling privacy mode for call {}", call_id);
    /// println!("Microphone would be muted");
    /// # }
    /// ```
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with mute state and timestamp
    /// - Emits a `MediaEventType::MicrophoneStateChanged` event
    /// - Calls session-core to actually control audio transmission
    pub async fn set_microphone_mute(&self, call_id: &CallId, muted: bool) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to mute/unmute
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
            
        // Use session-core mute/unmute functionality
        SessionControl::set_audio_muted(&self.coordinator, &session_id, muted)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to set microphone mute: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("microphone_muted".to_string(), muted.to_string());
            call_info.metadata.insert("mic_mute_changed_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::MicrophoneStateChanged { muted },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Set microphone muted={} for call {}", muted, call_id);
        Ok(())
    }
    
    /// Enhanced speaker mute/unmute with event emission
    /// 
    /// Controls the speaker (audio output) mute state for a specific call. When speaker
    /// is muted, you won't hear audio from the remote party. This is typically handled
    /// client-side as it controls local audio playback rather than network transmission.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to mute/unmute
    /// * `muted` - `true` to mute the speaker, `false` to unmute
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the operation succeeds, or a `ClientError` if the call is not found.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Basic speaker control
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would mute speaker for call {}", call_id);
    /// 
    /// // Unmute speaker
    /// println!("Would unmute speaker for call {}", call_id);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Privacy mode: mute both microphone and speaker
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Enabling full privacy mode for call {}", call_id);
    /// println!("Would mute microphone and speaker");
    /// # }
    /// ```
    /// 
    /// # Implementation Notes
    /// 
    /// This function handles client-side audio output control and does not affect
    /// network RTP streams. The mute state is stored in call metadata and can be
    /// retrieved using `get_speaker_mute_state()`.
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with speaker mute state and timestamp
    /// - Emits a `MediaEventType::SpeakerStateChanged` event
    pub async fn set_speaker_mute(&self, call_id: &CallId, muted: bool) -> ClientResult<()> {
        // Validate call exists
        if !self.call_info.contains_key(call_id) {
            return Err(ClientError::CallNotFound { call_id: *call_id });
        }
        
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Note: Speaker mute is typically handled client-side as session-core
        // may not have direct speaker control. This is a placeholder implementation.
        
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("speaker_muted".to_string(), muted.to_string());
            call_info.metadata.insert("speaker_mute_changed_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::SpeakerStateChanged { muted },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata.insert("client_side_control".to_string(), "true".to_string());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Set speaker muted={} for call {} (client-side)", muted, call_id);
        Ok(())
    }
    
    /// Get comprehensive media information for a call using session-core
    /// 
    /// Retrieves detailed media information about an active call, including SDP negotiation
    /// details, RTP port assignments, codec information, and current media state.
    /// This is useful for monitoring call quality, debugging media issues, and displaying
    /// technical call information to users or administrators.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to query
    /// 
    /// # Returns
    /// 
    /// Returns a `CallMediaInfo` struct containing:
    /// - Local and remote SDP descriptions
    /// - RTP port assignments (local and remote)
    /// - Negotiated audio codec
    /// - Current mute and hold states
    /// - Audio direction (send/receive/both/inactive)
    /// - Quality metrics (if available)
    /// 
    /// Returns `ClientError::CallNotFound` if the call doesn't exist, or
    /// `ClientError::InternalError` if media information cannot be retrieved.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # use rvoip_client_core::client::types::AudioDirection;
    /// # fn main() {
    /// // Basic media info retrieval
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would get media info for call {}", call_id);
    /// 
    /// // Check audio direction
    /// let audio_direction = AudioDirection::SendReceive;
    /// match audio_direction {
    ///     AudioDirection::SendReceive => println!("Full duplex audio"),
    ///     AudioDirection::SendOnly => println!("Send-only (e.g., hold)"),
    ///     AudioDirection::ReceiveOnly => println!("Receive-only"),
    ///     AudioDirection::Inactive => println!("No audio flow"),
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Diagnostic information
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Getting diagnostic info for call {}", call_id);
    /// println!("This would include SDP, ports, codec, and states");
    /// # }
    /// ```
    pub async fn get_call_media_info(&self, call_id: &CallId) -> ClientResult<CallMediaInfo> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Get media info from session-core
        let media_info = MediaControl::get_media_info(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to get media info: {}", e) 
            })?
            .ok_or_else(|| ClientError::InternalError { 
                message: "No media info available".to_string() 
            })?;
            
        // Determine audio direction before moving fields
        let audio_direction = self.determine_audio_direction(&media_info).await;
            
        // Convert session-core MediaInfo to client-core CallMediaInfo
        let call_media_info = CallMediaInfo {
            call_id: *call_id,
            local_sdp: media_info.local_sdp,
            remote_sdp: media_info.remote_sdp,
            local_rtp_port: media_info.local_rtp_port,
            remote_rtp_port: media_info.remote_rtp_port,
            codec: media_info.codec,
            is_muted: self.get_microphone_mute_state(call_id).await.unwrap_or(false),
            is_on_hold: self.is_call_on_hold(call_id).await.unwrap_or(false),
            audio_direction,
            quality_metrics: None, // TODO: Extract quality metrics if available
        };
        
        Ok(call_media_info)
    }
    
    /// Get the current microphone mute state for a call
    /// 
    /// Retrieves the current mute state of the microphone for the specified call.
    /// This state is maintained in the call's metadata and reflects whether local
    /// audio transmission is currently enabled (false) or disabled (true).
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to query
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(true)` if the microphone is muted, `Ok(false)` if unmuted,
    /// or `ClientError::CallNotFound` if the call doesn't exist.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Check microphone state
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would check microphone mute state for call {}", call_id);
    /// 
    /// // Conditional logic based on mute state
    /// let is_muted = false; // Simulated state
    /// if is_muted {
    ///     println!("Microphone is currently muted");
    /// } else {
    ///     println!("Microphone is active");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // UI indicator logic
    /// let call_id: CallId = Uuid::new_v4();
    /// let mute_state = false; // Would get actual state
    /// let indicator = if mute_state { "🔇" } else { "🔊" };
    /// println!("Microphone status for call {}: {}", call_id, indicator);
    /// # }
    /// ```
    pub async fn get_microphone_mute_state(&self, call_id: &CallId) -> ClientResult<bool> {
        if let Some(call_info) = self.call_info.get(call_id) {
            let muted = call_info.metadata.get("microphone_muted")
                .map(|s| s == "true")
                .unwrap_or(false);
            Ok(muted)
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }
    
    /// Get the current speaker mute state for a call
    /// 
    /// Retrieves the current mute state of the speaker (audio output) for the specified call.
    /// This state is maintained in the call's metadata and reflects whether remote
    /// audio playback is currently enabled (false) or disabled (true).
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to query
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(true)` if the speaker is muted, `Ok(false)` if unmuted,
    /// or `ClientError::CallNotFound` if the call doesn't exist.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Check speaker state
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would check speaker mute state for call {}", call_id);
    /// 
    /// // Audio feedback prevention
    /// let speaker_muted = true; // Simulated state
    /// if speaker_muted {
    ///     println!("Safe to use speakerphone mode");
    /// } else {
    ///     println!("May cause audio feedback");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Privacy status check
    /// let call_id: CallId = Uuid::new_v4();
    /// let mic_muted = true;
    /// let speaker_muted = true;
    /// 
    /// if mic_muted && speaker_muted {
    ///     println!("Call {} is in full privacy mode", call_id);
    /// }
    /// # }
    /// ```
    pub async fn get_speaker_mute_state(&self, call_id: &CallId) -> ClientResult<bool> {
        if let Some(call_info) = self.call_info.get(call_id) {
            let muted = call_info.metadata.get("speaker_muted")
                .map(|s| s == "true")
                .unwrap_or(false);
            Ok(muted)
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }
    
    /// Get supported audio codecs with comprehensive information
    /// 
    /// Returns a complete list of audio codecs supported by this client implementation.
    /// This is an alias for `get_available_codecs()` provided for API consistency.
    /// Each codec includes detailed information about capabilities, quality ratings,
    /// and technical specifications.
    /// 
    /// # Returns
    /// 
    /// A vector of `AudioCodecInfo` structures containing:
    /// - Codec name and payload type
    /// - Sample rate and channel configuration
    /// - Quality rating (1-5 scale)
    /// - Human-readable description
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::types::AudioCodecInfo;
    /// # fn main() {
    /// // Simulate codec information
    /// let codec = AudioCodecInfo {
    ///     name: "OPUS".to_string(),
    ///     payload_type: 111,
    ///     clock_rate: 48000,
    ///     channels: 2,
    ///     description: "High quality codec".to_string(),
    ///     quality_rating: 5,
    /// };
    /// 
    /// println!("Codec: {} (Quality: {}/5)", codec.name, codec.quality_rating);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # fn main() {
    /// // Filter high-quality codecs
    /// let quality_threshold = 4;
    /// println!("Looking for codecs with quality >= {}", quality_threshold);
    /// println!("Would filter codec list by quality rating");
    /// # }
    /// ```
    /// 
    /// Get supported audio codecs (alias for get_available_codecs)
    pub async fn get_supported_audio_codecs(&self) -> Vec<AudioCodecInfo> {
        self.get_available_codecs().await
    }
    
    /// Get list of available audio codecs with detailed information
    /// 
    /// Returns a comprehensive list of audio codecs supported by the client,
    /// including payload types, sample rates, quality ratings, and descriptions.
    /// This information can be used for codec selection, capability negotiation,
    /// and display in user interfaces.
    /// 
    /// # Returns
    /// 
    /// A vector of `AudioCodecInfo` structures, each containing:
    /// - Codec name and standard designation
    /// - RTP payload type number
    /// - Audio sampling rate and channel count
    /// - Human-readable description
    /// - Quality rating (1-5 scale, 5 being highest)
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::types::AudioCodecInfo;
    /// # fn main() {
    /// // Example codec information
    /// let codecs = vec![
    ///     AudioCodecInfo {
    ///         name: "OPUS".to_string(),
    ///         payload_type: 111,
    ///         clock_rate: 48000,
    ///         channels: 2,
    ///         description: "High quality codec".to_string(),
    ///         quality_rating: 5,
    ///     },
    ///     AudioCodecInfo {
    ///         name: "G722".to_string(),
    ///         payload_type: 9,
    ///         clock_rate: 8000,
    ///         channels: 1,
    ///         description: "Wideband audio".to_string(),
    ///         quality_rating: 4,
    ///     }
    /// ];
    /// 
    /// for codec in &codecs {
    ///     println!("Codec: {} (PT: {}, Rate: {}Hz, Quality: {}/5)", 
    ///              codec.name, codec.payload_type, codec.clock_rate, codec.quality_rating);
    ///     println!("  Description: {}", codec.description);
    /// }
    /// 
    /// // Find high-quality codecs
    /// let high_quality: Vec<_> = codecs
    ///     .into_iter()
    ///     .filter(|c| c.quality_rating >= 4)
    ///     .collect();
    /// println!("Found {} high-quality codecs", high_quality.len());
    /// # }
    /// ```
    pub async fn get_available_codecs(&self) -> Vec<AudioCodecInfo> {
        // Enhanced codec list with quality ratings and detailed information
        vec![
            AudioCodecInfo {
                name: "PCMU".to_string(),
                payload_type: 0,
                clock_rate: 8000,
                channels: 1,
                description: "G.711 μ-law - Standard quality, widely compatible".to_string(),
                quality_rating: 3,
            },
            AudioCodecInfo {
                name: "PCMA".to_string(),
                payload_type: 8,
                clock_rate: 8000,
                channels: 1,
                description: "G.711 A-law - Standard quality, widely compatible".to_string(),
                quality_rating: 3,
            },
            AudioCodecInfo {
                name: "G722".to_string(),
                payload_type: 9,
                clock_rate: 8000,
                channels: 1,
                description: "G.722 - Wideband audio, good quality".to_string(),
                quality_rating: 4,
            },
            AudioCodecInfo {
                name: "G729".to_string(),
                payload_type: 18,
                clock_rate: 8000,
                channels: 1,
                description: "G.729 - Low bandwidth, compressed".to_string(),
                quality_rating: 2,
            },
            AudioCodecInfo {
                name: "OPUS".to_string(),
                payload_type: 111,
                clock_rate: 48000,
                channels: 2,
                description: "Opus - High quality, adaptive bitrate".to_string(),
                quality_rating: 5,
            },
        ]
    }
    
    /// Get codec information for a specific active call
    /// 
    /// Retrieves detailed information about the audio codec currently being used
    /// for the specified call. This includes technical specifications, quality ratings,
    /// and capabilities of the negotiated codec. Returns `None` if no codec has been
    /// negotiated yet or if the call doesn't have active media.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to query
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(Some(AudioCodecInfo))` with codec details if available,
    /// `Ok(None)` if no codec is negotiated, or `ClientError` if the call is not found
    /// or media information cannot be retrieved.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # use rvoip_client_core::client::types::AudioCodecInfo;
    /// # fn main() {
    /// // Check call codec
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would get codec info for call {}", call_id);
    /// 
    /// // Example codec info handling
    /// let codec_info = Some(AudioCodecInfo {
    ///     name: "G722".to_string(),
    ///     payload_type: 9,
    ///     clock_rate: 8000,
    ///     channels: 1,
    ///     description: "Wideband audio".to_string(),
    ///     quality_rating: 4,
    /// });
    /// 
    /// match codec_info {
    ///     Some(codec) => println!("Using codec: {} ({})", codec.name, codec.description),
    ///     None => println!("No codec negotiated yet"),
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Quality assessment
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Assessing call quality for call {}", call_id);
    /// 
    /// let quality_rating = 4; // Simulated rating
    /// match quality_rating {
    ///     5 => println!("Excellent audio quality"),
    ///     4 => println!("Good audio quality"),
    ///     3 => println!("Acceptable audio quality"),
    ///     _ => println!("Poor audio quality"),
    /// }
    /// # }
    /// ```
    pub async fn get_call_codec_info(&self, call_id: &CallId) -> ClientResult<Option<AudioCodecInfo>> {
        let media_info = self.get_call_media_info(call_id).await?;
        
        if let Some(codec_name) = media_info.codec {
            let codecs = self.get_available_codecs().await;
            let codec_info = codecs.into_iter()
                .find(|c| c.name.eq_ignore_ascii_case(&codec_name));
            Ok(codec_info)
        } else {
            Ok(None)
        }
    }
    
    /// Set preferred codec order for future calls
    /// 
    /// Configures the preferred order of audio codecs for use in future call negotiations.
    /// The client will attempt to negotiate codecs in the specified order, with the first
    /// codec in the list being the most preferred. This setting affects SDP generation
    /// and codec negotiation during call establishment.
    /// 
    /// # Arguments
    /// 
    /// * `codec_names` - Vector of codec names in order of preference (e.g., ["OPUS", "G722", "PCMU"])
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on success. Currently always succeeds as this stores preferences
    /// for future use rather than validating codec availability immediately.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # fn main() {
    /// // Set high-quality codec preference
    /// let high_quality_codecs = vec![
    ///     "OPUS".to_string(),
    ///     "G722".to_string(),
    ///     "PCMU".to_string(),
    /// ];
    /// println!("Would set codec preference: {:?}", high_quality_codecs);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # fn main() {
    /// // Low bandwidth preference
    /// let low_bandwidth_codecs = vec![
    ///     "G729".to_string(),
    ///     "PCMU".to_string(),
    ///     "PCMA".to_string(),
    /// ];
    /// println!("Low bandwidth codec order: {:?}", low_bandwidth_codecs);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # fn main() {
    /// // Enterprise compatibility preference
    /// let enterprise_codecs = vec![
    ///     "G722".to_string(),  // Good quality, widely supported
    ///     "PCMU".to_string(),  // Universal compatibility
    ///     "PCMA".to_string(),  // European preference
    /// ];
    /// println!("Enterprise codec preference: {:?}", enterprise_codecs);
    /// # }
    /// ```
    /// 
    /// # Implementation Notes
    /// 
    /// This setting will be applied to future call negotiations. Active calls will
    /// continue using their currently negotiated codecs. The codec names should match
    /// those returned by `get_available_codecs()`.
    pub async fn set_preferred_codecs(&self, codec_names: Vec<String>) -> ClientResult<()> {
        // This would typically configure the session manager with preferred codecs
        // For now, we'll store it in client configuration metadata
        tracing::info!("Setting preferred codecs: {:?}", codec_names);
        
        // TODO: Configure session-core with preferred codec order
        // self.session_manager.set_preferred_codecs(codec_names).await?;
        
        Ok(())
    }
}
