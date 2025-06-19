//! Client builder for creating SIP clients

use std::sync::Arc;
use crate::{ClientConfig, ClientResult, client::ClientManager};
use super::config::{MediaConfig, MediaPreset};
use super::media_builder::MediaConfigBuilder;

/// Builder for creating a SIP client
pub struct ClientBuilder {
    config: ClientConfig,
}

impl ClientBuilder {
    /// Create a new client builder
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }
    
    /// Set the local SIP address
    pub fn local_address(mut self, addr: std::net::SocketAddr) -> Self {
        self.config.local_sip_addr = addr;
        self
    }
    
    /// Set the local media address
    pub fn media_address(mut self, addr: std::net::SocketAddr) -> Self {
        self.config.local_media_addr = addr;
        self
    }
    
    /// Set the user agent string
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }
    
    /// Set the SIP domain
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.config.domain = Some(domain.into());
        self
    }
    
    /// Set preferred codecs (convenience method)
    pub fn codecs<I, S>(mut self, codecs: I) -> Self 
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.media.preferred_codecs = codecs.into_iter().map(Into::into).collect();
        self
    }
    
    /// Configure media with a fluent sub-builder
    /// 
    /// # Example
    /// ```
    /// let client = ClientBuilder::new()
    ///     .with_media(|m| m
    ///         .codecs(vec!["opus", "PCMU"])
    ///         .require_srtp(true)
    ///         .echo_cancellation(true)
    ///         .max_bandwidth_kbps(128)
    ///     )
    ///     .build()
    ///     .await?;
    /// ```
    pub fn with_media<F>(mut self, f: F) -> Self 
    where
        F: FnOnce(MediaConfigBuilder) -> MediaConfigBuilder,
    {
        let builder = MediaConfigBuilder::new();
        self.config.media = f(builder).build();
        self
    }
    
    /// Set media configuration directly
    pub fn media_config(mut self, media: MediaConfig) -> Self {
        self.config.media = media;
        self
    }
    
    /// Apply a media preset
    /// 
    /// # Example
    /// ```
    /// let client = ClientBuilder::new()
    ///     .media_preset(MediaPreset::VoiceOptimized)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn media_preset(mut self, preset: MediaPreset) -> Self {
        self.config.media = MediaConfig::from_preset(preset);
        self
    }
    
    /// Enable or disable echo cancellation
    pub fn echo_cancellation(mut self, enabled: bool) -> Self {
        self.config.media.echo_cancellation = enabled;
        self
    }
    
    /// Enable or disable SRTP
    pub fn require_srtp(mut self, required: bool) -> Self {
        self.config.media.require_srtp = required;
        self
    }
    
    /// Set RTP port range
    pub fn rtp_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media.rtp_port_start = start;
        self.config.media.rtp_port_end = end;
        self
    }
    
    /// Set maximum concurrent calls
    pub fn max_concurrent_calls(mut self, max: usize) -> Self {
        self.config.max_concurrent_calls = max;
        self
    }
    
    /// Build the client
    pub async fn build(self) -> ClientResult<Arc<ClientManager>> {
        ClientManager::new(self.config).await
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
} 