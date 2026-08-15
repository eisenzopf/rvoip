//! Public call configuration and control-message types.

use std::fmt;

use rvoip_core::{CapabilityDescriptor, CodecInfo};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::{Result, VapiError};

/// Raw audio format on the Vapi WebSocket.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VapiAudioFormat {
    #[default]
    MuLaw8Khz,
    PcmS16Le16Khz,
}

impl VapiAudioFormat {
    pub const fn frame_bytes(self) -> usize {
        match self {
            Self::MuLaw8Khz => 160,
            Self::PcmS16Le16Khz => 640,
        }
    }

    /// Duration of one frame in milliseconds. Both formats are 20 ms; this
    /// exists so health figures can be expressed as delay rather than counts.
    pub const fn frame_ms(self) -> u32 {
        20
    }

    pub const fn timestamp_increment(self) -> u32 {
        match self {
            Self::MuLaw8Khz => 160,
            Self::PcmS16Le16Khz => 320,
        }
    }

    pub(crate) const fn payload_type(self) -> u8 {
        match self {
            Self::MuLaw8Khz => 0,
            Self::PcmS16Le16Khz => 120,
        }
    }

    pub fn codec(self) -> CodecInfo {
        match self {
            // Both advertise what this transport can carry rather than
            // reporting a negotiation, so neither names a payload type.
            // Both are static names that `codec_to_pt` already resolves.
            Self::MuLaw8Khz => CodecInfo {
                name: "PCMU".into(),
                clock_rate_hz: 8_000,
                channels: 1,
                fmtp: None,
                payload_type: None,
            },
            Self::PcmS16Le16Khz => CodecInfo {
                name: "pcm_s16le".into(),
                clock_rate_hz: 16_000,
                channels: 1,
                fmtp: None,
                payload_type: None,
            },
        }
    }

    pub fn capabilities(self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            audio_codecs: vec![self.codec()],
            max_streams_per_connection: 1,
            ..CapabilityDescriptor::default()
        }
    }

    pub(crate) const fn wire_format(self) -> &'static str {
        match self {
            Self::MuLaw8Khz => "mulaw",
            Self::PcmS16Le16Khz => "pcm_s16le",
        }
    }

    pub(crate) const fn sample_rate(self) -> u32 {
        match self {
            Self::MuLaw8Khz => 8_000,
            Self::PcmS16Le16Khz => 16_000,
        }
    }
}

/// Select a saved Vapi assistant or supply a transient assistant definition.
#[derive(Clone)]
pub enum VapiAssistant {
    Saved {
        id: String,
        overrides: Option<Value>,
    },
    Transient {
        definition: Value,
    },
}

impl VapiAssistant {
    pub fn saved(id: impl Into<String>) -> Self {
        Self::Saved {
            id: id.into(),
            overrides: None,
        }
    }

    pub fn saved_with_overrides(id: impl Into<String>, overrides: Value) -> Self {
        Self::Saved {
            id: id.into(),
            overrides: Some(overrides),
        }
    }

    pub fn transient(definition: Value) -> Self {
        Self::Transient { definition }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::Saved { id, overrides } => {
                if id.trim().is_empty() || id.chars().any(char::is_control) {
                    return Err(VapiError::InvalidCallOptions("the assistant ID is invalid"));
                }
                if overrides.as_ref().is_some_and(|value| !value.is_object()) {
                    return Err(VapiError::InvalidCallOptions(
                        "assistant overrides must be a JSON object",
                    ));
                }
            }
            Self::Transient { definition } if !definition.is_object() => {
                return Err(VapiError::InvalidCallOptions(
                    "the transient assistant must be a JSON object",
                ));
            }
            Self::Transient { .. } => {}
        }
        Ok(())
    }

    pub(crate) fn add_to_payload(&self, payload: &mut Map<String, Value>) {
        match self {
            Self::Saved { id, overrides } => {
                payload.insert("assistantId".into(), Value::String(id.clone()));
                if let Some(overrides) = overrides {
                    payload.insert("assistantOverrides".into(), overrides.clone());
                }
            }
            Self::Transient { definition } => {
                payload.insert("assistant".into(), definition.clone());
            }
        }
    }
}

impl fmt::Debug for VapiAssistant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Saved { overrides, .. } if overrides.is_some() => {
                "Saved { id: [redacted], overrides: present }"
            }
            Self::Saved { .. } => "Saved { id: [redacted], overrides: absent }",
            Self::Transient { .. } => "Transient { definition: [redacted] }",
        })
    }
}

/// What the high-level attachment supervisor does when Vapi terminates first.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VapiPeerFailurePolicy {
    #[default]
    EndCaller,
    LeaveCallerConnected,
}

/// Per-call options carried opaquely through `OriginateRequest`.
#[derive(Clone)]
pub struct VapiCallOptions {
    pub assistant: VapiAssistant,
    pub audio_format: VapiAudioFormat,
    pub name: Option<String>,
    pub metadata: Option<Value>,
    pub peer_failure_policy: VapiPeerFailurePolicy,
}

impl VapiCallOptions {
    pub fn new(assistant: VapiAssistant) -> Self {
        Self {
            assistant,
            audio_format: VapiAudioFormat::default(),
            name: None,
            metadata: None,
            peer_failure_policy: VapiPeerFailurePolicy::default(),
        }
    }

    pub fn with_audio_format(mut self, audio_format: VapiAudioFormat) -> Self {
        self.audio_format = audio_format;
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_peer_failure_policy(mut self, policy: VapiPeerFailurePolicy) -> Self {
        self.peer_failure_policy = policy;
        self
    }

    pub fn validate(&self) -> Result<()> {
        self.assistant.validate()?;
        if self.name.as_ref().is_some_and(|name| {
            name.trim().is_empty() || name.len() > 40 || name.chars().any(char::is_control)
        }) {
            return Err(VapiError::InvalidCallOptions("the call name is invalid"));
        }
        if self
            .metadata
            .as_ref()
            .is_some_and(|metadata| !metadata.is_object())
        {
            return Err(VapiError::InvalidCallOptions(
                "call metadata must be a JSON object",
            ));
        }
        Ok(())
    }

    pub(crate) fn create_call_payload(&self) -> Value {
        let mut payload = Map::new();
        self.assistant.add_to_payload(&mut payload);
        payload.insert(
            "transport".into(),
            serde_json::json!({
                "provider": "vapi.websocket",
                "audioFormat": {
                    "format": self.audio_format.wire_format(),
                    "container": "raw",
                    "sampleRate": self.audio_format.sample_rate(),
                }
            }),
        );
        if let Some(name) = &self.name {
            payload.insert("name".into(), Value::String(name.clone()));
        }
        if let Some(metadata) = &self.metadata {
            payload.insert("metadata".into(), metadata.clone());
        }
        Value::Object(payload)
    }
}

impl fmt::Debug for VapiCallOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VapiCallOptions")
            .field("assistant", &self.assistant)
            .field("audio_format", &self.audio_format)
            .field("name_present", &self.name.is_some())
            .field("metadata_present", &self.metadata.is_some())
            .field("peer_failure_policy", &self.peer_failure_policy)
            .finish()
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum VapiCommand {
    #[serde(rename = "say")]
    Say {
        content: String,
        #[serde(rename = "endCallAfterSpoken")]
        end_call_after_spoken: bool,
        #[serde(rename = "interruptAssistantEnabled")]
        interrupt_assistant_enabled: bool,
    },
    #[serde(rename = "add-message")]
    AddMessage {
        message: AddedMessage,
        #[serde(rename = "triggerResponseEnabled")]
        trigger_response_enabled: bool,
    },
    #[serde(rename = "control")]
    Control { control: &'static str },
}

#[derive(Clone, Serialize)]
pub(crate) struct AddedMessage {
    pub role: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_and_transient_payloads_match_vapi_shape() {
        let saved = VapiCallOptions::new(VapiAssistant::saved_with_overrides(
            "assistant-id",
            serde_json::json!({"firstMessage":"hello"}),
        ))
        .create_call_payload();
        assert_eq!(saved["assistantId"], "assistant-id");
        assert_eq!(saved["transport"]["provider"], "vapi.websocket");
        assert_eq!(saved["transport"]["audioFormat"]["format"], "mulaw");
        assert_eq!(saved["transport"]["audioFormat"]["sampleRate"], 8_000);

        let transient = VapiCallOptions::new(VapiAssistant::transient(
            serde_json::json!({"model":{"provider":"openai"}}),
        ))
        .with_audio_format(VapiAudioFormat::PcmS16Le16Khz)
        .create_call_payload();
        assert!(transient.get("assistantId").is_none());
        assert!(transient["assistant"].is_object());
        assert_eq!(transient["transport"]["audioFormat"]["format"], "pcm_s16le");
        assert_eq!(transient["transport"]["audioFormat"]["sampleRate"], 16_000);
    }

    #[test]
    fn option_diagnostics_redact_content() {
        let options = VapiCallOptions::new(VapiAssistant::saved_with_overrides(
            "assistant-canary",
            serde_json::json!({"secret":"override-canary"}),
        ))
        .with_name("name-canary")
        .with_metadata(serde_json::json!({"customer":"metadata-canary"}));
        let debug = format!("{options:?}");
        for canary in [
            "assistant-canary",
            "override-canary",
            "name-canary",
            "metadata-canary",
        ] {
            assert!(!debug.contains(canary));
        }
    }
}
