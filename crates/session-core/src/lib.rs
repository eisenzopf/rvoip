//! # rvoip-session-core
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
pub mod adapters;
pub mod auth;
pub mod session_registry;
pub mod session_store;
pub mod state_machine;
pub mod state_table;
pub mod types;

// ── Primary public API ──────────────────────────────────────────────────────

// Peer types
pub use api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ClosureHandler, EndReason, ShutdownHandle,
};
pub use api::stream_peer::{EventReceiver, PeerControl, StreamPeer, StreamPeerBuilder};

// Built-in handlers
pub use api::handlers::{
    AutoAnswerHandler, QueueHandler, RejectAllHandler, RoutingAction, RoutingHandler, RoutingRule,
};

// Call control
pub use api::audio::{AudioReceiver, AudioSender, AudioStream};
pub use api::handle::{CallId, SessionHandle};
pub use api::incoming::{IncomingCall, IncomingCallGuard};

// Configuration & registration
pub use api::unified::{AudioSource, BridgeError, BridgeHandle, Registration, RelUsage};
pub use api::{Config, RegistrationHandle, SipContactMode, SipTlsMode, UnifiedCoordinator};

// Events
pub use api::events::Event;

// Errors
pub use errors::{Result, SessionError};

// State / identity types
pub use state_table::types::SessionId;
pub use types::CallState;

// ── Prelude ─────────────────────────────────────────────────────────────────

/// Common imports for most use cases.
///
/// ```
/// use rvoip_session_core::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        AudioReceiver, AudioSender, AudioStream, CallHandler, CallHandlerDecision, CallId,
        CallState, CallbackPeer, Config, EndReason, Event, EventReceiver, IncomingCall,
        IncomingCallGuard, PeerControl, Registration, RegistrationHandle, Result, SessionError,
        SessionHandle, SipContactMode, SipTlsMode, StreamPeer, StreamPeerBuilder,
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
    pub use crate::adapters::{DialogAdapter, MediaAdapter};
    pub use crate::api::builder::SessionBuilder;
    pub use crate::api::types::{
        parse_sdp_connection, AudioStreamConfig, CallDecision, CallSession, MediaInfo, SdpInfo,
        SessionStats,
    };
    pub use crate::session_store::{
        ActionRecord, GuardResult, HistoryConfig, NegotiatedConfig, SessionHistory, SessionState,
        SessionStore, TransitionRecord,
    };
    pub use crate::state_machine::StateMachine;
    pub use crate::state_table::types::{EventType, Role, SessionId};
    pub use crate::state_table::{Action, Guard};
}
