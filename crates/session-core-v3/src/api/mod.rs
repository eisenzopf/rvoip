//! # Session Core v2 API
//!
//! A clean, state-machine driven SIP session management library with support for calls,
//! registration, subscriptions, and instant messaging.
//!
//! ## Quick Start
//!
//! The simplest way to use this library is through the `SimplePeer` API:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::simple::SimplePeer;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     // Create a SIP peer
//!     let peer = SimplePeer::new("alice").await?;
//!
//!     // Make a call
//!     let call_id = peer.call("sip:bob@192.168.1.100:5060").await?;
//!
//!     // Wait a bit for the call to be established
//!     tokio::time::sleep(std::time::Duration::from_secs(5)).await;
//!
//!     // End the call
//!     peer.hangup(&call_id).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Receiving Calls
//!
//! Handle incoming calls with accept/reject logic:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::simple::{SimplePeer, IncomingCall};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let mut peer = SimplePeer::new("bob").await?;
//!
//!     // Wait for an incoming call
//!     if let Some(incoming) = peer.incoming_call().await {
//!         // Note: incoming has session info, not a from() method directly
//!
//!         // Accept the call
//!         incoming.accept(&peer).await?;
//!
//!         // Talk for a while
//!         tokio::time::sleep(std::time::Duration::from_secs(10)).await;
//!
//!         // Note: hangup would use the call_id from the incoming call
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Call Features
//!
//! The API supports various call features:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::simple::SimplePeer;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let peer = SimplePeer::new("alice").await?;
//!     let call_id = peer.call("sip:bob@example.com:5060").await?;
//!
//!     // Hold the call
//!     peer.hold(&call_id).await?;
//!
//!     // Resume the call
//!     peer.resume(&call_id).await?;
//!
//!     // Send DTMF tones
//!     peer.send_dtmf(&call_id, '1').await?;
//!     peer.send_dtmf(&call_id, '#').await?;
//!
//!     // Transfer the call
//!     peer.transfer(&call_id, "sip:charlie@example.com:5060").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Registration
//!
//! Register with a SIP registrar for receiving calls:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::unified::{UnifiedCoordinator, Config};
//! use rvoip_session_core_v2::state_table::types::EventType;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let config = Config {
//!         local_uri: "sip:alice@example.com".to_string(),
//!         sip_port: 5060,
//!         ..Default::default()
//!     };
//!
//!     let coordinator = UnifiedCoordinator::new(config).await?;
//!
//!     // Registration happens through the state machine
//!     // You would typically trigger it through the simple API or events
//!
//!     // Registration is now active
//!     // The session will handle registration refreshes automatically
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Subscriptions and NOTIFY
//!
//! Subscribe to presence or other events:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::unified::{UnifiedCoordinator, Config};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let coordinator = UnifiedCoordinator::new(Config::default()).await?;
//!
//!     // Subscription support is built into the state machine
//!     // SUBSCRIBE/NOTIFY messages are handled automatically
//!     // when the appropriate events are triggered
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Instant Messaging (MESSAGE)
//!
//! Send and receive SIP MESSAGE requests:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::unified::{UnifiedCoordinator, Config};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let config = Config {
//!         local_uri: "sip:alice@example.com".to_string(),
//!         ..Default::default()
//!     };
//!     let coordinator = UnifiedCoordinator::new(config).await?;
//!
//!     // MESSAGE support is built into the state machine
//!     // SIP MESSAGE requests are handled through the dialog adapter
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Custom Configuration
//!
//! Configure the peer with specific network settings:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::simple::{SimplePeer, Config};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let config = Config {
//!         sip_port: 5080,  // Custom SIP port
//!         media_port_start: 10000,
//!         media_port_end: 20000,
//!         local_ip: "192.168.1.100".parse().unwrap(),
//!         bind_addr: "192.168.1.100:5080".parse().unwrap(),
//!         local_uri: "sip:alice@192.168.1.100:5080".to_string(),
//!         state_table_path: Some("custom_states.yaml".into()),
//!     };
//!
//!     let peer = SimplePeer::with_config("alice", config).await?;
//!
//!     // Use the peer normally
//!     let call_id = peer.call("sip:bob@192.168.1.200:5060").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Parallel Operations
//!
//! Handle multiple calls simultaneously:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::simple::SimplePeer;
//! use tokio::task::JoinSet;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let mut calls = JoinSet::new();
//!
//!     // Make multiple calls in parallel with separate peers
//!     for i in 1..=3 {
//!         let name = format!("operator{}", i);
//!         let uri = format!("sip:user{}@example.com:5060", i);
//!
//!         calls.spawn(async move {
//!             let peer = SimplePeer::new(&name).await?;
//!             let call_id = peer.call(&uri).await?;
//!             tokio::time::sleep(std::time::Duration::from_secs(10)).await;
//!             peer.hangup(&call_id).await?;
//!             Ok::<_, rvoip_session_core_v2::errors::SessionError>(())
//!         });
//!     }
//!
//!     // Wait for all calls to complete
//!     while let Some(result) = calls.join_next().await {
//!         result.unwrap()?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Conference Calls
//!
//! Create a simple conference by bridging calls:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::simple::SimplePeer;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let peer = SimplePeer::new("conference_host").await?;
//!
//!     // Make first call
//!     let call1 = peer.call("sip:alice@example.com:5060").await?;
//!
//!     // Make second call
//!     let call2 = peer.call("sip:bob@example.com:5060").await?;
//!
//!     // Create a conference (using the create_conference method that takes a CallId and name)
//!     peer.create_conference(&call1, "my_conference").await?;
//!
//!     // Add second call to the conference
//!     peer.add_to_conference(&call1, &call2).await?;
//!
//!     // Let them talk
//!     tokio::time::sleep(std::time::Duration::from_secs(60)).await;
//!
//!     // End the calls
//!     peer.hangup(&call1).await?;
//!     peer.hangup(&call2).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Error Handling
//!
//! Proper error handling with retry logic:
//!
//! ```rust,no_run
//! use rvoip_session_core_v2::api::simple::SimplePeer;
//! use rvoip_session_core_v2::errors::SessionError;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), rvoip_session_core_v2::errors::SessionError> {
//!     let peer = SimplePeer::new("reliable_caller").await?;
//!
//!     // Retry call with exponential backoff
//!     let mut retry_count = 0;
//!     let max_retries = 3;
//!     let mut delay = std::time::Duration::from_secs(1);
//!
//!     loop {
//!         match peer.call("sip:busy@example.com:5060").await {
//!             Ok(call_id) => {
//!                 println!("Call established: {:?}", call_id);
//!                 break;
//!             }
//!             Err(e) => {
//!                 retry_count += 1;
//!                 if retry_count >= max_retries {
//!                     return Err(e.into());
//!                 }
//!                 println!("Call failed, retrying in {:?}...", delay);
//!                 tokio::time::sleep(delay).await;
//!                 delay *= 2;  // Exponential backoff
//!             }
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Module Structure
//!
//! - `types` - Core data types (SessionId, CallState, etc.)
//! - `unified` - The unified coordinator that manages sessions
//! - `simple` - Simplified peer API for easy use
//! - `builder` - Session configuration builders
//!
//! The library uses a state machine approach where all business logic
//! is defined in YAML state tables, making it highly customizable
//! without code changes.

// Core modules only
pub mod types;      // Core types (legacy)
pub mod events;     // Event-driven API for v3
pub mod unified;    // Unified API
pub mod builder;    // Session builder
pub mod simple;     // Simple peer API

// Re-export the main types
pub use types::{
    SessionId, CallSession, IncomingCall, CallDecision,
    SessionStats, MediaInfo, AudioStreamConfig,
    parse_sdp_connection, SdpInfo,
};
pub use crate::types::CallState;

// Re-export the unified API
pub use unified::{UnifiedCoordinator, Config};

// Re-export the simple API (the one people should actually use)
pub use simple::SimplePeer;

// Re-export event types
pub use events::{
    Event, CallHandle, CallId,
};


// Re-export builder
pub use builder::SessionBuilder;


// Re-export from state table for consistency
pub use crate::state_table::types::{Role, EventType};

// Error types
pub use crate::errors::{Result, SessionError};