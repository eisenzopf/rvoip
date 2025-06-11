//! Session Event System
//!
//! Simple event system using tokio::sync::broadcast for session event handling.
//! Aligns with the event patterns used throughout the rest of the codebase.

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use serde::{Serialize, Deserialize};
use crate::api::types::{SessionId, CallSession, CallState};
use crate::media::types::{RtpProcessingType, RtpProcessingMode, RtpProcessingMetrics, RtpBufferPoolStats};
use crate::errors::Result;

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
    
    /// Session was terminated
    SessionTerminated { 
        session_id: SessionId, 
        reason: String,
    },
    
    /// Media event
    MediaEvent { 
        session_id: SessionId, 
        event: String,
    },
    
    /// DTMF digits received
    DtmfReceived {
        session_id: SessionId,
        digits: String,
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
    
    /// SDP event (offer, answer, or update)
    SdpEvent {
        session_id: SessionId,
        event_type: String, // "local_sdp_offer", "remote_sdp_answer", "sdp_update", etc.
        sdp: String,
    },
    
    /// Error event
    Error { 
        session_id: Option<SessionId>, 
        error: String,
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
}

impl Default for SessionEventProcessor {
    fn default() -> Self {
        Self::new()
    }
} 