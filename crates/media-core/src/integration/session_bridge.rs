//! Session Bridge - Integration with session-core
//!
//! This module provides the bridge between media-core and session-core for
//! SIP dialog coordination and codec negotiation.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, warn, info};

use crate::error::{Result, IntegrationError};
use crate::types::{DialogId, MediaSessionId};
use crate::session::{MediaSession, MediaSessionConfig};
use super::events::{
    IntegrationEvent, IntegrationEventType, MediaCapabilities, 
    AudioCodecCapability, CodecParameters
};

/// Configuration for session bridge
#[derive(Debug, Clone)]
pub struct SessionBridgeConfig {
    /// Default media session configuration
    pub default_session_config: MediaSessionConfig,
    /// Enable automatic codec negotiation
    pub enable_auto_negotiation: bool,
    /// Maximum concurrent sessions
    pub max_concurrent_sessions: usize,
}

impl Default for SessionBridgeConfig {
    fn default() -> Self {
        Self {
            default_session_config: MediaSessionConfig::default(),
            enable_auto_negotiation: true,
            max_concurrent_sessions: 1000,
        }
    }
}

/// SIP dialog information
#[derive(Debug, Clone)]
struct DialogInfo {
    /// Dialog ID
    dialog_id: DialogId,
    /// Associated media session ID
    media_session_id: MediaSessionId,
    /// Negotiated codec
    negotiated_codec: Option<String>,
    /// Dialog state
    state: DialogState,
    /// Creation timestamp
    created_at: std::time::Instant,
}

/// Dialog state
#[derive(Debug, Clone, PartialEq, Eq)]
enum DialogState {
    /// Dialog is being created
    Creating,
    /// Dialog is active
    Active,
    /// Dialog is being terminated
    Terminating,
    /// Dialog has been terminated
    Terminated,
}

/// Bridge between media-core and session-core
pub struct SessionBridge {
    /// Bridge configuration
    config: SessionBridgeConfig,
    /// Active dialogs
    dialogs: Arc<RwLock<HashMap<DialogId, DialogInfo>>>,
    /// Active media sessions
    #[allow(clippy::arc_with_non_send_sync)]
    media_sessions: Arc<RwLock<HashMap<MediaSessionId, Arc<MediaSession>>>>,
    /// Event channel for integration events
    event_tx: mpsc::UnboundedSender<IntegrationEvent>,
    /// Media session event receiver
    session_event_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<crate::session::MediaSessionEvent>>>>,
}

impl SessionBridge {
    /// Create a new session bridge
    pub fn new(
        config: SessionBridgeConfig,
        event_tx: mpsc::UnboundedSender<IntegrationEvent>,
    ) -> Self {
        debug!("Creating SessionBridge with config: {:?}", config);
        
        Self {
            config,
            dialogs: Arc::new(RwLock::new(HashMap::new())),
            #[allow(clippy::arc_with_non_send_sync)]
            media_sessions: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            session_event_rx: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Set up media session event channel
    pub async fn setup_session_events(
        &self,
        session_event_rx: mpsc::UnboundedReceiver<crate::session::MediaSessionEvent>,
    ) {
        let mut event_rx = self.session_event_rx.write().await;
        *event_rx = Some(session_event_rx);
        debug!("SessionBridge session event channel configured");
    }
    
    /// Get media capabilities for SDP negotiation
    pub fn get_media_capabilities(&self) -> MediaCapabilities {
        let mut capabilities = MediaCapabilities::default();
        
        // Add Opus codec if available
        #[cfg(feature = "opus")]
        {
            capabilities.audio_codecs.push(AudioCodecCapability {
                name: "Opus".to_string(),
                payload_type: 111, // Dynamic payload type
                sample_rates: vec![8000, 16000, 48000],
                channels: vec![1, 2],
                parameters: CodecParameters {
                    bitrate: Some(64000),
                    frame_size_ms: Some(20.0),
                    vbr: Some(true),
                    custom: std::collections::HashMap::new(),
                },
            });
        }
        
        capabilities
    }
    
    /// Handle codec negotiation request from session-core
    pub async fn handle_codec_negotiation(
        &self,
        dialog_id: DialogId,
        offered_codecs: Vec<String>,
    ) -> Result<String> {
        debug!("Handling codec negotiation for dialog {}: {:?}", dialog_id, offered_codecs);
        
        let our_capabilities = self.get_media_capabilities();
        let our_codec_names: Vec<String> = our_capabilities.audio_codecs
            .iter()
            .map(|c| c.name.clone())
            .collect();
        
        // Find first matching codec (preference order: Opus, PCMU, PCMA)
        let selected_codec = if offered_codecs.contains(&"Opus".to_string()) && our_codec_names.contains(&"Opus".to_string()) {
            "Opus".to_string()
        } else if offered_codecs.contains(&"PCMU".to_string()) && our_codec_names.contains(&"PCMU".to_string()) {
            "PCMU".to_string()
        } else if offered_codecs.contains(&"PCMA".to_string()) && our_codec_names.contains(&"PCMA".to_string()) {
            "PCMA".to_string()
        } else {
            return Err(IntegrationError::SessionCore {
                details: "No compatible codec found".to_string(),
            }.into());
        };
        
        // Send response event
        let codec_params = our_capabilities.audio_codecs
            .iter()
            .find(|c| c.name == selected_codec)
            .map(|c| c.parameters.clone())
            .unwrap_or_default();
        
        let event = IntegrationEvent::new(
            IntegrationEventType::CodecNegotiationResponse {
                dialog_id: dialog_id.clone(),
                selected_codec: selected_codec.clone(),
                parameters: codec_params,
            },
            "media-core",
            "session-core",
        );
        
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send codec negotiation response: {}", e);
        }
        
        info!("Selected codec {} for dialog {}", selected_codec, dialog_id);
        Ok(selected_codec)
    }
    
    /// Create media session for SIP dialog
    pub async fn create_media_session(&self, dialog_id: DialogId) -> Result<MediaSessionId> {
        // Check session limit
        {
            let sessions = self.media_sessions.read().await;
            if sessions.len() >= self.config.max_concurrent_sessions {
                return Err(IntegrationError::SessionCore {
                    details: "Maximum concurrent sessions reached".to_string(),
                }.into());
            }
        }
        
        // Create session event channel
        let (session_event_tx, session_event_rx) = mpsc::unbounded_channel();
        
        // Create media session
        let media_session_id = MediaSessionId::new(&format!("media-{}", dialog_id));
        #[allow(clippy::arc_with_non_send_sync)]
        let media_session = Arc::new(MediaSession::new(
            media_session_id.clone(),
            dialog_id.clone(),
            self.config.default_session_config.clone(),
            session_event_tx,
        )?);
        
        // Start the media session
        media_session.start().await?;
        
        // Store session and dialog info
        {
            let mut sessions = self.media_sessions.write().await;
            sessions.insert(media_session_id.clone(), media_session.clone());
        }
        
        {
            let mut dialogs = self.dialogs.write().await;
            dialogs.insert(dialog_id.clone(), DialogInfo {
                dialog_id: dialog_id.clone(),
                media_session_id: media_session_id.clone(),
                negotiated_codec: None,
                state: DialogState::Creating,
                created_at: std::time::Instant::now(),
            });
        }
        
        // Send media session ready event
        let capabilities = self.get_media_capabilities();
        let event = IntegrationEvent::media_session_ready(media_session_id.clone(), capabilities);
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send media session ready event: {}", e);
        }
        
        info!("Created media session {} for dialog {}", media_session_id, dialog_id);
        Ok(media_session_id)
    }
    
    /// Destroy media session for SIP dialog
    pub async fn destroy_media_session(&self, dialog_id: &DialogId) -> Result<()> {
        let media_session_id = {
            let mut dialogs = self.dialogs.write().await;
            if let Some(dialog_info) = dialogs.get_mut(dialog_id) {
                dialog_info.state = DialogState::Terminating;
                dialog_info.media_session_id.clone()
            } else {
                return Err(IntegrationError::SessionCore {
                    details: format!("Dialog {} not found", dialog_id),
                }.into());
            }
        };
        
        // Stop media session
        {
            let mut sessions = self.media_sessions.write().await;
            if let Some(session) = sessions.remove(&media_session_id) {
                session.stop().await?;
            }
        }
        
        // Update dialog state
        {
            let mut dialogs = self.dialogs.write().await;
            if let Some(dialog_info) = dialogs.get_mut(dialog_id) {
                dialog_info.state = DialogState::Terminated;
            }
        }
        
        // Send session destroyed event
        let event = IntegrationEvent::new(
            IntegrationEventType::MediaSessionDestroyed {
                session_id: media_session_id.clone(),
            },
            "media-core",
            "session-core",
        );
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send media session destroyed event: {}", e);
        }
        
        info!("Destroyed media session {} for dialog {}", media_session_id, dialog_id);
        Ok(())
    }
    
    /// Get media session for dialog
    pub async fn get_media_session(&self, dialog_id: &DialogId) -> Option<Arc<MediaSession>> {
        let dialogs = self.dialogs.read().await;
        let media_session_id = dialogs.get(dialog_id)?.media_session_id.clone();
        
        let sessions = self.media_sessions.read().await;
        sessions.get(&media_session_id).cloned()
    }
    
    /// Get all active dialogs
    pub async fn get_active_dialogs(&self) -> Vec<DialogId> {
        let dialogs = self.dialogs.read().await;
        dialogs
            .values()
            .filter(|d| d.state == DialogState::Active || d.state == DialogState::Creating)
            .map(|d| d.dialog_id.clone())
            .collect()
    }
    
    /// Get session statistics
    pub async fn get_session_count(&self) -> usize {
        let sessions = self.media_sessions.read().await;
        sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    
    #[tokio::test]
    async fn test_session_bridge_creation() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let bridge = SessionBridge::new(SessionBridgeConfig::default(), event_tx);
        
        let dialogs = bridge.get_active_dialogs().await;
        assert!(dialogs.is_empty());
        
        let session_count = bridge.get_session_count().await;
        assert_eq!(session_count, 0);
    }
    
    #[tokio::test]
    async fn test_media_capabilities() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let bridge = SessionBridge::new(SessionBridgeConfig::default(), event_tx);
        
        let capabilities = bridge.get_media_capabilities();
        assert!(!capabilities.audio_codecs.is_empty());
        
        // Should have at least PCMU and PCMA
        let codec_names: Vec<String> = capabilities.audio_codecs.iter().map(|c| c.name.clone()).collect();
        assert!(codec_names.contains(&"PCMU".to_string()));
        assert!(codec_names.contains(&"PCMA".to_string()));
    }
    
    #[tokio::test]
    async fn test_codec_negotiation() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let bridge = SessionBridge::new(SessionBridgeConfig::default(), event_tx);
        
        let dialog_id = DialogId::new("test-dialog");
        let offered_codecs = vec!["PCMU".to_string(), "G722".to_string()];
        
        let selected = bridge.handle_codec_negotiation(dialog_id, offered_codecs).await.unwrap();
        assert_eq!(selected, "PCMU");
    }
} 