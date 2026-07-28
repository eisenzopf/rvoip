//! External-crate compile coverage for the ordinary `SessionState` API.
//!
//! Cold fields are reached through `Deref`, which preserves constructor and
//! normal field read/write syntax. Public pattern destructuring of a cold
//! field is the deliberate source-compatibility caveat of the storage split.

use rvoip_sip::session_store::{SessionState, TransferState};
use rvoip_sip::state_table::{Role, SessionId};
use rvoip_sip::types::CallState;

#[test]
fn constructor_and_hot_and_cold_field_syntax_remain_available() {
    println!(
        "SessionState inline bytes: {}",
        std::mem::size_of::<SessionState>()
    );
    let mut state = SessionState::new(SessionId::from_string("api-compat"), Role::UAC);

    state.call_state = CallState::Ringing;
    state.local_sdp = Some("v=0\r\n".into());
    state.transfer_target = Some("sip:agent@example.test".into());
    state.registration_expires = Some(300);
    state.transfer_state = TransferState::TransferInitiated;

    assert_eq!(state.call_state, CallState::Ringing);
    assert_eq!(state.local_sdp.as_deref(), Some("v=0\r\n"));
    assert_eq!(
        state.transfer_target.as_deref(),
        Some("sip:agent@example.test")
    );
    assert_eq!(state.registration_expires, Some(300));
    assert_eq!(state.transfer_state, TransferState::TransferInitiated);
}
