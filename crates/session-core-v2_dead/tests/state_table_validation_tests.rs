//! State Table Validation Tests
//!
//! This test suite validates that the default state table:
//! - Can be loaded successfully without errors
//! - Has valid YAML structure and syntax
//! - Defines transitions for common scenarios
//! - Is compatible with the current state table loader

use rvoip_session_core_v2::state_table::{
    YamlTableLoader, StateTable, StateKey, EventType, Role
};
use rvoip_session_core_v2::types::CallState;
use std::path::Path;

/// Helper to load a state table from the state_tables directory
fn load_state_table(filename: &str) -> Result<StateTable, Box<dyn std::error::Error>> {
    let path = Path::new("state_tables").join(filename);
    YamlTableLoader::load_from_file(path)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

#[test]
fn test_default_state_table_loads() {
    let result = load_state_table("default.yaml");
    assert!(result.is_ok(), "Failed to load default.yaml: {:?}", result.err());

    let table = result.unwrap();

    // Verify basic call flow transitions exist
    // Check Idle -> Initiating transition exists for making a call
    let make_call_key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall { target: String::new() },
    };
    assert!(
        table.has_transition(&make_call_key),
        "Missing basic MakeCall transition from Idle state"
    );

    // Check for incoming call handling
    let incoming_call_key = StateKey {
        role: Role::UAS,
        state: CallState::Idle,
        event: EventType::IncomingCall { from: String::new(), sdp: None },
    };
    assert!(
        table.has_transition(&incoming_call_key),
        "Missing IncomingCall transition from Idle state"
    );
}

#[test]
fn test_embedded_default_table_loads() {
    // Test loading the embedded default table
    let result = YamlTableLoader::load_default();
    assert!(result.is_ok(), "Failed to load embedded default table: {:?}", result.err());

    let table = result.unwrap();

    // Verify it has basic transitions
    let make_call_key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall { target: String::new() },
    };
    assert!(
        table.has_transition(&make_call_key),
        "Embedded table missing basic MakeCall transition"
    );
}

#[test]
fn test_hold_resume_transitions() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check hold transition from Active
    let hold_key = StateKey {
        role: Role::Both,
        state: CallState::Active,
        event: EventType::HoldCall,
    };
    assert!(
        table.has_transition(&hold_key),
        "Missing hold transition from Active state"
    );

    // Check resume transition from OnHold
    let resume_key = StateKey {
        role: Role::Both,
        state: CallState::OnHold,
        event: EventType::ResumeCall,
    };
    assert!(
        table.has_transition(&resume_key),
        "Missing resume transition from OnHold state"
    );
}

#[test]
fn test_basic_call_termination() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check that Active state can handle hangup
    let hangup_key = StateKey {
        role: Role::Both,
        state: CallState::Active,
        event: EventType::HangupCall,
    };

    assert!(
        table.has_transition(&hangup_key),
        "Missing HangupCall transition from Active state"
    );
}

#[test]
fn test_error_handling_transitions() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check network error handling in various states
    let states_to_check = vec![
        CallState::Initiating,
        CallState::Ringing,
        CallState::Active,
    ];

    for state in states_to_check {
        let error_key = StateKey {
            role: Role::Both,
            state: state.clone(),
            event: EventType::DialogError(String::new()),
        };

        let has_error_handling = table.has_transition(&error_key);
        println!("State {:?} handles DialogError: {}", state, has_error_handling);
    }
}

#[test]
fn test_media_event_handling() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check media ready handling
    let media_ready_key = StateKey {
        role: Role::Both,
        state: CallState::Initiating,
        event: EventType::MediaSessionReady,
    };

    let has_media_ready = table.has_transition(&media_ready_key);
    println!("Table handles MediaReady event: {}", has_media_ready);

    // Check media failed handling
    let media_failed_key = StateKey {
        role: Role::Both,
        state: CallState::Initiating,
        event: EventType::MediaError(String::new()),
    };

    let has_media_failed = table.has_transition(&media_failed_key);
    println!("Table handles MediaFailed event: {}", has_media_failed);
}

#[test]
fn test_registration_transitions() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check registration start
    let register_key = StateKey {
        role: Role::Both,
        state: CallState::Idle,
        event: EventType::StartRegistration,
    };

    let has_register = table.has_transition(&register_key);
    assert!(has_register, "Missing registration transition from Idle");

    // Check registration success
    let register_ok_key = StateKey {
        role: Role::Both,
        state: CallState::Registering,
        event: EventType::Registration200OK,
    };

    let has_register_ok = table.has_transition(&register_ok_key);
    assert!(has_register_ok, "Missing registration success transition");
}

#[test]
fn test_subscription_transitions() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check subscription start
    let subscribe_key = StateKey {
        role: Role::Both,
        state: CallState::Idle,
        event: EventType::StartSubscription,
    };

    let has_subscribe = table.has_transition(&subscribe_key);
    assert!(has_subscribe, "Missing subscription transition from Idle");

    // Check notification handling
    let notify_key = StateKey {
        role: Role::Both,
        state: CallState::Subscribed,
        event: EventType::ReceiveNOTIFY,
    };

    let has_notify = table.has_transition(&notify_key);
    assert!(has_notify, "Missing NOTIFY handling in Subscribed state");
}

#[test]
fn test_transfer_transitions() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check blind transfer initiation
    let transfer_key = StateKey {
        role: Role::Both,
        state: CallState::Active,
        event: EventType::InitiateTransfer { target: String::new() },
    };

    let has_transfer = table.has_transition(&transfer_key);
    assert!(has_transfer, "Missing transfer transition from Active");

    // Check transfer completion
    let transfer_complete_key = StateKey {
        role: Role::Both,
        state: CallState::Transferring,
        event: EventType::TransferComplete,
    };

    let has_transfer_complete = table.has_transition(&transfer_complete_key);
    assert!(has_transfer_complete, "Missing transfer completion transition");
}

#[test]
fn test_message_handling() {
    let table = load_state_table("default.yaml")
        .expect("Failed to load default.yaml");

    // Check MESSAGE sending
    let send_message_key = StateKey {
        role: Role::Both,
        state: CallState::Idle,
        event: EventType::SendMessage,
    };

    let has_send_message = table.has_transition(&send_message_key);
    assert!(has_send_message, "Missing MESSAGE sending transition");

    // Check MESSAGE receiving
    let receive_message_key = StateKey {
        role: Role::Both,
        state: CallState::Idle,
        event: EventType::ReceiveMESSAGE,
    };

    let has_receive_message = table.has_transition(&receive_message_key);
    assert!(has_receive_message, "Missing MESSAGE receiving transition");
}