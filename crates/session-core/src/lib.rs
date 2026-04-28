//! # rvoip-session-core
//!
//! Application-facing SIP session orchestration for Rust VoIP applications.
//!
//! `session-core` sits above the lower-level SIP dialog and media crates. It
//! owns call/session state, registration state, SIP feature orchestration, and
//! the public control surfaces that applications use to build softphones,
//! test clients, IVRs, B2BUA legs, routing servers, and PBX/SBC interop tools.
//!
//! ## Choosing an API Surface
//!
//! | Surface | Best for | Programming model |
//! | --- | --- | --- |
//! | [`StreamPeer`] | Clients, scripts, softphones, integration tests | Sequential calls plus an event stream |
//! | [`CallbackPeer`] | Servers, IVR, routing apps, reactive endpoints | Implement [`CallHandler`] hooks |
//! | [`UnifiedCoordinator`] | B2BUAs, gateways, custom frameworks | Lower-level call/session orchestration |
//! | [`SessionHandle`] | Per-call control from any surface | Hold/resume, DTMF, transfer, audio, teardown |
//!
//! Most applications should start with [`StreamPeer`] or [`CallbackPeer`].
//! Use [`UnifiedCoordinator`] when you need to compose multiple call legs,
//! bridge media, subscribe to filtered event streams, or build your own peer
//! abstraction.
//!
//! ## StreamPeer: Sequential Client or Test Code
//!
//! [`StreamPeer`] owns a coordinator plus a typed event receiver. Its helpers
//! block until the next matching event, which keeps simple clients and tests
//! direct:
//!
//! ```rust,no_run
//! use rvoip_session_core::{Result, StreamPeer};
//!
//! # async fn example() -> Result<()> {
//! let mut alice = StreamPeer::new("alice").await?;
//! let call = alice.call("sip:bob@192.168.1.50:5060").await?;
//! let call = alice.wait_for_answered(call.id()).await?;
//!
//! call.send_dtmf('1').await?;
//! call.hold().await?;
//! call.resume().await?;
//! call.hangup().await?;
//! # Ok(())
//! # }
//! ```
//!
//! For concurrent code, split it into [`PeerControl`] and [`EventReceiver`].
//!
//! ## CallbackPeer: Reactive Server Code
//!
//! [`CallbackPeer`] dispatches typed events to a [`CallHandler`]. Return a
//! [`CallHandlerDecision`] for incoming calls, and implement only the hooks
//! your app needs:
//!
//! ```rust,no_run
//! use async_trait::async_trait;
//! use rvoip_session_core::{
//!     CallHandler, CallHandlerDecision, CallbackPeer, Config, IncomingCall, Result,
//! };
//!
//! struct App;
//!
//! #[async_trait]
//! impl CallHandler for App {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
//!         if call.to.contains("support") {
//!             CallHandlerDecision::Accept
//!         } else {
//!             CallHandlerDecision::Reject {
//!                 status: 404,
//!                 reason: "Not Found".into(),
//!             }
//!         }
//!     }
//! }
//!
//! # async fn example() -> Result<()> {
//! let peer = CallbackPeer::new(App, Config::default()).await?;
//! peer.run().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## UnifiedCoordinator: Custom Orchestration
//!
//! [`UnifiedCoordinator`] exposes the same session machinery without imposing
//! a peer style. It is useful when an application needs to manage several
//! calls at once, subscribe to raw event streams, bridge two active RTP
//! sessions, drive registrations, or construct a B2BUA on top:
//!
//! ```rust,no_run
//! use rvoip_session_core::{Config, Event, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("bridge", 5060)).await?;
//! let mut events = coordinator.events().await?;
//!
//! let outbound = coordinator
//!     .make_call("sip:bridge@127.0.0.1:5060", "sip:bob@127.0.0.1:5070")
//!     .await?;
//!
//! while let Some(event) = events.next().await {
//!     if matches!(event, Event::CallAnswered { .. }) {
//!         coordinator.hangup(&outbound).await?;
//!         break;
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! When you build directly on the coordinator, call-control methods generally
//! take a [`SessionId`]. The peer surfaces wrap those IDs in [`SessionHandle`]
//! for ergonomic per-call control.
//!
//! ## Features Exposed Through SessionHandle
//!
//! [`SessionHandle`] is the per-call control object shared by all three
//! surfaces. It currently exposes:
//!
//! - call teardown with [`SessionHandle::hangup`]
//! - local hold/resume with [`SessionHandle::hold`] and [`SessionHandle::resume`]
//! - RFC 4733 DTMF send with [`SessionHandle::send_dtmf`]
//! - blind transfer with [`SessionHandle::transfer_blind`]
//! - attended-transfer primitives with [`SessionHandle::dialog_identity`] and
//!   [`SessionHandle::transfer_attended`]
//! - inbound REFER accept/reject with [`SessionHandle::accept_refer`] and
//!   [`SessionHandle::reject_refer`]
//! - typed per-call events with [`SessionHandle::events`]
//! - decoded/encoded audio frames with [`SessionHandle::audio`]
//!
//! ## Configuration and Interop
//!
//! [`Config`] controls SIP binding, advertised addresses, TLS, registration
//! contact behavior, SRTP, session timers, 100rel, P-Asserted-Identity,
//! outbound proxy routing for INVITEs, STUN/static media address overrides,
//! and codec matching policy. The Asterisk examples under
//! `examples/asterisk` and `examples/asterisk_callback` are the best current
//! executable reference for PBX interop with UDP/RTP and TLS/SDES-SRTP.
//!
//! See the [`api`] module docs for the complete module map and additional
//! quick-start examples.

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
    CallHandler, CallHandlerDecision, CallbackPeer, CallbackPeerControl, ClosureHandler, EndReason,
    ShutdownHandle,
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
        CallState, CallbackPeer, CallbackPeerControl, Config, EndReason, Event, EventReceiver,
        IncomingCall, IncomingCallGuard, PeerControl, Registration, RegistrationHandle, Result,
        SessionError, SessionHandle, SipContactMode, SipTlsMode, StreamPeer, StreamPeerBuilder,
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
