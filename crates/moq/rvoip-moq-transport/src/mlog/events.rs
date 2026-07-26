// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

// TODO: Unimplemented control message events (not yet needed for basic relay interop testing):
// - TrackStatus (parsed/created)
// - SubscribeNamespace (parsed/created)
// - RequestUpdate (parsed/created)
// - Fetch, FetchOk (parsed/created)
// - Publish, PublishSkipped, PublishDone (parsed/created)
// Note: MaxRequestId/RequestsBlocked removed in draft-18 (#1471)
//
// TODO: Unimplemented data plane events (from draft-pardue-moq-qlog-moq-events):
// - stream_type_set (when stream type becomes known)
// - object_datagram_status_created/parsed
// - fetch_header_created/parsed
// - fetch_object_created/parsed
//
// TODO: stream_id field currently uses placeholder value (0)
// - Need to plumb actual QUIC stream IDs through web_transport abstractions
// - This would enable correlation between QUIC qlog and MoQ mlog events

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use crate::{coding, data, message, setup};

/// MoQ Transport event following qlog patterns
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Time in milliseconds since connection start
    pub time: f64,

    /// Event name in format "moqt:event_name"
    pub name: String,

    /// Event-specific data
    pub data: EventData,
}

/// Union of all MoQ Transport event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum EventData {
    #[serde(rename = "control_message_parsed")]
    ControlMessageParsed(ControlMessageParsed),

    #[serde(rename = "control_message_created")]
    ControlMessageCreated(ControlMessageCreated),

    #[serde(rename = "subgroup_header_parsed")]
    SubgroupHeaderParsed(SubgroupHeaderParsed),

    #[serde(rename = "subgroup_header_created")]
    SubgroupHeaderCreated(SubgroupHeaderCreated),

    #[serde(rename = "subgroup_object_parsed")]
    SubgroupObjectParsed(SubgroupObjectParsed),

    #[serde(rename = "subgroup_object_created")]
    SubgroupObjectCreated(SubgroupObjectCreated),

    #[serde(rename = "object_datagram_parsed")]
    ObjectDatagramParsed(ObjectDatagramParsed),

    #[serde(rename = "object_datagram_created")]
    ObjectDatagramCreated(ObjectDatagramCreated),

    #[serde(rename = "loglevel")]
    LogLevel(LogLevelEvent),
}

/// Control message parsed event (Section 4.2 of draft-pardue-moq-qlog-moq-events)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMessageParsed {
    pub stream_id: u64,
    pub message_type: String,

    /// Message-specific fields
    #[serde(flatten)]
    pub message: JsonValue,
}

/// Control message created event (Section 4.1 of draft-pardue-moq-qlog-moq-events)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMessageCreated {
    pub stream_id: u64,
    pub message_type: String,

    /// Message-specific fields
    #[serde(flatten)]
    pub message: JsonValue,
}

/// Subgroup header parsed event (data plane)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgroupHeaderParsed {
    pub stream_id: u64,

    /// Header-specific fields
    #[serde(flatten)]
    pub header: JsonValue,
}

/// Subgroup header created event (data plane)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgroupHeaderCreated {
    pub stream_id: u64,

    /// Header-specific fields
    #[serde(flatten)]
    pub header: JsonValue,
}

/// Subgroup object parsed event (data plane)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgroupObjectParsed {
    pub stream_id: u64,

    /// Object-specific fields
    #[serde(flatten)]
    pub object: JsonValue,
}

/// Subgroup object created event (data plane)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgroupObjectCreated {
    pub stream_id: u64,

    /// Object-specific fields
    #[serde(flatten)]
    pub object: JsonValue,
}

/// Object Datagram parsed event (data plane)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectDatagramParsed {
    pub stream_id: u64,

    /// Object-specific fields
    #[serde(flatten)]
    pub object: JsonValue,
}

/// Object Datagram created event (data plane)
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectDatagramCreated {
    pub stream_id: u64,

    /// Object-specific fields
    #[serde(flatten)]
    pub object: JsonValue,
}

/// LogLevel event for flexible logging (qlog loglevel schema)
/// See: <https://www.ietf.org/archive/id/draft-ietf-quic-qlog-main-schema-12.html#name-loglevel-events>
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLevelEvent {
    pub message: String,
}

// Helper functions to create vector of string pairs from KVPs
fn key_value_pairs_to_vec(kvps: &[coding::KeyValuePair]) -> Vec<(String, String)> {
    kvps.iter()
        .map(|kvp| (kvp.key.to_string(), format!("{:?}", kvp.value)))
        .collect()
}

// SETUP may carry bearer credentials. Keep the option's presence useful for
// diagnostics without ever serializing its value into mlog output.
fn setup_key_value_pairs_to_vec(kvps: &[coding::KeyValuePair]) -> Vec<(String, String)> {
    let authorization_key: u64 = setup::ParameterType::AuthorizationToken.into();
    let path_key: u64 = setup::ParameterType::Path.into();
    let authority_key: u64 = setup::ParameterType::Authority.into();
    kvps.iter()
        .map(|kvp| {
            let value = if kvp.key == authorization_key {
                "<redacted>".to_string()
            } else if kvp.key == authority_key {
                "<redacted-authority>".to_string()
            } else if kvp.key == path_key {
                match &kvp.value {
                    coding::Value::BytesValue(bytes) => {
                        let path = bytes
                            .split(|byte| *byte == b'?')
                            .next()
                            .and_then(|path| std::str::from_utf8(path).ok())
                            .unwrap_or("<invalid-path>");
                        if bytes.contains(&b'?') {
                            format!("{path}?<redacted>")
                        } else {
                            path.to_string()
                        }
                    }
                    _ => "<invalid-path>".to_string(),
                }
            } else {
                format!("{:?}", kvp.value)
            };
            (kvp.key.to_string(), value)
        })
        .collect()
}

// Request parameter type 0x03 is AUTHORIZATION_TOKEN. Keep this serializer
// separate from track/object extension serializers, where the same numeric
// key belongs to a different extension key space.
fn request_parameters_to_vec(kvps: &[coding::KeyValuePair]) -> Vec<(String, String)> {
    const AUTHORIZATION_TOKEN: u64 = 0x03;
    kvps.iter()
        .map(|kvp| {
            let value = if kvp.key == AUTHORIZATION_TOKEN {
                "<redacted>".to_string()
            } else {
                format!("{:?}", kvp.value)
            };
            (kvp.key.to_string(), value)
        })
        .collect()
}

fn create_control_message_event(
    time: f64,
    stream_id: u64,
    is_parsed: bool,
    msg_type: &str,
    message: JsonValue,
) -> Event {
    if is_parsed {
        Event {
            time,
            name: "moqt:control_message_parsed".to_string(),
            data: EventData::ControlMessageParsed(ControlMessageParsed {
                stream_id,
                message_type: msg_type.to_string(),
                message,
            }),
        }
    } else {
        Event {
            time,
            name: "moqt:control_message_created".to_string(),
            data: EventData::ControlMessageCreated(ControlMessageCreated {
                stream_id,
                message_type: msg_type.to_string(),
                message,
            }),
        }
    }
}

/// Create a control_message_parsed event for CLIENT_SETUP.
/// From draft-16 the setup payload carries only parameters; version is agreed via ALPN.
pub fn client_setup_parsed(time: f64, stream_id: u64, msg: &setup::Setup) -> Event {
    create_control_message_event(
        time,
        stream_id,
        true,
        "client_setup",
        json!({
            "parameters": setup_key_value_pairs_to_vec(&msg.params.0),
        }),
    )
}

/// Create a control_message_created event for SERVER_SETUP.
/// From draft-16 the setup payload carries only parameters; version is agreed via ALPN.
pub fn server_setup_created(time: f64, stream_id: u64, msg: &setup::Setup) -> Event {
    create_control_message_event(
        time,
        stream_id,
        false,
        "server_setup",
        json!({
            "parameters": setup_key_value_pairs_to_vec(&msg.params.0),
        }),
    )
}

/// Helper to convert SUBSCRIBE message to JSON
fn subscribe_to_json(msg: &message::Subscribe) -> JsonValue {
    json!({
        "subscribe_id": msg.id,
        "track_namespace": msg.track_namespace.to_string(),
        "track_name": msg.track_name.to_string(),
        "parameters": request_parameters_to_vec(&msg.params.0),
    })
}

/// Create a control_message_parsed event for SUBSCRIBE
pub fn subscribe_parsed(time: f64, stream_id: u64, msg: &message::Subscribe) -> Event {
    create_control_message_event(time, stream_id, true, "subscribe", subscribe_to_json(msg))
}

/// Create a control_message_created event for SUBSCRIBE
pub fn subscribe_created(time: f64, stream_id: u64, msg: &message::Subscribe) -> Event {
    create_control_message_event(time, stream_id, false, "subscribe", subscribe_to_json(msg))
}

/// Helper to convert SUBSCRIBE_OK message to JSON
fn subscribe_ok_to_json(msg: &message::SubscribeOk) -> JsonValue {
    json!({
        "subscribe_id": msg.id,
        "track_alias": msg.track_alias,
        "parameters": request_parameters_to_vec(&msg.params.0),
        "track_extensions": key_value_pairs_to_vec(&msg.track_extensions.0),
    })
}

/// Create a control_message_parsed event for SUBSCRIBE_OK
pub fn subscribe_ok_parsed(time: f64, stream_id: u64, msg: &message::SubscribeOk) -> Event {
    create_control_message_event(
        time,
        stream_id,
        true,
        "subscribe_ok",
        subscribe_ok_to_json(msg),
    )
}

/// Create a control_message_created event for SUBSCRIBE_OK
pub fn subscribe_ok_created(time: f64, stream_id: u64, msg: &message::SubscribeOk) -> Event {
    create_control_message_event(
        time,
        stream_id,
        false,
        "subscribe_ok",
        subscribe_ok_to_json(msg),
    )
}

/// Helper to convert PUBLISH_NAMESPACE message to JSON
fn publish_namespace_to_json(msg: &message::PublishNamespace) -> JsonValue {
    json!({
        "request_id": msg.id,
        "track_namespace": msg.track_namespace.to_string(),
        "parameters": request_parameters_to_vec(&msg.params.0),
    })
}

/// Create a control_message_parsed event for PUBLISH_NAMESPACE (was ANNOUNCE in earlier drafts)
pub fn publish_namespace_parsed(
    time: f64,
    stream_id: u64,
    msg: &message::PublishNamespace,
) -> Event {
    create_control_message_event(
        time,
        stream_id,
        true,
        "publish_namespace",
        publish_namespace_to_json(msg),
    )
}

/// Create a control_message_created event for PUBLISH_NAMESPACE
pub fn publish_namespace_created(
    time: f64,
    stream_id: u64,
    msg: &message::PublishNamespace,
) -> Event {
    create_control_message_event(
        time,
        stream_id,
        false,
        "publish_namespace",
        publish_namespace_to_json(msg),
    )
}

fn request_ok_to_json(request_kind: &str, msg: &message::RequestOk) -> JsonValue {
    json!({
        "request_id": msg.id,
        "request_kind": request_kind,
        "parameters": request_parameters_to_vec(&msg.params.0),
        "track_properties": key_value_pairs_to_vec(&msg.track_properties.0),
    })
}

/// Create a control_message_parsed event for REQUEST_OK.
pub fn request_ok_parsed(
    time: f64,
    stream_id: u64,
    request_kind: &str,
    msg: &message::RequestOk,
) -> Event {
    create_control_message_event(
        time,
        stream_id,
        true,
        "request_ok",
        request_ok_to_json(request_kind, msg),
    )
}

/// Create a control_message_created event for REQUEST_OK.
pub fn request_ok_created(
    time: f64,
    stream_id: u64,
    request_kind: &str,
    msg: &message::RequestOk,
) -> Event {
    create_control_message_event(
        time,
        stream_id,
        false,
        "request_ok",
        request_ok_to_json(request_kind, msg),
    )
}

fn request_error_to_json(request_kind: &str, msg: &message::RequestError) -> JsonValue {
    json!({
        "request_id": msg.id,
        "request_kind": request_kind,
        "error_code": msg.error_code,
        "retry_interval": msg.retry_interval,
        "reason_phrase": &msg.reason.0,
        "redirect": msg.redirect.as_ref().map(|redirect| json!({
            "connect_uri": &redirect.connect_uri.0,
            "track_namespace": redirect.track_namespace.to_utf8_path(),
            "track_name": redirect.track_name.to_string_lossy(),
        })),
    })
}

/// Create a control_message_parsed event for REQUEST_ERROR.
pub fn request_error_parsed(
    time: f64,
    stream_id: u64,
    request_kind: &str,
    msg: &message::RequestError,
) -> Event {
    create_control_message_event(
        time,
        stream_id,
        true,
        "request_error",
        request_error_to_json(request_kind, msg),
    )
}

/// Create a control_message_created event for REQUEST_ERROR.
pub fn request_error_created(
    time: f64,
    stream_id: u64,
    request_kind: &str,
    msg: &message::RequestError,
) -> Event {
    create_control_message_event(
        time,
        stream_id,
        false,
        "request_error",
        request_error_to_json(request_kind, msg),
    )
}

/// Create a control_message_parsed event for GOAWAY
pub fn go_away_parsed(time: f64, stream_id: u64, msg: &message::GoAway) -> Event {
    create_control_message_event(
        time,
        stream_id,
        true,
        "goaway",
        json!({
            "new_session_uri": &msg.uri.0,
            "timeout": msg.timeout,
        }),
    )
}

/// Create a control_message_created event for GOAWAY
pub fn go_away_created(time: f64, stream_id: u64, msg: &message::GoAway) -> Event {
    create_control_message_event(
        time,
        stream_id,
        false,
        "goaway",
        json!({
            "new_session_uri": &msg.uri.0,
            "timeout": msg.timeout,
        }),
    )
}

// Data plane events

/// Helper to convert SubgroupHeader to JSON
fn subgroup_header_to_json(header: &data::SubgroupHeader) -> JsonValue {
    let mut json = json!({
        "header_type": format!("{:?}", header.header_type),
        "track_alias": header.track_alias,
        "group_id": header.group_id,
        "publisher_priority": header.publisher_priority,
    });

    if let Some(subgroup_id) = header.subgroup_id {
        json["subgroup_id"] = json!(subgroup_id);
    }

    json
}

/// Create a subgroup_header_parsed event
pub fn subgroup_header_parsed(time: f64, stream_id: u64, header: &data::SubgroupHeader) -> Event {
    Event {
        time,
        name: "moqt:subgroup_header_parsed".to_string(),
        data: EventData::SubgroupHeaderParsed(SubgroupHeaderParsed {
            stream_id,
            header: subgroup_header_to_json(header),
        }),
    }
}

/// Create a subgroup_header_created event
pub fn subgroup_header_created(time: f64, stream_id: u64, header: &data::SubgroupHeader) -> Event {
    Event {
        time,
        name: "moqt:subgroup_header_created".to_string(),
        data: EventData::SubgroupHeaderCreated(SubgroupHeaderCreated {
            stream_id,
            header: subgroup_header_to_json(header),
        }),
    }
}

/// Helper to convert SubgroupObject to JSON
fn subgroup_object_to_json(
    group_id: u64,
    subgroup_id: u64,
    object_id: u64,
    object: &data::SubgroupObject,
) -> JsonValue {
    let mut object_data = json!({
        "group_id": group_id,
        "subgroup_id": subgroup_id,
        "object_id": object_id,
        // TODO send object_playload itself
        "object_payload_length": object.payload_length,
    });

    if let Some(status) = object.status {
        object_data["object_status"] = json!(format!("{:?}", status));
    }

    object_data
}

/// Create a subgroup_object_parsed event
pub fn subgroup_object_parsed(
    time: f64,
    stream_id: u64,
    group_id: u64,
    subgroup_id: u64,
    object_id: u64,
    object: &data::SubgroupObject,
) -> Event {
    Event {
        time,
        name: "moqt:subgroup_object_parsed".to_string(),
        data: EventData::SubgroupObjectParsed(SubgroupObjectParsed {
            stream_id,
            object: subgroup_object_to_json(group_id, subgroup_id, object_id, object),
        }),
    }
}

/// Create a subgroup_object_created event
pub fn subgroup_object_created(
    time: f64,
    stream_id: u64,
    group_id: u64,
    subgroup_id: u64,
    object_id: u64,
    object: &data::SubgroupObject,
) -> Event {
    Event {
        time,
        name: "moqt:subgroup_object_created".to_string(),
        data: EventData::SubgroupObjectCreated(SubgroupObjectCreated {
            stream_id,
            object: subgroup_object_to_json(group_id, subgroup_id, object_id, object),
        }),
    }
}

/// Helper to convert SubgroupObject to JSON
fn subgroup_object_ext_to_json(
    group_id: u64,
    subgroup_id: u64,
    object_id: u64,
    object: &data::SubgroupObjectExt,
) -> JsonValue {
    let mut object_data = json!({
        "group_id": group_id,
        "subgroup_id": subgroup_id,
        "object_id": object_id,
        "extension_headers": key_value_pairs_to_vec(&object.extension_headers.0),
        // TODO send object_playload itself
        "object_payload_length": object.payload_length,
    });

    if let Some(status) = object.status {
        object_data["object_status"] = json!(format!("{:?}", status));
    }

    object_data
}

/// Create a subgroup_object_parsed event (with extensions)
pub fn subgroup_object_ext_parsed(
    time: f64,
    stream_id: u64,
    group_id: u64,
    subgroup_id: u64,
    object_id: u64,
    object: &data::SubgroupObjectExt,
) -> Event {
    Event {
        time,
        name: "moqt:subgroup_object_parsed".to_string(),
        data: EventData::SubgroupObjectParsed(SubgroupObjectParsed {
            stream_id,
            object: subgroup_object_ext_to_json(group_id, subgroup_id, object_id, object),
        }),
    }
}

/// Create a subgroup_object_created event (with extensions)
pub fn subgroup_object_ext_created(
    time: f64,
    stream_id: u64,
    group_id: u64,
    subgroup_id: u64,
    object_id: u64,
    object: &data::SubgroupObjectExt,
) -> Event {
    Event {
        time,
        name: "moqt:subgroup_object_created".to_string(),
        data: EventData::SubgroupObjectCreated(SubgroupObjectCreated {
            stream_id,
            object: subgroup_object_ext_to_json(group_id, subgroup_id, object_id, object),
        }),
    }
}

/// Helper to convert Datagram to JSON
fn object_datagram_to_json(datagram: &data::Datagram) -> JsonValue {
    let mut json = json!({
        "datagram_type": format!("{:?}", datagram.datagram_type),
        "track_alias": datagram.track_alias,
        "group_id": datagram.group_id,
        "object_id": datagram.object_id.unwrap_or(0),
        "publisher_priority": datagram.publisher_priority,
        // TODO send object_playload
        "payload_length": datagram.payload.as_ref().map_or(0, |p| p.len()),
    });

    if let Some(extension_headers) = &datagram.extension_headers {
        json["extension_headers"] = json!(key_value_pairs_to_vec(&extension_headers.0));
    }

    if let Some(status) = datagram.status {
        json["object_status"] = json!(format!("{:?}", status));
    }

    json
}

/// Create a object_datagram_parsed event
pub fn object_datagram_parsed(time: f64, stream_id: u64, datagram: &data::Datagram) -> Event {
    Event {
        time,
        name: "moqt:object_datagram_parsed".to_string(),
        data: EventData::ObjectDatagramParsed(ObjectDatagramParsed {
            stream_id,
            object: object_datagram_to_json(datagram),
        }),
    }
}

/// Create a object_datagram_created event
pub fn object_datagram_created(time: f64, stream_id: u64, datagram: &data::Datagram) -> Event {
    Event {
        time,
        name: "moqt:object_datagram_created".to_string(),
        data: EventData::ObjectDatagramCreated(ObjectDatagramCreated {
            stream_id,
            object: object_datagram_to_json(datagram),
        }),
    }
}

// LogLevel events (generic logging)

/// Log levels for qlog loglevel events
#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Fatal,
    Error,
    Warn,
    Info,
    Debug,
    Verbose,
}

impl LogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Fatal => "fatal",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Verbose => "verbose",
        }
    }
}

/// Create a loglevel event for flexible logging
///
/// # Arguments
/// * `time` - Timestamp in milliseconds since connection start
/// * `level` - Log level (debug, info, warn, error, fatal, verbose)
/// * `message` - Freeform message text with structured information
///
/// # Example
/// ```ignore
/// loglevel_event(
///     12.345,
///     LogLevel::Debug,
///     "object_queued: track_alias=1 group=5 subgroup=2 object=10 payload_len=1024"
/// )
/// ```
pub fn loglevel_event(time: f64, level: LogLevel, message: String) -> Event {
    Event {
        time,
        name: format!("loglevel:{}", level.as_str()),
        data: EventData::LogLevel(LogLevelEvent { message }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_mlog_redacts_authorization_material() {
        let secret = "bearer-secret-that-must-not-leak";
        let mut params = coding::KeyValuePairs::default();
        params.set_bytesvalue(
            setup::ParameterType::AuthorizationToken.into(),
            secret.as_bytes().to_vec(),
        );
        params.set_bytesvalue(
            setup::ParameterType::Authority.into(),
            b"user:password@relay.example".to_vec(),
        );
        params.set_intvalue(setup::ParameterType::MaxRequestUpdates.into(), 4);
        let setup = setup::Setup { params };

        for event in [
            client_setup_parsed(1.0, 0, &setup),
            server_setup_created(1.0, 0, &setup),
        ] {
            let json = serde_json::to_string(&event).unwrap();
            assert!(!json.contains(secret));
            assert!(json.contains("<redacted>"));
            assert!(json.contains("<redacted-authority>"));
            assert!(!json.contains("75 73 65 72"));
            assert!(json.contains("[\"8\",\"4\"]"));
        }
    }

    #[test]
    fn setup_mlog_redacts_query_material_but_retains_path() {
        let mut params = coding::KeyValuePairs::default();
        params.set_bytesvalue(
            setup::ParameterType::Path.into(),
            b"/tenant/live?token=very-secret".to_vec(),
        );
        let event = client_setup_parsed(1.0, 0, &setup::Setup { params });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("/tenant/live?<redacted>"));
        assert!(!json.contains("very-secret"));
        assert!(!json.contains("76 65 72 79"));
    }

    #[test]
    fn request_mlog_redacts_authorization_without_redacting_extension_keyspace() {
        let mut params = coding::KeyValuePairs::default();
        params.set_bytesvalue(0x03, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let subscribe = message::Subscribe {
            id: 1,
            track_namespace: coding::TrackNamespace::from_utf8_path("tenant/live"),
            track_name: "audio".into(),
            params: params.clone(),
        };
        let publish = message::PublishNamespace {
            id: 2,
            track_namespace: coding::TrackNamespace::from_utf8_path("tenant/live"),
            params,
        };

        for event in [
            subscribe_parsed(1.0, 0, &subscribe),
            publish_namespace_created(1.0, 0, &publish),
        ] {
            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("<redacted>"));
            assert!(!json.contains("DE AD BE EF"));
        }
    }
}
