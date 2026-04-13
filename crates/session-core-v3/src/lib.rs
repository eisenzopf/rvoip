//! # rvoip-session-core-v3
//!
//! State-machine driven SIP session management for building clients, servers,
//! proxies, and call center software.
//!
//! ## Two API Styles
//!
//! | Type | Best for | Style |
//! |------|----------|-------|
//! | [`StreamPeer`] | Clients, scripts, tests | Sequential — call methods, await results |
//! | [`CallbackPeer`] | Servers, proxies, IVR | Reactive — implement [`CallHandler`] trait |
//!
//! See the [`api`] module docs for quick-start examples.

// ── Internal modules (pub for doc visibility, use the re-exports below) ─────

pub mod api;
pub mod errors;

// These are pub so internal code and advanced users can reach them,
// but the primary public surface is the re-exports below.
pub mod state_table;
pub mod state_machine;
pub mod session_store;
pub mod adapters;
pub mod auth;
pub mod session_registry;
pub mod types;

// ── Primary public API ──────────────────────────────────────────────────────

// Peer types
pub use api::stream_peer::{StreamPeer, PeerControl, EventReceiver, StreamPeerBuilder};
pub use api::callback_peer::{CallbackPeer, CallHandler, CallHandlerDecision, EndReason, ClosureHandler};

// Built-in handlers
pub use api::handlers::{AutoAnswerHandler, RejectAllHandler, RoutingHandler, RoutingAction, RoutingRule, QueueHandler};

// Call control
pub use api::handle::{SessionHandle, CallId};
pub use api::incoming::{IncomingCall, IncomingCallGuard};
pub use api::audio::{AudioStream, AudioSender, AudioReceiver};

// Configuration & registration
pub use api::{UnifiedCoordinator, Config, RegistrationHandle};
pub use api::unified::Registration;

// Events
pub use api::events::Event;

// Errors
pub use errors::{Result, SessionError};

// State / identity types
pub use types::CallState;
pub use state_table::types::SessionId;

// ── Prelude ─────────────────────────────────────────────────────────────────

/// Common imports for most use cases.
///
/// ```
/// use rvoip_session_core_v3::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        StreamPeer, StreamPeerBuilder, PeerControl, EventReceiver,
        CallbackPeer, CallHandler, CallHandlerDecision, EndReason,
        SessionHandle, CallId, IncomingCall, IncomingCallGuard,
        AudioStream, AudioSender, AudioReceiver,
        Event, Config, Registration, RegistrationHandle,
        Result, SessionError, CallState,
    };
}

// ── Legacy API (deprecated) ─────────────────────────────────────────────────

/// Deprecated: use [`StreamPeer`] instead.
#[deprecated(note = "Use StreamPeer instead")]
pub use api::simple::SimplePeer;

/// Deprecated: use [`SessionHandle`] instead.
#[deprecated(note = "Use SessionHandle instead")]
pub use api::events::CallHandle;

// ── Internals (for power users / testing) ───────────────────────────────────

/// Advanced types for power users who need direct access to the state machine,
/// session store, or adapters. Most users should not need these.
pub mod internals {
    pub use crate::session_store::{
        SessionStore, SessionState, NegotiatedConfig,
        SessionHistory, HistoryConfig, TransitionRecord, GuardResult, ActionRecord,
    };
    pub use crate::state_machine::StateMachine;
    pub use crate::state_table::{Guard, Action};
    pub use crate::state_table::types::{SessionId, Role, EventType};
    pub use crate::adapters::{DialogAdapter, MediaAdapter};
    pub use crate::api::builder::SessionBuilder;
    pub use crate::api::types::{
        CallSession, CallDecision, SessionStats, MediaInfo, AudioStreamConfig,
        parse_sdp_connection, SdpInfo,
    };
}