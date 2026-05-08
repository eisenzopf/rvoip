use crate::ids::*;
use chrono::{DateTime, Utc};
use rvoip_registrar_core::Transport;
use rvoip_session_core::SessionId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerIdentity {
    pub uri: String,
    pub display_name: Option<String>,
    pub asserted_identity: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl CallerIdentity {
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            display_name: None,
            asserted_identity: None,
            metadata: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallContext {
    pub external_ref: Option<String>,
    pub intent: Option<String>,
    pub language: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub id: CallId,
    pub status: CallStatus,
    pub direction: CallDirection,
    pub caller: CallerIdentity,
    pub dialed_uri: String,
    pub sip_call_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub legs: Vec<CallLeg>,
    pub queue_id: Option<QueueId>,
    pub assigned_agent_id: Option<AgentId>,
    pub active_bridge_id: Option<BridgeId>,
    pub priority: CallPriority,
    pub context: CallContext,
    pub metrics: CallMetrics,
    pub disposition: Option<CallDisposition>,
    pub recording_ids: Vec<RecordingId>,
    pub transcript_id: Option<TranscriptId>,
}

impl Call {
    pub fn inbound(caller: CallerIdentity, dialed_uri: impl Into<String>) -> Self {
        Self {
            id: CallId::new(),
            status: CallStatus::Incoming,
            direction: CallDirection::Inbound,
            caller,
            dialed_uri: dialed_uri.into(),
            sip_call_id: None,
            created_at: Utc::now(),
            answered_at: None,
            ended_at: None,
            legs: Vec::new(),
            queue_id: None,
            assigned_agent_id: None,
            active_bridge_id: None,
            priority: CallPriority::default(),
            context: CallContext::default(),
            metrics: CallMetrics::default(),
            disposition: None,
            recording_ids: Vec::new(),
            transcript_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallStatus {
    Incoming,
    Routing,
    Queued,
    OfferingAgent,
    ConnectingAgent,
    Connected,
    InVoiceAi,
    OnHold,
    Transferring,
    WrapUp,
    Ending,
    Ended,
    Failed,
    Abandoned,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallMetrics {
    pub queue_time: Duration,
    pub talk_time: Duration,
    pub hold_time: Duration,
    pub voice_ai_time: Duration,
    pub transfer_count: u32,
    pub hold_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AbandonPhase {
    Queued,
    OfferingAgent,
    ConnectingAgent,
    InVoiceAi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDisposition {
    Completed,
    Abandoned { phase: AbandonPhase },
    Rejected { status: u16, reason: String },
    Failed { reason: String },
    Transferred { target: TransferTarget },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLeg {
    pub id: CallLegId,
    pub role: CallLegRole,
    pub session_id: SessionId,
    pub sip_call_id: Option<String>,
    pub uri: String,
    pub status: CallLegStatus,
    pub agent_id: Option<AgentId>,
    pub created_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl CallLeg {
    pub fn new(role: CallLegRole, session_id: SessionId, uri: impl Into<String>) -> Self {
        Self {
            id: CallLegId::new(),
            role,
            session_id,
            sip_call_id: None,
            uri: uri.into(),
            status: CallLegStatus::Created,
            agent_id: None,
            created_at: Utc::now(),
            answered_at: None,
            ended_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallLegRole {
    Caller,
    HumanAgent,
    VoiceAiAgent,
    Consult,
    TransferTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallLegStatus {
    Created,
    Dialing,
    Ringing,
    EarlyMedia,
    Answered,
    Bridged,
    Held,
    Ended,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub kind: AgentKind,
    pub display_name: String,
    pub skills: Vec<Skill>,
    pub state: AgentState,
    pub capacity: AgentCapacity,
    pub connector: AgentConnector,
    pub last_state_change_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

impl Agent {
    pub fn human(id: impl Into<AgentId>, sip_uri: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            display_name: id.to_string(),
            id,
            kind: AgentKind::Human,
            skills: Vec::new(),
            state: AgentState::Offline,
            capacity: AgentCapacity::single(),
            connector: AgentConnector::SipUri(sip_uri.into()),
            last_state_change_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    pub fn voice_ai(id: impl Into<AgentId>, runtime_id: impl Into<VoiceAiId>) -> Self {
        let id = id.into();
        Self {
            display_name: id.to_string(),
            id,
            kind: AgentKind::VoiceAi,
            skills: Vec::new(),
            state: AgentState::Offline,
            capacity: AgentCapacity::single(),
            connector: AgentConnector::LocalVoiceAi(runtime_id.into()),
            last_state_change_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    pub fn is_routable(&self) -> bool {
        self.state == AgentState::Available && self.capacity.available_slots() > 0
    }

    pub fn has_required_skills(&self, required: &[Skill]) -> bool {
        if required.is_empty() {
            return true;
        }
        let agent_skills: HashSet<&Skill> = self.skills.iter().collect();
        required.iter().all(|skill| agent_skills.contains(skill))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Human,
    VoiceAi,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentConnector {
    SipUri(String),
    RegisteredSipUser { aor: String },
    LocalVoiceAi(VoiceAiId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Offline,
    Available,
    Reserved,
    Offering,
    Ringing,
    OnCall,
    WrapUp,
    Break,
    Away,
    DoNotDisturb,
}

impl Default for AgentState {
    fn default() -> Self {
        Self::Offline
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCapacity {
    pub max_active_calls: usize,
    pub active_calls: usize,
    pub reserved_calls: usize,
    pub wrap_up_until: Option<DateTime<Utc>>,
}

impl AgentCapacity {
    pub fn single() -> Self {
        Self {
            max_active_calls: 1,
            active_calls: 0,
            reserved_calls: 0,
            wrap_up_until: None,
        }
    }

    pub fn available_slots(&self) -> usize {
        self.max_active_calls
            .saturating_sub(self.active_calls + self.reserved_calls)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    pub id: QueueId,
    pub name: String,
    pub required_skills: Vec<Skill>,
    pub policy: QueuePolicy,
    pub max_size: Option<usize>,
    pub max_wait: Option<Duration>,
    pub overflow: Option<OverflowPolicy>,
}

impl Queue {
    pub fn new(id: impl Into<QueueId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            required_skills: Vec::new(),
            policy: QueuePolicy::Fifo,
            max_size: None,
            max_wait: None,
            overflow: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedCall {
    pub call_id: CallId,
    pub queue_id: QueueId,
    pub priority: CallPriority,
    pub required_skills: Vec<Skill>,
    pub enqueued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub previous_agent_ids: Vec<AgentId>,
    pub attempt_count: u32,
    pub escalation_reason: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueTarget {
    pub queue_id: QueueId,
    pub priority: Option<CallPriority>,
    pub required_skills: Vec<Skill>,
    pub previous_agent_ids: Vec<AgentId>,
    pub metadata: HashMap<String, String>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueuePolicy {
    Fifo,
    Priority,
    LeastBusy,
    LongestIdle,
    SkillBased,
    AiFirstThenHuman,
    HumanFirstThenAi,
}

impl Default for QueuePolicy {
    fn default() -> Self {
        Self::Fifo
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueStats {
    pub queue_id: QueueId,
    pub queued_calls: usize,
    pub oldest_wait: Option<Duration>,
    pub average_wait: Option<Duration>,
    pub available_agents: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOffer {
    pub id: AgentOfferId,
    pub call_id: CallId,
    pub queue_id: Option<QueueId>,
    pub agent_id: AgentId,
    pub reservation_id: Option<ReservationId>,
    pub status: AgentOfferStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub agent_leg_id: Option<CallLegId>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentOfferStatus {
    Reserved,
    Pending,
    Accepted,
    Rejected,
    TimedOut,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferTarget {
    Queue(QueueId),
    Agent(AgentId),
    SipUri(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverflowTarget {
    Queue(QueueId),
    SipUri(String),
    Hangup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverflowPolicy {
    pub target: OverflowTarget,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssignmentFailureAction {
    Requeue { priority_boost: Option<i16> },
    TryNextAgent,
    OverflowToQueue { queue_id: QueueId },
    TransferToSipUri { uri: String },
    Reject { status: u16, reason: String },
    Hangup { reason: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEligibilityRequest {
    pub queue_id: Option<QueueId>,
    pub required_skills: Vec<Skill>,
    pub excluded_agent_ids: Vec<AgentId>,
    pub preferred_kind: Option<AgentKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedContact {
    pub uri: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub source: ContactSource,
    pub transport: Option<Transport>,
    pub received: Option<String>,
    pub path: Vec<String>,
    pub instance_id: Option<String>,
    pub reg_id: Option<u32>,
    pub flow_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactSource {
    Static,
    Registrar,
    Custom,
}
