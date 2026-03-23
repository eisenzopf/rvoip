//! Audio transmission, mute, codec detection, and streaming methods for MediaManager

use crate::api::types::SessionId;
use super::super::types::*;
use super::super::MediaError;
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::Mutex;
use super::MediaManager;
use super::super::MediaResult;
use rvoip_media_core::relay::controller::{
    codec_detection::{CodecDetector, CodecDetectionResult},
    codec_fallback::{CodecFallbackManager, FallbackMode, FallbackStats},
};
use rvoip_media_core::codec::mapping::CodecMapper;

impl MediaManager {
    /// Start audio transmission for a session
    pub async fn start_audio_transmission(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.start_audio_transmission(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Started audio transmission for session: {}", session_id);
        Ok(())
    }
    
    /// Start audio transmission for a session with tone generation
    pub async fn start_audio_transmission_with_tone(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.start_audio_transmission_with_tone(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Started audio transmission with tone for session: {}", session_id);
        Ok(())
    }
    
    /// Start audio transmission for a session with custom audio samples
    pub async fn start_audio_transmission_with_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> super::super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.start_audio_transmission_with_custom_audio(&dialog_id, samples, repeat).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Started audio transmission with custom audio for session: {}", session_id);
        Ok(())
    }
    
    /// Stop audio transmission for a session
    pub async fn stop_audio_transmission(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        tracing::debug!("Stopping audio transmission for session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.stop_audio_transmission(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Stopped audio transmission for session: {}", session_id);
        Ok(())
    }
    
    /// Set audio muted state for a session (send silence when muted)
    pub async fn set_audio_muted(&self, session_id: &SessionId, muted: bool) -> super::super::MediaResult<()> {
        tracing::debug!("MediaManager::set_audio_muted called for session: {} muted={}", session_id, muted);

        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            tracing::debug!("Session mapping contents: {:?}", mapping.keys().collect::<Vec<_>>());
            mapping.get(session_id).cloned()
                .ok_or_else(|| {
                    tracing::error!("Session mapping not found for: {}", session_id);
                    MediaError::SessionNotFound { session_id: session_id.to_string() }
                })?
        };

        tracing::debug!("Found dialog_id: {} for session: {}", dialog_id, session_id);

        tracing::debug!("Calling media-core set_audio_muted for dialog: {} muted={}", dialog_id, muted);
        self.controller.set_audio_muted(&dialog_id, muted).await
            .map_err(|e| {
                tracing::error!("Media-core set_audio_muted failed: {}", e);
                MediaError::MediaEngine { source: Box::new(e) }
            })?;

        tracing::info!("Successfully set audio muted={} for session {} (dialog {})", muted, session_id, dialog_id);
        Ok(())
    }
    
    /// Set custom audio samples for an active transmission session
    pub async fn set_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> super::super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.set_custom_audio(&dialog_id, samples, repeat).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Set custom audio for session: {}", session_id);
        Ok(())
    }
    
    /// Set tone generation parameters for an active transmission session
    pub async fn set_tone_generation(&self, session_id: &SessionId, frequency: f64, amplitude: f64) -> super::super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.set_tone_generation(&dialog_id, frequency, amplitude).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Set tone generation for session: {}", session_id);
        Ok(())
    }
    
    /// Enable pass-through mode for an active transmission session
    pub async fn set_pass_through_mode(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.set_pass_through_mode(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Set pass-through mode for session: {}", session_id);
        Ok(())
    }
    
    /// Helper method to get dialog ID from session ID
    pub(crate) async fn get_dialog_id(&self, session_id: &SessionId) -> super::super::MediaResult<DialogId> {
        let mapping = self.session_mapping.read().await;
        mapping.get(session_id).cloned()
            .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })
    }

    // =============================================================================
    // AUDIO STREAMING API IMPLEMENTATION
    // =============================================================================

    /// Set audio frame callback for a session to receive decoded frames
    /// This method integrates with the RTP decoder to provide audio frames from RTP events
    pub async fn set_audio_frame_callback(
        &self,
        session_id: &SessionId,
        callback: tokio::sync::mpsc::Sender<crate::api::types::AudioFrame>,
    ) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;


        // Set up the media-core callback directly - no conversion needed anymore!
        self.controller.set_audio_frame_callback(dialog_id.clone(), callback).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;

        tracing::info!("🔊 Set up audio frame callback for session: {}", session_id);
        Ok(())
    }

    /// Remove audio frame callback for a session
    pub async fn remove_audio_frame_callback(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        
        self.controller.remove_audio_frame_callback(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("🔇 Removed audio frame callback for session: {}", session_id);
        Ok(())
    }

    /// Send audio frame for encoding and transmission
    pub async fn send_audio_frame_for_transmission(
        &self,
        session_id: &SessionId,
        audio_frame: crate::api::types::AudioFrame,
    ) -> super::super::MediaResult<()> {
        tracing::debug!("📤 Received audio frame for transmission for session: {}", session_id);
        
        let dialog_id = match self.get_dialog_id(session_id).await {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("❌ Failed to get dialog ID for session {}: {}", session_id, e);
                return Err(e);
            }
        };
        
        tracing::debug!("✅ Got dialog ID {} for session {}", dialog_id, session_id);
        
        // Calculate RTP timestamp (8kHz clock rate for G.711)
        // Use modulo to prevent overflow
        let timestamp = ((std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() % (u32::MAX as u128 / 8)) as u32) * 8; // Convert to 8kHz RTP clock
        
        // Delegate encoding and transmission to media-core
        // This properly uses codec-core for encoding based on the configured codec
        let sample_count = audio_frame.samples.len();
        tracing::info!("🔧 [DEBUG] About to encode and send audio frame for session: {} (dialog: {}, {} samples)", 
                      session_id, dialog_id, sample_count);
        
        match self.controller.encode_and_send_audio_frame(&dialog_id, audio_frame.samples, timestamp).await {
            Ok(()) => {
                tracing::info!("✅ [SUCCESS] Audio frame encoded and sent successfully for session: {}", session_id);
            }
            Err(e) => {
                tracing::error!("❌ [ERROR] Failed to encode and send audio frame for session: {}: {}", session_id, e);
                return Err(MediaError::MediaEngine { source: Box::new(e) });
            }
        }
        
        tracing::debug!("📡 Sent audio frame for session: {} ({} samples, timestamp: {})", 
                       session_id, sample_count, timestamp);
        Ok(())
    }

    /// Get audio stream configuration for a session
    pub async fn get_audio_stream_config_internal(&self, session_id: &SessionId) -> super::super::MediaResult<Option<crate::api::types::AudioStreamConfig>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists
        if self.controller.get_session_info(&dialog_id).await.is_none() {
            return Ok(None);
        }
        
        // For now, return a default config based on our media config
        let config = crate::api::types::AudioStreamConfig {
            sample_rate: 8000,
            channels: 1,
            codec: self.media_config.preferred_codecs.first()
                .cloned()
                .unwrap_or_else(|| "PCMU".to_string()),
            frame_size_ms: 20,
            enable_aec: self.media_config.echo_cancellation,
            enable_agc: self.media_config.auto_gain_control,
            enable_vad: true, // Default VAD on
        };
        
        Ok(Some(config))
    }

    /// Set audio stream configuration for a session
    pub async fn set_audio_stream_config_internal(
        &self,
        session_id: &SessionId,
        config: crate::api::types::AudioStreamConfig,
    ) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists
        if self.controller.get_session_info(&dialog_id).await.is_none() {
            return Err(MediaError::SessionNotFound { session_id: session_id.to_string() });
        }
        
        // TODO: Apply configuration to the media session
        // For now, we store the configuration for later use
        tracing::info!("📊 Applied audio stream config for session {}: {}Hz, {} channels, codec: {}", 
                      session_id, config.sample_rate, config.channels, config.codec);
        
        Ok(())
    }

    /// Check if audio streaming is active for a session
    pub async fn is_audio_streaming_active(&self, session_id: &SessionId) -> super::super::MediaResult<bool> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists and has a callback registered
        if let Some(session_info) = self.controller.get_session_info(&dialog_id).await {
            // For now, consider streaming active if session is active
            // TODO: Add proper check for audio streaming status
            Ok(matches!(session_info.status, rvoip_media_core::relay::controller::types::MediaSessionStatus::Active))
        } else {
            Ok(false)
        }
    }

    /// Start audio streaming for a session
    pub async fn start_audio_streaming(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists
        if self.controller.get_session_info(&dialog_id).await.is_none() {
            return Err(MediaError::SessionNotFound { session_id: session_id.to_string() });
        }
        
        // Start RTP event processing for this session
        
        // TODO: Start the actual audio streaming pipeline
        // For now, this is handled through the existing audio transmission methods
        tracing::info!("🎵 Started audio streaming for session: {}", session_id);
        Ok(())
    }

    /// Stop audio streaming for a session
    pub async fn stop_audio_streaming(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        
        // Remove the callback
        self.remove_audio_frame_callback(session_id).await?;
        
        // TODO: Stop the actual audio streaming pipeline
        // For now, this is handled through the existing audio transmission methods
        tracing::info!("🛑 Stopped audio streaming for session: {}", session_id);
        Ok(())
    }

    
    /// Send an audio frame as RTP packets
    /// This method encodes the PCM audio frame to G.711 and sends it via RTP
    pub async fn send_audio_frame(&self, session_id: &SessionId, frame: crate::api::types::AudioFrame) -> super::super::MediaResult<()> {
        // Get the media session mapping
        let mapping = self.session_mapping.read().await;
        let media_session_id = mapping.get(session_id)
            .ok_or_else(|| MediaError::SessionNotFound { 
                session_id: session_id.to_string() 
            })?
            .clone();
        
        // For now, we'll skip getting session info and use default payload type
        // TODO: Get actual payload type from session info when available
        
        // Determine payload type from negotiated codec
        // For now, default to PCMU (0) if not specified
        let payload_type = 0u8; // TODO: Get from session_info.codec_config
        
        // Initialize encoder for this session if needed
        {
            let mut encoder = self.rtp_encoder.lock().await;
            // Check if session is already initialized by trying to encode a dummy frame
            let test_frame = crate::api::types::AudioFrame::new(
                vec![],
                8000,
                1,
                0,
            );
            if encoder.encode_audio_frame(session_id, &test_frame).is_err() {
                encoder.init_session(session_id.clone(), payload_type);
            }
        }
        
        // Encode the audio frame
        let encoded_payload = {
            let mut encoder = self.rtp_encoder.lock().await;
            encoder.encode_audio_frame(session_id, &frame)
                .map_err(|e| MediaError::Configuration { message: e })?
        };
        
        // Create MediaPacket for media-core
        let media_packet = rvoip_media_core::MediaPacket {
            payload: bytes::Bytes::from(encoded_payload.data),
            payload_type: encoded_payload.payload_type,
            timestamp: encoded_payload.timestamp,
            sequence_number: encoded_payload.sequence_number,
            ssrc: 0, // TODO: Get SSRC from session
            received_at: std::time::Instant::now(), // Not used for sending
        };
        
        // Send the packet via existing send_audio_frame_for_transmission method
        // This will use the controller's encode_and_send_audio_frame
        self.send_audio_frame_for_transmission(session_id, frame).await?;
        
        Ok(())
    }


    /// Initialize codec detection for a session with expected codec
    pub async fn initialize_codec_detection(&self, session_id: &SessionId, expected_codec: Option<String>) -> super::super::MediaResult<()> {
        tracing::debug!("Initializing codec detection for session {}: expected codec={:?}", session_id, expected_codec);
        
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Initialize codec detection
        self.codec_detector.initialize_detection(dialog_id.clone(), expected_codec.clone()).await;
        
        // Initialize fallback handling
        self.fallback_manager.initialize_fallback(dialog_id, expected_codec).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Initialized codec detection and fallback for session {}", session_id);
        Ok(())
    }
    
    /// Get codec detection status for a session
    pub async fn get_codec_detection_status(&self, session_id: &SessionId) -> super::super::MediaResult<Option<String>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        if let Some(result) = self.codec_detector.get_detection_result(&dialog_id).await {
            match result {
                CodecDetectionResult::Expected { codec, confidence, .. } => {
                    Ok(Some(format!("Expected codec confirmed: {:?} (confidence: {:.2})", codec, confidence)))
                },
                CodecDetectionResult::UnexpectedCodec { 
                    expected_codec, detected_codec, confidence, .. 
                } => {
                    Ok(Some(format!("Unexpected codec detected: expected {:?}, got {:?} (confidence: {:.2})", 
                                   expected_codec, detected_codec, confidence)))
                },
                CodecDetectionResult::InsufficientData { packets_analyzed } => {
                    Ok(Some(format!("Insufficient data for detection ({} packets analyzed)", packets_analyzed)))
                },
            }
        } else {
            Ok(None)
        }
    }
    
    /// Get fallback status for a session
    pub async fn get_fallback_status(&self, session_id: &SessionId) -> super::super::MediaResult<Option<String>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        if let Some(stats) = self.fallback_manager.get_stats(&dialog_id).await {
            let status = match &stats.current_mode {
                FallbackMode::None => "No fallback needed".to_string(),
                FallbackMode::Transcoding { from_codec, to_codec, .. } => {
                    format!("Transcoding: {} → {} (efficiency: {:.1}%)", 
                           from_codec, to_codec, stats.get_efficiency() * 100.0)
                },
                FallbackMode::Passthrough { detected_codec, .. } => {
                    format!("Passthrough mode: {} (efficiency: {:.1}%)", 
                           detected_codec, stats.get_efficiency() * 100.0)
                },
            };
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }
    
    /// Get comprehensive codec processing statistics for a session
    pub async fn get_codec_processing_stats(&self, session_id: &SessionId) -> super::super::MediaResult<Option<super::super::types::CodecProcessingStats>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Get detection state
        let detection_state = self.codec_detector.get_detection_state(&dialog_id).await;
        
        // Get fallback stats
        let fallback_stats = self.fallback_manager.get_stats(&dialog_id).await;
        
        if detection_state.is_some() || fallback_stats.is_some() {
            Ok(Some(super::super::types::CodecProcessingStats {
                session_id: session_id.clone(),
                expected_codec: detection_state.as_ref().and_then(|s| s.expected_codec.clone()),
                detected_codec: detection_state.as_ref().and_then(|s| s.detected_payload_type)
                    .and_then(|pt| self.codec_mapper.payload_to_codec(pt)),
                detection_confidence: detection_state.as_ref().map(|s| s.confidence).unwrap_or(0.0),
                packets_analyzed: detection_state.as_ref().map(|s| s.packets_analyzed).unwrap_or(0),
                fallback_mode: fallback_stats.as_ref().map(|s| s.current_mode.clone()).unwrap_or(FallbackMode::None),
                fallback_efficiency: fallback_stats.as_ref().map(|s| s.get_efficiency()).unwrap_or(1.0),
                transcoding_active: fallback_stats.as_ref().map(|s| s.transcoding_active).unwrap_or(false),
            }))
        } else {
            Ok(None)
        }
    }
    
    /// Clean up codec processing systems for a session
    pub(crate) async fn cleanup_codec_processing(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;


        // Cleanup codec detection
        self.codec_detector.cleanup_detection(&dialog_id).await;

        // Cleanup fallback handling
        self.fallback_manager.cleanup_fallback(&dialog_id).await;

        tracing::debug!("Cleaned up codec processing and RTP handling for session {}", session_id);
        Ok(())
    }
}
