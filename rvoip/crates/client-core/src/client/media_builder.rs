//! Media configuration builder for fluent API
//! 
//! Provides a convenient way to configure media preferences using the builder pattern.

use std::collections::HashMap;
use super::config::MediaConfig;

/// Builder for MediaConfig with fluent API
pub struct MediaConfigBuilder {
    config: MediaConfig,
}

impl Default for MediaConfigBuilder {
    fn default() -> Self {
        Self {
            config: MediaConfig::default(),
        }
    }
}

impl MediaConfigBuilder {
    /// Create a new media config builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set preferred codecs
    pub fn codecs<I, S>(mut self, codecs: I) -> Self 
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.preferred_codecs = codecs.into_iter().map(Into::into).collect();
        self
    }
    
    /// Add a single codec to the preference list
    pub fn add_codec(mut self, codec: impl Into<String>) -> Self {
        self.config.preferred_codecs.push(codec.into());
        self
    }
    
    /// Enable or disable DTMF
    pub fn dtmf(mut self, enabled: bool) -> Self {
        self.config.dtmf_enabled = enabled;
        self
    }
    
    /// Enable or disable echo cancellation
    pub fn echo_cancellation(mut self, enabled: bool) -> Self {
        self.config.echo_cancellation = enabled;
        self
    }
    
    /// Enable or disable noise suppression
    pub fn noise_suppression(mut self, enabled: bool) -> Self {
        self.config.noise_suppression = enabled;
        self
    }
    
    /// Enable or disable automatic gain control
    pub fn auto_gain_control(mut self, enabled: bool) -> Self {
        self.config.auto_gain_control = enabled;
        self
    }
    
    /// Set maximum bandwidth in kbps
    pub fn max_bandwidth_kbps(mut self, bandwidth: u32) -> Self {
        self.config.max_bandwidth_kbps = Some(bandwidth);
        self
    }
    
    /// Require SRTP encryption
    pub fn require_srtp(mut self, required: bool) -> Self {
        self.config.require_srtp = required;
        self
    }
    
    /// Set SRTP profiles
    pub fn srtp_profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.srtp_profiles = profiles.into_iter().map(Into::into).collect();
        self
    }
    
    /// Set RTP port range
    pub fn rtp_ports(mut self, range: std::ops::Range<u16>) -> Self {
        self.config.rtp_port_start = range.start;
        self.config.rtp_port_end = range.end;
        self
    }
    
    /// Set preferred packetization time
    pub fn ptime(mut self, ptime: u8) -> Self {
        self.config.preferred_ptime = Some(ptime);
        self
    }
    
    /// Add a custom SDP attribute
    pub fn custom_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.custom_sdp_attributes.insert(key.into(), value.into());
        self
    }
    
    /// Set all audio processing options at once
    pub fn audio_processing(mut self, enabled: bool) -> Self {
        self.config.echo_cancellation = enabled;
        self.config.noise_suppression = enabled;
        self.config.auto_gain_control = enabled;
        self
    }
    
    /// Build the MediaConfig
    pub fn build(self) -> MediaConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_config_builder() {
        let config = MediaConfigBuilder::new()
            .codecs(vec!["opus", "PCMU"])
            .require_srtp(true)
            .echo_cancellation(true)
            .max_bandwidth_kbps(128)
            .rtp_ports(30000..40000)
            .ptime(20)
            .custom_attribute("a=tool", "rvoip-client")
            .build();
        
        assert_eq!(config.preferred_codecs, vec!["opus", "PCMU"]);
        assert!(config.require_srtp);
        assert!(config.echo_cancellation);
        assert_eq!(config.max_bandwidth_kbps, Some(128));
        assert_eq!(config.rtp_port_start, 30000);
        assert_eq!(config.rtp_port_end, 40000);
        assert_eq!(config.preferred_ptime, Some(20));
        assert_eq!(config.custom_sdp_attributes.get("a=tool"), Some(&"rvoip-client".to_string()));
    }
    
    #[test]
    fn test_audio_processing_shortcut() {
        let config = MediaConfigBuilder::new()
            .audio_processing(false)
            .build();
        
        assert!(!config.echo_cancellation);
        assert!(!config.noise_suppression);
        assert!(!config.auto_gain_control);
    }
} 