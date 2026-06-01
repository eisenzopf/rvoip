use std::collections::HashSet;

use rvoip_sip::state_table::wiring_manifest::{render_wiring_markdown, WiringKind, EVENT_WIRINGS};
use rvoip_sip::state_table::yaml_loader::{YamlAction, YamlEvent, YamlStateTable};
use rvoip_sip::state_table::YamlTableLoader;

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

    assert!(message_builder.contains("send_message_oob_with_options"));
    assert!(!message_builder.contains("SendOutboundMessage"));
    assert!(options_builder.contains("send_options_oob_with_options"));
    assert!(!options_builder.contains("SendOutboundOptions"));
    assert!(subscribe_builder.contains("send_subscribe_oob_with_options"));
    assert!(!subscribe_builder.contains("SendOutboundSubscribe"));
    assert!(!subscribe_builder.contains("stage_outbound_options"));
    assert!(unified.contains("bridge_rtp_sessions"));
    assert!(bridge.contains("bridge("));
}

#[test]
fn infra_common_and_dialog_preserve_method_specific_bye() {
    let infra = include_str!("../../infra-common/src/events/cross_crate.rs");
    let bye_handler = include_str!("../sip-dialog/src/protocol/bye_handler.rs");
    let event_hub = include_str!("../sip-dialog/src/events/event_hub.rs");
    let session_handler = include_str!("../src/adapters/session_event_handler.rs");

    assert!(infra.contains("ByeReceived"));
    assert!(bye_handler.contains("SessionCoordinationEvent::ByeReceived"));
    assert!(event_hub.contains("DialogToSessionEvent::ByeReceived"));
    assert!(session_handler.contains("EventType::DialogBYE"));
    assert!(session_handler
        .contains("CallState::Terminated => {\n                Some(EventType::DialogTerminated)"));
}

#[test]
fn response_fanout_is_guarded_by_cseq_method() {
    let event_hub = include_str!("../sip-dialog/src/events/event_hub.rs");

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
        "Unknown YAML event",
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
        "Unknown YAML action",
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
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains(expected),
        "expected error containing '{expected}', got: {error}"
    );
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
