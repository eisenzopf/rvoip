//! Session Manager Builder API
//!
//! Provides a fluent builder interface for creating and configuring
//! the SessionManager with all necessary components.
//! 
//! # Overview
//! 
//! The `SessionManagerBuilder` provides a convenient way to configure and create
//! a `SessionCoordinator` with all the necessary components. It uses the builder
//! pattern to allow flexible configuration while ensuring all required settings
//! are properly initialized.
//! 
//! # Basic Usage
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::sync::Arc;
//! 
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Simple configuration
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .build()
//!         .await?;
//!     
//!     // Start using the coordinator
//!     SessionControl::start(&coordinator).await?;
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Advanced Configuration
//! 
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder, SessionCoordinator};
//! use rvoip_session_core::examples::RoutingHandler;
//! use std::sync::Arc;
//! 
//! async fn create_pbx_system() -> Result<Arc<SessionCoordinator>, Box<dyn std::error::Error>> {
//!     // Create a routing handler
//!     let mut router = RoutingHandler::new();
//!     router.add_route("sip:support@", "sip:queue@support.local");
//!     router.add_route("sip:sales@", "sip:queue@sales.local");
//!     
//!     // Configure the coordinator
//!     let coordinator = SessionManagerBuilder::new()
//!         // Network settings
//!         .with_sip_port(5060)
//!         .with_local_address("sip:pbx@company.com:5060")
//!         
//!         // Media settings
//!         .with_media_ports(10000, 20000)  // RTP port range
//!         
//!         // NAT traversal (if needed)
//!         .with_stun("stun.l.google.com:19302")
//!         
//!         // Call handling
//!         .with_handler(Arc::new(router))
//!         
//!         .build()
//!         .await?;
//!     
//!     Ok(coordinator)
//! }
//! ```
//! 
//! # Configuration Examples
//! 
//! ## Softphone Configuration
//! 
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder};
//! use rvoip_session_core::examples::AutoAnswerHandler;
//! use std::sync::Arc;
//! 
//! async fn setup_softphone() -> Result<(), Box<dyn std::error::Error>> {
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_local_address("sip:john@192.168.1.100:5060")
//!         .with_handler(Arc::new(AutoAnswerHandler))
//!         .build()
//!         .await?;
//!     Ok(())
//! }
//! ```
//! 
//! ## Call Center Configuration
//! 
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder};
//! use rvoip_session_core::api::handlers::{QueueHandler, CompositeHandler, RoutingHandler};
//! use std::sync::Arc;
//! 
//! async fn setup_call_center() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create queue handler
//!     let queue = Arc::new(QueueHandler::new(100));
//!     
//!     // Create composite handler with multiple handlers
//!     let composite = CompositeHandler::new()
//!         .add_handler(queue.clone())
//!         .add_handler(Arc::new(RoutingHandler::default()));
//!     
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_local_address("sip:callcenter@company.com")
//!         .with_media_ports(30000, 40000)  // Larger range for many calls
//!         .with_handler(Arc::new(composite))
//!         .build()
//!         .await?;
//!     Ok(())
//! }
//! ```
//! 
//! ## Behind NAT Configuration
//! 
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder};
//! 
//! async fn setup_nat_config() -> Result<(), Box<dyn std::error::Error>> {
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_local_address("sip:user@publicdomain.com")
//!         .with_stun("stun.stunprotocol.org:3478")  // Enable STUN
//!         .with_media_ports(50000, 51000)  // Specific port range
//!         .build()
//!         .await?;
//!     Ok(())
//! }
//! ```
//! 
//! # Error Handling
//! 
//! The builder's `build()` method can fail if:
//! - Network ports are already in use
//! - Invalid configuration values
//! - System resource limitations
//! 
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder};
//! 
//! async fn handle_builder_errors() {
//!     match SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .build()
//!         .await 
//!     {
//!         Ok(coordinator) => {
//!             println!("Coordinator created successfully");
//!         }
//!         Err(e) => {
//!             eprintln!("Failed to create coordinator: {}", e);
//!             // Handle specific error types
//!             if e.to_string().contains("Address already in use") {
//!                 eprintln!("Port 5060 is already taken, try another port");
//!             }
//!         }
//!     }
//! }
//! ```

use std::sync::Arc;
use std::time::Duration;
use crate::api::handlers::CallHandler;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

// Re-export MediaConfig from the media module
pub use crate::media::types::MediaConfig;

/// Configuration for the SessionManager
#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    /// SIP listening port
    pub sip_port: u16,
    
    /// Local SIP address (e.g., "user@domain")
    pub local_address: String,
    
    /// Local bind address for media (RTP/RTCP)
    pub local_bind_addr: std::net::SocketAddr,
    
    /// Media port range start
    pub media_port_start: u16,
    
    /// Media port range end
    pub media_port_end: u16,
    
    /// Enable STUN for NAT traversal
    pub enable_stun: bool,
    
    /// STUN server address
    pub stun_server: Option<String>,
    
    /// Enable SIP client features (REGISTER, MESSAGE, etc.)
    pub enable_sip_client: bool,
    
    /// Media configuration preferences
    pub media_config: MediaConfig,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            sip_port: 5060,
            local_address: "sip:user@localhost".to_string(),
            local_bind_addr: "0.0.0.0:5060".parse().unwrap(), // Bind to all interfaces with standard SIP port
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: false,
            media_config: MediaConfig::default(),
        }
    }
}

/// Builder for creating a configured SessionManager
/// 
/// This builder ensures all components are properly configured before
/// creating the SessionCoordinator. It provides sensible defaults while
/// allowing customization of all aspects.
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::{SessionManagerBuilder, CallHandler, CallDecision, IncomingCall, CallSession};
/// use std::sync::Arc;
/// 
/// #[derive(Debug)]
/// struct MyCallHandler;
/// 
/// #[async_trait::async_trait]
/// impl CallHandler for MyCallHandler {
///     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
///         CallDecision::Accept(None)
///     }
///     async fn on_call_ended(&self, call: CallSession, reason: &str) {}
/// }
/// 
/// async fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let coordinator = SessionManagerBuilder::new()
///         .with_sip_port(5060)
///         .with_local_address("sip:alice@example.com")
///         .with_handler(Arc::new(MyCallHandler))
///         .build()
///         .await?;
///     Ok(())
/// }
/// ```
pub struct SessionManagerBuilder {
    config: SessionManagerConfig,
    handler: Option<Arc<dyn CallHandler>>,
    transaction_manager: Option<Arc<rvoip_transaction_core::TransactionManager>>,
}

impl SessionManagerBuilder {
    /// Create a new builder with default configuration
    /// 
    /// Default values:
    /// - SIP port: 5060
    /// - Local address: "sip:user@localhost"
    /// - Media ports: 10000-20000
    /// - STUN: disabled
    /// - Handler: None
    pub fn new() -> Self {
        Self {
            config: SessionManagerConfig::default(),
            handler: None,
            transaction_manager: None,
        }
    }
    
    /// Set the SIP listening port
    /// 
    /// # Arguments
    /// * `port` - The UDP port to listen on for SIP messages
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// // Use non-standard port to avoid conflicts
    /// let builder = SessionManagerBuilder::new()
    ///     .with_sip_port(5061);
    /// ```
    pub fn with_sip_port(mut self, port: u16) -> Self {
        self.config.sip_port = port;
        self
    }
    
    /// Set the local SIP address
    /// 
    /// This should be a full SIP URI that represents this endpoint.
    /// 
    /// # Arguments
    /// * `address` - SIP URI (e.g., "sip:alice@192.168.1.100:5060")
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// let builder = SessionManagerBuilder::new()
    ///     .with_local_address("sip:alice@company.com:5060");
    /// ```
    pub fn with_local_address(mut self, address: impl Into<String>) -> Self {
        self.config.local_address = address.into();
        self
    }
    
    /// Set the local bind address for media (RTP/RTCP)
    /// 
    /// This is the IP address that media sessions will bind to.
    /// Use `0.0.0.0:0` to bind to all interfaces (default).
    /// 
    /// # Arguments
    /// * `addr` - Socket address to bind media sessions to
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// let builder = SessionManagerBuilder::new()
    ///     .with_local_bind_addr("192.168.1.100:0".parse().unwrap());
    /// ```
    pub fn with_local_bind_addr(mut self, addr: std::net::SocketAddr) -> Self {
        self.config.local_bind_addr = addr;
        self
    }
    
    /// Set the media port range for RTP
    /// 
    /// These ports are used for RTP media streams. Each call uses
    /// one port from this range.
    /// 
    /// # Arguments
    /// * `start` - First port in the range (inclusive)
    /// * `end` - Last port in the range (inclusive)
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// // Reserve ports 30000-31000 for RTP
    /// let builder = SessionManagerBuilder::new()
    ///     .with_media_ports(30000, 31000);
    /// ```
    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media_port_start = start;
        self.config.media_port_end = end;
        self
    }
    
    /// Enable STUN for NAT traversal
    /// 
    /// STUN helps discover public IP addresses when behind NAT.
    /// 
    /// # Arguments
    /// * `server` - STUN server address (e.g., "stun.l.google.com:19302")
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// let builder = SessionManagerBuilder::new()
    ///     .with_stun("stun.stunprotocol.org:3478");
    /// ```
    /// 
    /// # Popular STUN Servers
    /// - Google: "stun.l.google.com:19302"
    /// - Twilio: "global.stun.twilio.com:3478"
    /// - Cloudflare: "stun.cloudflare.com:3478"
    pub fn with_stun(mut self, server: impl Into<String>) -> Self {
        self.config.enable_stun = true;
        self.config.stun_server = Some(server.into());
        self
    }
    
    /// Set the call event handler
    /// 
    /// The handler receives callbacks for incoming calls and other events.
    /// If no handler is set, incoming calls will be automatically rejected.
    /// 
    /// # Arguments
    /// * `handler` - Implementation of the CallHandler trait
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::{SessionManagerBuilder, CallHandler, CallDecision, IncomingCall, CallSession};
    /// use std::sync::Arc;
    /// 
    /// #[derive(Debug)]
    /// struct MyCallHandler;
    /// 
    /// impl MyCallHandler {
    ///     fn new() -> Self { Self }
    /// }
    /// 
    /// #[async_trait::async_trait]
    /// impl CallHandler for MyCallHandler {
    ///     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
    ///         CallDecision::Accept(None)
    ///     }
    ///     async fn on_call_ended(&self, call: CallSession, reason: &str) {}
    /// }
    /// 
    /// let handler = Arc::new(MyCallHandler::new());
    /// let builder = SessionManagerBuilder::new()
    ///     .with_handler(handler);
    /// ```
    pub fn with_handler(mut self, handler: Arc<dyn CallHandler>) -> Self {
        self.handler = Some(handler);
        self
    }
    
    /// Enable SIP client features
    /// 
    /// Enables non-session SIP operations like REGISTER, MESSAGE, and SUBSCRIBE.
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::{SessionManagerBuilder, SipClient};
    /// 
    /// async fn example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let coordinator = SessionManagerBuilder::new()
    ///         .with_sip_port(5060)
    ///         .enable_sip_client()
    ///         .build()
    ///         .await?;
    ///     
    ///     // Now can use SipClient methods
    ///     let registration = coordinator.register(
    ///         "sip:registrar.example.com",
    ///         "sip:alice@example.com",
    ///         "sip:alice@192.168.1.100:5060",
    ///         3600
    ///     ).await?;
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub fn enable_sip_client(mut self) -> Self {
        self.config.enable_sip_client = true;
        self
    }
    
    /// Set media configuration
    /// 
    /// Configure media preferences including codecs, audio processing, and SDP attributes.
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::{SessionManagerBuilder, MediaConfig};
    /// 
    /// async fn example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let media_config = MediaConfig {
    ///         preferred_codecs: vec!["opus".to_string(), "PCMU".to_string()],
    ///         echo_cancellation: true,
    ///         noise_suppression: true,
    ///         ..Default::default()
    ///     };
    ///     
    ///     let coordinator = SessionManagerBuilder::new()
    ///         .with_sip_port(5060)
    ///         .with_media_config(media_config)
    ///         .build()
    ///         .await?;
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub fn with_media_config(mut self, media_config: MediaConfig) -> Self {
        self.config.media_config = media_config;
        self
    }
    
    /// Set preferred codecs
    /// 
    /// Convenience method to set codec preferences without creating a full MediaConfig.
    /// 
    /// # Arguments
    /// * `codecs` - List of codec names in priority order (e.g., ["opus", "PCMU", "PCMA"])
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// async fn example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let coordinator = SessionManagerBuilder::new()
    ///         .with_sip_port(5060)
    ///         .with_preferred_codecs(vec!["opus", "G722", "PCMU"])
    ///         .build()
    ///         .await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn with_preferred_codecs<I, S>(mut self, codecs: I) -> Self 
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.media_config.preferred_codecs = codecs.into_iter()
            .map(Into::into)
            .collect();
        self
    }
    
    /// Enable or disable echo cancellation
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// async fn example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let coordinator = SessionManagerBuilder::new()
    ///         .with_sip_port(5060)
    ///         .with_echo_cancellation(true)
    ///         .build()
    ///         .await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn with_echo_cancellation(mut self, enabled: bool) -> Self {
        self.config.media_config.echo_cancellation = enabled;
        self
    }
    
    /// Configure audio processing options
    /// 
    /// Enable or disable all audio processing features at once.
    /// 
    /// # Arguments
    /// * `enabled` - Whether to enable echo cancellation, noise suppression, and AGC
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// 
    /// async fn example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let coordinator = SessionManagerBuilder::new()
    ///         .with_sip_port(5060)
    ///         .with_audio_processing(true)  // Enables all audio processing
    ///         .build()
    ///         .await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn with_audio_processing(mut self, enabled: bool) -> Self {
        self.config.media_config.echo_cancellation = enabled;
        self.config.media_config.noise_suppression = enabled;
        self.config.media_config.auto_gain_control = enabled;
        self
    }
    
    /// Build and initialize the SessionManager
    /// 
    /// This method:
    /// 1. Creates all subsystems (dialog manager, media manager, etc.)
    /// 2. Binds to the configured network ports
    /// 3. Starts background tasks for processing
    /// 4. Returns the ready-to-use SessionCoordinator
    /// 
    /// # Errors
    /// 
    /// Can fail if:
    /// - Network ports are already in use
    /// - Invalid configuration
    /// - System resource limitations
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::{SessionManagerBuilder, SessionControl};
    /// 
    /// async fn example() -> Result<(), Box<dyn std::error::Error>> {
    ///     let coordinator = SessionManagerBuilder::new()
    ///         .with_sip_port(5060)
    ///         .build()
    ///         .await?;
    ///         
    ///     // Now ready to make/receive calls
    ///     SessionControl::start(&coordinator).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn build(self) -> Result<Arc<SessionCoordinator>> {
        // Create the top-level coordinator with all subsystems
        let coordinator = SessionCoordinator::new(
            self.config,
            self.handler,
        ).await?;
        
        // Start all subsystems
        coordinator.start().await?;
        
        Ok(coordinator)
    }
    
    /// Set transaction manager for server mode
    /// 
    /// Required when building a server-oriented session manager.
    /// 
    /// # Arguments
    /// * `tm` - Transaction manager instance
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// use std::sync::Arc;
    /// 
    /// fn example() {
    ///     // let tm = Arc::new(TransactionManager::new(...));
    ///     // let builder = SessionManagerBuilder::new()
    ///     //     .with_transaction_manager(tm);
    /// }
    /// ```
    pub fn with_transaction_manager(mut self, tm: Arc<rvoip_transaction_core::TransactionManager>) -> Self {
        self.transaction_manager = Some(tm);
        self
    }
    
    /// Build with transaction manager for server applications
    /// 
    /// Similar to `build()` but uses an existing transaction manager
    /// instead of creating its own. This is used by call-engine.
    /// 
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::SessionManagerBuilder;
    /// use std::sync::Arc;
    /// 
    /// async fn example() -> Result<(), Box<dyn std::error::Error>> {
    ///     // let transaction_manager = Arc::new(TransactionManager::new(...));
    ///     // 
    ///     // let coordinator = SessionManagerBuilder::new()
    ///     //     .with_sip_port(5060)
    ///     //     .with_local_address("sip:server@example.com")
    ///     //     .build_with_transaction_manager(transaction_manager)
    ///     //     .await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn build_with_transaction_manager(
        self,
        _transaction_manager: Arc<rvoip_transaction_core::TransactionManager>,
    ) -> Result<Arc<SessionCoordinator>> {
        // For now, just use regular build
        // In the future, we'll integrate the transaction manager into dialog subsystem
        self.build().await
    }
}

impl Default for SessionManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_builder_defaults() {
        let builder = SessionManagerBuilder::new();
        assert_eq!(builder.config.sip_port, 5060);
        assert_eq!(builder.config.media_port_start, 10000);
        assert_eq!(builder.config.media_port_end, 20000);
    }
    
    #[test]
    fn test_builder_configuration() {
        let builder = SessionManagerBuilder::new()
            .with_sip_port(5061)
            .with_local_address("alice@example.com")
            .with_media_ports(30000, 40000)
            .with_stun("stun.example.com:3478");
            
        assert_eq!(builder.config.sip_port, 5061);
        assert_eq!(builder.config.local_address, "alice@example.com");
        assert_eq!(builder.config.media_port_start, 30000);
        assert_eq!(builder.config.media_port_end, 40000);
        assert!(builder.config.enable_stun);
        assert_eq!(builder.config.stun_server, Some("stun.example.com:3478".to_string()));
    }
} 