//! Checked map of SIP/API message wiring.
//!
//! This manifest intentionally documents both state-table-backed and
//! direct-wired paths. Tests render it to docs/state-machine-wiring.md and
//! assert that key source call sites still match the declared wiring kind.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiringKind {
    StateTable,
    Direct,
    DialogManaged,
    TransportOnly,
    Removed,
    Deferred,
    Internal,
}

impl WiringKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StateTable => "state-table",
            Self::Direct => "direct",
            Self::DialogManaged => "dialog-managed",
            Self::TransportOnly => "transport-only",
            Self::Removed => "removed",
            Self::Deferred => "deferred",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EventWiring {
    pub sip_message: &'static str,
    pub caller: &'static str,
    pub infra_event: &'static str,
    pub event_type: &'static str,
    pub yaml_event: &'static str,
    pub actions: &'static [&'static str],
    pub kind: WiringKind,
    pub notes: &'static str,
}

pub const EVENT_WIRINGS: &[EventWiring] = &[
    EventWiring {
        sip_message: "INVITE outbound",
        caller: "InviteBuilder::send / UnifiedCoordinator::invite",
        infra_event: "n/a",
        event_type: "SendOutboundInvite / MakeCall",
        yaml_event: "SendOutboundInvite / MakeCall",
        actions: &[
            "CreateDialog",
            "CreateMediaSession",
            "GenerateLocalSDP",
            "SendINVITEWithOptions",
        ],
        kind: WiringKind::StateTable,
        notes: "Initial INVITE owns session lifecycle and media setup.",
    },
    EventWiring {
        sip_message: "re-INVITE outbound",
        caller: "ReinviteBuilder::send",
        infra_event: "n/a",
        event_type: "SendOutboundReInvite",
        yaml_event: "SendOutboundReInvite",
        actions: &["SendReINVITEWithOptions"],
        kind: WiringKind::StateTable,
        notes: "In-dialog request, state remains Active.",
    },
    EventWiring {
        sip_message: "UPDATE outbound",
        caller: "UpdateBuilder::send",
        infra_event: "n/a",
        event_type: "SendOutboundUpdate",
        yaml_event: "SendOutboundUpdate",
        actions: &["SendUPDATEWithOptions"],
        kind: WiringKind::StateTable,
        notes: "In-dialog request, state remains Active.",
    },
    EventWiring {
        sip_message: "BYE outbound",
        caller: "ByeBuilder::send / SessionHandle::hangup",
        infra_event: "n/a",
        event_type: "SendOutboundBye / HangupCall",
        yaml_event: "SendOutboundBye / HangupCall",
        actions: &["SendBYEWithOptions", "SendBYE"],
        kind: WiringKind::StateTable,
        notes: "Local hangup sends BYE once the dialog is established; terminal confirmation or the retained exact-release fallback owns cleanup.",
    },
    EventWiring {
        sip_message: "BYE inbound",
        caller: "rvoip-sip-dialog bye_handler",
        infra_event: "DialogToSessionEvent::ByeReceived",
        event_type: "DialogBYE",
        yaml_event: "DialogBYE",
        actions: &["CleanupDialog", "CleanupMedia"],
        kind: WiringKind::StateTable,
        notes: "Dialog-core sends the 200 OK; state machine only cleans up.",
    },
    EventWiring {
        sip_message: "CANCEL outbound",
        caller: "CancelBuilder::send / pending hangup",
        infra_event: "n/a",
        event_type: "SendOutboundCancel / CancelCall",
        yaml_event: "SendOutboundCancel / CancelCall",
        actions: &["SendCANCELWithOptions"],
        kind: WiringKind::StateTable,
        notes: "Only legal after an INVITE provisional response.",
    },
    EventWiring {
        sip_message: "CANCEL inbound",
        caller: "rvoip-sip-dialog protocol_handlers",
        infra_event: "DialogToSessionEvent::CallCancelled",
        event_type: "Dialog487RequestTerminated / DialogCANCEL",
        yaml_event: "Dialog487RequestTerminated / DialogCANCEL",
        actions: &["CleanupDialog", "CleanupMedia"],
        kind: WiringKind::StateTable,
        notes: "Dialog-core owns wire responses; state machine publishes cancellation.",
    },
    EventWiring {
        sip_message: "REFER outbound",
        caller: "ReferBuilder::send",
        infra_event: "n/a",
        event_type: "SendOutboundRefer",
        yaml_event: "SendOutboundRefer",
        actions: &["SendREFERWithOptions"],
        kind: WiringKind::StateTable,
        notes: "In-dialog request, state remains Active.",
    },
    EventWiring {
        sip_message: "NOTIFY outbound",
        caller: "NotifyBuilder::send / transfer progress",
        infra_event: "n/a",
        event_type: "SendOutboundNotify",
        yaml_event: "SendOutboundNotify",
        actions: &["SendNOTIFYWithOptions"],
        kind: WiringKind::StateTable,
        notes: "In-dialog NOTIFY and REFER progress NOTIFYs.",
    },
    EventWiring {
        sip_message: "INFO outbound",
        caller: "InfoBuilder::send",
        infra_event: "n/a",
        event_type: "SendOutboundInfo",
        yaml_event: "SendOutboundInfo",
        actions: &["SendINFOWithOptions"],
        kind: WiringKind::StateTable,
        notes: "In-dialog INFO request.",
    },
    EventWiring {
        sip_message: "REGISTER",
        caller: "RegisterBuilder::send / RegisterRefreshBuilder::send",
        infra_event: "DialogToSessionEvent::AuthRequired",
        event_type: "StartRegistration / SendOutboundRegister / RefreshRegistration",
        yaml_event: "StartRegistration / SendOutboundRegister / RefreshRegistration",
        actions: &[
            "SendREGISTER",
            "SendREGISTERWithOptions",
            "SendREGISTERWithAuth",
        ],
        kind: WiringKind::StateTable,
        notes: "Registration owns lifecycle, auth, 423 retry, refresh, and unregister state.",
    },
    EventWiring {
        sip_message: "MESSAGE out-of-dialog",
        caller: "MessageBuilder::send",
        infra_event: "n/a",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["UnifiedCoordinator::send_message_oob_with_optional_auth"],
        kind: WiringKind::Direct,
        notes: "Public OOB MESSAGE bypasses the state table by design; with_credentials is Digest shorthand and with_auth uses direct UAC auth retry.",
    },
    EventWiring {
        sip_message: "MESSAGE in-dialog inbound",
        caller: "rvoip-sip-dialog event_hub",
        infra_event: "DialogToSessionEvent::MessageReceived",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["publish Event::MessageReceived"],
        kind: WiringKind::Direct,
        notes: "Delivered as an application event; no session lifecycle transition.",
    },
    EventWiring {
        sip_message: "OPTIONS out-of-dialog",
        caller: "OptionsBuilder::send",
        infra_event: "n/a",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["UnifiedCoordinator::send_options_oob_with_optional_auth"],
        kind: WiringKind::Direct,
        notes: "Public OOB OPTIONS bypasses the state table by design; with_credentials is Digest shorthand and with_auth uses direct UAC auth retry.",
    },
    EventWiring {
        sip_message: "OPTIONS inbound",
        caller: "rvoip-sip-dialog event_hub",
        infra_event: "DialogToSessionEvent::OptionsReceived",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["publish Event::OptionsReceived"],
        kind: WiringKind::Direct,
        notes: "Capability query event; in-dialog mapping is preserved when dialog-core supplies a session id.",
    },
    EventWiring {
        sip_message: "SUBSCRIBE",
        caller: "SubscribeBuilder::send / SubscribeRefreshBuilder::send",
        infra_event: "DialogToSessionEvent::NotifyReceived",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["UnifiedCoordinator::send_subscribe_oob_with_optional_auth"],
        kind: WiringKind::DialogManaged,
        notes: "Subscription sends are direct-wired with optional UAC auth retry; NOTIFY delivery is handled by dialog-core/session events.",
    },
    EventWiring {
        sip_message: "PUBLISH",
        caller: "rvoip-sip-dialog presence::PublishBuilder",
        infra_event: "n/a",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["deferred"],
        kind: WiringKind::Deferred,
        notes: "Presence publish still has a placeholder dialog-core path and no live rvoip-sip state-table row.",
    },
    EventWiring {
        sip_message: "Bridge",
        caller: "UnifiedCoordinator::bridge / SipBridgeStrategy",
        infra_event: "n/a",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["MediaAdapter::bridge_rtp_sessions"],
        kind: WiringKind::Direct,
        notes: "Bridge is media/core direct wiring, not a SIP state-table transition.",
    },
    EventWiring {
        sip_message: "Transport inbound message",
        caller: "rvoip-sip-transport live transports",
        infra_event: "n/a",
        event_type: "n/a",
        yaml_event: "n/a",
        actions: &["TransportEvent::MessageReceived"],
        kind: WiringKind::TransportOnly,
        notes: "Transport preserves typed SIP Message plus raw bytes for dialog transaction handling.",
    },
];

pub fn render_wiring_markdown() -> String {
    let mut out = String::new();
    out.push_str("# State Machine Wiring Manifest\n\n");
    out.push_str("Generated from `rvoip_sip::state_table::wiring_manifest`. Do not edit by hand; update the Rust manifest and rerun the drift test.\n\n");
    out.push_str("| SIP/API message | Caller/source | infra-common event | rvoip-sip event | YAML event | Actions/call path | Wiring | Notes |\n");
    out.push_str("|---|---|---|---|---|---|---|---|\n");
    for row in EVENT_WIRINGS {
        out.push_str("| ");
        out.push_str(&escape(row.sip_message));
        out.push_str(" | ");
        out.push_str(&escape(row.caller));
        out.push_str(" | ");
        out.push_str(&escape(row.infra_event));
        out.push_str(" | ");
        out.push_str(&escape(row.event_type));
        out.push_str(" | ");
        out.push_str(&escape(row.yaml_event));
        out.push_str(" | ");
        out.push_str(&escape(&row.actions.join(", ")));
        out.push_str(" | ");
        out.push_str(row.kind.as_str());
        out.push_str(" | ");
        out.push_str(&escape(row.notes));
        out.push_str(" |\n");
    }
    out
}

fn escape(value: &str) -> String {
    value.replace('|', "\\|")
}
