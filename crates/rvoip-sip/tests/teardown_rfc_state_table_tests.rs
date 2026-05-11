//! State-table coverage for RFC-correct user teardown timing.
//!
//! These tests intentionally stay at the table level: they catch accidental
//! regressions where a user hangup/cancel becomes a local terminal event before
//! SIP teardown is terminal on the wire.

use rvoip_sip::state_table::{
    Action, EventTemplate, EventType, Role, StateKey, StateTable, Transition, YamlTableLoader,
};
use rvoip_sip::types::CallState;

fn load() -> StateTable {
    YamlTableLoader::load_embedded_default().expect("embedded default should load")
}

fn transition(table: &StateTable, role: Role, state: CallState, event: EventType) -> &Transition {
    table
        .get(&StateKey { role, state, event })
        .unwrap_or_else(|| panic!("missing transition for {role:?}/{state:?}"))
}

fn assert_no_terminal_publish(t: &Transition) {
    assert!(
        !t.publish_events.iter().any(|event| matches!(
            event,
            EventTemplate::CallCancelled
                | EventTemplate::CallTerminated
                | EventTemplate::CallFailed
        )),
        "transition must not publish a terminal event: {:?}",
        t.publish_events
    );
}

#[test]
fn uac_initiating_user_teardown_records_cancel_intent_without_cancel() {
    let table = load();

    for event in [EventType::HangupCall, EventType::CancelCall] {
        let t = transition(&table, Role::UAC, CallState::Initiating, event);
        assert_eq!(t.next_state, Some(CallState::CancelPending));
        assert!(
            !t.actions.contains(&Action::SendCANCEL),
            "CANCEL is not legal before a provisional response"
        );
        assert_no_terminal_publish(t);
    }
}

#[test]
fn uac_cancel_pending_provisional_sends_cancel_without_terminal_event() {
    let table = load();

    for event in [
        EventType::Dialog180Ringing,
        EventType::Dialog183SessionProgress,
    ] {
        let t = transition(&table, Role::UAC, CallState::CancelPending, event);
        assert_eq!(t.next_state, Some(CallState::Cancelling));
        assert!(t.actions.contains(&Action::SendCANCEL));
        assert_no_terminal_publish(t);
    }
}

#[test]
fn uac_ringing_and_early_teardown_send_cancel_without_immediate_cancelled_event() {
    let table = load();

    for state in [CallState::Ringing, CallState::EarlyMedia] {
        for event in [EventType::HangupCall, EventType::CancelCall] {
            let t = transition(&table, Role::UAC, state, event);
            assert_eq!(t.next_state, Some(CallState::Cancelling));
            assert!(t.actions.contains(&Action::SendCANCEL));
            assert_no_terminal_publish(t);
        }
    }
}

#[test]
fn uac_cancelling_487_is_the_cancelled_terminal_point() {
    let table = load();
    let t = transition(
        &table,
        Role::UAC,
        CallState::Cancelling,
        EventType::Dialog487RequestTerminated,
    );

    assert_eq!(t.next_state, Some(CallState::Cancelled));
    assert!(t.actions.contains(&Action::CleanupDialog));
    assert!(t.publish_events.contains(&EventTemplate::CallCancelled));
}

#[test]
fn uac_late_200_after_cancel_acks_and_byes_without_established_event() {
    let table = load();

    for state in [CallState::CancelPending, CallState::Cancelling] {
        let t = transition(&table, Role::UAC, state, EventType::Dialog200OK);
        assert_eq!(t.next_state, Some(CallState::Cancelling));

        let ack_idx = t
            .actions
            .iter()
            .position(|action| matches!(action, Action::SendACK))
            .expect("late 200 OK must be ACKed");
        let bye_idx = t
            .actions
            .iter()
            .position(|action| matches!(action, Action::SendBYE))
            .expect("late 200 OK after cancel must be followed by BYE");

        assert!(
            ack_idx < bye_idx,
            "ACK must be completed before BYE is attempted"
        );
        assert_no_terminal_publish(t);
        assert!(
            !t.publish_events
                .iter()
                .any(|event| matches!(event, EventTemplate::CallEstablished)),
            "late 200 OK after cancel must not publish CallEstablished"
        );
    }
}

#[test]
fn uac_late_answer_bye_cleanup_publishes_cancelled_on_dialog_terminated() {
    let table = load();
    let t = transition(
        &table,
        Role::UAC,
        CallState::Cancelling,
        EventType::DialogTerminated,
    );

    assert_eq!(t.next_state, Some(CallState::Cancelled));
    assert!(t.publish_events.contains(&EventTemplate::CallCancelled));
}

#[test]
fn initiating_timeout_does_not_send_cancel() {
    let table = load();
    let t = transition(
        &table,
        Role::UAC,
        CallState::Initiating,
        EventType::DialogTimeout,
    );

    assert_eq!(
        t.next_state,
        Some(CallState::Failed(
            rvoip_sip::types::FailureReason::Other
        ))
    );
    assert!(
        !t.actions.contains(&Action::SendCANCEL),
        "Timer B before a provisional response must not send CANCEL"
    );
}

#[test]
fn uas_answering_hangup_waits_for_ack_before_bye() {
    let table = load();
    let t = transition(
        &table,
        Role::UAS,
        CallState::Answering,
        EventType::HangupCall,
    );

    assert_eq!(t.next_state, Some(CallState::AnsweringHangupPending));
    assert!(
        !t.actions.contains(&Action::SendBYE),
        "BYE is not legal until the UAC ACKs the 200 OK"
    );
    assert_no_terminal_publish(t);
}

#[test]
fn uas_answering_hangup_pending_ack_sends_bye_without_established_event() {
    let table = load();
    let t = transition(
        &table,
        Role::UAS,
        CallState::AnsweringHangupPending,
        EventType::DialogACK,
    );

    assert_eq!(t.next_state, Some(CallState::Terminating));
    assert!(t.actions.contains(&Action::SendBYE));
    assert!(
        !t.publish_events
            .iter()
            .any(|event| matches!(event, EventTemplate::CallEstablished)),
        "pending hangup ACK path must not publish CallEstablished"
    );
}

#[test]
fn uas_answering_ack_timeout_fails_without_bye() {
    let table = load();

    for state in [CallState::Answering, CallState::AnsweringHangupPending] {
        let t = transition(&table, Role::UAS, state, EventType::DialogTimeout);
        assert_eq!(
            t.next_state,
            Some(CallState::Failed(
                rvoip_sip::types::FailureReason::Other
            ))
        );
        assert!(!t.actions.contains(&Action::SendBYE));
        assert!(t.publish_events.contains(&EventTemplate::CallFailed));
    }
}
