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
//! All workspace crates are aligned on one release version. Maturity is
//! product-specific rather than encoded in the version number: the `sip`
//! surface is beta-qualified, while `webrtc`, `uctp`, the `voip-3`
//! extensions, and `client` are available as developer previews.
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
//! | `webrtc` | | WebRTC interop adapter (`rvoip::webrtc`) — developer preview |
//! | `uctp` | | UCTP substrate adapters — QUIC / WebTransport / WebSocket (`rvoip::uctp`) — developer preview |
//! | `vapi` | | Vapi bidirectional WebSocket agent adapter (`rvoip::vapi`) — developer preview |
//! | `sip-stir-shaken` | | RFC 8224 caller-ID attestation; requires `sip` (`rvoip::stir_shaken`) — developer preview |
//! | `voip-3` | | SIP + WebRTC + UCTP **+** vCon / identity / AI-harness extensions — developer preview |
//! | `client` | | Cross-transport client SDK (`rvoip::client`) — developer preview |
//! | `app` | | High-level gateway builder (`rvoip::app`) — developer preview |
//! | `full` | | `voip-3` + `vapi` + `sip-stir-shaken` + `client` + `app` |
//!
//! Deployment-oriented `bundle-*` features group these flags into tested
//! starting points. See the repository's `docs/FEATURE_BUNDLES.md` for the
//! exact machine-checked matrix and native dependency boundaries.
//!
//! The `vcon`, `identity`, and `harness` conversation-model extensions are
//! transport-agnostic and reachable **only** through the `voip-3` feature.
//!
//! ## Module layout
//!
//! The unifying voip-3 nouns are re-exported at the crate root via
//! `rvoip::core_traits`; the `Orchestrator` + `Config` at the root directly.
//! Each transport/extension lives under its own feature-gated module
//! (`rvoip::sip`, `rvoip::webrtc`, `rvoip::uctp`, `rvoip::app`,
//! `rvoip::vapi`, `rvoip::client`, …).

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
// WebRTC (developer preview)
// ---------------------------------------------------------------------------

/// WebRTC interop adapter — bridges DTLS-SRTP / ICE peers into the voip-3
/// `Session` abstraction. Off by default; enable the `webrtc` feature.
#[cfg(feature = "webrtc")]
pub mod webrtc {
    pub use rvoip_webrtc::*;
}

/// Bearer-credential validation shared by transport auth hooks.
///
/// An app owner implements [`auth::BearerValidator`] and hands it to a
/// transport hook (for WebRTC, `webrtc::signaling::auth::AuthCoreHook`)
/// so signaling upgrades authenticate against the owner's own control
/// plane. Exposed with the `webrtc` feature because that is the transport
/// whose app-level config accepts a hook today.
#[cfg(feature = "webrtc")]
pub mod auth {
    pub use rvoip_auth_core::*;
}

// ---------------------------------------------------------------------------
// UCTP substrates (developer preview)
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
// Vapi agent transport (developer preview)
// ---------------------------------------------------------------------------

/// Vapi bidirectional WebSocket agent adapter. It lets rvoip retain ownership
/// of a SIP or WebRTC caller leg while Vapi runs the remote voice-AI pipeline.
/// Off by default; enable the `vapi` feature.
#[cfg(feature = "vapi")]
pub mod vapi {
    pub use rvoip_vapi::*;
}

// ---------------------------------------------------------------------------
// voip-3 conversation-model extensions (developer preview) — reachable only via `voip-3`
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
// Client SDK (developer preview)
// ---------------------------------------------------------------------------

/// Client-side API for mobile / web / desktop / embedded apps, wrapping the
/// SIP / WebRTC / UCTP transports behind one surface. See `rvoip-client`.
#[cfg(feature = "client")]
pub mod client {
    pub use rvoip_client::*;
}

// ---------------------------------------------------------------------------
// High-level app/gateway API (developer preview)
// ---------------------------------------------------------------------------

/// High-level server/gateway API for building practical cross-transport VoIP
/// apps without manually wiring adapters, registrar state, core event loops,
/// and media bridges.
#[cfg(feature = "app")]
pub mod app;

/// The version of the rvoip facade crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod feature_bundle_contract_tests {
    #[cfg(feature = "bundle-sip-endpoint")]
    #[test]
    fn sip_endpoint_bundle_enables_only_its_declared_facade_surface() {
        assert!(cfg!(feature = "sip"));
        assert!(!cfg!(feature = "webrtc"));
        assert!(!cfg!(feature = "uctp"));
        assert!(!cfg!(feature = "dtls-srtp"));
        assert!(!cfg!(feature = "opus"));
    }

    #[cfg(feature = "bundle-carrier-sip")]
    #[test]
    fn carrier_bundle_enables_security_identity_and_pure_rust_telephony_codecs() {
        assert!(cfg!(feature = "sip"));
        assert!(cfg!(feature = "sip-stir-shaken"));
        assert!(cfg!(feature = "dtls-srtp"));
        assert!(cfg!(feature = "g729"));
        assert!(cfg!(feature = "amr-nb"));
        assert!(cfg!(feature = "amr-wb"));
        assert!(!cfg!(feature = "opus"));
    }

    #[cfg(feature = "bundle-browser-gateway")]
    #[test]
    fn browser_gateway_bundle_enables_app_transports_and_opus() {
        assert!(cfg!(feature = "app"));
        assert!(cfg!(feature = "sip"));
        assert!(cfg!(feature = "webrtc"));
        assert!(cfg!(feature = "uctp"));
        assert!(cfg!(feature = "opus"));
    }

    #[cfg(feature = "bundle-ai-conversation")]
    #[test]
    fn ai_conversation_bundle_enables_app_conversation_and_agent_surfaces() {
        assert!(cfg!(feature = "app"));
        assert!(cfg!(feature = "voip-3"));
        assert!(cfg!(feature = "vapi"));
        assert!(cfg!(feature = "opus"));
    }

    #[cfg(feature = "bundle-full-pure-rust")]
    #[test]
    fn pure_rust_full_bundle_keeps_native_opus_off() {
        assert!(cfg!(feature = "full"));
        assert!(cfg!(feature = "dtls-srtp"));
        assert!(cfg!(feature = "g729"));
        assert!(cfg!(feature = "amr-nb"));
        assert!(cfg!(feature = "amr-wb"));
        assert!(!cfg!(feature = "opus"));
    }

    #[cfg(feature = "bundle-full-native")]
    #[test]
    fn native_full_bundle_enables_every_mainline_codec() {
        assert!(cfg!(feature = "full"));
        assert!(cfg!(feature = "dtls-srtp"));
        assert!(cfg!(feature = "g729"));
        assert!(cfg!(feature = "amr-nb"));
        assert!(cfg!(feature = "amr-wb"));
        assert!(cfg!(feature = "opus"));
    }
}
