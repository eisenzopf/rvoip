//! Example demonstrating YAML-based state table loading
//! 
//! This example shows how to:
//! 1. Load the default embedded state table from YAML
//! 2. Load a custom state table from a file
//! 3. Merge multiple YAML files
//! 4. Use the state table with the state machine

use rvoip_session_core_v2::{
    state_table::{YamlTableLoader, StateTableBuilder, Role, CallState, EventType, StateKey},
    state_machine::executor::StateMachine,
    session_store::SessionStore,
    errors::Result,
};
use std::sync::Arc;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("YAML State Table Example");
    
    // Example 1: Load the default embedded state table
    info!("\n=== Loading Default Embedded State Table ===");
    let default_table = YamlTableLoader::load_default()?;
    info!("Loaded default table with {} transitions", count_transitions(&default_table));
    
    // Validate the default table
    match default_table.validate() {
        Ok(()) => info!("Default table validation passed"),
        Err(errors) => {
            error!("Default table validation failed:");
            for err in errors {
                error!("  - {}", err);
            }
        }
    }
    
    // Example 2: Load from a custom YAML file (if it exists)
    info!("\n=== Loading Custom State Table ===");
    let custom_path = "state_tables/custom.yaml";
    match YamlTableLoader::load_from_file(custom_path) {
        Ok(custom_table) => {
            info!("Loaded custom table from {} with {} transitions", 
                  custom_path, count_transitions(&custom_table));
        }
        Err(e) => {
            info!("No custom table found at {} (this is expected): {}", custom_path, e);
        }
    }
    
    // Example 3: Merge multiple YAML files
    info!("\n=== Merging Multiple YAML Files ===");
    let mut loader = YamlTableLoader::new();
    
    // Load the base table
    loader.load_from_string(include_str!("../state_tables/session_coordination.yaml"))?;
    
    // Create a simple extension YAML
    let extension_yaml = r#"
version: "1.0"
transitions:
  # Add a custom business logic transition
  - role: Both
    state: Active
    event: 
      type: Custom
      name: "StartConferenceMode"
    actions:
      - Custom(EnableConference)
    publish:
      - Custom(ConferenceModeStarted)
    description: "Custom transition for conference mode"
"#;
    
    // Merge the extension
    loader.merge_string(extension_yaml)?;
    let merged_table = loader.build()?;
    info!("Merged table has {} transitions", count_transitions(&merged_table));
    
    // Example 4: Use the state table with a state machine
    info!("\n=== Using State Table with State Machine ===");
    
    // Create session store and state machine
    let store = Arc::new(SessionStore::new());
    let state_machine = StateMachine::new(Arc::new(default_table), store.clone());
    
    // Create a test session
    let session_id = rvoip_session_core_v2::state_table::SessionId::new();
    store.create_session(session_id.clone(), Role::UAC).await?;
    
    // Test a transition: MakeCall from Idle
    info!("Testing transition: MakeCall from Idle state");
    let make_call_event = EventType::MakeCall { 
        target: "sip:bob@example.com".to_string() 
    };
    
    // Check if transition exists
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: make_call_event.clone(),
    };
    
    if state_machine.has_transition(&key) {
        info!("✓ Transition exists for MakeCall from Idle");
        
        // Process the event
        match state_machine.process_event(&session_id, make_call_event).await {
            Ok(result) => {
                info!("✓ Event processed successfully");
                if let Some(next_state) = result.next_state {
                    info!("  Transitioned to: {:?}", next_state);
                }
                if !result.actions_executed.is_empty() {
                    info!("  Actions executed: {:?}", result.actions_executed);
                }
                if !result.events_published.is_empty() {
                    info!("  Events published: {:?}", result.events_published);
                }
            }
            Err(e) => {
                error!("✗ Failed to process event: {}", e);
            }
        }
    } else {
        error!("✗ No transition found for MakeCall from Idle");
    }
    
    // Example 5: Demonstrate state table introspection
    info!("\n=== State Table Introspection ===");
    
    // Check various states for available transitions
    for state in [CallState::Idle, CallState::Active, CallState::OnHold] {
        let transition_count = count_state_transitions(&state_machine, Role::UAC, state);
        info!("State {:?} has {} available transitions for UAC", state, transition_count);
    }
    
    // Example 6: Create a minimal YAML table for testing
    info!("\n=== Creating Minimal Test Table ===");
    let minimal_yaml = r#"
version: "1.0"
metadata:
  description: "Minimal test table"
transitions:
  - role: UAC
    state: Idle
    event: MakeCall
    next_state: Initiating
    actions:
      - SendINVITE
    publish:
      - SessionCreated
  
  - role: UAC
    state: Initiating
    event: Dialog200OK
    next_state: Active
    actions:
      - SendACK
    publish:
      - CallEstablished
  
  - role: Both
    state: Active
    event: HangupCall
    next_state: Terminated
    actions:
      - SendBYE
    publish:
      - CallTerminated
"#;
    
    let mut minimal_loader = YamlTableLoader::new();
    minimal_loader.load_from_string(minimal_yaml)?;
    let minimal_table = minimal_loader.build()?;
    info!("Created minimal table with {} transitions", count_transitions(&minimal_table));
    
    info!("\n=== Example Complete ===");
    Ok(())
}

/// Helper function to count transitions in a state table
fn count_transitions(table: &rvoip_session_core_v2::state_table::StateTable) -> usize {
    // This is an approximation since we don't have direct access to the internal HashMap
    // In a real implementation, you might want to add a method to StateTable for this
    let mut count = 0;
    for role in [Role::UAC, Role::UAS, Role::Both] {
        for state in [
            CallState::Idle,
            CallState::Initiating,
            CallState::Ringing,
            CallState::EarlyMedia,
            CallState::Active,
            CallState::OnHold,
            CallState::Resuming,
            CallState::Bridged,
            CallState::Transferring,
            CallState::Terminating,
            CallState::Terminated,
        ] {
            // Check a few common events
            for event in [
                EventType::MakeCall { target: String::new() },
                EventType::AcceptCall,
                EventType::HangupCall,
                EventType::Dialog200OK,
                EventType::DialogBYE,
            ] {
                let key = StateKey {
                    role,
                    state,
                    event: event.clone(),
                };
                if table.has_transition(&key) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Helper function to count transitions available from a specific state
fn count_state_transitions(
    state_machine: &StateMachine,
    role: Role,
    state: CallState,
) -> usize {
    let mut count = 0;
    
    // Check common events that might be available
    let events = vec![
        EventType::MakeCall { target: String::new() },
        EventType::AcceptCall,
        EventType::RejectCall { reason: String::new() },
        EventType::HangupCall,
        EventType::HoldCall,
        EventType::ResumeCall,
        EventType::Dialog180Ringing,
        EventType::Dialog200OK,
        EventType::DialogBYE,
        EventType::DialogCANCEL,
        EventType::MediaEvent("media_ready".to_string()),
    ];
    
    for event in events {
        let key = StateKey { role, state, event };
        if state_machine.has_transition(&key) {
            count += 1;
        }
    }
    
    count
}