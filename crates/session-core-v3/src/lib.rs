//! Session-core v3 with single session and callback-based event handling
//! 
//! This is a refactored version of session-core that uses a master state table
//! to coordinate between dialog-core and media-core. The key benefits are:
//! 
//! 1. Deterministic state transitions
//! 2. Simplified event handling
//! 3. Easier testing and verification
//! 4. Reduced complexity
//! 
//! The architecture consists of:
//! - State Table: Defines all valid transitions
//! - State Machine: Executes transitions
//! - Session Store: Maintains session state
//! - Coordinator: Routes events to state machine
//! - Adapters: Interface with dialog-core and media-core

// Core modules
pub mod api;
pub mod state_table;
pub mod state_machine;
pub mod session_store;
pub mod adapters;
pub mod errors;
pub mod auth;

// New core infrastructure
pub mod session_registry;
pub mod types;


// ── Primary public API ──────────────────────────────────────────────────────

// Peer types (new v3 API)
pub use api::stream_peer::{StreamPeer, PeerControl, EventReceiver};
pub use api::callback_peer::{CallbackPeer, CallHandler, CallHandlerDecision, EndReason};

// Core session types
pub use api::handle::{SessionHandle, CallId};
pub use api::incoming::{IncomingCall, IncomingCallGuard};
pub use api::audio::{AudioStream, AudioSender, AudioReceiver};

// Configuration & registration
pub use api::{UnifiedCoordinator, SessionBuilder, Config, RegistrationHandle};

// Events
pub use api::events::{Event, CallHandle};

// Errors
pub use errors::{Result, SessionError};

// State / identity types
pub use state_table::types::{SessionId, Role, EventType};
pub use types::CallState;

// ── Legacy API (kept for backward compatibility) ────────────────────────────

/// Deprecated: use [`StreamPeer`] instead.
#[deprecated(note = "Use StreamPeer instead")]
pub use api::simple::SimplePeer;

// ── Internal / advanced usage ───────────────────────────────────────────────

// Re-export internal types for advanced usage (power users)
pub use session_store::{
    SessionStore, SessionState, NegotiatedConfig,
    SessionHistory, HistoryConfig, TransitionRecord, GuardResult, ActionRecord,
};
pub use state_machine::StateMachine;
pub use state_table::{Guard, Action};
pub use adapters::{DialogAdapter, MediaAdapter};