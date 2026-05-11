use crate::assignment::{Assignment, AssignmentManager};
use crate::config::OrchestrationConfig;
use crate::error::{OrchestrationError, Result};
use crate::events::{OrchestrationEvent, OrchestrationEventBus};
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::global_coordinator;
use crate::ids::*;
use crate::store::*;
use crate::traits::{agent_contact_request, RouteDecision, RouteRequest, Router, StaticRouter};
use rvoip_sip::server::{ContactResolver, StaticContactResolver};
use crate::types::*;
use crate::voice_ai::{TranscriptEvent, VoiceAiAction, VoiceAiRuntime};
use chrono::{Duration as ChronoDuration, Utc};
use rvoip_sip::types::IncomingCallInfo;
use rvoip_sip::{
    CallState, Config, Event, EventReceiver, SessionId, UnifiedCoordinator,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};
use tokio::time::{sleep, timeout, Instant};

// `BridgeHandle` is media-core's owned bridge resource (drop tears down).
// Lifted in CARVE_PLAN step 3: orchestration-core no longer defines its own
// `BridgeManager`; both come from `rvoip-core::bridge`. The `BridgeManager`
// type alias preserves the orchestration-core call signature
// (`BridgeManager<CallId, BridgeId>`) so existing call sites keep working.
pub use rvoip_core::bridge::BridgeHandle;
pub type BridgeManager = rvoip_core::bridge::BridgeManager<CallId, BridgeId>;

enum AgentOfferOutcome {
    Answered,
    Failed { status_code: u16, reason: String },
}

fn agent_leg_role(agent: &Agent) -> CallLegRole {
    match agent.kind {
        AgentKind::Human => CallLegRole::HumanAgent,
        AgentKind::VoiceAi => CallLegRole::VoiceAiAgent,
    }
}

async fn wait_for_session_active(
    coordinator: &UnifiedCoordinator,
    session_id: &SessionId,
    deadline: Duration,
) -> Result<()> {
    let end = Instant::now() + deadline;
    loop {
        let state = coordinator.get_state(session_id).await?;
        if matches!(state, CallState::Active | CallState::Bridged) {
            return Ok(());
        }
        if Instant::now() >= end {
            return Err(OrchestrationError::BridgeFailed(format!(
                "session {session_id} did not become active before bridge (state: {state:?})"
            )));
        }
        sleep(Duration::from_millis(50)).await;
    }
}

async fn next_terminal_event(events: &mut EventReceiver) -> Option<String> {
    while let Some(event) = events.next().await {
        match event {
            Event::CallEnded { reason, .. } => return Some(reason),
            Event::CallFailed { reason, .. } => return Some(reason),
            Event::CallCancelled { .. } => return Some("call cancelled".to_string()),
            _ => {}
        }
    }
    None
}

/// Per-process registry of voice-AI runtimes keyed by `VoiceAiId`.
///
/// Backed by `DashMap` — reads and writes are lock-free at the entry level.
/// Will move to `rvoip-harness` post-migration; the access pattern is
/// preserved so the move is mechanical.
#[derive(Clone, Default)]
pub struct VoiceAiRegistry {
    runtimes: Arc<DashMap<VoiceAiId, VoiceAiRuntime>>,
}

impl VoiceAiRegistry {
    pub fn insert(&self, id: VoiceAiId, runtime: VoiceAiRuntime) {
        self.runtimes.insert(id, runtime);
    }

    pub fn get(&self, id: &VoiceAiId) -> Option<VoiceAiRuntime> {
        self.runtimes.get(id).map(|entry| entry.clone())
    }
}

pub struct Orchestrator {
    coordinator: Option<Arc<UnifiedCoordinator>>,
    /// CARVE_PLAN step 9: cross-transport `Orchestrator` from rvoip-core,
    /// auto-constructed when a `UnifiedCoordinator` is provided. A
    /// `SipAdapter` wrapping the same coordinator is registered so the
    /// `Orchestrator → SipAdapter → UnifiedCoordinator → dialog/media`
    /// dispatch path is live alongside the workforce-flavored direct
    /// coordinator path. Future cross-transport adapters
    /// (rvoip-webrtc, rvoip-quic) register against this same handle.
    rvoip_orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    calls: SharedCallStore,
    agents: SharedAgentStore,
    queues: SharedQueueStore,
    offers: SharedAgentOfferStore,
    router: Arc<dyn Router>,
    contact_resolver: Arc<dyn ContactResolver>,
    bridge_manager: BridgeManager,
    voice_ai: VoiceAiRegistry,
    events: OrchestrationEventBus,
    /// Bounds inbound call setups in flight. A new INVITE acquires a permit
    /// at the door of `handle_incoming_call`; if the semaphore is exhausted
    /// the call is rejected with SIP 503 instead of joining a death spiral.
    /// Permit is released when `handle_incoming_call` returns (success or
    /// failure).
    admission: Arc<Semaphore>,
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
            contact_resolver: self.contact_resolver.clone(),
            bridge_manager: self.bridge_manager.clone(),
            config: self.config.clone(),
        }
    }

    pub fn coordinator(&self) -> Option<&Arc<UnifiedCoordinator>> {
        self.coordinator.as_ref()
    }

    /// Cross-transport orchestrator handle (CARVE_PLAN step 9). `Some` when a
    /// `UnifiedCoordinator` was provided to the builder, in which case a
    /// `SipAdapter` wrapping that coordinator was auto-registered. Consumers
    /// that want to dispatch through the cross-transport seam (e.g. before
    /// adding a future WebRTC or QUIC adapter) hold this handle and use its
    /// methods (`originate_connection`, `route_inbound_connection`, `end_connection`,
    /// `transfer_connection`, `hold`, `resume`, etc.).
    pub fn rvoip_orchestrator(&self) -> Option<&Arc<rvoip_core::Orchestrator>> {
        self.rvoip_orchestrator.as_ref()
    }

    pub fn events(&self) -> OrchestrationEventBus {
        self.events.clone()
    }

    pub fn voice_ai(&self) -> VoiceAiRegistry {
        self.voice_ai.clone()
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let coordinator = self
            .coordinator
            .as_ref()
            .ok_or_else(|| OrchestrationError::InvalidState("no session coordinator".into()))?
            .clone();

        while let Some(incoming) = coordinator.get_incoming_call().await {
            let me = self.clone();
            tokio::spawn(async move {
                if let Err(error) = me.handle_incoming_call(incoming).await {
                    eprintln!("orchestration-core: inbound call handling failed: {error}");
                }
            });
        }

        Ok(())
    }

    pub async fn handle_incoming_call(&self, incoming: IncomingCallInfo) -> Result<CallId> {
        // Admission gate — try to acquire a permit at the door. If the
        // orchestrator is at its setup-concurrency limit, reject SIP 503
        // before any work is done. The permit is held for the duration of
        // this function and released on return.
        let _permit = match self.admission.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let limit = self.config.inbound.max_concurrent_setups;
                if let Some(coordinator) = &self.coordinator {
                    let _ = coordinator
                        .reject_call(&incoming.session_id, 503, "Service Unavailable")
                        .await;
                }
                return Err(OrchestrationError::AdmissionRejected(limit));
            }
        };

        let call = self.create_inbound_call(&incoming);
        let call_id = call.id.clone();
        let from_status = call.status;

        self.calls.insert_call(call.clone()).await?;
        self.events.emit(OrchestrationEvent::InboundCallReceived {
            call_id: call_id.clone(),
            caller: call.caller.clone(),
            to: call.dialed_uri.clone(),
        });
        self.events.emit(OrchestrationEvent::CallCreated { call });

        self.set_call_status(&call_id, from_status, CallStatus::Routing)
            .await?;

        let request = RouteRequest {
            call_id: call_id.clone(),
            from: incoming.from.clone(),
            to: incoming.to.clone(),
            sip_call_id: Some(incoming.call_id.clone()),
            caller_identity: CallerIdentity {
                uri: incoming.from,
                display_name: None,
                asserted_identity: incoming.p_asserted_identity,
                metadata: HashMap::new(),
            },
            priority: CallPriority::default(),
            metadata: HashMap::new(),
        };

        let decision = match timeout(
            self.config.inbound.route_timeout,
            self.router.route(request),
        )
        .await
        {
            Ok(Ok(decision)) => decision,
            Ok(Err(error)) => {
                self.fail_inbound_call(&call_id, &incoming.session_id, error.to_string())
                    .await?;
                return Err(error);
            }
            Err(_) => {
                let reason = "inbound route timeout".to_string();
                self.fail_inbound_call(&call_id, &incoming.session_id, reason.clone())
                    .await?;
                return Err(OrchestrationError::RoutingFailed(reason));
            }
        };

        self.apply_inbound_route_decision(&call_id, &incoming.session_id, decision)
            .await?;
        Ok(call_id)
    }

    fn create_inbound_call(&self, incoming: &IncomingCallInfo) -> Call {
        let mut call = Call::inbound(
            CallerIdentity {
                uri: incoming.from.clone(),
                display_name: None,
                asserted_identity: incoming.p_asserted_identity.clone(),
                metadata: HashMap::new(),
            },
            incoming.to.clone(),
        );
        call.sip_call_id = Some(incoming.call_id.clone());

        let mut caller_leg = CallLeg::new(
            CallLegRole::Caller,
            incoming.session_id.clone(),
            incoming.from.clone(),
        );
        caller_leg.sip_call_id = Some(incoming.call_id.clone());
        caller_leg.status = CallLegStatus::Ringing;
        call.legs.push(caller_leg);

        call
    }

    async fn apply_inbound_route_decision(
        &self,
        call_id: &CallId,
        session_id: &rvoip_sip::SessionId,
        decision: RouteDecision,
    ) -> Result<()> {
        let handle = self.handle();
        match decision {
            RouteDecision::Reject { status, reason } => {
                if let Some(coordinator) = &self.coordinator {
                    coordinator.reject_call(session_id, status, &reason).await?;
                }
                self.finish_rejected_call(call_id, status, reason).await?;
            }
            RouteDecision::Queue { queue_id } => {
                handle
                    .enqueue_call(
                        call_id.clone(),
                        QueueTarget {
                            queue_id,
                            ..QueueTarget::default()
                        },
                    )
                    .await?;
            }
            RouteDecision::OfferAgent { agent_id } => {
                handle.offer_agent(call_id.clone(), agent_id).await?;
            }
            RouteDecision::DialSipUri { uri } => {
                let reason = format!(
                    "DialSipUri route to {uri} is unsupported until outbound dialing is implemented"
                );
                self.mark_call_failed(call_id, reason).await?;
            }
        }
        Ok(())
    }

    async fn set_call_status(
        &self,
        call_id: &CallId,
        from: CallStatus,
        to: CallStatus,
    ) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        call.status = to;
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: call_id.clone(),
            from,
            to,
        });
        Ok(())
    }

    async fn finish_rejected_call(
        &self,
        call_id: &CallId,
        status: u16,
        reason: String,
    ) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::Ended;
        call.ended_at = Some(Utc::now());
        call.disposition = Some(CallDisposition::Rejected {
            status,
            reason: reason.clone(),
        });
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::CallEnded {
            call_id: call_id.clone(),
            reason,
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: call_id.clone(),
            from,
            to: CallStatus::Ended,
        });
        Ok(())
    }

    async fn fail_inbound_call(
        &self,
        call_id: &CallId,
        session_id: &rvoip_sip::SessionId,
        reason: String,
    ) -> Result<()> {
        if self.config.routing.fail_closed {
            if let Some(coordinator) = &self.coordinator {
                let _ = coordinator
                    .reject_call(session_id, 500, "Routing Failed")
                    .await;
            }
        }
        self.mark_call_failed(call_id, reason).await
    }

    async fn mark_call_failed(&self, call_id: &CallId, reason: String) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::Failed;
        call.ended_at = Some(Utc::now());
        call.disposition = Some(CallDisposition::Failed {
            reason: reason.clone(),
        });
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::CallFailed {
            call_id: call_id.clone(),
            reason,
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: call_id.clone(),
            from,
            to: CallStatus::Failed,
        });
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

        // CARVE_PLAN step 9: when a UnifiedCoordinator is provided, wrap it
        // in a SipAdapter and register against a fresh rvoip_core::Orchestrator.
        // This is the proof-of-life for the cross-transport seam — the SIP
        // path can dispatch through Orchestrator → SipAdapter → UnifiedCoordinator
        // alongside the existing workforce-flavored direct-coordinator path.
        let rvoip_orchestrator = if let Some(coord) = coordinator.as_ref() {
            let adapter = rvoip_sip::SipAdapter::new(coord.clone()).await?;
            let rvoip_orch = rvoip_core::Orchestrator::new(rvoip_core::Config::default());
            rvoip_orch.register(adapter).map_err(|err| {
                OrchestrationError::InvalidState(format!(
                    "failed to register SipAdapter with rvoip_core::Orchestrator: {err}"
                ))
            })?;
            Some(rvoip_orch)
        } else {
            None
        };

        let voice_ai = VoiceAiRegistry::default();
        for (id, runtime) in self.voice_ai_runtimes {
            voice_ai.insert(id, runtime);
        }

        Ok(Orchestrator {
            coordinator,
            rvoip_orchestrator,
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
            events: OrchestrationEventBus::with_coordinator(
                self.config.events.channel_capacity,
                global_coordinator().await.clone(),
            ),
            admission: Arc::new(Semaphore::new(self.config.inbound.max_concurrent_setups)),
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
    contact_resolver: Arc<dyn ContactResolver>,
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

    pub async fn create_call(&self, call: Call) -> Result<()> {
        self.calls.insert_call(call.clone()).await?;
        self.events.emit(OrchestrationEvent::CallCreated { call });
        Ok(())
    }

    pub async fn upsert_agent(&self, agent: Agent) -> Result<()> {
        self.agents.upsert_agent(agent).await
    }

    pub async fn upsert_queue(&self, queue: Queue) -> Result<()> {
        self.queues.upsert_queue(queue).await
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
            previous_agent_ids: target.previous_agent_ids,
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
        let mut call = self
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
        self.agents
            .update_state(&agent_id, AgentState::Offering)
            .await?;
        let from = call.status;
        call.status = CallStatus::OfferingAgent;
        call.assigned_agent_id = Some(agent_id.clone());
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::AgentReserved {
            call_id: call_id.clone(),
            agent_id: agent_id.clone(),
            offer_id: offer_id.clone(),
        });
        self.events.emit(OrchestrationEvent::AgentStateChanged {
            agent_id: agent_id.clone(),
            from: AgentState::Reserved,
            to: AgentState::Offering,
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id,
            from,
            to: CallStatus::OfferingAgent,
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

    pub async fn assign_and_connect_next_call(
        &self,
        queue_id: &QueueId,
    ) -> Result<Option<Assignment>> {
        let Some(assignment) = self.assign_next_call(queue_id).await? else {
            return Ok(None);
        };
        self.connect_agent_offer(&assignment.offer_id).await?;
        Ok(Some(assignment))
    }

    pub async fn accept_offer(&self, offer_id: &AgentOfferId) -> Result<()> {
        AssignmentManager::new(
            self.calls.clone(),
            self.agents.clone(),
            self.queues.clone(),
            self.offers.clone(),
            self.events.clone(),
            self.config.assignment.clone(),
        )
        .accept_offer(offer_id)
        .await
    }

    pub async fn fail_offer(
        &self,
        offer_id: &AgentOfferId,
        status: AgentOfferStatus,
        reason: impl Into<String>,
    ) -> Result<()> {
        AssignmentManager::new(
            self.calls.clone(),
            self.agents.clone(),
            self.queues.clone(),
            self.offers.clone(),
            self.events.clone(),
            self.config.assignment.clone(),
        )
        .fail_offer(offer_id, status, reason)
        .await
    }

    pub async fn connect_agent_offer(&self, offer_id: &AgentOfferId) -> Result<CallLegId> {
        let coordinator = self
            .coordinator
            .as_ref()
            .ok_or_else(|| OrchestrationError::InvalidState("no session coordinator".into()))?;
        let mut offer = self
            .offers
            .get_offer(offer_id)
            .await?
            .ok_or_else(|| OrchestrationError::Store(format!("offer not found: {offer_id}")))?;
        let agent = self
            .agents
            .get_agent(&offer.agent_id)
            .await?
            .ok_or_else(|| OrchestrationError::AgentNotFound(offer.agent_id.clone()))?;

        if matches!(agent.connector, AgentConnector::LocalVoiceAi(_)) {
            return Err(OrchestrationError::VoiceAiFailed(format!(
                "local voice AI agent {} does not create an outbound SIP leg",
                agent.id
            )));
        }

        let request = agent_contact_request(&agent)?;
        let contact = self
            .contact_resolver
            .resolve_contact(&request)
            .await
            .map_err(|err| {
                OrchestrationError::ContactResolutionFailed(agent.id.clone(), err.to_string())
            })?;
        let mut call = self
            .calls
            .get_call(&offer.call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(offer.call_id.clone()))?;
        let from_status = call.status;
        let from_agent_state = agent.state;
        let from_uri = self.config.session.local_uri.clone();

        let session_id = coordinator.make_call(&from_uri, &contact.uri).await?;
        let mut agent_leg = CallLeg::new(agent_leg_role(&agent), session_id, contact.uri.clone());
        agent_leg.status = CallLegStatus::Dialing;
        agent_leg.agent_id = Some(agent.id.clone());
        let agent_leg_id = agent_leg.id.clone();

        call.status = CallStatus::ConnectingAgent;
        call.assigned_agent_id = Some(agent.id.clone());
        call.legs.push(agent_leg);
        self.calls.update_call(call).await?;

        offer.status = AgentOfferStatus::Pending;
        offer.agent_leg_id = Some(agent_leg_id.clone());
        self.offers.update_offer(offer.clone()).await?;
        self.agents
            .update_state(&offer.agent_id, AgentState::Ringing)
            .await?;

        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: offer.call_id.clone(),
            from: from_status,
            to: CallStatus::ConnectingAgent,
        });
        self.events.emit(OrchestrationEvent::AgentStateChanged {
            agent_id: offer.agent_id.clone(),
            from: from_agent_state,
            to: AgentState::Ringing,
        });

        Ok(agent_leg_id)
    }

    pub async fn wait_for_agent_offer_outcome(
        &self,
        offer_id: &AgentOfferId,
    ) -> Result<Option<BridgeId>> {
        let coordinator = self
            .coordinator
            .as_ref()
            .ok_or_else(|| OrchestrationError::InvalidState("no session coordinator".into()))?;
        let mut current_offer_id = offer_id.clone();
        loop {
            let offer = self
                .offers
                .get_offer(&current_offer_id)
                .await?
                .ok_or_else(|| {
                    OrchestrationError::Store(format!("offer not found: {current_offer_id}"))
                })?;
            let agent_session_id =
                self.agent_session_id_for_offer(&offer)
                    .await?
                    .ok_or_else(|| {
                        OrchestrationError::InvalidState(format!(
                            "offer {current_offer_id} has no agent leg"
                        ))
                    })?;
            let mut events = coordinator.events_for_session(&agent_session_id).await?;

            match timeout(self.config.assignment.outbound_answer_timeout, async {
                while let Some(event) = events.next().await {
                    match event {
                        Event::CallAnswered { .. } => return AgentOfferOutcome::Answered,
                        Event::CallFailed {
                            status_code,
                            reason,
                            ..
                        } => {
                            return AgentOfferOutcome::Failed {
                                status_code,
                                reason,
                            };
                        }
                        Event::CallEnded { reason, .. } => {
                            return AgentOfferOutcome::Failed {
                                status_code: 487,
                                reason,
                            };
                        }
                        Event::CallCancelled { .. } => {
                            return AgentOfferOutcome::Failed {
                                status_code: 487,
                                reason: "call cancelled".to_string(),
                            };
                        }
                        _ => {}
                    }
                }
                AgentOfferOutcome::Failed {
                    status_code: 500,
                    reason: "agent event stream closed".to_string(),
                }
            })
            .await
            {
                Ok(AgentOfferOutcome::Answered) => {
                    return self.bridge_agent_offer(&current_offer_id).await.map(Some);
                }
                Ok(AgentOfferOutcome::Failed { reason, .. }) => {
                    let Some(next_assignment) = self
                        .fail_agent_connection_and_retry_next(
                            &current_offer_id,
                            AgentOfferStatus::Failed,
                            reason,
                        )
                        .await?
                    else {
                        return Ok(None);
                    };
                    let Some(next_offer_id) =
                        self.connect_retry_assignment(next_assignment).await?
                    else {
                        return Ok(None);
                    };
                    current_offer_id = next_offer_id;
                }
                Err(_) => {
                    let Some(next_assignment) = self
                        .fail_agent_connection_and_retry_next(
                            &current_offer_id,
                            AgentOfferStatus::TimedOut,
                            "agent no answer",
                        )
                        .await?
                    else {
                        return Ok(None);
                    };
                    let Some(next_offer_id) =
                        self.connect_retry_assignment(next_assignment).await?
                    else {
                        return Ok(None);
                    };
                    current_offer_id = next_offer_id;
                }
            }
        }
    }

    pub async fn wait_for_single_agent_offer_outcome(
        &self,
        offer_id: &AgentOfferId,
    ) -> Result<Option<BridgeId>> {
        let coordinator = self
            .coordinator
            .as_ref()
            .ok_or_else(|| OrchestrationError::InvalidState("no session coordinator".into()))?;
        let offer = self
            .offers
            .get_offer(offer_id)
            .await?
            .ok_or_else(|| OrchestrationError::Store(format!("offer not found: {offer_id}")))?;
        let agent_session_id = self
            .agent_session_id_for_offer(&offer)
            .await?
            .ok_or_else(|| {
                OrchestrationError::InvalidState(format!("offer {offer_id} has no agent leg"))
            })?;
        let mut events = coordinator.events_for_session(&agent_session_id).await?;

        match timeout(self.config.assignment.outbound_answer_timeout, async {
            while let Some(event) = events.next().await {
                match event {
                    Event::CallAnswered { .. } => return AgentOfferOutcome::Answered,
                    Event::CallFailed {
                        status_code,
                        reason,
                        ..
                    } => {
                        return AgentOfferOutcome::Failed {
                            status_code,
                            reason,
                        }
                    }
                    Event::CallEnded { reason, .. } => {
                        return AgentOfferOutcome::Failed {
                            status_code: 487,
                            reason,
                        }
                    }
                    Event::CallCancelled { .. } => {
                        return AgentOfferOutcome::Failed {
                            status_code: 487,
                            reason: "call cancelled".to_string(),
                        }
                    }
                    _ => {}
                }
            }
            AgentOfferOutcome::Failed {
                status_code: 500,
                reason: "agent event stream closed".to_string(),
            }
        })
        .await
        {
            Ok(AgentOfferOutcome::Answered) => self.bridge_agent_offer(offer_id).await.map(Some),
            Ok(AgentOfferOutcome::Failed { reason, .. }) => {
                self.fail_agent_connection(offer_id, AgentOfferStatus::Failed, reason)
                    .await?;
                Ok(None)
            }
            Err(_) => {
                self.fail_agent_connection(offer_id, AgentOfferStatus::TimedOut, "agent no answer")
                    .await?;
                Ok(None)
            }
        }
    }

    pub async fn bridge_agent_offer(&self, offer_id: &AgentOfferId) -> Result<BridgeId> {
        let coordinator = self
            .coordinator
            .as_ref()
            .ok_or_else(|| OrchestrationError::InvalidState("no session coordinator".into()))?;
        let offer = self
            .offers
            .get_offer(offer_id)
            .await?
            .ok_or_else(|| OrchestrationError::Store(format!("offer not found: {offer_id}")))?;
        let call = self
            .calls
            .get_call(&offer.call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(offer.call_id.clone()))?;
        let caller_leg = call
            .legs
            .iter()
            .find(|leg| leg.role == CallLegRole::Caller)
            .cloned()
            .ok_or_else(|| {
                OrchestrationError::InvalidState(format!(
                    "call {} has no caller leg to bridge",
                    offer.call_id
                ))
            })?;
        let agent_leg = offer
            .agent_leg_id
            .as_ref()
            .and_then(|agent_leg_id| call.legs.iter().find(|leg| &leg.id == agent_leg_id))
            .cloned()
            .ok_or_else(|| {
                OrchestrationError::InvalidState(format!(
                    "offer {offer_id} has no recorded agent leg"
                ))
            })?;

        if self.config.inbound.auto_accept_before_bridge {
            coordinator.accept_call(&caller_leg.session_id).await?;
        }
        wait_for_session_active(coordinator, &caller_leg.session_id, Duration::from_secs(5))
            .await?;
        wait_for_session_active(coordinator, &agent_leg.session_id, Duration::from_secs(5)).await?;

        let bridge_handle = coordinator
            .bridge(&caller_leg.session_id, &agent_leg.session_id)
            .await
            .map_err(|error| OrchestrationError::BridgeFailed(error.to_string()))?;
        let bridge_id = BridgeId::new();
        self.bridge_manager
            .insert(bridge_id.clone(), offer.call_id.clone(), bridge_handle);
        self.accept_offer(offer_id).await?;
        self.mark_bridge_started(&offer.call_id, &caller_leg.id, &agent_leg.id, &bridge_id)
            .await?;
        self.spawn_bridge_teardown_watch(
            offer.call_id.clone(),
            offer.agent_id.clone(),
            bridge_id.clone(),
            caller_leg.id.clone(),
            caller_leg.session_id.clone(),
            agent_leg.id.clone(),
            agent_leg.session_id.clone(),
        );

        Ok(bridge_id)
    }

    pub async fn fail_agent_connection(
        &self,
        offer_id: &AgentOfferId,
        status: AgentOfferStatus,
        reason: impl Into<String>,
    ) -> Result<()> {
        let reason = reason.into();
        let offer = self
            .offers
            .get_offer(offer_id)
            .await?
            .ok_or_else(|| OrchestrationError::Store(format!("offer not found: {offer_id}")))?;

        self.hangup_agent_session_for_offer(&offer).await?;
        self.mark_agent_leg_terminal(&offer, CallLegStatus::Failed)
            .await?;
        self.fail_offer(offer_id, status, reason.clone()).await?;

        if let Some(queue_id) = offer.queue_id.clone() {
            self.clear_assignment_for_retry(&offer.call_id).await?;
            let previous_agent_ids = self.failed_agent_ids_for_call(&offer.call_id).await?;
            self.enqueue_call(
                offer.call_id,
                QueueTarget {
                    queue_id,
                    previous_agent_ids,
                    ..QueueTarget::default()
                },
            )
            .await?;
        } else {
            self.mark_call_failed(&offer.call_id, reason).await?;
        }

        Ok(())
    }

    pub async fn fail_agent_connection_and_retry_next(
        &self,
        offer_id: &AgentOfferId,
        status: AgentOfferStatus,
        reason: impl Into<String>,
    ) -> Result<Option<Assignment>> {
        let reason = reason.into();
        let offer = self
            .offers
            .get_offer(offer_id)
            .await?
            .ok_or_else(|| OrchestrationError::Store(format!("offer not found: {offer_id}")))?;

        self.hangup_agent_session_for_offer(&offer).await?;
        self.mark_agent_leg_terminal(&offer, CallLegStatus::Failed)
            .await?;
        self.fail_offer(offer_id, status, reason.clone()).await?;

        let Some(queue_id) = offer.queue_id.clone() else {
            self.mark_call_failed(&offer.call_id, reason).await?;
            return Ok(None);
        };

        self.clear_assignment_for_retry(&offer.call_id).await?;
        let previous_agent_ids = self.failed_agent_ids_for_call(&offer.call_id).await?;
        self.enqueue_call(
            offer.call_id,
            QueueTarget {
                queue_id: queue_id.clone(),
                previous_agent_ids,
                ..QueueTarget::default()
            },
        )
        .await?;

        self.assign_next_call(&queue_id).await
    }

    async fn connect_retry_assignment(
        &self,
        mut assignment: Assignment,
    ) -> Result<Option<AgentOfferId>> {
        loop {
            match self.connect_agent_offer(&assignment.offer_id).await {
                Ok(_) => return Ok(Some(assignment.offer_id)),
                Err(error) => {
                    let Some(next_assignment) = self
                        .fail_agent_connection_and_retry_next(
                            &assignment.offer_id,
                            AgentOfferStatus::Failed,
                            format!("agent connection failed: {error}"),
                        )
                        .await?
                    else {
                        return Ok(None);
                    };
                    assignment = next_assignment;
                }
            }
        }
    }

    pub async fn apply_voice_ai_action(
        &self,
        call_id: CallId,
        agent_id: AgentId,
        action: VoiceAiAction,
    ) -> Result<()> {
        match action {
            VoiceAiAction::Continue => Ok(()),
            VoiceAiAction::Say { text } => {
                let mut call = self
                    .calls
                    .get_call(&call_id)
                    .await?
                    .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
                let from = call.status;
                call.status = CallStatus::InVoiceAi;
                call.assigned_agent_id = Some(agent_id.clone());
                call.context
                    .metadata
                    .insert("last_voice_ai_say".to_string(), text.clone());
                self.calls.update_call(call).await?;
                self.events.emit(OrchestrationEvent::VoiceAiTranscript {
                    call_id: call_id.clone(),
                    agent_id,
                    transcript: TranscriptEvent::Final {
                        text,
                        confidence: None,
                    },
                });
                if from != CallStatus::InVoiceAi {
                    self.events.emit(OrchestrationEvent::CallStatusChanged {
                        call_id,
                        from,
                        to: CallStatus::InVoiceAi,
                    });
                }
                Ok(())
            }
            VoiceAiAction::TransferToQueue { queue_id } => {
                self.end_voice_ai_session(&call_id, &agent_id, "transfer to queue")
                    .await?;
                self.prepare_call_for_handoff(&call_id, &agent_id).await?;
                self.enqueue_call(
                    call_id,
                    QueueTarget {
                        queue_id,
                        previous_agent_ids: vec![agent_id],
                        ..QueueTarget::default()
                    },
                )
                .await
            }
            VoiceAiAction::TransferToAgent {
                agent_id: target_agent_id,
            } => {
                self.end_voice_ai_session(&call_id, &agent_id, "transfer to agent")
                    .await?;
                self.prepare_call_for_handoff(&call_id, &agent_id).await?;
                self.offer_agent(call_id, target_agent_id).await.map(|_| ())
            }
            VoiceAiAction::TransferToSipUri { uri } => {
                self.end_voice_ai_session(&call_id, &agent_id, "transfer to SIP URI")
                    .await?;
                self.prepare_call_for_handoff(&call_id, &agent_id).await?;
                self.transfer_call(call_id, TransferTarget::SipUri(uri))
                    .await
            }
            VoiceAiAction::Hangup { reason } => {
                self.end_voice_ai_session(&call_id, &agent_id, &reason)
                    .await?;
                self.end_call(call_id, reason).await
            }
        }
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

    pub async fn get_agent(&self, agent_id: &AgentId) -> Result<Option<Agent>> {
        self.agents.get_agent(agent_id).await
    }

    pub async fn list_offers_for_call(&self, call_id: &CallId) -> Result<Vec<AgentOffer>> {
        self.offers.list_offers_for_call(call_id).await
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
        self.bridge_manager.len()
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

    async fn mark_bridge_started(
        &self,
        call_id: &CallId,
        caller_leg_id: &CallLegId,
        agent_leg_id: &CallLegId,
        bridge_id: &BridgeId,
    ) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::Connected;
        call.active_bridge_id = Some(bridge_id.clone());
        call.answered_at.get_or_insert_with(Utc::now);
        for leg in &mut call.legs {
            if &leg.id == caller_leg_id || &leg.id == agent_leg_id {
                leg.status = CallLegStatus::Bridged;
                leg.answered_at.get_or_insert_with(Utc::now);
            }
        }
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::BridgeStarted {
            call_id: call_id.clone(),
            bridge_id: bridge_id.clone(),
            caller_leg_id: caller_leg_id.clone(),
            agent_leg_id: agent_leg_id.clone(),
        });
        if from != CallStatus::Connected {
            self.events.emit(OrchestrationEvent::CallStatusChanged {
                call_id: call_id.clone(),
                from,
                to: CallStatus::Connected,
            });
        }
        Ok(())
    }

    fn spawn_bridge_teardown_watch(
        &self,
        call_id: CallId,
        agent_id: AgentId,
        bridge_id: BridgeId,
        caller_leg_id: CallLegId,
        caller_session_id: SessionId,
        agent_leg_id: CallLegId,
        agent_session_id: SessionId,
    ) {
        let handle = self.clone();
        tokio::spawn(async move {
            let _ = handle
                .watch_bridge_teardown(
                    call_id,
                    agent_id,
                    bridge_id,
                    caller_leg_id,
                    caller_session_id,
                    agent_leg_id,
                    agent_session_id,
                )
                .await;
        });
    }

    async fn watch_bridge_teardown(
        &self,
        call_id: CallId,
        agent_id: AgentId,
        bridge_id: BridgeId,
        caller_leg_id: CallLegId,
        caller_session_id: SessionId,
        agent_leg_id: CallLegId,
        agent_session_id: SessionId,
    ) -> Result<()> {
        let coordinator = self
            .coordinator
            .as_ref()
            .ok_or_else(|| OrchestrationError::InvalidState("no session coordinator".into()))?;
        let mut caller_events = coordinator.events_for_session(&caller_session_id).await?;
        let mut agent_events = coordinator.events_for_session(&agent_session_id).await?;

        let reason = tokio::select! {
            reason = next_terminal_event(&mut caller_events) => {
                let reason = reason.unwrap_or_else(|| "caller leg event stream closed".to_string());
                let _ = coordinator.hangup(&agent_session_id).await;
                format!("caller leg ended: {reason}")
            }
            reason = next_terminal_event(&mut agent_events) => {
                let reason = reason.unwrap_or_else(|| "agent leg event stream closed".to_string());
                let _ = coordinator.hangup(&caller_session_id).await;
                format!("agent leg ended: {reason}")
            }
        };

        self.finish_bridged_call(
            &call_id,
            &agent_id,
            &bridge_id,
            &caller_leg_id,
            &agent_leg_id,
            reason,
        )
        .await
    }

    async fn finish_bridged_call(
        &self,
        call_id: &CallId,
        agent_id: &AgentId,
        bridge_id: &BridgeId,
        caller_leg_id: &CallLegId,
        agent_leg_id: &CallLegId,
        reason: String,
    ) -> Result<()> {
        let _ = self.bridge_manager.remove(bridge_id);

        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        if matches!(call.status, CallStatus::Ended | CallStatus::Failed) {
            return Ok(());
        }
        let from = call.status;
        call.status = CallStatus::Ended;
        call.ended_at = Some(Utc::now());
        call.active_bridge_id = None;
        if call.disposition.is_none() {
            call.disposition = Some(CallDisposition::Completed);
        }
        for leg in &mut call.legs {
            if &leg.id == caller_leg_id || &leg.id == agent_leg_id {
                leg.status = CallLegStatus::Ended;
                leg.ended_at.get_or_insert_with(Utc::now);
            }
        }
        self.calls.update_call(call).await?;

        for offer in self.offers.list_offers_for_call(call_id).await? {
            if &offer.agent_id == agent_id && offer.status == AgentOfferStatus::Accepted {
                if let Some(reservation_id) = offer.reservation_id {
                    self.agents.release_capacity(&reservation_id).await?;
                }
            }
        }

        if let Some(agent) = self.agents.get_agent(agent_id).await? {
            let previous = agent.state;
            self.agents
                .update_state(agent_id, AgentState::WrapUp)
                .await?;
            self.events.emit(OrchestrationEvent::AgentStateChanged {
                agent_id: agent_id.clone(),
                from: previous,
                to: AgentState::WrapUp,
            });
        }

        self.events.emit(OrchestrationEvent::BridgeEnded {
            call_id: call_id.clone(),
            bridge_id: bridge_id.clone(),
            reason: reason.clone(),
        });
        self.events.emit(OrchestrationEvent::CallEnded {
            call_id: call_id.clone(),
            reason,
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: call_id.clone(),
            from,
            to: CallStatus::Ended,
        });
        Ok(())
    }

    async fn mark_agent_leg_terminal(
        &self,
        offer: &AgentOffer,
        status: CallLegStatus,
    ) -> Result<()> {
        let Some(agent_leg_id) = &offer.agent_leg_id else {
            return Ok(());
        };
        let mut call = self
            .calls
            .get_call(&offer.call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(offer.call_id.clone()))?;
        if let Some(leg) = call.legs.iter_mut().find(|leg| &leg.id == agent_leg_id) {
            leg.status = status;
            leg.ended_at = Some(Utc::now());
        }
        self.calls.update_call(call).await?;
        Ok(())
    }

    async fn clear_assignment_for_retry(&self, call_id: &CallId) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        call.assigned_agent_id = None;
        self.calls.update_call(call).await?;
        Ok(())
    }

    async fn mark_call_failed(&self, call_id: &CallId, reason: String) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::Failed;
        call.ended_at = Some(Utc::now());
        call.disposition = Some(CallDisposition::Failed {
            reason: reason.clone(),
        });
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::CallFailed {
            call_id: call_id.clone(),
            reason,
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: call_id.clone(),
            from,
            to: CallStatus::Failed,
        });
        Ok(())
    }

    async fn agent_session_id_for_offer(&self, offer: &AgentOffer) -> Result<Option<SessionId>> {
        let Some(agent_leg_id) = &offer.agent_leg_id else {
            return Ok(None);
        };
        Ok(self.calls.get_call(&offer.call_id).await?.and_then(|call| {
            call.legs
                .into_iter()
                .find(|leg| &leg.id == agent_leg_id)
                .map(|leg| leg.session_id)
        }))
    }

    async fn hangup_agent_session_for_offer(&self, offer: &AgentOffer) -> Result<()> {
        let Some(coordinator) = &self.coordinator else {
            return Ok(());
        };
        let Some(session_id) = self.agent_session_id_for_offer(offer).await? else {
            return Ok(());
        };

        let _ = coordinator.hangup(&session_id).await;
        Ok(())
    }

    async fn failed_agent_ids_for_call(&self, call_id: &CallId) -> Result<Vec<AgentId>> {
        let mut agent_ids = Vec::new();
        for offer in self.offers.list_offers_for_call(call_id).await? {
            if matches!(
                offer.status,
                AgentOfferStatus::Rejected
                    | AgentOfferStatus::TimedOut
                    | AgentOfferStatus::Cancelled
                    | AgentOfferStatus::Failed
            ) && !agent_ids.contains(&offer.agent_id)
            {
                agent_ids.push(offer.agent_id);
            }
        }
        Ok(agent_ids)
    }

    async fn prepare_call_for_handoff(&self, call_id: &CallId, agent_id: &AgentId) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        call.assigned_agent_id = None;
        call.context
            .metadata
            .insert("handoff_from_agent_id".to_string(), agent_id.to_string());
        self.calls.update_call(call).await?;
        Ok(())
    }

    async fn end_voice_ai_session(
        &self,
        call_id: &CallId,
        agent_id: &AgentId,
        reason: &str,
    ) -> Result<()> {
        let offers = self.offers.list_offers_for_call(call_id).await?;
        for offer in offers {
            if &offer.agent_id == agent_id {
                if let Some(reservation_id) = offer.reservation_id {
                    self.agents.release_capacity(&reservation_id).await?;
                }
            }
        }
        if self.agents.get_agent(agent_id).await?.is_some() {
            self.agents
                .update_state(agent_id, AgentState::Available)
                .await?;
        }
        self.events.emit(OrchestrationEvent::VoiceAiEnded {
            call_id: call_id.clone(),
            agent_id: agent_id.clone(),
            reason: reason.to_string(),
        });
        Ok(())
    }
}
