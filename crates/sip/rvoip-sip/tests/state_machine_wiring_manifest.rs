use std::collections::HashSet;

use rvoip_sip::state_table::wiring_manifest::{render_wiring_markdown, WiringKind, EVENT_WIRINGS};
use rvoip_sip::state_table::yaml_loader::{YamlAction, YamlEvent, YamlStateTable};
use rvoip_sip::state_table::YamlTableLoader;
use rvoip_sip::SessionError;

const DEFAULT_YAML: &str = include_str!("../state_tables/default.yaml");

#[test]
fn wiring_manifest_markdown_is_current() {
    let expected = include_str!("../docs/state-machine-wiring.md");
    assert_eq!(render_wiring_markdown(), expected);
}

#[test]
fn state_table_manifest_rows_resolve_to_yaml_events_and_actions() {
    let yaml: YamlStateTable = serde_yaml::from_str(DEFAULT_YAML).unwrap();
    let yaml_events: HashSet<String> = yaml
        .transitions
        .iter()
        .map(|transition| event_name(&transition.event))
        .collect();
    let yaml_actions: HashSet<String> = yaml
        .transitions
        .iter()
        .flat_map(|transition| transition.actions.iter().map(action_name))
        .collect();

    for row in EVENT_WIRINGS
        .iter()
        .filter(|row| row.kind == WiringKind::StateTable)
    {
        for event in row.yaml_event.split(" / ").filter(|event| *event != "n/a") {
            assert!(
                yaml_events.contains(event),
                "manifest row '{}' references missing YAML event '{}'",
                row.sip_message,
                event
            );
        }

        for action in row
            .actions
            .iter()
            .filter(|action| !action.contains("::") && **action != "n/a")
        {
            assert!(
                yaml_actions.contains(*action),
                "manifest row '{}' references missing YAML action '{}'",
                row.sip_message,
                action
            );
        }
    }
}

#[test]
fn every_yaml_event_is_manifested_or_marked_internal() {
    let yaml: YamlStateTable = serde_yaml::from_str(DEFAULT_YAML).unwrap();
    let manifest_events: HashSet<&str> = EVENT_WIRINGS
        .iter()
        .filter(|row| row.kind == WiringKind::StateTable)
        .flat_map(|row| row.yaml_event.split(" / "))
        .filter(|event| *event != "n/a")
        .collect();
    let internal_events: HashSet<&str> = [
        "AcceptCall",
        "AuthRequired",
        "CancelCall",
        "Dialog180Ringing",
        "Dialog183SessionProgress",
        "Dialog200OK",
        "Dialog3xxRedirect",
        "Dialog4xxFailure",
        "Dialog5xxFailure",
        "Dialog6xxFailure",
        "DialogACK",
        "DialogTerminated",
        "DialogTimeout",
        "HoldCall",
        "IncomingCall",
        "IncomingCallAutoAccept",
        "InternalSessionRefreshPeerExpired",
        "InternalSessionRefreshReinviteDue",
        "InternalSessionRefreshReinviteFailed",
        "InternalSessionRefreshReinviteSucceeded",
        "InternalSessionRefreshUpdateDue",
        "InternalSessionRefreshUpdateFailed",
        "InternalSessionRefreshUpdateSucceeded",
        "RedirectCall",
        "Registration200OK",
        "RegistrationFailed",
        "ReinviteGlare",
        "ReinviteReceived",
        "RejectCall",
        "ResumeCall",
        "SendEarlyMedia",
        "SessionIntervalTooSmall",
        "StartUnregistration",
        "TransferRequested",
        "Unregistration200OK",
        "UnregistrationFailed",
        "UpdateReceived",
    ]
    .into_iter()
    .collect();

    for transition in &yaml.transitions {
        let event = event_name(&transition.event);
        assert!(
            manifest_events.contains(event.as_str()) || internal_events.contains(event.as_str()),
            "YAML event '{}' is neither in the wiring manifest nor marked internal",
            event
        );
    }
}

#[test]
fn default_state_declarations_and_references_are_bidirectional() {
    let yaml: YamlStateTable = serde_yaml::from_str(DEFAULT_YAML).unwrap();
    let declared = yaml
        .states
        .iter()
        .map(|state| state.name.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        declared.len(),
        yaml.states.len(),
        "default.yaml contains duplicate state declarations"
    );
    let referenced = yaml
        .transitions
        .iter()
        .flat_map(|transition| {
            std::iter::once(transition.state.as_str()).chain(transition.next_state.as_deref())
        })
        .collect::<HashSet<_>>();

    for state in &declared {
        assert!(
            referenced.contains(state),
            "default.yaml declares unreachable state '{}'",
            state
        );
    }

    let undeclared_allowances = [(
        "Any",
        "wildcard: transition source selector, not a CallState declaration",
    )];
    for (state, owner) in undeclared_allowances {
        assert!(owner.contains(':'), "state allowance needs an owner");
        assert!(
            referenced.contains(state),
            "stale undeclared-state allowance for '{state}': {owner}"
        );
        assert!(
            !declared.contains(state),
            "state '{state}' is now declared; remove its allowance ({owner})"
        );
    }

    for state in referenced.difference(&declared) {
        assert!(
            undeclared_allowances
                .iter()
                .any(|(allowed, _)| allowed == state),
            "default.yaml references undeclared state '{state}'"
        );
    }
}

#[test]
fn default_condition_declarations_and_writers_are_bidirectional() {
    let yaml: YamlStateTable = serde_yaml::from_str(DEFAULT_YAML).unwrap();
    let declared = yaml
        .conditions
        .iter()
        .map(|condition| condition.name.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        declared.len(),
        yaml.conditions.len(),
        "default.yaml contains duplicate condition declarations"
    );

    let supported = [
        ("DialogEstablished", "dialog_established"),
        ("MediaSessionReady", "media_session_ready"),
        ("SDPNegotiated", "sdp_negotiated"),
    ];
    let supported_declarations = supported
        .iter()
        .map(|(declaration, _)| *declaration)
        .collect::<HashSet<_>>();
    assert_eq!(
        declared, supported_declarations,
        "default condition declarations drifted from the typed condition-update schema"
    );

    let mut written = HashSet::new();
    for transition in &yaml.transitions {
        if transition.conditions.dialog_established.is_some() {
            written.insert("dialog_established");
        }
        if transition.conditions.media_session_ready.is_some() {
            written.insert("media_session_ready");
        }
        if transition.conditions.sdp_negotiated.is_some() {
            written.insert("sdp_negotiated");
        }
    }

    let expected_writers = supported
        .iter()
        .map(|(_, field)| *field)
        .collect::<HashSet<_>>();
    assert_eq!(
        written, expected_writers,
        "default condition writers must exactly cover every declared typed condition"
    );
}

#[test]
fn direct_wired_messages_have_no_dead_yaml_rows() {
    for forbidden in [
        "SendOutboundMessage",
        "SendOutboundOptions",
        "SendOutboundSubscribe",
        "BridgeSessions",
        "CreateMediaBridge",
        "StartPublish",
        "SendPUBLISH",
        "SendMessage",
        "ReceiveMESSAGE",
    ] {
        assert!(
            !DEFAULT_YAML.contains(forbidden),
            "default.yaml still contains dead/direct-wired row '{}'",
            forbidden
        );
    }
}

#[test]
fn direct_wired_source_paths_match_manifest() {
    let message_builder = include_str!("../src/api/send/message.rs");
    let options_builder = include_str!("../src/api/send/options.rs");
    let subscribe_builder = include_str!("../src/api/send/subscribe.rs");
    let unified = include_str!("../src/api/unified.rs");
    let bridge = include_str!("../src/server/bridge.rs");

    assert!(message_builder.contains("send_message_oob_with_optional_auth"));
    assert!(!message_builder.contains("SendOutboundMessage"));
    assert!(options_builder.contains("send_options_oob_with_optional_auth"));
    assert!(!options_builder.contains("SendOutboundOptions"));
    assert!(subscribe_builder.contains("send_subscribe_oob_with_optional_auth"));
    assert!(!subscribe_builder.contains("SendOutboundSubscribe"));
    assert!(!subscribe_builder.contains("stage_outbound_options"));
    assert!(unified.contains("bridge_rtp_sessions"));
    assert!(bridge.contains("bridge("));
}

#[test]
fn standalone_requests_have_one_wire_and_auth_implementation() {
    let adapter = include_str!("../src/adapters/dialog_adapter.rs");
    let coordinator = include_str!("../src/api/unified.rs");

    assert!(adapter.contains("async fn send_standalone_request("));
    assert_eq!(
        adapter
            .matches(".send_message_out_of_dialog_with_options(")
            .count(),
        1,
        "MESSAGE regained a second standalone wire path"
    );
    assert_eq!(
        adapter
            .matches(".send_options_out_of_dialog_with_options(")
            .count(),
        1,
        "OPTIONS regained a second standalone wire path"
    );
    assert_eq!(
        adapter.matches(".send_subscribe_with_options(").count(),
        1,
        "SUBSCRIBE regained a second standalone wire path"
    );
    assert!(coordinator.contains("async fn send_standalone_oob_with_optional_auth("));
    assert!(coordinator.contains("StandaloneRequestOptions::Message(opts)"));
    assert!(coordinator.contains("StandaloneRequestOptions::Options(opts)"));
    assert!(coordinator.contains("StandaloneRequestOptions::Subscribe"));
}

#[test]
fn session_coordination_ingress_remains_typed() {
    let session_handler = include_str!("../src/adapters/session_event_handler.rs");

    assert!(session_handler.contains("downcast_ref::<RvoipCrossCrateEvent>()"));
    assert!(
        session_handler.contains("handle_dialog_to_session_event(typed, exact_handle.as_ref())")
    );
    assert!(session_handler.contains("capture_dialog_ingress_handle("));

    for forbidden in [
        "event_str: &str",
        "extract_session_id(",
        "extract_field(",
        "extract_debug_string_field(",
        "extract_optional_field(",
    ] {
        assert!(
            !session_handler.contains(forbidden),
            "debug-string session event routing returned: {forbidden}"
        );
    }
}

#[test]
fn infra_common_and_dialog_preserve_method_specific_bye() {
    let infra = include_str!("../../../foundation/infra-common/src/events/cross_crate.rs");
    let bye_handler = include_str!("../../sip-dialog/src/protocol/bye_handler.rs");
    let event_hub = include_str!("../../sip-dialog/src/events/event_hub.rs");
    let session_handler = include_str!("../src/adapters/session_event_handler.rs");

    assert!(infra.contains("ByeReceived"));
    assert!(bye_handler.contains("SessionCoordinationEvent::ByeReceived"));
    assert!(event_hub.contains("DialogToSessionEvent::ByeReceived"));
    assert!(session_handler.contains("EventType::DialogBYE"));
    assert!(session_handler
        .contains("Ignoring observational terminated state; awaiting typed CallTerminated"));
    assert!(!session_handler
        .contains("CallState::Terminated => {\n                Some(EventType::DialogTerminated)"));
}

#[test]
fn response_fanout_is_guarded_by_cseq_method() {
    let event_hub = include_str!("../../sip-dialog/src/events/event_hub.rs");

    assert!(event_hub.contains("response.cseq()"));
    assert!(event_hub.contains("is_invite_response"));
    assert!(event_hub.contains("200 if is_invite_response"));
    assert!(event_hub.contains("100..=199 if is_invite_response"));
    assert!(event_hub.contains("is_invite_response && (400..700).contains(&code)"));
}

#[test]
fn raw_yaml_validator_rejects_duplicate_transition_keys() {
    assert_yaml_fails(
        r#"
version: "2.0"
states:
  - name: "Idle"
  - name: "Initiating"
transitions:
  - role: "UAC"
    state: "Idle"
    event:
      type: "MakeCall"
    next_state: "Initiating"
    actions:
      - type: "SendINVITE"
  - role: "UAC"
    state: "Idle"
    event:
      type: "MakeCall"
    next_state: "Initiating"
    actions:
      - type: "SendINVITE"
"#,
        "duplicates transition",
    );
}

#[test]
fn raw_yaml_validator_rejects_unknown_events_actions_conditions_and_states() {
    assert_yaml_fails(
        r#"
version: "2.0"
states:
  - name: "Idle"
transitions:
  - role: "UAC"
    state: "Idle"
    event:
      type: "NotARealEvent"
"#,
        "has invalid event",
    );

    assert_yaml_fails(
        r#"
version: "2.0"
states:
  - name: "Idle"
transitions:
  - role: "UAC"
    state: "Idle"
    event:
      type: "MakeCall"
    actions:
      - type: "NotARealAction"
"#,
        "has invalid action",
    );

    assert_yaml_fails(
        r#"
version: "2.0"
states:
  - name: "Idle"
  - name: "Initiating"
transitions:
  - role: "UAC"
    state: "Idle"
    event:
      type: "MakeCall"
    next_state: "Initiating"
    conditions:
      is_registered: true
"#,
        "unsupported condition update",
    );

    assert_yaml_fails(
        r#"
version: "2.0"
states:
  - name: "Idle"
transitions:
  - role: "UAC"
    state: "Idle"
    event:
      type: "MakeCall"
    next_state: "MissingState"
"#,
        "undeclared next_state",
    );
}

fn assert_yaml_fails(yaml: &str, expected: &str) {
    let mut loader = YamlTableLoader::new();
    let result = loader.load_from_string(yaml).and_then(|_| loader.build());
    let error = match result {
        Ok(_) => panic!("fixture should fail validation"),
        Err(error) => error,
    };
    let SessionError::InternalError(detail) = &error else {
        panic!("expected typed InternalError from YAML validation, got {error:?}");
    };
    assert!(
        detail.contains(expected),
        "expected internal error detail containing '{expected}', got: {detail}"
    );

    let rendered = error.to_string();
    assert!(
        !rendered.contains(detail),
        "SessionError Display must not expose YAML validation details"
    );
    assert!(rendered.contains("redacted"));
}

fn event_name(event: &YamlEvent) -> String {
    match event {
        YamlEvent::Simple(name) => name.clone(),
        YamlEvent::Complex { event_type, .. } => event_type.clone(),
    }
}

fn action_name(action: &YamlAction) -> String {
    match action {
        YamlAction::Simple(name) => name.clone(),
        YamlAction::Complex { action_type, .. } => action_type.clone(),
    }
}
