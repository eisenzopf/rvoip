//! # rvoip-core
//!
//! Transport-agnostic spine for RVoIP. Carries the voip-3 vocabulary
//! ([`Conversation`], [`Session`], [`Connection`], [`MediaStream`], [`Message`],
//! [`Participant`]), the [`ConnectionAdapter`] trait that adapter crates implement,
//! the cross-transport [`BridgeManager`], and the [`Orchestrator`] entry point.
//!
//! See `INTERFACE_DESIGN.md` §3, §6, §7.5, §10.2 for the canonical design.
//!
//! ## Two-layer surface
//!
//! - **Pure-SIP consumers** (carriers, SIP softphones, SIP-only B2BUAs) stay on
//!   `rvoip_sip::api::*` and `rvoip_sip::server::*` — they don't touch this
//!   crate. See `rvoip-sip` for that surface.
//! - **Cross-transport consumers** (Thelve, future CPaaS, anyone planning to
//!   add WebRTC / QUIC adapters) build a [`Orchestrator`] here and register
//!   adapters against it. SIP is one such adapter today (`rvoip_sip::SipAdapter`);
//!   `rvoip-webrtc` and `rvoip-quic` register against the same handle when
//!   they ship.
//!
//! ## Cross-transport entry point
//!
//! ```rust,no_run
//! use rvoip_core::{Config, Orchestrator, Transport};
//! # use std::sync::Arc;
//! # struct MyAdapter;
//! # #[async_trait::async_trait]
//! # impl rvoip_core::ConnectionAdapter for MyAdapter {
//! #     fn transport(&self) -> Transport { Transport::Sip }
//! #     fn kind(&self) -> rvoip_core::AdapterKind { rvoip_core::AdapterKind::Interop }
//! #     async fn originate(&self, _: rvoip_core::OriginateRequest) -> rvoip_core::Result<rvoip_core::ConnectionHandle> { unimplemented!() }
//! #     async fn accept(&self, _: rvoip_core::ConnectionId) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn reject(&self, _: rvoip_core::ConnectionId, _: rvoip_core::RejectReason) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn end(&self, _: rvoip_core::ConnectionId, _: rvoip_core::EndReason) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn hold(&self, _: rvoip_core::ConnectionId) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn resume(&self, _: rvoip_core::ConnectionId) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn transfer(&self, _: rvoip_core::ConnectionId, _: rvoip_core::TransferTarget) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn streams(&self, _: rvoip_core::ConnectionId) -> rvoip_core::Result<Vec<Arc<dyn rvoip_core::MediaStream>>> { Ok(vec![]) }
//! #     async fn send_message(&self, _: rvoip_core::ConnectionId, _: rvoip_core::Message) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn send_dtmf(&self, _: rvoip_core::ConnectionId, _: &str, _: u32) -> rvoip_core::Result<()> { Ok(()) }
//! #     async fn renegotiate_media(&self, _: rvoip_core::ConnectionId, _: rvoip_core::CapabilityDescriptor) -> rvoip_core::Result<rvoip_core::NegotiatedCodecs> { Ok(Default::default()) }
//! #     fn subscribe_events(&self) -> tokio::sync::mpsc::Receiver<rvoip_core::AdapterEvent> { tokio::sync::mpsc::channel(1).1 }
//! #     fn capabilities(&self) -> rvoip_core::CapabilityDescriptor { Default::default() }
//! #     async fn verify_request_signature(&self, _: rvoip_core::ConnectionId, _: rvoip_core::SignatureHeaders) -> rvoip_core::Result<rvoip_core::IdentityAssurance> { Ok(rvoip_core::IdentityAssurance::Anonymous) }
//! # }
//! # async fn example(my_adapter: Arc<MyAdapter>) -> rvoip_core::Result<()> {
//! let orchestrator = Orchestrator::new(Config::default());
//! orchestrator.register(my_adapter)?;
//!
//! let mut events = orchestrator.subscribe_events();
//! // events.recv() yields normalized rvoip-core Events (Connection*, Session*,
//! // Conversation*) translated from each adapter's protocol-native AdapterEvents.
//! # let _ = events;
//! # Ok(())
//! # }
//! ```
//!
//! [`Orchestrator::register`] dispatches every per-connection command
//! ([`Orchestrator::route_inbound_connection`], [`Orchestrator::originate_connection`],
//! [`Orchestrator::end_connection`], [`Orchestrator::hold`], [`Orchestrator::resume`],
//! [`Orchestrator::transfer_connection`], [`Orchestrator::send_dtmf`],
//! [`Orchestrator::mute`], [`Orchestrator::unmute`],
//! [`Orchestrator::play_audio`]) through the adapter for the
//! connection's [`Transport`]. Cross-transport bridging
//! (cross-codec frame-pump per `INTERFACE_DESIGN.md` §10.2) is fully
//! wired, including hot-swap on `renegotiate_media` and DTMF auto-
//! route across legs.
//!
//! ## Conversation / Session / Participant lifecycle
//!
//! Beyond per-Connection dispatch, the Orchestrator owns live
//! Conversation/Session/Participant state. See
//! [`Orchestrator::open_conversation`], [`Orchestrator::start_session`],
//! [`Orchestrator::join_session`], [`Orchestrator::leave_session`],
//! [`Orchestrator::end_session`], [`Orchestrator::close_conversation`]
//! and the cross-substrate messaging methods
//! [`Orchestrator::send_message_to_conversation`] +
//! [`Orchestrator::list_messages`] + [`Orchestrator::mark_message_read`].
//!
//! ## vCon, recording, transcription, AI harness
//!
//! Every Session gets a [`DefaultVconBuilder`] auto-bound at
//! `start_session`; on `end_session` the snapshot is encoded, persisted
//! via [`VconStore`], and emitted as `Event::VconReady`. Recording
//! and transcription dispatch via consumer-registered providers
//! ([`Orchestrator::register_recording_sink`],
//! [`Orchestrator::register_asr_provider`]); the AI harness path
//! includes barge-in support (`Event::BargeInDetected`).
//!
//! ## Tenant scoping, capacity, observability
//!
//! [`Config::TenantQuotas`](crate::config::TenantQuotas) enforces
//! per-tenant maxes on concurrent sessions, recordings, and AI
//! attachments. Periodic emit cadences live in
//! [`Orchestrator::spawn_capacity_scheduler`],
//! [`Orchestrator::spawn_media_quality_sampler`], and
//! [`Orchestrator::spawn_idle_closer`] (Ephemeral Conversation
//! close-after-idle driver).
//!
//! See `examples/sip_only_orchestrator.rs` for the working SIP-adapter
//! flow end-to-end, and [`GAP_PLAN.md`](../../docs/GAP_PLAN.md)
//! for the phased-roadmap status.
//!
//! ## Layering rule
//!
//! Per `INTERFACE_DESIGN.md` §18: this crate never imports an adapter crate.
//! Adapters depend on `rvoip-core`, not the other way round. The only
//! external dep that's transport-flavored is `rvoip-media-core` (a common
//! crate, not an adapter), used by [`bridge`] for the SIP-fast-path bridge
//! handle.

pub mod adapter;
pub mod bridge;
pub mod broadcast;
pub mod capability;
pub mod commands;
pub mod config;
pub mod connection;
pub mod conversation;
pub mod error;
pub mod events;
pub mod harness;
pub mod identity;
pub mod ids;
pub mod media_graph;
pub mod message;
pub mod orchestrator;
pub mod participant;
pub mod session;
pub mod signing;
pub mod store;
pub mod stream;
pub mod subscriptions;
pub mod vcon;
pub mod virtual_publisher;

pub use adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    PlaybackHandle, RejectReason, SignatureHeaders, TransferTarget,
};
pub use bridge::{BridgeError, BridgeHandle, BridgeManager};
pub use broadcast::{
    BroadcastDescriptor, BroadcastDrainDescriptor, BroadcastDrainReason, BroadcastDrainRequest,
    BroadcastDrainState, BroadcastEndpoint, BroadcastHealthDescriptor, BroadcastHealthIssue,
    BroadcastHealthStatus, BroadcastLifecycleDescriptor, BroadcastLifecycleState,
    BroadcastProtocolDescriptor, BroadcastProtocolFamily, BroadcastPublisher, BroadcastRelayHop,
    BroadcastRelayRole, BroadcastResource, BroadcastSanitizedEvent,
    BroadcastSanitizedEventCapability, BroadcastSanitizedEventError, BroadcastSanitizedEventKind,
    BroadcastSubstrate, BroadcastTransport, MAX_BROADCAST_EVENT_JSON_INTEGER,
};
pub use capability::{CapabilityDescriptor, CapabilityIntersection, CodecInfo, NegotiatedCodecs};
pub use commands::{
    AttachmentRef, AudioSource, Command, InboundAction, ListenerSink, ListenerTarget,
    MuteDirection, RecordingSink, RecordingTarget,
};
pub use config::Config;
pub use connection::{Connection, ConnectionState, Direction, Transport};
pub use conversation::{Conversation, ConversationPolicy, ConversationState};
pub use error::{Result, RvoipError};
pub use events::{AnomalyKind, ConnectionProgressKind, Event, SessionQualityReport, UsageKind};
pub use identity::{
    AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, Credential, CredentialKind,
    Device, DtlsFingerprint, Identity, IdentityAssurance, IdentityProvider, Jwk,
    PrincipalOwnershipKey,
};
pub use ids::{
    AiAttachmentId, AttachmentId, BridgeId, ConnectionId, ConversationId, DeviceId, IdentityId,
    ListenerId, MediaRouteId, MessageId, ParticipantId, PlaybackId, RecordingId, SessionId,
    StreamId, TenantId, TranscriptionId,
};
pub use media_graph::{
    start_media_graph, MediaGraphHandle, MediaGraphPolicy, DEFAULT_MEDIA_GRAPH_MAX_SINKS,
};
pub use message::{ContentType, Message, MessageOrigin, MessageRecipients};
pub use orchestrator::Orchestrator;
pub use participant::{Participant, ParticipantKind, ParticipantRole};
pub use rvoip_core_traits::data::{
    DataMessage, DataMessageValidationError, DataReliability, MAX_CONTENT_TYPE_BYTES,
    MAX_DATA_LABEL_BYTES, MAX_DATA_MESSAGE_BYTES, MAX_DATA_MESSAGE_ID_BYTES,
};
pub use session::{Session, SessionMedium, SessionState};
pub use store::{
    ConversationFilter, ConversationStore, MemoryConversationStore, MemoryMessageStore,
    MemoryVconStore, MessageFilter, MessagePage, MessageStore, PageCursor, VconStore,
};
pub use stream::{MediaFrame, MediaStream, MediaStreamHandle, QualitySnapshot, StreamKind};
pub use vcon::{
    DefaultVconBuilder, VconAnalysis, VconAnalysisKind, VconAttachment, VconBuilderHandle,
    VconDialog, VconDialogKind, VconParty, VconRef, VconSnapshot,
};
pub use virtual_publisher::{
    ManagedVirtualPublisher, VirtualPublisherDescriptor, DEFAULT_VIRTUAL_PUBLISHER_QUEUE_CAPACITY,
};

// V2.A.8 — when `vcon-signing` is enabled, re-export the
// `rvoip-vcon` crate's surface so consumers can sign vCons + plug
// their own `VconStore` impl without adding rvoip-vcon as a separate
// Cargo dep. The orchestrator's auto-emission path still produces
// raw bytes via `vcon::encode_snapshot` for the unsigned default; a
// consumer wanting JWS signing constructs an adapter `VconStore`
// impl that builds an `rvoip_vcon::Vcon` from the snapshot, calls
// `signed_vcon::sign_jws`, then persists.
#[cfg(feature = "vcon-signing")]
pub mod signed_vcon {
    //! V2.A.8 — feature-gated re-export of `rvoip-vcon` so consumers
    //! enabling `vcon-signing` get the signing surface from a single
    //! dep on rvoip-core.
    pub use rvoip_vcon::*;
}
