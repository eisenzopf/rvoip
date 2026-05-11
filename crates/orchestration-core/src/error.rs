use crate::ids::{AgentId, AgentOfferId, CallId, QueueId};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, OrchestrationError>;

#[derive(Debug, Error)]
pub enum OrchestrationError {
    #[error("session-core error: {0}")]
    SessionCore(#[from] rvoip_session_core::SessionError),

    #[error("call not found: {0}")]
    CallNotFound(CallId),

    #[error("agent not found: {0}")]
    AgentNotFound(AgentId),

    #[error("queue not found: {0}")]
    QueueNotFound(QueueId),

    #[error("agent unavailable: {0}")]
    AgentUnavailable(AgentId),

    #[error("agent reservation failed: {0}")]
    AgentReservationFailed(AgentId),

    #[error("offer timed out: {0}")]
    OfferTimedOut(AgentOfferId),

    #[error("assignment conflict: {0}")]
    AssignmentConflict(String),

    #[error("contact resolution failed for {0}: {1}")]
    ContactResolutionFailed(AgentId, String),

    #[error("bridge failed: {0}")]
    BridgeFailed(String),

    #[error("routing failed: {0}")]
    RoutingFailed(String),

    #[error("voice AI failed: {0}")]
    VoiceAiFailed(String),

    #[error("recording failed: {0}")]
    RecordingFailed(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("admission rejected: orchestrator at concurrency limit ({0} setups in flight)")]
    AdmissionRejected(usize),
}
