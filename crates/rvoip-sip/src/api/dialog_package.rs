//! RFC 4235 dialog event package parsing and typed state.

use crate::errors::{Result, SessionError};

/// RFC 4235 dialog state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogPackageState {
    Trying,
    Proceeding,
    Early,
    Confirmed,
    Terminated,
    Unknown(String),
}

impl DialogPackageState {
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

    pub fn is_terminated(&self) -> bool {
        matches!(self, Self::Terminated)
    }
}

/// RFC 4235 dialog-state event/cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogPackageEvent {
    Cancelled,
    Rejected,
    Replaced,
    LocalBye,
    RemoteBye,
    Error,
    Timeout,
    Unknown(String),
}

impl DialogPackageEvent {
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
    pub id: String,
    pub call_id: Option<String>,
    pub local_tag: Option<String>,
    pub remote_tag: Option<String>,
    pub direction: Option<String>,
    pub state: DialogPackageState,
    pub event: Option<DialogPackageEvent>,
    pub local_uri: Option<String>,
    pub remote_uri: Option<String>,
    pub raw_state: String,
    pub raw_event: Option<String>,
}

impl DialogInfo {
    pub fn is_terminated(&self) -> bool {
        self.state.is_terminated()
    }
}

/// Parsed RFC 4235 `dialog-info` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogInfoDocument {
    pub entity: Option<String>,
    pub version: Option<u32>,
    pub state: Option<String>,
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
