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
//! [`Orchestrator::transfer_connection`], [`Orchestrator::send_dtmf`]) through the
//! adapter for the connection's [`Transport`]. Cross-transport bridging
//! (cross-codec frame-pump per `INTERFACE_DESIGN.md` §10.2) lands in a
//! follow-up.
//!
//! See `examples/sip_only_orchestrator.rs` for the working SIP-adapter
//! flow end-to-end.
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
pub mod capability;
pub mod commands;
pub mod config;
pub mod connection;
pub mod conversation;
pub mod error;
pub mod events;
pub mod identity;
pub mod ids;
pub mod message;
pub mod orchestrator;
pub mod participant;
pub mod session;
pub mod store;
pub mod stream;
pub mod vcon;

pub use adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
pub use bridge::{BridgeError, BridgeHandle, BridgeManager};
pub use capability::{CapabilityDescriptor, CapabilityIntersection, CodecInfo, NegotiatedCodecs};
pub use commands::{
    AttachmentRef, AudioSource, Command, InboundAction, ListenerSink, ListenerTarget,
    MuteDirection, RecordingSink, RecordingTarget,
};
pub use config::Config;
pub use connection::{Connection, ConnectionState, Direction, Transport};
pub use conversation::{Conversation, ConversationPolicy, ConversationState};
pub use error::{Result, RvoipError};
pub use events::{AnomalyKind, ConnectionProgressKind, Event, UsageKind};
pub use identity::{
    Credential, CredentialKind, Device, Identity, IdentityAssurance, IdentityProvider, Jwk,
};
pub use ids::{
    AiAttachmentId, AttachmentId, BridgeId, ConnectionId, ConversationId, DeviceId, IdentityId,
    ListenerId, MessageId, ParticipantId, RecordingId, SessionId, StreamId, TenantId,
};
pub use message::{ContentType, Message, MessageOrigin, MessageRecipients};
pub use orchestrator::Orchestrator;
pub use participant::{Participant, ParticipantKind, ParticipantRole};
pub use session::{Session, SessionMedium, SessionState};
pub use store::{ConversationStore, MemoryConversationStore, MemoryVconStore, VconStore};
pub use stream::{MediaFrame, MediaStream, MediaStreamHandle, QualitySnapshot, StreamKind};
pub use vcon::{
    VconAnalysis, VconAnalysisKind, VconAttachment, VconBuilderHandle, VconDialog, VconDialogKind,
    VconParty, VconSnapshot,
};
