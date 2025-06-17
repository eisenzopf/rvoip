//! Public API for session-core
//!
//! This module provides the main public interface for the session-core crate.
//! 
//! # Quick Start
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::sync::Arc;
//! 
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // 1. Create a session coordinator
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_local_address("sip:user@192.168.1.100:5060")
//!         .build()
//!         .await?;
//!     
//!     // 2. Start the coordinator
//!     SessionControl::start(&coordinator).await?;
//!     
//!     // 3. Make an outgoing call
//!     let session = SessionControl::create_outgoing_call(
//!         &coordinator,
//!         "sip:alice@example.com",
//!         "sip:bob@192.168.1.100",
//!         None  // SDP will be generated automatically
//!     ).await?;
//!     
//!     // 4. Clean shutdown
//!     SessionControl::stop(&coordinator).await?;
//!     Ok(())
//! }
//! ```
//! 
//! # Architecture Overview
//! 
//! The API is organized into several key modules:
//! 
//! - **`types`** - Core data types (SessionId, CallSession, etc.)
//! - **`control`** - Main control interface (SessionControl trait)
//! - **`media`** - Media control operations (MediaControl trait)
//! - **`handlers`** - Event handling system (CallHandler trait)
//! - **`builder`** - Configuration and construction (SessionManagerBuilder)
//! 
//! # Two Ways to Handle Incoming Calls
//! 
//! ## 1. Immediate Decision Pattern
//! 
//! Make decisions synchronously in the CallHandler callback:
//! 
//! ```rust
//! #[derive(Debug)]
//! struct MyHandler;
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for MyHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Decision made immediately
//!         if is_allowed(&call.from) {
//!             CallDecision::Accept(generate_sdp_answer())
//!         } else {
//!             CallDecision::Reject("Not authorized".to_string())
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         println!("Call {} ended: {}", call.id(), reason);
//!     }
//! }
//! ```
//! 
//! ## 2. Deferred Decision Pattern
//! 
//! Defer the decision for asynchronous processing:
//! 
//! ```rust
//! #[derive(Debug)]
//! struct AsyncHandler {
//!     pending: Arc<Mutex<Vec<IncomingCall>>>,
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for AsyncHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Store for later processing
//!         self.pending.lock().unwrap().push(call);
//!         CallDecision::Defer
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         // Handle end of call
//!     }
//! }
//! 
//! // Process deferred calls asynchronously
//! async fn process_pending(
//!     coordinator: &Arc<SessionCoordinator>,
//!     pending: &Arc<Mutex<Vec<IncomingCall>>>
//! ) -> Result<()> {
//!     let calls = pending.lock().unwrap().drain(..).collect::<Vec<_>>();
//!     
//!     for call in calls {
//!         // Async operations: database lookup, authentication, etc.
//!         if async_check_allowed(&call).await? {
//!             // Generate SDP answer
//!             let answer = MediaControl::generate_sdp_answer(
//!                 coordinator,
//!                 &call.id,
//!                 &call.sdp.unwrap()
//!             ).await?;
//!             
//!             // Accept the call
//!             SessionControl::accept_incoming_call(
//!                 coordinator,
//!                 &call,
//!                 Some(answer)
//!             ).await?;
//!         } else {
//!             // Reject the call
//!             SessionControl::reject_incoming_call(
//!                 coordinator,
//!                 &call,
//!                 "Not authorized"
//!             ).await?;
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Common Use Cases
//! 
//! ## Basic Softphone
//! ```rust
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(5060)
//!     .with_handler(Arc::new(AutoAnswerHandler))
//!     .build()
//!     .await?;
//! ```
//! 
//! ## Call Center Queue
//! ```rust
//! let queue = Arc::new(QueueHandler::new(100));
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(5060)
//!     .with_handler(queue.clone())
//!     .build()
//!     .await?;
//! 
//! // Process queued calls in another task
//! tokio::spawn(process_queue(coordinator.clone(), queue));
//! ```
//! 
//! ## PBX with Routing
//! ```rust
//! let mut router = RoutingHandler::new();
//! router.add_route("sip:support@", "sip:queue@support.local");
//! router.add_route("sip:sales@", "sip:queue@sales.local");
//! 
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(5060)
//!     .with_handler(Arc::new(router))
//!     .build()
//!     .await?;
//! ```
//! 
//! # Best Practices
//! 
//! 1. **Use the public API only** - Don't access internal fields
//! 2. **Handle errors properly** - Network operations can fail
//! 3. **Monitor call quality** - Use MediaControl statistics methods
//! 4. **Clean up resources** - Always call terminate_session when done
//! 5. **Choose the right pattern** - Immediate for simple cases, deferred for complex logic
//! 
//! # See Also
//! 
//! - [COOKBOOK.md](../COOKBOOK.md) - Practical recipes
//! - [examples/](../examples/) - Full working examples
//! - [API_IMPROVEMENTS.md](API_IMPROVEMENTS.md) - Recent improvements

pub mod types;
pub mod handlers;
pub mod builder;
pub mod control;
pub mod media;
pub mod create;
pub mod examples;

// New API modules
pub mod bridge;
pub mod server_types;

// Re-export main types
pub use types::{
    SessionId, CallSession, CallState, IncomingCall, CallDecision, 
    SessionStats, MediaInfo, PreparedCall, CallDirection, TerminationReason,
    SdpInfo, parse_sdp_connection,
};
pub use handlers::CallHandler;
pub use builder::{SessionManagerBuilder, SessionManagerConfig};
pub use control::SessionControl;
pub use media::MediaControl;

// Re-export bridge functionality
pub use bridge::{
    BridgeId, BridgeInfo, BridgeEvent, BridgeEventType,
};

// Re-export server types
pub use server_types::{
    IncomingCallEvent, CallerInfo,
};

// Re-export conference functionality (make it public)
pub use crate::conference::{
    ConferenceManager, ConferenceApi, ConferenceCoordinator,
    ConferenceId, ConferenceConfig, ConferenceEvent,
    ConferenceRoom, ConferenceParticipant,
};

// Re-export error types
pub use crate::errors::{Result, SessionError};

// Type aliases for compatibility with call-engine
pub type Session = CallSession;
pub type ServerSessionManager = SessionCoordinator;
pub type ServerConfig = SessionManagerConfig;
pub type IncomingCallNotification = IncomingCallEvent;

// Re-export create helper function
pub async fn create_full_server_manager(
    transaction_manager: std::sync::Arc<rvoip_transaction_core::TransactionManager>,
    _config: ServerConfig,
) -> Result<std::sync::Arc<ServerSessionManager>> {
    // Use builder to create coordinator with transaction manager
    SessionManagerBuilder::new()
        .build_with_transaction_manager(transaction_manager)
        .await
}

// Re-export the SessionCoordinator as the main entry point
pub use crate::coordinator::SessionCoordinator; 