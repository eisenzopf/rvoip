//! # rvoip-sip
//!
//! Application-facing SIP session orchestration for Rust VoIP applications.
//!
//! `rvoip-sip` sits above the lower-level SIP dialog and media crates. It
//! owns call/session state, registration state, SIP feature orchestration, and
//! the public control surfaces that applications use to build softphones,
//! test clients, IVRs, B2BUA legs, routing servers, and PBX/SBC interop tools.
//!
//! ## Where it fits in the workspace
//!
//! - [`rvoip_sip_dialog`] — RFC 3261 dialog/transaction layer
//!   ([`Dialog`](rvoip_sip_dialog::Dialog),
//!   [`DialogId`](rvoip_sip_dialog::DialogId),
//!   [`DialogManager`](rvoip_sip_dialog::DialogManager))
//! - [`rvoip_sip_core`] — SIP message parser and builder
//!   ([`Message`](rvoip_sip_core::Message),
//!   [`Request`](rvoip_sip_core::Request),
//!   [`Response`](rvoip_sip_core::Response),
//!   [`Uri`](rvoip_sip_core::Uri))
//! - [`rvoip_sip_registrar`] — registrar/location service
//!   ([`Registrar`](rvoip_sip_registrar::Registrar),
//!   [`RegistrarService`](rvoip_sip_registrar::RegistrarService))
//! - [`rvoip_media_core`] — codecs, media sessions, audio processing
//!   ([`MediaSession`](rvoip_media_core::MediaSession),
//!   [`MediaEngine`](rvoip_media_core::MediaEngine))
//! - [`rvoip_rtp_core`] — RTP/SRTP transport
//!   ([`RtpSession`](rvoip_rtp_core::RtpSession),
//!   [`RtpPacket`](rvoip_rtp_core::RtpPacket))
//! - [`rvoip_core`] — transport-agnostic orchestrator
//!   ([`Orchestrator`](rvoip_core::Orchestrator),
//!   [`ConnectionAdapter`](rvoip_core::ConnectionAdapter))
//!
//! This crate is the application seam; the layers above resolve into one of
//! [`Endpoint`], [`StreamPeer`], [`CallbackPeer`], or [`UnifiedCoordinator`]
//! depending on how much orchestration the caller wants to own.
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
//! let call = endpoint.call_and_wait("1002", Some(Duration::from_secs(30))).await?;
//! call.hangup().await?;
//! endpoint.shutdown().await?;
//! # Ok(())
//! # }
//! ```
//!
//! Runnable example: `cargo run -p rvoip-sip --example endpoint_local_call`
//! (`examples/endpoint/01_local_call/main.rs`).
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
//! let call_id = alice.invite("sip:bob@192.168.1.50:5060").send().await?;
//! let call = alice.coordinator().session(&call_id);
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
//! Runnable example: `cargo run -p rvoip-sip --example stream_peer_basic_call`
//! (`examples/stream_peer/01_basic_call/main.rs`).
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
//! Runnable example:
//! `cargo run -p rvoip-sip --example callback_peer_auto_answer_server`
//! (`examples/callback_peer/01_auto_answer/server.rs`). Built-in handlers
//! ([`AutoAnswerHandler`], [`RoutingHandler`], [`QueueHandler`]) each have
//! their own numbered scenario under `examples/callback_peer/`.
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
//!     .invite(Some("sip:bridge@127.0.0.1:5060".to_string()), "sip:bob@127.0.0.1:5070")
//!     .send()
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
//! Runnable example: `cargo run -p rvoip-sip --example unified_basic_call`
//! (`examples/unified/01_basic_call/main.rs`). The
//! `examples/unified/04_b2bua_bridge/` scenario demonstrates a three-party
//! bridge built directly on the coordinator.
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
//! ## Cross-transport via [`rvoip_core::Orchestrator`] + [`SipAdapter`]
//!
//! [`SipAdapter`] implements [`rvoip_core::ConnectionAdapter`], so SIP plugs
//! into the cross-transport [`rvoip_core::Orchestrator`] alongside future
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
//! ## Custom INVITE headers
//!
//! All API surfaces share one builder: call `invite(from, to)` (or the
//! peer-scoped `invite(to)`), then attach `.with_extra_headers(...)` to
//! ship a `Vec<`[`TypedHeader`]`>` with the outgoing INVITE. Use this for
//! headers RFC 3261 leaves outside the request line — `Diversion`,
//! `History-Info`, `Call-Info`, `User-to-User`, or vendor `X-*` headers
//! required by a specific PBX or SBC. The extras append after any
//! synthesized `P-Asserted-Identity` and before the outbound-proxy Route,
//! so the on-wire ordering is deterministic. [`HeaderName`] and [`TypedHeader`] are re-exported at the
//! crate root for ergonomic authoring without pulling in `rvoip-sip-core`
//! directly.
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
//!
//! ## Gateway / B2BUA / SBC Authoring
//!
//! For applications that need to inspect every inbound SIP field, author
//! arbitrary outbound headers, and compose inbound/outbound legs across
//! trust boundaries (B2BUAs, SBCs, gateways, call-center frontends),
//! `rvoip-sip` provides a uniform builder-shaped request/response API
//! introduced by `SIP_API_DESIGN_2.md`. The four cornerstones:
//!
//! - [`api::headers::SipHeaderView`] — inbound-header inspection,
//!   implemented by every `IncomingCall` / `IncomingRequest` /
//!   [`api::incoming::IncomingResponse`] / `IncomingRegister`. Generic
//!   over the wrapper so carry-through code can write
//!   `with_headers_from(&inbound, &[...])` once.
//! - [`api::headers::SipRequestOptions`] — outbound and response
//!   builder shape. Every builder (`coord.invite(..).send()`,
//!   `coord.refer(..).send()`, `coord.accept(..).send()`, …)
//!   implements it. In-dialog builders are also reachable directly
//!   on [`api::handle::SessionHandle`] —
//!   `session.bye().send()`, `session.refer(target).send()`, etc. —
//!   so application code that already holds a session doesn't need
//!   to reach back through the coordinator.
//! - [`api::headers::policy`] — layer-boundary enforcement that
//!   classifies every header for every method into
//!   `StackManaged` / `MethodShaped` / `ApplicationControlled` so the
//!   dialog state machine remains authoritative.
//! - [`api::headers::convenience`] — typed constructors for headers
//!   without a first-class `TypedHeader` variant in sip-core
//!   (`Diversion`, `History-Info`, `Replaces`, `Target-Dialog`,
//!   `Session-Expires`, `Min-SE`, `P-Charging-Vector`,
//!   `P-Called-Party-ID`) plus body factories (`sdp`, `dtmf_relay`,
//!   `pidf_xml`, multipart construction/parsing).
//!
//! ### Decision chart
//!
//! | If you say… | Use | Example |
//! |---|---|---|
//! | "I just want to make a call, library handles SIP" | Pure Config | `coord.invite(None, target).send()` |
//! | "I need credentials on outbound calls" | One builder | `coord.invite(from, to).with_credentials(c).send()` |
//! | "I need to attach one custom X-* header" | One builder | `coord.invite(from, to).with_raw_header("X-Foo", "bar")?.send()` |
//! | "I'm building a B2BUA — carry headers across legs" | Builder + carry-through | `coord.invite(...).with_headers_from(&inbound, &[...])?.send()` |
//! | "I need lenient validation for messy upstream" | `with_strictness(Lenient)` | `coord.invite(...).with_strictness(BuilderStrictness::Lenient).send()` |
//! | "I need to inspect every inbound header" | [`api::headers::SipHeaderView`] | `incoming.header(&HeaderName::Diversion)` |
//! | "I'm authoring custom 4xx with Retry-After" | `RejectBuilder` | `incoming.reject_builder().with_status(503).with_retry_after(120).send()` |
//! | "I'm a registrar with Service-Route on 200 OK" | `RegisterResponseBuilder` | `incoming.accept_builder().with_service_route(routes).with_path_echo().send()` |
//!
//! ### B2BUA composition (the litmus test)
//!
//! ```rust,no_run
//! # use std::sync::Arc;
//! # use rvoip_sip::{UnifiedCoordinator, IncomingCall};
//! # use rvoip_sip_core::types::headers::HeaderName;
//! # async fn example(coord: Arc<UnifiedCoordinator>, incoming: IncomingCall) -> rvoip_sip::Result<()> {
//! use rvoip_sip::api::headers::{SipHeaderView, SipRequestOptions};
//!
//! // Inspect inbound
//! let _original_pai = incoming.header(&HeaderName::Other("P-Asserted-Identity".into()));
//! let _history = incoming.headers_named(&HeaderName::Other("History-Info".into()));
//!
//! // Build outbound leg — every with_* returns Result; `?` chains cleanly
//! let upstream_target = "sip:bob@upstream.example";
//! let (outbound, _report) = coord
//!     .invite(None, upstream_target)
//!     .with_headers_from(&incoming, &[
//!         HeaderName::Other("History-Info".into()),
//!         HeaderName::Other("Diversion".into()),
//!     ])?;
//! let outbound = outbound
//!     .strip_header(&HeaderName::Other("Privacy".into()))
//!     .with_raw_header(
//!         HeaderName::Other("P-Asserted-Identity".into()),
//!         "sip:+15551234@gw.local",
//!     )?;
//!
//! let _session = outbound.send().await?;
//! # Ok(()) }
//! ```
//!
//! ### Trust-boundary patterns
//!
//! Three canonical postures cover most B2BUA / SBC use cases:
//!
//! 1. **Trusted → untrusted egress.** Strip identity headers, keep
//!    routing breadcrumbs only when regulator-mandated.
//! 2. **Untrusted → trusted ingress.** Assert identity from local
//!    AAA, ignore inbound PAI entirely.
//! 3. **Trusted-to-trusted (intra-domain).** Carry through verbatim
//!    so the downstream peer sees the upstream's headers.
//!
//! All three are illustrated in `SIP_API_DESIGN_2.md` §11.3; the
//! [`api::headers::policy::forbidden_for_carry_through`] guard
//! ensures none of them can accidentally leak `Via` / `Call-ID` /
//! `CSeq` / `Max-Forwards` to the downstream wire — topology hiding
//! is automatic.
//!
//! ### Header classification reference
//!
//! - **StackManaged**: `Call-ID`, `CSeq`, `Via`, `Max-Forwards`,
//!   `Content-Length`, `Record-Route`, `Route`. Hard-rejected at
//!   `with_header()` time regardless of `BuilderStrictness`.
//! - **MethodShaped**: `Contact` (initial INVITE/REGISTER/SUBSCRIBE),
//!   `Authorization` (UAC requests with `with_credentials`),
//!   `Expires` (REGISTER/SUBSCRIBE), `Refer-To` (REFER), `Event` /
//!   `Subscription-State` (SUBSCRIBE/NOTIFY). Rejected under
//!   `BuilderStrictness::Strict`; downgraded to a `tracing::warn!`
//!   under `Lenient`.
//! - **ApplicationControlled**: `Diversion`, `History-Info`,
//!   `Referred-By`, `Replaces`, `P-Asserted-Identity`,
//!   `P-Preferred-Identity`, `Privacy`, `Reason`, `Retry-After`,
//!   `Warning`, `Subject`, `Date`, `User-Agent`, `Server`, `Accept`,
//!   `Allow`, `Supported`, `Require`, `Path`, `Service-Route`,
//!   `Reply-To`, `Target-Dialog`, `Session-Expires`, `Min-SE`, all
//!   `X-*`, and every `Other(_)` not listed above. Free to stage.
//!
//! See [`api::headers::policy::classify`] for the per-method matrix.

#![deny(rustdoc::bare_urls)]
#![deny(rustdoc::broken_intra_doc_links)]

// ── Internal modules (pub for doc visibility, use the re-exports below) ─────

pub mod adapter;
pub mod api;
pub mod errors;
/// D4 — `MediaStream` wrapper that bridges a SIP audio session into
/// `rvoip_core::ConnectionAdapter::streams`. See module docs.
pub mod media_stream;
pub mod server;

// These modules remain public for existing internal-style integrations, but
// they are hidden from generated docs. Application code should use the
// UnifiedCoordinator, StreamPeer, CallbackPeer, and SessionHandle surfaces.
#[doc(hidden)]
pub mod adapters;
#[doc(hidden)]
pub mod auth;
#[doc(hidden)]
pub mod cleanup_diag;
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
pub use api::headers::options::{
    BuilderHeaderState, BuilderStrictness, HeaderCarryThroughReport, HeaderPolicyViolation,
    SipRequestOptions, ViolationReason,
};
pub use api::headers::view::SipHeaderView;
pub use api::incoming::{
    IncomingCall, IncomingCallGuard, IncomingRegister, IncomingRequest, IncomingResponse,
};
pub use api::trace_redactor::{PassthroughRedactor, RedactionDecision, TraceRedactor};

/// SIP_API_DESIGN_2 §3.6 — convenience body constructors. Each
/// helper returns `(content_type, Bytes)` for attachment to a SIP
/// body via the new outbound builders. The §10 #24 `multipart_mixed`
/// / `multipart_parse` helpers live in
/// [`crate::api::headers::convenience`] and
/// are re-exported here for surface symmetry.
pub mod bodies {
    pub use crate::api::bodies::*;
    pub use crate::api::headers::convenience::{
        multipart_mixed, multipart_parse, MultipartParseError, MultipartPart,
    };
}
pub use api::lifecycle::{
    CallAnsweredInfo, CallLifecycleSnapshot, CallProgressInfo, CallTerminalInfo,
};

// Configuration & registration
pub use api::unified::{AudioSource, BridgeError, BridgeHandle, Registration, RelUsage};
pub use api::{
    Config, MediaMode, RegistrationHandle, RegistrationInfo, RegistrationStatus, SipContactMode,
    SipTlsMode, SrtpSuitePolicy, UnifiedCoordinator,
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

// SIP header authoring (re-exported from rvoip-sip-core so callers using
// the `_with_headers` outbound-INVITE variants can construct typed headers
// without importing the lower-level crate directly).
pub use rvoip_sip_core::types::{HeaderName, TypedHeader};

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
        EventReceiver, HeaderName, IncomingCall, IncomingCallGuard, MediaMode, MediaSecurityKeying,
        MediaSecurityProfile, MediaSecurityState, PeerControl, Registration, RegistrationHandle,
        RegistrationInfo, RegistrationStatus, Result, SessionError, SessionHandle, SipContactMode,
        SipReason, SipTlsMode, SipTrace, SipTraceConfig, SipTraceDirection, SrtpSuitePolicy,
        StreamPeer, StreamPeerBuilder, SubscriptionState, TransferDialogMatcher, TransferKind,
        TransferLifecycleOptions, TransferOutcome, TransferTargetEvidence, TransferWaitMode,
        TypedHeader,
    };
}

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
