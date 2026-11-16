//! Amazon Connect's v1 extension framing. Payload-bearing types omit Debug.
use crate::{A2aError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const EXTENSION: &str = "https://docs.aws.amazon.com/connect/a2a/ext/v1";
pub const EVENT_TYPE: &str = "https://docs.aws.amazon.com/connect/a2a/ext/v1/eventType";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioConfiguration {
    pub encoding: String,
    pub sample_rate_hertz: u32,
    pub sample_size_bits: u16,
    pub channel_count: u16,
}
impl AudioConfiguration {
    pub fn pcm(sample_rate_hertz: u32) -> Self {
        Self {
            encoding: "LINEAR_PCM".into(),
            sample_rate_hertz,
            sample_size_bits: 16,
            channel_count: 1,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.encoding != "LINEAR_PCM"
            || self.sample_size_bits != 16
            || self.channel_count != 1
            || ![8000, 16000, 24000].contains(&self.sample_rate_hertz)
        {
            return Err(A2aError::Unsupported);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinishType {
    Complete,
    Escalate,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InterruptionType {
    UserAudioInput,
    DtmfInput,
    System,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitSession {
    /// Effective instance binding. Live Connect may omit this wire field;
    /// in that case it is resolved from `contactArn` before admission.
    #[serde(default)]
    pub instance_arn: String,
    pub contact_arn: String,
    #[serde(default)]
    pub supported_finish_types: Vec<FinishType>,
    #[serde(default)]
    pub subscribe_to_tracing_events: bool,
    pub audio_input_configuration: Option<AudioConfiguration>,
    pub audio_output_configuration: Option<AudioConfiguration>,
    #[serde(default)]
    pub history: Vec<Value>,
    pub contact_context: Option<Value>,
}
impl InitSession {
    pub(crate) fn validate(&mut self) -> Result<()> {
        let contact_instance = self
            .contact_arn
            .rsplit_once("/contact/")
            .filter(|(instance, contact)| {
                instance.starts_with("arn:")
                    && instance.contains(":instance/")
                    && uuid::Uuid::parse_str(contact).is_ok()
            })
            .map(|(instance, _)| instance);
        if self.instance_arn.is_empty() {
            self.instance_arn = contact_instance.ok_or(A2aError::Protocol)?.to_owned();
        } else if contact_instance.is_some_and(|instance| instance != self.instance_arn) {
            return Err(A2aError::Protocol);
        }
        if self.instance_arn.is_empty()
            || self.contact_arn.is_empty()
            || self.instance_arn.len() > 2048
            || self.contact_arn.len() > 2048
        {
            return Err(A2aError::Protocol);
        }
        match (
            &self.audio_input_configuration,
            &self.audio_output_configuration,
        ) {
            (Some(input), Some(output)) => {
                input.validate()?;
                output.validate()?;
            }
            (None, None) => (),
            _ => return Err(A2aError::Protocol),
        }
        Ok(())
    }
    pub fn is_audio(&self) -> bool {
        self.audio_input_configuration.is_some()
    }
}

#[derive(Deserialize)]
pub(crate) struct Request {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    pub params: Params,
}
#[derive(Deserialize)]
pub(crate) struct Params {
    pub message: Message,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Message {
    pub message_id: String,
    #[serde(default)]
    pub context_id: String,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Part {
    pub text: Option<String>,
    pub raw: Option<String>,
    pub data: Option<Value>,
    pub media_type: Option<String>,
}
impl Request {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let request: Self = serde_json::from_str(text).map_err(|_| A2aError::Protocol)?;
        if request.jsonrpc != "2.0"
            || request.method != "SendMessage"
            || !(request.id.is_string() || request.id.is_i64() || request.id.is_u64())
            || request.params.message.message_id.is_empty()
            || request.params.message.message_id.len() > 512
            || request.params.message.context_id.len() > 512
            || request.params.message.parts.is_empty()
            || request.params.message.parts.len() > 128
        {
            return Err(A2aError::Protocol);
        }
        Ok(request)
    }
    pub(crate) fn event(&self) -> Option<&str> {
        self.params
            .message
            .metadata
            .get(EVENT_TYPE)
            .and_then(Value::as_str)
    }
    pub(crate) fn data<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<T> {
        if self.params.message.parts.len() != 1 {
            return Err(A2aError::Protocol);
        }
        let value = self.params.message.parts[0]
            .data
            .as_ref()
            .and_then(|v| v.get(key))
            .ok_or(A2aError::Protocol)?;
        serde_json::from_value(value.clone()).map_err(|_| A2aError::Protocol)
    }
}
pub(crate) fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub(crate) fn envelope(value: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id(),"result":value})
}
pub(crate) fn message(context: &str, event: &str, data: Value) -> Value {
    json!({"messageId":id(),"contextId":context,"role":"ROLE_AGENT","parts":[{"data":data,"mediaType":"application/json"}],"extensions":[EXTENSION],"metadata":{EVENT_TYPE:event}})
}
pub(crate) fn rpc_error(id: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Invalid A2A request","data":{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"INVALID_REQUEST","domain":"a2a-protocol.org"}}})
}

pub(crate) fn rpc_error_from_text(text: &str) -> Value {
    let (id, code, message) = match serde_json::from_str::<Value>(text) {
        Err(_) => (Value::Null, -32700, "Parse error"),
        Ok(value) => {
            let id = value
                .get("id")
                .filter(|id| id.is_string() || id.is_i64() || id.is_u64())
                .cloned()
                .unwrap_or(Value::Null);
            if value.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
                && value
                    .get("method")
                    .and_then(Value::as_str)
                    .is_some_and(|method| method != "SendMessage")
            {
                (id, -32601, "Method not found")
            } else {
                (id, -32600, "Invalid A2A request")
            }
        }
    };
    let mut response = rpc_error(id);
    response["error"]["code"] = json!(code);
    response["error"]["message"] = json!(message);
    response
}
