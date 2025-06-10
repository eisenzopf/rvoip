//! Session Event System
//!
//! Integrates with infra-common zero-copy event system for high-performance session event handling.

use std::sync::Arc;
use std::any::Any;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use infra_common::events::{
    types::{Event, EventPriority, EventResult},
    system::EventSystem,
    builder::{EventSystemBuilder, ImplementationType},
    api::{EventSystem as EventSystemTrait, EventSubscriber},
};
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
    
    // ========== NEW: RTP Processing Events ==========
    
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

impl Event for SessionEvent {
    fn event_type() -> &'static str {
        "session_event"
    }
    
    fn priority() -> EventPriority {
        // Default priority for all session events, individual events can override if needed
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl SessionEvent {
    /// Get priority for this specific event instance
    pub fn get_priority(&self) -> EventPriority {
        match self {
            // High priority events - immediate session state changes
            SessionEvent::SessionCreated { .. } => EventPriority::High,
            SessionEvent::StateChanged { .. } => EventPriority::High,
            SessionEvent::SessionTerminated { .. } => EventPriority::High,
            
            // Critical priority events - RTP processing errors that need immediate attention
            SessionEvent::RtpProcessingError { .. } => EventPriority::Critical,
            
            // High priority events - RTP mode changes affect session performance
            SessionEvent::RtpProcessingModeChanged { .. } => EventPriority::High,
            
            // Normal priority events - regular RTP processing and statistics
            SessionEvent::RtpPacketProcessed { .. } => EventPriority::Normal,
            SessionEvent::RtpBufferPoolUpdate { .. } => EventPriority::Normal,
            
            // Normal priority events - media and session updates
            SessionEvent::MediaEvent { .. } => EventPriority::Normal,
            SessionEvent::DtmfReceived { .. } => EventPriority::Normal,
            SessionEvent::SessionHeld { .. } => EventPriority::Normal,
            SessionEvent::SessionResumed { .. } => EventPriority::Normal,
            SessionEvent::MediaUpdate { .. } => EventPriority::Normal,
            SessionEvent::SdpEvent { .. } => EventPriority::Normal,
            
            // Low priority events - errors and logging
            SessionEvent::Error { .. } => EventPriority::Low,
        }
    }
}

/// Event processor for session events using infra-common zero-copy event system
pub struct SessionEventProcessor {
    event_system: EventSystem,
    publisher: Arc<RwLock<Option<Box<dyn infra_common::events::api::EventPublisher<SessionEvent> + Send + Sync>>>>,
}

impl std::fmt::Debug for SessionEventProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionEventProcessor")
            .field("has_publisher", &self.publisher.try_read().map(|p| p.is_some()).unwrap_or(false))
            .finish()
    }
}

impl SessionEventProcessor {
    /// Create a new session event processor
    pub fn new() -> Self {
        let event_system = EventSystemBuilder::new()
            .implementation(ImplementationType::ZeroCopy)
            .channel_capacity(10_000)
            .max_concurrent_dispatches(1_000)
            .enable_priority(true)
            .shard_count(8)
            .enable_metrics(false)
            .build();

        Self {
            event_system,
            publisher: Arc::new(RwLock::new(None)),
        }
    }

    /// Start the event processor
    pub async fn start(&self) -> Result<()> {
        self.event_system.start()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to start event system: {}", e)))?;
        
        // Create publisher
        let publisher = self.event_system.create_publisher::<SessionEvent>();
        *self.publisher.write().await = Some(publisher);

        tracing::info!("Session event processor started");
        Ok(())
    }

    /// Stop the event processor
    pub async fn stop(&self) -> Result<()> {
        *self.publisher.write().await = None;
        
        self.event_system.shutdown()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to stop event system: {}", e)))?;

        tracing::info!("Session event processor stopped");
        Ok(())
    }

    /// Publish a session event with instance-specific priority
    pub async fn publish_event(&self, event: SessionEvent) -> Result<()> {
        let publisher = self.publisher.read().await;
        if let Some(publisher) = publisher.as_ref() {
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
            
            publisher.publish(event)
                .await
                .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to publish event: {}", e)))?;
        } else {
            tracing::warn!("Event processor not running, dropping event");
        }
        Ok(())
    }

    /// Subscribe to session events (for testing and monitoring)
    pub async fn subscribe(&self) -> Result<Box<dyn EventSubscriber<SessionEvent> + Send>> {
        let subscriber = self.event_system.subscribe::<SessionEvent>()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to subscribe to events: {}", e)))?;
        
        Ok(subscriber)
    }

    /// Subscribe to session events with a filter
    pub async fn subscribe_filtered<F>(&self, filter: F) -> Result<Box<dyn EventSubscriber<SessionEvent> + Send>>
    where
        F: Fn(&SessionEvent) -> bool + Send + Sync + 'static,
    {
        let subscriber = self.event_system.subscribe_filtered::<SessionEvent, F>(filter)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to subscribe to filtered events: {}", e)))?;
        
        Ok(subscriber)
    }

    /// Check if the event processor is running
    pub async fn is_running(&self) -> bool {
        self.publisher.read().await.is_some()
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