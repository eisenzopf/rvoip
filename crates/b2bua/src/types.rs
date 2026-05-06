use std::fmt;

use rvoip_session_core::SessionId;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Stable B2BUA-level call id.
///
/// This id correlates the inbound and outbound `session-core` sessions that
/// form one two-leg call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct B2buaCallId(String);

impl B2buaCallId {
    /// Generate a new B2BUA call id.
    pub fn new() -> Self {
        Self(format!("b2bua_{}", Uuid::new_v4()))
    }

    /// Borrow the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for B2buaCallId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for B2buaCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable id for a B2BUA media bridge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BridgeId(String);

impl BridgeId {
    /// Generate a new bridge id.
    pub fn new() -> Self {
        Self(format!("bridge_{}", Uuid::new_v4()))
    }

    /// Borrow the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for BridgeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BridgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Role of a leg inside a B2BUA call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LegRole {
    /// Caller-facing leg accepted by the B2BUA.
    Inbound,
    /// Target-facing leg originated by the B2BUA.
    Outbound,
}

impl LegRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LegRole::Inbound => "inbound",
            LegRole::Outbound => "outbound",
        }
    }
}

/// One SIP/session leg owned by a B2BUA call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct B2buaLeg {
    /// Leg role.
    pub role: LegRole,
    /// Underlying `session-core` session id.
    pub session_id: SessionId,
    /// Remote or local URI associated with this leg.
    pub uri: String,
}

/// Request passed to a router when an inbound call arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteRequest {
    /// B2BUA-level call id.
    pub call_id: B2buaCallId,
    /// Inbound leg.
    pub inbound: B2buaLeg,
    /// Caller URI.
    pub from: String,
    /// Called URI.
    pub to: String,
    /// SIP Call-ID header value from the inbound INVITE.
    pub sip_call_id: String,
    /// Inbound P-Asserted-Identity, when present.
    pub p_asserted_identity: Option<String>,
}

/// Reject response selected by routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectDecision {
    /// SIP status code.
    pub status_code: u16,
    /// Reason phrase.
    pub reason: String,
}

impl RejectDecision {
    /// Create a reject decision.
    pub fn new(status_code: u16, reason: impl Into<String>) -> Self {
        Self {
            status_code,
            reason: reason.into(),
        }
    }
}

/// Redirect response selected by routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedirectDecision {
    /// SIP 3xx status code.
    pub status_code: u16,
    /// Contact URIs.
    pub contacts: Vec<String>,
}

impl RedirectDecision {
    /// Create a redirect decision.
    pub fn new(status_code: u16, contacts: Vec<String>) -> Self {
        Self {
            status_code,
            contacts,
        }
    }
}

/// Router output for an inbound call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteDecision {
    /// Dial a target URI and bridge the inbound leg after answer.
    Dial {
        /// Outbound target URI.
        target: String,
        /// Optional outbound From URI. Defaults to the B2BUA service local URI.
        from: Option<String>,
    },
    /// Reject the inbound call.
    Reject(RejectDecision),
    /// Redirect the inbound call to alternate contacts.
    Redirect(RedirectDecision),
}

impl RouteDecision {
    /// Dial an outbound SIP target using the service local URI as From.
    pub fn dial(target: impl Into<String>) -> Self {
        Self::Dial {
            target: target.into(),
            from: None,
        }
    }

    /// Dial an outbound SIP target with an explicit From URI.
    pub fn dial_from(target: impl Into<String>, from: impl Into<String>) -> Self {
        Self::Dial {
            target: target.into(),
            from: Some(from.into()),
        }
    }

    /// Reject with a SIP status and reason phrase.
    pub fn reject(status_code: u16, reason: impl Into<String>) -> Self {
        Self::Reject(RejectDecision::new(status_code, reason))
    }

    /// Redirect with one or more Contact URIs.
    pub fn redirect(status_code: u16, contacts: Vec<String>) -> Self {
        Self::Redirect(RedirectDecision::new(status_code, contacts))
    }
}

/// Coarse lifecycle status for a B2BUA call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum B2buaCallStatus {
    /// Inbound call has been observed.
    Incoming,
    /// Router is selecting a destination.
    Routing,
    /// Outbound leg is being dialed.
    Dialing,
    /// Outbound leg answered and inbound leg is being accepted.
    Answering,
    /// Both legs are active and the bridge is live.
    Bridged,
    /// Call is ending.
    Ending,
    /// Call ended normally.
    Ended,
    /// Call failed before a normal end.
    Failed,
    /// Inbound call was rejected by policy.
    Rejected,
    /// Inbound call was redirected by policy.
    Redirected,
}

/// Current observable state of a B2BUA call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct B2buaCallSnapshot {
    /// B2BUA-level call id.
    pub id: B2buaCallId,
    /// Current status.
    pub status: B2buaCallStatus,
    /// Inbound leg.
    pub inbound: B2buaLeg,
    /// Outbound leg, once created.
    pub outbound: Option<B2buaLeg>,
    /// Bridge id, once media is bridged.
    pub bridge_id: Option<BridgeId>,
    /// Last human-readable status or failure reason.
    pub reason: Option<String>,
}

impl B2buaCallSnapshot {
    pub(crate) fn new(id: B2buaCallId, inbound: B2buaLeg) -> Self {
        Self {
            id,
            status: B2buaCallStatus::Incoming,
            inbound,
            outbound: None,
            bridge_id: None,
            reason: None,
        }
    }
}

/// Handle returned after a call has been accepted into the B2BUA service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct B2buaCallHandle {
    /// B2BUA-level call id.
    pub id: B2buaCallId,
    /// Inbound leg.
    pub inbound: B2buaLeg,
}

/// Events emitted by the B2BUA layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum B2buaEvent {
    /// Inbound call arrived.
    IncomingReceived {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Inbound leg.
        inbound: B2buaLeg,
        /// Caller URI.
        from: String,
        /// Called URI.
        to: String,
    },
    /// Router selected a destination.
    RouteSelected {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Decision selected by the router.
        decision: RouteDecision,
    },
    /// Inbound call was rejected.
    InboundRejected {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// SIP status code.
        status_code: u16,
        /// Reason phrase.
        reason: String,
    },
    /// Inbound call was redirected.
    InboundRedirected {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// SIP status code.
        status_code: u16,
        /// Contact URIs.
        contacts: Vec<String>,
    },
    /// Outbound leg is being dialed.
    OutboundDialing {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Outbound leg.
        outbound: B2buaLeg,
        /// Outbound target URI.
        target: String,
    },
    /// Outbound provisional response arrived.
    OutboundProgress {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// SIP status code.
        status_code: u16,
        /// Reason phrase.
        reason: String,
    },
    /// Outbound leg answered.
    OutboundAnswered {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Outbound leg session id.
        outbound_session_id: SessionId,
        /// Whether the answer carried SDP.
        has_sdp: bool,
    },
    /// Inbound leg was accepted.
    InboundAccepted {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Inbound session id.
        inbound_session_id: SessionId,
    },
    /// Media bridge was established.
    BridgeEstablished {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// B2BUA-level bridge id.
        bridge_id: BridgeId,
        /// Inbound leg session id.
        inbound_session_id: SessionId,
        /// Outbound leg session id.
        outbound_session_id: SessionId,
    },
    /// DTMF received on one leg.
    DtmfReceived {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Leg that received the DTMF event.
        leg: LegRole,
        /// DTMF digit.
        digit: char,
    },
    /// REFER received on one leg.
    TransferRequested {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Leg that received REFER.
        leg: LegRole,
        /// Refer-To URI.
        refer_to: String,
        /// Referred-By header, when present.
        referred_by: Option<String>,
        /// Replaces header/parameter, when present.
        replaces: Option<String>,
    },
    /// One leg ended.
    LegEnded {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Leg that ended.
        leg: LegRole,
        /// Underlying session id.
        session_id: SessionId,
        /// Human-readable reason.
        reason: String,
    },
    /// Call ended.
    CallEnded {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Human-readable reason.
        reason: String,
    },
    /// Call failed.
    CallFailed {
        /// B2BUA-level call id.
        call_id: B2buaCallId,
        /// Human-readable reason.
        reason: String,
    },
}

/// Receiver for B2BUA events.
pub type B2buaEventReceiver = broadcast::Receiver<B2buaEvent>;
