//! Conference API Trait
//!
//! Defines the main interface for conference operations.
//! This trait provides a clean abstraction for conference management.

use async_trait::async_trait;
use crate::api::types::SessionId;
use crate::errors::Result;
use super::types::*;

/// Main API trait for conference operations
#[async_trait]
pub trait ConferenceApi: Send + Sync {
    /// Create a new conference with the given configuration
    async fn create_conference(&self, config: ConferenceConfig) -> Result<ConferenceId>;

    /// Create a conference with a specific ID/name
    async fn create_named_conference(&self, id: ConferenceId, config: ConferenceConfig) -> Result<()>;

    /// Join a participant (session) to a conference
    async fn join_conference(&self, conference_id: &ConferenceId, session_id: &SessionId) -> Result<ParticipantInfo>;

    /// Remove a participant from a conference
    async fn leave_conference(&self, conference_id: &ConferenceId, session_id: &SessionId) -> Result<()>;

    /// List all participants in a conference
    async fn list_participants(&self, conference_id: &ConferenceId) -> Result<Vec<ParticipantInfo>>;

    /// Get conference statistics
    async fn get_conference_stats(&self, conference_id: &ConferenceId) -> Result<ConferenceStats>;

    /// List all active conferences
    async fn list_conferences(&self) -> Result<Vec<ConferenceId>>;

    /// Terminate a conference
    async fn terminate_conference(&self, conference_id: &ConferenceId) -> Result<()>;

    /// Update participant status (e.g., mute/unmute)
    async fn update_participant_status(&self, conference_id: &ConferenceId, session_id: &SessionId, status: ParticipantStatus) -> Result<()>;

    /// Generate SDP for a participant joining the conference
    async fn generate_conference_sdp(&self, conference_id: &ConferenceId, session_id: &SessionId) -> Result<String>;

    /// Check if a conference exists
    async fn conference_exists(&self, conference_id: &ConferenceId) -> bool;

    /// Get conference configuration
    async fn get_conference_config(&self, conference_id: &ConferenceId) -> Result<ConferenceConfig>;

    /// Update conference configuration (if supported)
    async fn update_conference_config(&self, conference_id: &ConferenceId, config: ConferenceConfig) -> Result<()>;
}

/// Convenience functions for common conference operations
pub trait ConferenceApiExt: ConferenceApi {
    /// Join a participant by SIP URI (creates session first)
    async fn join_by_sip_uri(&self, conference_id: &ConferenceId, sip_uri: &str) -> Result<SessionId> {
        // This would integrate with SessionManager to create a session first
        // For now, return an error indicating it needs to be implemented
        Err(crate::errors::SessionError::Other("join_by_sip_uri requires SessionManager integration".to_string()))
    }

    /// Invite a participant to join the conference
    async fn invite_participant(&self, conference_id: &ConferenceId, sip_uri: &str) -> Result<SessionId> {
        // This would create an outgoing call to invite the participant
        Err(crate::errors::SessionError::Other("invite_participant requires SessionManager integration".to_string()))
    }

    /// Kick a participant from the conference
    async fn kick_participant(&self, conference_id: &ConferenceId, session_id: &SessionId, reason: &str) -> Result<()> {
        // Update status to leaving and terminate the session
        self.update_participant_status(conference_id, session_id, ParticipantStatus::Leaving).await?;
        self.leave_conference(conference_id, session_id).await
    }

    /// Mute all participants in a conference
    async fn mute_all(&self, conference_id: &ConferenceId) -> Result<()> {
        let participants = self.list_participants(conference_id).await?;
        for participant in participants {
            if participant.status == ParticipantStatus::Active {
                let _ = self.update_participant_status(conference_id, &participant.session_id, ParticipantStatus::Muted).await;
            }
        }
        Ok(())
    }

    /// Unmute all participants in a conference
    async fn unmute_all(&self, conference_id: &ConferenceId) -> Result<()> {
        let participants = self.list_participants(conference_id).await?;
        for participant in participants {
            if participant.status == ParticipantStatus::Muted {
                let _ = self.update_participant_status(conference_id, &participant.session_id, ParticipantStatus::Active).await;
            }
        }
        Ok(())
    }

    /// Lock a conference (prevent new participants)
    async fn lock_conference(&self, conference_id: &ConferenceId) -> Result<()> {
        // This would update the conference state to Locked
        // Implementation depends on how conference state is managed
        Err(crate::errors::SessionError::Other("lock_conference not yet implemented".to_string()))
    }

    /// Unlock a conference (allow new participants)
    async fn unlock_conference(&self, conference_id: &ConferenceId) -> Result<()> {
        // This would update the conference state to Active
        Err(crate::errors::SessionError::Other("unlock_conference not yet implemented".to_string()))
    }
}

/// Automatically implement the extension trait for any type that implements ConferenceApi
impl<T: ConferenceApi> ConferenceApiExt for T {} 