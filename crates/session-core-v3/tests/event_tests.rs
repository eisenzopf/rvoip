//! Event System Tests
//!
//! Tests Event enum construction, helper methods, and the call_id accessor.

use rvoip_session_core_v3::Event;
use rvoip_session_core_v3::state_table::types::SessionId;

fn test_id() -> SessionId {
    SessionId::new()
}

// ── Event construction ──────────────────────────────────────────────────────

#[test]
fn test_incoming_call_event() {
    let id = test_id();
    let e = Event::IncomingCall {
        call_id: id.clone(),
        from: "sip:alice@example.com".into(),
        to: "sip:bob@example.com".into(),
        sdp: Some("v=0\r\n".into()),
    };
    assert_eq!(e.call_id(), Some(&id));
    assert!(e.is_call_event());
    assert!(!e.is_transfer_event());
    assert!(!e.is_media_event());
}

#[test]
fn test_call_answered_event() {
    let id = test_id();
    let e = Event::CallAnswered {
        call_id: id.clone(),
        sdp: None,
    };
    assert_eq!(e.call_id(), Some(&id));
    assert!(e.is_call_event());
}

#[test]
fn test_call_ended_event() {
    let id = test_id();
    let e = Event::CallEnded {
        call_id: id.clone(),
        reason: "Normal".into(),
    };
    assert_eq!(e.call_id(), Some(&id));
    assert!(e.is_call_event());
}

#[test]
fn test_call_failed_event() {
    let id = test_id();
    let e = Event::CallFailed {
        call_id: id.clone(),
        status_code: 486,
        reason: "Busy Here".into(),
    };
    assert_eq!(e.call_id(), Some(&id));
    assert!(e.is_call_event());
}

// ── Transfer events ─────────────────────────────────────────────────────────

#[test]
fn test_refer_received_event() {
    let id = test_id();
    let e = Event::ReferReceived {
        call_id: id.clone(),
        refer_to: "sip:charlie@example.com".into(),
        referred_by: Some("sip:alice@example.com".into()),
        replaces: None,
        transaction_id: "tx-123".into(),
        transfer_type: "blind".into(),
    };
    assert_eq!(e.call_id(), Some(&id));
    assert!(e.is_transfer_event());
    assert!(!e.is_call_event());
}

#[test]
fn test_transfer_completed_event() {
    let old_id = test_id();
    let new_id = test_id();
    let e = Event::TransferCompleted {
        old_call_id: old_id.clone(),
        new_call_id: new_id,
        target: "sip:charlie@example.com".into(),
    };
    // call_id() returns old_call_id
    assert_eq!(e.call_id(), Some(&old_id));
    assert!(e.is_transfer_event());
}

#[test]
fn test_transfer_failed_event() {
    let id = test_id();
    let e = Event::TransferFailed {
        call_id: id.clone(),
        reason: "Declined".into(),
        status_code: 603,
    };
    assert!(e.is_transfer_event());
}

#[test]
fn test_transfer_progress_event() {
    let id = test_id();
    let e = Event::TransferProgress {
        call_id: id.clone(),
        status_code: 180,
        reason: "Ringing".into(),
    };
    assert!(e.is_transfer_event());
}

// ── Call state events ───────────────────────────────────────────────────────

#[test]
fn test_call_on_hold_event() {
    let id = test_id();
    let e = Event::CallOnHold { call_id: id.clone() };
    assert_eq!(e.call_id(), Some(&id));
    assert!(!e.is_call_event()); // hold is not a lifecycle event
}

#[test]
fn test_call_resumed_event() {
    let id = test_id();
    let e = Event::CallResumed { call_id: id.clone() };
    assert_eq!(e.call_id(), Some(&id));
}

#[test]
fn test_call_muted_unmuted_events() {
    let id = test_id();
    let muted = Event::CallMuted { call_id: id.clone() };
    let unmuted = Event::CallUnmuted { call_id: id.clone() };
    assert_eq!(muted.call_id(), Some(&id));
    assert_eq!(unmuted.call_id(), Some(&id));
}

// ── Media events ────────────────────────────────────────────────────────────

#[test]
fn test_dtmf_received_event() {
    let id = test_id();
    let e = Event::DtmfReceived {
        call_id: id.clone(),
        digit: '5',
    };
    assert_eq!(e.call_id(), Some(&id));
    assert!(e.is_media_event());
    assert!(!e.is_call_event());
}

#[test]
fn test_media_quality_changed_event() {
    let id = test_id();
    let e = Event::MediaQualityChanged {
        call_id: id.clone(),
        packet_loss_percent: 5,
        jitter_ms: 30,
    };
    assert!(e.is_media_event());
}

// ── Registration events ─────────────────────────────────────────────────────

#[test]
fn test_registration_success_has_no_call_id() {
    let e = Event::RegistrationSuccess {
        registrar: "sip:registrar.example.com".into(),
        expires: 3600,
        contact: "sip:alice@192.168.1.50:5060".into(),
    };
    assert_eq!(e.call_id(), None);
    assert!(!e.is_call_event());
    assert!(!e.is_transfer_event());
    assert!(!e.is_media_event());
}

#[test]
fn test_registration_failed_has_no_call_id() {
    let e = Event::RegistrationFailed {
        registrar: "sip:registrar.example.com".into(),
        status_code: 401,
        reason: "Unauthorized".into(),
    };
    assert_eq!(e.call_id(), None);
}

#[test]
fn test_unregistration_events_have_no_call_id() {
    let success = Event::UnregistrationSuccess {
        registrar: "sip:registrar.example.com".into(),
    };
    let failed = Event::UnregistrationFailed {
        registrar: "sip:registrar.example.com".into(),
        reason: "Timeout".into(),
    };
    assert_eq!(success.call_id(), None);
    assert_eq!(failed.call_id(), None);
}

// ── Error events ────────────────────────────────────────────────────────────

#[test]
fn test_network_error_with_call_id() {
    let id = test_id();
    let e = Event::NetworkError {
        call_id: Some(id.clone()),
        error: "Connection refused".into(),
    };
    assert_eq!(e.call_id(), Some(&id));
}

#[test]
fn test_network_error_without_call_id() {
    let e = Event::NetworkError {
        call_id: None,
        error: "Interface down".into(),
    };
    assert_eq!(e.call_id(), None);
}

#[test]
fn test_authentication_required_event() {
    let id = test_id();
    let e = Event::AuthenticationRequired {
        call_id: id.clone(),
        realm: "example.com".into(),
    };
    assert_eq!(e.call_id(), Some(&id));
}

// ── Debug formatting ────────────────────────────────────────────────────────

#[test]
fn test_event_debug_does_not_panic() {
    let id = test_id();
    let events: Vec<Event> = vec![
        Event::IncomingCall {
            call_id: id.clone(),
            from: "a".into(),
            to: "b".into(),
            sdp: None,
        },
        Event::CallEnded {
            call_id: id.clone(),
            reason: "BYE".into(),
        },
        Event::DtmfReceived {
            call_id: id.clone(),
            digit: '#',
        },
        Event::RegistrationSuccess {
            registrar: "r".into(),
            expires: 60,
            contact: "c".into(),
        },
    ];
    for e in &events {
        let _ = format!("{:?}", e); // should not panic
    }
}
