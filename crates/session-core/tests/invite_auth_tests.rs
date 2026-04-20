//! Unit tests for RFC 3261 §22.2 digest auth retry wiring.
//!
//! Covers the state-table transitions added alongside `StreamPeerBuilder::
//! with_credentials` / `PeerControl::call_with_auth`. End-to-end wire
//! behaviour is exercised by `tests/invite_auth_integration.rs` when a
//! challenging UAS fixture is available.

use rvoip_session_core::state_table::{
    Action, EventType, Role, StateKey, StateTable, YamlTableLoader,
};
use rvoip_session_core::types::CallState;

fn load() -> StateTable {
    YamlTableLoader::load_embedded_default().expect("embedded default should load")
}

fn key(role: Role, state: CallState, event: EventType) -> StateKey {
    StateKey { role, state, event }
}

fn auth_event() -> EventType {
    EventType::AuthRequired {
        status_code: 401,
        challenge: String::new(),
    }
}

#[test]
fn initiating_auth_required_drives_invite_retry() {
    let table = load();
    let t = table
        .get(&key(Role::UAC, CallState::Initiating, auth_event()))
        .expect("UAC Initiating + AuthRequired transition must exist");

    assert_eq!(t.next_state, Some(CallState::Initiating));
    assert!(
        t.actions.contains(&Action::StoreAuthChallenge),
        "transition must parse the challenge before retrying"
    );
    assert!(
        t.actions.contains(&Action::SendINVITEWithAuth),
        "transition must issue the authenticated INVITE retry"
    );
}

#[test]
fn registering_auth_required_drives_register_retry() {
    let table = load();
    let t = table
        .get(&key(Role::UAC, CallState::Registering, auth_event()))
        .expect("UAC Registering + AuthRequired transition must exist");

    assert_eq!(t.next_state, Some(CallState::Registering));
    assert!(t.actions.contains(&Action::StoreAuthChallenge));
    assert!(
        t.actions.contains(&Action::SendREGISTERWithAuth),
        "REGISTER auth retry goes through SendREGISTERWithAuth (not INVITE variant)"
    );
}

#[test]
fn auth_required_normalizes_for_lookup() {
    // The state table is keyed on normalized EventType — both a populated
    // payload (as would arrive from dialog-core with a real challenge) and
    // the default-valued form used by YAML parsing must resolve to the
    // same transition entry.
    let table = load();
    let populated = table.get(&key(
        Role::UAC,
        CallState::Initiating,
        EventType::AuthRequired {
            status_code: 407,
            challenge: "Digest realm=\"rvoip-test\", nonce=\"abcd\"".to_string(),
        },
    ));
    let empty = table.get(&key(Role::UAC, CallState::Initiating, auth_event()));

    assert!(
        populated.is_some() && empty.is_some(),
        "payload-carrying and payload-free AuthRequired must both resolve"
    );
    let a = populated.unwrap();
    let b = empty.unwrap();
    assert_eq!(a.next_state, b.next_state);
    assert_eq!(a.actions, b.actions);
}

#[test]
fn registration401_alias_resolves_to_auth_required() {
    // Backward-compat: external state tables that still reference the old
    // `Registration401` event name should silently map to AuthRequired so
    // the `Registering + AuthRequired` transition covers them too.
    use rvoip_session_core::state_table::YamlTableLoader;

    let yaml = r#"
version: "1.0"
transitions:
  - role: UAC
    state: Registering
    event: Registration401
    next_state: Registering
    actions:
      - StoreAuthChallenge
      - SendREGISTERWithAuth
"#;
    let mut loader = YamlTableLoader::new();
    loader
        .load_from_string(yaml)
        .expect("alias test table should parse");
    let table = loader.build().expect("alias test table should build");

    let k = key(Role::UAC, CallState::Registering, auth_event());
    assert!(
        table.get(&k).is_some(),
        "legacy Registration401 YAML entry should route through AuthRequired"
    );
}
