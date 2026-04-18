//! Unit tests for RFC 3262 early-media transitions added alongside the
//! public `send_early_media` API. These cover the state-table wiring only;
//! the end-to-end reliable-183 wire flow is verified by the multi-binary
//! `prack_integration` test.

use rvoip_session_core_v3::state_table::{
    YamlTableLoader, StateTable, StateKey, EventType, Role, Action,
};
use rvoip_session_core_v3::types::CallState;

fn load() -> StateTable {
    YamlTableLoader::load_embedded_default().expect("embedded default should load")
}

fn key(role: Role, state: CallState, event: EventType) -> StateKey {
    StateKey { role, state, event }
}

#[test]
fn ringing_send_early_media_transitions_to_early_media() {
    let table = load();
    let t = table
        .get(&key(
            Role::UAS,
            CallState::Ringing,
            EventType::SendEarlyMedia { sdp: None },
        ))
        .expect("UAS Ringing + SendEarlyMedia transition must exist");

    assert_eq!(t.next_state, Some(CallState::EarlyMedia));

    assert!(
        t.actions.contains(&Action::PrepareEarlyMediaSDP),
        "transition must prepare the SDP before sending"
    );
    assert!(
        t.actions
            .iter()
            .any(|a| matches!(a, Action::SendSIPResponse(183, _))),
        "transition must send a 183 Session Progress response"
    );
}

#[test]
fn early_media_send_early_media_self_loops() {
    // RFC 3262 allows multiple reliable provisionals per call (each with a
    // fresh RSeq). The self-loop here is what supports re-emission — e.g.
    // updating the announcement SDP mid-ring.
    let table = load();
    let t = table
        .get(&key(
            Role::UAS,
            CallState::EarlyMedia,
            EventType::SendEarlyMedia { sdp: None },
        ))
        .expect("UAS EarlyMedia + SendEarlyMedia (re-emit) transition must exist");

    assert_eq!(t.next_state, Some(CallState::EarlyMedia));
    assert!(t.actions.contains(&Action::PrepareEarlyMediaSDP));
    assert!(t
        .actions
        .iter()
        .any(|a| matches!(a, Action::SendSIPResponse(183, _))));
}

#[test]
fn early_media_accept_skips_renegotiation() {
    // This is the subtle invariant: once we've sent the 183 with a
    // negotiated SDP, the 200 OK *must reuse* that SDP. Re-running
    // NegotiateSDPAsUAS here would open a second media session and change
    // ports mid-call — a regression we want to catch at the table level.
    let table = load();
    let t = table
        .get(&key(
            Role::UAS,
            CallState::EarlyMedia,
            EventType::AcceptCall,
        ))
        .expect("UAS EarlyMedia + AcceptCall transition must exist");

    assert_eq!(t.next_state, Some(CallState::Answering));

    assert!(
        !t.actions.contains(&Action::NegotiateSDPAsUAS),
        "AcceptCall from EarlyMedia must NOT re-negotiate SDP (see plan)"
    );
    assert!(
        !t.actions.contains(&Action::GenerateLocalSDP),
        "AcceptCall from EarlyMedia must NOT regenerate local SDP"
    );
    assert!(
        t.actions
            .iter()
            .any(|a| matches!(a, Action::SendSIPResponse(200, _))),
        "AcceptCall from EarlyMedia must still send 200 OK"
    );
}

#[test]
fn send_early_media_normalizes_for_lookup() {
    // The state table is keyed on normalized EventType — passing an SDP
    // payload on the event must still resolve to the same transition.
    let table = load();
    let with_payload = table.get(&key(
        Role::UAS,
        CallState::Ringing,
        EventType::SendEarlyMedia {
            sdp: Some("v=0\r\n...".to_string()),
        },
    ));
    let without_payload = table.get(&key(
        Role::UAS,
        CallState::Ringing,
        EventType::SendEarlyMedia { sdp: None },
    ));

    assert!(
        with_payload.is_some() && without_payload.is_some(),
        "payload-carrying and payload-free SendEarlyMedia must both resolve"
    );
    // Both point at the same Transition data — the YAML only defines one.
    let a = with_payload.unwrap();
    let b = without_payload.unwrap();
    assert_eq!(a.next_state, b.next_state);
    assert_eq!(a.actions, b.actions);
}
