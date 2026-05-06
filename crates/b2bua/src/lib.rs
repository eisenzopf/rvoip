//! B2BUA orchestration for RVoIP.
//!
//! This crate is the first platform layer above `session-core`. It keeps
//! `UnifiedCoordinator` as the internal SIP/session/media engine and adds the
//! call-topology behavior needed by servers: inbound leg, outbound leg, route
//! decision, bridge lifetime, event correlation, and teardown propagation.

mod error;
mod service;
mod types;

pub use error::{B2buaError, Result};
pub use service::{B2buaConfig, B2buaService, Router, StaticRouter};
pub use types::{
    B2buaCallHandle, B2buaCallId, B2buaCallSnapshot, B2buaCallStatus, B2buaEvent,
    B2buaEventReceiver, B2buaLeg, BridgeId, LegRole, RedirectDecision, RejectDecision,
    RouteDecision, RouteRequest,
};

pub use rvoip_session_core::{Config as SessionConfig, UnifiedCoordinator};
