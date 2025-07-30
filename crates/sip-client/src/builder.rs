//! Builder pattern for creating SIP clients

use crate::{
    error::{SipClientError, SipClientResult},
    types::{AudioConfig, CodecConfig, CodecPriority, RegistrationConfig, SipClientConfig},
};
use std::net::SocketAddr;
use std::time::Duration;

/// Builder for creating a SIP client with custom configuration
pub struct SipClientBuilder {
    config: SipClientConfig,
}

impl SipClientBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: SipClientConfig::default(),
        }
    }
    
    /// Set the SIP identity (required)
    pub fn sip_identity(mut self, identity: impl Into<String>) -> Self {
        self.config.sip_identity = identity.into();
        self
    }
    
    /// Set the SIP server address
    pub fn sip_server(mut self, server: impl Into<String>) -> Self {
        self.config.sip_server = Some(server.into());
        self
    }
    
    /// Set the local address to bind to
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
        self.config.local_address = addr;
        self
    }
    
    /// Set the user agent string
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.config.user_agent = ua.into();
        self
    }
    
    /// Configure audio settings
    pub fn audio(mut self, f: impl FnOnce(AudioConfigBuilder) -> AudioConfigBuilder) -> Self {
        let builder = f(AudioConfigBuilder::new(self.config.audio));
        self.config.audio = builder.build();
        self
    }
    
    /// Configure audio with a pre-built configuration
    pub fn audio_config(mut self, config: AudioConfig) -> Self {
        self.config.audio = config;
        self
    }
    
    /// Use default audio settings optimized for VoIP
    pub fn audio_defaults(self) -> Self {
        self.audio(|a| a
            .echo_cancellation(true)
            .noise_suppression(true)
            .auto_gain_control(true)
            .sample_rate(16000) // Wideband
            .channels(1)        // Mono
            .frame_duration_ms(20)
        )
    }
    
    /// Configure codec priorities
    pub fn codecs(mut self, priorities: Vec<CodecPriority>) -> Self {
        self.config.codecs.priorities = priorities;
        self
    }
    
    /// Configure codec settings
    pub fn codec_config(mut self, f: impl FnOnce(CodecConfigBuilder) -> CodecConfigBuilder) -> Self {
        let builder = f(CodecConfigBuilder::new(self.config.codecs));
        self.config.codecs = builder.build();
        self
    }
    
    /// Enable registration with the SIP server
    pub fn register(mut self, f: impl FnOnce(RegistrationConfigBuilder) -> RegistrationConfigBuilder) -> Self {
        let builder = f(RegistrationConfigBuilder::new());
        self.config.registration = Some(builder.build());
        self
    }
    
    /// Set call timeout
    pub fn call_timeout(mut self, timeout: Duration) -> Self {
        self.config.call_timeout = timeout;
        self
    }
    
    /// Enable automatic call recording
    pub fn auto_record(mut self, enable: bool) -> Self {
        self.config.auto_record = enable;
        self
    }
    
    /// Build the SIP client
    #[cfg(feature = "simple-api")]
    pub async fn build(self) -> SipClientResult<crate::simple::SipClient> {
        self.validate()?;
        crate::simple::SipClient::from_config(self.config).await
    }
    
    /// Build the advanced SIP client
    #[cfg(feature = "advanced-api")]
    pub async fn build_advanced(self) -> SipClientResult<crate::advanced::AdvancedSipClient> {
        self.validate()?;
        crate::advanced::AdvancedSipClient::from_config(self.config).await
    }
    
    /// Get the configuration without building
    pub fn into_config(self) -> SipClientResult<SipClientConfig> {
        self.validate()?;
        Ok(self.config)
    }
    
    /// Validate the configuration
    fn validate(&self) -> SipClientResult<()> {
        if self.config.sip_identity.is_empty() {
            return Err(SipClientError::config("SIP identity is required"));
        }
        
        if !self.config.sip_identity.starts_with("sip:") {
            return Err(SipClientError::config("SIP identity must start with 'sip:'"));
        }
        
        if self.config.codecs.priorities.is_empty() {
            return Err(SipClientError::config("At least one codec must be configured"));
        }
        
        Ok(())
    }
}

impl Default for SipClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for audio configuration
pub struct AudioConfigBuilder {
    config: AudioConfig,
}

impl AudioConfigBuilder {
    fn new(config: AudioConfig) -> Self {
        Self { config }
    }
    
    /// Set the input device
    pub fn input_device(mut self, device: impl Into<String>) -> Self {
        self.config.input_device = Some(device.into());
        self
    }
    
    /// Set the output device
    pub fn output_device(mut self, device: impl Into<String>) -> Self {
        self.config.output_device = Some(device.into());
        self
    }
    
    /// Enable/disable echo cancellation
    pub fn echo_cancellation(mut self, enable: bool) -> Self {
        self.config.echo_cancellation = enable;
        self
    }
    
    /// Enable/disable noise suppression
    pub fn noise_suppression(mut self, enable: bool) -> Self {
        self.config.noise_suppression = enable;
        self
    }
    
    /// Enable/disable automatic gain control
    pub fn auto_gain_control(mut self, enable: bool) -> Self {
        self.config.auto_gain_control = enable;
        self
    }
    
    /// Set the sample rate
    pub fn sample_rate(mut self, rate: u32) -> Self {
        self.config.sample_rate = rate;
        self
    }
    
    /// Set the number of channels
    pub fn channels(mut self, channels: u16) -> Self {
        self.config.channels = channels;
        self
    }
    
    /// Set the frame duration in milliseconds
    pub fn frame_duration_ms(mut self, ms: u32) -> Self {
        self.config.frame_duration_ms = ms;
        self
    }
    
    fn build(self) -> AudioConfig {
        self.config
    }
}

/// Builder for codec configuration
pub struct CodecConfigBuilder {
    config: CodecConfig,
}

impl CodecConfigBuilder {
    fn new(config: CodecConfig) -> Self {
        Self { config }
    }
    
    /// Set codec priorities
    pub fn priorities(mut self, priorities: Vec<CodecPriority>) -> Self {
        self.config.priorities = priorities;
        self
    }
    
    /// Add a codec with priority
    pub fn add_codec(mut self, name: impl Into<String>, priority: u8) -> Self {
        self.config.priorities.push(CodecPriority::new(name, priority));
        self
    }
    
    /// Allow dynamic codec switching during calls
    pub fn allow_dynamic_switching(mut self, allow: bool) -> Self {
        self.config.allow_dynamic_switching = allow;
        self
    }
    
    /// Set preferred packet time
    pub fn preferred_ptime(mut self, ms: u32) -> Self {
        self.config.preferred_ptime = Some(ms);
        self
    }
    
    /// Set maximum packet time
    pub fn max_ptime(mut self, ms: u32) -> Self {
        self.config.max_ptime = Some(ms);
        self
    }
    
    fn build(self) -> CodecConfig {
        self.config
    }
}

/// Builder for registration configuration
pub struct RegistrationConfigBuilder {
    expires: u32,
    username: Option<String>,
    password: Option<String>,
    realm: Option<String>,
}

impl RegistrationConfigBuilder {
    fn new() -> Self {
        Self {
            expires: 3600,
            username: None,
            password: None,
            realm: None,
        }
    }
    
    /// Set registration expiry time in seconds
    pub fn expires(mut self, seconds: u32) -> Self {
        self.expires = seconds;
        self
    }
    
    /// Set authentication credentials
    pub fn credentials(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }
    
    /// Set authentication realm
    pub fn realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = Some(realm.into());
        self
    }
    
    fn build(self) -> RegistrationConfig {
        RegistrationConfig {
            expires: self.expires,
            username: self.username,
            password: self.password,
            realm: self.realm,
        }
    }
}