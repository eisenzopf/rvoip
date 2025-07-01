//! # RVOIP Builder - Flexible VoIP Composition Patterns
//!
//! This crate provides maximum flexibility for building custom VoIP solutions
//! by composing individual RVOIP components in sophisticated ways.
//!
//! ## Architecture Levels
//!
//! - **Simple**: Use `rvoip-simple` for basic applications
//! - **Presets**: Use `rvoip-presets` for common patterns  
//! - **Builder**: Use `rvoip-builder` for complete customization
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use rvoip_builder::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Build a custom VoIP platform
//!     let platform = VoipPlatform::new("custom-voip")
//!         .with_sip_stack(SipStackConfig::custom())
//!         .with_rtp_engine(RtpEngineConfig::secure())
//!         .with_call_engine(CallEngineConfig::enterprise())
//!         .with_api_server(ApiServerConfig::rest_and_websocket())
//!         .build().await?;
//!
//!     platform.start().await?;
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{info, warn, error, debug};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

// Re-export convenience types
pub use rvoip_simple::{SimpleVoipError, SecurityConfig, MediaConfig};
pub use rvoip_presets::{DeploymentConfig, Environment, SecurityProfile, FeatureSet};

pub mod sip_stack;
pub mod rtp_engine;
pub mod call_engine;
pub mod api_server;
pub mod composition;
pub mod config;
pub mod runtime;

pub use sip_stack::*;
pub use rtp_engine::*;
pub use call_engine::*;
pub use api_server::*;
pub use composition::*;
pub use config::*;
pub use runtime::*;

/// Main VoIP platform that composes all components
#[derive(Debug)]
pub struct VoipPlatform {
    /// Platform identifier
    pub id: String,
    /// Platform configuration
    pub config: PlatformConfig,
    /// Component registry
    pub components: ComponentRegistry,
    /// Runtime state
    pub runtime: Arc<RwLock<RuntimeState>>,
    /// Event bus for inter-component communication
    pub event_bus: EventBus,
}

/// Platform configuration combining all component configs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    /// Platform metadata
    pub metadata: PlatformMetadata,
    /// SIP stack configuration
    pub sip_stack: Option<SipStackConfig>,
    /// RTP engine configuration
    pub rtp_engine: Option<RtpEngineConfig>,
    /// Call engine configuration
    pub call_engine: Option<CallEngineConfig>,
    /// API server configuration
    pub api_server: Option<ApiServerConfig>,
    /// ICE configuration
    pub ice_config: Option<IceConfig>,
    /// Media configuration
    pub media_config: Option<MediaConfig>,
    /// Security configuration
    pub security_config: Option<SecurityConfig>,
}

/// Platform metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformMetadata {
    /// Platform name
    pub name: String,
    /// Version information
    pub version: String,
    /// Deployment environment
    pub environment: Environment,
    /// Creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Custom tags/labels
    pub tags: HashMap<String, String>,
}

/// Component registry for managing active components
#[derive(Debug)]
pub struct ComponentRegistry {
    /// Active SIP stack instances
    pub sip_stacks: HashMap<String, Box<dyn SipStackComponent>>,
    /// Active RTP engines
    pub rtp_engines: HashMap<String, Box<dyn RtpEngineComponent>>,
    /// Active call engines
    pub call_engines: HashMap<String, Box<dyn CallEngineComponent>>,
    /// Active API servers
    pub api_servers: HashMap<String, Box<dyn ApiServerComponent>>,
}

/// Runtime state for the platform
#[derive(Debug, Default)]
pub struct RuntimeState {
    /// Platform status
    pub status: PlatformStatus,
    /// Component health status
    pub component_health: HashMap<String, ComponentHealth>,
    /// Runtime metrics
    pub metrics: RuntimeMetrics,
    /// Active connections
    pub connections: u64,
    /// Total processed calls
    pub total_calls: u64,
}

/// Platform status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformStatus {
    Initializing,
    Starting,
    Running,
    Stopping,
    Stopped,
    Error(String),
}

impl Default for PlatformStatus {
    fn default() -> Self {
        Self::Initializing
    }
}

/// Component health status
#[derive(Debug, Clone)]
pub struct ComponentHealth {
    /// Component status
    pub status: ComponentStatus,
    /// Last health check
    pub last_check: chrono::DateTime<chrono::Utc>,
    /// Health score (0.0-1.0)
    pub health_score: f64,
    /// Status message
    pub message: Option<String>,
}

/// Component status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// Runtime metrics
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    /// CPU usage percentage
    pub cpu_usage: f64,
    /// Memory usage in bytes
    pub memory_usage: u64,
    /// Network bytes received
    pub network_rx_bytes: u64,
    /// Network bytes transmitted
    pub network_tx_bytes: u64,
    /// Current active sessions
    pub active_sessions: u64,
    /// Calls per second
    pub calls_per_second: f64,
}

/// Event bus for inter-component communication
#[derive(Debug, Clone)]
pub struct EventBus {
    /// Event sender
    pub sender: broadcast::Sender<PlatformEvent>,
}

/// Platform events
#[derive(Debug, Clone)]
pub enum PlatformEvent {
    /// Component status change
    ComponentStatusChanged(String, ComponentStatus),
    /// New connection established
    ConnectionEstablished(String),
    /// Connection terminated
    ConnectionTerminated(String),
    /// Call started
    CallStarted(String, String), // call_id, participant_info
    /// Call ended
    CallEnded(String, Duration), // call_id, duration
    /// Security event
    SecurityEvent(SecurityEvent),
    /// Performance metric update
    MetricUpdate(String, f64), // metric_name, value
}

/// Security events
#[derive(Debug, Clone)]
pub enum SecurityEvent {
    AuthenticationFailed(String),
    EncryptionNegotiated(String, String), // connection_id, algorithm
    CertificateExpiring(String, Duration), // cert_id, days_until_expiry
    SecurityViolation(String), // description
}

impl VoipPlatform {
    /// Create a new VoIP platform builder
    pub fn new(name: impl Into<String>) -> VoipPlatformBuilder {
        VoipPlatformBuilder::new(name.into())
    }

    /// Start the platform and all components
    pub async fn start(&self) -> Result<(), VoipBuilderError> {
        info!("Starting VoIP platform: {}", self.id);
        
        // Update runtime state
        {
            let mut runtime = self.runtime.write().await;
            runtime.status = PlatformStatus::Starting;
        }

        // TODO: Start all components in dependency order
        // 1. Infrastructure components (event bus, metrics)
        // 2. Core components (SIP stack, RTP engine) 
        // 3. Application components (call engine, API server)

        // Simulate startup
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Update status to running
        {
            let mut runtime = self.runtime.write().await;
            runtime.status = PlatformStatus::Running;
        }

        // Emit startup event
        let _ = self.event_bus.sender.send(PlatformEvent::ComponentStatusChanged(
            self.id.clone(),
            ComponentStatus::Healthy,
        ));

        info!("✅ VoIP platform started successfully");
        Ok(())
    }

    /// Stop the platform gracefully
    pub async fn stop(&self) -> Result<(), VoipBuilderError> {
        info!("Stopping VoIP platform: {}", self.id);
        
        {
            let mut runtime = self.runtime.write().await;
            runtime.status = PlatformStatus::Stopping;
        }

        // TODO: Stop components in reverse dependency order
        // 1. Application components
        // 2. Core components
        // 3. Infrastructure components

        {
            let mut runtime = self.runtime.write().await;
            runtime.status = PlatformStatus::Stopped;
        }

        info!("✅ VoIP platform stopped");
        Ok(())
    }

    /// Get platform status
    pub async fn status(&self) -> PlatformStatus {
        self.runtime.read().await.status.clone()
    }

    /// Get runtime metrics
    pub async fn metrics(&self) -> RuntimeMetrics {
        self.runtime.read().await.metrics.clone()
    }

    /// Subscribe to platform events
    pub fn subscribe_events(&self) -> broadcast::Receiver<PlatformEvent> {
        self.event_bus.sender.subscribe()
    }

    /// Health check for the platform
    pub async fn health_check(&self) -> ComponentHealth {
        // TODO: Aggregate health from all components
        ComponentHealth {
            status: ComponentStatus::Healthy,
            last_check: chrono::Utc::now(),
            health_score: 1.0,
            message: Some("All components healthy".to_string()),
        }
    }
}

/// Builder for VoIP platform
pub struct VoipPlatformBuilder {
    name: String,
    sip_stack: Option<SipStackConfig>,
    rtp_engine: Option<RtpEngineConfig>,
    call_engine: Option<CallEngineConfig>,
    api_server: Option<ApiServerConfig>,
    ice_config: Option<IceConfig>,
    environment: Environment,
    tags: HashMap<String, String>,
}

impl VoipPlatformBuilder {
    fn new(name: String) -> Self {
        Self {
            name,
            sip_stack: None,
            rtp_engine: None,
            call_engine: None,
            api_server: None,
            ice_config: None,
            environment: Environment::Development,
            tags: HashMap::new(),
        }
    }

    /// Configure SIP stack
    pub fn with_sip_stack(mut self, config: SipStackConfig) -> Self {
        self.sip_stack = Some(config);
        self
    }

    /// Configure RTP engine
    pub fn with_rtp_engine(mut self, config: RtpEngineConfig) -> Self {
        self.rtp_engine = Some(config);
        self
    }

    /// Configure call engine
    pub fn with_call_engine(mut self, config: CallEngineConfig) -> Self {
        self.call_engine = Some(config);
        self
    }

    /// Configure API server
    pub fn with_api_server(mut self, config: ApiServerConfig) -> Self {
        self.api_server = Some(config);
        self
    }

    /// Configure ICE
    pub fn with_ice_config(mut self, config: IceConfig) -> Self {
        self.ice_config = Some(config);
        self
    }

    /// Set deployment environment
    pub fn environment(mut self, env: Environment) -> Self {
        self.environment = env;
        self
    }

    /// Add custom tags
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Build the VoIP platform
    pub async fn build(self) -> Result<VoipPlatform, VoipBuilderError> {
        let id = Uuid::new_v4().to_string();
        
        let metadata = PlatformMetadata {
            name: self.name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            environment: self.environment,
            created_at: chrono::Utc::now(),
            tags: self.tags,
        };

        let config = PlatformConfig {
            metadata,
            sip_stack: self.sip_stack,
            rtp_engine: self.rtp_engine,
            call_engine: self.call_engine,
            api_server: self.api_server,
            ice_config: self.ice_config,
            media_config: None,
            security_config: None,
        };

        let (event_sender, _) = broadcast::channel(10000);
        let event_bus = EventBus { sender: event_sender };

        let components = ComponentRegistry {
            sip_stacks: HashMap::new(),
            rtp_engines: HashMap::new(),
            call_engines: HashMap::new(),
            api_servers: HashMap::new(),
        };

        let runtime = Arc::new(RwLock::new(RuntimeState::default()));

        Ok(VoipPlatform {
            id,
            config,
            components,
            runtime,
            event_bus,
        })
    }
}

/// Error types for VoIP builder
#[derive(thiserror::Error, Debug)]
pub enum VoipBuilderError {
    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Component error: {0}")]
    Component(String),

    #[error("Runtime error: {0}")]
    Runtime(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Security error: {0}")]
    Security(String),

    #[error("Simple VoIP error: {0}")]
    SimpleVoip(#[from] SimpleVoipError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl EventBus {
    /// Create a new event bus
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(10000);
        Self { sender }
    }

    /// Emit an event
    pub fn emit(&self, event: PlatformEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<PlatformEvent> {
        self.sender.subscribe()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self {
            sip_stacks: HashMap::new(),
            rtp_engines: HashMap::new(),
            call_engines: HashMap::new(),
            api_servers: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_platform_creation() {
        let platform = VoipPlatform::new("test-platform")
            .environment(Environment::Development)
            .with_tag("test", "true")
            .build().await.unwrap();

        assert_eq!(platform.config.metadata.name, "test-platform");
        assert_eq!(platform.config.metadata.environment, Environment::Development);
        assert_eq!(platform.config.metadata.tags.get("test"), Some(&"true".to_string()));
    }

    #[tokio::test]
    async fn test_platform_lifecycle() {
        let platform = VoipPlatform::new("lifecycle-test")
            .build().await.unwrap();

        // Initial state
        assert_eq!(platform.status().await, PlatformStatus::Initializing);

        // Start platform
        platform.start().await.unwrap();
        assert_eq!(platform.status().await, PlatformStatus::Running);

        // Stop platform
        platform.stop().await.unwrap();
        assert_eq!(platform.status().await, PlatformStatus::Stopped);
    }

    #[test]
    fn test_event_bus() {
        let event_bus = EventBus::new();
        let mut receiver = event_bus.subscribe();

        // Emit event
        event_bus.emit(PlatformEvent::ComponentStatusChanged(
            "test".to_string(),
            ComponentStatus::Healthy,
        ));

        // Event should be receivable
        let event = receiver.try_recv().unwrap();
        if let PlatformEvent::ComponentStatusChanged(id, status) = event {
            assert_eq!(id, "test");
            assert_eq!(status, ComponentStatus::Healthy);
        } else {
            panic!("Unexpected event type");
        }
    }
} 