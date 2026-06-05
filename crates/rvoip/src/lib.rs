//! # rvoip — Universal real-time gateway library
//!
//! `rvoip` is the facade crate for the rvoip workspace. It always compiles the
//! voip-3 substrate (`rvoip-core` + `rvoip-core-traits` — the cross-transport
//! `Orchestrator` and the `Conversation`/`Session`/`Connection`/`Stream`/
//! `Message`/`Participant` model) and lets you opt into transports and
//! extensions behind cargo features, defaulting to the SIP product.
//!
//! ## Maturity tiers
//!
//! Versions are plain numeric (no `-alpha`/`-beta` suffixes): **`0.1.x` = alpha,
//! `0.2.x` = beta, `1.0` = stable**. The `sip` surface is beta; the other
//! surfaces (`webrtc`, `uctp`, the `voip-3` extensions, `client`) are alpha.
//!
//! See `docs/PRD.md`, `INTERFACE_DESIGN.md`, and `CONVERSATION_PROTOCOL.md`
//! for the architectural context.
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
//! | Feature | Default | Pulls in |
//! |---|:---:|---|
//! | `sip` | ✅ | SIP interop adapter (`rvoip::sip`) — **beta** |
//! | `webrtc` | | WebRTC interop adapter (`rvoip::webrtc`) — alpha |
//! | `uctp` | | UCTP substrate adapters — QUIC / WebTransport / WebSocket (`rvoip::uctp`) — alpha |
//! | `sip-stir-shaken` | | RFC 8224 caller-ID attestation; requires `sip` (`rvoip::stir_shaken`) — alpha |
//! | `voip-3` | | The full experience: every transport **+** vCon / identity / AI-harness extensions — alpha |
//! | `client` | | Cross-transport client SDK (`rvoip::client`) — alpha |
//! | `full` | | `voip-3` + `sip-stir-shaken` + `client` |
//!
//! The `vcon`, `identity`, and `harness` conversation-model extensions are
//! transport-agnostic and reachable **only** through the `voip-3` feature.
//!
//! ## Module layout
//!
//! The unifying voip-3 nouns are re-exported at the crate root via
//! `rvoip::core_traits`; the `Orchestrator` + `Config` at the root directly.
//! Each transport/extension lives under its own feature-gated module
//! (`rvoip::sip`, `rvoip::webrtc`, `rvoip::uctp`, `rvoip::client`, …).

#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

// ---------------------------------------------------------------------------
// Always compiled: voip-3 spine (Orchestrator + nouns)
// ---------------------------------------------------------------------------

// The implementation crate. Always pulled in (the facade depends on it
// directly per `[bans.wrappers]` in workspace `deny.toml`).
pub use rvoip_core::{Config, Orchestrator};

// The shared trait / data surface. Adapter crates depend on this rather than
// on `rvoip-core` to avoid pulling in the orchestrator implementation.
pub use rvoip_core_traits as core_traits;

// ---------------------------------------------------------------------------
// SIP (beta)
// ---------------------------------------------------------------------------

/// SIP interop adapter — bridges SIP/RTP into the voip-3 `Session`
/// abstraction. See `rvoip-sip` for the full surface.
#[cfg(feature = "sip")]
pub mod sip {
    pub use rvoip_sip::*;
}

/// STIR/SHAKEN (RFC 8224) caller-ID attestation for SIP — `PASSporT`
/// signing/verification plugged into the SIP dialog layer. SIP-only;
/// enabled by the `sip-stir-shaken` feature (which implies `sip`).
#[cfg(feature = "sip-stir-shaken")]
pub mod stir_shaken {
    pub use rvoip_stir_shaken::*;
}

// ---------------------------------------------------------------------------
// WebRTC (alpha)
// ---------------------------------------------------------------------------

/// WebRTC interop adapter — bridges DTLS-SRTP / ICE peers into the voip-3
/// `Session` abstraction. Off by default; enable the `webrtc` feature.
#[cfg(feature = "webrtc")]
pub mod webrtc {
    pub use rvoip_webrtc::*;
}

// ---------------------------------------------------------------------------
// UCTP substrates (alpha)
// ---------------------------------------------------------------------------

/// UCTP substrate adapters and protocol primitives. Per
/// `CONVERSATION_PROTOCOL.md` §4, UCTP runs over QUIC, WebTransport, and
/// WebSocket substrates; this module re-exports all three plus the
/// wire-level protocol from `rvoip-uctp`. Enable the `uctp` feature.
#[cfg(feature = "uctp")]
pub mod uctp {
    /// UCTP-over-QUIC substrate adapter.
    pub use rvoip_quic as quic;
    /// Envelope encode/decode, capability negotiation, session state machine.
    pub use rvoip_uctp as protocol;
    /// UCTP-over-WebSocket substrate adapter.
    pub use rvoip_websocket as websocket;
    /// UCTP-over-WebTransport substrate adapter.
    pub use rvoip_webtransport as webtransport;
}

// ---------------------------------------------------------------------------
// voip-3 conversation-model extensions (alpha) — reachable only via `voip-3`
// ---------------------------------------------------------------------------

/// vCon (IETF Virtualized Conversations) container builder, signer, and store —
/// emitted per Session regardless of transport. Part of the `voip-3` feature.
#[cfg(feature = "voip-3")]
pub mod vcon {
    pub use rvoip_vcon::*;
}

/// `IdentityProvider` backends — bearer, OAuth 2.1 + DPoP, OIDC, passkeys,
/// SIP Digest, AAuth. Transport-agnostic; part of the `voip-3` feature.
#[cfg(feature = "voip-3")]
pub mod identity {
    pub use rvoip_identity::*;
}

/// AI voice harness — in-process ASR / TTS / Dialog runtime that attaches to a
/// `Connection` via the orchestrator. Part of the `voip-3` feature.
#[cfg(feature = "voip-3")]
pub mod harness {
    pub use rvoip_harness::*;
}

// ---------------------------------------------------------------------------
// Client SDK (alpha)
// ---------------------------------------------------------------------------

/// Client-side API for mobile / web / desktop / embedded apps, wrapping the
/// SIP / WebRTC / UCTP transports behind one surface. See `rvoip-client`.
#[cfg(feature = "client")]
pub mod client {
    pub use rvoip_client::*;
}

/// The version of the rvoip facade crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
