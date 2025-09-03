use rvoip_session_core_v2::{
    state_table::*,
    session_store::SessionState,
    Role, CallState, EventType,
};

#[tokio::test]
async fn test_uac_normal_flow() {
    let table = &*MASTER_TABLE;
    
    // Test UAC transitions: Idle -> Initiating -> Ringing -> Active
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall,
    };
    
    let transition = table.get(&key).expect("Should have MakeCall transition");
    assert_eq!(transition.next_state, Some(CallState::Initiating));
    
    // Initiating -> Ringing
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Initiating,
        event: EventType::Dialog180Ringing,
    };
    
    let transition = table.get(&key).expect("Should have 180 Ringing transition");
    assert_eq!(transition.next_state, Some(CallState::Ringing));
    
    // Ringing -> Active
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Ringing,
        event: EventType::Dialog200OK,
    };
    
    let transition = table.get(&key).expect("Should have 200 OK transition");
    assert_eq!(transition.next_state, Some(CallState::Active));
}

#[tokio::test]
async fn test_uas_normal_flow() {
    let table = &*MASTER_TABLE;
    
    // Test UAS transitions: Idle -> Initiating -> Active
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Idle,
        event: EventType::DialogInvite,
    };
    
    let transition = table.get(&key).expect("Should have INVITE transition");
    assert_eq!(transition.next_state, Some(CallState::Initiating));
    
    // Initiating -> Active (accept call)
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Initiating,
        event: EventType::AcceptCall,
    };
    
    let transition = table.get(&key).expect("Should have AcceptCall transition");
    assert_eq!(transition.next_state, Some(CallState::Active));
}

#[tokio::test]
async fn test_media_flow_established_events() {
    let table = &*MASTER_TABLE;
    
    // UAC publishes MediaFlowEstablished after rfc_compliant_media_creation_uac
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Active,
        event: EventType::MediaEvent("rfc_compliant_media_creation_uac".to_string()),
    };
    
    let transition = table.get(&key).expect("Should have UAC media event transition");
    assert!(transition.publish_events.contains(&EventTemplate::MediaFlowEstablished));
    
    // UAS publishes MediaFlowEstablished after rfc_compliant_media_creation_uas
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::MediaEvent("rfc_compliant_media_creation_uas".to_string()),
    };
    
    let transition = table.get(&key).expect("Should have UAS media event transition");
    assert!(transition.publish_events.contains(&EventTemplate::MediaFlowEstablished));
}

#[tokio::test]
async fn test_condition_updates() {
    let table = &*MASTER_TABLE;
    
    // Test that 200 OK sets dialog_established for UAC
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Ringing,
        event: EventType::Dialog200OK,
    };
    
    let transition = table.get(&key).expect("Should have 200 OK transition");
    assert_eq!(transition.condition_updates.dialog_established, Some(true));
    
    // Test that ACK sets dialog_established for UAS
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::DialogACK,
    };
    
    let transition = table.get(&key).expect("Should have ACK transition");
    assert_eq!(transition.condition_updates.dialog_established, Some(true));
}

#[tokio::test]
async fn test_termination_flow() {
    let table = &*MASTER_TABLE;
    
    // UAC hangup
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Active,
        event: EventType::HangupCall,
    };
    
    let transition = table.get(&key).expect("Should have HangupCall transition");
    assert_eq!(transition.next_state, Some(CallState::Terminating));
    assert!(transition.actions.contains(&Action::SendBYE));
    
    // UAS receives BYE
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::DialogBYE,
    };
    
    let transition = table.get(&key).expect("Should have BYE transition");
    assert_eq!(transition.next_state, Some(CallState::Terminating));
    assert!(transition.actions.contains(&Action::SendSIPResponse(200, "OK".to_string())));
}

#[tokio::test]
async fn test_cancel_flow() {
    let table = &*MASTER_TABLE;
    
    // UAC cancels while initiating
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Initiating,
        event: EventType::HangupCall,
    };
    
    let transition = table.get(&key).expect("Should have cancel transition");
    assert_eq!(transition.next_state, Some(CallState::Terminating));
    assert!(transition.actions.contains(&Action::SendCANCEL));
    
    // UAS receives CANCEL
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Initiating,
        event: EventType::DialogCANCEL,
    };
    
    let transition = table.get(&key).expect("Should have CANCEL transition");
    assert_eq!(transition.next_state, Some(CallState::Terminating));
}

#[tokio::test]
async fn test_session_state_conditions() {
    let session_id = rvoip_session_core_v2::api::types::SessionId::new();
    let mut session = SessionState::new(session_id, Role::UAC);
    
    // Initially no conditions are met
    assert!(!session.all_conditions_met());
    
    // Set conditions one by one
    session.dialog_established = true;
    assert!(!session.all_conditions_met());
    
    session.media_session_ready = true;
    assert!(!session.all_conditions_met());
    
    session.sdp_negotiated = true;
    assert!(session.all_conditions_met());
    
    // Test condition updates
    let updates = ConditionUpdates {
        dialog_established: Some(false),
        media_session_ready: None,
        sdp_negotiated: Some(false),
    };
    
    session.apply_condition_updates(&updates);
    assert!(!session.dialog_established);
    assert!(session.media_session_ready); // Unchanged
    assert!(!session.sdp_negotiated);
}