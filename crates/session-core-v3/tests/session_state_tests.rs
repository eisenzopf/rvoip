//! SessionState Logic Tests
//!
//! Tests the core SessionState methods: construction, condition tracking,
//! state transitions, and history recording.

use rvoip_session_core_v3::internals::{
    SessionId, Role,
    SessionState,
};
use rvoip_session_core_v3::session_store::{
    HistoryConfig, TransferState,
};
use rvoip_session_core_v3::state_table::ConditionUpdates;
use rvoip_session_core_v3::types::CallState;

// ── Construction ────────────────────────────────────────────────────────────

#[test]
fn test_new_session_defaults() {
    let id = SessionId::new();
    let s = SessionState::new(id.clone(), Role::UAC);

    assert_eq!(s.session_id, id);
    assert_eq!(s.role, Role::UAC);
    assert!(matches!(s.call_state, CallState::Idle));
    assert!(!s.dialog_established);
    assert!(!s.media_session_ready);
    assert!(!s.sdp_negotiated);
    assert!(!s.call_established_triggered);
    assert!(s.local_sdp.is_none());
    assert!(s.remote_sdp.is_none());
    assert!(s.negotiated_config.is_none());
    assert!(s.dialog_id.is_none());
    assert!(s.media_session_id.is_none());
    assert!(s.call_id.is_none());
    assert!(s.local_uri.is_none());
    assert!(s.remote_uri.is_none());
    assert!(s.bridged_to.is_none());
    assert_eq!(s.transfer_state, TransferState::None);
    assert!(!s.is_transfer_call);
    assert!(!s.is_registered);
    assert_eq!(s.registration_retry_count, 0);
    assert!(s.history.is_none());
}

#[test]
fn test_new_session_uas_role() {
    let s = SessionState::new(SessionId::new(), Role::UAS);
    assert_eq!(s.role, Role::UAS);
}

#[test]
fn test_with_history_creates_history() {
    let s = SessionState::with_history(
        SessionId::new(),
        Role::UAC,
        HistoryConfig::default(),
    );
    assert!(s.history.is_some());
}

// ── all_conditions_met ──────────────────────────────────────────────────────

#[test]
fn test_all_conditions_met_all_true() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    s.dialog_established = true;
    s.media_session_ready = true;
    s.sdp_negotiated = true;
    assert!(s.all_conditions_met());
}

#[test]
fn test_all_conditions_met_only_dialog() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    s.dialog_established = true;
    assert!(!s.all_conditions_met());
}

#[test]
fn test_all_conditions_met_only_media() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    s.media_session_ready = true;
    assert!(!s.all_conditions_met());
}

#[test]
fn test_all_conditions_met_only_sdp() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    s.sdp_negotiated = true;
    assert!(!s.all_conditions_met());
}

#[test]
fn test_all_conditions_met_two_of_three() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    s.dialog_established = true;
    s.media_session_ready = true;
    assert!(!s.all_conditions_met());
}

#[test]
fn test_all_conditions_met_none() {
    let s = SessionState::new(SessionId::new(), Role::UAC);
    assert!(!s.all_conditions_met());
}

// ── apply_condition_updates ─────────────────────────────────────────────────

#[test]
fn test_apply_condition_updates_all() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    let updates = ConditionUpdates {
        dialog_established: Some(true),
        media_session_ready: Some(true),
        sdp_negotiated: Some(true),
    };
    s.apply_condition_updates(&updates);
    assert!(s.dialog_established);
    assert!(s.media_session_ready);
    assert!(s.sdp_negotiated);
}

#[test]
fn test_apply_condition_updates_selective() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    // Only update dialog_established
    let updates = ConditionUpdates {
        dialog_established: Some(true),
        media_session_ready: None,
        sdp_negotiated: None,
    };
    s.apply_condition_updates(&updates);
    assert!(s.dialog_established);
    assert!(!s.media_session_ready);
    assert!(!s.sdp_negotiated);
}

#[test]
fn test_apply_condition_updates_can_unset() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    s.dialog_established = true;
    s.media_session_ready = true;
    let updates = ConditionUpdates {
        dialog_established: Some(false),
        media_session_ready: None, // leave unchanged
        sdp_negotiated: None,
    };
    s.apply_condition_updates(&updates);
    assert!(!s.dialog_established);
    assert!(s.media_session_ready); // unchanged
}

#[test]
fn test_apply_condition_updates_none_is_noop() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    s.dialog_established = true;
    let updates = ConditionUpdates::none();
    s.apply_condition_updates(&updates);
    assert!(s.dialog_established); // still true
    assert!(!s.media_session_ready);
}

// ── transition_to ───────────────────────────────────────────────────────────

#[test]
fn test_transition_to_updates_call_state() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    assert!(matches!(s.call_state, CallState::Idle));
    s.transition_to(CallState::Active);
    assert!(matches!(s.call_state, CallState::Active));
}

#[test]
fn test_transition_to_resets_entered_state_at() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    let first = s.entered_state_at;
    // Small spin to ensure the Instant advances
    std::thread::sleep(std::time::Duration::from_millis(5));
    s.transition_to(CallState::Initiating);
    assert!(s.entered_state_at > first);
}

#[test]
fn test_transition_to_records_history() {
    let mut s = SessionState::with_history(
        SessionId::new(),
        Role::UAC,
        HistoryConfig::default(),
    );
    s.transition_to(CallState::Active);
    let history = s.history.as_ref().unwrap();
    assert!(history.total_transitions > 0);
}

#[test]
fn test_transition_to_without_history_is_fine() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    // No history — should not panic
    s.transition_to(CallState::Active);
    assert!(matches!(s.call_state, CallState::Active));
}

// ── time helpers ────────────────────────────────────────────────────────────

#[test]
fn test_time_in_state_after_sleep() {
    let s = SessionState::new(SessionId::new(), Role::UAC);
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(s.time_in_state().as_millis() >= 4);
}

#[test]
fn test_session_duration_after_sleep() {
    let s = SessionState::new(SessionId::new(), Role::UAC);
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(s.session_duration().as_millis() >= 4);
}

// ── transfer state lifecycle ────────────────────────────────────────────────

#[test]
fn test_transfer_state_lifecycle() {
    let mut s = SessionState::new(SessionId::new(), Role::UAC);
    assert_eq!(s.transfer_state, TransferState::None);

    s.transfer_state = TransferState::TransferInitiated;
    assert_eq!(s.transfer_state, TransferState::TransferInitiated);

    s.transfer_state = TransferState::TransferCompleted;
    assert_eq!(s.transfer_state, TransferState::TransferCompleted);
}
