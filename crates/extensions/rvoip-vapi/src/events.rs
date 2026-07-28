//! Typed, forward-compatible Vapi event surface.

use std::fmt;

use rvoip_core::ConnectionId;
use serde_json::Value;

#[derive(Clone)]
#[non_exhaustive]
pub enum VapiEvent {
    StatusUpdate {
        status: String,
        ended_reason: Option<String>,
    },
    SpeechUpdate {
        status: String,
        role: Option<String>,
    },
    Transcript {
        role: Option<String>,
        transcript_type: Option<String>,
        transcript: Option<String>,
    },
    ModelOutput {
        output: Option<String>,
    },
    ConversationUpdate {
        conversation: Value,
    },
    ToolCall {
        event_type: String,
        payload: Value,
    },
    EndOfCallReport {
        report: Value,
    },
    Unknown(Value),
    Malformed {
        byte_len: usize,
    },
}

impl VapiEvent {
    pub(crate) fn parse(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return Self::Malformed {
                byte_len: text.len(),
            };
        };
        let Some(event_type) = value.get("type").and_then(Value::as_str) else {
            return Self::Unknown(value);
        };
        match event_type {
            "status-update" => Self::StatusUpdate {
                status: string_field(&value, "status").unwrap_or_default(),
                ended_reason: string_field(&value, "endedReason"),
            },
            "speech-update" => Self::SpeechUpdate {
                status: string_field(&value, "status").unwrap_or_default(),
                role: string_field(&value, "role"),
            },
            "transcript" => Self::Transcript {
                role: string_field(&value, "role"),
                transcript_type: string_field(&value, "transcriptType"),
                transcript: string_field(&value, "transcript"),
            },
            "model-output" => Self::ModelOutput {
                output: string_field(&value, "output")
                    .or_else(|| string_field(&value, "modelOutput")),
            },
            "conversation-update" => Self::ConversationUpdate {
                conversation: value.get("conversation").cloned().unwrap_or(Value::Null),
            },
            "function-call"
            | "function-call-result"
            | "tool-call"
            | "tool-calls"
            | "tool-calls-result" => Self::ToolCall {
                event_type: event_type.to_owned(),
                payload: value,
            },
            "end-of-call-report" => Self::EndOfCallReport { report: value },
            _ => Self::Unknown(value),
        }
    }

    pub(crate) fn is_terminal_status(&self) -> bool {
        matches!(
            self,
            Self::StatusUpdate { status, .. } if status.eq_ignore_ascii_case("ended")
        ) || matches!(self, Self::EndOfCallReport { .. })
    }
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).and_then(Value::as_str).map(str::to_owned)
}

impl fmt::Debug for VapiEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StatusUpdate {
                status,
                ended_reason,
            } => formatter
                .debug_struct("StatusUpdate")
                .field("status_present", &!status.is_empty())
                .field("ended_reason_present", &ended_reason.is_some())
                .finish(),
            Self::SpeechUpdate { status, role } => formatter
                .debug_struct("SpeechUpdate")
                .field("status_present", &!status.is_empty())
                .field("role_present", &role.is_some())
                .finish(),
            Self::Transcript {
                role,
                transcript_type,
                transcript,
            } => formatter
                .debug_struct("Transcript")
                .field("role_present", &role.is_some())
                .field("transcript_type_present", &transcript_type.is_some())
                .field("transcript_present", &transcript.is_some())
                .field(
                    "transcript_bytes",
                    &transcript.as_ref().map_or(0, String::len),
                )
                .finish(),
            Self::ModelOutput { output } => formatter
                .debug_struct("ModelOutput")
                .field("output_present", &output.is_some())
                .field("output_bytes", &output.as_ref().map_or(0, String::len))
                .finish(),
            Self::ConversationUpdate { conversation } => formatter
                .debug_struct("ConversationUpdate")
                .field("entry_count", &conversation.as_array().map_or(0, Vec::len))
                .finish(),
            Self::ToolCall { event_type, .. } => formatter
                .debug_struct("ToolCall")
                .field("event_type_present", &!event_type.is_empty())
                .finish(),
            Self::EndOfCallReport { .. } => formatter.write_str("EndOfCallReport([redacted])"),
            Self::Unknown(_) => formatter.write_str("Unknown([redacted])"),
            Self::Malformed { byte_len } => formatter
                .debug_struct("Malformed")
                .field("byte_len", byte_len)
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct VapiEventEnvelope {
    pub connection_id: ConnectionId,
    pub event: VapiEvent,
}

impl fmt::Debug for VapiEventEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VapiEventEnvelope")
            .field("connection_id", &self.connection_id)
            .field("event", &self.event)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_unknown_and_malformed_events() {
        let transcript = VapiEvent::parse(
            r#"{"type":"transcript","role":"user","transcriptType":"partial","transcript":"hi"}"#,
        );
        assert!(matches!(
            transcript,
            VapiEvent::Transcript {
                transcript: Some(ref text),
                ..
            } if text == "hi"
        ));
        assert!(matches!(
            VapiEvent::parse(r#"{"type":"future-event","secret":"value"}"#),
            VapiEvent::Unknown(_)
        ));
        assert!(matches!(
            VapiEvent::parse("{bad"),
            VapiEvent::Malformed { byte_len: 4 }
        ));
    }

    #[test]
    fn diagnostics_do_not_render_event_content() {
        let event = VapiEvent::parse(
            r#"{"type":"transcript","role":"role-canary","transcript":"content-canary"}"#,
        );
        let debug = format!("{event:?}");
        assert!(!debug.contains("role-canary"));
        assert!(!debug.contains("content-canary"));
    }
}
