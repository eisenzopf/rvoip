//! SIP B2BUA / gateway helpers built on top of `api::UnifiedCoordinator`.
//!
//! Per CARVE_PLAN §2 layering rule: every helper here ultimately calls into
//! `api::UnifiedCoordinator`. The `server::*` surface is *coordination glue*
//! on top of the proven api/ surface — bridge ID assignment, AOR-to-Contact
//! resolution, B2BUA lifecycle patterns — not a parallel access path to
//! `dialog-core` / `media-core`.
//!
//! ## Modules
//!
//! - [`bridge`] — `SipBridgeStrategy` for SIP↔SIP same-codec fast-path bridges.
//!   Wraps `UnifiedCoordinator::bridge(a, b)` and returns the
//!   `media-core::BridgeHandle` so the caller can store it in a registry.
//! - [`contact_resolver`] — AOR → live Contact URI resolution against
//!   `registrar-core`. Lifted from `orchestration-core/src/traits.rs:81-198`
//!   with a SIP-flavored `ContactRequest` input (the workforce-flavored
//!   `Agent` parameter stays in orchestration-core).
//! - [`transfer`] — B2BUA-side transfer orchestration helpers (blind /
//!   attended / external) wrapping the `UnifiedCoordinator::refer(...)`
//!   builder and `UnifiedCoordinator::accept_refer`. The actual REFER
//!   mechanics stay in `api::unified`; these helpers add scenario-specific
//!   glue.
//! - [`b2bua`] — Optional convenience `SipB2bua` that wires the canonical
//!   pattern (incoming INVITE → originate outbound → bridge) entirely
//!   through `api::UnifiedCoordinator`.

pub mod b2bua;
pub mod bridge;
pub mod contact_resolver;
pub mod transfer;

pub use bridge::{sip_bridge, SipBridgeStrategy};
pub use contact_resolver::{
    ContactRequest, ContactResolver, ContactResolverError, ContactSource, RegistrarContactResolver,
    ResolvedContact, StaticContactResolver,
};
pub use transfer::{accept_inbound_refer, attended_transfer, blind_transfer, TransferError};
