use super::types::*;

/// Builder for constructing the state table
pub struct StateTableBuilder {
    table: MasterStateTable,
}

impl StateTableBuilder {
    pub fn new() -> Self {
        Self {
            table: MasterStateTable::new(),
        }
    }
    
    /// Add a transition to the table
    pub fn add_transition(
        &mut self,
        role: Role,
        state: CallState,
        event: EventType,
        transition: Transition,
    ) -> &mut Self {
        let key = StateKey { role, state, event };
        self.table.insert(key, transition);
        self
    }
    
    /// Add a simple state change transition
    pub fn add_state_change(
        &mut self,
        role: Role,
        from_state: CallState,
        event: EventType,
        to_state: CallState,
    ) -> &mut Self {
        self.add_transition(
            role,
            from_state,
            event,
            Transition {
                guards: vec![],
                actions: vec![],
                next_state: Some(to_state),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![EventTemplate::StateChanged],
            },
        )
    }
    
    /// Build the final state table
    pub fn build(self) -> MasterStateTable {
        self.table
    }
}

/// Helper methods for building common transition patterns
impl StateTableBuilder {
    /// Add a transition that sets a condition flag
    pub fn add_condition_setter(
        &mut self,
        role: Role,
        state: CallState,
        event: EventType,
        condition: Condition,
        value: bool,
    ) -> &mut Self {
        let condition_updates = match condition {
            Condition::DialogEstablished => ConditionUpdates::set_dialog_established(value),
            Condition::MediaSessionReady => ConditionUpdates::set_media_ready(value),
            Condition::SDPNegotiated => ConditionUpdates::set_sdp_negotiated(value),
        };
        
        self.add_transition(
            role,
            state,
            event,
            Transition {
                guards: vec![],
                actions: vec![Action::SetCondition(condition, value)],
                next_state: None,
                condition_updates,
                publish_events: vec![],
            },
        )
    }
    
    /// Add a transition that publishes MediaFlowEstablished
    pub fn add_media_flow_publisher(
        &mut self,
        role: Role,
        state: CallState,
        event: EventType,
        guards: Vec<Guard>,
    ) -> &mut Self {
        self.add_transition(
            role,
            state,
            event,
            Transition {
                guards,
                actions: vec![],
                next_state: None,
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![EventTemplate::MediaFlowEstablished],
            },
        )
    }
}