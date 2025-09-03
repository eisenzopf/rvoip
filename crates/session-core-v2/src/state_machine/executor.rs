use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error, debug};
use crate::state_table::SessionId;

use crate::{
    state_table::{MASTER_TABLE, StateKey, EventType, EventTemplate, Action, Transition, CallState},
    session_store::{SessionStore, SessionState},
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
};

use super::{actions, guards, effects};

/// Result of processing an event through the state machine
#[derive(Debug, Clone)]
pub struct ProcessEventResult {
    /// The old state before processing
    pub old_state: CallState,
    /// The new state after processing
    pub next_state: Option<CallState>,
    /// The transition that was executed (if any)
    pub transition: Option<Transition>,
    /// Actions that were executed
    pub actions_executed: Vec<Action>,
    /// Events that were published
    pub events_published: Vec<EventTemplate>,
}

/// The state machine executor that processes events through the state table
pub struct StateMachine {
    /// The master state table (static rules)
    table: Arc<crate::state_table::MasterStateTable>,
    
    /// Session state storage
    store: Arc<SessionStore>,
    
    /// Adapter to dialog-core
    dialog_adapter: Arc<DialogAdapter>,
    
    /// Adapter to media-core
    media_adapter: Arc<MediaAdapter>,
    
    /// Event publisher
    event_tx: tokio::sync::mpsc::Sender<SessionEvent>,
}

/// Events that flow through the system
#[derive(Debug, Clone)]
pub enum SessionEvent {
    StateChanged {
        session_id: SessionId,
        old_state: crate::state_table::CallState,
        new_state: crate::state_table::CallState,
    },
    MediaFlowEstablished {
        session_id: SessionId,
        local_addr: String,
        remote_addr: String,
        direction: crate::state_table::MediaFlowDirection,
    },
    CallEstablished {
        session_id: SessionId,
    },
    CallTerminated {
        session_id: SessionId,
    },
    Custom {
        session_id: SessionId,
        event: String,
    },
}

impl StateMachine {
    pub fn new(
        table: Arc<crate::state_table::MasterStateTable>,
        store: Arc<SessionStore>,
    ) -> Self {
        // Create dummy adapters for now (will be provided separately)
        let (event_tx, _) = tokio::sync::mpsc::channel(100);
        Self {
            table,
            store,
            dialog_adapter: Arc::new(DialogAdapter::new_mock()),
            media_adapter: Arc::new(MediaAdapter::new_mock()),
            event_tx,
        }
    }
    
    pub fn new_with_adapters(
        store: Arc<SessionStore>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        event_tx: tokio::sync::mpsc::Sender<SessionEvent>,
    ) -> Self {
        Self {
            table: MASTER_TABLE.clone(),
            store,
            dialog_adapter,
            media_adapter,
            event_tx,
        }
    }
    
    /// Check if a transition exists for the given state key
    pub fn has_transition(&self, key: &StateKey) -> bool {
        self.table.has_transition(key)
    }
    
    /// Process an event for a session
    pub async fn process_event(
        &self,
        session_id: &SessionId,
        event: EventType,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        debug!("Processing event {:?} for session {}", event, session_id);
        
        // 1. Get current session state
        let mut session = self.store.get_session(session_id).await?;
        let old_state = session.call_state;
        
        // 1a. Store event-specific data in session state
        match &event {
            EventType::MakeCall { target } => {
                session.remote_uri = Some(target.clone());
                // Set a default local URI if not already set
                if session.local_uri.is_none() {
                    session.local_uri = Some("sip:user@localhost".to_string());
                }
            }
            EventType::IncomingCall { from, sdp } => {
                session.remote_uri = Some(from.clone());
                if let Some(sdp_data) = sdp {
                    session.remote_sdp = Some(sdp_data.clone());
                }
            }
            _ => {}
        }
        
        // 2. Build state key for lookup
        let key = StateKey {
            role: session.role,
            state: session.call_state,
            event: event.clone(),
        };
        
        // 3. Look up transition in table
        let transition = match self.table.get(&key) {
            Some(t) => t,
            None => {
                warn!("No transition defined for {:?}", key);
                return Ok(ProcessEventResult {
                    old_state,
                    transition: None,
                    actions_executed: vec![],
                });
            }
        };
        
        // 4. Check guards
        for guard in &transition.guards {
            if !guards::check_guard(guard, &session).await {
                debug!("Guard {:?} not satisfied, skipping transition", guard);
                return Ok(ProcessEventResult {
                    old_state,
                    transition: None,
                    actions_executed: vec![],
                });
            }
        }
        
        info!("Executing transition for {:?} + {:?}", old_state, event);
        
        // 5. Execute actions
        let mut actions_executed = Vec::new();
        for action in &transition.actions {
            if let Err(e) = actions::execute_action(
                action,
                &mut session,
                &self.dialog_adapter,
                &self.media_adapter,
            ).await {
                error!("Failed to execute action {:?}: {}", action, e);
                return Err(e);
            }
            actions_executed.push(action.clone());
        }
        
        // 6. Update state if specified
        if let Some(next_state) = transition.next_state {
            session.transition_to(next_state);
            info!("State transition: {:?} -> {:?}", old_state, next_state);
        }
        
        // 7. Apply condition updates
        session.apply_condition_updates(&transition.condition_updates);
        
        // 8. Save updated session state
        self.store.update_session(session.clone()).await?;
        
        // 9. Publish events
        for event_template in &transition.publish_events {
            let event = self.instantiate_event(event_template, &session, old_state).await;
            if let Err(e) = self.event_tx.send(event).await {
                error!("Failed to publish event: {}", e);
            }
        }
        
        // 10. Check if conditions trigger internal events
        if session.all_conditions_met() && !session.call_established_triggered {
            debug!("All conditions met, triggering InternalCheckReady");
            Box::pin(self.process_event(session_id, EventType::InternalCheckReady)).await?;
        }
        
        Ok(ProcessEventResult {
            old_state,
            next_state: transition.next_state,
            transition: Some(transition.clone()),
            actions_executed,
            events_published: transition.publish_events.clone(),
        })
    }
    
    /// Convert event template to concrete event
    async fn instantiate_event(
        &self,
        template: &EventTemplate,
        session: &SessionState,
        old_state: crate::state_table::CallState,
    ) -> SessionEvent {
        match template {
            EventTemplate::StateChanged => SessionEvent::StateChanged {
                session_id: session.session_id.clone(),
                old_state,
                new_state: session.call_state,
            },
            EventTemplate::MediaFlowEstablished => {
                let negotiated = session.negotiated_config.as_ref();
                SessionEvent::MediaFlowEstablished {
                    session_id: session.session_id.clone(),
                    local_addr: negotiated.map(|n| n.local_addr.to_string()).unwrap_or_default(),
                    remote_addr: negotiated.map(|n| n.remote_addr.to_string()).unwrap_or_default(),
                    direction: crate::state_table::MediaFlowDirection::Both,
                }
            }
            EventTemplate::CallEstablished => SessionEvent::CallEstablished {
                session_id: session.session_id.clone(),
            },
            EventTemplate::CallTerminated => SessionEvent::CallTerminated {
                session_id: session.session_id.clone(),
            },
            EventTemplate::Custom(event) => SessionEvent::Custom {
                session_id: session.session_id.clone(),
                event: event.clone(),
            },
            _ => SessionEvent::Custom {
                session_id: session.session_id.clone(),
                event: format!("{:?}", template),
            },
        }
    }
}