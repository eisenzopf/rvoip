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
pub mod supervisor;

pub use connection::{ConnectionInput, ConnectionMachine, UctpConnectionState};
pub use coordinator::{
    default_v0_descriptor, UctpCoordinator, UctpCoordinatorCaps, UctpResourceSnapshot,
    UctpScopePolicy, DEFAULT_REPLAY_WINDOW, ENVELOPE_CHANNEL_CAP, MAX_CONNECTIONS_PER_PEER,
    MAX_SESSIONS_PER_PEER, MAX_STREAMS_PER_CONNECTION, SIGNALING_SEND_TIMEOUT, UCTP_DATA_SCOPE,
    UCTP_RECEIVE_ONLY_SCOPE, UCTP_SESSION_SCOPE, UCTP_SUBSCRIBE_SCOPE,
};
pub use events::UctpSessionEvent;
pub use orchestrator_handler::{OrchestratorSubscriptionHandler, DEFAULT_ACCEPTED_CODECS};
pub use session::{SessionInput, SessionMachine, UctpSessionState};
pub use signature_policy::{Sig9421Config, Sig9421Policy};
pub use subscription::{
    rejecting_handler, BoundSubscriptionHandler, NamespacedSubscriptionHandler,
    PeerResourceBindings, PeerScopedSessionResolver, PublisherInfo, RejectingHandler,
    ResourceBindingError, SessionBindingResolver, SubscriptionHandler, SubscriptionOutcome,
};
pub use supervisor::{
    spawn_auth_lifecycle_guard, spawn_resource_authorization_guard, supervise_peer_tasks,
    supervise_peer_tasks_with_media_cancel, try_deliver_adapter_event,
    try_deliver_orchestrator_event, PeerSessionExit, DEFAULT_AUTHENTICATION_DEADLINE,
};
