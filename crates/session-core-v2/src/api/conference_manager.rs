//! Conference Manager - Multi-party call management
//!
//! This module provides conference call functionality including
//! creating conferences, managing participants, and audio mixing.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::types::{
    ConferenceId, MediaSessionId,
};
use crate::state_table::types::SessionId;
use crate::session_registry::SessionRegistry;
use crate::api::session_manager::SessionManager;
use crate::adapters::media_adapter::MediaAdapter;
use crate::errors::{Result, SessionError};

/// Conference participant information
#[derive(Debug, Clone)]
pub struct Participant {
    /// Session ID of the participant
    pub session_id: SessionId,
    /// Media session ID
    pub media_id: MediaSessionId,
    /// Participant name/URI
    pub name: String,
    /// Is participant muted
    pub is_muted: bool,
    /// Is participant speaking
    pub is_speaking: bool,
    /// Join time
    pub joined_at: std::time::SystemTime,
}

/// Conference state information
#[derive(Debug, Clone)]
pub struct ConferenceState {
    /// Conference ID
    pub id: ConferenceId,
    /// Conference name
    pub name: String,
    /// List of participants
    pub participants: Vec<Participant>,
    /// Audio mixer ID
    pub mixer_id: Option<MediaSessionId>,
    /// Conference creation time
    pub created_at: std::time::SystemTime,
    /// Is conference locked (no new participants)
    pub is_locked: bool,
    /// Is recording active
    pub is_recording: bool,
}

/// Conference manager for multi-party calls
pub struct ConferenceManager {
    /// Active conferences
    conferences: Arc<RwLock<HashMap<ConferenceId, ConferenceState>>>,
    /// Session to conference mapping
    session_to_conference: Arc<RwLock<HashMap<SessionId, ConferenceId>>>,
    /// Session manager
    session_manager: Arc<SessionManager>,
    /// Media adapter
    media_adapter: Arc<MediaAdapter>,
    /// Session registry
    registry: Arc<SessionRegistry>,
}

impl ConferenceManager {
    /// Create a new conference manager
    pub fn new(
        session_manager: Arc<SessionManager>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
    ) -> Self {
        Self {
            conferences: Arc::new(RwLock::new(HashMap::new())),
            session_to_conference: Arc::new(RwLock::new(HashMap::new())),
            session_manager,
            media_adapter,
            registry,
        }
    }

    /// Create a new conference
    pub async fn create(&self, name: String) -> Result<ConferenceId> {
        let conference_id = ConferenceId::new();
        info!("Creating conference {} with name: {}", conference_id, name);

        // Create audio mixer
        let mixer_id = self.media_adapter.create_audio_mixer().await?;

        let conference = ConferenceState {
            id: conference_id.clone(),
            name,
            participants: Vec::new(),
            mixer_id: Some(mixer_id),
            created_at: std::time::SystemTime::now(),
            is_locked: false,
            is_recording: false,
        };

        self.conferences.write().await.insert(conference_id.clone(), conference);

        Ok(conference_id)
    }

    /// Add a participant to a conference
    pub async fn add_participant(
        &self,
        conference_id: &ConferenceId,
        session_id: SessionId,
        name: String,
    ) -> Result<()> {
        info!("Adding participant {} to conference {}", name, conference_id);

        // Get conference
        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        // Check if conference is locked
        if conference.is_locked {
            return Err(SessionError::Other("Conference is locked".to_string()));
        }

        // Check if participant already in conference
        if conference.participants.iter().any(|p| p.session_id == session_id) {
            return Err(SessionError::Other("Participant already in conference".to_string()));
        }

        // Get media session for participant
        let media_id = self.registry.get_media_by_session(&session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Redirect audio to mixer
        if let Some(mixer_id) = &conference.mixer_id {
            self.media_adapter.redirect_to_mixer(media_id.clone(), mixer_id.clone()).await?;
        }

        // Add participant
        let participant = Participant {
            session_id: session_id.clone(),
            media_id,
            name: name.clone(),
            is_muted: false,
            is_speaking: false,
            joined_at: std::time::SystemTime::now(),
        };
        conference.participants.push(participant);

        // Update session mapping
        self.session_to_conference.write().await
            .insert(session_id.clone(), conference_id.clone());

        // Notify other participants
        self.notify_participant_joined(conference_id, &name).await;

        Ok(())
    }

    /// Remove a participant from a conference
    pub async fn remove_participant(
        &self,
        conference_id: &ConferenceId,
        session_id: &SessionId,
    ) -> Result<()> {
        info!("Removing participant {} from conference {}", session_id, conference_id);

        // Get conference
        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        // Find and remove participant
        let participant_index = conference.participants
            .iter()
            .position(|p| p.session_id == *session_id)
            .ok_or_else(|| SessionError::Other("Participant not in conference".to_string()))?;

        let participant = conference.participants.remove(participant_index);

        // Remove from mixer
        if let Some(mixer_id) = &conference.mixer_id {
            self.media_adapter.remove_from_mixer(participant.media_id, mixer_id.clone()).await?;
        }

        // Remove session mapping
        self.session_to_conference.write().await.remove(session_id);

        // Notify other participants
        self.notify_participant_left(conference_id, &participant.name).await;

        // If no participants left, optionally destroy conference
        if conference.participants.is_empty() {
            debug!("Conference {} is now empty", conference_id);
        }

        Ok(())
    }

    /// Mute a participant in a conference
    pub async fn mute_participant(
        &self,
        conference_id: &ConferenceId,
        session_id: &SessionId,
    ) -> Result<()> {
        debug!("Muting participant {} in conference {}", session_id, conference_id);

        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        let participant = conference.participants
            .iter_mut()
            .find(|p| p.session_id == *session_id)
            .ok_or_else(|| SessionError::Other("Participant not in conference".to_string()))?;

        participant.is_muted = true;
        
        // Mute in media adapter
        self.media_adapter.set_mute(participant.media_id.clone(), true).await?;

        Ok(())
    }

    /// Unmute a participant in a conference
    pub async fn unmute_participant(
        &self,
        conference_id: &ConferenceId,
        session_id: &SessionId,
    ) -> Result<()> {
        debug!("Unmuting participant {} in conference {}", session_id, conference_id);

        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        let participant = conference.participants
            .iter_mut()
            .find(|p| p.session_id == *session_id)
            .ok_or_else(|| SessionError::Other("Participant not in conference".to_string()))?;

        participant.is_muted = false;
        
        // Unmute in media adapter
        self.media_adapter.set_mute(participant.media_id.clone(), false).await?;

        Ok(())
    }

    /// Lock a conference (prevent new participants)
    pub async fn lock(&self, conference_id: &ConferenceId) -> Result<()> {
        info!("Locking conference {}", conference_id);

        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        conference.is_locked = true;
        Ok(())
    }

    /// Unlock a conference
    pub async fn unlock(&self, conference_id: &ConferenceId) -> Result<()> {
        info!("Unlocking conference {}", conference_id);

        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        conference.is_locked = false;
        Ok(())
    }

    /// Start recording a conference
    pub async fn start_recording(&self, conference_id: &ConferenceId) -> Result<()> {
        info!("Starting recording for conference {}", conference_id);

        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        if conference.is_recording {
            return Err(SessionError::Other("Conference is already being recorded".to_string()));
        }

        // Start recording on mixer
        if let Some(mixer_id) = &conference.mixer_id {
            // Convert MediaSessionId to SessionId for recording
            let session_id = SessionId(format!("mixer-{}", mixer_id.0));
            self.media_adapter.start_recording(&session_id).await?;
        }

        conference.is_recording = true;
        Ok(())
    }

    /// Stop recording a conference
    pub async fn stop_recording(&self, conference_id: &ConferenceId) -> Result<()> {
        info!("Stopping recording for conference {}", conference_id);

        let mut conferences = self.conferences.write().await;
        let conference = conferences.get_mut(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        if !conference.is_recording {
            return Err(SessionError::Other("Conference is not being recorded".to_string()));
        }

        // Stop recording on mixer
        if let Some(mixer_id) = &conference.mixer_id {
            // Convert MediaSessionId to SessionId for recording
            let session_id = SessionId(format!("mixer-{}", mixer_id.0));
            self.media_adapter.stop_recording(&session_id).await?;
        }

        conference.is_recording = false;
        Ok(())
    }

    /// Destroy a conference
    pub async fn destroy(&self, conference_id: &ConferenceId) -> Result<()> {
        info!("Destroying conference {}", conference_id);

        let mut conferences = self.conferences.write().await;
        let conference = conferences.remove(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        // Remove all participants from mixer
        if let Some(mixer_id) = &conference.mixer_id {
            for participant in &conference.participants {
                self.media_adapter.remove_from_mixer(
                    participant.media_id.clone(),
                    mixer_id.clone()
                ).await?;
            }
            
            // Destroy the mixer
            self.media_adapter.destroy_mixer(mixer_id.clone()).await?;
        }

        // Remove all session mappings
        let mut session_map = self.session_to_conference.write().await;
        for participant in &conference.participants {
            session_map.remove(&participant.session_id);
        }

        Ok(())
    }

    /// Get conference state
    pub async fn get_conference(&self, conference_id: &ConferenceId) -> Option<ConferenceState> {
        self.conferences.read().await.get(conference_id).cloned()
    }

    /// Get all active conferences
    pub async fn list_conferences(&self) -> Vec<ConferenceState> {
        self.conferences.read().await.values().cloned().collect()
    }

    /// Get conference for a session
    pub async fn get_session_conference(&self, session_id: &SessionId) -> Option<ConferenceId> {
        self.session_to_conference.read().await.get(session_id).cloned()
    }

    /// Get participants in a conference
    pub async fn get_participants(&self, conference_id: &ConferenceId) -> Result<Vec<Participant>> {
        let conferences = self.conferences.read().await;
        let conference = conferences.get(conference_id)
            .ok_or_else(|| SessionError::Other(format!("Conference {} not found", conference_id)))?;

        Ok(conference.participants.clone())
    }

    /// Check if a session is in any conference
    pub async fn is_in_conference(&self, session_id: &SessionId) -> bool {
        self.session_to_conference.read().await.contains_key(session_id)
    }

    /// Notify participants about new participant
    async fn notify_participant_joined(&self, conference_id: &ConferenceId, name: &str) {
        debug!("Notifying conference {} about new participant: {}", conference_id, name);
        // TODO: Implement notification system
    }

    /// Notify participants about participant leaving
    async fn notify_participant_left(&self, conference_id: &ConferenceId, name: &str) {
        debug!("Notifying conference {} about participant leaving: {}", conference_id, name);
        // TODO: Implement notification system
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::{UnifiedCoordinator, Config};

    async fn create_test_manager() -> ConferenceManager {
        create_test_manager_with_port(15062).await
    }
    
    async fn create_test_manager_with_port(port: u16) -> ConferenceManager {
        let config = Config {
            sip_port: port,
            media_port_start: 18000 + (port - 15062) * 1000,
            media_port_end: 19000 + (port - 15062) * 1000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: format!("127.0.0.1:{}", port).parse().unwrap(),
            state_table_path: None,
        };
        
        let coordinator = UnifiedCoordinator::new(config).await.unwrap();
        let session_manager = coordinator.session_manager().await.unwrap();
        let media_adapter = coordinator.media_adapter();
        let registry = coordinator.session_registry();
        
        ConferenceManager::new(
            session_manager,
            media_adapter,
            registry,
        )
    }

    #[tokio::test]
    async fn test_create_conference() {
        let manager = create_test_manager_with_port(15080).await;
        
        let conf_id = manager.create("Test Conference".to_string()).await.unwrap();
        
        let conf = manager.get_conference(&conf_id).await.unwrap();
        assert_eq!(conf.name, "Test Conference");
        assert_eq!(conf.participants.len(), 0);
        assert!(!conf.is_locked);
        assert!(!conf.is_recording);
    }

    #[tokio::test]
    async fn test_add_remove_participant() {
        let manager = create_test_manager_with_port(15081).await;
        
        let conf_id = manager.create("Test Conference".to_string()).await.unwrap();
        let session_id = SessionId::new();
        
        // Add media mapping for the session
        manager.registry.map_media(session_id.clone(), MediaSessionId::new());
        
        // Add participant
        manager.add_participant(&conf_id, session_id.clone(), "Alice".to_string()).await.unwrap();
        
        let participants = manager.get_participants(&conf_id).await.unwrap();
        assert_eq!(participants.len(), 1);
        assert_eq!(participants[0].name, "Alice");
        
        // Check session is in conference
        assert!(manager.is_in_conference(&session_id).await);
        
        // Remove participant
        manager.remove_participant(&conf_id, &session_id).await.unwrap();
        
        let participants = manager.get_participants(&conf_id).await.unwrap();
        assert_eq!(participants.len(), 0);
        
        // Check session is no longer in conference
        assert!(!manager.is_in_conference(&session_id).await);
    }

    #[tokio::test]
    async fn test_conference_locking() {
        let manager = create_test_manager_with_port(15082).await;
        
        let conf_id = manager.create("Test Conference".to_string()).await.unwrap();
        
        // Lock conference
        manager.lock(&conf_id).await.unwrap();
        
        let conf = manager.get_conference(&conf_id).await.unwrap();
        assert!(conf.is_locked);
        
        // Try to add participant to locked conference
        let session_id = SessionId::new();
        manager.registry.map_media(session_id.clone(), MediaSessionId::new());
        
        let result = manager.add_participant(&conf_id, session_id, "Bob".to_string()).await;
        assert!(result.is_err());
        
        // Unlock conference
        manager.unlock(&conf_id).await.unwrap();
        
        let conf = manager.get_conference(&conf_id).await.unwrap();
        assert!(!conf.is_locked);
    }

    #[tokio::test]
    async fn test_destroy_conference() {
        let manager = create_test_manager_with_port(15083).await;
        
        let conf_id = manager.create("Test Conference".to_string()).await.unwrap();
        
        // Add a participant
        let session_id = SessionId::new();
        manager.registry.map_media(session_id.clone(), MediaSessionId::new());
        manager.add_participant(&conf_id, session_id.clone(), "Alice".to_string()).await.unwrap();
        
        // Destroy conference
        manager.destroy(&conf_id).await.unwrap();
        
        // Conference should not exist
        assert!(manager.get_conference(&conf_id).await.is_none());
        
        // Session should not be in any conference
        assert!(!manager.is_in_conference(&session_id).await);
    }

}