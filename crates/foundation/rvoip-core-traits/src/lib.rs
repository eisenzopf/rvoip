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
//! ## What lives here
//!
//! - [`ids`] — every `*Id` newtype (`ConnectionId`, `SessionId`,
//!   `ConversationId`, `ParticipantId`, `IdentityId`, etc.).
//! - [`adapter`] — pure adapter-facing request/event/reason structs
//!   (`AdapterEvent`, `OriginateRequest`, `EndReason`, etc.).
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
//! commands, full identity trait, vcon types, signing, store traits,
//! and eventually the full `ConnectionAdapter` trait once message and
//! command types have moved) as the workspace's appetite for the
//! move-cost tradeoff grows.

pub mod adapter;
pub mod broadcast;
pub mod capability;
pub mod connection;
pub mod data;
pub mod error;
pub mod harness;
pub mod identity;
pub mod ids;
pub mod stream;

pub use adapter::{
    ExternalConnectionReference, ExternalConnectionReferenceError, InboundConnectionContext,
    InboundContextError, InboundRoutingHint, InboundSignalingMetadata, OriginateContext,
    OutboundActivation, MAX_EXTERNAL_CONNECTION_REFERENCES, MAX_EXTERNAL_REFERENCE_KIND_BYTES,
    MAX_EXTERNAL_REFERENCE_VALUE_BYTES, MAX_INBOUND_METADATA_BYTES, MAX_INBOUND_METADATA_FIELDS,
    MAX_INBOUND_METADATA_NAME_BYTES, MAX_INBOUND_METADATA_VALUE_BYTES,
    MAX_INBOUND_ROUTING_HINT_BYTES,
};
pub use broadcast::{
    BroadcastDescriptor, BroadcastDrainDescriptor, BroadcastDrainReason, BroadcastDrainRequest,
    BroadcastDrainState, BroadcastEndpoint, BroadcastHealthDescriptor, BroadcastHealthIssue,
    BroadcastHealthStatus, BroadcastLifecycleDescriptor, BroadcastLifecycleState,
    BroadcastProtocolDescriptor, BroadcastProtocolFamily, BroadcastPublisher, BroadcastRelayHop,
    BroadcastRelayRole, BroadcastResource, BroadcastSanitizedEvent,
    BroadcastSanitizedEventCapability, BroadcastSanitizedEventError, BroadcastSanitizedEventKind,
    BroadcastSubstrate, BroadcastTransport, MAX_BROADCAST_EVENT_JSON_INTEGER,
};
pub use data::{
    DataMessage, DataMessageValidationError, DataReliability, MAX_CONTENT_TYPE_BYTES,
    MAX_DATA_LABEL_BYTES, MAX_DATA_MESSAGE_BYTES, MAX_DATA_MESSAGE_ID_BYTES,
};
pub use identity::{
    AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, PrincipalOwnershipKey,
};
