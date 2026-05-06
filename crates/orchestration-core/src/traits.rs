use crate::error::{OrchestrationError, Result};
use crate::ids::*;
use crate::types::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[async_trait]
pub trait Router: Send + Sync {
    async fn route(&self, request: RouteRequest) -> Result<RouteDecision>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteRequest {
    pub call_id: CallId,
    pub from: String,
    pub to: String,
    pub sip_call_id: Option<String>,
    pub caller_identity: CallerIdentity,
    pub priority: CallPriority,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteDecision {
    Reject { status: u16, reason: String },
    Queue { queue_id: QueueId },
    OfferAgent { agent_id: AgentId },
    DialSipUri { uri: String },
}

#[derive(Debug, Clone)]
pub struct StaticRouter {
    decision: RouteDecision,
}

impl StaticRouter {
    pub fn new(decision: RouteDecision) -> Self {
        Self { decision }
    }
}

#[async_trait]
impl Router for StaticRouter {
    async fn route(&self, _request: RouteRequest) -> Result<RouteDecision> {
        Ok(self.decision.clone())
    }
}

#[async_trait]
pub trait QueueSelector: Send + Sync {
    async fn select_agent(
        &self,
        queued_call: &QueuedCall,
        candidates: Vec<Agent>,
    ) -> Result<Option<AgentId>>;
}

#[derive(Debug, Default, Clone)]
pub struct FirstAvailableSelector;

#[async_trait]
impl QueueSelector for FirstAvailableSelector {
    async fn select_agent(
        &self,
        queued_call: &QueuedCall,
        candidates: Vec<Agent>,
    ) -> Result<Option<AgentId>> {
        Ok(candidates
            .into_iter()
            .filter(|agent| agent.is_routable())
            .filter(|agent| agent.has_required_skills(&queued_call.required_skills))
            .find(|agent| !queued_call.previous_agent_ids.contains(&agent.id))
            .map(|agent| agent.id))
    }
}

#[async_trait]
pub trait ContactResolver: Send + Sync {
    async fn resolve_contact(&self, agent: &Agent) -> Result<ResolvedContact>;
}

#[derive(Debug, Default, Clone)]
pub struct StaticContactResolver;

#[async_trait]
impl ContactResolver for StaticContactResolver {
    async fn resolve_contact(&self, agent: &Agent) -> Result<ResolvedContact> {
        match &agent.connector {
            AgentConnector::SipUri(uri) => Ok(ResolvedContact {
                uri: uri.clone(),
                expires_at: None,
                source: ContactSource::Static,
            }),
            AgentConnector::RegisteredSipUser { aor } => {
                Err(OrchestrationError::ContactResolutionFailed(
                    agent.id.clone(),
                    format!("no registrar-backed resolver configured for {aor}"),
                ))
            }
            AgentConnector::LocalVoiceAi(_) => Err(OrchestrationError::ContactResolutionFailed(
                agent.id.clone(),
                "local voice AI agents do not have SIP contacts".to_string(),
            )),
            _ => Err(OrchestrationError::ContactResolutionFailed(
                agent.id.clone(),
                "unsupported agent connector".to_string(),
            )),
        }
    }
}
