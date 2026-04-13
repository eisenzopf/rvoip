//! # Session Core v3 API
//!
//! A state-machine driven SIP session management library for building clients,
//! servers, proxies, and call center software.
//!
//! ## Two API Styles
//!
//! | API | Best for | Style |
//! |-----|----------|-------|
//! | [`StreamPeer`] | Clients, scripts, tests | Sequential — call methods, await results |
//! | [`CallbackPeer`] | Servers, proxies, IVR | Reactive — implement [`CallHandler`] trait |
//!
//! ## Quick Start — Making a Call
//!
//! ```rust,no_run
//! use rvoip_session_core_v3::*;
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
//! ## Quick Start — Receiving a Call
//!
//! ```rust,no_run
//! use rvoip_session_core_v3::*;
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
//! ## Call Features
//!
//! [`SessionHandle`] provides hold, resume, transfer, DTMF, and audio:
//!
//! ```rust,no_run
//! # use rvoip_session_core_v3::*;
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
//! ## Server with CallbackPeer
//!
//! For servers, implement [`CallHandler`] or use a built-in handler:
//!
//! ```rust,no_run
//! use rvoip_session_core_v3::*;
//! use rvoip_session_core_v3::api::handlers::{RoutingHandler, RoutingAction};
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
//! ## Custom Configuration
//!
//! Use [`StreamPeer::builder()`] or [`Config`] directly:
//!
//! ```rust,no_run
//! use rvoip_session_core_v3::*;
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
//! - [`stream_peer`] — Sequential SIP peer for clients and scripts
//! - [`callback_peer`] — Reactive SIP peer for servers and proxies
//! - [`handlers`] — Built-in [`CallHandler`] implementations
//! - [`handle`] — [`SessionHandle`] for controlling active calls
//! - [`incoming`] — [`IncomingCall`] and [`IncomingCallGuard`]
//! - [`audio`] — [`AudioStream`], [`AudioSender`], [`AudioReceiver`]
//! - [`events`] — [`Event`] enum for session lifecycle events
//! - [`unified`] — [`UnifiedCoordinator`] and [`Config`]
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

// Core modules only
pub mod types;      // Core types (legacy)
pub mod events;     // Event-driven API for v3
pub mod unified;    // Unified API
pub mod builder;    // Session builder
pub mod simple;     // Simple peer API (legacy — use StreamPeer instead)

// New v3 API modules
pub mod audio;          // AudioStream, AudioSender, AudioReceiver
pub mod handle;         // SessionHandle, CallId
pub mod incoming;       // IncomingCall, IncomingCallGuard
pub mod stream_peer;    // StreamPeer, PeerControl, EventReceiver, StreamPeerBuilder
pub mod callback_peer;  // CallbackPeer, CallHandler, CallHandlerDecision, EndReason
pub mod handlers;       // Built-in CallHandler impls: AutoAnswerHandler, RejectAllHandler, etc.

// Re-export the main types
pub use types::{
    SessionId, CallSession, CallDecision,
    SessionStats, MediaInfo, AudioStreamConfig,
    parse_sdp_connection, SdpInfo,
};
// IncomingCall from types (data-only, legacy) is NOT re-exported here to avoid
// clash with the new IncomingCall in `incoming`. Use `api::types::IncomingCall` if needed.
pub use crate::types::CallState;

// Re-export the unified API
pub use unified::{UnifiedCoordinator, Config, RegistrationHandle};

// Re-export the simple API (legacy)
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

// ── New public API surface ──────────────────────────────────────────────────

// Audio
pub use audio::{AudioStream, AudioSender, AudioReceiver};

// SessionHandle
pub use handle::SessionHandle;

// IncomingCall / IncomingCallGuard
pub use incoming::{IncomingCall, IncomingCallGuard};

// StreamPeer (replaces SimplePeer for new code)
pub use stream_peer::{StreamPeer, PeerControl, EventReceiver};

// CallbackPeer (reactive server-style)
pub use callback_peer::{CallbackPeer, CallHandler, CallHandlerDecision, EndReason};