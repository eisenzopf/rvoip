// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::coding::{Decode, DecodeError, Encode, EncodeError};
use crate::data::{FetchHeader, SubgroupHeader};
use std::fmt;

const SUBGROUP_PROPERTIES: u64 = 0x01;
const SUBGROUP_ID_MODE: u64 = 0x06;
const SUBGROUP_END_OF_GROUP: u64 = 0x08;
const SUBGROUP_MARKER: u64 = 0x10;
const SUBGROUP_DEFAULT_PRIORITY: u64 = 0x20;
const SUBGROUP_FIRST_OBJECT: u64 = 0x40;
const SUBGROUP_TYPE_MASK: u64 = 0x7f;

/// The encoding selected for the Subgroup ID in a `SUBGROUP_HEADER`.
#[derive(Copy, Debug, Clone, Eq, PartialEq)]
pub enum SubgroupIdMode {
    /// The field is omitted and the Subgroup ID is zero.
    Zero,
    /// The field is omitted and the first Object ID is the Subgroup ID.
    FirstObject,
    /// The field is present in the header.
    Explicit,
}

impl SubgroupIdMode {
    const fn bits(self) -> u64 {
        match self {
            Self::Zero => 0x00,
            Self::FirstObject => 0x02,
            Self::Explicit => 0x04,
        }
    }

    const fn from_type(value: u64) -> Option<Self> {
        match value & SUBGROUP_ID_MODE {
            0x00 => Some(Self::Zero),
            0x02 => Some(Self::FirstObject),
            0x04 => Some(Self::Explicit),
            _ => None,
        }
    }
}

/// A validated draft-19 unidirectional stream header type.
///
/// `SUBGROUP_HEADER` uses six independent/type fields and therefore has 48
/// valid wire values. Keeping the validated value avoids an error-prone enum of
/// every permutation while the legacy associated names below preserve existing
/// callers.
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct StreamHeaderType(u64);

#[allow(non_upper_case_globals)]
impl StreamHeaderType {
    pub const Fetch: Self = Self(0x05);

    pub const SubgroupZeroId: Self = Self(0x10);
    pub const SubgroupZeroIdExt: Self = Self(0x11);
    pub const SubgroupFirstObjectId: Self = Self(0x12);
    pub const SubgroupFirstObjectIdExt: Self = Self(0x13);
    pub const SubgroupId: Self = Self(0x14);
    pub const SubgroupIdExt: Self = Self(0x15);
    pub const SubgroupZeroIdEndOfGroup: Self = Self(0x18);
    pub const SubgroupZeroIdExtEndOfGroup: Self = Self(0x19);
    pub const SubgroupFirstObjectIdEndOfGroup: Self = Self(0x1a);
    pub const SubgroupFirstObjectIdExtEndOfGroup: Self = Self(0x1b);
    pub const SubgroupIdEndOfGroup: Self = Self(0x1c);
    pub const SubgroupIdExtEndOfGroup: Self = Self(0x1d);

    /// Construct any valid draft-19 `SUBGROUP_HEADER` bitfield combination.
    pub const fn subgroup(
        properties: bool,
        id_mode: SubgroupIdMode,
        end_of_group: bool,
        default_priority: bool,
        first_object: bool,
    ) -> Self {
        let mut value = SUBGROUP_MARKER | id_mode.bits();
        if properties {
            value |= SUBGROUP_PROPERTIES;
        }
        if end_of_group {
            value |= SUBGROUP_END_OF_GROUP;
        }
        if default_priority {
            value |= SUBGROUP_DEFAULT_PRIORITY;
        }
        if first_object {
            value |= SUBGROUP_FIRST_OBJECT;
        }
        Self(value)
    }

    /// Return the validated wire value.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Validate and construct a draft-19 stream header type.
    pub const fn from_value(value: u64) -> Option<Self> {
        if value == Self::Fetch.0 || Self::is_valid_subgroup_value(value) {
            Some(Self(value))
        } else {
            None
        }
    }

    const fn is_valid_subgroup_value(value: u64) -> bool {
        value & !SUBGROUP_TYPE_MASK == 0
            && value & SUBGROUP_MARKER != 0
            && SubgroupIdMode::from_type(value).is_some()
    }

    pub const fn is_subgroup(self) -> bool {
        Self::is_valid_subgroup_value(self.0)
    }

    pub const fn is_fetch(self) -> bool {
        self.0 == Self::Fetch.0
    }

    /// Whether each object on this subgroup stream carries Object Properties.
    pub const fn has_properties(self) -> bool {
        self.is_subgroup() && self.0 & SUBGROUP_PROPERTIES != 0
    }

    /// Legacy name retained for the existing subgroup/fetch decoder.
    pub const fn has_extension_headers(self) -> bool {
        self.is_fetch() || self.has_properties()
    }

    pub const fn subgroup_id_mode(self) -> Option<SubgroupIdMode> {
        if self.is_subgroup() {
            SubgroupIdMode::from_type(self.0)
        } else {
            None
        }
    }

    pub const fn has_subgroup_id(self) -> bool {
        matches!(self.subgroup_id_mode(), Some(SubgroupIdMode::Explicit))
    }

    pub const fn uses_first_object_id_as_subgroup_id(self) -> bool {
        matches!(self.subgroup_id_mode(), Some(SubgroupIdMode::FirstObject))
    }

    pub const fn contains_end_of_group(self) -> bool {
        self.is_subgroup() && self.0 & SUBGROUP_END_OF_GROUP != 0
    }

    pub const fn uses_default_priority(self) -> bool {
        self.is_subgroup() && self.0 & SUBGROUP_DEFAULT_PRIORITY != 0
    }

    pub const fn is_first_object(self) -> bool {
        self.is_subgroup() && self.0 & SUBGROUP_FIRST_OBJECT != 0
    }
}

impl Encode for StreamHeaderType {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        let val = self.value();
        tracing::trace!(
            "[ENCODE] StreamHeaderType: encoding {:?} as {:#x}",
            self,
            val
        );
        val.encode(w)?;
        tracing::trace!("[ENCODE] StreamHeaderType: encoded successfully");
        Ok(())
    }
}

impl Decode for StreamHeaderType {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        tracing::trace!(
            "[DECODE] StreamHeaderType: starting decode, buffer_remaining={} bytes",
            r.remaining()
        );

        let type_value = u64::decode(r)?;
        tracing::trace!(
            "[DECODE] StreamHeaderType: decoded type value={:#x}",
            type_value
        );

        let header_type = match Self::from_value(type_value) {
            Some(header_type) => Ok(header_type),
            None => {
                tracing::error!(
                    "[DECODE] StreamHeaderType: INVALID type value={:#x}",
                    type_value
                );
                Err(DecodeError::InvalidHeaderType)
            }
        };

        if let Ok(header_type_inner) = &header_type {
            tracing::trace!(
                "[DECODE] StreamHeaderType: {}, has_subgroup_id={}, has_extension_headers={}",
                header_type_inner,
                header_type_inner.has_subgroup_id(),
                header_type_inner.has_extension_headers()
            );
        }

        header_type
    }
}

impl fmt::Display for StreamHeaderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_fetch() {
            write!(f, "Fetch ({:#x})", self.value())
        } else {
            write!(f, "Subgroup ({:#x})", self.value())
        }
    }
}

impl fmt::Debug for StreamHeaderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StreamHeader {
    /// Subgroup Header Type
    pub header_type: StreamHeaderType,

    /// Subgroup Header for StreamHeaderTypes that are Subgroup header types
    pub subgroup_header: Option<SubgroupHeader>,

    /// Fetch Header for StreamHeaderTypes that are Fetch header types
    pub fetch_header: Option<FetchHeader>,
}

impl Decode for StreamHeader {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        tracing::trace!(
            "[DECODE] StreamHeader: starting decode, buffer_remaining={} bytes",
            r.remaining()
        );

        let header_type = StreamHeaderType::decode(r)?;
        tracing::trace!(
            "[DECODE] StreamHeader: decoded header_type={:?}",
            header_type
        );

        let subgroup_header = match header_type.is_subgroup() {
            true => {
                tracing::trace!("[DECODE] StreamHeader: decoding subgroup header");
                Some(SubgroupHeader::decode(header_type, r)?)
            }
            false => {
                tracing::trace!("[DECODE] StreamHeader: no subgroup header (not a subgroup type)");
                None
            }
        };

        let fetch_header = match header_type.is_fetch() {
            true => {
                tracing::trace!("[DECODE] StreamHeader: decoding fetch header");
                Some(FetchHeader::decode(header_type, r)?)
            }
            false => {
                tracing::trace!("[DECODE] StreamHeader: no fetch header (not a fetch type)");
                None
            }
        };

        tracing::trace!(
            "[DECODE] StreamHeader complete: type={:?}, has_subgroup={}, has_fetch={}, buffer_remaining={} bytes",
            header_type,
            subgroup_header.is_some(),
            fetch_header.is_some(),
            r.remaining()
        );

        Ok(Self {
            header_type,
            subgroup_header,
            fetch_header,
        })
    }
}

impl Encode for StreamHeader {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        tracing::trace!(
            "[ENCODE] StreamHeader: starting encode for type={:?}, has_subgroup={}, has_fetch={}",
            self.header_type,
            self.subgroup_header.is_some(),
            self.fetch_header.is_some()
        );

        // Note: we are intentionally not encoding the header_type here, it will be encoded in the
        //       appropriate substructures.
        //self.header_type.encode(w)?;
        if self.header_type.is_subgroup() {
            if let Some(subgroup_header) = &self.subgroup_header {
                if subgroup_header.header_type != self.header_type || self.fetch_header.is_some() {
                    return Err(EncodeError::InvalidValue);
                }
                tracing::trace!("[ENCODE] StreamHeader: encoding subgroup header");
                subgroup_header.encode(w)?;
            } else {
                tracing::error!(
                    "[ENCODE] StreamHeader: MISSING subgroup header for subgroup type={:?}",
                    self.header_type
                );
                return Err(EncodeError::MissingField("SubgroupHeader".to_string()));
            }
        } else if let Some(fetch_header) = &self.fetch_header {
            if fetch_header.header_type != self.header_type || self.subgroup_header.is_some() {
                return Err(EncodeError::InvalidValue);
            }
            tracing::trace!("[ENCODE] StreamHeader: encoding fetch header");
            fetch_header.encode(w)?;
        } else {
            tracing::error!(
                "[ENCODE] StreamHeader: MISSING fetch header for fetch type={:?}",
                self.header_type
            );
            return Err(EncodeError::MissingField("FetchHeader".to_string()));
        }

        tracing::trace!("[ENCODE] StreamHeader complete");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use bytes::BytesMut;

    #[test]
    fn encode_decode_stream_header_type() {
        let mut buf = BytesMut::new();

        let ht = StreamHeaderType::Fetch;
        ht.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x05]);
        let decoded = StreamHeaderType::decode(&mut buf).unwrap();
        assert_eq!(decoded, ht);
        assert!(ht.is_fetch());
        assert!(!ht.is_subgroup());
        assert!(!ht.has_subgroup_id());

        let ht = StreamHeaderType::SubgroupZeroId;
        ht.encode(&mut buf).unwrap();
        assert_eq!(buf.to_vec(), vec![0x10]);
        let decoded = StreamHeaderType::decode(&mut buf).unwrap();
        assert_eq!(decoded, ht);
        assert!(ht.is_subgroup());
        assert!(!ht.is_fetch());
        assert!(!ht.has_subgroup_id());

        let ht = StreamHeaderType::SubgroupFirstObjectId;
        assert!(ht.uses_first_object_id_as_subgroup_id());

        let ht = StreamHeaderType::SubgroupId;
        assert!(!ht.uses_first_object_id_as_subgroup_id());
    }

    #[test]
    fn decode_bad_stream_header_type() {
        let data: Vec<u8> = vec![0x00]; // Invalid filter type
        let mut buf: Bytes = data.into();
        let result = StreamHeaderType::decode(&mut buf);
        assert!(matches!(result, Err(DecodeError::InvalidHeaderType)));
    }

    #[test]
    fn draft19_accepts_all_48_subgroup_bitfields() {
        let mut valid = Vec::new();
        for first_object in [false, true] {
            for default_priority in [false, true] {
                for end_of_group in [false, true] {
                    for id_mode in [
                        SubgroupIdMode::Zero,
                        SubgroupIdMode::FirstObject,
                        SubgroupIdMode::Explicit,
                    ] {
                        for properties in [false, true] {
                            valid.push(StreamHeaderType::subgroup(
                                properties,
                                id_mode,
                                end_of_group,
                                default_priority,
                                first_object,
                            ));
                        }
                    }
                }
            }
        }

        valid.sort_by_key(|header_type| header_type.value());
        valid.dedup();
        assert_eq!(valid.len(), 48);

        for header_type in valid {
            let mut wire = BytesMut::new();
            header_type.encode(&mut wire).unwrap();
            assert_eq!(StreamHeaderType::decode(&mut wire).unwrap(), header_type);
            assert!(wire.is_empty());
        }
    }

    #[test]
    fn draft19_rejects_reserved_subgroup_id_modes_and_non_header_values() {
        for value in [
            0x00_u64, 0x0f, 0x16, 0x17, 0x1e, 0x1f, 0x36, 0x3f, 0x56, 0x5f, 0x76, 0x7f, 0x80,
        ] {
            let mut wire = BytesMut::new();
            value.encode(&mut wire).unwrap();
            assert!(matches!(
                StreamHeaderType::decode(&mut wire),
                Err(DecodeError::InvalidHeaderType)
            ));
        }
    }

    #[test]
    fn draft19_full_subgroup_header_golden_vector() {
        let header_type =
            StreamHeaderType::subgroup(true, SubgroupIdMode::Explicit, true, true, true);
        assert_eq!(header_type.value(), 0x7d);
        assert!(header_type.has_properties());
        assert!(header_type.contains_end_of_group());
        assert!(header_type.uses_default_priority());
        assert!(header_type.is_first_object());

        let header = StreamHeader {
            header_type,
            subgroup_header: Some(SubgroupHeader {
                header_type,
                track_alias: 2,
                group_id: 3,
                subgroup_id: Some(4),
                publisher_priority: crate::data::DEFAULT_PUBLISHER_PRIORITY,
            }),
            fetch_header: None,
        };
        let mut wire = BytesMut::new();
        header.encode(&mut wire).unwrap();
        assert_eq!(wire.as_ref(), &[0x7d, 0x02, 0x03, 0x04]);

        let decoded = StreamHeader::decode(&mut wire).unwrap();
        assert_eq!(decoded, header);
        assert!(wire.is_empty());
    }

    #[test]
    fn stream_header_rejects_disagreeing_nested_type() {
        let header = StreamHeader {
            header_type: StreamHeaderType::SubgroupId,
            subgroup_header: Some(SubgroupHeader {
                header_type: StreamHeaderType::SubgroupZeroId,
                track_alias: 1,
                group_id: 2,
                subgroup_id: None,
                publisher_priority: 3,
            }),
            fetch_header: None,
        };
        assert!(matches!(
            header.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));
    }

    #[test]
    fn encode_decode_stream_header() {
        let mut buf = BytesMut::new();

        let sh = StreamHeader {
            header_type: StreamHeaderType::Fetch,
            subgroup_header: None,
            fetch_header: Some(FetchHeader {
                header_type: StreamHeaderType::Fetch,
                request_id: 10,
            }),
        };
        sh.encode(&mut buf).unwrap();
        let decoded = StreamHeader::decode(&mut buf).unwrap();
        assert_eq!(decoded, sh);
        assert!(sh.header_type.is_fetch());
        assert!(!sh.header_type.is_subgroup());
        assert!(!sh.header_type.has_subgroup_id());

        let sh = StreamHeader {
            header_type: StreamHeaderType::SubgroupId,
            subgroup_header: Some(SubgroupHeader {
                header_type: StreamHeaderType::SubgroupId,
                track_alias: 10,
                group_id: 0,
                subgroup_id: Some(1),
                publisher_priority: 100,
            }),
            fetch_header: None,
        };
        sh.encode(&mut buf).unwrap();
        let decoded = StreamHeader::decode(&mut buf).unwrap();
        assert_eq!(decoded, sh);
        assert!(sh.header_type.is_subgroup());
        assert!(!sh.header_type.is_fetch());
        assert!(sh.header_type.has_subgroup_id());
    }
}
