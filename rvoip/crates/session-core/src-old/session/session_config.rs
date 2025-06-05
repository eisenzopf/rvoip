use std::net::SocketAddr;
use crate::media::AudioCodecType;

/// Session configuration
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Local address for signaling
    pub local_signaling_addr: SocketAddr,
    
    /// Local address for media
    pub local_media_addr: SocketAddr,
    
    /// Supported audio codecs
    pub supported_codecs: Vec<AudioCodecType>,
    
    /// Default display name
    pub display_name: Option<String>,
    
    /// User agent identifier
    pub user_agent: String,
    
    /// Maximum call duration in seconds (0 for unlimited)
    pub max_duration: u32,
    
    /// Maximum number of concurrent sessions (None for unlimited)
    pub max_sessions: Option<usize>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            local_signaling_addr: "0.0.0.0:5060".parse().unwrap(),
            local_media_addr: "0.0.0.0:10000".parse().unwrap(),
            supported_codecs: vec![AudioCodecType::PCMU, AudioCodecType::PCMA],
            display_name: None,
            user_agent: "RVOIP/0.1.0".to_string(),
            max_duration: 0,
            max_sessions: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_config_default() {
        let config = SessionConfig::default();
        
        assert_eq!(config.local_signaling_addr.to_string(), "0.0.0.0:5060");
        assert_eq!(config.local_media_addr.to_string(), "0.0.0.0:10000");
        assert_eq!(config.supported_codecs.len(), 2);
        assert_eq!(config.display_name, None);
        assert_eq!(config.user_agent, "RVOIP/0.1.0");
        assert_eq!(config.max_duration, 0);
        assert_eq!(config.max_sessions, None);
    }
    
    #[test]
    fn test_session_config_custom() {
        let config = SessionConfig {
            local_signaling_addr: "127.0.0.1:5080".parse().unwrap(),
            local_media_addr: "127.0.0.1:10001".parse().unwrap(),
            supported_codecs: vec![AudioCodecType::PCMU],
            display_name: Some("Test User".to_string()),
            user_agent: "TestAgent/1.0".to_string(),
            max_duration: 3600,
            max_sessions: Some(1000),
        };
        
        assert_eq!(config.local_signaling_addr.to_string(), "127.0.0.1:5080");
        assert_eq!(config.local_media_addr.to_string(), "127.0.0.1:10001");
        assert_eq!(config.supported_codecs.len(), 1);
        assert_eq!(config.display_name, Some("Test User".to_string()));
        assert_eq!(config.user_agent, "TestAgent/1.0");
        assert_eq!(config.max_duration, 3600);
        assert_eq!(config.max_sessions, Some(1000));
    }
} 