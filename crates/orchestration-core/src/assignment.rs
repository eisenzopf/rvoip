use crate::config::AssignmentConfig;
use crate::error::{OrchestrationError, Result};
use crate::events::{OrchestrationEvent, OrchestrationEventBus};
use crate::ids::*;
use crate::store::{SharedAgentOfferStore, SharedAgentStore, SharedCallStore, SharedQueueStore};
use crate::traits::{FirstAvailableSelector, QueueSelector};
use crate::types::*;
use chrono::{Duration as ChronoDuration, Utc};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Assignment {
    pub call_id: CallId,
    pub queue_id: QueueId,
    pub agent_id: AgentId,
    pub offer_id: AgentOfferId,
    pub reservation_id: ReservationId,
}

pub struct AssignmentManager {
    calls: SharedCallStore,
    agents: SharedAgentStore,
    queues: SharedQueueStore,
    offers: SharedAgentOfferStore,
    selector: Arc<dyn QueueSelector>,
    events: OrchestrationEventBus,
    config: AssignmentConfig,
}

impl AssignmentManager {
    pub fn new(
        calls: SharedCallStore,
        agents: SharedAgentStore,
        queues: SharedQueueStore,
        offers: SharedAgentOfferStore,
        events: OrchestrationEventBus,
        config: AssignmentConfig,
    ) -> Self {
        Self {
            calls,
            agents,
            queues,
            offers,
            selector: Arc::new(FirstAvailableSelector),
            events,
            config,
        }
    }

    pub fn with_selector(mut self, selector: Arc<dyn QueueSelector>) -> Self {
        self.selector = selector;
        self
    }

    pub async fn assign_next(&self, queue_id: &QueueId) -> Result<Option<Assignment>> {
        let queue = self
            .queues
            .get_queue(queue_id)
            .await?
            .ok_or_else(|| OrchestrationError::QueueNotFound(queue_id.clone()))?;
        let queued_calls = self.queues.list_queued(queue_id).await?;

        for queued_call in queued_calls {
            let required_skills =
                merged_skills(&queue.required_skills, &queued_call.required_skills);
            for preferred_kind in preferred_kind_order(queue.policy) {
                let candidates = self
                    .agents
                    .list_eligible_agents(AgentEligibilityRequest {
                        queue_id: Some(queue_id.clone()),
                        required_skills: required_skills.clone(),
                        excluded_agent_ids: queued_call.previous_agent_ids.clone(),
                        preferred_kind,
                    })
                    .await?;
                let Some(agent_id) = self.selector.select_agent(&queued_call, candidates).await?
                else {
                    continue;
                };

                let Some(reservation_id) = self
                    .agents
                    .reserve_capacity(&agent_id, &queued_call.call_id)
                    .await?
                else {
                    continue;
                };

                match self
                    .try_claim_for_reserved_agent(&queue, &queued_call, agent_id, reservation_id)
                    .await?
                {
                    Some(assignment) => return Ok(Some(assignment)),
                    None => continue,
                }
            }
        }

        Ok(None)
    }

    async fn try_claim_for_reserved_agent(
        &self,
        queue: &Queue,
        queued_call: &QueuedCall,
        agent_id: AgentId,
        reservation_id: ReservationId,
    ) -> Result<Option<Assignment>> {
        let Some(mut agent) = self.agents.get_agent(&agent_id).await? else {
            self.agents.release_capacity(&reservation_id).await?;
            return Err(OrchestrationError::AgentNotFound(agent_id));
        };
        agent.state = AgentState::Reserved;

        let Some(claimed) = self
            .queues
            .claim_for_agent(&queue.id, &agent, &reservation_id)
            .await?
        else {
            self.agents.release_capacity(&reservation_id).await?;
            return Ok(None);
        };

        if claimed.call_id != queued_call.call_id {
            // The queue store may legally return another eligible call for this
            // reserved agent, but assignment is clearer when the attempt history
            // is carried forward from the claimed call.
        }

        let offer_id = AgentOfferId::new();
        let now = Utc::now();
        let expires_at = now
            + ChronoDuration::from_std(self.config.offer_timeout)
                .unwrap_or_else(|_| ChronoDuration::seconds(30));
        let offer = AgentOffer {
            id: offer_id.clone(),
            call_id: claimed.call_id.clone(),
            queue_id: Some(queue.id.clone()),
            agent_id: agent_id.clone(),
            reservation_id: Some(reservation_id.clone()),
            status: AgentOfferStatus::Reserved,
            created_at: now,
            expires_at,
            agent_leg_id: None,
            failure_reason: None,
        };
        self.offers.insert_offer(offer).await?;
        self.agents
            .update_state(&agent_id, AgentState::Offering)
            .await?;
        self.update_call_for_offer(&claimed.call_id, &queue.id, &agent_id)
            .await?;

        self.events.emit(OrchestrationEvent::CallDequeued {
            call_id: claimed.call_id.clone(),
            queue_id: queue.id.clone(),
        });
        self.events.emit(OrchestrationEvent::AgentReserved {
            call_id: claimed.call_id.clone(),
            agent_id: agent_id.clone(),
            offer_id: offer_id.clone(),
        });
        self.events.emit(OrchestrationEvent::AgentStateChanged {
            agent_id: agent_id.clone(),
            from: AgentState::Reserved,
            to: AgentState::Offering,
        });

        Ok(Some(Assignment {
            call_id: claimed.call_id,
            queue_id: queue.id.clone(),
            agent_id,
            offer_id,
            reservation_id,
        }))
    }

    async fn update_call_for_offer(
        &self,
        call_id: &CallId,
        queue_id: &QueueId,
        agent_id: &AgentId,
    ) -> Result<()> {
        let mut call = self
            .calls
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::OfferingAgent;
        call.queue_id = Some(queue_id.clone());
        call.assigned_agent_id = Some(agent_id.clone());
        self.calls.update_call(call).await?;
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: call_id.clone(),
            from,
            to: CallStatus::OfferingAgent,
        });
        Ok(())
    }

    pub async fn accept_offer(&self, offer_id: &AgentOfferId) -> Result<()> {
        let mut offer = self
            .offers
            .get_offer(offer_id)
            .await?
            .ok_or_else(|| OrchestrationError::Store(format!("offer not found: {offer_id}")))?;
        let Some(reservation_id) = offer.reservation_id.clone() else {
            return Err(OrchestrationError::InvalidState(format!(
                "offer {offer_id} has no reservation"
            )));
        };
        self.agents.activate_capacity(&reservation_id).await?;
        offer.status = AgentOfferStatus::Accepted;
        self.offers.update_offer(offer.clone()).await?;

        let mut call = self
            .calls
            .get_call(&offer.call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(offer.call_id.clone()))?;
        let from = call.status;
        call.status = CallStatus::Connected;
        call.answered_at.get_or_insert_with(Utc::now);
        self.calls.update_call(call).await?;

        self.events.emit(OrchestrationEvent::AgentOfferAccepted {
            call_id: offer.call_id.clone(),
            agent_id: offer.agent_id.clone(),
            offer_id: offer.id.clone(),
        });
        self.events.emit(OrchestrationEvent::CallStatusChanged {
            call_id: offer.call_id,
            from,
            to: CallStatus::Connected,
        });
        Ok(())
    }

    pub async fn fail_offer(
        &self,
        offer_id: &AgentOfferId,
        status: AgentOfferStatus,
        reason: impl Into<String>,
    ) -> Result<()> {
        if !matches!(
            status,
            AgentOfferStatus::Rejected
                | AgentOfferStatus::TimedOut
                | AgentOfferStatus::Cancelled
                | AgentOfferStatus::Failed
        ) {
            return Err(OrchestrationError::InvalidState(format!(
                "status {status:?} is not a failure terminal status"
            )));
        }

        let reason = reason.into();
        let mut offer = self
            .offers
            .get_offer(offer_id)
            .await?
            .ok_or_else(|| OrchestrationError::Store(format!("offer not found: {offer_id}")))?;
        if let Some(reservation_id) = &offer.reservation_id {
            self.agents.release_capacity(reservation_id).await?;
        }
        offer.status = status;
        offer.failure_reason = Some(reason.clone());
        self.offers.update_offer(offer.clone()).await?;

        match status {
            AgentOfferStatus::Rejected => {
                self.events.emit(OrchestrationEvent::AgentOfferRejected {
                    call_id: offer.call_id,
                    agent_id: offer.agent_id,
                    offer_id: offer.id,
                    reason,
                })
            }
            AgentOfferStatus::TimedOut => {
                self.events.emit(OrchestrationEvent::AgentOfferTimedOut {
                    call_id: offer.call_id,
                    agent_id: offer.agent_id,
                    offer_id: offer.id,
                })
            }
            AgentOfferStatus::Cancelled | AgentOfferStatus::Failed => {
                self.events.emit(OrchestrationEvent::AgentOfferFailed {
                    call_id: offer.call_id,
                    agent_id: offer.agent_id,
                    offer_id: offer.id,
                    reason,
                })
            }
            _ => unreachable!(),
        };
        Ok(())
    }
}

fn merged_skills(queue_skills: &[Skill], call_skills: &[Skill]) -> Vec<Skill> {
    let mut skills = queue_skills.to_vec();
    for skill in call_skills {
        if !skills.contains(skill) {
            skills.push(skill.clone());
        }
    }
    skills
}

fn preferred_kind_order(policy: QueuePolicy) -> Vec<Option<AgentKind>> {
    match policy {
        QueuePolicy::AiFirstThenHuman => vec![Some(AgentKind::VoiceAi), Some(AgentKind::Human)],
        QueuePolicy::HumanFirstThenAi => vec![Some(AgentKind::Human), Some(AgentKind::VoiceAi)],
        _ => vec![None],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        MemoryAgentOfferStore, MemoryAgentStore, MemoryCallStore, MemoryQueueStore,
    };
    use async_trait::async_trait;

    fn test_call() -> Call {
        let mut call = Call::inbound(CallerIdentity::new("sip:caller@example.com"), "sip:support");
        call.status = CallStatus::Queued;
        call
    }

    fn test_agent() -> Agent {
        let mut agent = Agent::human("alice", "sip:alice@example.com");
        agent.state = AgentState::Available;
        agent.skills.push(Skill::from("support"));
        agent
    }

    async fn fixture() -> (
        SharedCallStore,
        SharedAgentStore,
        SharedQueueStore,
        SharedAgentOfferStore,
        OrchestrationEventBus,
        Call,
        Agent,
        QueueId,
    ) {
        let calls: SharedCallStore = Arc::new(MemoryCallStore::new());
        let agents: SharedAgentStore = Arc::new(MemoryAgentStore::new());
        let queues: SharedQueueStore = Arc::new(MemoryQueueStore::new());
        let offers: SharedAgentOfferStore = Arc::new(MemoryAgentOfferStore::new());
        let events = OrchestrationEventBus::new(16);

        let queue_id = QueueId::from("support");
        let mut queue = Queue::new(queue_id.clone(), "Support");
        queue.required_skills.push(Skill::from("support"));
        queues.upsert_queue(queue).await.unwrap();

        let call = test_call();
        calls.insert_call(call.clone()).await.unwrap();
        queues
            .enqueue(QueuedCall {
                call_id: call.id.clone(),
                queue_id: queue_id.clone(),
                priority: CallPriority::NORMAL,
                required_skills: vec![Skill::from("support")],
                enqueued_at: Utc::now(),
                expires_at: None,
                previous_agent_ids: Vec::new(),
                attempt_count: 0,
                escalation_reason: None,
                metadata: Default::default(),
            })
            .await
            .unwrap();

        let agent = test_agent();
        agents.upsert_agent(agent.clone()).await.unwrap();

        (calls, agents, queues, offers, events, call, agent, queue_id)
    }

    #[tokio::test]
    async fn assign_next_claims_call_reserves_agent_and_creates_offer() {
        let (calls, agents, queues, offers, events, call, agent, queue_id) = fixture().await;
        let manager = AssignmentManager::new(
            calls.clone(),
            agents.clone(),
            queues.clone(),
            offers.clone(),
            events,
            AssignmentConfig::default(),
        );

        let assignment = manager.assign_next(&queue_id).await.unwrap().unwrap();

        assert_eq!(assignment.call_id, call.id);
        assert_eq!(assignment.agent_id, agent.id);
        assert_eq!(queues.stats(&queue_id).await.unwrap().queued_calls, 0);

        let updated_call = calls.get_call(&assignment.call_id).await.unwrap().unwrap();
        assert_eq!(updated_call.status, CallStatus::OfferingAgent);
        assert_eq!(
            updated_call.assigned_agent_id,
            Some(assignment.agent_id.clone())
        );

        let updated_agent = agents
            .get_agent(&assignment.agent_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated_agent.state, AgentState::Offering);
        assert_eq!(updated_agent.capacity.reserved_calls, 1);

        let offer = offers
            .get_offer(&assignment.offer_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(offer.status, AgentOfferStatus::Reserved);
        assert_eq!(offer.reservation_id, Some(assignment.reservation_id));
    }

    #[tokio::test]
    async fn accept_offer_activates_capacity_and_connects_call() {
        let (calls, agents, queues, offers, events, _call, _agent, queue_id) = fixture().await;
        let manager = AssignmentManager::new(
            calls.clone(),
            agents.clone(),
            queues,
            offers.clone(),
            events,
            AssignmentConfig::default(),
        );
        let assignment = manager.assign_next(&queue_id).await.unwrap().unwrap();

        manager.accept_offer(&assignment.offer_id).await.unwrap();

        let call = calls.get_call(&assignment.call_id).await.unwrap().unwrap();
        assert_eq!(call.status, CallStatus::Connected);
        assert!(call.answered_at.is_some());

        let agent = agents
            .get_agent(&assignment.agent_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(agent.state, AgentState::OnCall);
        assert_eq!(agent.capacity.reserved_calls, 0);
        assert_eq!(agent.capacity.active_calls, 1);

        let offer = offers
            .get_offer(&assignment.offer_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(offer.status, AgentOfferStatus::Accepted);
    }

    #[tokio::test]
    async fn failed_offer_releases_reserved_capacity() {
        let (calls, agents, queues, offers, events, _call, _agent, queue_id) = fixture().await;
        let manager = AssignmentManager::new(
            calls,
            agents.clone(),
            queues,
            offers.clone(),
            events,
            AssignmentConfig::default(),
        );
        let assignment = manager.assign_next(&queue_id).await.unwrap().unwrap();

        manager
            .fail_offer(
                &assignment.offer_id,
                AgentOfferStatus::TimedOut,
                "no answer",
            )
            .await
            .unwrap();

        let agent = agents
            .get_agent(&assignment.agent_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(agent.state, AgentState::Available);
        assert_eq!(agent.capacity.reserved_calls, 0);
        assert_eq!(agent.capacity.active_calls, 0);

        let offer = offers
            .get_offer(&assignment.offer_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(offer.status, AgentOfferStatus::TimedOut);
        assert_eq!(offer.failure_reason.as_deref(), Some("no answer"));
    }

    struct BadSelector {
        agent_id: AgentId,
    }

    #[async_trait]
    impl QueueSelector for BadSelector {
        async fn select_agent(
            &self,
            _queued_call: &QueuedCall,
            _candidates: Vec<Agent>,
        ) -> Result<Option<AgentId>> {
            Ok(Some(self.agent_id.clone()))
        }
    }

    #[tokio::test]
    async fn assignment_releases_reservation_when_claim_fails() {
        let (calls, agents, queues, offers, events, call, agent, queue_id) = fixture().await;
        queues.remove_call(&call.id).await.unwrap();
        queues
            .enqueue(QueuedCall {
                call_id: call.id.clone(),
                queue_id: queue_id.clone(),
                priority: CallPriority::NORMAL,
                required_skills: vec![Skill::from("support")],
                enqueued_at: Utc::now(),
                expires_at: None,
                previous_agent_ids: vec![agent.id.clone()],
                attempt_count: 1,
                escalation_reason: None,
                metadata: Default::default(),
            })
            .await
            .unwrap();

        let manager = AssignmentManager::new(
            calls,
            agents.clone(),
            queues.clone(),
            offers.clone(),
            events,
            AssignmentConfig::default(),
        )
        .with_selector(Arc::new(BadSelector {
            agent_id: agent.id.clone(),
        }));

        assert!(manager.assign_next(&queue_id).await.unwrap().is_none());

        let agent = agents.get_agent(&agent.id).await.unwrap().unwrap();
        assert_eq!(agent.state, AgentState::Available);
        assert_eq!(agent.capacity.reserved_calls, 0);
        assert!(offers
            .list_offers_for_call(&call.id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(queues.stats(&queue_id).await.unwrap().queued_calls, 1);
    }

    #[tokio::test]
    async fn ai_first_policy_falls_back_to_human_when_ai_unavailable() {
        let (calls, agents, queues, offers, events, _call, human, queue_id) = fixture().await;
        let mut queue = queues.get_queue(&queue_id).await.unwrap().unwrap();
        queue.policy = QueuePolicy::AiFirstThenHuman;
        queues.upsert_queue(queue).await.unwrap();

        let mut ai = Agent::voice_ai("support-ai", "support-ai-runtime");
        ai.state = AgentState::Offline;
        ai.skills.push(Skill::from("support"));
        agents.upsert_agent(ai).await.unwrap();

        let manager = AssignmentManager::new(
            calls,
            agents,
            queues,
            offers,
            events,
            AssignmentConfig::default(),
        );

        let assignment = manager.assign_next(&queue_id).await.unwrap().unwrap();
        assert_eq!(assignment.agent_id, human.id);
    }
}
