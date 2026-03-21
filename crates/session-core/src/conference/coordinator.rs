//! Conference Coordinator
//!
//! Coordinates between conferences and the session layer.
//! Bridges session-core and conference functionality.

use std::sync::Arc;
use crate::api::types::{SessionId, CallState};
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};
use super::types::*;
use super::manager::ConferenceManager;
use super::api::ConferenceApi;

/// Coordinates between conference management and session management
pub struct ConferenceCoordinator {
    session_coordinator: Arc<SessionCoordinator>,
    conference_manager: Arc<ConferenceManager>,
}

impl ConferenceCoordinator {
    /// Create a new conference coordinator
    pub fn new(session_coordinator: Arc<SessionCoordinator>, conference_manager: Arc<ConferenceManager>) -> Self {
        Self {
            session_coordinator,
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
        // Validate session exists in session coordinator's registry
        let session = self.session_coordinator.registry
            .get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(session_id.as_str()))?;

        // Verify session is in a valid state for joining a conference
        match session.call_session.state {
            CallState::Active => {}
            other => {
                return Err(SessionError::invalid_state(
                    &format!("Session {} is in state {:?}, must be Active to join conference", session_id, other)
                ));
            }
        }

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
        // Retrieve full session from the coordinator's registry
        let session = self.session_coordinator.registry
            .get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(session_id.as_str()))?;

        // Extract media port from the media manager if available
        let media_info = self.session_coordinator.media_manager
            .get_media_info(session_id).await
            .ok()
            .flatten();

        let media_port = media_info.as_ref()
            .and_then(|info| info.local_rtp_port);

        // Build SIP URI from the session's call data
        let sip_uri = if session.call_session.from.is_empty() {
            format!("sip:participant_{}@conference.local", session_id.as_str())
        } else {
            session.call_session.from.clone()
        };

        // Extract codec from negotiated media if available
        let negotiated = self.session_coordinator.get_negotiated_config(session_id).await;
        let remote_addr = negotiated.as_ref().map(|n| n.remote_addr);
        let codec_preferences = if let Some(ref neg) = negotiated {
            vec![neg.codec.clone()]
        } else {
            vec!["PCMU/8000".to_string(), "PCMA/8000".to_string()]
        };

        Ok(SessionInfo {
            session_id: session_id.clone(),
            sip_uri,
            remote_addr,
            media_port,
            codec_preferences,
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

        // Generate conference SDP using current timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Get local IP from the session coordinator's config
        let local_ip = self.session_coordinator.config.local_bind_addr.ip();

        Ok(format!(
            "v=0\r\n\
             o=conference_{} {} {} IN IP4 {}\r\n\
             s=Conference Room {}\r\n\
             c=IN IP4 {}\r\n\
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
            local_ip,
            conference_id,
            local_ip,
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
