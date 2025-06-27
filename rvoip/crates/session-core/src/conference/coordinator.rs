//! Conference Coordinator
//!
//! Coordinates between conferences and the session layer.
//! Bridges session-core and conference functionality.

use std::sync::Arc;
use crate::api::types::SessionId;
use crate::manager::core::SessionManager;
use crate::errors::Result;
use super::types::*;
use super::manager::ConferenceManager;
use super::api::ConferenceApi;

/// Coordinates between conference management and session management
pub struct ConferenceCoordinator {
    session_manager: Arc<SessionManager>,
    conference_manager: Arc<ConferenceManager>,
}

impl ConferenceCoordinator {
    /// Create a new conference coordinator
    pub fn new(session_manager: Arc<SessionManager>, conference_manager: Arc<ConferenceManager>) -> Self {
        Self {
            session_manager,
            conference_manager,
        }
    }

    /// Create a new conference and return its ID
    pub async fn create_conference(&self, config: ConferenceConfig) -> Result<ConferenceId> {
        self.conference_manager.create_conference(config).await
    }

    /// Add a session to a conference
    pub async fn add_session_to_conference(
        &self,
        conference_id: &ConferenceId,
        session_id: &SessionId,
    ) -> Result<ParticipantInfo> {
        // TODO: Validate session exists in session manager
        // For now, assume session is valid
        
        // Add to conference
        self.conference_manager.join_conference(conference_id, session_id).await
    }

    /// Remove a session from a conference
    pub async fn remove_session_from_conference(
        &self,
        conference_id: &ConferenceId,
        session_id: &SessionId,
    ) -> Result<()> {
        self.conference_manager.leave_conference(conference_id, session_id).await
    }

    /// Get session information for generating conference SDP
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        // Get session details from session manager
        // TODO: Implement proper session info retrieval from SessionManager
        Ok(SessionInfo {
            session_id: session_id.clone(),
            sip_uri: format!("sip:participant_{}@conference.local", session_id.as_str()),
            remote_addr: None,
            media_port: None,
            codec_preferences: vec!["PCMU/8000".to_string(), "PCMA/8000".to_string()],
        })
    }

    /// Generate conference-specific SDP for a participant
    pub async fn generate_conference_sdp(
        &self,
        conference_id: &ConferenceId,
        session_id: &SessionId,
        media_port: u16,
    ) -> Result<String> {
        // Get conference configuration
        let config = self.conference_manager.get_conference_config(conference_id).await?;
        
        // Get participant count for session naming
        let participants = self.conference_manager.list_participants(conference_id).await?;
        let participant_count = participants.len();

        // Generate conference SDP using current timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(format!(
            "v=0\r\n\
             o=conference_{} {} {} IN IP4 127.0.0.1\r\n\
             s=Conference Room {}\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8\r\n\
             a=sendrecv\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=ptime:20\r\n\
             a=maxptime:40\r\n\
             {}",
            session_id.as_str(),
            timestamp,
            timestamp,
            conference_id,
            media_port,
            if config.audio_mixing_enabled {
                "a=conf:audio-mixing\r\n"
            } else {
                ""
            }
        ))
    }

    /// Coordinate session termination with conference cleanup
    pub async fn handle_session_termination(&self, session_id: &SessionId) -> Result<()> {
        // Find all conferences this session belongs to
        let conferences = self.conference_manager.list_conferences().await?;
        
        for conference_id in conferences {
            // Check if this session is in this conference
            if let Ok(participants) = self.conference_manager.list_participants(&conference_id).await {
                if participants.iter().any(|p| &p.session_id == session_id) {
                    // Remove from conference
                    let _ = self.conference_manager.leave_conference(&conference_id, session_id).await;
                }
            }
        }

        Ok(())
    }

    /// Get all conferences a session belongs to
    pub async fn get_session_conferences(&self, session_id: &SessionId) -> Result<Vec<ConferenceId>> {
        let mut session_conferences = Vec::new();
        let conferences = self.conference_manager.list_conferences().await?;
        
        for conference_id in conferences {
            if let Ok(participants) = self.conference_manager.list_participants(&conference_id).await {
                if participants.iter().any(|p| &p.session_id == session_id) {
                    session_conferences.push(conference_id);
                }
            }
        }

        Ok(session_conferences)
    }
}

/// Information about a session for conference coordination
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: SessionId,
    pub sip_uri: String,
    pub remote_addr: Option<std::net::SocketAddr>,
    pub media_port: Option<u16>,
    pub codec_preferences: Vec<String>,
}
