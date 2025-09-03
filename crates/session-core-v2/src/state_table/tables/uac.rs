use crate::state_table::{
    StateTableBuilder, Role, CallState, EventType, Transition, Guard, Action,
    ConditionUpdates, EventTemplate, Condition,
};

/// Add all UAC (User Agent Client) transitions to the table
pub fn add_uac_transitions(builder: &mut StateTableBuilder) {
    // Idle -> Initiating: Start a call
    builder.add_transition(
        Role::UAC,
        CallState::Idle,
        EventType::MakeCall { target: String::new() },  // Placeholder
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendINVITE,
                Action::StartMediaSession,
            ],
            next_state: Some(CallState::Initiating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::SessionCreated],
        },
    );
    
    // Initiating -> Ringing: Remote is ringing
    builder.add_state_change(
        Role::UAC,
        CallState::Initiating,
        EventType::Dialog180Ringing,
        CallState::Ringing,
    );
    
    // Ringing -> Active: Call answered
    builder.add_transition(
        Role::UAC,
        CallState::Ringing,
        EventType::Dialog200OK,
        Transition {
            guards: vec![Guard::HasRemoteSDP],
            actions: vec![
                Action::SendACK,
                Action::NegotiateSDPAsUAC,
                Action::StoreRemoteSDP,
            ],
            next_state: Some(CallState::Active),
            condition_updates: ConditionUpdates::set_dialog_established(true),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
    
    // Active: ACK was sent (internal event)
    builder.add_transition(
        Role::UAC,
        CallState::Active,
        EventType::InternalACKSent,
        Transition {
            guards: vec![],
            actions: vec![],
            next_state: None,
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::Custom("rfc_compliant_media_creation_uac".to_string())],
        },
    );
    
    // Active: Media negotiated
    builder.add_condition_setter(
        Role::UAC,
        CallState::Active,
        EventType::MediaNegotiated,
        Condition::SDPNegotiated,
        true,
    );
    
    // Active: Media session ready
    builder.add_condition_setter(
        Role::UAC,
        CallState::Active,
        EventType::MediaSessionReady,
        Condition::MediaSessionReady,
        true,
    );
    
    // Active: Publish MediaFlowEstablished for UAC after media event
    builder.add_transition(
        Role::UAC,
        CallState::Active,
        EventType::MediaEvent("rfc_compliant_media_creation_uac".to_string()),
        Transition {
            guards: vec![Guard::HasNegotiatedConfig],
            actions: vec![],
            next_state: None,
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::MediaFlowEstablished],
        },
    );
    
    // Active: Check if all conditions are met
    builder.add_transition(
        Role::UAC,
        CallState::Active,
        EventType::InternalCheckReady,
        Transition {
            guards: vec![Guard::AllConditionsMet],
            actions: vec![Action::TriggerCallEstablished],
            next_state: None,
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::CallEstablished],
        },
    );
    
    // Active -> Terminating: User hangs up
    builder.add_transition(
        Role::UAC,
        CallState::Active,
        EventType::HangupCall,
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendBYE,
                Action::StartMediaCleanup,
            ],
            next_state: Some(CallState::Terminating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
    
    // Active -> Terminating: Remote hangs up
    builder.add_transition(
        Role::UAC,
        CallState::Active,
        EventType::DialogBYE,
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendSIPResponse(200, "OK".to_string()),
                Action::StartMediaCleanup,
                Action::StartDialogCleanup,
            ],
            next_state: Some(CallState::Terminating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
    
    // Terminating -> Terminated: Cleanup complete
    builder.add_transition(
        Role::UAC,
        CallState::Terminating,
        EventType::InternalCleanupComplete,
        Transition {
            guards: vec![],
            actions: vec![Action::TriggerCallTerminated],
            next_state: Some(CallState::Terminated),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::CallTerminated],
        },
    );
    
    // Initiating -> Terminating: Call cancelled
    builder.add_transition(
        Role::UAC,
        CallState::Initiating,
        EventType::HangupCall,
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendCANCEL,
                Action::StartMediaCleanup,
                Action::StartDialogCleanup,
            ],
            next_state: Some(CallState::Terminating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
    
    // Ringing -> Terminating: Call cancelled while ringing
    builder.add_transition(
        Role::UAC,
        CallState::Ringing,
        EventType::HangupCall,
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendCANCEL,
                Action::StartMediaCleanup,
                Action::StartDialogCleanup,
            ],
            next_state: Some(CallState::Terminating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
}