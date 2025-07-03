//! Conference Events
//!
//! Defines events that occur during conference operations.
//! These events can be used for monitoring, logging, and integration.

use std::time::Instant;
use crate::api::types::SessionId;
use super::types::*;

/// Events that can occur in a conference
#[derive(Debug, Clone)]
pub enum ConferenceEvent {
    /// A new conference was created
    ConferenceCreated {
        conference_id: ConferenceId,
        config: ConferenceConfig,
        created_at: Instant,
    },

    /// A conference was terminated
    ConferenceTerminated {
        conference_id: ConferenceId,
        reason: String,
        terminated_at: Instant,
        final_stats: ConferenceStats,
    },

    /// A participant joined the conference
    ParticipantJoined {
        conference_id: ConferenceId,
        session_id: SessionId,
        participant_info: ParticipantInfo,
        joined_at: Instant,
    },

    /// A participant left the conference
    ParticipantLeft {
        conference_id: ConferenceId,
        session_id: SessionId,
        reason: String,
        left_at: Instant,
    },

    /// A participant's status changed
    ParticipantStatusChanged {
        conference_id: ConferenceId,
        session_id: SessionId,
        old_status: ParticipantStatus,
        new_status: ParticipantStatus,
        changed_at: Instant,
    },

    /// Conference state changed
    ConferenceStateChanged {
        conference_id: ConferenceId,
        old_state: ConferenceState,
        new_state: ConferenceState,
        changed_at: Instant,
    },

    /// Media session established for a participant
    ParticipantMediaEstablished {
        conference_id: ConferenceId,
        session_id: SessionId,
        rtp_port: u16,
        established_at: Instant,
    },

    /// Media session terminated for a participant
    ParticipantMediaTerminated {
        conference_id: ConferenceId,
        session_id: SessionId,
        terminated_at: Instant,
    },

    /// Conference statistics updated
    StatsUpdated {
        conference_id: ConferenceId,
        stats: ConferenceStats,
        updated_at: Instant,
    },

    /// Conference error occurred
    ConferenceError {
        conference_id: ConferenceId,
        error: String,
        occurred_at: Instant,
    },

    /// Participant error occurred
    ParticipantError {
        conference_id: ConferenceId,
        session_id: SessionId,
        error: String,
        occurred_at: Instant,
    },
}

impl ConferenceEvent {
    /// Get the conference ID associated with this event
    pub fn conference_id(&self) -> &ConferenceId {
        match self {
            ConferenceEvent::ConferenceCreated { conference_id, .. } => conference_id,
            ConferenceEvent::ConferenceTerminated { conference_id, .. } => conference_id,
            ConferenceEvent::ParticipantJoined { conference_id, .. } => conference_id,
            ConferenceEvent::ParticipantLeft { conference_id, .. } => conference_id,
            ConferenceEvent::ParticipantStatusChanged { conference_id, .. } => conference_id,
            ConferenceEvent::ConferenceStateChanged { conference_id, .. } => conference_id,
            ConferenceEvent::ParticipantMediaEstablished { conference_id, .. } => conference_id,
            ConferenceEvent::ParticipantMediaTerminated { conference_id, .. } => conference_id,
            ConferenceEvent::StatsUpdated { conference_id, .. } => conference_id,
            ConferenceEvent::ConferenceError { conference_id, .. } => conference_id,
            ConferenceEvent::ParticipantError { conference_id, .. } => conference_id,
        }
    }

    /// Get the session ID associated with this event (if any)
    pub fn session_id(&self) -> Option<&SessionId> {
        match self {
            ConferenceEvent::ParticipantJoined { session_id, .. } => Some(session_id),
            ConferenceEvent::ParticipantLeft { session_id, .. } => Some(session_id),
            ConferenceEvent::ParticipantStatusChanged { session_id, .. } => Some(session_id),
            ConferenceEvent::ParticipantMediaEstablished { session_id, .. } => Some(session_id),
            ConferenceEvent::ParticipantMediaTerminated { session_id, .. } => Some(session_id),
            ConferenceEvent::ParticipantError { session_id, .. } => Some(session_id),
            _ => None,
        }
    }

    /// Get the timestamp of when this event occurred
    pub fn timestamp(&self) -> Instant {
        match self {
            ConferenceEvent::ConferenceCreated { created_at, .. } => *created_at,
            ConferenceEvent::ConferenceTerminated { terminated_at, .. } => *terminated_at,
            ConferenceEvent::ParticipantJoined { joined_at, .. } => *joined_at,
            ConferenceEvent::ParticipantLeft { left_at, .. } => *left_at,
            ConferenceEvent::ParticipantStatusChanged { changed_at, .. } => *changed_at,
            ConferenceEvent::ConferenceStateChanged { changed_at, .. } => *changed_at,
            ConferenceEvent::ParticipantMediaEstablished { established_at, .. } => *established_at,
            ConferenceEvent::ParticipantMediaTerminated { terminated_at, .. } => *terminated_at,
            ConferenceEvent::StatsUpdated { updated_at, .. } => *updated_at,
            ConferenceEvent::ConferenceError { occurred_at, .. } => *occurred_at,
            ConferenceEvent::ParticipantError { occurred_at, .. } => *occurred_at,
        }
    }

    /// Check if this is an error event
    pub fn is_error(&self) -> bool {
        matches!(self, ConferenceEvent::ConferenceError { .. } | ConferenceEvent::ParticipantError { .. })
    }

    /// Check if this event involves a participant
    pub fn involves_participant(&self) -> bool {
        self.session_id().is_some()
    }
}

/// Trait for handling conference events
#[async_trait::async_trait]
pub trait ConferenceEventHandler: Send + Sync + 'static {
    /// Handle a conference event
    async fn handle_event(&self, event: ConferenceEvent);
}

/// Simple event handler that logs events
#[derive(Debug, Default)]
pub struct LoggingEventHandler;

#[async_trait::async_trait]
impl ConferenceEventHandler for LoggingEventHandler {
    async fn handle_event(&self, event: ConferenceEvent) {
        match &event {
            ConferenceEvent::ConferenceCreated { conference_id, .. } => {
                tracing::info!("🎪 Conference created: {}", conference_id);
            }
            ConferenceEvent::ConferenceTerminated { conference_id, reason, .. } => {
                tracing::info!("🎪 Conference terminated: {} (reason: {})", conference_id, reason);
            }
            ConferenceEvent::ParticipantJoined { conference_id, session_id, .. } => {
                tracing::info!("👤 Participant joined: {} in {}", session_id, conference_id);
            }
            ConferenceEvent::ParticipantLeft { conference_id, session_id, reason, .. } => {
                tracing::info!("👤 Participant left: {} from {} (reason: {})", session_id, conference_id, reason);
            }
            ConferenceEvent::ParticipantStatusChanged { conference_id, session_id, old_status, new_status, .. } => {
                tracing::info!("👤 Participant status changed: {} in {} ({:?} -> {:?})", 
                              session_id, conference_id, old_status, new_status);
            }
            ConferenceEvent::ConferenceStateChanged { conference_id, old_state, new_state, .. } => {
                tracing::info!("🎪 Conference state changed: {} ({:?} -> {:?})", 
                              conference_id, old_state, new_state);
            }
            ConferenceEvent::ParticipantMediaEstablished { conference_id, session_id, rtp_port, .. } => {
                tracing::info!("🎵 Media established: {} in {} (RTP port: {})", 
                              session_id, conference_id, rtp_port);
            }
            ConferenceEvent::ParticipantMediaTerminated { conference_id, session_id, .. } => {
                tracing::info!("🎵 Media terminated: {} in {}", session_id, conference_id);
            }
            ConferenceEvent::StatsUpdated { conference_id, stats, .. } => {
                tracing::debug!("📊 Stats updated: {} ({} participants)", 
                               conference_id, stats.total_participants);
            }
            ConferenceEvent::ConferenceError { conference_id, error, .. } => {
                tracing::error!("❌ Conference error: {} - {}", conference_id, error);
            }
            ConferenceEvent::ParticipantError { conference_id, session_id, error, .. } => {
                tracing::error!("❌ Participant error: {} in {} - {}", session_id, conference_id, error);
            }
        }
    }
} 