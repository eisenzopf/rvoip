//! State-machine tests per `UCTP_IMPLEMENTATION_PLAN.md` §3.8.

use rvoip_uctp::ids::StreamId;
use rvoip_uctp::state::connection::AcceptedStream;
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
fn externally_allocated_stream_binding_round_trip() {
    let mut m = ConnectionMachine::new_negotiating();
    m.apply(ConnectionInput::AnswerReceived).unwrap();
    let s1 = StreamId::new();
    let s2 = StreamId::new();
    m.bind_stream(41, s1.clone()).unwrap();
    m.bind_stream(42, s2.clone()).unwrap();
    assert_eq!(m.resolve_stream(41), Some(&s1));
    assert_eq!(m.resolve_stream(42), Some(&s2));
    assert_eq!(m.resolve_stream(999), None);
    assert!(m.bind_stream(0, StreamId::new()).is_err());
    assert!(m.bind_stream(41, StreamId::new()).is_err());
}

#[test]
fn pending_unannounced_streams_are_not_teardown_cleanup_candidates() {
    let mut machine = ConnectionMachine::new_negotiating();
    machine.set_pending_streams(vec![AcceptedStream {
        strm_id: "audio/main".into(),
        kind: "audio".into(),
        direction: "sendrecv".into(),
        chosen_codec: Some("opus".into()),
        participant: "publisher".into(),
    }]);
    assert!(machine.stream_ids().is_empty());

    let pending = machine.take_pending_streams();
    machine
        .bind_stream(77, StreamId::from_string(pending[0].strm_id.clone()))
        .unwrap();
    assert_eq!(machine.stream_ids(), vec!["audio/main".to_string()]);
}

#[test]
fn illegal_transitions_return_error() {
    // Inviting → ReadyReceived is illegal at the Session layer (use Connection input instead).
    let mut m = SessionMachine::new_inviting();
    assert!(m.apply(SessionInput::LastConnectionEnded).is_err());

    let mut c = ConnectionMachine::new_negotiating();
    assert!(c.apply(ConnectionInput::HoldRequested).is_err());
}

#[test]
fn connection_machine_carries_lifetime_span() {
    // C5: the ConnectionMachine stores a per-Connection lifetime span
    // that the coordinator builds at offer time and re-enters at every
    // subsequent envelope. The default constructor produces a no-op
    // span (`Span::none()`); the spanned constructor stores whatever
    // the caller passes, retrievable via `lifetime_span()`.
    //
    // We compare the returned span's `id()` to the original to prove
    // the same span is roundtripped — span IDs are subscriber-assigned
    // and only present when a subscriber is installed, so we install
    // a fmt subscriber locally with `with_default`.
    use tracing::subscriber::with_default;
    use tracing_subscriber::fmt;
    with_default(fmt::Subscriber::builder().finish(), || {
        let m = ConnectionMachine::new_negotiating();
        assert!(
            m.lifetime_span().is_none(),
            "default constructor must produce Span::none() (no id assigned)"
        );

        let real_span = tracing::info_span!("uctp.connection.lifetime", connid = "conn_x");
        let original_id = real_span.id();
        let m_with = ConnectionMachine::new_negotiating_with_span(real_span);
        let returned = m_with.lifetime_span();
        assert_eq!(
            returned.id(),
            original_id,
            "spanned constructor must propagate the same span (matching id)"
        );
    });
}
