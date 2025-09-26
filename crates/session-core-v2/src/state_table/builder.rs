use super::types::*;
use crate::types::CallState;

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
    
    /// Create a builder with YAML-loaded table
    pub fn from_yaml() -> crate::errors::Result<Self> {
        let table = super::yaml_loader::YamlTableLoader::load_default()?;
        Ok(Self { table })
    }
    
    /// Load state table from a YAML file
    pub fn from_yaml_file(path: &str) -> crate::errors::Result<Self> {
        let table = super::yaml_loader::YamlTableLoader::load_from_file(path)?;
        Ok(Self { table })
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
    
    /// Add a raw transition with StateKey (used by YAML loader)
    pub fn add_raw_transition(&mut self, key: StateKey, transition: Transition) {
        self.table.insert(key, transition);
    }
    
    /// Add a wildcard transition that applies to any state
    pub fn add_wildcard_transition(&mut self, role: Role, event: EventType, transition: Transition) {
        self.table.insert_wildcard(role, event, transition);
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