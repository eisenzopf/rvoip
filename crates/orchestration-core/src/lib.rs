//! SIP-focused voice orchestration primitives built on top of `rvoip-sip`.
//!
//! This crate intentionally provides SIP voice orchestration building blocks
//! rather than a full omnichannel contact-center product. The main business
//! object is [`Call`], human and AI workers are both [`Agent`]s, and queue
//! assignment is modeled as a reservation-backed workflow.

pub mod assignment;
pub mod config;
pub mod error;
pub mod events;
pub mod ids;
pub mod orchestrator;
pub mod store;
pub mod traits;
pub mod types;
pub mod voice_ai;

pub use assignment::*;
pub use config::*;
pub use error::{OrchestrationError, Result};
pub use events::*;
pub use ids::*;
pub use orchestrator::*;
pub use store::*;
pub use traits::*;
pub use types::*;
pub use voice_ai::*;

// CARVE_PLAN step 8: ContactResolver / impls / ResolvedContact / ContactSource
// lifted to `rvoip_sip::server`. Re-exported here so existing
// `rvoip_orchestration_core::ContactResolver` consumers keep compiling
// without changing their import paths.
pub use rvoip_sip::server::{
    ContactRequest, ContactResolver, ContactResolverError, ContactSource, RegistrarContactResolver,
    ResolvedContact, StaticContactResolver,
};

pub mod prelude {
    pub use crate::{
        Agent, AgentCapacity, AgentConnector, AgentId, AgentKind, AgentOffer, AgentOfferStatus,
        AgentState, AsrConfig, AsrProvider, AsrSession, Assignment, AssignmentManager,
        AudioEncoding, AudioFormat, Call, CallContext, CallDirection, CallDisposition, CallId,
        CallLeg, CallLegId, CallLegRole, CallLegStatus, CallMetrics, CallPriority, CallStatus,
        CallerIdentity, ContactResolver, ContactSource, DialogCallContext, DialogManager,
        DialogSessionId, DialogTurn, MemoryAgentOfferStore, MemoryAgentStore, MemoryCallStore,
        MemoryQueueStore, OrchestrationConfig, OrchestrationError, OrchestrationEvent,
        OrchestrationEventBus, OrchestrationEventEnvelope, OrchestrationHandle, Orchestrator,
        OrchestratorBuilder, Queue, QueueId, QueuePolicy, QueueSelector, QueueStats, QueueTarget,
        QueuedCall, RecordingSink, RegistrarContactResolver, ResolvedContact, Result,
        RouteDecision, RouteRequest, Router, Skill, StaticRouter, TranscriptEvent, TransferTarget,
        TtsProvider, TtsRequest, TtsStream, VoiceAiAction, VoiceAiId, VoiceAiRuntime,
        VoiceAiRuntimeConfig, VoiceAiSession, VoiceAiSessionId, VoiceAiSessionStatus,
    };
}
