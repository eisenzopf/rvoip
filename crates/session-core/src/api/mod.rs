//! # Session Core API
//!
//! Public developer interfaces for building SIP applications on top of
//! `session-core`.
//!
//! This module is organized around three API surfaces:
//!
//! | API | Best for | Style |
//! | --- | --- | --- |
//! | [`StreamPeer`] | Clients, softphones, scripts, tests | Sequential helpers and typed events |
//! | [`CallbackPeer`] | Servers, IVR, routing endpoints | Reactive [`CallHandler`] hooks |
//! | [`UnifiedCoordinator`] | B2BUAs, gateways, custom frameworks | Explicit session IDs and orchestration methods |
//!
//! All three surfaces drive the same coordinator, state table, dialog adapter,
//! and media adapter. Choosing a surface is mostly about how your application
//! wants to structure control flow.
//!
//! ## Common Building Blocks
//!
//! - [`Config`] configures SIP transports, contact behavior, TLS, SRTP,
//!   registration, NAT/media address advertisement, session timers, 100rel,
//!   and codec negotiation policy.
//! - [`SessionHandle`] controls a single call once it exists.
//! - [`IncomingCall`] represents a ringing inbound INVITE that must be accepted,
//!   rejected, redirected, or deferred.
//! - [`Event`] is the typed application event enum used by `StreamPeer` and
//!   lower-level coordinator subscribers.
//! - [`Registration`] describes outbound SIP REGISTER attempts.
//!
//! ## StreamPeer: Making a Call
//!
//! ```rust,no_run
//! use rvoip_session_core::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let mut peer = StreamPeer::new("alice").await?;
//!     let handle = peer.call("sip:bob@192.168.1.100:5060").await?;
//!
//!     // Wait for the remote side to answer
//!     peer.wait_for_answered(handle.id()).await?;
//!     tokio::time::sleep(std::time::Duration::from_secs(10)).await;
//!     handle.hangup().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## StreamPeer: Receiving a Call
//!
//! ```rust,no_run
//! use rvoip_session_core::*;
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
//! [`SessionHandle`] provides hold, resume, transfer, DTMF, and audio:
//!
//! ```rust,no_run
//! # use rvoip_session_core::*;
//! # async fn example(handle: SessionHandle) -> Result<()> {
//! handle.hold().await?;
//! handle.resume().await?;
//! handle.send_dtmf('1').await?;
//! handle.transfer_blind("sip:charlie@example.com").await?;
//!
//! let audio = handle.audio().await?;
//! let (sender, receiver) = audio.split();
//! # Ok(())
//! # }
//! ```
//!
//! ## CallbackPeer: Reactive Server
//!
//! For servers, implement [`CallHandler`] or use a built-in handler:
//!
//! ```rust,no_run
//! use rvoip_session_core::*;
//! use rvoip_session_core::api::handlers::{RoutingHandler, RoutingAction};
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
//! application runtime, bridge legs, or subscribe to filtered event streams:
//!
//! ```rust,no_run
//! use rvoip_session_core::{Config, Event, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("app", 5060)).await?;
//! let mut events = coordinator.events().await?;
//!
//! let call_id = coordinator
//!     .make_call("sip:app@127.0.0.1:5060", "sip:bob@127.0.0.1:5070")
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
//! ## Custom Configuration
//!
//! Use [`StreamPeer::builder()`] or [`Config`] directly:
//!
//! ```rust,no_run
//! use rvoip_session_core::*;
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

// Core modules only
pub mod builder; // Session builder
pub mod events; // Event-driven API for v3
pub mod simple;
pub mod types; // Core types (legacy)
pub mod unified; // Unified API // Simple peer API (legacy — use StreamPeer instead)

// New v3 API modules
pub mod audio; // AudioStream, AudioSender, AudioReceiver
pub mod callback_peer; // CallbackPeer, CallHandler, CallHandlerDecision, EndReason
pub mod handle; // SessionHandle, CallId
pub mod handlers;
pub mod incoming; // IncomingCall, IncomingCallGuard
pub mod stream_peer; // StreamPeer, PeerControl, EventReceiver, StreamPeerBuilder // Built-in CallHandler impls: AutoAnswerHandler, RejectAllHandler, etc.

// Re-export the main types
pub use types::{
    parse_sdp_connection, AudioStreamConfig, CallDecision, CallSession, MediaInfo, SdpInfo,
    SessionId, SessionStats,
};
// IncomingCall from types (data-only, legacy) is NOT re-exported here to avoid
// clash with the new IncomingCall in `incoming`. Use `api::types::IncomingCall` if needed.
pub use crate::types::CallState;

// Re-export the unified API
pub use unified::{Config, RegistrationHandle, SipContactMode, SipTlsMode, UnifiedCoordinator};

// Re-export the simple API (legacy)
pub use simple::SimplePeer;

// Re-export event types
pub use events::{CallHandle, CallId, Event};

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
pub use handle::SessionHandle;

// DialogIdentity (used when orchestrating attended transfer from a higher layer)
pub use types::DialogIdentity;

// IncomingCall / IncomingCallGuard
pub use incoming::{IncomingCall, IncomingCallGuard};

// StreamPeer (replaces SimplePeer for new code)
pub use stream_peer::{EventReceiver, PeerControl, StreamPeer};

// CallbackPeer (reactive server-style)
pub use callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, CallbackPeerControl, EndReason,
};
