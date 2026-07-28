// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Messages sent over MOQT control and request streams.
//!
//! Draft-19 uses a pair of unidirectional control streams. After `SETUP`,
//! `GOAWAY` is the only message in this module that may be sent on a control
//! stream; every other message is confined to a bidirectional request stream.
//!
//! Wire format per draft-ietf-moq-transport-19 §10:
//!
//! ```text
//! MOQT Control Message {
//!   Message Type (i),
//!   Message Length (16),   ← 16-bit unsigned big-endian
//!   Message Payload (..),
//! }
//! ```
//!
//! The receiver MUST close the session with PROTOCOL_VIOLATION if the
//! payload length does not match Message Length.  Unknown message types
//! MUST also close the session.

mod fetch;
mod fetch_ok;
mod fetch_type;
mod filter_type;
mod go_away;
mod group_order;

mod namespace;
mod params;
mod publish;
mod publish_done;
mod publish_namespace;
mod publish_skipped;
mod publisher;
mod request_error;
mod request_ok;
mod request_update;

mod subscribe;
mod subscribe_namespace;
mod subscribe_ok;
mod subscribe_tracks;
mod subscriber;
mod track_status;

pub use fetch::*;
pub use fetch_ok::*;
pub use fetch_type::*;
pub use filter_type::*;
pub use go_away::*;
pub use group_order::*;

pub use namespace::*;
pub use params::*;
pub use publish::*;
pub use publish_done::*;
pub use publish_namespace::*;
pub use publish_skipped::*;
pub use publisher::*;
pub use request_error::*;
pub use request_ok::*;
pub use request_update::*;

pub use subscribe::*;
pub use subscribe_namespace::*;
pub use subscribe_ok::*;
pub use subscribe_tracks::*;
pub use subscriber::*;
pub use track_status::*;

use crate::coding::{Decode, DecodeError, Encode, EncodeError};
use bytes::Buf as _;
use std::fmt;

/// Streams on which a draft-19 message is permitted.
///
/// `SETUP` is represented by [`crate::setup::Setup`] rather than [`Message`],
/// so there is no control-only variant in this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessagePlacement {
    /// The message is valid only on a bidirectional request stream.
    RequestOnly,
    /// The message is valid on either the session control stream or an
    /// established request stream.
    ControlOrRequest,
}

impl MessagePlacement {
    /// Whether the message may be encoded on the session control stream.
    pub const fn allows_control(self) -> bool {
        matches!(self, Self::ControlOrRequest)
    }

    /// Whether the message may be encoded on a bidirectional request stream.
    pub const fn allows_request(self) -> bool {
        true
    }
}

// Use a macro to generate the Message enum and its encode/decode impls.
macro_rules! message_types {
    {$($name:ident = $val:expr,)*} => {
        /// Wire type IDs for control and request messages (draft-19 Table 5).
        ///
        /// These are the `u64` values used in the `Message Type` field on
        /// the wire. Use these constants instead of hardcoded hex literals
        /// when matching or constructing message frames.
        #[allow(non_upper_case_globals)]
        pub mod wire_id {
            $(pub const $name: u64 = $val;)*
        }

        /// All supported framed message types after `SETUP`.
        #[derive(Clone)]
        pub enum Message {
            $($name($name)),*
        }

        impl Decode for Message {
            fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
                let t = u64::decode(r)?;
                let len = u16::decode(r)? as usize;

                // Enforce the length field: read exactly `len` bytes as the
                // payload and decode from that slice, so a truncated or
                // overlong payload is detected immediately.
                <u64 as Decode>::decode_remaining(r, len)?;
                let mut payload = r.copy_to_bytes(len);

                let msg = match t {
                    $($val => {
                        let msg = $name::decode(&mut payload)?;
                        Ok(Self::$name(msg))
                    })*
                    _ => Err(DecodeError::InvalidMessage(t)),
                }?;

                // Any bytes left in the payload slice mean the message was
                // shorter than declared — that is a PROTOCOL_VIOLATION.
                if payload.has_remaining() {
                    return Err(DecodeError::InvalidMessage(t));
                }

                Ok(msg)
            }
        }

        impl Encode for Message {
            fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
                match self {
                    $(Self::$name(ref m) => {
                        self.id().encode(w)?;

                        let mut buf = Vec::new();
                        m.encode(&mut buf)?;
                        if buf.len() > u16::MAX as usize {
                            return Err(EncodeError::MsgBoundsExceeded);
                        }
                        (buf.len() as u16).encode(w)?;

                        Self::encode_remaining(w, buf.len())?;
                        w.put_slice(&buf);
                        Ok(())
                    },)*
                }
            }
        }

        impl Message {
            pub fn id(&self) -> u64 {
                match self {
                    $(Self::$name(_) => $val,)*
                }
            }

            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$name(_) => stringify!($name),)*
                }
            }

            /// Return the stream placement required by draft-19 Table 5.
            ///
            /// This deliberately uses an exhaustive match rather than a
            /// default so adding a new message requires an explicit protocol
            /// placement decision.
            pub const fn placement(&self) -> MessagePlacement {
                match self {
                    Self::GoAway(_) => MessagePlacement::ControlOrRequest,
                    Self::RequestUpdate(_)
                    | Self::RequestError(_)
                    | Self::RequestOk(_)
                    | Self::Subscribe(_)
                    | Self::SubscribeOk(_)
                    | Self::PublishNamespace(_)
                    | Self::Namespace(_)
                    | Self::NamespaceDone(_)
                    | Self::TrackStatus(_)
                    | Self::Publish(_)
                    | Self::PublishDone(_)
                    | Self::Fetch(_)
                    | Self::FetchOk(_)
                    | Self::PublishSkipped(_)
                    | Self::SubscribeNamespace(_)
                    | Self::SubscribeTracks(_) => MessagePlacement::RequestOnly,
                }
            }

            /// Return the request ID if this message participates in request ID sequencing.
            ///
            /// Responses and cancellation messages reference existing request IDs
            /// and therefore return `None`. This is used only for request ID
            /// sequencing validation on receive.
            pub fn sequenced_request_id(&self) -> Option<u64> {
                match self {
                    Self::Subscribe(m) => Some(m.id),
                    Self::RequestUpdate(m) => Some(m.id),
                    Self::Fetch(m) => Some(m.id),
                    Self::TrackStatus(m) => Some(m.id),
                    Self::SubscribeNamespace(m) => Some(m.id),
                    Self::SubscribeTracks(m) => Some(m.id),
                    Self::Publish(m) => Some(m.id),
                    Self::PublishNamespace(m) => Some(m.id),
                    _ => None,
                }
            }

            /// Return the target request ID for response/follow-up messages sent
            /// back on a bidi stream (draft-19). Returns `None` for request-initiating
            /// messages and session-level messages.
            pub fn response_target_id(&self) -> Option<u64> {
                match self {
                    Self::RequestOk(m) => Some(m.id),
                    Self::RequestError(m) => Some(m.id),
                    Self::SubscribeOk(m) => Some(m.id),
                    Self::PublishDone(m) => Some(m.id),
                    Self::FetchOk(m) => Some(m.id),
                    _ => None,
                }
            }
        }

        $(impl From<$name> for Message {
            fn from(m: $name) -> Self {
                Message::$name(m)
            }
        })*

        impl fmt::Debug for Message {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(Self::$name(ref m) => m.fmt(f),)*
                }
            }
        }
    }
}

// Wire IDs per draft-ietf-moq-transport-19 Table 5.
message_types! {
    // NOTE: Setup messages live in a separate module (setup::Client/Server).

    // ── Shared request responses (new in draft-16) ───────────────────────────
    RequestUpdate   = 0x2,
    RequestError    = 0x5,   // draft-16: REQUEST_ERROR
    RequestOk       = 0x7,   // draft-16: REQUEST_OK

    // ── SUBSCRIBE family ─────────────────────────────────────────────────────
    Subscribe       = 0x3,
    SubscribeOk     = 0x4,

    // ── PUBLISH_NAMESPACE family ──────────────────────────────────────────────
    PublishNamespace        = 0x6,
    Namespace               = 0x8,
    NamespaceDone           = 0xe,

    // ── TRACK_STATUS ──────────────────────────────────────────────────────────
    TrackStatus     = 0xd,

    // ── PUBLISH family ────────────────────────────────────────────────────────
    Publish         = 0x1d,
    PublishDone     = 0xb,
    // 0x1e is reserved (PUBLISH_OK in drafts <= 17). PUBLISH uses REQUEST_OK.

    // ── FETCH family ─────────────────────────────────────────────────────────
    Fetch           = 0x16,
    FetchOk         = 0x18,

    // ── Namespace and track discovery ──────────────────────────────────────────────────────────
    PublishSkipped      = 0xf,
    SubscribeNamespace = 0x50,
    SubscribeTracks    = 0x51,

    // ── Session management ────────────────────────────────────────────────────
    GoAway          = 0x10,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::coding::{
        KeyValuePairs, Location, ReasonPhrase, TrackNamespace, TrackNamespacePrefix,
    };

    fn namespace() -> TrackNamespace {
        TrackNamespace::from_utf8_path("test/ns")
    }

    pub(crate) fn request_only_messages() -> Vec<Message> {
        let namespace = namespace();
        let prefix = TrackNamespacePrefix::from_utf8_path("test/ns");
        vec![
            Message::RequestUpdate(RequestUpdate {
                id: 0,
                params: KeyValuePairs::default(),
            }),
            Message::RequestError(RequestError {
                id: 0,
                error_code: 0,
                retry_interval: 0,
                reason: ReasonPhrase(String::new()),
                redirect: None,
            }),
            Message::RequestOk(RequestOk {
                id: 0,
                params: KeyValuePairs::default(),
                track_properties: TrackProperties::default(),
            }),
            Message::Subscribe(Subscribe {
                id: 0,
                track_namespace: namespace.clone(),
                track_name: "track".into(),
                params: KeyValuePairs::default(),
            }),
            Message::SubscribeOk(SubscribeOk {
                id: 0,
                track_alias: 0,
                params: KeyValuePairs::default(),
                track_extensions: TrackExtensions::default(),
            }),
            Message::PublishNamespace(PublishNamespace {
                id: 0,
                track_namespace: namespace.clone(),
                params: KeyValuePairs::default(),
            }),
            Message::Namespace(Namespace {
                track_namespace_suffix: prefix.clone(),
            }),
            Message::NamespaceDone(NamespaceDone {
                track_namespace_suffix: prefix.clone(),
            }),
            Message::TrackStatus(TrackStatus {
                id: 0,
                track_namespace: namespace.clone(),
                track_name: "track".into(),
                params: KeyValuePairs::default(),
            }),
            Message::Publish(Publish {
                id: 0,
                track_namespace: namespace.clone(),
                track_name: "track".into(),
                track_alias: 0,
                params: KeyValuePairs::default(),
                track_extensions: TrackExtensions::default(),
            }),
            Message::PublishDone(PublishDone {
                id: 0,
                status_code: 0,
                stream_count: 0,
                reason: ReasonPhrase(String::new()),
            }),
            Message::Fetch(Fetch {
                id: 0,
                fetch_type: FetchType::Standalone,
                standalone_fetch: Some(StandaloneFetch {
                    track_namespace: namespace,
                    track_name: "track".into(),
                    start_location: Location::new(0, 0),
                    end_location: Location::new(0, 1),
                }),
                joining_fetch: None,
                params: KeyValuePairs::default(),
            }),
            Message::FetchOk(FetchOk {
                id: 0,
                end_of_track: false,
                end_location: Location::new(0, 0),
                params: KeyValuePairs::default(),
                track_extensions: TrackExtensions::default(),
            }),
            Message::PublishSkipped(PublishSkipped {
                track_namespace_suffix: prefix.clone(),
                track_name: "track".into(),
            }),
            Message::SubscribeNamespace(SubscribeNamespace {
                id: 0,
                track_namespace_prefix: prefix.clone(),
                params: KeyValuePairs::default(),
            }),
            Message::SubscribeTracks(SubscribeTracks {
                id: 0,
                track_namespace_prefix: prefix,
                params: KeyValuePairs::default(),
            }),
        ]
    }

    #[test]
    fn draft19_stream_placement_is_exhaustive() {
        let request_only = request_only_messages();
        assert_eq!(request_only.len(), 16);
        for message in request_only {
            assert_eq!(message.placement(), MessagePlacement::RequestOnly);
            assert!(!message.placement().allows_control());
            assert!(message.placement().allows_request());
        }

        let goaway = Message::GoAway(GoAway {
            uri: crate::coding::SessionUri(String::new()),
            timeout: 0,
        });
        assert_eq!(goaway.placement(), MessagePlacement::ControlOrRequest);
        assert!(goaway.placement().allows_control());
        assert!(goaway.placement().allows_request());
    }

    fn assert_sequenced(msg: Message, id: u64) {
        assert_eq!(msg.sequenced_request_id(), Some(id));
    }

    fn assert_not_sequenced(msg: Message) {
        assert_eq!(msg.sequenced_request_id(), None);
    }

    #[test]
    fn sequenced_request_id_covers_all_request_start_messages() {
        assert_sequenced(
            Message::Subscribe(Subscribe {
                id: 0,
                track_namespace: namespace(),
                track_name: "track".into(),
                params: KeyValuePairs::default(),
            }),
            0,
        );

        assert_sequenced(
            Message::RequestUpdate(RequestUpdate {
                id: 2,
                params: KeyValuePairs::default(),
            }),
            2,
        );

        assert_sequenced(
            Message::Fetch(Fetch {
                id: 4,
                fetch_type: FetchType::Standalone,
                standalone_fetch: Some(StandaloneFetch {
                    track_namespace: namespace(),
                    track_name: "track".into(),
                    start_location: Location::new(0, 0),
                    end_location: Location::new(0, 1),
                }),
                joining_fetch: None,
                params: KeyValuePairs::default(),
            }),
            4,
        );

        assert_sequenced(
            Message::TrackStatus(TrackStatus {
                id: 6,
                track_namespace: namespace(),
                track_name: "track".into(),
                params: KeyValuePairs::default(),
            }),
            6,
        );

        assert_sequenced(
            Message::SubscribeNamespace(SubscribeNamespace {
                id: 8,
                track_namespace_prefix: TrackNamespacePrefix::from_utf8_path("test/ns"),
                params: KeyValuePairs::default(),
            }),
            8,
        );

        assert_sequenced(
            Message::SubscribeTracks(SubscribeTracks {
                id: 10,
                track_namespace_prefix: TrackNamespacePrefix::from_utf8_path("test/ns"),
                params: KeyValuePairs::default(),
            }),
            10,
        );

        assert_sequenced(
            Message::Publish(Publish {
                id: 12,
                track_namespace: namespace(),
                track_name: "track".into(),
                track_alias: 1,
                params: KeyValuePairs::default(),
                track_extensions: TrackExtensions::default(),
            }),
            12,
        );

        assert_sequenced(
            Message::PublishNamespace(PublishNamespace {
                id: 14,
                track_namespace: namespace(),
                params: KeyValuePairs::default(),
            }),
            14,
        );
    }

    #[test]
    fn sequenced_request_id_ignores_messages_that_reference_existing_requests() {
        assert_not_sequenced(Message::RequestOk(RequestOk {
            id: 0,
            params: KeyValuePairs::default(),
            track_properties: TrackProperties::default(),
        }));

        assert_not_sequenced(Message::RequestError(RequestError {
            id: 0,
            error_code: 0,
            retry_interval: 0,
            reason: ReasonPhrase(String::new()),
            redirect: None,
        }));

        assert_not_sequenced(Message::SubscribeOk(SubscribeOk {
            id: 0,
            track_alias: 1,
            params: KeyValuePairs::default(),
            track_extensions: TrackExtensions::default(),
        }));

        assert_not_sequenced(Message::FetchOk(FetchOk {
            id: 0,
            end_of_track: false,
            end_location: Location::new(0, 0),
            params: KeyValuePairs::default(),
            track_extensions: TrackExtensions::default(),
        }));

        assert_not_sequenced(Message::PublishDone(PublishDone {
            id: 0,
            status_code: 0,
            stream_count: 0,
            reason: ReasonPhrase(String::new()),
        }));
    }

    #[test]
    fn decode_rejects_legacy_stub_message_type() {
        let mut buf = bytes::BytesMut::new();
        0x100u64.encode(&mut buf).unwrap();
        0u16.encode(&mut buf).unwrap();

        let err = Message::decode(&mut buf).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidMessage(0x100)));
    }

    #[test]
    fn draft19_wire_layouts_for_changed_control_messages() {
        fn encoded(msg: Message) -> Vec<u8> {
            let mut buf = bytes::BytesMut::new();
            msg.encode(&mut buf).unwrap();
            buf.to_vec()
        }

        let ns = TrackNamespace::from_utf8_path("ns");
        let prefix = TrackNamespacePrefix::new();

        assert_eq!(
            encoded(Message::Subscribe(Subscribe {
                id: 0,
                track_namespace: ns.clone(),
                track_name: "t".into(),
                params: KeyValuePairs::default(),
            })),
            vec![0x03, 0x00, 0x08, 0x00, 0x01, 0x02, b'n', b's', 0x01, b't', 0x00]
        );

        assert_eq!(
            encoded(Message::SubscribeOk(SubscribeOk {
                id: 0,
                track_alias: 1,
                params: KeyValuePairs::default(),
                track_extensions: TrackExtensions::default(),
            })),
            vec![0x04, 0x00, 0x03, 0x00, 0x01, 0x00]
        );

        assert_eq!(
            encoded(Message::TrackStatus(TrackStatus {
                id: 0,
                track_namespace: ns.clone(),
                track_name: "t".into(),
                params: KeyValuePairs::default(),
            })),
            vec![0x0d, 0x00, 0x08, 0x00, 0x01, 0x02, b'n', b's', 0x01, b't', 0x00]
        );

        assert_eq!(
            encoded(Message::Publish(Publish {
                id: 0,
                track_namespace: ns.clone(),
                track_name: "t".into(),
                track_alias: 5,
                params: KeyValuePairs::default(),
                track_extensions: TrackExtensions::default(),
            })),
            vec![0x1d, 0x00, 0x09, 0x00, 0x01, 0x02, b'n', b's', 0x01, b't', 0x05, 0x00]
        );

        assert_eq!(
            encoded(Message::Fetch(Fetch {
                id: 0,
                fetch_type: FetchType::Standalone,
                standalone_fetch: Some(StandaloneFetch {
                    track_namespace: ns,
                    track_name: "t".into(),
                    start_location: Location::new(0, 0),
                    end_location: Location::new(0, 1),
                }),
                joining_fetch: None,
                params: KeyValuePairs::default(),
            })),
            vec![
                0x16, 0x00, 0x0d, 0x00, 0x01, 0x01, 0x02, b'n', b's', 0x01, b't', 0x00, 0x00, 0x00,
                0x01, 0x00
            ]
        );

        assert_eq!(
            encoded(Message::FetchOk(FetchOk {
                id: 0,
                end_of_track: false,
                end_location: Location::new(0, 1),
                params: KeyValuePairs::default(),
                track_extensions: TrackExtensions::default(),
            })),
            vec![0x18, 0x00, 0x05, 0x00, 0x00, 0x00, 0x01, 0x00]
        );

        assert_eq!(
            encoded(Message::SubscribeNamespace(SubscribeNamespace {
                id: 0,
                track_namespace_prefix: prefix.clone(),
                params: KeyValuePairs::default(),
            })),
            vec![0x50, 0x00, 0x03, 0x00, 0x00, 0x00]
        );

        assert_eq!(
            encoded(Message::SubscribeTracks(SubscribeTracks {
                id: 2,
                track_namespace_prefix: prefix,
                params: KeyValuePairs::default(),
            })),
            vec![0x51, 0x00, 0x03, 0x02, 0x00, 0x00]
        );

        assert_eq!(
            encoded(Message::PublishSkipped(PublishSkipped {
                track_namespace_suffix: TrackNamespacePrefix::from_utf8_path("west"),
                track_name: "main".into(),
            })),
            vec![
                0x0f, 0x00, 0x0b, 0x01, 0x04, b'w', b'e', b's', b't', 0x04, b'm', b'a', b'i', b'n'
            ]
        );

        assert_eq!(
            encoded(Message::GoAway(GoAway {
                uri: crate::coding::SessionUri(String::new()),
                timeout: 300,
            })),
            vec![0x10, 0x00, 0x03, 0x00, 0x81, 0x2c]
        );
    }

    #[test]
    fn draft19_rejects_reserved_publish_ok_type() {
        let mut buf = bytes::BytesMut::from(&[0x1e, 0x00, 0x00][..]);
        assert!(matches!(
            Message::decode(&mut buf).unwrap_err(),
            DecodeError::InvalidMessage(0x1e)
        ));
    }

    #[test]
    fn draft19_rejects_removed_cancellation_message_types() {
        for msg_type in [0x09_u8, 0x0a, 0x0c, 0x17] {
            let mut buf = bytes::BytesMut::from(&[msg_type, 0x00, 0x00][..]);
            assert!(matches!(
                Message::decode(&mut buf).unwrap_err(),
                DecodeError::InvalidMessage(id) if id == u64::from(msg_type)
            ));
        }
    }

    #[test]
    fn draft19_rejects_goaway_without_timeout() {
        // Legacy shape: GOAWAY with only a zero-length URI.
        let mut buf = bytes::BytesMut::from(&[0x10, 0x00, 0x01, 0x00][..]);
        assert!(matches!(
            Message::decode(&mut buf).unwrap_err(),
            DecodeError::More(1)
        ));
    }
}
