use crate::state_table::{
    StateTableBuilder, Role, CallState, EventType, Transition, Guard, Action,
    ConditionUpdates, EventTemplate, Condition,
};

/// Add all UAS (User Agent Server) transitions to the table
pub fn add_uas_transitions(builder: &mut StateTableBuilder) {
    // Idle -> Initiating: Incoming call
    builder.add_transition(
        Role::UAS,
        CallState::Idle,
        EventType::DialogInvite,
        Transition {
            guards: vec![Guard::HasRemoteSDP],
            actions: vec![
                Action::StoreRemoteSDP,
                Action::StartMediaSession,
                Action::NegotiateSDPAsUAS,
            ],
            next_state: Some(CallState::Initiating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::IncomingCall],
        },
    );
    
    // Initiating: Accept the call
    builder.add_transition(
        Role::UAS,
        CallState::Initiating,
        EventType::AcceptCall,
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendSIPResponse(200, "OK".to_string()),
                Action::StoreLocalSDP,
            ],
            next_state: Some(CallState::Active),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
    
    // Initiating: Reject the call
    builder.add_transition(
        Role::UAS,
        CallState::Initiating,
        EventType::RejectCall { reason: String::new() },  // Placeholder
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendSIPResponse(486, "Busy Here".to_string()),
                Action::StartMediaCleanup,
                Action::StartDialogCleanup,
            ],
            next_state: Some(CallState::Terminating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
    
    // Active: ACK received - dialog is now established
    builder.add_transition(
        Role::UAS,
        CallState::Active,
        EventType::DialogACK,
        Transition {
            guards: vec![],
            actions: vec![],
            next_state: None,
            condition_updates: ConditionUpdates::set_dialog_established(true),
            publish_events: vec![EventTemplate::Custom("rfc_compliant_media_creation_uas".to_string())],
        },
    );
    
    // Active: Publish MediaFlowEstablished for UAS after media event
    builder.add_transition(
        Role::UAS,
        CallState::Active,
        EventType::MediaEvent("rfc_compliant_media_creation_uas".to_string()),
        Transition {
            guards: vec![Guard::HasNegotiatedConfig],
            actions: vec![],
            next_state: None,
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::MediaFlowEstablished],
        },
    );
    
    // Active: Media negotiated
    builder.add_condition_setter(
        Role::UAS,
        CallState::Active,
        EventType::MediaNegotiated,
        Condition::SDPNegotiated,
        true,
    );
    
    // Active: Media session ready
    builder.add_condition_setter(
        Role::UAS,
        CallState::Active,
        EventType::MediaSessionReady,
        Condition::MediaSessionReady,
        true,
    );
    
    // Active: Check if all conditions are met
    builder.add_transition(
        Role::UAS,
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
        Role::UAS,
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
        Role::UAS,
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
        Role::UAS,
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
    
    // Initiating -> Terminating: Incoming call cancelled
    builder.add_transition(
        Role::UAS,
        CallState::Initiating,
        EventType::DialogCANCEL,
        Transition {
            guards: vec![],
            actions: vec![
                Action::SendSIPResponse(200, "OK".to_string()),
                Action::SendSIPResponse(487, "Request Terminated".to_string()),
                Action::StartMediaCleanup,
                Action::StartDialogCleanup,
            ],
            next_state: Some(CallState::Terminating),
            condition_updates: ConditionUpdates::none(),
            publish_events: vec![EventTemplate::StateChanged],
        },
    );
}