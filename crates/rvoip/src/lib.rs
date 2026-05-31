//! # rvoip — Universal real-time gateway library
//!
//! `rvoip` is the facade crate for the rvoip workspace. It bundles
//! the voip-3 substrate (`rvoip-core`, `rvoip-core-traits`), the
//! Universal Conversation Transport Protocol (UCTP) family of
//! substrates, the SIP and WebRTC interop adapters, the AI voice
//! harness, and the vCon container builder — feature-gated so
//! consumers pull only what they need.
//!
//! See `crates/rvoip-core/PRD.md`, `INTERFACE_DESIGN.md`, and
//! `CONVERSATION_PROTOCOL.md` for the architectural context.
//!
//! ## Quick start
//!
//! ```no_run
//! use rvoip::{Orchestrator, Config};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // `Orchestrator::new` returns an `Arc<Orchestrator>`.
//! let orchestrator = Orchestrator::new(Config::default());
//!
//! // Register interop adapters (e.g. `rvoip::sip::SipAdapter::new(coordinator).await?`,
//! // built from a configured `UnifiedCoordinator`) via `orchestrator.register(adapter)?`.
//!
//! let mut events = orchestrator.subscribe_events();
//! while let Ok(event) = events.recv().await {
//!     // handle each orchestrator event
//!     drop(event);
//! }
//! # Ok(()) }
//! ```
//!
//! ## Cargo features
//!
//! Per `INTERFACE_DESIGN.md` §2.2:
//!
//! | Feature | Pulls in |
//! |---|---|
//! | `uctp` (default) | UCTP substrate adapters: quic, webtransport, websocket |
//! | `sip` (default) | SIP interop adapter |
//! | `webrtc` | WebRTC interop adapter (off by default) |
//! | `vcon` (default) | vCon container builder + signing |
//! | `identity` (default) | Identity provider backends |
//! | `harness` | In-process AI voice harness |
//! | `client` | Client-side API (`rvoip-client`) |
//! | `aauth-experimental` | AAuth identity backend (experimental) |
//! | `identity-fingerprint-binding` | DTLS-SRTP fingerprint binding |
//! | `full` | All of the above |
//!
//! ## Module layout
//!
//! Per INTERFACE_DESIGN §15.3, per-protocol native surfaces live at
//! `rvoip::sip`, `rvoip::webrtc`, `rvoip::uctp`. The unifying voip-3
//! nouns (`Conversation`, `Session`, `Connection`, `Stream`,
//! `Message`, `Participant`) are re-exported at the crate root from
//! `rvoip-core-traits`.

#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

// ---------------------------------------------------------------------------
// Top-level: voip-3 nouns + Orchestrator from rvoip-core / rvoip-core-traits
// ---------------------------------------------------------------------------

// The implementation crate. Always pulled in (the facade depends on
// it directly per `[bans.wrappers]` in workspace `deny.toml`).
pub use rvoip_core::{Config, Orchestrator};

// The shared trait / data surface. Adapter crates depend on this
// rather than on `rvoip-core` to avoid pulling in the orchestrator
// implementation.
pub use rvoip_core_traits as core_traits;

// ---------------------------------------------------------------------------
// SIP interop adapter
// ---------------------------------------------------------------------------

/// SIP interop adapter — bridges SIP/RTP into the UCTP `Session`
/// abstraction. See `rvoip-sip` for the full surface.
#[cfg(feature = "sip")]
pub mod sip {
    pub use rvoip_sip::*;
}

// ---------------------------------------------------------------------------
// WebRTC interop adapter
// ---------------------------------------------------------------------------

/// WebRTC interop adapter — bridges DTLS-SRTP / ICE peers into the
/// UCTP `Session` abstraction. Off by default; enable the `webrtc`
/// feature.
#[cfg(feature = "webrtc")]
pub mod webrtc {
    pub use rvoip_webrtc::*;
}

// ---------------------------------------------------------------------------
// UCTP substrates (umbrella module re-exporting all three substrate adapters)
// ---------------------------------------------------------------------------

/// UCTP substrate adapters and protocol primitives.
///
/// Per `CONVERSATION_PROTOCOL.md` §4, UCTP runs over QUIC,
/// WebTransport, and WebSocket substrates. Each substrate has its
/// own adapter crate; this module re-exports all three plus the
/// wire-level protocol from `rvoip-uctp`.
#[cfg(feature = "uctp")]
pub mod uctp {
    /// Envelope encode/decode, capability negotiation, session state
    /// machine — protocol-level types shared across substrates.
    pub use rvoip_uctp as protocol;

    /// UCTP-over-QUIC substrate adapter.
    pub use rvoip_quic as quic;

    /// UCTP-over-WebTransport substrate adapter.
    pub use rvoip_webtransport as webtransport;

    /// UCTP-over-WebSocket substrate adapter (signaling on WebSocket,
    /// media via co-located WebRTC PeerConnection per
    /// `CONVERSATION_PROTOCOL.md` §4.3).
    pub use rvoip_websocket as websocket;
}

// ---------------------------------------------------------------------------
// AI voice harness
// ---------------------------------------------------------------------------

/// AI voice harness — in-process ASR / TTS / Dialog runtime that
/// attaches to a `Connection` via `Orchestrator::attach_ai`. See
/// `rvoip-harness`.
#[cfg(feature = "harness")]
pub mod harness {
    pub use rvoip_harness::*;
}

// ---------------------------------------------------------------------------
// vCon container builder + store
// ---------------------------------------------------------------------------

/// vCon (IETF Virtualized Conversations) container builder, signer,
/// and store. Emitted at end-of-Session per
/// `INTERFACE_DESIGN.md` §3.9.
#[cfg(feature = "vcon")]
pub mod vcon {
    pub use rvoip_vcon::*;
}

// ---------------------------------------------------------------------------
// Identity provider backends
// ---------------------------------------------------------------------------

/// `IdentityProvider` backends — bearer, OAuth 2.1 + DPoP, OIDC,
/// passkeys, AAuth. See `rvoip-identity`.
#[cfg(feature = "identity")]
pub mod identity {
    pub use rvoip_identity::*;
}

// ---------------------------------------------------------------------------
// Client SDK (separate crate, created by P12.3)
// ---------------------------------------------------------------------------
//
// When the `client` feature is enabled, the client-side `Client`
// surface from `rvoip-client` becomes available as `rvoip::client::*`.
// The crate is created by P12.3; until then this module is a stub
// behind the feature flag.

/// Client-side API for mobile / web / desktop / embedded apps. See
/// `rvoip-client`.
#[cfg(feature = "client")]
pub mod client {
    pub use rvoip_client::*;
}

/// The version of the rvoip facade crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
