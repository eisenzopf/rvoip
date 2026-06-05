//! # rvoip-core-traits
//!
//! Pure-data trait + type surface for the `rvoip` ecosystem.
//!
//! Carved out from `rvoip-core` (per GAP_PLAN.md V2.A) to break the
//! `rvoip-core → rvoip-vcon → rvoip-auth-core → rvoip-core` dep
//! cycle. Consumer crates (`rvoip-auth-core`, `rvoip-vcon`,
//! `rvoip-harness`, `rvoip-identity`, and every adapter) depend on
//! this crate for the types they need; `rvoip-core` itself
//! re-exports from here so call sites like `use rvoip_core::ids::ConnectionId`
//! keep working.
//!
//! ## Layering rule
//!
//! This crate has zero `rvoip-*` dependencies — that's what breaks
//! the cycle. It depends only on `bytes`, `chrono`, `serde`,
//! `serde_json`, and `uuid`.
//!
//! ## What lives here (V2.A.1)
//!
//! - [`ids`] — every `*Id` newtype (`ConnectionId`, `SessionId`,
//!   `ConversationId`, `ParticipantId`, `IdentityId`, etc.).
//! - [`identity`] — the pure-data identity types `IdentityAssurance`,
//!   `Jwk`, `CredentialKind`, `Credential`, `IdentityKind`,
//!   `DeviceKind`. The `IdentityProvider` trait + the structs that
//!   reference rvoip-core's `Result` type (Identity, Device,
//!   ReachabilityHint) stay in `rvoip-core::identity` because they
//!   need the orchestrator's error type.
//!
//! ## Future scope
//!
//! Subsequent V2.A.* phases can move more modules here (events,
//! commands, capability, full identity trait, vcon types, signing,
//! store traits) as the workspace's appetite for the move-cost
//! tradeoff grows. The minimal carve-out shipped today is purely
//! cycle-breaking.

pub mod capability;
pub mod connection;
pub mod error;
pub mod harness;
pub mod identity;
pub mod ids;
pub mod stream;
