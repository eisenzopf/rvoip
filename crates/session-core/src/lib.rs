//! RVOIP Session Core Library
//!
//! This library provides Session Initiation Protocol (SIP) session and dialog management 
//! for the RVOIP stack. It serves as the middle layer between low-level SIP transaction 
//! processing and high-level application logic.
//! 
//! # Overview
//! 
//! Session-core coordinates SIP dialogs, media sessions, and call control operations
//! while integrating with dialog-core (SIP protocol), media-core (audio processing),
//! and providing APIs for higher-level applications.
//! 
//! # Quick Start
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::sync::Arc;
//! use std::net::SocketAddr;
//! 
//! # fn main() {
//! # let rt = tokio::runtime::Runtime::new().unwrap();
//! # rt.block_on(async {
//! // Create a session coordinator
//! let addr: SocketAddr = "127.0.0.1:15070".parse().unwrap();
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(15070)
//!     .with_local_bind_addr(addr)
//!     .with_media_ports(10000, 20000)
//!     .build()
//!     .await
//!     .unwrap();
//!     
//! // Start accepting calls
//! SessionControl::start(&coordinator).await.unwrap();
//! # });
//! # }
//! ```
//! 
//! # Network Configuration
//! 
//! ## Bind Address Propagation
//! 
//! Session-core respects configured bind addresses and propagates them through all layers:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::net::SocketAddr;
//! 
//! # fn main() {
//! # let rt = tokio::runtime::Runtime::new().unwrap();
//! # rt.block_on(async {
//! // Configure specific IP for production deployment
//! let addr: SocketAddr = "127.0.0.1:15071".parse().unwrap();  // Use localhost for test
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(15071)
//!     .with_local_bind_addr(addr)
//!     .with_media_ports(10000, 20000)
//!     .build()
//!     .await
//!     .unwrap();
//! # });
//! # }
//! ```
//! 
//! The configured IP propagates to dialog-core and transport layers.
//! No more hardcoded 0.0.0.0 addresses when you specify an IP.
//! 
//! ## Media Port Configuration
//! 
//! The library supports automatic port allocation when you use port 0:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::net::SocketAddr;
//! 
//! # fn main() {
//! # let rt = tokio::runtime::Runtime::new().unwrap();
//! # rt.block_on(async {
//! // Port 0 signals automatic allocation from the configured range
//! let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
//! let coordinator = SessionManagerBuilder::new()
//!     .with_local_bind_addr(addr)  // Port 0 = auto
//!     .with_media_ports(10000, 20000)
//!     .build()
//!     .await
//!     .unwrap();
//! # });
//! # }
//! ```
//! 
//! When port is 0:
//! - It means "allocate automatically when needed"
//! - Actual allocation happens when media sessions are created  
//! - Uses the configured media_port_start to media_port_end range
//! - Each session gets unique ports from the pool

pub mod api;
pub mod session;
pub mod dialog;        // NEW - dialog-core integration
pub mod media;         // EXISTING - media-core integration
pub mod sdp;           // NEW - SDP negotiation engine
pub mod manager;       // SIMPLIFIED - orchestration only
pub mod coordination;
pub mod bridge;
pub mod conference;    // NEW - Conference functionality
pub mod coordinator;
pub mod events;        // NEW - High-performance federated event system
pub mod auth;          // NEW - OAuth 2.0 authentication

// Core error types
mod errors;
pub use errors::{SessionError, Result};

// Re-export the main API for convenience
pub use api::*;

// Re-export SessionManager for direct access
// pub use manager::SessionManager; // Using coordinator directly

// Prelude module for common imports
pub mod prelude {
    pub use crate::api::*;
    pub use crate::errors::{SessionError, Result};
    pub use crate::manager::events::{SessionEvent, SessionEventProcessor};
    // pub use crate::manager::SessionManager; // Disabled - using SessionCoordinator directly
    pub use crate::dialog::DialogManager;  // NEW
    pub use crate::conference::prelude::*; // NEW - Conference functionality
    
    // Audio streaming types
    pub use crate::api::types::{AudioFrame, AudioStreamConfig};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_compiles() {
        // Basic smoke test to ensure the library structure compiles
        assert!(true);
    }
}

// Feature flags
#[cfg(feature = "testing")]
pub mod testing {
    //! Testing utilities and mocks
    pub use crate::manager::testing::*;
}

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the session core library
/// 
/// This should be called once at application startup to initialize
/// any global state or resources.
pub fn init() {
    // Initialize logging if not already done
    let _ = tracing_subscriber::fmt::try_init();
    
    // Any other global initialization
    tracing::info!("RVoIP Session Core v{} initialized", VERSION);
} 