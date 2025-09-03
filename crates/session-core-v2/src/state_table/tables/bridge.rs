//! Bridge and transfer state transitions

use crate::state_table::{StateTableBuilder, types::*};
use std::collections::HashMap;

/// Add bridge and transfer transitions to the builder
pub fn add_bridge_transitions(builder: &mut StateTableBuilder) {
    // Bridge operations for both UAC and UAS
    for role in [Role::UAC, Role::UAS] {
        // Active -> Bridged: Create bridge with another session
        builder.add_transition(
            role,
            CallState::Active,
            EventType::BridgeSessions { 
                other_session: SessionId::new() // Placeholder
            },
            Transition {
                guards: vec![Guard::DialogEstablished, Guard::MediaReady],
                actions: vec![
                    Action::CreateBridge(SessionId::new()), // Placeholder
                ],
                next_state: Some(CallState::Bridged),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("BridgeCreated".to_string()),
                ],
            },
        );
        
        // Bridged -> Terminating: Destroy bridge on hangup
        builder.add_transition(
            role,
            CallState::Bridged,
            EventType::HangupCall,
            Transition {
                guards: vec![],
                actions: vec![
                    Action::DestroyBridge,
                    Action::SendBYE,
                ],
                next_state: Some(CallState::Terminating),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("BridgeDestroyed".to_string()),
                ],
            },
        );
        
        // Active -> Transferring: Blind transfer
        builder.add_transition(
            role,
            CallState::Active,
            EventType::BlindTransfer { 
                target: String::new() // Placeholder
            },
            Transition {
                guards: vec![Guard::DialogEstablished],
                actions: vec![
                    Action::InitiateBlindTransfer(String::new()), // Placeholder
                    Action::Custom("SendREFER".to_string()),
                ],
                next_state: Some(CallState::Transferring),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("TransferInitiated".to_string()),
                ],
            },
        );
        
        // Active -> Transferring: Attended transfer
        builder.add_transition(
            role,
            CallState::Active,
            EventType::AttendedTransfer { 
                target: String::new() // Placeholder
            },
            Transition {
                guards: vec![Guard::DialogEstablished, Guard::MediaReady],
                actions: vec![
                    Action::InitiateAttendedTransfer(String::new()), // Placeholder
                    Action::Custom("CreateConsultCall".to_string()),
                ],
                next_state: Some(CallState::Transferring),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("AttendedTransferStarted".to_string()),
                ],
            },
        );
        
        // Transferring -> Terminated: Transfer completed
        builder.add_transition(
            role,
            CallState::Transferring,
            EventType::DialogBYE,
            Transition {
                guards: vec![],
                actions: vec![
                    Action::StartMediaCleanup,
                    Action::StartDialogCleanup,
                ],
                next_state: Some(CallState::Terminated),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("TransferCompleted".to_string()),
                    EventTemplate::CallTerminated,
                ],
            },
        );
    }
}

/// Load bridge and transfer transitions (legacy function for compatibility)
pub fn load_bridge_transitions() -> HashMap<StateKey, Transition> {
    let mut table = HashMap::new();
    
    // Bridge operations for both UAC and UAS
    for role in [Role::UAC, Role::UAS] {
        // Active -> Bridged: Create bridge with another session
        table.insert(
            StateKey {
                role,
                state: CallState::Active,
                event: EventType::BridgeSessions { 
                    other_session: SessionId::new() // Placeholder, actual ID comes from event
                },
            },
            Transition {
                guards: vec![Guard::DialogEstablished, Guard::MediaReady],
                actions: vec![
                    Action::CreateBridge(SessionId::new()), // Placeholder
                ],
                next_state: Some(CallState::Bridged),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("BridgeCreated".to_string()),
                ],
            },
        );
        
        // Bridged -> Active: Destroy bridge
        table.insert(
            StateKey {
                role,
                state: CallState::Bridged,
                event: EventType::HangupCall,
            },
            Transition {
                guards: vec![],
                actions: vec![
                    Action::DestroyBridge,
                    Action::SendBYE,
                ],
                next_state: Some(CallState::Terminating),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("BridgeDestroyed".to_string()),
                ],
            },
        );
        
        // Active -> Transferring: Blind transfer
        table.insert(
            StateKey {
                role,
                state: CallState::Active,
                event: EventType::BlindTransfer { 
                    target: String::new() // Placeholder
                },
            },
            Transition {
                guards: vec![Guard::DialogEstablished],
                actions: vec![
                    Action::InitiateBlindTransfer(String::new()), // Placeholder
                    Action::Custom("SendREFER".to_string()),
                ],
                next_state: Some(CallState::Transferring),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("TransferInitiated".to_string()),
                ],
            },
        );
        
        // Active -> Transferring: Attended transfer
        table.insert(
            StateKey {
                role,
                state: CallState::Active,
                event: EventType::AttendedTransfer { 
                    target: String::new() // Placeholder
                },
            },
            Transition {
                guards: vec![Guard::DialogEstablished, Guard::MediaReady],
                actions: vec![
                    Action::InitiateAttendedTransfer(String::new()), // Placeholder
                    Action::Custom("CreateConsultCall".to_string()),
                ],
                next_state: Some(CallState::Transferring),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("AttendedTransferStarted".to_string()),
                ],
            },
        );
        
        // Transferring -> Terminated: Transfer completed
        table.insert(
            StateKey {
                role,
                state: CallState::Transferring,
                event: EventType::DialogBYE,
            },
            Transition {
                guards: vec![],
                actions: vec![
                    Action::StartMediaCleanup,
                    Action::StartDialogCleanup,
                ],
                next_state: Some(CallState::Terminated),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("TransferCompleted".to_string()),
                    EventTemplate::CallTerminated,
                ],
            },
        );
        
        // OnHold -> Bridged: Bridge a held call
        table.insert(
            StateKey {
                role,
                state: CallState::OnHold,
                event: EventType::BridgeSessions { 
                    other_session: SessionId::new() 
                },
            },
            Transition {
                guards: vec![],
                actions: vec![
                    Action::SendReINVITE, // Resume media
                    Action::CreateBridge(SessionId::new()),
                ],
                next_state: Some(CallState::Bridged),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::Custom("BridgeCreatedFromHold".to_string()),
                ],
            },
        );
    }
    
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bridge_transitions() {
        let table = load_bridge_transitions();
        
        // Test that we have bridge transitions for both roles in Active state
        let has_uac_bridge = table.keys().any(|k| {
            k.role == Role::UAC && 
            k.state == CallState::Active &&
            matches!(k.event, EventType::BridgeSessions { .. })
        });
        assert!(has_uac_bridge, "UAC should have bridge transition from Active");
        
        let has_uas_bridge = table.keys().any(|k| {
            k.role == Role::UAS && 
            k.state == CallState::Active &&
            matches!(k.event, EventType::BridgeSessions { .. })
        });
        assert!(has_uas_bridge, "UAS should have bridge transition from Active");
        
        // Test blind transfer
        let has_blind_transfer = table.keys().any(|k| {
            k.role == Role::UAC && 
            k.state == CallState::Active &&
            matches!(k.event, EventType::BlindTransfer { .. })
        });
        assert!(has_blind_transfer, "Should have blind transfer transition from Active");
    }
}