use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OrchestrationConfig {
    pub session: rvoip_session_core::Config,
    pub inbound: InboundCallConfig,
    pub routing: RoutingConfig,
    pub queues: QueueConfig,
    pub agents: AgentConfig,
    pub assignment: AssignmentConfig,
    pub contacts: ContactConfig,
    pub voice_ai: VoiceAiConfig,
    pub recording: RecordingConfig,
    pub events: EventConfig,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            session: rvoip_session_core::Config::default(),
            inbound: InboundCallConfig::default(),
            routing: RoutingConfig::default(),
            queues: QueueConfig::default(),
            agents: AgentConfig::default(),
            assignment: AssignmentConfig::default(),
            contacts: ContactConfig::default(),
            voice_ai: VoiceAiConfig::default(),
            recording: RecordingConfig::default(),
            events: EventConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InboundCallConfig {
    pub route_timeout: Duration,
    pub auto_accept_before_bridge: bool,
}

impl Default for InboundCallConfig {
    fn default() -> Self {
        Self {
            route_timeout: Duration::from_secs(5),
            auto_accept_before_bridge: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoutingConfig {
    pub fail_closed: bool,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self { fail_closed: true }
    }
}

#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub default_max_wait: Option<Duration>,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            default_max_wait: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub default_wrap_up_timeout: Duration,
    pub heartbeat_timeout: Option<Duration>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_wrap_up_timeout: Duration::from_secs(10),
            heartbeat_timeout: Some(Duration::from_secs(90)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AssignmentConfig {
    pub offer_timeout: Duration,
    pub outbound_answer_timeout: Duration,
    pub max_attempts_per_call: u32,
}

impl Default for AssignmentConfig {
    fn default() -> Self {
        Self {
            offer_timeout: Duration::from_secs(30),
            outbound_answer_timeout: Duration::from_secs(30),
            max_attempts_per_call: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContactConfig {
    pub mark_unresolved_agents_away: bool,
}

impl Default for ContactConfig {
    fn default() -> Self {
        Self {
            mark_unresolved_agents_away: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VoiceAiConfig {
    pub allow_early_media: bool,
    pub enable_barge_in: bool,
}

impl Default for VoiceAiConfig {
    fn default() -> Self {
        Self {
            allow_early_media: false,
            enable_barge_in: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RecordingConfig {
    pub enabled_by_default: bool,
}

#[derive(Debug, Clone)]
pub struct EventConfig {
    pub channel_capacity: usize,
}

impl Default for EventConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 1024,
        }
    }
}
