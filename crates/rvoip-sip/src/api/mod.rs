//! # rvoip-sip API
//!
//! Public developer interfaces for building SIP applications on top of
//! `rvoip-sip`.
//!
//! This module is organized around four API surfaces:
//!
//! | API | Best for | Style |
//! | --- | --- | --- |
//! | [`Endpoint`] | Softphones, PBX accounts, simple demos | Account/profile builder plus call helpers |
//! | [`StreamPeer`] | Clients, softphones, scripts, tests | Sequential helpers and typed events |
//! | [`CallbackPeer`] | Servers, IVR, routing endpoints | Closure builder or reactive [`CallHandler`] hooks |
//! | [`UnifiedCoordinator`] | B2BUAs, gateways, custom frameworks | Explicit session IDs and orchestration methods |
//!
//! All three surfaces drive the same session coordinator and signaling/media
//! runtime. Choosing a surface is mostly about how your application wants to
//! structure control flow; applications should not need lower-layer dialog
//! plumbing directly.
//!
//! ## Common Building Blocks
//!
//! - [`Config`] configures SIP transports, contact behavior, TLS, SRTP,
//!   registration refresh/unregister behavior, outbound proxy routing,
//!   NAT/media address advertisement, session timers, 100rel, and codec
//!   negotiation policy.
//! - [`SessionHandle`] controls a single call once it exists, including
//!   deterministic wait helpers for teardown, provisional progress, media
//!   security, and blind transfer.
//! - [`IncomingCall`] represents a ringing inbound INVITE that must be accepted,
//!   rejected, redirected, or deferred.
//! - [`Event`] is the typed application event enum used by `StreamPeer` and
//!   lower-level coordinator subscribers, with helper views for transfer kind,
//!   NOTIFY subscription state, provisional progress, REFER lifecycle, dialog
//!   package notifications, and SRTP media security.
//! - [`Registration`] describes outbound SIP REGISTER attempts; query
//!   [`RegistrationInfo`] for accepted expiry, refresh timing, GRUU, and
//!   Service-Route metadata.
//!
//! ## Endpoint: PBX Account or Softphone
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
//! let call = endpoint.call_and_wait("1002", Some(Duration::from_secs(30))).await?;
//! call.wait_for_answered(Some(Duration::from_secs(30))).await?;
//! call.hangup().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## StreamPeer: Making a Call
//!
//! ```rust,no_run
//! use rvoip_sip::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let mut peer = StreamPeer::new("alice").await?;
//!     let call_id = peer.invite("sip:bob@192.168.1.100:5060").send().await?;
//!     let handle = peer.coordinator().session(&call_id);
//!
//!     // Wait for the remote side to answer
//!     let handle = handle.wait_for_answered(Some(std::time::Duration::from_secs(30))).await?;
//!     tokio::time::sleep(std::time::Duration::from_secs(10)).await;
//!     handle.hangup_and_wait(Some(std::time::Duration::from_secs(5))).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## StreamPeer: Receiving a Call
//!
//! ```rust,no_run
//! use rvoip_sip::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let mut peer = StreamPeer::new("bob").await?;
//!     let incoming = peer.wait_for_incoming().await?;
//!     println!("Call from {}", incoming.from);
//!
//!     let handle = incoming.accept().await?;
//!     handle.wait_for_end(None).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Per-Call Control
//!
//! [`SessionHandle`] provides hold, resume, transfer, DTMF, audio, and
//! deterministic wait helpers:
//!
//! ```rust,no_run
//! # use rvoip_sip::*;
//! # async fn example(handle: SessionHandle) -> Result<()> {
//! handle.hold().await?;
//! handle.resume().await?;
//! handle.send_dtmf('1').await?;
//! handle
//!     .transfer_blind_and_wait_for_outcome(
//!         "sip:charlie@example.com",
//!         TransferWaitMode::NotifyFinal,
//!         None,
//!     )
//!     .await?;
//!
//! let audio = handle.audio().await?;
//! let (sender, receiver) = audio.split();
//! # Ok(())
//! # }
//! ```
//!
//! ## CallbackPeer: Reactive Server
//!
//! For common servers, use the closure builder:
//!
//! ```rust,no_run
//! use rvoip_sip::{CallHandlerDecision, CallbackPeer, Config, Result};
//!
//! # async fn example() -> Result<()> {
//! let peer = CallbackPeer::builder(Config::default())
//!     .on_incoming(|_call| async move { CallHandlerDecision::Accept })
//!     .on_dtmf(|call, digit| async move {
//!         println!("{} pressed {}", call.id(), digit);
//!         Ok(())
//!     })
//!     .build()
//!     .await?;
//! # let _ = peer;
//! # Ok(())
//! # }
//! ```
//!
//! For full control, implement [`CallHandler`] or use a built-in handler:
//!
//! ```rust,no_run
//! use rvoip_sip::*;
//! use rvoip_sip::api::handlers::{RoutingHandler, RoutingAction};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let handler = RoutingHandler::new()
//!         .with_rule("support@", RoutingAction::Accept)
//!         .with_rule("spam@", RoutingAction::Reject {
//!             status: 403,
//!             reason: "Forbidden".into(),
//!         });
//!
//!     let peer = CallbackPeer::new(handler, Config::default()).await?;
//!     peer.run().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## UnifiedCoordinator: Custom Orchestration
//!
//! Use the coordinator directly when you need to build a higher-level
//! application runtime, bridge legs, subscribe to filtered event streams, or
//! inspect registration lifecycle state:
//!
//! ```rust,no_run
//! use rvoip_sip::{Config, Event, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("app", 5060)).await?;
//! let mut events = coordinator.events().await?;
//!
//! let call_id = coordinator
//!     .invite(Some("sip:app@127.0.0.1:5060".to_string()), "sip:bob@127.0.0.1:5070")
//!     .send()
//!     .await?;
//! let mut call_events = coordinator.events_for_session(&call_id).await?;
//!
//! while let Some(event) = call_events.next().await {
//!     match event {
//!         Event::CallAnswered { .. } => coordinator.send_dtmf(&call_id, '1').await?,
//!         Event::CallEnded { .. } | Event::CallFailed { .. } => break,
//!         _ => {}
//!     }
//! }
//! # drop(events);
//! # Ok(())
//! # }
//! ```
//!
//! ## Registration Lifecycle
//!
//! Registration helpers return a [`RegistrationHandle`]. Use
//! [`UnifiedCoordinator::registration_info`] for richer lifecycle metadata and
//! [`UnifiedCoordinator::unregister_and_wait`] when tests or servers need
//! deterministic teardown:
//!
//! ```rust,no_run
//! use rvoip_sip::{Config, Registration, RegistrationStatus, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("alice", 5060)).await?;
//! let handle = coordinator
//!     .register("sip:registrar.example.com", "alice", "secret")
//!     .with_expires(600)
//!     .send()
//!     .await?;
//!
//! let info = coordinator.registration_info(&handle).await?;
//! if info.status == RegistrationStatus::Registered {
//!     println!("accepted expiry: {:?}", info.accepted_expires_secs);
//! }
//!
//! coordinator.unregister_and_wait(&handle, None).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Custom Configuration
//!
//! Use [`StreamPeer::builder()`] or [`Config`] directly:
//!
//! ```rust,no_run
//! use rvoip_sip::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Builder style
//!     let peer = StreamPeer::builder()
//!         .name("alice")
//!         .sip_port(5080)
//!         .local_ip("192.168.1.100".parse().unwrap())
//!         .media_ports(10000, 20000)
//!         .build()
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Module Structure
//!
//! - [`endpoint`] - simplified endpoint wrapper for softphones and PBX accounts.
//! - [`stream_peer`] - sequential SIP peer for clients and scripts.
//! - [`callback_peer`] - reactive SIP peer for servers and proxies.
//! - [`handlers`] - built-in [`CallHandler`] implementations.
//! - [`handle`] - [`SessionHandle`] for controlling active calls.
//! - [`incoming`] - [`IncomingCall`] and [`IncomingCallGuard`].
//! - [`audio`] - [`AudioStream`], [`AudioSender`], [`AudioReceiver`].
//! - [`events`] - [`Event`] enum for session lifecycle events.
//! - [`unified`] - [`UnifiedCoordinator`], [`Config`], and [`Registration`].
//!
//! [`StreamPeer`]: stream_peer::StreamPeer
//! [`Endpoint`]: endpoint::Endpoint
//! [`CallbackPeer`]: callback_peer::CallbackPeer
//! [`CallHandler`]: callback_peer::CallHandler
//! [`SessionHandle`]: handle::SessionHandle
//! [`IncomingCall`]: incoming::IncomingCall
//! [`IncomingCallGuard`]: incoming::IncomingCallGuard
//! [`AudioStream`]: audio::AudioStream
//! [`AudioSender`]: audio::AudioSender
//! [`AudioReceiver`]: audio::AudioReceiver
//! [`Event`]: events::Event
//! [`UnifiedCoordinator`]: unified::UnifiedCoordinator
//! [`Config`]: unified::Config
//! [`Registration`]: unified::Registration
//!
//! ## Rustdoc Policy
//!
//! Rustdoc is the canonical developer documentation for `rvoip-sip`.
//! Public developer-facing APIs in [`unified`], [`stream_peer`],
//! [`callback_peer`], [`handle`], [`incoming`], and [`audio`] are compiled with
//! `missing_docs` denied. Each public method should explain what it does, call
//! out observable events or important failure modes when relevant, and include
//! a focused `# Examples` section unless it is a trivial accessor or a
//! callback hook covered by the trait-level example.
//!
//! Examples should compile whenever practical. Use `rust,no_run` for SIP,
//! RTP, or network flows that need a peer at runtime, and ordinary `rust`
//! examples for pure builders or configuration helpers. Prefer intra-doc links
//! to item names so refactors break documentation at build time instead of
//! leaving stale text behind.

pub mod audio; // AudioStream, AudioSender, AudioReceiver
pub mod bodies; // SIP_API_DESIGN_2 §3.6 — Convenience body constructors
pub mod builder; // Session builder
pub mod callback_peer; // CallbackPeer, CallHandler, CallHandlerDecision, EndReason
pub mod dialog_package;
pub mod dialog_subscription;
pub mod endpoint;
pub mod events; // Event enum + supporting types
pub mod handle; // SessionHandle, CallId
pub mod handlers; // Built-in CallHandler impls: AutoAnswerHandler, RejectAllHandler, etc.
pub mod headers; // SipHeaderView, SipRequestOptions, HeaderPolicy (SIP_API_DESIGN_2)
pub mod incoming; // IncomingCall, IncomingCallGuard, IncomingRequest, IncomingResponse, IncomingRegister
pub mod lifecycle;
pub mod performance;
pub mod proxy_coordinator; // Stateful SIP proxy entry point (Phase 6)
pub mod respond; // Response builders (SIP_API_DESIGN_2 Phase D)
pub mod send; // Outbound builders (SIP_API_DESIGN_2 Phase C)
pub mod stream_peer; // StreamPeer, PeerControl, EventReceiver, StreamPeerBuilder
pub mod trace_redactor; // SIP_API_DESIGN_2 §12.4 — pluggable trace-output redaction
pub mod types; // Core data types shared across surfaces
pub mod unified; // UnifiedCoordinator (lowest-level surface)

// Re-export the main types
pub use types::{
    parse_sdp_connection, AudioStreamConfig, CallDecision, CallSession, MediaInfo, SdpInfo,
    SessionId, SessionStats,
};
// `api::types::IncomingCall` is a legacy data-only struct (no SIP control surface);
// the canonical type for application code is `incoming::IncomingCall`. The legacy
// type stays available via `api::types::IncomingCall` only to avoid a breaking
// re-export change for any out-of-tree consumer that still references it.
pub use crate::types::CallState;

// Re-export the unified API
pub use unified::{
    Config, MediaMode, RegistrationHandle, RegistrationInfo, RegistrationStatus, SipContactMode,
    SipTlsMode, SrtpSuitePolicy, UnifiedCoordinator,
};

// Re-export event types
pub use dialog_package::{DialogInfo, DialogInfoDocument, DialogPackageEvent, DialogPackageState};
pub use dialog_subscription::DialogSubscriptionHandle;
pub use events::{
    CallId, Event, MediaSecurityKeying, MediaSecurityProfile, MediaSecurityState, SipTrace,
    SipTraceConfig, SipTraceDirection, SubscriptionState, TransferKind, TransferTargetEvidence,
};

// Re-export builder
pub use builder::SessionBuilder;

// Re-export from state table for consistency
pub use crate::state_table::types::{EventType, Role};

// Error types
pub use crate::errors::{Result, SessionError};

// ── New public API surface ──────────────────────────────────────────────────

// Audio
pub use audio::{AudioReceiver, AudioSender, AudioStream};

// SessionHandle
pub use handle::{
    SessionHandle, SipReason, TransferDialogMatcher, TransferLifecycleOptions, TransferOutcome,
    TransferWaitMode,
};

// DialogIdentity (used when orchestrating attended transfer from a higher layer)
pub use types::DialogIdentity;

// IncomingCall / IncomingCallGuard / IncomingRequest / IncomingResponse / IncomingRegister
pub use incoming::{
    IncomingCall, IncomingCallGuard, IncomingRegister, IncomingRequest, IncomingResponse,
};

// Header view + builder trait surface (SIP_API_DESIGN_2)
pub use headers::{
    BuilderHeaderState, BuilderStrictness, HeaderCarryThroughReport, HeaderPolicyViolation,
    HeaderRole, MissingRequiredHeader, SipHeaderView, SipRequestOptions, ViolationReason,
};

// Outbound builders (SIP_API_DESIGN_2 Phase C)
pub use send::{
    ByeBuilder, CancelBuilder, InfoBuilder, MessageBuilder, NotifyBuilder, OptionsBuilder,
    OutboundCallBuilder, ReInviteBuilder, ReferBuilder, RegisterBuilder, RegisterRefreshBuilder,
    SubscribeBuilder, SubscribeRefreshBuilder, Surface, SurfaceBuilder, UpdateBuilder,
};

// Response builders (SIP_API_DESIGN_2 Phase D)
pub use lifecycle::{CallAnsweredInfo, CallLifecycleSnapshot, CallProgressInfo, CallTerminalInfo};
pub use respond::{
    AcceptBuilder, AuthChallengeBuilder, AuthScheme, GenericResponseBuilder, ProvisionalBuilder,
    RedirectBuilder, RegisterResponseBuilder, RejectBuilder,
};

// StreamPeer
pub use stream_peer::{EventReceiver, PeerControl, StreamPeer};

// CallbackPeer (reactive server-style)
pub use callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, CallbackPeerBuilder, CallbackPeerControl,
    EndReason,
};
pub use endpoint::{
    Endpoint, EndpointAccount, EndpointAccountConfig, EndpointAudio, EndpointAudioFrame,
    EndpointAudioReceiver, EndpointAudioSender, EndpointBuilder, EndpointCall, EndpointCallId,
    EndpointConfig, EndpointControl, EndpointEvent, EndpointEvents, EndpointIncomingCall,
    EndpointMediaConfig, EndpointNetworkConfig, EndpointProfile, EndpointProfileName,
    EndpointRegistrationInfo, EndpointRegistrationStatus, EndpointSipTrace, EndpointSrtpMode,
    EndpointTransport,
};
pub use performance::{PerformanceConfig, PerformanceRecipe, PerformanceRecipeBook};
