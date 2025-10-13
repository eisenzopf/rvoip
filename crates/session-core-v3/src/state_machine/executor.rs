use std::sync::Arc;
use tracing::{info, error, debug};
use crate::state_table::SessionId;

use crate::{
    state_table::{MASTER_TABLE, StateKey, EventType, EventTemplate, Action, Transition},
    session_store::{SessionStore, SessionState},
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    types::CallState,
    // Event import removed - events handled by SessionCrossCrateEventHandler
};

use super::{actions, guards};

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
    pub(crate) store: Arc<SessionStore>,
    
    /// Adapter to dialog-core
    dialog_adapter: Arc<DialogAdapter>,
    
    /// Adapter to media-core
    media_adapter: Arc<MediaAdapter>,
    
    /// Event publisher (optional - for legacy compatibility)
    event_tx: Option<tokio::sync::mpsc::Sender<SessionEvent>>,
    
    // SimplePeer events now handled by SessionCrossCrateEventHandler
}

/// Events that flow through the system
#[derive(Debug, Clone)]
pub enum SessionEvent {
    StateChanged {
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
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
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
    ) -> Self {
        Self {
            table,
            store,
            dialog_adapter,
            media_adapter,
            event_tx: None, // No event channel by default
            // SimplePeer events handled by SessionCrossCrateEventHandler
        }
    }
    
    // new_with_simple_peer_events removed - using SessionCrossCrateEventHandler for event forwarding
    
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
            event_tx: Some(event_tx),
            // SimplePeer events handled by SessionCrossCrateEventHandler
        }
    }
    
    pub fn new_with_custom_table(
        table: Arc<crate::state_table::MasterStateTable>,
        store: Arc<SessionStore>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        event_tx: tokio::sync::mpsc::Sender<SessionEvent>,
    ) -> Self {
        Self {
            table,
            store,
            dialog_adapter,
            media_adapter,
            event_tx: Some(event_tx),
            // SimplePeer events handled by SessionCrossCrateEventHandler
        }
    }
    
    // Callback registry removed - using event-driven approach
    
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
        use std::time::Instant;
        use crate::session_store::{TransitionRecord, GuardResult, ActionRecord};
        
        debug!("Processing event {:?} for session {}", event, session_id);
        let transition_start = Instant::now();
        
        // 1. Get current session state
        let mut session = match self.store.get_session(session_id).await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to get session {}: {}", session_id, e);
                return Err(crate::errors::SessionError::SessionNotFound(session_id.to_string()).into());
            }
        };
        let old_state = session.call_state;
        
        // Initialize tracking for history
        let mut guards_evaluated = Vec::new();
        let mut actions_executed_history = Vec::new();
        let mut errors = Vec::new();
        
        // 1a. Store event-specific data in session state
        match &event {
            EventType::MakeCall { target } => {
                session.remote_uri = Some(target.clone());
                // local_uri should be set when session is created
            }
            EventType::IncomingCall { from, sdp } => {
                session.remote_uri = Some(from.clone());
                if let Some(sdp_data) = sdp {
                    session.remote_sdp = Some(sdp_data.clone());
                }
            }
            // BlindTransfer event removed
            EventType::TransferRequested { refer_to, transfer_type, transaction_id } => {
                session.transfer_target = Some(refer_to.clone());
                session.transfer_notify_dialog = session.dialog_id.clone();
                session.refer_transaction_id = Some(transaction_id.clone());
                debug!("Set transfer target from REFER: {}, type: {:?}, transaction: {}", refer_to, transfer_type, transaction_id);
            }
            // StartAttendedTransfer event removed
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
                debug!("No transition defined for {:?}", key);
                
                // Record failed transition attempt in history
                if session.history.is_some() {
                    let now = Instant::now();
                    let record = TransitionRecord {
                        sequence: 0, // Will be set by history
                        timestamp: now,
                        timestamp_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        from_state: old_state,
                        event: event.clone(),
                        to_state: Some(old_state),
                        guards_evaluated: vec![],
                        actions_executed: vec![],
                        duration_ms: transition_start.elapsed().as_millis() as u64,
                        errors: vec![format!("No transition defined for {:?}", key)],
                        events_published: vec![],
                    };
                    session.record_transition(record);
                    self.store.update_session(session).await?;
                }
                
                return Ok(ProcessEventResult {
                    old_state,
                    next_state: None,
                    transition: None,
                    actions_executed: vec![],
                    events_published: vec![],
                });
            }
        };
        
        // 4. Check guards
        for guard in &transition.guards {
            let guard_start = Instant::now();
            let satisfied = guards::check_guard(guard, &session).await;
            let guard_duration = guard_start.elapsed().as_millis() as u64;
            
            guards_evaluated.push(GuardResult {
                guard: guard.clone(),
                passed: satisfied,
                evaluation_time_us: guard_duration * 1000,
            });
            
            if !satisfied {
                debug!("Guard {:?} not satisfied, skipping transition", guard);
                
                // Record guard failure in history
                if session.history.is_some() {
                    let now = Instant::now();
                    let record = TransitionRecord {
                        sequence: 0,
                        timestamp: now,
                        timestamp_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        from_state: old_state,
                        event: event.clone(),
                        to_state: Some(old_state),
                        guards_evaluated,
                        actions_executed: vec![],
                        duration_ms: transition_start.elapsed().as_millis() as u64,
                        errors: vec![format!("Guard {:?} not satisfied", guard)],
                        events_published: vec![],
                    };
                    session.record_transition(record);
                    self.store.update_session(session).await?;
                }
                
                return Ok(ProcessEventResult {
                    old_state,
                    next_state: None,
                    transition: None,
                    actions_executed: vec![],
                    events_published: vec![],
                });
            }
        }
        
        info!("Executing transition for {:?} + {:?}", old_state, event);
        
        // 5. Execute actions
        let mut actions_executed = Vec::new();
        for action in &transition.actions {
            let action_start = Instant::now();
            let result = actions::execute_action(
                action,
                &mut session,
                &self.dialog_adapter,
                &self.media_adapter,
                &self.store,
                &None, // No SimplePeer event channel - handled by SessionCrossCrateEventHandler
            ).await;
            let action_duration = action_start.elapsed().as_millis() as u64;
            
            let (success, error_opt, exec_error) = match result {
                Ok(_) => {
                    actions_executed.push(action.clone());
                    (true, None, None)
                }
                Err(e) => {
                    let error_msg = format!("Failed to execute action {:?}: {}", action, e);
                    error!("{}", error_msg);
                    errors.push(error_msg.clone());
                    (false, Some(error_msg), Some(e))
                }
            };
            
            actions_executed_history.push(ActionRecord {
                action: action.clone(),
                success,
                execution_time_us: action_duration * 1000,
                error: error_opt,
            });
            
            if !success {
                // Record failed action in history
                if session.history.is_some() {
                    let now = Instant::now();
                    let record = TransitionRecord {
                        sequence: 0,
                        timestamp: now,
                        timestamp_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        from_state: old_state,
                        event: event.clone(),
                        to_state: Some(old_state),
                        guards_evaluated,
                        actions_executed: actions_executed_history,
                        duration_ms: transition_start.elapsed().as_millis() as u64,
                        errors,
                        events_published: vec![],
                    };
                    session.record_transition(record);
                    self.store.update_session(session).await?;
                }
                
                return Err(exec_error.unwrap());
            }
        }
        
        // 6. Update state if specified
        let next_state = transition.next_state;
        if let Some(new_state) = next_state {
            info!("State transition: {:?} -> {:?}", old_state, new_state);
        }
        
        // Record successful transition in history
        if session.history.is_some() {
            let now = Instant::now();
            let record = TransitionRecord {
                sequence: 0,
                timestamp: now,
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                from_state: old_state,
                event: event.clone(),
                to_state: next_state,
                guards_evaluated,
                actions_executed: actions_executed_history,
                duration_ms: transition_start.elapsed().as_millis() as u64,
                errors,
                events_published: transition.publish_events.clone(),
            };
            session.record_transition(record);
        }
        
        // Apply state change after recording history
        if let Some(new_state) = transition.next_state {
            session.call_state = new_state;
            session.entered_state_at = Instant::now();
        }
        
        // 7. Apply condition updates
        session.apply_condition_updates(&transition.condition_updates);
        
        // 8. Save updated session state
        self.store.update_session(session.clone()).await?;
        
        // 9. Publish events (if channel is available)
        if let Some(ref event_tx) = self.event_tx {
            for event_template in &transition.publish_events {
                let event = self.instantiate_event(event_template, &session, old_state).await;
                if let Err(e) = event_tx.send(event).await {
                    error!("Failed to publish event: {}", e);
                }
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
        old_state: CallState,
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