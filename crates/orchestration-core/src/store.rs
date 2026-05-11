use crate::error::{OrchestrationError, Result};
use crate::ids::*;
use crate::types::*;
use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_session_core::SessionId;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[async_trait]
pub trait CallStore: Send + Sync {
    async fn insert_call(&self, call: Call) -> Result<()>;
    async fn update_call(&self, call: Call) -> Result<()>;
    async fn get_call(&self, id: &CallId) -> Result<Option<Call>>;
    async fn get_call_by_session(&self, session_id: &SessionId) -> Result<Option<Call>>;
    async fn list_active_calls(&self) -> Result<Vec<Call>>;
}

#[async_trait]
pub trait AgentStore: Send + Sync {
    async fn upsert_agent(&self, agent: Agent) -> Result<()>;
    async fn get_agent(&self, id: &AgentId) -> Result<Option<Agent>>;
    async fn list_agents(&self) -> Result<Vec<Agent>>;
    async fn list_eligible_agents(&self, request: AgentEligibilityRequest) -> Result<Vec<Agent>>;
    async fn reserve_capacity(
        &self,
        id: &AgentId,
        call_id: &CallId,
    ) -> Result<Option<ReservationId>>;
    async fn activate_capacity(&self, reservation_id: &ReservationId) -> Result<()>;
    async fn release_capacity(&self, reservation_id: &ReservationId) -> Result<()>;
    async fn update_state(&self, id: &AgentId, state: AgentState) -> Result<()>;
}

#[async_trait]
pub trait QueueStore: Send + Sync {
    async fn upsert_queue(&self, queue: Queue) -> Result<()>;
    async fn get_queue(&self, id: &QueueId) -> Result<Option<Queue>>;
    async fn list_queues(&self) -> Result<Vec<Queue>>;
    async fn enqueue(&self, queued_call: QueuedCall) -> Result<()>;
    async fn remove_call(&self, call_id: &CallId) -> Result<Option<QueuedCall>>;
    async fn list_queued(&self, queue_id: &QueueId) -> Result<Vec<QueuedCall>>;
    async fn claim_for_agent(
        &self,
        queue_id: &QueueId,
        agent: &Agent,
        reservation_id: &ReservationId,
    ) -> Result<Option<QueuedCall>>;
    async fn stats(&self, queue_id: &QueueId) -> Result<QueueStats>;
}

#[async_trait]
pub trait AgentOfferStore: Send + Sync {
    async fn insert_offer(&self, offer: AgentOffer) -> Result<()>;
    async fn update_offer(&self, offer: AgentOffer) -> Result<()>;
    async fn get_offer(&self, id: &AgentOfferId) -> Result<Option<AgentOffer>>;
    async fn list_offers_for_call(&self, call_id: &CallId) -> Result<Vec<AgentOffer>>;
}

pub type SharedCallStore = Arc<dyn CallStore>;
pub type SharedAgentStore = Arc<dyn AgentStore>;
pub type SharedQueueStore = Arc<dyn QueueStore>;
pub type SharedAgentOfferStore = Arc<dyn AgentOfferStore>;

/// In-memory call store backed by `DashMap` plus secondary indices.
///
/// - `calls`: primary CallId → Call store.
/// - `by_session`: SIP `SessionId` → `CallId` for O(1) `get_call_by_session`.
///   One Call has many legs and therefore many SessionIds; each leg's
///   session_id maps back to the same CallId.
/// - `by_dialog`: SIP Call-ID string → `CallId` — populated when
///   `Call::sip_call_id` is set. Pays its cost on writes; readers will be
///   added during the rvoip-sip migration when correlating wire-level
///   dialog identifiers becomes a hot path.
///
/// Indices are maintained on every `insert_call` / `update_call` by
/// diffing the previous stored state against the new one. There is no
/// per-key serialization beyond the entry-level lock — callers that
/// read-modify-write a Call must coordinate sequentially per CallId, the
/// same invariant the prior `RwLock<HashMap>` implementation imposed.
#[derive(Default)]
pub struct MemoryCallStore {
    calls: DashMap<CallId, Call>,
    by_session: DashMap<SessionId, CallId>,
    by_dialog: DashMap<String, CallId>,
}

impl MemoryCallStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reindex secondary maps after a primary `insert`. Called with the
    /// just-stored Call and the previously-stored Call (if any). Adds new
    /// session/dialog entries and removes ones that are no longer present.
    fn reindex(&self, new_call: &Call, old: Option<Call>) {
        for leg in &new_call.legs {
            self.by_session
                .insert(leg.session_id.clone(), new_call.id.clone());
        }
        if let Some(sip_id) = &new_call.sip_call_id {
            self.by_dialog.insert(sip_id.clone(), new_call.id.clone());
        }

        if let Some(old) = old {
            let new_sessions: HashSet<&SessionId> =
                new_call.legs.iter().map(|leg| &leg.session_id).collect();
            for old_leg in &old.legs {
                if !new_sessions.contains(&old_leg.session_id) {
                    // Only remove if the index still points to this CallId.
                    // Guards against the (unlikely) case where another call
                    // re-claimed the SessionId before this update landed.
                    self.by_session
                        .remove_if(&old_leg.session_id, |_, owner| owner == &new_call.id);
                }
            }
            if old.sip_call_id != new_call.sip_call_id {
                if let Some(old_sip_id) = &old.sip_call_id {
                    self.by_dialog
                        .remove_if(old_sip_id, |_, owner| owner == &new_call.id);
                }
            }
        }
    }
}

#[async_trait]
impl CallStore for MemoryCallStore {
    async fn insert_call(&self, call: Call) -> Result<()> {
        let old = self.calls.insert(call.id.clone(), call.clone());
        self.reindex(&call, old);
        Ok(())
    }

    async fn update_call(&self, call: Call) -> Result<()> {
        let old = self.calls.insert(call.id.clone(), call.clone());
        self.reindex(&call, old);
        Ok(())
    }

    async fn get_call(&self, id: &CallId) -> Result<Option<Call>> {
        Ok(self.calls.get(id).map(|entry| entry.clone()))
    }

    async fn get_call_by_session(&self, session_id: &SessionId) -> Result<Option<Call>> {
        let Some(call_id) = self.by_session.get(session_id).map(|entry| entry.clone()) else {
            return Ok(None);
        };
        Ok(self.calls.get(&call_id).map(|entry| entry.clone()))
    }

    async fn list_active_calls(&self) -> Result<Vec<Call>> {
        Ok(self
            .calls
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.value().status,
                    CallStatus::Ended | CallStatus::Failed | CallStatus::Abandoned
                )
            })
            .map(|entry| entry.value().clone())
            .collect())
    }
}

#[derive(Debug, Clone)]
struct ReservationRecord {
    agent_id: AgentId,
    call_id: CallId,
    active: bool,
}

#[derive(Default)]
pub struct MemoryAgentStore {
    agents: RwLock<HashMap<AgentId, Agent>>,
    reservations: RwLock<HashMap<ReservationId, ReservationRecord>>,
}

impl MemoryAgentStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AgentStore for MemoryAgentStore {
    async fn upsert_agent(&self, agent: Agent) -> Result<()> {
        self.agents.write().await.insert(agent.id.clone(), agent);
        Ok(())
    }

    async fn get_agent(&self, id: &AgentId) -> Result<Option<Agent>> {
        Ok(self.agents.read().await.get(id).cloned())
    }

    async fn list_agents(&self) -> Result<Vec<Agent>> {
        Ok(self.agents.read().await.values().cloned().collect())
    }

    async fn list_eligible_agents(&self, request: AgentEligibilityRequest) -> Result<Vec<Agent>> {
        Ok(self
            .agents
            .read()
            .await
            .values()
            .filter(|agent| agent.is_routable())
            .filter(|agent| agent.has_required_skills(&request.required_skills))
            .filter(|agent| !request.excluded_agent_ids.contains(&agent.id))
            .filter(|agent| {
                request
                    .preferred_kind
                    .map_or(true, |kind| agent.kind == kind)
            })
            .cloned()
            .collect())
    }

    async fn reserve_capacity(
        &self,
        id: &AgentId,
        call_id: &CallId,
    ) -> Result<Option<ReservationId>> {
        let mut agents = self.agents.write().await;
        let Some(agent) = agents.get_mut(id) else {
            return Err(OrchestrationError::AgentNotFound(id.clone()));
        };

        if !agent.is_routable() {
            return Ok(None);
        }

        let reservation_id = ReservationId::new();
        agent.capacity.reserved_calls += 1;
        agent.state = AgentState::Reserved;
        agent.last_state_change_at = Utc::now();
        drop(agents);

        self.reservations.write().await.insert(
            reservation_id.clone(),
            ReservationRecord {
                agent_id: id.clone(),
                call_id: call_id.clone(),
                active: false,
            },
        );

        Ok(Some(reservation_id))
    }

    async fn activate_capacity(&self, reservation_id: &ReservationId) -> Result<()> {
        let mut reservations = self.reservations.write().await;
        let Some(reservation) = reservations.get_mut(reservation_id) else {
            return Err(OrchestrationError::Store(format!(
                "reservation not found: {reservation_id}"
            )));
        };
        if reservation.active {
            return Ok(());
        }

        let mut agents = self.agents.write().await;
        let Some(agent) = agents.get_mut(&reservation.agent_id) else {
            return Err(OrchestrationError::AgentNotFound(
                reservation.agent_id.clone(),
            ));
        };
        reservation.active = true;
        agent.capacity.reserved_calls = agent.capacity.reserved_calls.saturating_sub(1);
        agent.capacity.active_calls += 1;
        agent.state = AgentState::OnCall;
        agent.last_state_change_at = Utc::now();
        Ok(())
    }

    async fn release_capacity(&self, reservation_id: &ReservationId) -> Result<()> {
        let Some(reservation) = self.reservations.write().await.remove(reservation_id) else {
            return Ok(());
        };

        let mut agents = self.agents.write().await;
        let Some(agent) = agents.get_mut(&reservation.agent_id) else {
            return Ok(());
        };

        if reservation.active {
            agent.capacity.active_calls = agent.capacity.active_calls.saturating_sub(1);
        } else {
            agent.capacity.reserved_calls = agent.capacity.reserved_calls.saturating_sub(1);
        }

        if agent.capacity.active_calls == 0
            && agent.capacity.reserved_calls == 0
            && matches!(
                agent.state,
                AgentState::Reserved
                    | AgentState::Offering
                    | AgentState::Ringing
                    | AgentState::OnCall
            )
        {
            agent.state = AgentState::Available;
            agent.last_state_change_at = Utc::now();
        }
        Ok(())
    }

    async fn update_state(&self, id: &AgentId, state: AgentState) -> Result<()> {
        let mut agents = self.agents.write().await;
        let Some(agent) = agents.get_mut(id) else {
            return Err(OrchestrationError::AgentNotFound(id.clone()));
        };
        agent.state = state;
        agent.last_state_change_at = Utc::now();
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryQueueStore {
    queues: RwLock<HashMap<QueueId, Queue>>,
    calls: RwLock<HashMap<QueueId, VecDeque<QueuedCall>>>,
}

impl MemoryQueueStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl QueueStore for MemoryQueueStore {
    async fn upsert_queue(&self, queue: Queue) -> Result<()> {
        self.calls
            .write()
            .await
            .entry(queue.id.clone())
            .or_insert_with(VecDeque::new);
        self.queues.write().await.insert(queue.id.clone(), queue);
        Ok(())
    }

    async fn get_queue(&self, id: &QueueId) -> Result<Option<Queue>> {
        Ok(self.queues.read().await.get(id).cloned())
    }

    async fn list_queues(&self) -> Result<Vec<Queue>> {
        Ok(self.queues.read().await.values().cloned().collect())
    }

    async fn enqueue(&self, queued_call: QueuedCall) -> Result<()> {
        if self.get_queue(&queued_call.queue_id).await?.is_none() {
            self.upsert_queue(Queue::new(
                queued_call.queue_id.clone(),
                queued_call.queue_id.to_string(),
            ))
            .await?;
        }

        let queue = self.get_queue(&queued_call.queue_id).await?.unwrap();
        let mut calls = self.calls.write().await;
        let entries = calls
            .entry(queued_call.queue_id.clone())
            .or_insert_with(VecDeque::new);

        if let Some(max_size) = queue.max_size {
            if entries.len() >= max_size {
                return Err(OrchestrationError::Store(format!(
                    "queue {} is full",
                    queued_call.queue_id
                )));
            }
        }

        let insert_at = entries
            .iter()
            .position(|existing| existing.priority > queued_call.priority)
            .unwrap_or(entries.len());
        entries.insert(insert_at, queued_call);
        Ok(())
    }

    async fn remove_call(&self, call_id: &CallId) -> Result<Option<QueuedCall>> {
        let mut calls = self.calls.write().await;
        for entries in calls.values_mut() {
            if let Some(index) = entries.iter().position(|queued| &queued.call_id == call_id) {
                return Ok(entries.remove(index));
            }
        }
        Ok(None)
    }

    async fn list_queued(&self, queue_id: &QueueId) -> Result<Vec<QueuedCall>> {
        Ok(self
            .calls
            .read()
            .await
            .get(queue_id)
            .map(|entries| entries.iter().cloned().collect())
            .unwrap_or_default())
    }

    async fn claim_for_agent(
        &self,
        queue_id: &QueueId,
        agent: &Agent,
        _reservation_id: &ReservationId,
    ) -> Result<Option<QueuedCall>> {
        let mut calls = self.calls.write().await;
        let Some(entries) = calls.get_mut(queue_id) else {
            return Ok(None);
        };

        let now = Utc::now();
        entries.retain(|queued| {
            queued
                .expires_at
                .map_or(true, |expires_at| expires_at > now)
        });

        let Some(index) = entries.iter().position(|queued| {
            agent.has_required_skills(&queued.required_skills)
                && !queued.previous_agent_ids.contains(&agent.id)
        }) else {
            return Ok(None);
        };

        Ok(entries.remove(index))
    }

    async fn stats(&self, queue_id: &QueueId) -> Result<QueueStats> {
        let calls = self.calls.read().await;
        let entries = calls.get(queue_id);
        let now = Utc::now();
        let waits: Vec<Duration> = entries
            .into_iter()
            .flat_map(|queue| queue.iter())
            .filter_map(|queued| now.signed_duration_since(queued.enqueued_at).to_std().ok())
            .collect();

        let queued_calls = waits.len();
        let oldest_wait = waits.iter().max().copied();
        let average_wait = if waits.is_empty() {
            None
        } else {
            let total_millis: u128 = waits.iter().map(|duration| duration.as_millis()).sum();
            Some(Duration::from_millis(
                (total_millis / waits.len() as u128) as u64,
            ))
        };

        Ok(QueueStats {
            queue_id: queue_id.clone(),
            queued_calls,
            oldest_wait,
            average_wait,
            available_agents: 0,
        })
    }
}

#[derive(Default)]
pub struct MemoryAgentOfferStore {
    offers: RwLock<HashMap<AgentOfferId, AgentOffer>>,
}

impl MemoryAgentOfferStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AgentOfferStore for MemoryAgentOfferStore {
    async fn insert_offer(&self, offer: AgentOffer) -> Result<()> {
        self.offers.write().await.insert(offer.id.clone(), offer);
        Ok(())
    }

    async fn update_offer(&self, offer: AgentOffer) -> Result<()> {
        self.offers.write().await.insert(offer.id.clone(), offer);
        Ok(())
    }

    async fn get_offer(&self, id: &AgentOfferId) -> Result<Option<AgentOffer>> {
        Ok(self.offers.read().await.get(id).cloned())
    }

    async fn list_offers_for_call(&self, call_id: &CallId) -> Result<Vec<AgentOffer>> {
        Ok(self
            .offers
            .read()
            .await
            .values()
            .filter(|offer| &offer.call_id == call_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available_agent() -> Agent {
        let mut agent = Agent::human("alice", "sip:alice@example.com");
        agent.state = AgentState::Available;
        agent.skills.push(Skill::from("support"));
        agent
    }

    fn queued_call(id: &str, priority: u8) -> QueuedCall {
        QueuedCall {
            call_id: CallId::from(id),
            queue_id: QueueId::from("support"),
            priority: CallPriority::from(priority),
            required_skills: vec![Skill::from("support")],
            enqueued_at: Utc::now(),
            expires_at: None,
            previous_agent_ids: Vec::new(),
            attempt_count: 0,
            escalation_reason: None,
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn reservation_release_restores_available_capacity() {
        let store = MemoryAgentStore::new();
        let agent = available_agent();
        let agent_id = agent.id.clone();
        let call_id = CallId::new();
        store.upsert_agent(agent).await.unwrap();

        let reservation_id = store
            .reserve_capacity(&agent_id, &call_id)
            .await
            .unwrap()
            .unwrap();
        let reserved = store.get_agent(&agent_id).await.unwrap().unwrap();
        assert_eq!(reserved.state, AgentState::Reserved);
        assert_eq!(reserved.capacity.reserved_calls, 1);

        store.release_capacity(&reservation_id).await.unwrap();
        let released = store.get_agent(&agent_id).await.unwrap().unwrap();
        assert_eq!(released.state, AgentState::Available);
        assert_eq!(released.capacity.reserved_calls, 0);
        assert_eq!(released.capacity.active_calls, 0);
    }

    #[tokio::test]
    async fn queue_orders_by_priority_and_claims_once() {
        let store = MemoryQueueStore::new();
        let queue_id = QueueId::from("support");
        store
            .upsert_queue(Queue::new(queue_id.clone(), "Support"))
            .await
            .unwrap();
        store.enqueue(queued_call("normal", 5)).await.unwrap();
        store.enqueue(queued_call("vip", 0)).await.unwrap();

        let queued = store.list_queued(&queue_id).await.unwrap();
        assert_eq!(queued[0].call_id, CallId::from("vip"));
        assert_eq!(queued[1].call_id, CallId::from("normal"));

        let agent = available_agent();
        let claimed = store
            .claim_for_agent(&queue_id, &agent, &ReservationId::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.call_id, CallId::from("vip"));
        assert_eq!(store.stats(&queue_id).await.unwrap().queued_calls, 1);
    }
}
