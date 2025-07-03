//! Conference Participant
//!
//! Wraps a SessionId with conference-specific state and behavior.

use std::time::Instant;
use crate::api::types::SessionId;
use super::types::{ParticipantStatus, ParticipantInfo};
use crate::errors::Result;

/// Represents a participant in a conference
/// This wraps a SessionId with conference-specific state
#[derive(Debug, Clone)]
pub struct ConferenceParticipant {
    /// The underlying session ID
    pub session_id: SessionId,
    /// SIP URI of the participant
    pub sip_uri: String,
    /// Display name (if available)
    pub display_name: Option<String>,
    /// Current status in the conference
    pub status: ParticipantStatus,
    /// RTP port for media (if established)
    pub rtp_port: Option<u16>,
    /// Whether participant has audio active
    pub audio_active: bool,
    /// When the participant joined the conference
    pub joined_at: Instant,
    /// Last time participant status was updated
    pub last_updated: Instant,
}

impl ConferenceParticipant {
    /// Create a new conference participant
    pub fn new(session_id: SessionId, sip_uri: String) -> Self {
        let now = Instant::now();
        Self {
            session_id,
            sip_uri,
            display_name: None,
            status: ParticipantStatus::Joining,
            rtp_port: None,
            audio_active: false,
            joined_at: now,
            last_updated: now,
        }
    }

    /// Update the participant's status
    pub fn update_status(&mut self, new_status: ParticipantStatus) -> ParticipantStatus {
        let old_status = self.status.clone();
        self.status = new_status;
        self.last_updated = Instant::now();
        old_status
    }

    /// Set the participant's RTP port
    pub fn set_rtp_port(&mut self, port: u16) {
        self.rtp_port = Some(port);
        self.last_updated = Instant::now();
    }

    /// Set the participant's audio active status
    pub fn set_audio_active(&mut self, active: bool) {
        self.audio_active = active;
        self.last_updated = Instant::now();
    }

    /// Set the participant's display name
    pub fn set_display_name(&mut self, name: Option<String>) {
        self.display_name = name;
        self.last_updated = Instant::now();
    }

    /// Check if the participant is active in the conference
    pub fn is_active(&self) -> bool {
        matches!(self.status, ParticipantStatus::Active)
    }

    /// Check if the participant is muted
    pub fn is_muted(&self) -> bool {
        matches!(self.status, ParticipantStatus::Muted)
    }

    /// Check if the participant is on hold
    pub fn is_on_hold(&self) -> bool {
        matches!(self.status, ParticipantStatus::OnHold)
    }

    /// Check if the participant is leaving or has left
    pub fn is_leaving_or_left(&self) -> bool {
        matches!(self.status, ParticipantStatus::Leaving | ParticipantStatus::Left)
    }

    /// Get how long the participant has been in the conference
    pub fn duration_in_conference(&self) -> std::time::Duration {
        self.joined_at.elapsed()
    }

    /// Convert to ParticipantInfo for API responses
    pub fn to_participant_info(&self) -> ParticipantInfo {
        ParticipantInfo {
            session_id: self.session_id.clone(),
            sip_uri: self.sip_uri.clone(),
            display_name: self.display_name.clone(),
            status: self.status.clone(),
            rtp_port: self.rtp_port,
            audio_active: self.audio_active,
            joined_at: self.joined_at,
        }
    }

    /// Update from ParticipantInfo
    pub fn update_from_info(&mut self, info: &ParticipantInfo) {
        self.sip_uri = info.sip_uri.clone();
        self.display_name = info.display_name.clone();
        self.status = info.status.clone();
        self.rtp_port = info.rtp_port;
        self.audio_active = info.audio_active;
        self.last_updated = Instant::now();
    }

    /// Validate that the participant state is consistent
    pub fn validate(&self) -> Result<()> {
        // Check that session_id is valid
        if self.session_id.as_str().is_empty() {
            return Err(crate::errors::SessionError::invalid_state("Participant session_id cannot be empty"));
        }

        // Check that sip_uri is valid
        if self.sip_uri.is_empty() {
            return Err(crate::errors::SessionError::invalid_state("Participant sip_uri cannot be empty"));
        }

        // Check status transitions are valid
        match self.status {
            ParticipantStatus::Left => {
                // Left participants shouldn't have active audio or RTP ports
                if self.audio_active {
                    return Err(crate::errors::SessionError::invalid_state("Left participants cannot have active audio"));
                }
            }
            ParticipantStatus::Leaving => {
                // Leaving participants are transitioning out
                // They might still have audio/RTP until fully disconnected
            }
            _ => {
                // Other statuses are fine
            }
        }

        Ok(())
    }
} 