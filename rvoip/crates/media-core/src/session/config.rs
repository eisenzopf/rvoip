//! Media session configuration
//!
//! This module provides configuration options for media sessions.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

/// Media session configuration
#[derive(Debug, Clone)]
pub struct MediaSessionConfig {
    /// Session ID
    pub session_id: String,
    
    /// Media types enabled for this session
    pub media_types: Vec<MediaType>,
    
    /// DTMF mode
    pub dtmf_mode: DtmfMode,
    
    /// RTCP interval
    pub rtcp_interval: Duration,
    
    /// Audio jitter buffer size in milliseconds
    pub audio_jitter_buffer_ms: u32,
    
    /// Video jitter buffer size in milliseconds
    pub video_jitter_buffer_ms: u32,
    
    /// Enable SRTP
    pub srtp_enabled: bool,
    
    /// Enable RTP header extensions
    pub rtp_extensions_enabled: bool,
    
    /// Custom parameters
    pub parameters: HashMap<String, String>,
}

impl Default for MediaSessionConfig {
    fn default() -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            media_types: vec![MediaType::Audio],
            dtmf_mode: DtmfMode::Rfc2833,
            rtcp_interval: Duration::from_secs(5),
            audio_jitter_buffer_ms: 60,
            video_jitter_buffer_ms: 100,
            srtp_enabled: true,
            rtp_extensions_enabled: true,
            parameters: HashMap::new(),
        }
    }
}

/// Media type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Audio media
    Audio,
    
    /// Video media
    Video,
}

/// DTMF handling mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtmfMode {
    /// RFC 2833 (RTP events)
    Rfc2833,
    
    /// SIP INFO method
    SipInfo,
    
    /// In-band DTMF tones
    InBand,
    
    /// No DTMF support
    None,
}

/// Builder for session configuration
pub struct MediaSessionConfigBuilder {
    config: MediaSessionConfig,
}

impl MediaSessionConfigBuilder {
    /// Create a new config builder
    pub fn new() -> Self {
        Self {
            config: MediaSessionConfig::default(),
        }
    }
    
    /// Set the session ID
    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.config.session_id = id.into();
        self
    }
    
    /// Set media types
    pub fn with_media_types(mut self, types: Vec<MediaType>) -> Self {
        self.config.media_types = types;
        self
    }
    
    /// Add audio support
    pub fn with_audio(mut self) -> Self {
        if !self.config.media_types.contains(&MediaType::Audio) {
            self.config.media_types.push(MediaType::Audio);
        }
        self
    }
    
    /// Add video support
    pub fn with_video(mut self) -> Self {
        if !self.config.media_types.contains(&MediaType::Video) {
            self.config.media_types.push(MediaType::Video);
        }
        self
    }
    
    /// Set DTMF mode
    pub fn with_dtmf_mode(mut self, mode: DtmfMode) -> Self {
        self.config.dtmf_mode = mode;
        self
    }
    
    /// Set RTCP interval
    pub fn with_rtcp_interval(mut self, interval: Duration) -> Self {
        self.config.rtcp_interval = interval;
        self
    }
    
    /// Set audio jitter buffer size
    pub fn with_audio_jitter_buffer(mut self, ms: u32) -> Self {
        self.config.audio_jitter_buffer_ms = ms;
        self
    }
    
    /// Set video jitter buffer size
    pub fn with_video_jitter_buffer(mut self, ms: u32) -> Self {
        self.config.video_jitter_buffer_ms = ms;
        self
    }
    
    /// Enable or disable SRTP
    pub fn with_srtp(mut self, enabled: bool) -> Self {
        self.config.srtp_enabled = enabled;
        self
    }
    
    /// Enable or disable RTP extensions
    pub fn with_rtp_extensions(mut self, enabled: bool) -> Self {
        self.config.rtp_extensions_enabled = enabled;
        self
    }
    
    /// Add a custom parameter
    pub fn with_parameter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.parameters.insert(key.into(), value.into());
        self
    }
    
    /// Build the configuration
    pub fn build(self) -> MediaSessionConfig {
        self.config
    }
} 