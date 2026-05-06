use crate::assignment::{Assignment, AssignmentManager};
use crate::config::OrchestrationConfig;
use crate::error::{OrchestrationError, Result};
use crate::events::{OrchestrationEvent, OrchestrationEventBus};
use crate::ids::*;
use crate::store::*;
use crate::traits::{ContactResolver, RouteDecision, Router, StaticContactResolver, StaticRouter};
use crate::types::*;
use crate::voice_ai::VoiceAiRuntime;
use chrono::{Duration as ChronoDuration, Utc};
use rvoip_session_core::{BridgeHandle, Config, UnifiedCoordinator};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct BridgeManager {
    bridges: Arc<RwLock<HashMap<BridgeId, BridgeHandle>>>,
}

impl BridgeManager {
    pub async fn insert(&self, bridge_id: BridgeId, handle: BridgeHandle) {
        self.bridges.write().await.insert(bridge_id, handle);
    }

    pub async fn remove(&self, bridge_id: &BridgeId) -> Option<BridgeHandle> {
        self.bridges.write().await.remove(bridge_id)
    }

    pub async fn len(&self) -> usize {
        self.bridges.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[derive(Clone, Default)]
pub struct VoiceAiRegistry {
    runtimes: Arc<RwLock<HashMap<VoiceAiId, VoiceAiRuntime>>>,
}

impl VoiceAiRegistry {
    pub async fn insert(&self, id: VoiceAiId, runtime: VoiceAiRuntime) {
        self.runtimes.write().await.insert(id, runtime);
    }

    pub async fn get(&self, id: &VoiceAiId) -> Option<VoiceAiRuntime> {
        self.runtimes.read().await.get(id).cloned()
    }
}

pub struct Orchestrator {
    coordinator: Option<Arc<UnifiedCoordinator>>,
    calls: SharedCallStore,
    agents: SharedAgentStore,
    queues: SharedQueueStore,
    offers: SharedAgentOfferStore,
    router: Arc<dyn Router>,
    contact_resolver: Arc<dyn ContactResolver>,
    bridge_manager: BridgeManager,
    voice_ai: VoiceAiRegistry,
    events: OrchestrationEventBus,
    config: OrchestrationConfig,
}

impl Orchestrator {
    pub fn builder() -> OrchestratorBuilder {
        OrchestratorBuilder::default()
    }

    pub fn handle(&self) -> OrchestrationHandle {
        OrchestrationHandle {
            coordinator: self.coordinator.clone(),
            calls: self.calls.clone(),
            agents: self.agents.clone(),
            queues: self.queues.clone(),
            offers: self.offers.clone(),
            events: self.events.clone(),
            bridge_manager: self.bridge_manager.clone(),
            config: self.config.clone(),
        }
    }

    pub fn coordinator(&self) -> Option<&Arc<UnifiedCoordinator>> {
        self.coordinator.as_ref()
    }

    pub fn events(&self) -> OrchestrationEventBus {
        self.events.clone()
    }

    pub fn voice_ai(&self) -> VoiceAiRegistry {
        self.voice_ai.clone()
    }

    pub async fn run(&self) -> Result<()> {
        Ok(())
    }
}

pub struct OrchestratorBuilder {
    config: OrchestrationConfig,
    coordinator: Option<Arc<UnifiedCoordinator>>,
    session_config: Option<Config>,
    calls: Option<SharedCallStore>,
    agents: Option<SharedAgentStore>,
    queues: Option<SharedQueueStore>,
    offers: Option<SharedAgentOfferStore>,
    router: Option<Arc<dyn Router>>,
    contact_resolver: Option<Arc<dyn ContactResolver>>,
    initial_agents: Vec<Agent>,
    initial_queues: Vec<Queue>,
    voice_ai_runtimes: Vec<(VoiceAiId, VoiceAiRuntime)>,
}

impl Default for OrchestratorBuilder {
    fn default() -> Self {
        Self {
            config: OrchestrationConfig::default(),
            coordinator: None,
            session_config: None,
            calls: None,
            agents: None,
            queues: None,
            offers: None,
            router: None,
            contact_resolver: None,
            initial_agents: Vec::new(),
            initial_queues: Vec::new(),
            voice_ai_runtimes: Vec::new(),
        }
    }
}

impl OrchestratorBuilder {
    pub fn with_config(mut self, config: OrchestrationConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_session_config(mut self, config: Config) -> Self {
        self.session_config = Some(config);
        self
    }

    pub fn with_coordinator(mut self, coordinator: Arc<UnifiedCoordinator>) -> Self {
        self.coordinator = Some(coordinator);
        self
    }

    pub fn with_call_store(mut self, store: SharedCallStore) -> Self {
        self.calls = Some(store);
        self
    }

    pub fn with_agent_store(mut self, store: SharedAgentStore) -> Self {
        self.agents = Some(store);
        self
    }

    pub fn with_queue_store(mut self, store: SharedQueueStore) -> Self {
        self.queues = Some(store);
        self
    }

    pub fn with_agent_offer_store(mut self, store: SharedAgentOfferStore) -> Self {
        self.offers = Some(store);
        self
    }

    pub fn with_router<R>(mut self, router: R) -> Self
    where
        R: Router + 'static,
    {
        self.router = Some(Arc::new(router));
        self
    }

    pub fn with_router_arc(mut self, router: Arc<dyn Router>) -> Self {
        self.router = Some(router);
        self
    }

    pub fn with_contact_resolver<R>(mut self, resolver: R) -> Self
    where
        R: ContactResolver + 'static,
    {
        self.contact_resolver = Some(Arc::new(resolver));
        self
    }

    pub fn with_agent(mut self, agent: Agent) -> Self {
        self.initial_agents.push(agent);
        self
    }

    pub fn with_queue(mut self, queue: Queue) -> Self {
        self.initial_queues.push(queue);
        self
    }

    pub fn with_voice_ai_runtime(
        mut self,
        id: impl Into<VoiceAiId>,
        runtime: VoiceAiRuntime,
    ) -> Self {
        self.voice_ai_runtimes.push((id.into(), runtime));
        self
    }

    pub async fn build(self) -> Result<Orchestrator> {
        let calls: SharedCallStore = self
            .calls
            .unwrap_or_else(|| Arc::new(MemoryCallStore::new()));
        let agents: SharedAgentStore = self
            .agents
            .unwrap_or_else(|| Arc::new(MemoryAgentStore::new()));
        let queues: SharedQueueStore = self
            .queues
            .unwrap_or_else(|| Arc::new(MemoryQueueStore::new()));
        let offers: SharedAgentOfferStore = self
            .offers
            .unwrap_or_else(|| Arc::new(MemoryAgentOfferStore::new()));

        for agent in self.initial_agents {
            agents.upsert_agent(agent).await?;
        }

        for queue in self.initial_queues {
            queues.upsert_queue(queue).await?;
        }

        let coordinator = match (self.coordinator, self.session_config) {
            (Some(coordinator), _) => Some(coordinator),
            (None, Some(config)) => Some(UnifiedCoordinator::new(config).await?),
            (None, None) => None,
        };

        let voice_ai = VoiceAiRegistry::default();
        for (id, runtime) in self.voice_ai_runtimes {
            voice_ai.insert(id, runtime).await;
        }

        Ok(Orchestrator {
            coordinator,
            calls,
            agents,
            queues,
            offers,
            router: self.router.unwrap_or_else(|| {
                Arc::new(StaticRouter::new(RouteDecision::Reject {
                    status: 503,
                    reason: "no router configured".to_string(),
                }))
            }),
            contact_resolver: self
                .contact_resolver
                .unwrap_or_else(|| Arc::new(StaticContactResolver)),
            bridge_manager: BridgeManager::default(),
            voice_ai,
            events: OrchestrationEventBus::new(self.config.events.channel_capacity),
            config: self.config,
        })
    }
}

#[derive(Clone)]
pub struct OrchestrationHandle {
    coordinator: Option<Arc<UnifiedCoordinator>>,
    calls: SharedCallStore,
    agents: SharedAgentStore,
    queues: SharedQueueStore,
    offers: SharedAgentOfferStore,
    events: OrchestrationEventBus,
    bridge_manager: BridgeManager,
    config: OrchestrationConfig,
}

impl OrchestrationHandle {
    pub fn events(&self) -> OrchestrationEventBus {
        self.events.clone()
    }

    pub fn coordinator(&self) -> Option<&Arc<UnifiedCoordinator>> {
        self.coordinator.as_ref()
    }

    pub async fn insert_call(&self, call: Call) -> Result<()> {
        self.calls.insert_call(call.clone()).await?;
        self.events.emit(OrchestrationEvent::CallCreated { call });
        Ok(())
    }

    pub async fn enqueue_call(&self, call_id: CallId, target: QueueTarget) -> Result<()> {
        let mut call = self
            .calls
            .get_call(&call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        let queue_id = target.queue_id.clone();
        let queued_call = QueuedCall {
            call_id: call_id.clone(),
            queue_id: queue_id.clone(),
            priority: target.priority.unwrap_or(call.priority),
            required_skills: target.required_skills,
            enqueued_at: Utc::now(),
            expires_at: None,
            previous_agent_ids: Vec::new(),
            attempt_count: 0,
            escalation_reason: None,
            metadata: target.metadata,
        };
        self.queues.enqueue(queued_call).await?;

        call.status = CallStatus::Queued;
        call.queue_id = Some(queue_id.clone());
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::CallQueued {
            call_id: call_id.clone(),
            queue_id,
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id,
            from,
            to: CallStatus::Queued,
        });
        Ok(())
    }

    pub async fn offer_agent(&self, call_id: CallId, agent_id: AgentId) -> Result<AgentOfferId> {
        let _call = self
            .calls
            .get_call(&call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let reservation_id = self
            .agents
            .reserve_capacity(&agent_id, &call_id)
            .await?
            .ok_or_else(|| OrchestrationError::AgentReservationFailed(agent_id.clone()))?;
        let offer_id = AgentOfferId::new();
        let expires_at = Utc::now()
            + ChronoDuration::from_std(self.config.assignment.offer_timeout)
                .unwrap_or_else(|_| ChronoDuration::seconds(30));
        let offer = AgentOffer {
            id: offer_id.clone(),
            call_id: call_id.clone(),
            queue_id: None,
            agent_id: agent_id.clone(),
            reservation_id: Some(reservation_id),
            status: AgentOfferStatus::Reserved,
            created_at: Utc::now(),
            expires_at,
            agent_leg_id: None,
            failure_reason: None,
        };
        self.offers.insert_offer(offer).await?;
        self.events.emit(OrchestrationEvent::AgentReserved {
            call_id,
            agent_id,
            offer_id: offer_id.clone(),
        });
        Ok(offer_id)
    }

    pub async fn assign_next_call(&self, queue_id: &QueueId) -> Result<Option<Assignment>> {
        AssignmentManager::new(
            self.calls.clone(),
            self.agents.clone(),
            self.queues.clone(),
            self.offers.clone(),
            self.events.clone(),
            self.config.assignment.clone(),
        )
        .assign_next(queue_id)
        .await
    }

    pub async fn transfer_call(&self, call_id: CallId, target: TransferTarget) -> Result<()> {
        let mut call = self
            .calls
            .get_call(&call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::Transferring;
        call.metrics.transfer_count += 1;
        let from_agent_id = call.assigned_agent_id.clone();
        self.calls.update_call(call).await?;
        if let Some(from_agent_id) = from_agent_id {
            self.events.emit(OrchestrationEvent::TransferRequested {
                call_id: call_id.clone(),
                from_agent_id,
                target,
            });
        }
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id,
            from,
            to: CallStatus::Transferring,
        });
        Ok(())
    }

    pub async fn hold_call(&self, call_id: CallId) -> Result<()> {
        self.transition_call(call_id, CallStatus::OnHold).await
    }

    pub async fn resume_call(&self, call_id: CallId) -> Result<()> {
        self.transition_call(call_id, CallStatus::Connected).await
    }

    pub async fn end_call(&self, call_id: CallId, reason: String) -> Result<()> {
        let mut call = self
            .calls
            .get_call(&call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::Ended;
        call.ended_at = Some(Utc::now());
        if call.disposition.is_none() {
            call.disposition = Some(CallDisposition::Completed);
        }
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::CallEnded {
            call_id: call_id.clone(),
            reason,
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id,
            from,
            to: CallStatus::Ended,
        });
        Ok(())
    }

    pub async fn update_agent_state(&self, agent_id: AgentId, state: AgentState) -> Result<()> {
        let previous = self
            .agents
            .get_agent(&agent_id)
            .await?
            .ok_or_else(|| OrchestrationError::AgentNotFound(agent_id.clone()))?
            .state;
        self.agents.update_state(&agent_id, state).await?;
        self.events.emit(OrchestrationEvent::AgentStateChanged {
            agent_id,
            from: previous,
            to: state,
        });
        Ok(())
    }

    pub async fn complete_wrap_up(&self, agent_id: AgentId, _call_id: CallId) -> Result<()> {
        self.update_agent_state(agent_id, AgentState::Available)
            .await
    }

    pub async fn get_call(&self, call_id: &CallId) -> Result<Option<Call>> {
        self.calls.get_call(call_id).await
    }

    pub async fn get_queue_stats(&self, queue_id: &QueueId) -> Result<QueueStats> {
        let mut stats = self.queues.stats(queue_id).await?;
        if let Some(queue) = self.queues.get_queue(queue_id).await? {
            stats.available_agents = self
                .agents
                .list_eligible_agents(AgentEligibilityRequest {
                    queue_id: Some(queue_id.clone()),
                    required_skills: queue.required_skills,
                    excluded_agent_ids: Vec::new(),
                    preferred_kind: None,
                })
                .await?
                .len();
        }
        Ok(stats)
    }

    pub async fn active_bridge_count(&self) -> usize {
        self.bridge_manager.len().await
    }

    async fn transition_call(&self, call_id: CallId, to: CallStatus) -> Result<()> {
        let mut call = self
            .calls
            .get_call(&call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = to;
        self.calls.update_call(call).await?;
        self.events
            .emit(OrchestrationEvent::CallStatusChanged { call_id, from, to });
        Ok(())
    }
}
