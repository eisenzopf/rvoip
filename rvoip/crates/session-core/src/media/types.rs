//! Media Types for Session-Core Integration
//!
//! Modern type definitions adapted to the new session-core architecture,
//! providing clean interfaces between SIP signaling and media processing.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Session identifier for media coordination
pub type MediaSessionId = String;

/// RTP port number
pub type RtpPort = u16;

/// Media session information
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    pub session_id: MediaSessionId,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub local_rtp_port: Option<RtpPort>,
    pub remote_rtp_port: Option<RtpPort>,
    pub codec: Option<String>,
    pub quality_metrics: Option<QualityMetrics>,
}

impl Default for MediaSessionInfo {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            local_sdp: None,
            remote_sdp: None,
            local_rtp_port: None,
            remote_rtp_port: None,
            codec: None,
            quality_metrics: None,
        }
    }
}

/// Quality metrics for media sessions
#[derive(Debug, Clone)]
pub struct QualityMetrics {
    pub mos_score: Option<f32>,
    pub packet_loss: Option<f32>,
    pub jitter: Option<f32>,
    pub latency: Option<u32>,
}

/// Media capabilities supported by the engine
#[derive(Debug, Clone)]
pub struct MediaCapabilities {
    pub codecs: Vec<CodecInfo>,
    pub max_sessions: usize,
    pub port_range: (RtpPort, RtpPort),
}

/// Codec information
#[derive(Debug, Clone)]
pub struct CodecInfo {
    pub name: String,
    pub payload_type: u8,
    pub sample_rate: u32,
    pub channels: u8,
}

/// Media configuration for session setup
#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub preferred_codecs: Vec<String>,
    pub port_range: Option<(RtpPort, RtpPort)>,
    pub quality_monitoring: bool,
    pub dtmf_support: bool,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            port_range: Some((10000, 20000)),
            quality_monitoring: true,
            dtmf_support: true,
        }
    }
}

/// Media event types for session coordination
#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// Media session successfully established
    SessionEstablished {
        session_id: MediaSessionId,
        info: MediaSessionInfo,
    },
    
    /// Media session terminated
    SessionTerminated {
        session_id: MediaSessionId,
    },
    
    /// Quality metrics updated
    QualityUpdate {
        session_id: MediaSessionId,
        metrics: QualityMetrics,
    },
    
    /// DTMF tone detected
    DtmfDetected {
        session_id: MediaSessionId,
        tone: char,
        duration: u32,
    },
    
    /// Media error occurred
    Error {
        session_id: MediaSessionId,
        error: String,
    },
}

/// Media session state
#[derive(Debug, Clone, PartialEq)]
pub enum MediaSessionState {
    /// Session is being created
    Creating,
    /// Session is active and processing media
    Active,
    /// Session is on hold (media paused)
    OnHold,
    /// Session is being terminated
    Terminating,
    /// Session has been terminated
    Terminated,
}

/// Storage for active media sessions
pub type MediaSessionStorage = Arc<RwLock<HashMap<MediaSessionId, MediaSessionInfo>>>;

/// Trait for media engines that can be integrated with session-core
#[async_trait::async_trait]
pub trait MediaEngine: Send + Sync {
    /// Get supported media capabilities
    fn get_capabilities(&self) -> MediaCapabilities;
    
    /// Create a new media session
    async fn create_session(&self, config: &MediaConfig) -> Result<MediaSessionInfo, Box<dyn std::error::Error + Send + Sync>>;
    
    /// Update an existing media session with new SDP
    async fn update_session(&self, session_id: &MediaSessionId, sdp: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Terminate a media session
    async fn terminate_session(&self, session_id: &MediaSessionId) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Get current session information
    async fn get_session_info(&self, session_id: &MediaSessionId) -> Result<Option<MediaSessionInfo>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Mock media engine for testing and compilation
#[derive(Debug)]
pub struct MockMediaEngine {
    capabilities: MediaCapabilities,
    sessions: MediaSessionStorage,
}

impl MockMediaEngine {
    pub fn new() -> Self {
        Self {
            capabilities: MediaCapabilities {
                codecs: vec![
                    CodecInfo {
                        name: "PCMU".to_string(),
                        payload_type: 0,
                        sample_rate: 8000,
                        channels: 1,
                    },
                    CodecInfo {
                        name: "PCMA".to_string(),
                        payload_type: 8,
                        sample_rate: 8000,
                        channels: 1,
                    },
                ],
                max_sessions: 100,
                port_range: (10000, 20000),
            },
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl MediaEngine for MockMediaEngine {
    fn get_capabilities(&self) -> MediaCapabilities {
        self.capabilities.clone()
    }
    
    async fn create_session(&self, config: &MediaConfig) -> Result<MediaSessionInfo, Box<dyn std::error::Error + Send + Sync>> {
        let session_id = format!("mock-session-{}", uuid::Uuid::new_v4());
        let info = MediaSessionInfo {
            session_id: session_id.clone(),
            local_sdp: None,
            remote_sdp: None,
            local_rtp_port: Some(10000),
            remote_rtp_port: None,
            codec: config.preferred_codecs.first().cloned(),
            quality_metrics: None,
        };
        
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), info.clone());
        
        Ok(info)
    }
    
    async fn update_session(&self, session_id: &MediaSessionId, _sdp: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sessions = self.sessions.read().await;
        if sessions.contains_key(session_id) {
            Ok(())
        } else {
            Err("Session not found".into())
        }
    }
    
    async fn terminate_session(&self, session_id: &MediaSessionId) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
        Ok(())
    }
    
    async fn get_session_info(&self, session_id: &MediaSessionId) -> Result<Option<MediaSessionInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(session_id).cloned())
    }
}

impl Default for MockMediaEngine {
    fn default() -> Self {
        Self::new()
    }
} 