// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::coding::{Decode, DecodeError, Encode, EncodeError};
use crate::data::{ExtensionHeaders, ObjectStatus, PublisherPriority, DEFAULT_PUBLISHER_PRIORITY};

const DATAGRAM_PROPERTIES: u64 = 0x01;
const DATAGRAM_END_OF_GROUP: u64 = 0x02;
const DATAGRAM_ZERO_OBJECT_ID: u64 = 0x04;
const DATAGRAM_DEFAULT_PRIORITY: u64 = 0x08;
const DATAGRAM_STATUS: u64 = 0x20;
const OBJECT_DATAGRAM_TYPE_MASK: u64 = 0x2f;
const PADDING_DATAGRAM_TYPE: u64 = 0x132b_3e29;

/// A validated draft-19 datagram type.
///
/// The object datagram type is a bitfield with 24 valid combinations. The
/// legacy associated names are retained so existing `Datagram` initializers do
/// not need to change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct DatagramType(u64);

#[allow(non_upper_case_globals)]
impl DatagramType {
    pub const ObjectIdPayload: Self = Self(0x00);
    pub const ObjectIdPayloadExt: Self = Self(0x01);
    pub const ObjectIdPayloadEndOfGroup: Self = Self(0x02);
    pub const ObjectIdPayloadExtEndOfGroup: Self = Self(0x03);
    pub const Payload: Self = Self(0x04);
    pub const PayloadExt: Self = Self(0x05);
    pub const PayloadEndOfGroup: Self = Self(0x06);
    pub const PayloadExtEndOfGroup: Self = Self(0x07);
    pub const ObjectIdPayloadDefaultPriority: Self = Self(0x08);
    pub const ObjectIdPayloadExtDefaultPriority: Self = Self(0x09);
    pub const ObjectIdPayloadEndOfGroupDefaultPriority: Self = Self(0x0a);
    pub const ObjectIdPayloadExtEndOfGroupDefaultPriority: Self = Self(0x0b);
    pub const PayloadDefaultPriority: Self = Self(0x0c);
    pub const PayloadExtDefaultPriority: Self = Self(0x0d);
    pub const PayloadEndOfGroupDefaultPriority: Self = Self(0x0e);
    pub const PayloadExtEndOfGroupDefaultPriority: Self = Self(0x0f);
    pub const ObjectIdStatus: Self = Self(0x20);
    pub const ObjectIdStatusExt: Self = Self(0x21);
    pub const Status: Self = Self(0x24);
    pub const StatusExt: Self = Self(0x25);
    pub const ObjectIdStatusDefaultPriority: Self = Self(0x28);
    pub const ObjectIdStatusExtDefaultPriority: Self = Self(0x29);
    pub const StatusDefaultPriority: Self = Self(0x2c);
    pub const StatusExtDefaultPriority: Self = Self(0x2d);
    pub const Padding: Self = Self(PADDING_DATAGRAM_TYPE);

    pub const fn value(self) -> u64 {
        self.0
    }

    pub const fn from_value(value: u64) -> Option<Self> {
        if value == PADDING_DATAGRAM_TYPE || Self::is_valid_object_value(value) {
            Some(Self(value))
        } else {
            None
        }
    }

    const fn is_valid_object_value(value: u64) -> bool {
        value & !OBJECT_DATAGRAM_TYPE_MASK == 0
            && !(value & DATAGRAM_STATUS != 0 && value & DATAGRAM_END_OF_GROUP != 0)
    }

    pub const fn is_object(self) -> bool {
        Self::is_valid_object_value(self.0)
    }

    pub const fn is_padding(self) -> bool {
        self.0 == PADDING_DATAGRAM_TYPE
    }

    pub const fn has_properties(self) -> bool {
        self.is_object() && self.0 & DATAGRAM_PROPERTIES != 0
    }

    /// Legacy name retained while Object Properties replace extension headers
    /// in the public API.
    pub const fn has_extension_headers(self) -> bool {
        self.has_properties()
    }

    pub const fn is_end_of_group(self) -> bool {
        self.is_object() && self.0 & DATAGRAM_END_OF_GROUP != 0
    }

    pub const fn has_object_id(self) -> bool {
        self.is_object() && self.0 & DATAGRAM_ZERO_OBJECT_ID == 0
    }

    pub const fn uses_default_priority(self) -> bool {
        self.is_object() && self.0 & DATAGRAM_DEFAULT_PRIORITY != 0
    }

    pub const fn has_status(self) -> bool {
        self.is_object() && self.0 & DATAGRAM_STATUS != 0
    }
}

impl Decode for DatagramType {
    fn decode<B: bytes::Buf>(r: &mut B) -> Result<Self, DecodeError> {
        Self::from_value(u64::decode(r)?).ok_or(DecodeError::InvalidDatagramType)
    }
}

impl Encode for DatagramType {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.value().encode(w)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Datagram {
    /// The type of this datagram object
    pub datagram_type: DatagramType,

    /// The track alias.
    pub track_alias: u64,

    /// The sequence number within the track.
    pub group_id: u64,

    /// The object ID within the group.
    pub object_id: Option<u64>,

    /// Publisher priority, where **smaller** values are sent first.
    ///
    /// This is 128 after decoding when `DEFAULT_PRIORITY` is set and no Track
    /// property has yet been applied. [`Self::priority`] distinguishes
    /// inheritance from an explicitly encoded priority of 128.
    pub publisher_priority: u8,

    /// Optional extension headers if type is 0x1 (NoEndOfGroupWithExtensions) or 0x3 (EndofGroupWithExtensions)
    pub extension_headers: Option<ExtensionHeaders>,

    /// The Object Status.
    pub status: Option<ObjectStatus>,

    /// The payload.
    pub payload: Option<bytes::Bytes>,
}

impl Decode for Datagram {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let datagram_type = DatagramType::decode(r)?;

        if datagram_type.is_padding() {
            let payload = r.copy_to_bytes(r.remaining());
            if payload.iter().any(|byte| *byte != 0) {
                return Err(DecodeError::InvalidValue);
            }
            return Ok(Self {
                datagram_type,
                track_alias: 0,
                group_id: 0,
                object_id: None,
                publisher_priority: 0,
                extension_headers: None,
                status: None,
                payload: Some(payload),
            });
        }

        let track_alias = u64::decode(r)?;
        let group_id = u64::decode(r)?;

        let object_id = if datagram_type.has_object_id() {
            Some(u64::decode(r)?)
        } else {
            None
        };

        let publisher_priority = if datagram_type.uses_default_priority() {
            DEFAULT_PUBLISHER_PRIORITY
        } else {
            u8::decode(r)?
        };

        let extension_headers = if datagram_type.has_extension_headers() {
            let headers = ExtensionHeaders::decode(r)?;
            if headers.is_empty() {
                return Err(DecodeError::InvalidValue);
            }
            Some(headers)
        } else {
            None
        };

        let status = if datagram_type.has_status() {
            Some(ObjectStatus::decode(r)?)
        } else {
            None
        };

        if status.is_some_and(|status| !status.allows_properties()) && extension_headers.is_some() {
            return Err(DecodeError::InvalidValue);
        }

        let payload = if datagram_type.has_status() {
            if r.has_remaining() {
                return Err(DecodeError::InvalidValue);
            }
            None
        } else {
            let payload = r.copy_to_bytes(r.remaining());
            // A zero-length Normal Object must explicitly encode Normal status.
            if payload.is_empty() {
                return Err(DecodeError::InvalidValue);
            }
            Some(payload)
        };

        Ok(Self {
            datagram_type,
            track_alias,
            group_id,
            object_id,
            publisher_priority,
            extension_headers,
            status,
            payload,
        })
    }
}

impl Encode for Datagram {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.validate()?;
        self.datagram_type.encode(w)?;

        if self.datagram_type.is_padding() {
            let payload = self
                .payload
                .as_ref()
                .ok_or_else(|| EncodeError::MissingField("PaddingData".to_string()))?;
            Self::encode_remaining(w, payload.len())?;
            w.put_slice(payload);
            return Ok(());
        }

        self.track_alias.encode(w)?;
        self.group_id.encode(w)?;

        if let Some(object_id) = self.object_id {
            object_id.encode(w)?;
        }

        if !self.datagram_type.uses_default_priority() {
            self.publisher_priority.encode(w)?;
        }

        if let Some(extension_headers) = &self.extension_headers {
            extension_headers.encode(w)?;
        }

        if let Some(status) = self.status {
            status.encode(w)?;
        }

        if let Some(payload) = &self.payload {
            Self::encode_remaining(w, payload.len())?;
            w.put_slice(payload);
        }

        Ok(())
    }
}

impl Datagram {
    /// Return whether priority is explicit or inherited from the Track.
    pub const fn priority(&self) -> PublisherPriority {
        if self.datagram_type.uses_default_priority() {
            PublisherPriority::Inherited
        } else {
            PublisherPriority::Explicit(self.publisher_priority)
        }
    }

    /// Resolve the effective priority with the Track default when available.
    pub const fn effective_priority(&self, track_default: Option<u8>) -> u8 {
        self.priority().resolve(track_default)
    }

    fn validate(&self) -> Result<(), EncodeError> {
        if self.datagram_type.is_padding() {
            let payload = self
                .payload
                .as_ref()
                .ok_or_else(|| EncodeError::MissingField("PaddingData".to_string()))?;
            if self.track_alias != 0
                || self.group_id != 0
                || self.object_id.is_some()
                || self.publisher_priority != 0
                || self.extension_headers.is_some()
                || self.status.is_some()
                || payload.iter().any(|byte| *byte != 0)
            {
                return Err(EncodeError::InvalidValue);
            }
            return Ok(());
        }

        if !self.datagram_type.is_object() {
            return Err(EncodeError::InvalidValue);
        }
        if self.datagram_type.uses_default_priority()
            && self.publisher_priority != DEFAULT_PUBLISHER_PRIORITY
        {
            return Err(EncodeError::InvalidValue);
        }

        match (self.datagram_type.has_object_id(), self.object_id) {
            (true, None) => return Err(EncodeError::MissingField("ObjectId".to_string())),
            (false, Some(_)) => return Err(EncodeError::InvalidValue),
            _ => {}
        }

        match (
            self.datagram_type.has_properties(),
            self.extension_headers.as_ref(),
        ) {
            (true, None) => return Err(EncodeError::MissingField("ExtensionHeaders".to_string())),
            (true, Some(properties)) if properties.is_empty() => {
                return Err(EncodeError::InvalidValue)
            }
            (false, Some(_)) => return Err(EncodeError::InvalidValue),
            _ => {}
        }

        if self.datagram_type.has_status() {
            let status = self
                .status
                .ok_or_else(|| EncodeError::MissingField("Status".to_string()))?;
            if self.payload.is_some()
                || (self.extension_headers.is_some() && !status.allows_properties())
            {
                return Err(EncodeError::InvalidValue);
            }
        } else {
            if self.status.is_some() {
                return Err(EncodeError::InvalidValue);
            }
            let payload = self
                .payload
                .as_ref()
                .ok_or_else(|| EncodeError::MissingField("Payload".to_string()))?;
            if payload.is_empty() {
                return Err(EncodeError::InvalidValue);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use bytes::BytesMut;

    #[test]
    fn encode_decode_datagram_type() {
        let mut buf = BytesMut::new();

        let dt = DatagramType::ObjectIdPayload;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x00]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::ObjectIdPayloadExt;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x01]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::ObjectIdPayloadEndOfGroup;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x02]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::ObjectIdPayloadExtEndOfGroup;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x03]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::Payload;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x04]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::PayloadExt;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x05]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::PayloadEndOfGroup;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x06]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::PayloadExtEndOfGroup;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x07]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::ObjectIdStatus;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x20]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);

        let dt = DatagramType::ObjectIdStatusExt;
        dt.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x21]);
        let decoded = DatagramType::decode(&mut buf).unwrap();
        assert_eq!(decoded, dt);
    }

    #[test]
    fn draft19_accepts_all_24_object_datagram_bitfields() {
        let valid = (0_u64..=0x2f)
            .filter_map(DatagramType::from_value)
            .filter(|datagram_type| datagram_type.is_object())
            .collect::<Vec<_>>();
        assert_eq!(valid.len(), 24);

        for datagram_type in valid {
            let mut wire = BytesMut::new();
            datagram_type.encode(&mut wire).unwrap();
            assert_eq!(DatagramType::decode(&mut wire).unwrap(), datagram_type);
            assert!(wire.is_empty());
        }
    }

    #[test]
    fn draft19_rejects_reserved_object_datagram_bitfields() {
        for value in [
            0x10_u64, 0x1f, 0x22, 0x23, 0x26, 0x27, 0x2a, 0x2b, 0x2e, 0x2f, 0x30, 0x40,
        ] {
            let mut wire = BytesMut::new();
            value.encode(&mut wire).unwrap();
            assert!(matches!(
                DatagramType::decode(&mut wire),
                Err(DecodeError::InvalidDatagramType)
            ));
        }
    }

    #[test]
    fn draft19_default_priority_properties_golden_vector() {
        let mut properties = ExtensionHeaders::new();
        properties.set_intvalue(0, 1);
        let datagram = Datagram {
            datagram_type: DatagramType::PayloadExtDefaultPriority,
            track_alias: 2,
            group_id: 3,
            object_id: None,
            publisher_priority: DEFAULT_PUBLISHER_PRIORITY,
            extension_headers: Some(properties),
            status: None,
            payload: Some(Bytes::from_static(b"abc")),
        };

        let mut wire = BytesMut::new();
        datagram.encode(&mut wire).unwrap();
        assert_eq!(
            wire.as_ref(),
            &[0x0d, 0x02, 0x03, 0x02, 0x00, 0x01, b'a', b'b', b'c']
        );

        let decoded = Datagram::decode(&mut wire).unwrap();
        assert_eq!(decoded, datagram);
        assert!(decoded.datagram_type.uses_default_priority());
        assert_eq!(decoded.priority(), PublisherPriority::Inherited);
        assert_eq!(decoded.effective_priority(None), DEFAULT_PUBLISHER_PRIORITY);
        assert_eq!(decoded.effective_priority(Some(37)), 37);
        assert!(!decoded.datagram_type.has_object_id());
        assert!(wire.is_empty());
    }

    #[test]
    fn draft19_status_properties_golden_vector() {
        let mut properties = ExtensionHeaders::new();
        properties.set_intvalue(0, 1);
        let datagram = Datagram {
            datagram_type: DatagramType::ObjectIdStatusExtDefaultPriority,
            track_alias: 2,
            group_id: 3,
            object_id: Some(4),
            publisher_priority: DEFAULT_PUBLISHER_PRIORITY,
            extension_headers: Some(properties),
            status: Some(ObjectStatus::NormalObject),
            payload: None,
        };

        let mut wire = BytesMut::new();
        datagram.encode(&mut wire).unwrap();
        assert_eq!(
            wire.as_ref(),
            &[0x29, 0x02, 0x03, 0x04, 0x02, 0x00, 0x01, 0x00]
        );
        assert_eq!(Datagram::decode(&mut wire).unwrap(), datagram);
        assert!(wire.is_empty());
    }

    #[test]
    fn draft19_padding_datagram_golden_and_negative_vectors() {
        let datagram = Datagram {
            datagram_type: DatagramType::Padding,
            track_alias: 0,
            group_id: 0,
            object_id: None,
            publisher_priority: 0,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from_static(&[0, 0, 0])),
        };
        let mut wire = BytesMut::new();
        datagram.encode(&mut wire).unwrap();
        assert_eq!(wire.as_ref(), &[0xf0, 0x13, 0x2b, 0x3e, 0x29, 0, 0, 0]);
        assert_eq!(Datagram::decode(&mut wire).unwrap(), datagram);

        let mut malformed = Bytes::from_static(&[0xf0, 0x13, 0x2b, 0x3e, 0x29, 0, 1]);
        assert!(matches!(
            Datagram::decode(&mut malformed),
            Err(DecodeError::InvalidValue)
        ));
    }

    #[test]
    fn draft19_rejects_zero_payload_and_status_trailing_bytes() {
        let mut zero_payload = Bytes::from_static(&[
            0x00, // object payload
            0x01, // alias
            0x01, // group
            0x01, // object
            0x01, // priority
        ]);
        assert!(matches!(
            Datagram::decode(&mut zero_payload),
            Err(DecodeError::InvalidValue)
        ));

        let mut status_with_payload = Bytes::from_static(&[
            0x20, // object status
            0x01, // alias
            0x01, // group
            0x01, // object
            0x01, // priority
            0x00, // Normal status
            0xff, // forbidden payload/trailing byte
        ]);
        assert!(matches!(
            Datagram::decode(&mut status_with_payload),
            Err(DecodeError::InvalidValue)
        ));
    }

    #[test]
    fn draft19_encode_rejects_fields_absent_from_selected_type() {
        let zero_object_type_with_id = Datagram {
            datagram_type: DatagramType::Payload,
            track_alias: 1,
            group_id: 1,
            object_id: Some(0),
            publisher_priority: 1,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from_static(b"x")),
        };
        assert!(matches!(
            zero_object_type_with_id.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));

        let payload_type_with_status = Datagram {
            datagram_type: DatagramType::ObjectIdPayload,
            track_alias: 1,
            group_id: 1,
            object_id: Some(1),
            publisher_priority: 1,
            extension_headers: None,
            status: Some(ObjectStatus::NormalObject),
            payload: Some(Bytes::from_static(b"x")),
        };
        assert!(matches!(
            payload_type_with_status.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));

        let status_type_with_payload = Datagram {
            datagram_type: DatagramType::ObjectIdStatus,
            track_alias: 1,
            group_id: 1,
            object_id: Some(1),
            publisher_priority: 1,
            extension_headers: None,
            status: Some(ObjectStatus::NormalObject),
            payload: Some(Bytes::from_static(b"x")),
        };
        assert!(matches!(
            status_type_with_payload.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));
    }

    #[test]
    fn default_priority_encoding_rejects_a_misleading_placeholder() {
        let datagram = Datagram {
            datagram_type: DatagramType::PayloadDefaultPriority,
            track_alias: 1,
            group_id: 1,
            object_id: None,
            publisher_priority: 0,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from_static(b"x")),
        };
        assert!(matches!(
            datagram.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));
    }

    #[test]
    fn encode_decode_datagram() {
        let mut buf = BytesMut::new();

        // One ExtensionHeader for testing
        let mut ext_hdrs = ExtensionHeaders::new();
        ext_hdrs.set_bytesvalue(123, vec![0x00, 0x01, 0x02, 0x03]);

        // DatagramType = ObjectIdPayload
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayload,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Type(1)+Alias(1)+GroupId(1)+ObjectId(2)+Priority(1)+Payload(7) = 13
        assert_eq!(13, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = ObjectIdPayloadExt
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayloadExt,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: Some(ext_hdrs.clone()),
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Same as above plus NumExt(1),ExtensionKey(1),ExtensionValueLen(1),ExtensionValue(4) = 13 + 7 = 20
        assert_eq!(20, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = ObjectIdPayloadEndOfGroup
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayloadEndOfGroup,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Type(1)+Alias(1)+GroupId(1)+ObjectId(2)+Priority(1)+Payload(7) = 13
        assert_eq!(13, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = ObjectIdPayloadExtEndOfGroup
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayloadExtEndOfGroup,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: Some(ext_hdrs.clone()),
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Same as above plus NumExt(1),ExtensionKey(1),ExtensionValueLen(1),ExtensionValue(4) = 13 + 7 = 20
        assert_eq!(20, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = ObjectIdStatus
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdStatus,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: None,
            status: Some(ObjectStatus::NormalObject),
            payload: None,
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Type(1)+Alias(1)+GroupId(1)+ObjectId(2)+Priority(1)+Status(1) = 7
        assert_eq!(7, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = ObjectIdStatusExt
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdStatusExt,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: Some(ext_hdrs.clone()),
            status: Some(ObjectStatus::NormalObject),
            payload: None,
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Same as above plus NumExt(1),ExtensionKey(1),ExtensionValueLen(1),ExtensionValue(4) = 7 + 7 = 14
        assert_eq!(14, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = Payload
        let msg = Datagram {
            datagram_type: DatagramType::Payload,
            track_alias: 12,
            group_id: 10,
            object_id: None,
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Type(1)+Alias(1)+GroupId(1)+Priority(1)+Payload(7) = 11
        assert_eq!(11, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = PayloadExt
        let msg = Datagram {
            datagram_type: DatagramType::PayloadExt,
            track_alias: 12,
            group_id: 10,
            object_id: None,
            publisher_priority: 127,
            extension_headers: Some(ext_hdrs.clone()),
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Same as above plus NumExt(1),ExtensionKey(1),ExtensionValueLen(1),ExtensionValue(4) = 11 + 7 = 18
        assert_eq!(18, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = PayloadEndOfGroup
        let msg = Datagram {
            datagram_type: DatagramType::PayloadEndOfGroup,
            track_alias: 12,
            group_id: 10,
            object_id: None,
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Type(1)+Alias(1)+GroupId(1)+Priority(1)+Payload(7) = 11
        assert_eq!(11, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);

        // DatagramType = PayloadExtEndOfGroup
        let msg = Datagram {
            datagram_type: DatagramType::PayloadExtEndOfGroup,
            track_alias: 12,
            group_id: 10,
            object_id: None,
            publisher_priority: 127,
            extension_headers: Some(ext_hdrs.clone()),
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        msg.encode(&mut buf).unwrap();
        // Length should be: Same as above plus NumExt(1),ExtensionKey(1),ExtensionValueLen(1),ExtensionValue(4) = 11 + 7 = 18
        assert_eq!(18, buf.len());
        let decoded = Datagram::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn encode_datagram_missing_fields() {
        let mut buf = BytesMut::new();

        // DatagramType = ObjectIdPayloadExt - missing extensions
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayloadExt,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        let encoded = msg.encode(&mut buf);
        assert!(matches!(encoded.unwrap_err(), EncodeError::MissingField(_)));

        // DatagramType = ObjectIdPayloadExtEndOfGroup - missing extensions
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayloadExtEndOfGroup,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: Some(Bytes::from("payload")),
        };
        let encoded = msg.encode(&mut buf);
        assert!(matches!(encoded.unwrap_err(), EncodeError::MissingField(_)));

        // DatagramType = ObjectIdPayloadExtEndOfGroup - missing extensions
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayloadExtEndOfGroup,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: None,
            status: Some(ObjectStatus::EndOfTrack),
            payload: None,
        };
        let encoded = msg.encode(&mut buf);
        assert!(matches!(encoded.unwrap_err(), EncodeError::MissingField(_)));

        // DatagramType = Payload - missing payload
        let msg = Datagram {
            datagram_type: DatagramType::Payload,
            track_alias: 12,
            group_id: 10,
            object_id: None,
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: None,
        };
        let encoded = msg.encode(&mut buf);
        assert!(matches!(encoded.unwrap_err(), EncodeError::MissingField(_)));

        // DatagramType = ObjectIdStatus - missing status
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdStatus,
            track_alias: 12,
            group_id: 10,
            object_id: Some(1234),
            publisher_priority: 127,
            extension_headers: None,
            status: None,
            payload: None,
        };
        let encoded = msg.encode(&mut buf);
        assert!(matches!(encoded.unwrap_err(), EncodeError::MissingField(_)));

        // TODO SLG - add tests
    }

    #[test]
    fn decode_rejects_extension_bit_with_zero_length() {
        let data = vec![
            0x01, // ObjectIdPayloadExt
            0x01, // track alias
            0x01, // group id
            0x01, // object id
            0x7f, // publisher priority
            0x00, // extension headers length
        ];
        let mut buf: Bytes = data.into();

        assert!(matches!(
            Datagram::decode(&mut buf).unwrap_err(),
            DecodeError::InvalidValue
        ));
    }

    #[test]
    fn encode_rejects_extension_bit_with_empty_headers() {
        let mut buf = BytesMut::new();
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdPayloadExt,
            track_alias: 1,
            group_id: 1,
            object_id: Some(1),
            publisher_priority: 1,
            extension_headers: Some(ExtensionHeaders::default()),
            status: None,
            payload: Some(Bytes::new()),
        };

        assert!(matches!(
            msg.encode(&mut buf).unwrap_err(),
            EncodeError::InvalidValue
        ));
    }

    #[test]
    fn decode_rejects_non_normal_status_with_extension_headers() {
        let data = vec![
            0x21, // ObjectIdStatusExt
            0x01, // track alias
            0x01, // group id
            0x01, // object id
            0x7f, // publisher priority
            0x02, // extension headers byte length
            0x00, // extension delta type
            0x01, // extension value
            0x04, // EndOfTrack
        ];
        let mut buf: Bytes = data.into();

        assert!(matches!(
            Datagram::decode(&mut buf).unwrap_err(),
            DecodeError::InvalidValue
        ));
    }

    #[test]
    fn encode_rejects_non_normal_status_with_extension_headers() {
        let mut ext_hdrs = ExtensionHeaders::new();
        ext_hdrs.set_intvalue(0, 1);
        let mut buf = BytesMut::new();
        let msg = Datagram {
            datagram_type: DatagramType::ObjectIdStatusExt,
            track_alias: 1,
            group_id: 1,
            object_id: Some(1),
            publisher_priority: 1,
            extension_headers: Some(ext_hdrs),
            status: Some(ObjectStatus::EndOfTrack),
            payload: None,
        };

        assert!(matches!(
            msg.encode(&mut buf).unwrap_err(),
            EncodeError::InvalidValue
        ));
    }
}
