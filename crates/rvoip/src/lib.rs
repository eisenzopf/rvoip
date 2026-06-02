//! # rvoip — Universal real-time gateway library
//!
//! `rvoip` is the facade crate for the rvoip workspace. This **beta**
//! release exposes the SIP product: the voip-3 substrate
//! (`rvoip-core`, `rvoip-core-traits`) plus the SIP interop adapter
//! (`rvoip-sip`), with the optional vCon container builder and in-process
//! AI voice harness — feature-gated so consumers pull only what they need.
//! The remaining substrates (WebRTC, the UCTP family, identity backends,
//! and the client SDK) are not part of this release and return in a later
//! version.
//!
//! See `docs/PRD.md`, `INTERFACE_DESIGN.md`, and
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
//! | Feature | Pulls in |
//! |---|---|
//! | `sip` (default) | SIP interop adapter (`rvoip-sip`) |
//! | `vcon` (default) | vCon container builder + signing (`rvoip-vcon`) |
//! | `harness` | In-process AI voice harness (`rvoip-harness`) |
//! | `full` | All of the above |
//!
//! ## Module layout
//!
//! The SIP native surface lives at `rvoip::sip`. The unifying voip-3
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

/// The version of the rvoip facade crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
