//! State-machine tests per `UCTP_IMPLEMENTATION_PLAN.md` §3.8.

use rvoip_uctp::ids::StreamId;
use rvoip_uctp::state::{
    ConnectionInput, ConnectionMachine, SessionInput, SessionMachine, UctpConnectionState,
    UctpSessionState,
};

#[test]
fn session_invite_accept_roundtrip() {
    let mut m = SessionMachine::new_inviting();
    assert_eq!(m.state(), UctpSessionState::Inviting);
    let next = m.apply(SessionInput::AcceptReceived).unwrap();
    assert_eq!(next, UctpSessionState::Active);
    assert_eq!(m.state(), UctpSessionState::Active);
}

#[test]
fn session_cancel_during_inviting_ends_session() {
    let mut m = SessionMachine::new_inviting();
    let next = m.apply(SessionInput::CancelReceived).unwrap();
    assert_eq!(next, UctpSessionState::Ended);
}

#[test]
fn session_end_with_two_connections_requires_both_to_end() {
    let mut m = SessionMachine::new_inviting();
    m.apply(SessionInput::AcceptReceived).unwrap();
    assert_eq!(m.state(), UctpSessionState::Active);

    // First connection.end → Session moves to Ending.
    m.apply(SessionInput::EndReceived).unwrap();
    assert_eq!(m.state(), UctpSessionState::Ending);

    // A second connection.end (idempotent during Ending) keeps Ending.
    m.apply(SessionInput::EndReceived).unwrap();
    assert_eq!(m.state(), UctpSessionState::Ending);

    // Last connection.end → LastConnectionEnded transitions to Ended.
    m.apply(SessionInput::LastConnectionEnded).unwrap();
    assert_eq!(m.state(), UctpSessionState::Ended);
}

#[test]
fn connection_negotiate_happy_path() {
    let mut m = ConnectionMachine::new_negotiating();
    assert_eq!(m.state(), UctpConnectionState::Negotiating);
    m.apply(ConnectionInput::AnswerReceived).unwrap();
    assert_eq!(m.state(), UctpConnectionState::Connected);
    m.apply(ConnectionInput::HoldRequested).unwrap();
    assert_eq!(m.state(), UctpConnectionState::OnHold);
    m.apply(ConnectionInput::ResumeRequested).unwrap();
    assert_eq!(m.state(), UctpConnectionState::Connected);
    m.apply(ConnectionInput::EndReceived).unwrap();
    assert_eq!(m.state(), UctpConnectionState::Ending);
    m.apply(ConnectionInput::EndReceived).unwrap();
    assert_eq!(m.state(), UctpConnectionState::Ended);
}

#[test]
fn stream_local_id_allocator_round_trip() {
    let mut m = ConnectionMachine::new_negotiating();
    m.apply(ConnectionInput::AnswerReceived).unwrap();
    let s1 = StreamId::new();
    let s2 = StreamId::new();
    let l1 = m.open_stream(s1.clone()).unwrap();
    let l2 = m.open_stream(s2.clone()).unwrap();
    assert_ne!(l1, l2);
    assert_eq!(m.resolve_stream(l1), Some(&s1));
    assert_eq!(m.resolve_stream(l2), Some(&s2));
    assert_eq!(m.resolve_stream(999), None);
}

#[test]
fn illegal_transitions_return_error() {
    // Inviting → ReadyReceived is illegal at the Session layer (use Connection input instead).
    let mut m = SessionMachine::new_inviting();
    assert!(m.apply(SessionInput::LastConnectionEnded).is_err());

    let mut c = ConnectionMachine::new_negotiating();
    assert!(c.apply(ConnectionInput::HoldRequested).is_err());
}
