//! SIP-focused voice orchestration primitives built on top of `rvoip-session-core`.
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

pub mod prelude {
    pub use crate::{
        Agent, AgentCapacity, AgentConnector, AgentKind, AgentOffer, AgentOfferStatus, AgentState,
        AsrConfig, AsrProvider, AsrSession, Assignment, AssignmentManager, Call, CallContext,
        CallDirection, CallDisposition, CallId, CallLeg, CallLegId, CallLegRole, CallLegStatus,
        CallMetrics, CallPriority, CallStatus, ContactResolver, DialogManager, DialogTurn,
        MemoryAgentOfferStore, MemoryAgentStore, MemoryCallStore, MemoryQueueStore,
        OrchestrationConfig, OrchestrationError, OrchestrationEvent, OrchestrationEventBus,
        OrchestrationEventEnvelope, OrchestrationHandle, Orchestrator, OrchestratorBuilder, Queue,
        QueuePolicy, QueueSelector, QueueStats, QueueTarget, QueuedCall, RecordingSink,
        ResolvedContact, Result, RouteDecision, RouteRequest, Router, Skill, TransferTarget,
        TtsProvider, TtsRequest, TtsStream, VoiceAiAction, VoiceAiRuntime, VoiceAiRuntimeConfig,
        VoiceAiSession,
    };
}
