//! Session Event System
//!
//! Simple event system using tokio::sync::broadcast for session event handling.
//! Aligns with the event patterns used throughout the rest of the codebase.

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use serde::{Serialize, Deserialize};
use crate::api::types::{SessionId, CallSession, CallState, AudioFrame, AudioStreamConfig};
use crate::media::types::{RtpProcessingType, RtpProcessingMode, RtpProcessingMetrics, RtpBufferPoolStats};
use crate::errors::Result;
use chrono;

// ========== Supporting Types for Events ==========

/// Media quality alert levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MediaQualityAlertLevel {
    /// Good quality (MOS >= 4.0)
    Good,
    /// Fair quality (MOS >= 3.0)
    Fair,
    /// Poor quality (MOS >= 2.0)
    Poor,
    /// Critical quality (MOS < 2.0)
    Critical,
}

/// Media flow direction
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MediaFlowDirection {
    /// Sending media only
    Send,
    /// Receiving media only
    Receive,
    /// Both sending and receiving
    Both,
}

/// Warning categories for non-fatal issues
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WarningCategory {
    /// Network-related warnings
    Network,
    /// Media processing warnings
    Media,
    /// Protocol compliance warnings
    Protocol,
    /// Resource usage warnings
    Resource,
}

/// Session events that can be published through the event system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    /// Session was created
    SessionCreated { 
        session_id: SessionId, 
        from: String,
        to: String,
        call_state: CallState,
    },
    
    /// Session state changed
    StateChanged { 
        session_id: SessionId, 
        old_state: CallState, 
        new_state: CallState,
    },
    
    /// Enhanced state change event with metadata
    DetailedStateChange {
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
        timestamp: chrono::DateTime<chrono::Utc>,
        reason: Option<String>,
    },
    
    /// Session is terminating (Phase 1 - cleanup in progress)
    SessionTerminating {
        session_id: SessionId,
        reason: String,
    },
    
    /// Session was terminated (Phase 2 - cleanup complete)
    SessionTerminated { 
        session_id: SessionId, 
        reason: String,
    },
    
    /// Cleanup confirmation from a layer
    CleanupConfirmation {
        session_id: SessionId,
        layer: String,
    },
    
    /// Media event
    MediaEvent { 
        session_id: SessionId, 
        event: String,
    },
    
    /// Media quality metrics event
    MediaQuality {
        session_id: SessionId,
        mos_score: f32,
        packet_loss: f32,
        jitter_ms: f32,
        round_trip_ms: f32,
        alert_level: MediaQualityAlertLevel,
    },
    
    /// Media flow status change
    MediaFlowChange {
        session_id: SessionId,
        direction: MediaFlowDirection,
        active: bool,
        codec: String,
    },
    
    /// DTMF digits received
    DtmfReceived {
        session_id: SessionId,
        digits: String,
    },
    
    /// DTMF digit received (enhanced version)
    DtmfDigit {
        session_id: SessionId,
        digit: char,
        duration_ms: u32,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    
    /// Session was held
    SessionHeld {
        session_id: SessionId,
    },
    
    /// Session was resumed from hold
    SessionResumed {
        session_id: SessionId,
    },
    
    /// Media update requested (e.g., re-INVITE with new SDP)
    MediaUpdate {
        session_id: SessionId,
        offered_sdp: Option<String>,
    },
    
    /// Media negotiated successfully
    MediaNegotiated {
        session_id: SessionId,
        local_addr: std::net::SocketAddr,
        remote_addr: std::net::SocketAddr,
        codec: String,
    },
    
    /// SDP negotiation requested
    SdpNegotiationRequested {
        session_id: SessionId,
        role: String,  // "uac" or "uas"
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
    },
    
    /// SDP event (offer, answer, or update)
    SdpEvent {
        session_id: SessionId,
        event_type: String, // "local_sdp_offer", "remote_sdp_answer", "sdp_update", etc.
        sdp: String,
    },
    
    /// Non-fatal warning event
    Warning {
        session_id: Option<SessionId>,
        category: WarningCategory,
        message: String,
    },
    
    /// Error event
    Error { 
        session_id: Option<SessionId>, 
        error: String,
    },
    
    /// SIP REGISTER request received
    RegistrationRequest {
        transaction_id: String,
        from_uri: String,
        contact_uri: String,
        expires: u32,
    },
    
    // ========== RTP Processing Events ==========
    
    /// RTP packet processed with zero-copy optimization
    RtpPacketProcessed {
        session_id: SessionId,
        processing_type: RtpProcessingType,
        performance_metrics: RtpProcessingMetrics,
    },
    
    /// RTP processing mode changed for a session
    RtpProcessingModeChanged {
        session_id: SessionId,
        old_mode: RtpProcessingMode,
        new_mode: RtpProcessingMode,
    },
    
    /// RTP processing error occurred
    RtpProcessingError {
        session_id: SessionId,
        error: String,
        fallback_applied: bool,
    },
    
    /// RTP buffer pool statistics update
    RtpBufferPoolUpdate {
        stats: RtpBufferPoolStats,
    },
    
    // ========== AUDIO STREAMING EVENTS ==========
    
    /// Decoded audio frame received (for playback)
    AudioFrameReceived {
        session_id: SessionId,
        /// Decoded audio frame ready for playback
        audio_frame: AudioFrame,
        /// Stream identifier (multiple streams per session)
        stream_id: Option<String>,
    },
    
    /// Audio frame requested for capture and encoding
    AudioFrameRequested {
        session_id: SessionId,
        /// Expected audio format for the frame
        config: AudioStreamConfig,
        /// Stream identifier (multiple streams per session)
        stream_id: Option<String>,
    },
    
    /// Audio stream configuration changed
    AudioStreamConfigChanged {
        session_id: SessionId,
        /// Previous configuration
        old_config: AudioStreamConfig,
        /// New configuration
        new_config: AudioStreamConfig,
        /// Stream identifier
        stream_id: Option<String>,
    },
    
    /// Audio stream started
    AudioStreamStarted {
        session_id: SessionId,
        /// Stream configuration
        config: AudioStreamConfig,
        /// Stream identifier
        stream_id: String,
        /// Direction (Send, Receive, Both)
        direction: MediaFlowDirection,
    },
    
    /// Audio stream stopped
    AudioStreamStopped {
        session_id: SessionId,
        /// Stream identifier
        stream_id: String,
        /// Reason for stopping
        reason: String,
    },
}

/// Simple subscriber wrapper for session events
pub struct SessionEventSubscriber {
    receiver: broadcast::Receiver<SessionEvent>,
}

impl SessionEventSubscriber {
    pub fn new(receiver: broadcast::Receiver<SessionEvent>) -> Self {
        Self { receiver }
    }

    /// Receive the next event
    pub async fn receive(&mut self) -> Result<SessionEvent> {
        self.receiver.recv().await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to receive event: {}", e)))
    }

    /// Try to receive an event without blocking
    pub fn try_receive(&mut self) -> Result<Option<SessionEvent>> {
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(e) => Err(crate::errors::SessionError::internal(&format!("Failed to try receive event: {}", e))),
        }
    }
}

/// Event processor for session events using tokio::sync::broadcast
pub struct SessionEventProcessor {
    sender: Arc<RwLock<Option<broadcast::Sender<SessionEvent>>>>,
    is_running: Arc<RwLock<bool>>,
}

impl std::fmt::Debug for SessionEventProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionEventProcessor")
            .field("has_sender", &self.sender.try_read().map(|s| s.is_some()).unwrap_or(false))
            .finish()
    }
}

impl SessionEventProcessor {
    /// Create a new session event processor
    pub fn new() -> Self {
        Self {
            sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the event processor
    pub async fn start(&self) -> Result<()> {
        let (sender, _) = broadcast::channel(1000); // Buffer for 1000 events
        *self.sender.write().await = Some(sender);
        *self.is_running.write().await = true;
        
        tracing::info!("Session event processor started");
        Ok(())
    }

    /// Stop the event processor
    pub async fn stop(&self) -> Result<()> {
        *self.sender.write().await = None;
        *self.is_running.write().await = false;
        
        tracing::info!("Session event processor stopped");
        Ok(())
    }

    /// Publish a session event
    pub async fn publish_event(&self, event: SessionEvent) -> Result<()> {
        let sender_guard = self.sender.read().await;
        if let Some(sender) = sender_guard.as_ref() {
            // Log RTP events with more detail for monitoring
            match &event {
                SessionEvent::RtpPacketProcessed { session_id, processing_type, performance_metrics } => {
                    tracing::debug!(
                        "📡 RTP packet processed for session {}: {:?} (zero_copy: {}, traditional: {}, fallbacks: {})",
                        session_id,
                        processing_type,
                        performance_metrics.zero_copy_packets_processed,
                        performance_metrics.traditional_packets_processed,
                        performance_metrics.fallback_events
                    );
                }
                SessionEvent::RtpProcessingModeChanged { session_id, old_mode, new_mode } => {
                    tracing::info!(
                        "🔄 RTP processing mode changed for session {}: {:?} → {:?}",
                        session_id, old_mode, new_mode
                    );
                }
                SessionEvent::RtpProcessingError { session_id, error, fallback_applied } => {
                    if *fallback_applied {
                        tracing::warn!(
                            "⚠️ RTP processing error for session {} (fallback applied): {}",
                            session_id, error
                        );
                    } else {
                        tracing::error!(
                            "❌ RTP processing error for session {} (no fallback): {}",
                            session_id, error
                        );
                    }
                }
                SessionEvent::RtpBufferPoolUpdate { stats } => {
                    tracing::debug!(
                        "📊 RTP buffer pool update: {}/{} buffers in use ({}% efficiency)",
                        stats.in_use_buffers,
                        stats.total_buffers,
                        stats.efficiency_percentage
                    );
                }
                // Audio streaming events with detailed logging
                SessionEvent::AudioFrameReceived { session_id, audio_frame, stream_id } => {
                    tracing::debug!(
                        "🎵 Audio frame received for session {}: {} samples, {}Hz, {} channels{}",
                        session_id,
                        audio_frame.samples.len(),
                        audio_frame.sample_rate,
                        audio_frame.channels,
                        stream_id.as_ref().map(|s| format!(", stream: {}", s)).unwrap_or_default()
                    );
                }
                SessionEvent::AudioFrameRequested { session_id, config, stream_id } => {
                    tracing::debug!(
                        "🎤 Audio frame requested for session {}: {}Hz, {} channels, {}{}",
                        session_id,
                        config.sample_rate,
                        config.channels,
                        config.codec,
                        stream_id.as_ref().map(|s| format!(", stream: {}", s)).unwrap_or_default()
                    );
                }
                SessionEvent::AudioStreamConfigChanged { session_id, old_config, new_config, stream_id } => {
                    tracing::info!(
                        "🔧 Audio config changed for session {}: {}Hz → {}Hz, {} → {}{}",
                        session_id,
                        old_config.sample_rate,
                        new_config.sample_rate,
                        old_config.codec,
                        new_config.codec,
                        stream_id.as_ref().map(|s| format!(", stream: {}", s)).unwrap_or_default()
                    );
                }
                SessionEvent::AudioStreamStarted { session_id, config, stream_id, direction } => {
                    tracing::info!(
                        "▶️ Audio stream started for session {}: {} ({}Hz, {} channels, {:?})",
                        session_id,
                        stream_id,
                        config.sample_rate,
                        config.channels,
                        direction
                    );
                }
                SessionEvent::AudioStreamStopped { session_id, stream_id, reason } => {
                    tracing::info!(
                        "⏹️ Audio stream stopped for session {}: {} (reason: {})",
                        session_id,
                        stream_id,
                        reason
                    );
                }
                _ => {} // Other events use default logging
            }
            
            match sender.send(event) {
                Ok(_) => {}, // Event sent successfully
                Err(broadcast::error::SendError(_)) => {
                    // No receivers are currently listening, which is fine
                    tracing::debug!("No subscribers listening for event, but this is acceptable");
                }
            }
        } else {
            tracing::warn!("Event processor not running, dropping event");
        }
        Ok(())
    }

    /// Subscribe to session events
    pub async fn subscribe(&self) -> Result<SessionEventSubscriber> {
        let sender_guard = self.sender.read().await;
        if let Some(sender) = sender_guard.as_ref() {
            let receiver = sender.subscribe();
            Ok(SessionEventSubscriber::new(receiver))
        } else {
            Err(crate::errors::SessionError::internal("Event processor not running"))
        }
    }

    /// Subscribe to session events with a filter (compatibility method)
    pub async fn subscribe_filtered<F>(&self, _filter: F) -> Result<SessionEventSubscriber>
    where
        F: Fn(&SessionEvent) -> bool + Send + Sync + 'static,
    {
        // For now, just return a regular subscriber
        // Filtering can be done by the subscriber if needed
        self.subscribe().await
    }

    /// Check if the event processor is running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    // ========== RTP Event Publishing Helper Methods ==========

    /// Publish an RTP packet processed event
    pub async fn publish_rtp_packet_processed(
        &self,
        session_id: SessionId,
        processing_type: RtpProcessingType,
        performance_metrics: RtpProcessingMetrics,
    ) -> Result<()> {
        let event = SessionEvent::RtpPacketProcessed {
            session_id,
            processing_type,
            performance_metrics,
        };
        self.publish_event(event).await
    }

    /// Publish an RTP processing mode changed event
    pub async fn publish_rtp_processing_mode_changed(
        &self,
        session_id: SessionId,
        old_mode: RtpProcessingMode,
        new_mode: RtpProcessingMode,
    ) -> Result<()> {
        let event = SessionEvent::RtpProcessingModeChanged {
            session_id,
            old_mode,
            new_mode,
        };
        self.publish_event(event).await
    }

    /// Publish an RTP processing error event
    pub async fn publish_rtp_processing_error(
        &self,
        session_id: SessionId,
        error: String,
        fallback_applied: bool,
    ) -> Result<()> {
        let event = SessionEvent::RtpProcessingError {
            session_id,
            error,
            fallback_applied,
        };
        self.publish_event(event).await
    }

    /// Publish an RTP buffer pool update event
    pub async fn publish_rtp_buffer_pool_update(&self, stats: RtpBufferPoolStats) -> Result<()> {
        let event = SessionEvent::RtpBufferPoolUpdate { stats };
        self.publish_event(event).await
    }

    // ========== New Event Publishing Helper Methods ==========

    /// Publish a detailed state change event
    pub async fn publish_detailed_state_change(
        &self,
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
        reason: Option<String>,
    ) -> Result<()> {
        let event = SessionEvent::DetailedStateChange {
            session_id,
            old_state,
            new_state,
            timestamp: chrono::Utc::now(),
            reason,
        };
        self.publish_event(event).await
    }

    /// Publish a media quality event
    pub async fn publish_media_quality(
        &self,
        session_id: SessionId,
        mos_score: f32,
        packet_loss: f32,
        jitter_ms: f32,
        round_trip_ms: f32,
    ) -> Result<()> {
        let alert_level = if mos_score >= 4.0 {
            MediaQualityAlertLevel::Good
        } else if mos_score >= 3.0 {
            MediaQualityAlertLevel::Fair
        } else if mos_score >= 2.0 {
            MediaQualityAlertLevel::Poor
        } else {
            MediaQualityAlertLevel::Critical
        };

        let event = SessionEvent::MediaQuality {
            session_id,
            mos_score,
            packet_loss,
            jitter_ms,
            round_trip_ms,
            alert_level,
        };
        self.publish_event(event).await
    }

    /// Publish a DTMF digit event
    pub async fn publish_dtmf_digit(
        &self,
        session_id: SessionId,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        let event = SessionEvent::DtmfDigit {
            session_id,
            digit,
            duration_ms,
            timestamp: chrono::Utc::now(),
        };
        self.publish_event(event).await
    }

    /// Publish a media flow change event
    pub async fn publish_media_flow_change(
        &self,
        session_id: SessionId,
        direction: MediaFlowDirection,
        active: bool,
        codec: String,
    ) -> Result<()> {
        let event = SessionEvent::MediaFlowChange {
            session_id,
            direction,
            active,
            codec,
        };
        self.publish_event(event).await
    }

    /// Publish a warning event
    pub async fn publish_warning(
        &self,
        session_id: Option<SessionId>,
        category: WarningCategory,
        message: String,
    ) -> Result<()> {
        let event = SessionEvent::Warning {
            session_id,
            category,
            message,
        };
        self.publish_event(event).await
    }
    
    // ========== AUDIO STREAMING EVENT Publishing Helper Methods ==========
    
    /// Publish an audio frame received event
    pub async fn publish_audio_frame_received(
        &self,
        session_id: SessionId,
        audio_frame: AudioFrame,
        stream_id: Option<String>,
    ) -> Result<()> {
        let event = SessionEvent::AudioFrameReceived {
            session_id,
            audio_frame,
            stream_id,
        };
        self.publish_event(event).await
    }
    
    /// Publish an audio frame requested event
    pub async fn publish_audio_frame_requested(
        &self,
        session_id: SessionId,
        config: AudioStreamConfig,
        stream_id: Option<String>,
    ) -> Result<()> {
        let event = SessionEvent::AudioFrameRequested {
            session_id,
            config,
            stream_id,
        };
        self.publish_event(event).await
    }
    
    /// Publish an audio stream configuration changed event
    pub async fn publish_audio_stream_config_changed(
        &self,
        session_id: SessionId,
        old_config: AudioStreamConfig,
        new_config: AudioStreamConfig,
        stream_id: Option<String>,
    ) -> Result<()> {
        let event = SessionEvent::AudioStreamConfigChanged {
            session_id,
            old_config,
            new_config,
            stream_id,
        };
        self.publish_event(event).await
    }
    
    /// Publish an audio stream started event
    pub async fn publish_audio_stream_started(
        &self,
        session_id: SessionId,
        config: AudioStreamConfig,
        stream_id: String,
        direction: MediaFlowDirection,
    ) -> Result<()> {
        let event = SessionEvent::AudioStreamStarted {
            session_id,
            config,
            stream_id,
            direction,
        };
        self.publish_event(event).await
    }
    
    /// Publish an audio stream stopped event
    pub async fn publish_audio_stream_stopped(
        &self,
        session_id: SessionId,
        stream_id: String,
        reason: String,
    ) -> Result<()> {
        let event = SessionEvent::AudioStreamStopped {
            session_id,
            stream_id,
            reason,
        };
        self.publish_event(event).await
    }
}

impl Default for SessionEventProcessor {
    fn default() -> Self {
        Self::new()
    }
} 