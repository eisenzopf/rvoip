//! Conference Room
//!
//! Manages a single conference with multiple participants.
//! Provides participant management, state transitions, and media coordination.

use std::time::Instant;
use dashmap::DashMap;
use crate::api::types::SessionId;
use super::types::*;
use super::participant::ConferenceParticipant;
use crate::errors::Result;

/// Manages a single conference room with multiple participants
#[derive(Debug)]
pub struct ConferenceRoom {
    /// Conference ID
    pub id: ConferenceId,
    /// Conference configuration
    pub config: ConferenceConfig,
    /// Current state
    pub state: ConferenceState,
    /// Participants in this conference (concurrent HashMap)
    pub participants: DashMap<SessionId, ConferenceParticipant>,
    /// When the conference was created
    pub created_at: Instant,
    /// Last time the conference was updated
    pub last_updated: Instant,
}

impl ConferenceRoom {
    /// Create a new conference room
    pub fn new(id: ConferenceId, config: ConferenceConfig) -> Self {
        let now = Instant::now();
        Self {
            id,
            config,
            state: ConferenceState::Creating,
            participants: DashMap::new(),
            created_at: now,
            last_updated: now,
        }
    }

    /// Add a participant to the conference
    pub fn add_participant(&mut self, participant: ConferenceParticipant) -> Result<()> {
        // Validate capacity
        if self.participants.len() >= self.config.max_participants {
            return Err(crate::errors::SessionError::ResourceLimitExceeded("Conference is full".to_string()));
        }

        // Validate participant
        participant.validate()?;

        let session_id = participant.session_id.clone();
        self.participants.insert(session_id, participant);
        self.last_updated = Instant::now();
        Ok(())
    }

    /// Remove a participant from the conference
    pub fn remove_participant(&mut self, session_id: &SessionId) -> Option<ConferenceParticipant> {
        let result = self.participants.remove(session_id);
        if result.is_some() {
            self.last_updated = Instant::now();
        }
        result.map(|(_, participant)| participant)
    }

    /// Get statistics for this conference
    pub fn get_stats(&self) -> ConferenceStats {
        let active_participants = self.participants
            .iter()
            .filter(|entry| entry.value().is_active())
            .count();
        
        let audio_participants = self.participants
            .iter()
            .filter(|entry| entry.value().audio_active)
            .count();

        ConferenceStats {
            total_participants: self.participants.len(),
            active_participants,
            audio_participants,
            duration: self.created_at.elapsed(),
            state: self.state.clone(),
            audio_mixing_enabled: self.config.audio_mixing_enabled,
            created_at: self.created_at,
        }
    }

    /// Update conference state with validation
    pub fn set_state(&mut self, new_state: ConferenceState) -> Result<()> {
        // Validate state transitions
        match (&self.state, &new_state) {
            // Valid transitions
            (ConferenceState::Creating, ConferenceState::Active) => {},
            (ConferenceState::Active, ConferenceState::Locked) => {},
            (ConferenceState::Locked, ConferenceState::Active) => {},
            (ConferenceState::Active, ConferenceState::Terminating) => {},
            (ConferenceState::Locked, ConferenceState::Terminating) => {},
            (ConferenceState::Terminating, ConferenceState::Terminated) => {},
            // Same state is allowed
            (current, new) if current == new => {},
            // Invalid transitions
            _ => {
                return Err(crate::errors::SessionError::invalid_state(
                    &format!("Invalid state transition from {:?} to {:?}", self.state, new_state)
                ));
            }
        }

        self.state = new_state;
        self.last_updated = Instant::now();
        Ok(())
    }

    /// Get participant by session ID
    pub fn get_participant(&self, session_id: &SessionId) -> Option<ConferenceParticipant> {
        self.participants.get(session_id).map(|entry| entry.clone())
    }

    /// Update participant status
    pub fn update_participant_status(&mut self, session_id: &SessionId, status: ParticipantStatus) -> Result<()> {
        if let Some(mut participant_entry) = self.participants.get_mut(session_id) {
            participant_entry.update_status(status);
            self.last_updated = Instant::now();
            Ok(())
        } else {
            Err(crate::errors::SessionError::session_not_found(&session_id.to_string()))
        }
    }

    /// Set participant audio activity
    pub fn set_participant_audio(&mut self, session_id: &SessionId, active: bool) -> Result<()> {
        if let Some(mut participant_entry) = self.participants.get_mut(session_id) {
            participant_entry.set_audio_active(active);
            self.last_updated = Instant::now();
            Ok(())
        } else {
            Err(crate::errors::SessionError::session_not_found(&session_id.to_string()))
        }
    }

    /// Set participant RTP port
    pub fn set_participant_rtp_port(&mut self, session_id: &SessionId, port: u16) -> Result<()> {
        if let Some(mut participant_entry) = self.participants.get_mut(session_id) {
            participant_entry.set_rtp_port(port);
            self.last_updated = Instant::now();
            Ok(())
        } else {
            Err(crate::errors::SessionError::session_not_found(&session_id.to_string()))
        }
    }

    /// Check if conference is ready for media operations
    pub fn is_media_ready(&self) -> bool {
        match self.state {
            ConferenceState::Active => {
                // At least 2 participants needed for a conference
                self.participants.len() >= 2 &&
                // All participants should have RTP ports assigned
                self.participants.iter().all(|entry| entry.rtp_port.is_some())
            },
            _ => false
        }
    }

    /// Get all active participant session IDs
    pub fn get_active_participants(&self) -> Vec<SessionId> {
        self.participants
            .iter()
            .filter(|entry| entry.value().is_active())
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get participants by status
    pub fn get_participants_by_status(&self, status: ParticipantStatus) -> Vec<SessionId> {
        self.participants
            .iter()
            .filter(|entry| entry.value().status == status)
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Check if conference should be terminated (no active participants)
    pub fn should_terminate(&self) -> bool {
        let active_count = self.participants
            .iter()
            .filter(|entry| entry.value().is_active())
            .count();
        
        // Terminate if no active participants or only one left
        active_count <= 1
    }

    /// Get conference capacity utilization (0.0 to 1.0)
    pub fn capacity_utilization(&self) -> f64 {
        self.participants.len() as f64 / self.config.max_participants as f64
    }

    /// Check if conference is full
    pub fn is_full(&self) -> bool {
        self.participants.len() >= self.config.max_participants
    }

    /// Get conference age
    pub fn age(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }

    /// Initialize media mixing for active participants
    /// TODO: Integrate with media-core for actual audio mixing
    pub fn initialize_media_mixing(&self) -> Result<()> {
        if !self.config.audio_mixing_enabled {
            return Ok(());
        }

        // Placeholder for media-core integration
        // This would set up audio bridges between participants
        Ok(())
    }

    /// Generate basic conference SDP template
    /// TODO: Integrate with proper SDP generation from session-core
    pub fn generate_base_sdp(&self) -> String {
        format!(
            "v=0\r\n\
             s=Conference Room {}\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n",
            self.id
        )
    }
} 