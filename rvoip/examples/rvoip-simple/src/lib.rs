//! # RVOIP Simple - Developer-Friendly VoIP APIs
//!
//! This crate provides simple, high-level APIs for building VoIP applications
//! without requiring deep knowledge of SIP, RTP, or other protocols.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use rvoip_simple::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a simple VoIP client
//!     let client = SimpleVoipClient::new("user@domain.com", "password")
//!         .with_display_name("John Doe")
//!         .with_auto_answer(false)
//!         .connect().await?;
//!
//!     // Make a call
//!     let call = client.make_call("friend@domain.com").await?;
//!     
//!     // Handle call events
//!     while let Some(event) = call.next_event().await {
//!         match event {
//!             CallEvent::Answered => println!("Call answered!"),
//!             CallEvent::Ended => break,
//!             _ => {}
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn, error, debug};
use thiserror::Error;
use serde::{Serialize, Deserialize};

pub mod client;
pub mod call;
pub mod config;
pub mod error;
pub mod events;

pub use client::*;
pub use call::*;
pub use config::*;
pub use error::*;
pub use events::*;

/// Simple VoIP client for making and receiving calls
#[derive(Debug)]
pub struct SimpleVoipClient {
    config: ClientConfig,
    state: ClientState,
    event_tx: broadcast::Sender<ClientEvent>,
    call_manager: CallManager,
}

/// Configuration for a simple VoIP client
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// SIP URI (e.g., "user@domain.com")
    pub sip_uri: String,
    /// Authentication password
    pub password: String,
    /// Display name for outgoing calls
    pub display_name: Option<String>,
    /// SIP registrar server (auto-detected if None)
    pub registrar: Option<String>,
    /// Local bind address (auto-detected if None)
    pub local_address: Option<SocketAddr>,
    /// Security configuration
    pub security: SecurityConfig,
    /// Media configuration
    pub media: MediaConfig,
    /// Auto-answer incoming calls
    pub auto_answer: bool,
    /// Call timeout duration
    pub call_timeout: Duration,
}

/// Security configuration options
#[derive(Debug, Clone)]
pub enum SecurityConfig {
    /// No encryption (not recommended for production)
    None,
    /// Automatic security (DTLS-SRTP for WebRTC, SDES for SIP)
    Auto,
    /// Force DTLS-SRTP (WebRTC-compatible)
    DtlsSrtp,
    /// Force ZRTP for peer-to-peer calling
    Zrtp,
    /// Enterprise MIKEY with pre-shared key
    MikeyPsk { key: Vec<u8> },
    /// Enterprise MIKEY with certificates
    MikeyPke { 
        certificate: Vec<u8>, 
        private_key: Vec<u8>,
        peer_certificate: Option<Vec<u8>>,
    },
}

/// Media configuration options
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// Preferred audio codecs (in order of preference)
    pub audio_codecs: Vec<AudioCodec>,
    /// Preferred video codecs (in order of preference)  
    pub video_codecs: Vec<VideoCodec>,
    /// Enable echo cancellation
    pub echo_cancellation: bool,
    /// Enable noise suppression
    pub noise_suppression: bool,
    /// Enable automatic gain control
    pub auto_gain_control: bool,
    /// Audio quality preference
    pub audio_quality: AudioQuality,
}

/// Supported audio codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodec {
    Opus,
    G722,
    G711u, // μ-law
    G711a, // A-law
}

/// Supported video codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    VP8,
    VP9,
    AV1,
}

/// Audio quality preferences
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioQuality {
    /// Optimize for bandwidth (lower quality, less data)
    Bandwidth,
    /// Balanced quality and bandwidth
    Balanced,
    /// Optimize for quality (higher quality, more data)
    Quality,
}

/// Client state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientState {
    Disconnected,
    Connecting,
    Registering,
    Connected,
    Error(String),
}

/// Client events
#[derive(Debug, Clone)]
pub enum ClientEvent {
    StateChanged(ClientState),
    IncomingCall(IncomingCall),
    RegistrationSuccess,
    RegistrationFailed(String),
    NetworkError(String),
}

/// Represents an incoming call
#[derive(Debug, Clone)]
pub struct IncomingCall {
    pub call_id: String,
    pub caller: String,
    pub caller_display_name: Option<String>,
    pub has_video: bool,
    pub timestamp: std::time::SystemTime,
}

/// Active call management
#[derive(Debug)]
pub struct CallManager {
    active_calls: HashMap<String, ActiveCall>,
    call_events: mpsc::Sender<CallEvent>,
}

/// Represents an active call
#[derive(Debug)]
pub struct ActiveCall {
    pub id: String,
    pub remote_party: String,
    pub state: CallState,
    pub direction: CallDirection,
    pub start_time: Option<std::time::SystemTime>,
    pub media_stats: MediaStats,
}

/// Call state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallState {
    Initiating,
    Ringing,
    Answered,
    OnHold,
    Ended,
    Failed(String),
}

/// Call direction
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallDirection {
    Outgoing,
    Incoming,
}

/// Call events
#[derive(Debug, Clone)]
pub enum CallEvent {
    StateChanged(String, CallState), // call_id, new_state
    MediaConnected(String),          // call_id
    MediaDisconnected(String),       // call_id
    QualityChanged(String, CallQuality), // call_id, quality
    DtmfReceived(String, char),      // call_id, digit
    Answered,
    Ended,
}

/// Call quality indicators
#[derive(Debug, Clone)]
pub struct CallQuality {
    pub mos_score: f32,      // Mean Opinion Score (1.0-5.0)
    pub packet_loss: f32,    // Percentage (0.0-100.0)
    pub jitter: Duration,    // Network jitter
    pub rtt: Duration,       // Round-trip time
}

/// Media statistics for active calls
#[derive(Debug, Clone, Default)]
pub struct MediaStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_lost: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub current_bitrate: u32, // bits per second
    pub quality: Option<CallQuality>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            sip_uri: String::new(),
            password: String::new(),
            display_name: None,
            registrar: None,
            local_address: None,
            security: SecurityConfig::Auto,
            media: MediaConfig::default(),
            auto_answer: false,
            call_timeout: Duration::from_secs(30),
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            audio_codecs: vec![AudioCodec::Opus, AudioCodec::G722, AudioCodec::G711u],
            video_codecs: vec![VideoCodec::H264, VideoCodec::VP8],
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            audio_quality: AudioQuality::Balanced,
        }
    }
}

impl SimpleVoipClient {
    /// Create a new VoIP client with basic credentials
    pub fn new(sip_uri: impl Into<String>, password: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(sip_uri.into(), password.into())
    }

    /// Subscribe to client events
    pub fn subscribe_events(&self) -> broadcast::Receiver<ClientEvent> {
        self.event_tx.subscribe()
    }

    /// Get current client state
    pub fn state(&self) -> &ClientState {
        &self.state
    }

    /// Make an outgoing call
    pub async fn make_call(&self, target: impl Into<String>) -> Result<Call, SimpleVoipError> {
        let target = target.into();
        info!("Making call to: {}", target);
        
        // TODO: Implement actual call logic using sip-client and rtp-core
        // For now, return a placeholder
        Ok(Call::new_outgoing("call-123".to_string(), target))
    }

    /// Answer an incoming call
    pub async fn answer_call(&self, call_id: impl Into<String>) -> Result<Call, SimpleVoipError> {
        let call_id = call_id.into();
        info!("Answering call: {}", call_id);
        
        // TODO: Implement actual answer logic
        Ok(Call::new_incoming(call_id, "caller@domain.com".to_string()))
    }

    /// Reject an incoming call
    pub async fn reject_call(&self, call_id: impl Into<String>) -> Result<(), SimpleVoipError> {
        let call_id = call_id.into();
        info!("Rejecting call: {}", call_id);
        
        // TODO: Implement actual reject logic
        Ok(())
    }

    /// Get list of active calls
    pub fn active_calls(&self) -> Vec<&ActiveCall> {
        self.call_manager.active_calls.values().collect()
    }

    /// Disconnect the client
    pub async fn disconnect(&mut self) -> Result<(), SimpleVoipError> {
        info!("Disconnecting VoIP client");
        self.state = ClientState::Disconnected;
        Ok(())
    }
}

/// Builder for configuring a VoIP client
pub struct ClientBuilder {
    config: ClientConfig,
}

impl ClientBuilder {
    fn new(sip_uri: String, password: String) -> Self {
        Self {
            config: ClientConfig {
                sip_uri,
                password,
                ..Default::default()
            }
        }
    }

    /// Set display name for outgoing calls
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.config.display_name = Some(name.into());
        self
    }

    /// Set SIP registrar server
    pub fn with_registrar(mut self, registrar: impl Into<String>) -> Self {
        self.config.registrar = Some(registrar.into());
        self
    }

    /// Set local bind address
    pub fn with_local_address(mut self, addr: SocketAddr) -> Self {
        self.config.local_address = Some(addr);
        self
    }

    /// Configure security settings
    pub fn with_security(mut self, security: SecurityConfig) -> Self {
        self.config.security = security;
        self
    }

    /// Configure media settings
    pub fn with_media(mut self, media: MediaConfig) -> Self {
        self.config.media = media;
        self
    }

    /// Enable auto-answer for incoming calls
    pub fn with_auto_answer(mut self, auto_answer: bool) -> Self {
        self.config.auto_answer = auto_answer;
        self
    }

    /// Set call timeout duration
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.config.call_timeout = timeout;
        self
    }

    /// Connect the client and register with the SIP server
    pub async fn connect(self) -> Result<SimpleVoipClient, SimpleVoipError> {
        info!("Connecting VoIP client for: {}", self.config.sip_uri);
        
        let (event_tx, _) = broadcast::channel(1000);
        let (call_event_tx, _call_event_rx) = mpsc::channel(100);
        
        let client = SimpleVoipClient {
            config: self.config,
            state: ClientState::Connecting,
            event_tx,
            call_manager: CallManager {
                active_calls: HashMap::new(),
                call_events: call_event_tx,
            },
        };

        // TODO: Implement actual SIP registration using sip-client
        // For now, simulate successful connection
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        Ok(client)
    }
}

/// Convenience functions for common configurations
impl SimpleVoipClient {
    /// Create a client optimized for mobile use
    pub fn mobile(sip_uri: impl Into<String>, password: impl Into<String>) -> ClientBuilder {
        Self::new(sip_uri, password)
            .with_security(SecurityConfig::Auto)
            .with_media(MediaConfig {
                audio_quality: AudioQuality::Bandwidth,
                echo_cancellation: true,
                noise_suppression: true,
                auto_gain_control: true,
                ..Default::default()
            })
    }

    /// Create a client optimized for desktop use
    pub fn desktop(sip_uri: impl Into<String>, password: impl Into<String>) -> ClientBuilder {
        Self::new(sip_uri, password)
            .with_security(SecurityConfig::Auto)
            .with_media(MediaConfig {
                audio_quality: AudioQuality::Quality,
                video_codecs: vec![VideoCodec::H264, VideoCodec::VP8, VideoCodec::VP9],
                ..Default::default()
            })
    }

    /// Create a client for peer-to-peer calling (no server required)
    pub fn p2p() -> ClientBuilder {
        Self::new("anonymous@p2p.local", "")
            .with_security(SecurityConfig::Zrtp)
            .with_media(MediaConfig {
                audio_quality: AudioQuality::Quality,
                ..Default::default()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert_eq!(config.auto_answer, false);
        assert_eq!(config.call_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_media_config_default() {
        let config = MediaConfig::default();
        assert!(config.echo_cancellation);
        assert_eq!(config.audio_quality, AudioQuality::Balanced);
        assert!(config.audio_codecs.contains(&AudioCodec::Opus));
    }

    #[tokio::test]
    async fn test_client_builder() {
        let builder = SimpleVoipClient::new("test@example.com", "password")
            .with_display_name("Test User")
            .with_auto_answer(true);
        
        assert_eq!(builder.config.sip_uri, "test@example.com");
        assert_eq!(builder.config.display_name, Some("Test User".to_string()));
        assert_eq!(builder.config.auto_answer, true);
    }

    #[test]
    fn test_convenience_constructors() {
        let mobile = SimpleVoipClient::mobile("user@domain.com", "pass");
        assert!(matches!(mobile.config.media.audio_quality, AudioQuality::Bandwidth));

        let desktop = SimpleVoipClient::desktop("user@domain.com", "pass");
        assert!(matches!(desktop.config.media.audio_quality, AudioQuality::Quality));

        let p2p = SimpleVoipClient::p2p();
        assert!(matches!(p2p.config.security, SecurityConfig::Zrtp));
    }
} 