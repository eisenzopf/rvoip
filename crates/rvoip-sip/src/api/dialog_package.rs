//! RFC 4235 dialog event package parsing and typed state.
//!
//! Decodes `application/dialog-info+xml` NOTIFY bodies into typed
//! [`DialogInfoDocument`] / [`DialogInfo`] values so subscribers built on
//! [`DialogSubscriptionHandle`](crate::DialogSubscriptionHandle) can react to
//! dialog lifecycle changes (early/confirmed/terminated, with cause) without
//! re-parsing XML.

use crate::errors::{Result, SessionError};

/// RFC 4235 dialog state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogPackageState {
    /// `trying` — INVITE sent, no provisional response yet.
    Trying,
    /// `proceeding` — provisional response received without early media.
    Proceeding,
    /// `early` — early dialog established (e.g. 180 with To-tag).
    Early,
    /// `confirmed` — final 2xx received and the dialog is established.
    Confirmed,
    /// `terminated` — dialog has ended.
    Terminated,
    /// Vendor-specific state outside the RFC 4235 enum; the raw value is
    /// preserved verbatim.
    Unknown(String),
}

impl DialogPackageState {
    /// Parse a `<state>` element body into a [`DialogPackageState`]. Unknown
    /// values are surfaced via [`Self::Unknown`] rather than failing so
    /// vendor extensions still reach the caller.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "trying" => Self::Trying,
            "proceeding" => Self::Proceeding,
            "early" => Self::Early,
            "confirmed" => Self::Confirmed,
            "terminated" => Self::Terminated,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// True iff the dialog has reached its terminal state.
    pub fn is_terminated(&self) -> bool {
        matches!(self, Self::Terminated)
    }
}

/// RFC 4235 dialog-state event/cause carried on the `event` attribute of a
/// `<state>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogPackageEvent {
    /// `cancelled` — UAC sent CANCEL before answer.
    Cancelled,
    /// `rejected` — UAS returned a non-2xx final.
    Rejected,
    /// `replaced` — dialog was replaced by a `Replaces` INVITE.
    Replaced,
    /// `local-bye` — local UA sent BYE.
    LocalBye,
    /// `remote-bye` — remote UA sent BYE.
    RemoteBye,
    /// `error` — transport or protocol error tore the dialog down.
    Error,
    /// `timeout` — dialog ended due to timer expiry (e.g. session timer,
    /// no ACK).
    Timeout,
    /// Vendor-specific cause outside the RFC 4235 enum; the raw value is
    /// preserved verbatim.
    Unknown(String),
}

impl DialogPackageEvent {
    /// Parse a `state` element's `event` attribute into a typed
    /// [`DialogPackageEvent`]. Unknown values are surfaced via
    /// [`Self::Unknown`] rather than failing.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "cancelled" | "canceled" => Self::Cancelled,
            "rejected" => Self::Rejected,
            "replaced" => Self::Replaced,
            "local-bye" => Self::LocalBye,
            "remote-bye" => Self::RemoteBye,
            "error" => Self::Error,
            "timeout" => Self::Timeout,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// One `<dialog>` entry from an RFC 4235 `dialog-info` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogInfo {
    /// Value of the `id` attribute on `<dialog>`.
    pub id: String,
    /// `call-id` attribute, when present.
    pub call_id: Option<String>,
    /// `local-tag` attribute, when present.
    pub local_tag: Option<String>,
    /// `remote-tag` attribute, when present.
    pub remote_tag: Option<String>,
    /// `direction` attribute (typically `initiator` or `recipient`).
    pub direction: Option<String>,
    /// Typed dialog state parsed from `<state>`.
    pub state: DialogPackageState,
    /// Typed dialog-state event parsed from `<state event=…>`, when present.
    pub event: Option<DialogPackageEvent>,
    /// Local-side URI (from `<local><identity>` or `<local><target uri=…>`).
    pub local_uri: Option<String>,
    /// Remote-side URI (from `<remote><identity>` or
    /// `<remote><target uri=…>`).
    pub remote_uri: Option<String>,
    /// Verbatim `<state>` text, preserved so callers can recover from
    /// [`DialogPackageState::Unknown`] without re-fetching the XML.
    pub raw_state: String,
    /// Verbatim `event` attribute, preserved so callers can recover from
    /// [`DialogPackageEvent::Unknown`].
    pub raw_event: Option<String>,
}

impl DialogInfo {
    /// True iff the underlying dialog state is `terminated`.
    pub fn is_terminated(&self) -> bool {
        self.state.is_terminated()
    }
}

/// Parsed RFC 4235 `dialog-info` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogInfoDocument {
    /// `entity` attribute on the root `<dialog-info>` element — usually the
    /// monitored AOR.
    pub entity: Option<String>,
    /// `version` attribute on the root, when present.
    pub version: Option<u32>,
    /// Root document state (typically `full` or `partial`).
    pub state: Option<String>,
    /// One entry per `<dialog>` child.
    pub dialogs: Vec<DialogInfo>,
}

/// Parse an RFC 4235 `application/dialog-info+xml` NOTIFY body.
pub fn parse_dialog_info_xml(body: &str) -> Result<DialogInfoDocument> {
    let doc = roxmltree::Document::parse(body)
        .map_err(|e| SessionError::Other(format!("failed to parse dialog-info XML body: {e}")))?;
    let root = doc
        .descendants()
        .find(|node| node.has_tag_name("dialog-info"))
        .ok_or_else(|| SessionError::Other("dialog-info XML missing root element".to_string()))?;

    let dialogs = root
        .children()
        .filter(|node| node.has_tag_name("dialog"))
        .map(parse_dialog_node)
        .collect();

    Ok(DialogInfoDocument {
        entity: root.attribute("entity").map(ToString::to_string),
        version: root
            .attribute("version")
            .and_then(|value| value.parse::<u32>().ok()),
        state: root.attribute("state").map(ToString::to_string),
        dialogs,
    })
}

fn parse_dialog_node(node: roxmltree::Node<'_, '_>) -> DialogInfo {
    let id = node.attribute("id").unwrap_or_default().to_string();
    let state_node = node.children().find(|child| child.has_tag_name("state"));
    let raw_state = state_node
        .and_then(|state| state.text())
        .unwrap_or_default()
        .trim()
        .to_string();
    let raw_event = state_node
        .and_then(|state| state.attribute("event"))
        .map(ToString::to_string);

    DialogInfo {
        id,
        call_id: node.attribute("call-id").map(ToString::to_string),
        local_tag: node.attribute("local-tag").map(ToString::to_string),
        remote_tag: node.attribute("remote-tag").map(ToString::to_string),
        direction: node.attribute("direction").map(ToString::to_string),
        state: DialogPackageState::parse(&raw_state),
        event: raw_event.as_deref().map(DialogPackageEvent::parse),
        local_uri: endpoint_uri(node, "local"),
        remote_uri: endpoint_uri(node, "remote"),
        raw_state,
        raw_event,
    }
}

fn endpoint_uri(dialog: roxmltree::Node<'_, '_>, tag: &str) -> Option<String> {
    let endpoint = dialog.children().find(|child| child.has_tag_name(tag))?;
    endpoint
        .children()
        .find(|child| child.has_tag_name("target"))
        .and_then(|target| target.attribute("uri"))
        .or_else(|| {
            endpoint
                .children()
                .find(|child| child.has_tag_name("identity"))
                .and_then(|identity| identity.text())
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_confirmed_and_terminated_dialogs() {
        let xml = r#"
            <dialog-info xmlns="urn:ietf:params:xml:ns:dialog-info"
                         version="3"
                         state="partial"
                         entity="sip:pbx@example.com">
              <dialog id="dlg-1" call-id="call-a" local-tag="lt" remote-tag="rt" direction="initiator">
                <state>confirmed</state>
                <local><identity>sip:1002@example.com</identity></local>
                <remote><target uri="sip:1003@example.com"/></remote>
              </dialog>
              <dialog id="dlg-2" call-id="call-b">
                <state event="remote-bye">terminated</state>
              </dialog>
            </dialog-info>
        "#;

        let parsed = parse_dialog_info_xml(xml).unwrap();
        assert_eq!(parsed.entity.as_deref(), Some("sip:pbx@example.com"));
        assert_eq!(parsed.version, Some(3));
        assert_eq!(parsed.dialogs.len(), 2);
        assert_eq!(parsed.dialogs[0].state, DialogPackageState::Confirmed);
        assert_eq!(
            parsed.dialogs[0].remote_uri.as_deref(),
            Some("sip:1003@example.com")
        );
        assert_eq!(parsed.dialogs[1].state, DialogPackageState::Terminated);
        assert_eq!(parsed.dialogs[1].event, Some(DialogPackageEvent::RemoteBye));
    }

    #[test]
    fn preserves_unknown_dialog_state_and_event() {
        let xml = r#"
            <dialog-info xmlns="urn:ietf:params:xml:ns:dialog-info">
              <dialog id="dlg">
                <state event="vendor-cause">vendor-state</state>
              </dialog>
            </dialog-info>
        "#;

        let parsed = parse_dialog_info_xml(xml).unwrap();
        assert_eq!(
            parsed.dialogs[0].state,
            DialogPackageState::Unknown("vendor-state".to_string())
        );
        assert_eq!(
            parsed.dialogs[0].event,
            Some(DialogPackageEvent::Unknown("vendor-cause".to_string()))
        );
    }
}
