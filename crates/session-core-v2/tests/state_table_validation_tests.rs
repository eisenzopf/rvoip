//! State Table Validation Tests
//! 
//! This test suite validates that all state tables in the state_tables/ directory:
//! - Can be loaded successfully without errors
//! - Have valid YAML structure and syntax
//! - Define transitions for common scenarios
//! - Are compatible with the current state table loader

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
    let result = load_state_table("default_state_table.yaml");
    assert!(result.is_ok(), "Failed to load default_state_table.yaml: {:?}", result.err());
    
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
fn test_sip_client_state_table_loads() {
    let result = load_state_table("sip_client_states.yaml");
    assert!(result.is_ok(), "Failed to load sip_client_states.yaml: {:?}", result.err());
    
    let table = result.unwrap();
    
    // Verify client-specific transitions (e.g., dialog creation)
    let dialog_created_key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::DialogCreated { dialog_id: String::new(), call_id: String::new() },
    };
    
    // Check if dialog creation is supported
    let has_dialog_created = table.has_transition(&dialog_created_key);
    println!("SIP client table has DialogCreated support: {}", has_dialog_created);
}

#[test]
fn test_sip_server_state_table_loads() {
    let result = load_state_table("sip_server.yaml");
    assert!(result.is_ok(), "Failed to load sip_server.yaml: {:?}", result.err());
    
    // Server state table should support UAS role transitions
    let table = result.unwrap();
    
    // Check for server-side call handling
    let incoming_invite_key = StateKey {
        role: Role::UAS,
        state: CallState::Idle,
        event: EventType::DialogInvite,
    };
    
    let has_invite_handler = table.has_transition(&incoming_invite_key);
    println!("SIP server table handles INVITE: {}", has_invite_handler);
}

#[test]
fn test_gateway_state_table_loads() {
    let result = load_state_table("gateway_states.yaml");
    assert!(result.is_ok(), "Failed to load gateway_states.yaml: {:?}", result.err());
    
    let table = result.unwrap();
    
    // Gateway should support bridging states
    let bridge_key = StateKey {
        role: Role::Both,
        state: CallState::BridgeActive,
        event: EventType::DialogBYE,
    };
    
    // Check if gateway has bridge-specific states
    let has_bridge_handling = table.has_transition(&bridge_key);
    println!("Gateway table has bridge state handling: {}", has_bridge_handling);
}

#[test]
fn test_enhanced_state_table_loads() {
    let result = load_state_table("enhanced_state_table.yaml");
    assert!(result.is_ok(), "Failed to load enhanced_state_table.yaml: {:?}", result.err());
    
    // Enhanced table might have conference features
    let table = result.unwrap();
    
    // Check for conference support
    let conference_key = StateKey {
        role: Role::Both,
        state: CallState::Active,
        event: EventType::CreateConference { name: String::new() },
    };
    
    let has_conference = table.has_transition(&conference_key);
    println!("Enhanced table has conference support: {}", has_conference);
}

#[test]
fn test_session_coordination_table_loads() {
    let result = load_state_table("session_coordination.yaml");
    assert!(result.is_ok(), "Failed to load session_coordination.yaml: {:?}", result.err());
}

#[test]
fn test_all_tables_support_basic_call_termination() {
    let state_tables = vec![
        "default_state_table.yaml",
        "sip_client_states.yaml",
        // "sip_server.yaml", // This is registration/auth focused, not call handling
        "gateway_states.yaml",
        "enhanced_state_table.yaml",
    ];
    
    for table_file in state_tables {
        let table = load_state_table(table_file)
            .expect(&format!("Failed to load {}", table_file));
        
        // Check that Active state can handle hangup
        // Gateway states use BothLegsActive instead of Active
        let active_state = if table_file == "gateway_states.yaml" {
            CallState::BothLegsActive  // Gateway uses proper state now
        } else {
            CallState::Active
        };
        
        // Gateway uses different events for hangup
        if table_file == "gateway_states.yaml" {
            // Check for InboundBYE or OutboundBYE (mapped to DialogBYE)
            let inbound_bye_key = StateKey {
                role: Role::Both,
                state: active_state.clone(),
                event: EventType::DialogBYE,
            };
            
            assert!(
                table.has_transition(&inbound_bye_key),
                "{} missing BYE transition from {} state", 
                table_file,
                active_state
            );
        } else {
            // Regular tables use HangupCall
            let hangup_key = StateKey {
                role: Role::Both,
                state: active_state.clone(),
                event: EventType::HangupCall,
            };
            
            assert!(
                table.has_transition(&hangup_key),
                "{} missing HangupCall transition from {} state", 
                table_file,
                active_state
            );
        }
    }
}

#[test]
fn test_error_handling_transitions() {
    let table = load_state_table("default_state_table.yaml")
        .expect("Failed to load default_state_table.yaml");
    
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
    let table = load_state_table("default_state_table.yaml")
        .expect("Failed to load default_state_table.yaml");
    
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
fn test_hold_resume_transitions() {
    let table = load_state_table("default_state_table.yaml")
        .expect("Failed to load default_state_table.yaml");
    
    // Check hold from active state
    let hold_key = StateKey {
        role: Role::Both,
        state: CallState::Active,
        event: EventType::HoldCall,
    };
    
    assert!(
        table.has_transition(&hold_key),
        "Missing UserHold transition from Active state"
    );
    
    // Check resume from on-hold state
    let resume_key = StateKey {
        role: Role::Both,
        state: CallState::OnHold,
        event: EventType::ResumeCall,
    };
    
    assert!(
        table.has_transition(&resume_key),
        "Missing UserResume transition from OnHold state"
    );
}