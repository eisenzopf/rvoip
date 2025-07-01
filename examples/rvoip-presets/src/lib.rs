//! # RVOIP Presets - Pre-configured VoIP Patterns
//!
//! This crate provides pre-configured patterns and templates for common VoIP use cases,
//! making it easy to get started with RVOIP without needing to understand all the
//! underlying configuration details.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use rvoip_presets::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Enterprise PBX in one line
//!     let pbx = EnterprisePbx::new("corp.example.com")
//!         .with_user_capacity(1000)
//!         .with_encryption_required(true)
//!         .start().await?;
//!
//!     // Mobile VoIP app
//!     let app = MobileVoipApp::new("MyVoIPApp")
//!         .with_p2p_calling()
//!         .with_push_notifications()
//!         .launch().await?;
//!
//!     Ok(())
//! }
//! ```

use std::net::SocketAddr;
use std::time::Duration;
use std::collections::HashMap;

use tokio::sync::broadcast;
use tracing::{info, warn, error};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

// Re-export from rvoip-simple for convenience
pub use rvoip_simple::{
    SimpleVoipClient, SecurityConfig, MediaConfig, AudioCodec, VideoCodec, AudioQuality,
    SimpleVoipError, ClientState, CallState, CallEvent, ClientEvent
};

pub mod enterprise;
pub mod mobile;
pub mod webrtc;
pub mod cloud;
pub mod contact_center;
pub mod specialized;

pub use enterprise::*;
pub use mobile::*;
pub use webrtc::*;
pub use cloud::*;
pub use contact_center::*;
pub use specialized::*;

/// Common configuration patterns for VoIP deployments
#[derive(Debug, Clone)]
pub struct DeploymentConfig {
    /// Deployment name/identifier
    pub name: String,
    /// Target environment (development, staging, production)
    pub environment: Environment,
    /// Expected concurrent users/calls
    pub capacity: CapacityConfig,
    /// Security requirements
    pub security: SecurityProfile,
    /// Network configuration
    pub network: NetworkConfig,
    /// Feature set to enable
    pub features: FeatureSet,
}

/// Deployment environment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Staging,
    Production,
}

/// Capacity configuration for scaling
#[derive(Debug, Clone)]
pub struct CapacityConfig {
    /// Maximum concurrent users
    pub max_users: u32,
    /// Maximum concurrent calls
    pub max_calls: u32,
    /// Expected average call duration
    pub avg_call_duration: Duration,
    /// Peak usage multiplier
    pub peak_multiplier: f32,
}

/// Security profile for different deployment types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityProfile {
    /// Development/testing (minimal security)
    Development,
    /// Standard security for business use
    Standard,
    /// High security for financial/healthcare
    HighSecurity,
    /// Enterprise security with PKI
    Enterprise,
    /// Government/defense grade security
    Government,
}

/// Network configuration
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Primary listen address
    pub listen_address: SocketAddr,
    /// External/public address for NAT traversal
    pub external_address: Option<SocketAddr>,
    /// STUN/TURN server configuration
    pub ice_servers: Vec<IceServer>,
    /// Quality of Service settings
    pub qos: QosConfig,
}

/// ICE server configuration
#[derive(Debug, Clone)]
pub struct IceServer {
    /// Server URLs
    pub urls: Vec<String>,
    /// Authentication username
    pub username: Option<String>,
    /// Authentication credential
    pub credential: Option<String>,
}

/// Quality of Service configuration
#[derive(Debug, Clone)]
pub struct QosConfig {
    /// Enable DSCP marking
    pub dscp_marking: bool,
    /// Audio packet priority
    pub audio_priority: u8,
    /// Video packet priority
    pub video_priority: u8,
    /// Signaling packet priority
    pub signaling_priority: u8,
}

/// Feature set configuration
#[derive(Debug, Clone)]
pub struct FeatureSet {
    /// Voice calling
    pub voice_calling: bool,
    /// Video calling  
    pub video_calling: bool,
    /// Conference calling
    pub conferencing: bool,
    /// Call recording
    pub recording: bool,
    /// Call transfer/hold
    pub call_control: bool,
    /// Presence/status
    pub presence: bool,
    /// Instant messaging
    pub messaging: bool,
    /// File transfer
    pub file_transfer: bool,
    /// Screen sharing
    pub screen_sharing: bool,
    /// Mobile push notifications
    pub push_notifications: bool,
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            name: "RVOIP Deployment".to_string(),
            environment: Environment::Development,
            capacity: CapacityConfig::small(),
            security: SecurityProfile::Standard,
            network: NetworkConfig::default(),
            features: FeatureSet::basic(),
        }
    }
}

impl CapacityConfig {
    /// Small deployment (up to 100 users)
    pub fn small() -> Self {
        Self {
            max_users: 100,
            max_calls: 50,
            avg_call_duration: Duration::from_secs(300), // 5 minutes
            peak_multiplier: 2.0,
        }
    }

    /// Medium deployment (up to 1000 users)
    pub fn medium() -> Self {
        Self {
            max_users: 1000,
            max_calls: 500,
            avg_call_duration: Duration::from_secs(300),
            peak_multiplier: 2.5,
        }
    }

    /// Large deployment (up to 10000 users)
    pub fn large() -> Self {
        Self {
            max_users: 10000,
            max_calls: 5000,
            avg_call_duration: Duration::from_secs(300),
            peak_multiplier: 3.0,
        }
    }

    /// Enterprise deployment (10000+ users)
    pub fn enterprise() -> Self {
        Self {
            max_users: 50000,
            max_calls: 25000,
            avg_call_duration: Duration::from_secs(300),
            peak_multiplier: 4.0,
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_address: "0.0.0.0:5060".parse().unwrap(),
            external_address: None,
            ice_servers: vec![
                IceServer {
                    urls: vec!["stun:stun.l.google.com:19302".to_string()],
                    username: None,
                    credential: None,
                }
            ],
            qos: QosConfig::default(),
        }
    }
}

impl Default for QosConfig {
    fn default() -> Self {
        Self {
            dscp_marking: true,
            audio_priority: 46, // EF (Expedited Forwarding)
            video_priority: 34, // AF41 (Assured Forwarding)
            signaling_priority: 24, // CS3 (Class Selector)
        }
    }
}

impl FeatureSet {
    /// Basic feature set (voice calling only)
    pub fn basic() -> Self {
        Self {
            voice_calling: true,
            video_calling: false,
            conferencing: false,
            recording: false,
            call_control: true,
            presence: false,
            messaging: false,
            file_transfer: false,
            screen_sharing: false,
            push_notifications: false,
        }
    }

    /// Standard feature set (voice + video + basic features)
    pub fn standard() -> Self {
        Self {
            voice_calling: true,
            video_calling: true,
            conferencing: true,
            recording: false,
            call_control: true,
            presence: true,
            messaging: false,
            file_transfer: false,
            screen_sharing: false,
            push_notifications: false,
        }
    }

    /// Full feature set (everything enabled)
    pub fn full() -> Self {
        Self {
            voice_calling: true,
            video_calling: true,
            conferencing: true,
            recording: true,
            call_control: true,
            presence: true,
            messaging: true,
            file_transfer: true,
            screen_sharing: true,
            push_notifications: true,
        }
    }

    /// Mobile-optimized feature set
    pub fn mobile() -> Self {
        Self {
            voice_calling: true,
            video_calling: true,
            conferencing: false, // Reduced for mobile
            recording: false,
            call_control: true,
            presence: true,
            messaging: true,
            file_transfer: false,
            screen_sharing: false,
            push_notifications: true,
        }
    }
}

/// Security configuration helpers
impl SecurityConfig {
    /// Create configuration based on security profile
    pub fn from_profile(profile: SecurityProfile) -> Self {
        match profile {
            SecurityProfile::Development => Self::None,
            SecurityProfile::Standard => Self::Auto,
            SecurityProfile::HighSecurity => Self::DtlsSrtp,
            SecurityProfile::Enterprise => Self::enterprise_psk(vec![0u8; 32]), // Would use real key
            SecurityProfile::Government => Self::enterprise_pke(vec![], vec![]), // Would use real certs
        }
    }
}

/// Media configuration helpers
impl MediaConfig {
    /// Create configuration based on security profile
    pub fn from_security_profile(profile: SecurityProfile) -> Self {
        match profile {
            SecurityProfile::Development => Self::low_bandwidth(),
            SecurityProfile::Standard => Self::desktop(),
            SecurityProfile::HighSecurity => Self::high_quality(),
            SecurityProfile::Enterprise => Self::conferencing(),
            SecurityProfile::Government => Self::high_quality(),
        }
    }
}

/// Deployment builder for creating configurations
pub struct DeploymentBuilder {
    config: DeploymentConfig,
}

impl DeploymentBuilder {
    /// Create a new deployment builder
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            config: DeploymentConfig {
                name: name.into(),
                ..Default::default()
            }
        }
    }

    /// Set the deployment environment
    pub fn environment(mut self, env: Environment) -> Self {
        self.config.environment = env;
        self
    }

    /// Set the capacity configuration
    pub fn capacity(mut self, capacity: CapacityConfig) -> Self {
        self.config.capacity = capacity;
        self
    }

    /// Set the security profile
    pub fn security(mut self, security: SecurityProfile) -> Self {
        self.config.security = security;
        self
    }

    /// Set the network configuration
    pub fn network(mut self, network: NetworkConfig) -> Self {
        self.config.network = network;
        self
    }

    /// Set the feature set
    pub fn features(mut self, features: FeatureSet) -> Self {
        self.config.features = features;
        self
    }

    /// Build the deployment configuration
    pub fn build(self) -> DeploymentConfig {
        self.config
    }
}

/// Common preset configurations
pub struct Presets;

impl Presets {
    /// Small office deployment (10-50 users)
    pub fn small_office() -> DeploymentConfig {
        DeploymentBuilder::new("Small Office")
            .environment(Environment::Production)
            .capacity(CapacityConfig::small())
            .security(SecurityProfile::Standard)
            .features(FeatureSet::standard())
            .build()
    }

    /// Enterprise deployment (1000+ users)
    pub fn enterprise() -> DeploymentConfig {
        DeploymentBuilder::new("Enterprise")
            .environment(Environment::Production)
            .capacity(CapacityConfig::enterprise())
            .security(SecurityProfile::Enterprise)
            .features(FeatureSet::full())
            .build()
    }

    /// Mobile app deployment
    pub fn mobile_app() -> DeploymentConfig {
        DeploymentBuilder::new("Mobile App")
            .environment(Environment::Production)
            .capacity(CapacityConfig::large())
            .security(SecurityProfile::Standard)
            .features(FeatureSet::mobile())
            .build()
    }

    /// WebRTC platform deployment
    pub fn webrtc_platform() -> DeploymentConfig {
        DeploymentBuilder::new("WebRTC Platform")
            .environment(Environment::Production)
            .capacity(CapacityConfig::large())
            .security(SecurityProfile::Standard)
            .features(FeatureSet::full())
            .build()
    }

    /// Contact center deployment
    pub fn contact_center() -> DeploymentConfig {
        DeploymentBuilder::new("Contact Center")
            .environment(Environment::Production)
            .capacity(CapacityConfig::enterprise())
            .security(SecurityProfile::HighSecurity)
            .features(FeatureSet {
                recording: true, // Important for contact centers
                conferencing: true,
                ..FeatureSet::standard()
            })
            .build()
    }

    /// Healthcare deployment (HIPAA compliant)
    pub fn healthcare() -> DeploymentConfig {
        DeploymentBuilder::new("Healthcare")
            .environment(Environment::Production)
            .capacity(CapacityConfig::medium())
            .security(SecurityProfile::HighSecurity)
            .features(FeatureSet {
                recording: true, // For compliance
                file_transfer: false, // Security concern
                ..FeatureSet::standard()
            })
            .build()
    }

    /// Financial services deployment
    pub fn financial() -> DeploymentConfig {
        DeploymentBuilder::new("Financial Services")
            .environment(Environment::Production)
            .capacity(CapacityConfig::large())
            .security(SecurityProfile::Government)
            .features(FeatureSet {
                recording: true, // Regulatory requirement
                ..FeatureSet::standard()
            })
            .build()
    }

    /// Development/testing deployment
    pub fn development() -> DeploymentConfig {
        DeploymentBuilder::new("Development")
            .environment(Environment::Development)
            .capacity(CapacityConfig::small())
            .security(SecurityProfile::Development)
            .features(FeatureSet::full()) // Enable all features for testing
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_creation() {
        let small_office = Presets::small_office();
        assert_eq!(small_office.name, "Small Office");
        assert_eq!(small_office.environment, Environment::Production);
        assert_eq!(small_office.security, SecurityProfile::Standard);

        let enterprise = Presets::enterprise();
        assert_eq!(enterprise.security, SecurityProfile::Enterprise);
        assert!(enterprise.features.recording);
        assert!(enterprise.features.conferencing);
    }

    #[test]
    fn test_capacity_configs() {
        let small = CapacityConfig::small();
        assert_eq!(small.max_users, 100);
        assert_eq!(small.max_calls, 50);

        let enterprise = CapacityConfig::enterprise();
        assert_eq!(enterprise.max_users, 50000);
        assert!(enterprise.peak_multiplier > small.peak_multiplier);
    }

    #[test]
    fn test_feature_sets() {
        let basic = FeatureSet::basic();
        assert!(basic.voice_calling);
        assert!(!basic.video_calling);
        assert!(!basic.recording);

        let full = FeatureSet::full();
        assert!(full.voice_calling);
        assert!(full.video_calling);
        assert!(full.recording);
        assert!(full.screen_sharing);

        let mobile = FeatureSet::mobile();
        assert!(mobile.push_notifications);
        assert!(!mobile.screen_sharing); // Typically not on mobile
    }

    #[test]
    fn test_deployment_builder() {
        let config = DeploymentBuilder::new("Test Deployment")
            .environment(Environment::Staging)
            .capacity(CapacityConfig::medium())
            .security(SecurityProfile::HighSecurity)
            .build();

        assert_eq!(config.name, "Test Deployment");
        assert_eq!(config.environment, Environment::Staging);
        assert_eq!(config.security, SecurityProfile::HighSecurity);
    }

    #[test]
    fn test_specialized_presets() {
        let healthcare = Presets::healthcare();
        assert_eq!(healthcare.security, SecurityProfile::HighSecurity);
        assert!(healthcare.features.recording); // Compliance requirement

        let financial = Presets::financial();
        assert_eq!(financial.security, SecurityProfile::Government);
        assert!(financial.features.recording); // Regulatory requirement
    }
} 