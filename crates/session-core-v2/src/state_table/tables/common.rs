use crate::state_table::{
    StateTableBuilder, Role, CallState, EventType, Transition, Action,
    ConditionUpdates, EventTemplate, FailureReason,
};

/// Add common transitions that apply to both UAC and UAS
pub fn add_common_transitions(builder: &mut StateTableBuilder) {
    // Add transitions for both roles
    for role in [Role::UAC, Role::UAS] {
        // Any state -> Failed: Dialog error
        for state in [
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
        ] {
            builder.add_transition(
                role,
                state,
                EventType::DialogError("network".to_string()),
                Transition {
                    guards: vec![],
                    actions: vec![
                        Action::StartMediaCleanup,
                        Action::StartDialogCleanup,
                    ],
                    next_state: Some(CallState::Failed(FailureReason::NetworkError)),
                    condition_updates: ConditionUpdates::none(),
                    publish_events: vec![EventTemplate::StateChanged],
                },
            );
            
            builder.add_transition(
                role,
                state,
                EventType::MediaError("media".to_string()),
                Transition {
                    guards: vec![],
                    actions: vec![
                        Action::StartMediaCleanup,
                        Action::StartDialogCleanup,
                    ],
                    next_state: Some(CallState::Failed(FailureReason::MediaError)),
                    condition_updates: ConditionUpdates::none(),
                    publish_events: vec![EventTemplate::StateChanged],
                },
            );
        }
        
        // Active -> OnHold: Put call on hold
        builder.add_transition(
            role,
            CallState::Active,
            EventType::HoldCall,
            Transition {
                guards: vec![],
                actions: vec![
                    Action::SendReINVITE,
                ],
                next_state: Some(CallState::OnHold),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![EventTemplate::StateChanged],
            },
        );
        
        // OnHold -> Active: Resume call
        builder.add_transition(
            role,
            CallState::OnHold,
            EventType::ResumeCall,
            Transition {
                guards: vec![],
                actions: vec![
                    Action::SendReINVITE,
                ],
                next_state: Some(CallState::Active),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![EventTemplate::StateChanged],
            },
        );
    }
}