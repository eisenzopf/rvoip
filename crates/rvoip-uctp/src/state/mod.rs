//! UCTP state machines and the per-peer [`coordinator::UctpCoordinator`].
//!
//! See `UCTP_IMPLEMENTATION_PLAN.md` §3.5 for the design.

pub mod connection;
pub mod coordinator;
pub mod events;
pub mod orchestrator_handler;
pub mod session;
pub mod signature_policy;
pub mod subscription;

pub use connection::{ConnectionInput, ConnectionMachine, UctpConnectionState};
pub use coordinator::{
    default_v0_descriptor, UctpCoordinator, UctpCoordinatorCaps, ENVELOPE_CHANNEL_CAP,
    MAX_SESSIONS_PER_PEER, SIGNALING_SEND_TIMEOUT,
};
pub use events::UctpSessionEvent;
pub use orchestrator_handler::{OrchestratorSubscriptionHandler, DEFAULT_ACCEPTED_CODECS};
pub use session::{SessionInput, SessionMachine, UctpSessionState};
pub use signature_policy::Sig9421Policy;
pub use subscription::{
    rejecting_handler, PublisherInfo, RejectingHandler, SubscriptionHandler, SubscriptionOutcome,
};
