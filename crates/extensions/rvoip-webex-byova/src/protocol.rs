use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const MAX_TEXT_BYTES: usize = 131_072;
pub const MAX_AUDIO_BYTES: usize = 32_000;

/// Errors intentionally contain no untrusted input or credentials.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid envelope")]
    Envelope,
    #[error("invalid session state")]
    State,
    #[error("session identity mismatch")]
    Identity,
    #[error("sequence regression")]
    Sequence,
    #[error("unsupported or invalid media")]
    Media,
    #[error("message exceeds limit")]
    Limit,
    #[error("stale or finalized reply")]
    StaleReply,
    #[error("utterance shorter than the qualified minimum chunk")]
    ShortAudio,
}

impl ProtocolError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Media | Self::ShortAudio => "unsupported_media",
            Self::Limit => "rate_limit",
            _ => "bad_request",
        }
    }
}

/// Application input. Content is intentionally omitted from Debug.
pub enum Input {
    Start,
    Audio(Vec<u8>),
    Dtmf(String),
    DtmfStart,
    NoInput,
    Custom { name: String, parameters: Value },
    End,
    Ping(u64),
    Pong(u64),
}

impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Start => "Start",
            Self::Audio(_) => "Audio([redacted])",
            Self::Dtmf(_) => "Dtmf([redacted])",
            Self::DtmfStart => "DtmfStart",
            Self::NoInput => "NoInput",
            Self::Custom { .. } => "Custom([redacted])",
            Self::End => "End",
            Self::Ping(_) => "Ping",
            Self::Pong(_) => "Pong",
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    #[serde(rename = "type")]
    pub kind: String,
    pub seq: u64,
    pub ts: String,
    pub conversation_id: String,
    #[serde(default, deserialize_with = "nullable_map")]
    pub metadata: BTreeMap<String, Value>,
    pub payload: Option<Request>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub conversation_id: String,
    pub customer_org_id: String,
    pub virtual_agent_id: Option<String>,
    #[serde(default)]
    pub allow_partial_responses: bool,
    pub vendor_specific_config: Option<String>,
    pub voice_va_input_type: InputWrapper,
    #[serde(default, deserialize_with = "nullable_map")]
    pub additional_info: BTreeMap<String, String>,
}

// Cisco's live WebSocket serializer emits null for absent optional maps.
// Treat only these maps as empty; required fields and populated map values
// retain their original types and validation.
fn nullable_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<BTreeMap<String, T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputWrapper {
    pub audio_input: Option<Audio>,
    pub dtmf_input: Option<Dtmf>,
    pub event_input: Option<Event>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Audio {
    pub caller_audio_b64: String,
    pub encoding: String,
    pub sample_rate_hertz: u32,
    pub audio_timestamp: Option<String>,
    pub language_code: Option<String>,
    pub is_single_utterance: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Dtmf {
    pub dtmf_events: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Event {
    pub event_type: String,
    pub name: Option<String>,
    #[serde(default)]
    pub parameters: Value,
}

pub(crate) fn parse(text: &str) -> Result<Envelope, ProtocolError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ProtocolError::Limit);
    }
    let envelope: Envelope = serde_json::from_str(text).map_err(|_| ProtocolError::Envelope)?;
    if envelope.seq == 0
        || envelope.conversation_id.is_empty()
        || envelope.conversation_id.len() > 256
        || envelope.conversation_id.chars().any(char::is_control)
        || DateTime::parse_from_rfc3339(&envelope.ts).is_err()
        || envelope.metadata.len() > 32
    {
        return Err(ProtocolError::Envelope);
    }
    Ok(envelope)
}

pub(crate) fn input(wrapper: InputWrapper) -> Result<Input, ProtocolError> {
    let count = usize::from(wrapper.audio_input.is_some())
        + usize::from(wrapper.dtmf_input.is_some())
        + usize::from(wrapper.event_input.is_some());
    if count != 1 {
        return Err(ProtocolError::Envelope);
    }
    if let Some(audio) = wrapper.audio_input {
        if audio.encoding != "MULAW_FORMAT" || audio.sample_rate_hertz != 8000 {
            return Err(ProtocolError::Media);
        }
        if audio.caller_audio_b64.len() > MAX_AUDIO_BYTES.div_ceil(3) * 4 {
            return Err(ProtocolError::Limit);
        }
        if audio
            .audio_timestamp
            .as_ref()
            .is_some_and(|ts| DateTime::parse_from_rfc3339(ts).is_err())
            || audio.language_code.as_ref().is_some_and(|s| s.len() > 64)
        {
            return Err(ProtocolError::Envelope);
        }
        // is_single_utterance is a hint, not an endpointing decision.
        let _single_utterance = audio.is_single_utterance;
        let bytes = STANDARD
            .decode(audio.caller_audio_b64)
            .map_err(|_| ProtocolError::Media)?;
        if bytes.is_empty() || bytes.len() > MAX_AUDIO_BYTES || bytes.starts_with(b"RIFF") {
            return Err(ProtocolError::Media);
        }
        return Ok(Input::Audio(bytes));
    }
    if let Some(dtmf) = wrapper.dtmf_input {
        if dtmf.dtmf_events.is_empty() || dtmf.dtmf_events.len() > 64 {
            return Err(ProtocolError::Limit);
        }
        let digits = dtmf
            .dtmf_events
            .iter()
            .map(|digit| match digit.as_str() {
                "DTMF_DIGIT_ZERO" => Ok('0'),
                "DTMF_DIGIT_ONE" => Ok('1'),
                "DTMF_DIGIT_TWO" => Ok('2'),
                "DTMF_DIGIT_THREE" => Ok('3'),
                "DTMF_DIGIT_FOUR" => Ok('4'),
                "DTMF_DIGIT_FIVE" => Ok('5'),
                "DTMF_DIGIT_SIX" => Ok('6'),
                "DTMF_DIGIT_SEVEN" => Ok('7'),
                "DTMF_DIGIT_EIGHT" => Ok('8'),
                "DTMF_DIGIT_NINE" => Ok('9'),
                "DTMF_DIGIT_A" => Ok('A'),
                "DTMF_DIGIT_B" => Ok('B'),
                "DTMF_DIGIT_C" => Ok('C'),
                "DTMF_DIGIT_D" => Ok('D'),
                "DTMF_DIGIT_STAR" => Ok('*'),
                "DTMF_DIGIT_POUND" => Ok('#'),
                _ => Err(ProtocolError::Envelope),
            })
            .collect::<Result<String, _>>()?;
        return Ok(Input::Dtmf(digits));
    }
    let event = wrapper.event_input.ok_or(ProtocolError::Envelope)?;
    match event.event_type.as_str() {
        "SESSION_START" => Ok(Input::Start),
        "SESSION_END" => Ok(Input::End),
        "START_OF_DTMF" => Ok(Input::DtmfStart),
        "NO_INPUT" => Ok(Input::NoInput),
        "CUSTOM_EVENT" => {
            let name = event
                .name
                .filter(|n| !n.is_empty() && n.len() <= 128)
                .ok_or(ProtocolError::Envelope)?;
            if !event.parameters.is_null() && !event.parameters.is_object() {
                return Err(ProtocolError::Envelope);
            }
            Ok(Input::Custom {
                name,
                parameters: event.parameters,
            })
        }
        _ => Err(ProtocolError::Envelope),
    }
}
