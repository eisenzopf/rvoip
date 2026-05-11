//! # rvoip-sip
//!
//! Application-facing SIP session orchestration for Rust VoIP applications.
//!
//! `rvoip-sip` sits above the lower-level SIP dialog and media crates. It
//! owns call/session state, registration state, SIP feature orchestration, and
//! the public control surfaces that applications use to build softphones,
//! test clients, IVRs, B2BUA legs, routing servers, and PBX/SBC interop tools.
//!
//! ## Choosing an API Surface
//!
//! | Surface | Best for | Programming model |
//! | --- | --- | --- |
//! | [`Endpoint`] | Softphones, PBX accounts, demos, simple IVR legs | Account/profile builder plus call helpers |
//! | [`StreamPeer`] | Clients, scripts, softphones, integration tests | Sequential calls plus an event stream |
//! | [`CallbackPeer`] | Servers, IVR, routing apps, reactive endpoints | Implement [`CallHandler`] hooks |
//! | [`UnifiedCoordinator`] | B2BUAs, gateways, custom frameworks | Lower-level call/session orchestration |
//! | [`SessionHandle`] | Per-call control from any surface | Hold/resume, DTMF, transfer, audio, teardown |
//!
//! Most applications should start with [`Endpoint`]. Move to [`StreamPeer`]
//! when you want to own the event stream, [`CallbackPeer`] when you want the
//! library to dispatch events into hooks, and [`UnifiedCoordinator`] when you
//! need to compose multiple call legs, bridge media, subscribe to filtered
//! event streams, inspect registration lifecycle metadata, or build your own
//! peer abstraction.
//!
//! ## Endpoint: PBX Account or Softphone
//!
//! [`Endpoint`] wraps [`StreamPeer`] with account/profile setup and bare
//! extension dialing:
//!
//! ```rust,no_run
//! use std::time::Duration;
//! use rvoip_sip::{Endpoint, EndpointProfile, Result};
//!
//! # async fn example() -> Result<()> {
//! let mut endpoint = Endpoint::builder()
//!     .name("alice")
//!     .account("1001")
//!     .password("secret")
//!     .registrar("sips:pbx.example.com:5061")
//!     .profile(EndpointProfile::AsteriskTlsSrtpRegisteredFlow)
//!     .build()
//!     .await?;
//!
//! endpoint.register().await?;
//! let call = endpoint.call("1002").await?;
//! call.wait_for_answered(Some(Duration::from_secs(30))).await?;
//! call.hangup().await?;
//! endpoint.shutdown().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## StreamPeer: Sequential Client or Test Code
//!
//! [`StreamPeer`] owns a coordinator plus a typed event receiver. Its helpers
//! block until the next matching event, which keeps simple clients and tests
//! direct:
//!
//! ```rust,no_run
//! use rvoip_sip::{Result, StreamPeer};
//!
//! # async fn example() -> Result<()> {
//! let mut alice = StreamPeer::new("alice").await?;
//! let call = alice.call("sip:bob@192.168.1.50:5060").await?;
//! let call = call.wait_for_answered(Some(std::time::Duration::from_secs(30))).await?;
//!
//! call.send_dtmf('1').await?;
//! call.hold().await?;
//! call.resume().await?;
//! call.hangup_and_wait(Some(std::time::Duration::from_secs(5))).await?;
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
//! use rvoip_sip::{
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
//! use rvoip_sip::{Config, Event, Result, UnifiedCoordinator};
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
//! - call teardown with [`SessionHandle::hangup`] or deterministic
//!   [`SessionHandle::hangup_and_wait`]
//! - provisional progress with [`SessionHandle::wait_for_progress`]
//! - answered-call waits with [`SessionHandle::wait_for_answered`]
//! - local hold/resume with [`SessionHandle::hold`] and [`SessionHandle::resume`]
//! - RFC 4733 DTMF send with [`SessionHandle::send_dtmf`]
//! - blind transfer with [`SessionHandle::transfer_blind`] or
//!   [`SessionHandle::transfer_blind_and_wait_for_outcome`]
//! - typed transfer lifecycle events that distinguish REFER completion from
//!   target-leg evidence
//! - attended-transfer primitives with [`SessionHandle::dialog_identity`] and
//!   [`SessionHandle::transfer_attended`]
//! - inbound REFER accept/reject with [`SessionHandle::accept_refer`] and
//!   [`SessionHandle::reject_refer`]
//! - typed SRTP negotiation state with [`SessionHandle::media_security`] or
//!   [`SessionHandle::wait_for_media_security`]
//! - typed per-call events with [`SessionHandle::events`]
//! - decoded/encoded audio frames with [`SessionHandle::audio`]
//!
//! ## B2BUA via `server::*`
//!
//! [`server`] adds B2BUA / gateway helpers on top of [`UnifiedCoordinator`] —
//! coordination glue, not a parallel access path to dialog/media. Three entry
//! points: [`server::SipBridgeStrategy`] for SIP↔SIP same-codec fast-path
//! bridges, [`server::ContactResolver`] for AOR → live Contact lookups
//! against `rvoip-sip-registrar`, and [`server::transfer`] for blind/
//! attended/external REFER orchestration. The optional [`server::b2bua`]
//! convenience wires the canonical inbound→originate→bridge pattern in one
//! call.
//!
//! ```rust,no_run
//! use rvoip_sip::api::events::Event;
//! use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
//! use rvoip_sip::server::b2bua::SipB2bua;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let coordinator = UnifiedCoordinator::new(Config::local("b2bua", 5070)).await?;
//! let b2bua = SipB2bua::new(coordinator.clone());
//! let mut events = coordinator.events().await?;
//! while let Some(Event::IncomingCall { call_id, .. }) = events.next().await {
//!     let _bridge = b2bua
//!         .handle_inbound("sip:b2bua@127.0.0.1", &call_id, "sip:upstream@example.com")
//!         .await?;
//!     // Drop the BridgeHandle to tear the bridge down.
//! }
//! # Ok(())
//! # }
//! ```
//!
//! See `examples/sip_b2bua.rs` for a complete CLI-driven runner.
//!
//! ## Cross-transport via `rvoip-core::Orchestrator` + `SipAdapter`
//!
//! [`SipAdapter`] implements `rvoip_core::ConnectionAdapter`, so SIP plugs
//! into the cross-transport `Orchestrator` alongside future
//! `rvoip-webrtc` / `rvoip-quic` adapters. Consumers that plan to add other
//! transports later use this surface today; the single SIP adapter
//! demonstrates the seam.
//!
//! ```rust,no_run
//! use rvoip_core::{Config as CoreConfig, Orchestrator};
//! use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
//! use rvoip_sip::SipAdapter;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let coordinator = UnifiedCoordinator::new(SipConfig::local("sip-leg", 5072)).await?;
//! let adapter = SipAdapter::new(coordinator).await?;
//! let orchestrator = Orchestrator::new(CoreConfig::default());
//! orchestrator.register(adapter)?;
//!
//! let mut events = orchestrator.subscribe_events();
//! // events.recv() yields normalized rvoip-core Events (translated from api::Event).
//! # let _ = events;
//! # Ok(())
//! # }
//! ```
//!
//! See `crates/rvoip-core/examples/sip_only_orchestrator.rs` for a complete
//! runner. When `rvoip-webrtc` and `rvoip-quic` ship, they register against
//! the same `Orchestrator` handle without reshaping consumer code.
//!
//! ## Configuration and Interop
//!
//! [`Config`] controls SIP binding, advertised addresses, TLS, registration
//! contact behavior, registration auto-refresh and graceful unregister,
//! SRTP, session timers, 100rel, P-Asserted-Identity, outbound proxy routing
//! for INVITEs and REGISTERs, STUN/static media address overrides, and codec
//! matching policy. Deployment profile constructors cover local labs, LAN PBX
//! use, Asterisk TLS registered-flow, FreeSWITCH internal profile, and
//! carrier/SBC starting points.
//!
//! PBX interop examples live under `examples/pbx`. That unified runner drives
//! the same Asterisk and FreeSWITCH scenarios through `Endpoint`, `StreamPeer`,
//! and `CallbackPeer::builder`.
//!
//! See the [`api`] module docs for the complete module map and additional
//! quick-start examples.

#![deny(rustdoc::bare_urls)]
#![deny(rustdoc::broken_intra_doc_links)]

// ── Internal modules (pub for doc visibility, use the re-exports below) ─────

pub mod adapter;
pub mod api;
pub mod errors;
pub mod server;

// These modules remain public for existing internal-style integrations, but
// they are hidden from generated docs. Application code should use the
// UnifiedCoordinator, StreamPeer, CallbackPeer, and SessionHandle surfaces.
#[doc(hidden)]
pub mod adapters;
#[doc(hidden)]
pub mod auth;
#[doc(hidden)]
pub mod session_registry;
#[doc(hidden)]
pub mod session_store;
#[doc(hidden)]
pub mod state_machine;
#[doc(hidden)]
pub mod state_table;
#[doc(hidden)]
pub mod types;

// ── Primary public API ──────────────────────────────────────────────────────

// Peer types
pub use adapter::SipAdapter;

pub use api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, CallbackPeerBuilder, CallbackPeerControl,
    ClosureHandler, EndReason, ShutdownHandle,
};
pub use api::endpoint::{
    Endpoint, EndpointAccount, EndpointAccountConfig, EndpointAudio, EndpointAudioFrame,
    EndpointAudioReceiver, EndpointAudioSender, EndpointBuilder, EndpointCall, EndpointCallId,
    EndpointConfig, EndpointControl, EndpointEvent, EndpointEvents, EndpointIncomingCall,
    EndpointMediaConfig, EndpointNetworkConfig, EndpointProfile, EndpointProfileName,
    EndpointRegistrationInfo, EndpointRegistrationStatus, EndpointSipTrace, EndpointSrtpMode,
    EndpointTransport,
};
pub use api::stream_peer::{EventReceiver, PeerControl, StreamPeer, StreamPeerBuilder};

// Built-in handlers
pub use api::handlers::{
    AutoAnswerHandler, QueueHandler, RejectAllHandler, RoutingAction, RoutingHandler, RoutingRule,
};

// Call control
pub use api::audio::{AudioReceiver, AudioSender, AudioStream};
pub use api::handle::{
    CallId, SessionHandle, SipReason, TransferDialogMatcher, TransferLifecycleOptions,
    TransferOutcome, TransferWaitMode,
};
pub use api::incoming::{IncomingCall, IncomingCallGuard};
pub use api::lifecycle::{
    CallAnsweredInfo, CallLifecycleSnapshot, CallProgressInfo, CallTerminalInfo,
};

// Configuration & registration
pub use api::unified::{AudioSource, BridgeError, BridgeHandle, Registration, RelUsage};
pub use api::{
    Config, RegistrationHandle, RegistrationInfo, RegistrationStatus, SipContactMode, SipTlsMode,
    SrtpSuitePolicy, UnifiedCoordinator,
};

// Events
pub use api::dialog_package::{
    DialogInfo, DialogInfoDocument, DialogPackageEvent, DialogPackageState,
};
pub use api::dialog_subscription::DialogSubscriptionHandle;
pub use api::events::{
    Event, MediaSecurityKeying, MediaSecurityProfile, MediaSecurityState, SipTrace, SipTraceConfig,
    SipTraceDirection, SubscriptionState, TransferKind, TransferTargetEvidence,
};

// Errors
pub use errors::{Result, SessionError};

// State / identity types
pub use state_table::types::SessionId;
pub use types::CallState;

// ── Prelude ─────────────────────────────────────────────────────────────────

/// Common imports for most use cases.
///
/// ```
/// use rvoip_sip::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        AudioReceiver, AudioSender, AudioStream, CallAnsweredInfo, CallHandler,
        CallHandlerDecision, CallId, CallLifecycleSnapshot, CallProgressInfo, CallState,
        CallTerminalInfo, CallbackPeer, CallbackPeerBuilder, CallbackPeerControl, Config,
        DialogInfo, DialogInfoDocument, DialogPackageEvent, DialogPackageState,
        DialogSubscriptionHandle, EndReason, Endpoint, EndpointAccount, EndpointAccountConfig,
        EndpointAudio, EndpointAudioFrame, EndpointAudioReceiver, EndpointAudioSender,
        EndpointBuilder, EndpointCall, EndpointCallId, EndpointConfig, EndpointControl,
        EndpointEvent, EndpointEvents, EndpointIncomingCall, EndpointMediaConfig,
        EndpointNetworkConfig, EndpointProfile, EndpointProfileName, EndpointRegistrationInfo,
        EndpointRegistrationStatus, EndpointSipTrace, EndpointSrtpMode, EndpointTransport, Event,
        EventReceiver, IncomingCall, IncomingCallGuard, MediaSecurityKeying, MediaSecurityProfile,
        MediaSecurityState, PeerControl, Registration, RegistrationHandle, RegistrationInfo,
        RegistrationStatus, Result, SessionError, SessionHandle, SipContactMode, SipReason,
        SipTlsMode, SipTrace, SipTraceConfig, SipTraceDirection, SrtpSuitePolicy, StreamPeer,
        StreamPeerBuilder, SubscriptionState, TransferDialogMatcher, TransferKind,
        TransferLifecycleOptions, TransferOutcome, TransferTargetEvidence, TransferWaitMode,
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
