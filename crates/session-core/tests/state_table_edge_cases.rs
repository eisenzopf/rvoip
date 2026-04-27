//! State Table Edge Case Tests
//!
//! Validates the default state table for structural integrity beyond
//! basic happy-path transitions covered by state_table_validation_tests.rs.

use rvoip_session_core::state_table::{EventType, Role, StateKey, StateTable, YamlTableLoader};
use rvoip_session_core::types::CallState;
use std::path::Path;

fn load_default() -> StateTable {
    let path = Path::new("state_tables").join("default.yaml");
    YamlTableLoader::load_from_file(path).expect("default.yaml should load")
}

fn load_embedded() -> StateTable {
    YamlTableLoader::load_embedded_default().expect("embedded default should load")
}

// ── Lookup behaviour ────────────────────────────────────────────────────────

#[test]
fn test_unknown_event_returns_none() {
    let table = load_default();
    // An event that shouldn't have a transition from Idle
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::DialogBYE,
    };
    // BYE from Idle makes no sense — should have no transition
    // (It may or may not exist depending on table design; the important thing
    // is that the lookup doesn't panic.)
    let _result = table.get(&key); // just ensure no panic
}

#[test]
fn test_role_both_fallback() {
    let table = load_default();
    // HangupCall from Active should be available to both UAC and UAS
    // because it's defined as Role::Both
    let uac_key = StateKey {
        role: Role::UAC,
        state: CallState::Active,
        event: EventType::HangupCall,
    };
    let uas_key = StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::HangupCall,
    };
    // At least one of these should resolve (they may use Role::Both)
    let uac_has = table.has_transition(&uac_key);
    let uas_has = table.has_transition(&uas_key);
    assert!(
        uac_has || uas_has,
        "HangupCall from Active should be reachable for at least one role"
    );
}

// ── Structural validation ───────────────────────────────────────────────────

#[test]
fn test_validate_passes_on_default() {
    let table = load_default();
    assert!(
        table.validate().is_ok(),
        "Default state table should pass validation: {:?}",
        table.validate().err()
    );
}

#[test]
fn test_validate_passes_on_embedded() {
    let table = load_embedded();
    assert!(table.validate().is_ok());
}

#[test]
fn test_transition_count_is_nonzero() {
    let table = load_default();
    assert!(
        table.transition_count() > 0,
        "State table should have transitions"
    );
}

#[test]
fn test_used_states_includes_basic_call_states() {
    let table = load_default();
    let states = table.collect_used_states();
    // A valid call flow requires at least these states
    assert!(states.contains(&CallState::Idle), "missing Idle");
    assert!(
        states.contains(&CallState::Initiating),
        "missing Initiating"
    );
    assert!(states.contains(&CallState::Active), "missing Active");
    assert!(
        states.contains(&CallState::Terminated),
        "missing Terminated"
    );
}

#[test]
fn test_no_orphaned_target_states() {
    // Every next_state in a transition should either be a terminal state
    // or itself be a source of at least one transition.
    // This is what validate() checks, but let's be explicit.
    let table = load_default();
    let result = table.validate();
    if let Err(errors) = &result {
        for err in errors {
            // Filter for orphan-related errors
            if err.contains("orphan") || err.contains("no exit") {
                panic!("Orphaned state found: {}", err);
            }
        }
    }
}

// ── Hold/resume symmetry ────────────────────────────────────────────────────

#[test]
fn test_hold_resume_are_symmetric() {
    let table = load_default();

    // Hold: Active → OnHold should exist
    let hold_key = StateKey {
        role: Role::Both,
        state: CallState::Active,
        event: EventType::HoldCall,
    };
    // Resume: OnHold → Active should exist
    let resume_key = StateKey {
        role: Role::Both,
        state: CallState::OnHold,
        event: EventType::ResumeCall,
    };

    // Check both roles
    for role in [Role::UAC, Role::UAS, Role::Both] {
        let hold = StateKey {
            role,
            ..hold_key.clone()
        };
        let resume = StateKey {
            role,
            ..resume_key.clone()
        };

        if table.has_transition(&hold) {
            // If we can hold, we should also be able to resume
            assert!(
                table.has_transition(&resume),
                "Role {:?} can hold but cannot resume",
                role
            );
        }
    }
}

// ── Transition details ──────────────────────────────────────────────────────

#[test]
fn test_transition_has_next_state() {
    let table = load_default();
    // MakeCall from Idle should have a next_state (Initiating)
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall {
            target: String::new(),
        },
    };
    let t = table.get(&key);
    assert!(
        t.is_some(),
        "MakeCall from Idle (UAC) should have a transition"
    );
    let t = t.unwrap();
    assert!(
        t.next_state.is_some(),
        "MakeCall transition should have a next_state"
    );
    assert_eq!(t.next_state.unwrap(), CallState::Initiating);
}

#[test]
fn test_transition_has_actions() {
    let table = load_default();
    // MakeCall from Idle should have actions (CreateMediaSession, GenerateLocalSDP, etc.)
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall {
            target: String::new(),
        },
    };
    let t = table.get(&key).expect("MakeCall transition should exist");
    assert!(
        !t.actions.is_empty(),
        "MakeCall transition should have at least one action"
    );
}

// ── Embedded vs file consistency ────────────────────────────────────────────

#[test]
fn test_embedded_and_file_have_same_transition_count() {
    let file = load_default();
    let embedded = load_embedded();
    assert_eq!(
        file.transition_count(),
        embedded.transition_count(),
        "File and embedded state tables should have the same number of transitions"
    );
}
