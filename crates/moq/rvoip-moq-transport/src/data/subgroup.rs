// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::coding::{Decode, DecodeError, Encode, EncodeError};
use crate::data::{
    decode_payload_length, encode_payload_length, ExtensionHeaders, ObjectStatus,
    PublisherPriority, StreamHeaderType, SubgroupIdMode, DEFAULT_PUBLISHER_PRIORITY,
};

/// How a subgroup header determines its effective Subgroup ID.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SubgroupIdReference {
    Zero,
    FirstObject,
    Explicit(u64),
}

impl SubgroupIdReference {
    /// Resolve the effective ID. `first_object_id` is required only for a
    /// `FIRST_OBJECT` reference.
    pub const fn resolve(self, first_object_id: Option<u64>) -> Option<u64> {
        match self {
            Self::Zero => Some(0),
            Self::FirstObject => first_object_id,
            Self::Explicit(subgroup_id) => Some(subgroup_id),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SubgroupHeader {
    /// Subgroup Header Type
    pub header_type: StreamHeaderType,

    /// The track alias.
    pub track_alias: u64,

    /// The group sequence number
    pub group_id: u64,

    /// The subgroup sequence number
    pub subgroup_id: Option<u64>,

    /// Publisher priority, where **smaller** values are sent first.
    ///
    /// This is 128 after decoding when the header's `DEFAULT_PRIORITY` bit is
    /// set and no Track property has yet been applied. Callers must use
    /// [`Self::priority`] to distinguish inheritance from an explicit 128.
    pub publisher_priority: u8,
}

// Note:  Not using the Decode trait, since we need to know the header_type to properly parse this, and it
//        is read before knowing we need to decode this.
impl SubgroupHeader {
    /// Return an unambiguous description of how the Subgroup ID is derived.
    pub fn subgroup_id_reference(&self) -> Result<SubgroupIdReference, DecodeError> {
        match self.header_type.subgroup_id_mode() {
            Some(SubgroupIdMode::Zero) if self.subgroup_id.is_none() => {
                Ok(SubgroupIdReference::Zero)
            }
            Some(SubgroupIdMode::FirstObject) => Ok(SubgroupIdReference::FirstObject),
            Some(SubgroupIdMode::Explicit) => self
                .subgroup_id
                .map(SubgroupIdReference::Explicit)
                .ok_or(DecodeError::InvalidValue),
            _ => Err(DecodeError::InvalidValue),
        }
    }

    /// Return the effective Subgroup ID when it is already resolvable.
    ///
    /// A decoded `FIRST_OBJECT` header returns `None` until the receiver has
    /// decoded the first object and recorded its ID in `subgroup_id`. This
    /// makes an unresolved reference impossible to mistake for subgroup zero.
    pub fn resolved_subgroup_id(&self) -> Result<Option<u64>, DecodeError> {
        match self.subgroup_id_reference()? {
            SubgroupIdReference::Zero => Ok(Some(0)),
            SubgroupIdReference::FirstObject => Ok(self.subgroup_id),
            SubgroupIdReference::Explicit(subgroup_id) => Ok(Some(subgroup_id)),
        }
    }

    /// Return whether priority is explicit or inherited from the Track.
    pub const fn priority(&self) -> PublisherPriority {
        if self.header_type.uses_default_priority() {
            PublisherPriority::Inherited
        } else {
            PublisherPriority::Explicit(self.publisher_priority)
        }
    }

    /// Resolve the effective priority with the Track default when available.
    pub const fn effective_priority(&self, track_default: Option<u8>) -> u8 {
        self.priority().resolve(track_default)
    }

    pub fn decode<R: bytes::Buf>(
        header_type: StreamHeaderType,
        r: &mut R,
    ) -> Result<Self, DecodeError> {
        if !header_type.is_subgroup() {
            return Err(DecodeError::InvalidHeaderType);
        }

        tracing::trace!(
            "[DECODE] SubgroupHeader: starting decode with header_type={:?}, buffer_remaining={} bytes",
            header_type,
            r.remaining()
        );

        let track_alias = u64::decode(r)?;
        tracing::trace!("[DECODE] SubgroupHeader: track_alias={}", track_alias);

        let group_id = u64::decode(r)?;
        tracing::trace!("[DECODE] SubgroupHeader: group_id={}", group_id);

        let subgroup_id = match header_type.has_subgroup_id() {
            true => {
                let id = u64::decode(r)?;
                tracing::trace!("[DECODE] SubgroupHeader: subgroup_id={}", id);
                Some(id)
            }
            false => {
                tracing::trace!(
                    "[DECODE] SubgroupHeader: subgroup_id=None (not present for this header type)"
                );
                None
            }
        };

        let publisher_priority = if header_type.uses_default_priority() {
            DEFAULT_PUBLISHER_PRIORITY
        } else {
            u8::decode(r)?
        };
        tracing::trace!(
            "[DECODE] SubgroupHeader: publisher_priority={}, buffer_remaining={} bytes",
            publisher_priority,
            r.remaining()
        );

        let result = Self {
            header_type,
            track_alias,
            group_id,
            subgroup_id,
            publisher_priority,
        };

        tracing::trace!(
            "[DECODE] SubgroupHeader complete: track_alias={}, group_id={}, subgroup_id={:?}, priority={}",
            result.track_alias,
            result.group_id,
            result.subgroup_id,
            result.publisher_priority
        );

        Ok(result)
    }
}

impl Encode for SubgroupHeader {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        if !self.header_type.is_subgroup() {
            return Err(EncodeError::InvalidValue);
        }
        if self.header_type.uses_default_priority()
            && self.publisher_priority != DEFAULT_PUBLISHER_PRIORITY
        {
            return Err(EncodeError::InvalidValue);
        }

        tracing::trace!(
            "[ENCODE] SubgroupHeader: starting encode - track_alias={}, group_id={}, subgroup_id={:?}, priority={}, header_type={:?}",
            self.track_alias,
            self.group_id,
            self.subgroup_id,
            self.publisher_priority,
            self.header_type
        );

        let start_pos = w.remaining_mut();

        self.header_type.encode(w)?;
        tracing::trace!("[ENCODE] SubgroupHeader: encoded header_type");

        self.track_alias.encode(w)?;
        tracing::trace!(
            "[ENCODE] SubgroupHeader: encoded track_alias={}",
            self.track_alias
        );

        self.group_id.encode(w)?;
        tracing::trace!(
            "[ENCODE] SubgroupHeader: encoded group_id={}",
            self.group_id
        );

        if self.header_type.has_subgroup_id() {
            if let Some(subgroup_id) = self.subgroup_id {
                subgroup_id.encode(w)?;
                tracing::trace!(
                    "[ENCODE] SubgroupHeader: encoded subgroup_id={}",
                    subgroup_id
                );
            } else {
                tracing::error!(
                    "[ENCODE] SubgroupHeader: MISSING subgroup_id for header_type={:?}",
                    self.header_type
                );
                return Err(EncodeError::MissingField("SubgroupId".to_string()));
            }
        } else if self.subgroup_id.is_some() {
            return Err(EncodeError::InvalidValue);
        } else {
            tracing::trace!("[ENCODE] SubgroupHeader: subgroup_id not encoded (not required for this header type)");
        }

        if !self.header_type.uses_default_priority() {
            self.publisher_priority.encode(w)?;
            tracing::trace!(
                "[ENCODE] SubgroupHeader: encoded publisher_priority={}",
                self.publisher_priority
            );
        }

        let bytes_written = start_pos - w.remaining_mut();
        tracing::trace!(
            "[ENCODE] SubgroupHeader complete: wrote {} bytes",
            bytes_written
        );

        Ok(())
    }
}

// Subgroup Object without Extension headers (version with ExtensionHeaders is below)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SubgroupObject {
    pub object_id_delta: u64,
    pub payload_length: usize,
    pub status: Option<ObjectStatus>,
    //pub payload: bytes::Bytes,  // TODO SLG - payload is sent outside this right now - decide which way to go
}

impl SubgroupObject {
    /// Resolve this object's absolute ID using the draft-19 checked-delta rule.
    ///
    /// The first object's delta is its absolute Object ID. Every later Object
    /// ID is `previous + delta + 1`; overflow is a protocol violation.
    pub fn resolve_object_id(&self, previous: Option<u64>) -> Result<u64, DecodeError> {
        resolve_object_id(previous, self.object_id_delta)
    }
}

impl Decode for SubgroupObject {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        tracing::trace!(
            "[DECODE] SubgroupObject: starting decode, buffer_remaining={} bytes",
            r.remaining()
        );

        let object_id_delta = u64::decode(r)?;
        tracing::trace!(
            "[DECODE] SubgroupObject: object_id_delta={}",
            object_id_delta
        );

        let payload_length = decode_payload_length(r)?;
        tracing::trace!("[DECODE] SubgroupObject: payload_length={}", payload_length);

        let status = match payload_length {
            0 => {
                let s = ObjectStatus::decode(r)?;
                tracing::trace!("[DECODE] SubgroupObject: status={:?} (payload_length=0)", s);
                Some(s)
            }
            _ => {
                tracing::trace!("[DECODE] SubgroupObject: status=None (payload_length > 0)");
                None
            }
        };

        //Self::decode_remaining(r, payload_length);
        //let payload = r.copy_to_bytes(payload_length);

        tracing::trace!(
            "[DECODE] SubgroupObject complete: object_id_delta={}, payload_length={}, status={:?}, buffer_remaining={} bytes",
            object_id_delta,
            payload_length,
            status,
            r.remaining()
        );

        Ok(Self {
            object_id_delta,
            payload_length,
            status,
            //payload,
        })
    }
}

impl Encode for SubgroupObject {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        tracing::trace!(
            "[ENCODE] SubgroupObject: starting encode - object_id_delta={}, payload_length={}, status={:?}",
            self.object_id_delta,
            self.payload_length,
            self.status
        );

        self.object_id_delta.encode(w)?;
        tracing::trace!(
            "[ENCODE] SubgroupObject: encoded object_id_delta={}",
            self.object_id_delta
        );

        encode_payload_length(self.payload_length, w)?;
        tracing::trace!(
            "[ENCODE] SubgroupObject: encoded payload_length={}",
            self.payload_length
        );

        if self.payload_length == 0 {
            if let Some(status) = self.status {
                status.encode(w)?;
                tracing::trace!("[ENCODE] SubgroupObject: encoded status={:?}", status);
            } else {
                tracing::error!("[ENCODE] SubgroupObject: MISSING status for payload_length=0");
                return Err(EncodeError::MissingField("Status".to_string()));
            }
        } else if self.status.is_some() {
            return Err(EncodeError::InvalidValue);
        }
        //Self::encode_remaining(w, self.payload.len())?;
        //w.put_slice(&self.payload);

        tracing::trace!("[ENCODE] SubgroupObject complete");

        Ok(())
    }
}

// Subgroup Object with Extension headers
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SubgroupObjectExt {
    pub object_id_delta: u64,
    pub extension_headers: ExtensionHeaders,
    pub payload_length: usize,
    pub status: Option<ObjectStatus>,
    //pub payload: bytes::Bytes,  // TODO SLG - payload is sent outside this right now - decide which way to go
}

impl SubgroupObjectExt {
    /// Resolve this object's absolute ID using the draft-19 checked-delta rule.
    pub fn resolve_object_id(&self, previous: Option<u64>) -> Result<u64, DecodeError> {
        resolve_object_id(previous, self.object_id_delta)
    }
}

impl Decode for SubgroupObjectExt {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        tracing::trace!(
            "[DECODE] SubgroupObjectExt: starting decode, buffer_remaining={} bytes",
            r.remaining()
        );

        let object_id_delta = u64::decode(r)?;
        tracing::trace!(
            "[DECODE] SubgroupObjectExt: object_id_delta={}",
            object_id_delta
        );

        let extension_headers = ExtensionHeaders::decode(r)?;
        tracing::trace!(
            "[DECODE] SubgroupObjectExt: extension_headers={:?}",
            extension_headers
        );

        let payload_length = decode_payload_length(r)?;
        tracing::trace!(
            "[DECODE] SubgroupObjectExt: payload_length={}",
            payload_length
        );

        let status = match payload_length {
            0 => {
                let s = ObjectStatus::decode(r)?;
                tracing::trace!(
                    "[DECODE] SubgroupObjectExt: status={:?} (payload_length=0)",
                    s
                );
                Some(s)
            }
            _ => {
                tracing::trace!("[DECODE] SubgroupObjectExt: status=None (payload_length > 0)");
                None
            }
        };

        if status.is_some_and(|status| status != ObjectStatus::NormalObject)
            && !extension_headers.is_empty()
        {
            return Err(DecodeError::InvalidValue);
        }

        //Self::decode_remaining(r, payload_length);
        //let payload = r.copy_to_bytes(payload_length);

        tracing::trace!(
            "[DECODE] SubgroupObjectExt complete: object_id_delta={}, payload_length={}, status={:?}, buffer_remaining={} bytes",
            object_id_delta,
            payload_length,
            status,
            r.remaining()
        );

        Ok(Self {
            object_id_delta,
            extension_headers,
            payload_length,
            status,
            //payload,
        })
    }
}

impl Encode for SubgroupObjectExt {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        tracing::trace!(
            "[ENCODE] SubgroupObjectExt: starting encode - object_id_delta={}, payload_length={}, status={:?}, extension_headers={:?}",
            self.object_id_delta,
            self.payload_length,
            self.status,
            self.extension_headers
        );

        self.object_id_delta.encode(w)?;
        tracing::trace!(
            "[ENCODE] SubgroupObjectExt: encoded object_id_delta={}",
            self.object_id_delta
        );

        self.extension_headers.encode(w)?;
        tracing::trace!("[ENCODE] SubgroupObjectExt: encoded extension_headers");

        encode_payload_length(self.payload_length, w)?;
        tracing::trace!(
            "[ENCODE] SubgroupObjectExt: encoded payload_length={}",
            self.payload_length
        );

        if self.payload_length == 0 {
            if let Some(status) = self.status {
                if status != ObjectStatus::NormalObject && !self.extension_headers.is_empty() {
                    return Err(EncodeError::InvalidValue);
                }
                status.encode(w)?;
                tracing::trace!("[ENCODE] SubgroupObjectExt: encoded status={:?}", status);
            } else {
                tracing::error!("[ENCODE] SubgroupObjectExt: MISSING status for payload_length=0");
                return Err(EncodeError::MissingField("Status".to_string()));
            }
        } else if self.status.is_some() {
            return Err(EncodeError::InvalidValue);
        }
        //Self::encode_remaining(w, self.payload.len())?;
        //w.put_slice(&self.payload);

        tracing::trace!("[ENCODE] SubgroupObjectExt complete");

        Ok(())
    }
}

fn resolve_object_id(previous: Option<u64>, delta: u64) -> Result<u64, DecodeError> {
    match previous {
        None => Ok(delta),
        Some(previous) => previous
            .checked_add(delta)
            .and_then(|value| value.checked_add(1))
            .ok_or(DecodeError::InvalidValue),
    }
}

// TODO SLG - add more unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use bytes::BytesMut;

    #[test]
    fn encode_decode_object() {
        let mut buf = BytesMut::new();

        let msg = SubgroupObject {
            object_id_delta: 0,
            payload_length: 7,
            status: None,
        };
        msg.encode(&mut buf).unwrap();
        let decoded = SubgroupObject::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn encode_decode_object_ext() {
        let mut buf = BytesMut::new();

        // One ExtensionHeader for testing
        let mut ext_hdrs = ExtensionHeaders::new();
        ext_hdrs.set_bytesvalue(123, vec![0x00, 0x01, 0x02, 0x03]);

        let msg = SubgroupObjectExt {
            object_id_delta: 0,
            extension_headers: ext_hdrs,
            payload_length: 7,
            status: None,
        };
        msg.encode(&mut buf).unwrap();
        let decoded = SubgroupObjectExt::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn decode_rejects_non_normal_status_with_extension_headers() {
        let data = vec![
            0x00, // object id delta
            0x02, // extension headers byte length
            0x00, // extension delta type
            0x01, // extension value
            0x00, // payload length
            0x04, // EndOfTrack
        ];
        let mut buf: Bytes = data.into();

        assert!(matches!(
            SubgroupObjectExt::decode(&mut buf).unwrap_err(),
            DecodeError::InvalidValue
        ));
    }

    #[test]
    fn encode_rejects_non_normal_status_with_extension_headers() {
        let mut ext_hdrs = ExtensionHeaders::new();
        ext_hdrs.set_intvalue(0, 1);
        let msg = SubgroupObjectExt {
            object_id_delta: 0,
            extension_headers: ext_hdrs,
            payload_length: 0,
            status: Some(ObjectStatus::EndOfTrack),
        };
        let mut buf = BytesMut::new();

        assert!(matches!(
            msg.encode(&mut buf).unwrap_err(),
            EncodeError::InvalidValue
        ));
    }

    #[test]
    fn default_priority_header_omits_priority_golden_vector() {
        let header_type = StreamHeaderType::subgroup(
            true,
            crate::data::SubgroupIdMode::Explicit,
            true,
            true,
            true,
        );
        let header = SubgroupHeader {
            header_type,
            track_alias: 2,
            group_id: 3,
            subgroup_id: Some(4),
            publisher_priority: DEFAULT_PUBLISHER_PRIORITY,
        };
        let mut wire = BytesMut::new();
        header.encode(&mut wire).unwrap();
        assert_eq!(wire.as_ref(), &[0x7d, 0x02, 0x03, 0x04]);

        let decoded_type = StreamHeaderType::decode(&mut wire).unwrap();
        let decoded = SubgroupHeader::decode(decoded_type, &mut wire).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded.priority(), PublisherPriority::Inherited);
        assert_eq!(decoded.effective_priority(None), DEFAULT_PUBLISHER_PRIORITY);
        assert_eq!(decoded.effective_priority(Some(37)), 37);
        assert!(wire.is_empty());
    }

    #[test]
    fn header_rejects_subgroup_id_when_mode_omits_it() {
        let header = SubgroupHeader {
            header_type: StreamHeaderType::SubgroupZeroId,
            track_alias: 1,
            group_id: 1,
            subgroup_id: Some(0),
            publisher_priority: 1,
        };
        assert!(matches!(
            header.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));
    }

    #[test]
    fn object_id_delta_resolution_is_checked() {
        let first = SubgroupObject {
            object_id_delta: 7,
            payload_length: 1,
            status: None,
        };
        assert_eq!(first.resolve_object_id(None).unwrap(), 7);

        let next = SubgroupObject {
            object_id_delta: 2,
            payload_length: 1,
            status: None,
        };
        assert_eq!(next.resolve_object_id(Some(7)).unwrap(), 10);

        let overflowing = SubgroupObjectExt {
            object_id_delta: 0,
            extension_headers: ExtensionHeaders::new(),
            payload_length: 1,
            status: None,
        };
        assert!(matches!(
            overflowing.resolve_object_id(Some(u64::MAX)),
            Err(DecodeError::InvalidValue)
        ));
    }

    #[test]
    fn subgroup_id_reference_never_conflates_zero_and_first_object() {
        let zero = SubgroupHeader {
            header_type: StreamHeaderType::SubgroupZeroId,
            track_alias: 1,
            group_id: 1,
            subgroup_id: None,
            publisher_priority: 1,
        };
        assert_eq!(
            zero.subgroup_id_reference().unwrap(),
            SubgroupIdReference::Zero
        );
        assert_eq!(
            zero.subgroup_id_reference().unwrap().resolve(Some(77)),
            Some(0)
        );
        assert_eq!(zero.resolved_subgroup_id().unwrap(), Some(0));

        let first = SubgroupHeader {
            header_type: StreamHeaderType::SubgroupFirstObjectId,
            track_alias: 1,
            group_id: 1,
            subgroup_id: None,
            publisher_priority: 1,
        };
        assert_eq!(
            first.subgroup_id_reference().unwrap(),
            SubgroupIdReference::FirstObject
        );
        assert_eq!(first.subgroup_id_reference().unwrap().resolve(None), None);
        assert_eq!(
            first.subgroup_id_reference().unwrap().resolve(Some(77)),
            Some(77)
        );
        assert_eq!(first.resolved_subgroup_id().unwrap(), None);

        let mut resolved_first = first;
        resolved_first.subgroup_id = Some(77);
        assert_eq!(resolved_first.resolved_subgroup_id().unwrap(), Some(77));

        let explicit = SubgroupHeader {
            header_type: StreamHeaderType::SubgroupId,
            track_alias: 1,
            group_id: 1,
            subgroup_id: Some(9),
            publisher_priority: 1,
        };
        assert_eq!(
            explicit.subgroup_id_reference().unwrap(),
            SubgroupIdReference::Explicit(9)
        );
        assert_eq!(
            explicit.subgroup_id_reference().unwrap().resolve(Some(77)),
            Some(9)
        );
        assert_eq!(explicit.resolved_subgroup_id().unwrap(), Some(9));
    }

    #[test]
    fn default_priority_header_rejects_a_misleading_placeholder() {
        let header = SubgroupHeader {
            header_type: StreamHeaderType::subgroup(
                false,
                SubgroupIdMode::Zero,
                false,
                true,
                false,
            ),
            track_alias: 1,
            group_id: 1,
            subgroup_id: None,
            publisher_priority: 0,
        };
        assert!(matches!(
            header.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));
    }

    #[test]
    fn encode_rejects_status_for_nonempty_payload() {
        let object = SubgroupObject {
            object_id_delta: 0,
            payload_length: 1,
            status: Some(ObjectStatus::NormalObject),
        };
        assert!(matches!(
            object.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));

        let object = SubgroupObjectExt {
            object_id_delta: 0,
            extension_headers: ExtensionHeaders::new(),
            payload_length: 1,
            status: Some(ObjectStatus::NormalObject),
        };
        assert!(matches!(
            object.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));
    }
}
