/// Standalone test for the state table core functionality
/// This tests the state table without requiring the full API layer

#[test]
fn test_state_table_loads() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, MASTER_TABLE
    };
    
    // This will panic if the state table is invalid
    let table = &*MASTER_TABLE;
    
    // Test that we have some transitions
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall,
    };
    
    assert!(table.get(&key).is_some(), "Should have MakeCall transition for UAC");
}

#[test]
fn test_uac_transitions() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, MASTER_TABLE
    };
    
    let table = &*MASTER_TABLE;
    
    // Test UAC flow: Idle -> Initiating -> Ringing -> Active
    let transitions = vec![
        (CallState::Idle, EventType::MakeCall, Some(CallState::Initiating)),
        (CallState::Initiating, EventType::Dialog180Ringing, Some(CallState::Ringing)),
        (CallState::Ringing, EventType::Dialog200OK, Some(CallState::Active)),
    ];
    
    for (from_state, event, expected_next) in transitions {
        let key = StateKey {
            role: Role::UAC,
            state: from_state,
            event,
        };
        
        let transition = table.get(&key)
            .expect(&format!("Should have transition for {:?}", key));
        
        assert_eq!(
            transition.next_state, expected_next,
            "Wrong next state for {:?}", key
        );
    }
}

#[test]
fn test_uas_transitions() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, MASTER_TABLE
    };
    
    let table = &*MASTER_TABLE;
    
    // Test UAS flow: Idle -> Initiating -> Active
    let transitions = vec![
        (CallState::Idle, EventType::DialogInvite, Some(CallState::Initiating)),
        (CallState::Initiating, EventType::AcceptCall, Some(CallState::Active)),
    ];
    
    for (from_state, event, expected_next) in transitions {
        let key = StateKey {
            role: Role::UAS,
            state: from_state,
            event,
        };
        
        let transition = table.get(&key)
            .expect(&format!("Should have transition for {:?}", key));
        
        assert_eq!(
            transition.next_state, expected_next,
            "Wrong next state for {:?}", key
        );
    }
}

#[test]
fn test_media_flow_established_publishing() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, EventTemplate, MASTER_TABLE
    };
    
    let table = &*MASTER_TABLE;
    
    // UAC publishes MediaFlowEstablished after rfc_compliant_media_creation_uac
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Active,
        event: EventType::MediaEvent("rfc_compliant_media_creation_uac".to_string()),
    };
    
    let transition = table.get(&key)
        .expect("Should have UAC media event transition");
    
    assert!(
        transition.publish_events.contains(&EventTemplate::MediaFlowEstablished),
        "UAC should publish MediaFlowEstablished"
    );
    
    // UAS publishes MediaFlowEstablished after rfc_compliant_media_creation_uas
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::MediaEvent("rfc_compliant_media_creation_uas".to_string()),
    };
    
    let transition = table.get(&key)
        .expect("Should have UAS media event transition");
    
    assert!(
        transition.publish_events.contains(&EventTemplate::MediaFlowEstablished),
        "UAS should publish MediaFlowEstablished"
    );
}

#[test]
fn test_condition_updates() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, MASTER_TABLE
    };
    
    let table = &*MASTER_TABLE;
    
    // Test that 200 OK sets dialog_established for UAC
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Ringing,
        event: EventType::Dialog200OK,
    };
    
    let transition = table.get(&key)
        .expect("Should have 200 OK transition");
    
    assert_eq!(
        transition.condition_updates.dialog_established, Some(true),
        "200 OK should set dialog_established for UAC"
    );
    
    // Test that ACK sets dialog_established for UAS
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::DialogACK,
    };
    
    let transition = table.get(&key)
        .expect("Should have ACK transition");
    
    assert_eq!(
        transition.condition_updates.dialog_established, Some(true),
        "ACK should set dialog_established for UAS"
    );
}

#[test]
fn test_termination_transitions() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, Action, MASTER_TABLE
    };
    
    let table = &*MASTER_TABLE;
    
    // Test UAC hangup
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Active,
        event: EventType::HangupCall,
    };
    
    let transition = table.get(&key)
        .expect("Should have HangupCall transition");
    
    assert_eq!(transition.next_state, Some(CallState::Terminating));
    assert!(transition.actions.contains(&Action::SendBYE));
    
    // Test UAS receives BYE
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::DialogBYE,
    };
    
    let transition = table.get(&key)
        .expect("Should have BYE transition");
    
    assert_eq!(transition.next_state, Some(CallState::Terminating));
    assert!(transition.actions.contains(&Action::SendSIPResponse(200, "OK".to_string())));
}

#[test]
fn test_cancel_flow() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, Action, MASTER_TABLE
    };
    
    let table = &*MASTER_TABLE;
    
    // UAC cancels while initiating
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Initiating,
        event: EventType::HangupCall,
    };
    
    let transition = table.get(&key)
        .expect("Should have cancel transition");
    
    assert_eq!(transition.next_state, Some(CallState::Terminating));
    assert!(transition.actions.contains(&Action::SendCANCEL));
    
    // UAS receives CANCEL
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Initiating,
        event: EventType::DialogCANCEL,
    };
    
    let transition = table.get(&key)
        .expect("Should have CANCEL transition");
    
    assert_eq!(transition.next_state, Some(CallState::Terminating));
}

#[test]
fn test_guards_present() {
    use rvoip_session_core_v2::state_table::{
        StateKey, Role, CallState, EventType, Guard, MASTER_TABLE
    };
    
    let table = &*MASTER_TABLE;
    
    // Test that 200 OK for UAC requires remote SDP
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Ringing,
        event: EventType::Dialog200OK,
    };
    
    let transition = table.get(&key)
        .expect("Should have 200 OK transition");
    
    assert!(
        transition.guards.contains(&Guard::HasRemoteSDP),
        "200 OK should require remote SDP"
    );
    
    // Test that INVITE for UAS requires remote SDP
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Idle,
        event: EventType::DialogInvite,
    };
    
    let transition = table.get(&key)
        .expect("Should have INVITE transition");
    
    assert!(
        transition.guards.contains(&Guard::HasRemoteSDP),
        "INVITE should require remote SDP"
    );
}