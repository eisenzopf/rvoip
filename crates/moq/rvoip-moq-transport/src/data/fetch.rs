// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Draft-19 FETCH stream headers and stateful object serialization.
//!
//! FETCH object fields are delta-compressed against the preceding serialized
//! item, so they deliberately do not implement [`Decode`] or [`Encode`]
//! directly. Use [`FetchObjectDecoder`] or [`FetchObjectEncoder`] for every
//! item on one FETCH stream and keep that codec for the lifetime of the stream.
//!
//! Object payload bytes are transferred separately. After decoding an
//! [`FetchItem::Object`], the caller must consume exactly
//! [`FetchObject::payload_length`] bytes before decoding the next item.

use crate::coding::{Decode, DecodeError, Encode, EncodeError, Location};
use crate::data::{
    decode_payload_length, encode_payload_length, ExtensionHeaders, StreamHeaderType,
};
use crate::message::GroupOrder;

const SUBGROUP_ID_MASK: u64 = 0x03;
const SUBGROUP_ID_ZERO: u64 = 0x00;
const SUBGROUP_ID_PREVIOUS: u64 = 0x01;
const SUBGROUP_ID_PREVIOUS_PLUS_ONE: u64 = 0x02;
const SUBGROUP_ID_EXPLICIT: u64 = 0x03;
const OBJECT_ID_DELTA: u64 = 0x04;
const GROUP_ID_DELTA: u64 = 0x08;
const PUBLISHER_PRIORITY: u64 = 0x10;
const PROPERTIES: u64 = 0x20;
const DATAGRAM: u64 = 0x40;
const MAX_NORMAL_SERIALIZATION_FLAGS: u64 = 0x7f;

/// Marks the inclusive end of a range of Objects known not to exist.
pub const END_OF_NON_EXISTENT_RANGE: u64 = 0x8c;

/// Marks the inclusive end of a range whose Object status is unknown.
pub const END_OF_UNKNOWN_RANGE: u64 = 0x10c;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FetchHeader {
    /// FETCH stream header type (`0x05`).
    pub header_type: StreamHeaderType,

    /// The Request ID of the FETCH carried by this stream.
    pub request_id: u64,
}

// The stream type is decoded before the rest of the header, so this cannot use
// the ordinary Decode trait.
impl FetchHeader {
    pub fn decode<R: bytes::Buf>(
        header_type: StreamHeaderType,
        r: &mut R,
    ) -> Result<Self, DecodeError> {
        if !header_type.is_fetch() {
            return Err(DecodeError::InvalidHeaderType);
        }

        Ok(Self {
            header_type,
            request_id: u64::decode(r)?,
        })
    }
}

impl Encode for FetchHeader {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        if !self.header_type.is_fetch() {
            return Err(EncodeError::InvalidValue);
        }

        self.header_type.encode(w)?;
        self.request_id.encode(w)
    }
}

/// The Object forwarding preference retained in a FETCH response.
///
/// FETCH itself always uses one reliable stream, but the original forwarding
/// preference remains metadata that relays must preserve.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FetchForwardingPreference {
    Subgroup(u64),
    Datagram,
}

/// One normal Object header on a FETCH stream.
///
/// Draft-19 does not carry Object Status in FETCH. A zero payload length is a
/// normal zero-length Object; non-existent and unknown Objects are represented
/// by [`FetchRangeEnd`] markers instead.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FetchObject {
    pub group_id: u64,
    pub object_id: u64,
    pub forwarding_preference: FetchForwardingPreference,

    /// Publisher priority, where smaller values are sent first.
    pub publisher_priority: u8,

    /// Length-prefixed draft-19 Object Properties.
    pub properties: ExtensionHeaders,

    /// Number of payload bytes immediately following this encoded header.
    pub payload_length: usize,
}

impl FetchObject {
    pub fn location(&self) -> Location {
        Location::new(self.group_id, self.object_id)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FetchRangeKind {
    NonExistent,
    Unknown,
}

/// Inclusive end of a non-existent or unknown Object range.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct FetchRangeEnd {
    pub kind: FetchRangeKind,
    pub location: Location,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FetchItem {
    Object(FetchObject),
    EndOfRange(FetchRangeEnd),
}

#[derive(Debug, Clone, Copy, Default)]
struct FetchObjectState {
    /// The preceding Object location, or the latest end-of-range location.
    location: Option<Location>,

    /// Values from the last actual Object. End-of-range markers preserve them.
    subgroup_id: Option<u64>,
    publisher_priority: Option<u8>,
}

impl FetchObjectState {
    fn location_follows(&self, next: Location, order: GroupOrder) -> bool {
        let Some(previous) = self.location else {
            return true;
        };

        if next.group_id == previous.group_id {
            return next.object_id > previous.object_id;
        }

        match order {
            GroupOrder::Ascending => next.group_id > previous.group_id,
            GroupOrder::Descending => next.group_id < previous.group_id,
            GroupOrder::Publisher => false,
        }
    }

    fn record_object(&mut self, object: &FetchObject) {
        self.location = Some(object.location());
        self.subgroup_id = match object.forwarding_preference {
            FetchForwardingPreference::Subgroup(id) => Some(id),
            FetchForwardingPreference::Datagram => None,
        };
        self.publisher_priority = Some(object.publisher_priority);
    }

    fn record_range_end(&mut self, range: FetchRangeEnd) {
        self.location = Some(range.location);
    }
}

/// Stateful encoder for all Object items on one FETCH stream.
#[derive(Debug)]
pub struct FetchObjectEncoder {
    group_order: GroupOrder,
    state: FetchObjectState,
}

impl FetchObjectEncoder {
    pub fn new(group_order: GroupOrder) -> Result<Self, EncodeError> {
        if group_order == GroupOrder::Publisher {
            return Err(EncodeError::InvalidValue);
        }

        Ok(Self {
            group_order,
            state: FetchObjectState::default(),
        })
    }

    /// Encode one Object header or end-of-range marker.
    ///
    /// For an Object, the caller writes exactly `payload_length` payload bytes
    /// before invoking this method again.
    pub fn encode<W: bytes::BufMut>(
        &mut self,
        item: &FetchItem,
        w: &mut W,
    ) -> Result<(), EncodeError> {
        match item {
            FetchItem::Object(object) => self.encode_object(object, w),
            FetchItem::EndOfRange(range) => self.encode_range_end(*range, w),
        }
    }

    fn encode_object<W: bytes::BufMut>(
        &mut self,
        object: &FetchObject,
        w: &mut W,
    ) -> Result<(), EncodeError> {
        let location = object.location();
        if !self.state.location_follows(location, self.group_order) {
            return Err(EncodeError::InvalidValue);
        }

        let previous_location = self.state.location;
        let (group_delta_present, group_delta) = match previous_location {
            None => (true, object.group_id),
            Some(previous) if previous.group_id == object.group_id => (false, 0),
            Some(previous) => {
                let delta = match self.group_order {
                    GroupOrder::Ascending => object
                        .group_id
                        .checked_sub(previous.group_id)
                        .and_then(|value| value.checked_sub(1)),
                    GroupOrder::Descending => previous
                        .group_id
                        .checked_sub(object.group_id)
                        .and_then(|value| value.checked_sub(1)),
                    GroupOrder::Publisher => None,
                }
                .ok_or(EncodeError::InvalidValue)?;
                (true, delta)
            }
        };

        // When a Group ID Delta is present, an Object ID Delta is interpreted
        // as the absolute Object ID. Always include it on group transitions;
        // this is canonical and avoids depending on a prior group's Object ID.
        let (object_delta_present, object_delta) = match previous_location {
            None => (true, object.object_id),
            Some(_) if group_delta_present => (true, object.object_id),
            Some(previous) => match previous.object_id.checked_add(1) {
                Some(next) if next == object.object_id => (false, 0),
                _ => (
                    true,
                    object
                        .object_id
                        .checked_sub(previous.object_id)
                        .ok_or(EncodeError::InvalidValue)?,
                ),
            },
        };

        let mut flags = 0;
        let explicit_subgroup_id = match object.forwarding_preference {
            FetchForwardingPreference::Datagram => {
                flags |= DATAGRAM;
                None
            }
            FetchForwardingPreference::Subgroup(0) => None,
            FetchForwardingPreference::Subgroup(id) => match self.state.subgroup_id {
                Some(previous) if id == previous => {
                    flags |= SUBGROUP_ID_PREVIOUS;
                    None
                }
                Some(previous) if previous.checked_add(1) == Some(id) => {
                    flags |= SUBGROUP_ID_PREVIOUS_PLUS_ONE;
                    None
                }
                _ => {
                    flags |= SUBGROUP_ID_EXPLICIT;
                    Some(id)
                }
            },
        };

        if group_delta_present {
            flags |= GROUP_ID_DELTA;
        }
        if object_delta_present {
            flags |= OBJECT_ID_DELTA;
        }

        let priority_present = self.state.publisher_priority != Some(object.publisher_priority);
        if priority_present {
            flags |= PUBLISHER_PRIORITY;
        }
        if !object.properties.is_empty() {
            flags |= PROPERTIES;
        }

        flags.encode(w)?;
        if group_delta_present {
            group_delta.encode(w)?;
        }
        if let Some(subgroup_id) = explicit_subgroup_id {
            subgroup_id.encode(w)?;
        }
        if object_delta_present {
            object_delta.encode(w)?;
        }
        if priority_present {
            object.publisher_priority.encode(w)?;
        }
        if !object.properties.is_empty() {
            object.properties.encode(w)?;
        }
        encode_payload_length(object.payload_length, w)?;

        self.state.record_object(object);
        Ok(())
    }

    fn encode_range_end<W: bytes::BufMut>(
        &mut self,
        range: FetchRangeEnd,
        w: &mut W,
    ) -> Result<(), EncodeError> {
        if !self
            .state
            .location_follows(range.location, self.group_order)
        {
            return Err(EncodeError::InvalidValue);
        }

        let flags = match range.kind {
            FetchRangeKind::NonExistent => END_OF_NON_EXISTENT_RANGE,
            FetchRangeKind::Unknown => END_OF_UNKNOWN_RANGE,
        };

        // End-of-range values are special Serialization Flags, not bitfields.
        // Their following Group ID and Object ID are absolute values.
        flags.encode(w)?;
        range.location.group_id.encode(w)?;
        range.location.object_id.encode(w)?;

        self.state.record_range_end(range);
        Ok(())
    }
}

/// Stateful decoder for all Object items on one FETCH stream.
#[derive(Debug)]
pub struct FetchObjectDecoder {
    group_order: GroupOrder,
    state: FetchObjectState,
}

impl FetchObjectDecoder {
    pub fn new(group_order: GroupOrder) -> Result<Self, DecodeError> {
        if group_order == GroupOrder::Publisher {
            return Err(DecodeError::InvalidGroupOrder);
        }

        Ok(Self {
            group_order,
            state: FetchObjectState::default(),
        })
    }

    /// Decode one Object header or end-of-range marker.
    ///
    /// For an Object, the caller consumes exactly `payload_length` payload
    /// bytes before invoking this method again.
    pub fn decode<R: bytes::Buf>(&mut self, r: &mut R) -> Result<FetchItem, DecodeError> {
        let flags = u64::decode(r)?;
        match flags {
            END_OF_NON_EXISTENT_RANGE => {
                return self.decode_range_end(FetchRangeKind::NonExistent, r)
            }
            END_OF_UNKNOWN_RANGE => return self.decode_range_end(FetchRangeKind::Unknown, r),
            value if value > MAX_NORMAL_SERIALIZATION_FLAGS => {
                return Err(DecodeError::InvalidValue)
            }
            _ => {}
        }

        let group_delta_present = flags & GROUP_ID_DELTA != 0;
        let object_delta_present = flags & OBJECT_ID_DELTA != 0;
        let priority_present = flags & PUBLISHER_PRIORITY != 0;
        let is_datagram = flags & DATAGRAM != 0;

        if self.state.location.is_none() && (!group_delta_present || !object_delta_present) {
            return Err(DecodeError::InvalidValue);
        }
        if self.state.publisher_priority.is_none() && !priority_present {
            return Err(DecodeError::InvalidValue);
        }

        let group_delta = group_delta_present.then(|| u64::decode(r)).transpose()?;

        let forwarding_preference = if is_datagram {
            // The two subgroup mode bits are ignored for Datagram Objects.
            FetchForwardingPreference::Datagram
        } else {
            match flags & SUBGROUP_ID_MASK {
                SUBGROUP_ID_ZERO => FetchForwardingPreference::Subgroup(0),
                SUBGROUP_ID_PREVIOUS => FetchForwardingPreference::Subgroup(
                    self.state.subgroup_id.ok_or(DecodeError::InvalidValue)?,
                ),
                SUBGROUP_ID_PREVIOUS_PLUS_ONE => FetchForwardingPreference::Subgroup(
                    self.state
                        .subgroup_id
                        .and_then(|id| id.checked_add(1))
                        .ok_or(DecodeError::InvalidValue)?,
                ),
                SUBGROUP_ID_EXPLICIT => FetchForwardingPreference::Subgroup(u64::decode(r)?),
                _ => unreachable!(),
            }
        };

        let object_delta = object_delta_present.then(|| u64::decode(r)).transpose()?;

        let group_id = self.resolve_group_id(group_delta)?;
        let object_id = self.resolve_object_id(group_delta_present, object_delta)?;
        let location = Location::new(group_id, object_id);
        if !self.state.location_follows(location, self.group_order) {
            return Err(DecodeError::InvalidValue);
        }

        let publisher_priority = if priority_present {
            u8::decode(r)?
        } else {
            self.state
                .publisher_priority
                .ok_or(DecodeError::InvalidValue)?
        };

        let properties = if flags & PROPERTIES != 0 {
            ExtensionHeaders::decode(r)?
        } else {
            ExtensionHeaders::default()
        };
        let payload_length = decode_payload_length(r)?;

        let object = FetchObject {
            group_id,
            object_id,
            forwarding_preference,
            publisher_priority,
            properties,
            payload_length,
        };
        self.state.record_object(&object);
        Ok(FetchItem::Object(object))
    }

    fn decode_range_end<R: bytes::Buf>(
        &mut self,
        kind: FetchRangeKind,
        r: &mut R,
    ) -> Result<FetchItem, DecodeError> {
        let range = FetchRangeEnd {
            kind,
            // Unlike normal fields, these special marker fields are absolute.
            location: Location::new(u64::decode(r)?, u64::decode(r)?),
        };
        if !self
            .state
            .location_follows(range.location, self.group_order)
        {
            return Err(DecodeError::InvalidValue);
        }

        self.state.record_range_end(range);
        Ok(FetchItem::EndOfRange(range))
    }

    fn resolve_group_id(&self, delta: Option<u64>) -> Result<u64, DecodeError> {
        match (self.state.location, delta) {
            (None, Some(absolute)) => Ok(absolute),
            (None, None) => Err(DecodeError::InvalidValue),
            (Some(previous), None) => Ok(previous.group_id),
            (Some(previous), Some(delta)) => {
                let step = delta.checked_add(1).ok_or(DecodeError::InvalidValue)?;
                match self.group_order {
                    GroupOrder::Ascending => previous
                        .group_id
                        .checked_add(step)
                        .ok_or(DecodeError::InvalidValue),
                    GroupOrder::Descending => previous
                        .group_id
                        .checked_sub(step)
                        .ok_or(DecodeError::InvalidValue),
                    GroupOrder::Publisher => Err(DecodeError::InvalidGroupOrder),
                }
            }
        }
    }

    fn resolve_object_id(
        &self,
        group_delta_present: bool,
        delta: Option<u64>,
    ) -> Result<u64, DecodeError> {
        match (self.state.location, delta) {
            (None, Some(absolute)) => Ok(absolute),
            (None, None) => Err(DecodeError::InvalidValue),
            (Some(_), Some(absolute)) if group_delta_present => Ok(absolute),
            (Some(previous), Some(delta)) => previous
                .object_id
                .checked_add(delta)
                .ok_or(DecodeError::InvalidValue),
            (Some(previous), None) => previous
                .object_id
                .checked_add(1)
                .ok_or(DecodeError::InvalidValue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Buf, BufMut, Bytes, BytesMut};

    fn object(
        group_id: u64,
        object_id: u64,
        forwarding_preference: FetchForwardingPreference,
        publisher_priority: u8,
        payload_length: usize,
    ) -> FetchItem {
        FetchItem::Object(FetchObject {
            group_id,
            object_id,
            forwarding_preference,
            publisher_priority,
            properties: ExtensionHeaders::default(),
            payload_length,
        })
    }

    fn encode_items(order: GroupOrder, items: &[FetchItem]) -> BytesMut {
        let mut encoder = FetchObjectEncoder::new(order).unwrap();
        let mut wire = BytesMut::new();
        for item in items {
            encoder.encode(item, &mut wire).unwrap();
        }
        wire
    }

    #[test]
    fn fetch_header_has_exact_wire_vector_and_rejects_other_types() {
        let header = FetchHeader {
            header_type: StreamHeaderType::Fetch,
            request_id: 42,
        };
        let mut wire = BytesMut::new();
        header.encode(&mut wire).unwrap();
        assert_eq!(&wire[..], &[0x05, 0x2a]);

        let header_type = StreamHeaderType::decode(&mut wire).unwrap();
        assert_eq!(FetchHeader::decode(header_type, &mut wire).unwrap(), header);
        assert!(wire.is_empty());

        let invalid = FetchHeader {
            header_type: StreamHeaderType::SubgroupZeroId,
            request_id: 42,
        };
        assert!(matches!(
            invalid.encode(&mut BytesMut::new()),
            Err(EncodeError::InvalidValue)
        ));
        assert!(matches!(
            FetchHeader::decode(StreamHeaderType::SubgroupZeroId, &mut Bytes::new()),
            Err(DecodeError::InvalidHeaderType)
        ));
    }

    #[test]
    fn ascending_objects_have_exact_serialization_vectors() {
        let first = object(3, 5, FetchForwardingPreference::Subgroup(7), 10, 4);

        let mut properties = ExtensionHeaders::new();
        properties.set_intvalue(2, 7);
        let second = FetchItem::Object(FetchObject {
            group_id: 3,
            object_id: 6,
            forwarding_preference: FetchForwardingPreference::Subgroup(7),
            publisher_priority: 10,
            properties,
            payload_length: 0,
        });
        let datagram = object(3, 9, FetchForwardingPreference::Datagram, 10, 2);
        let next_group = object(5, 1, FetchForwardingPreference::Subgroup(0), 20, 1);

        let wire = encode_items(
            GroupOrder::Ascending,
            &[
                first.clone(),
                second.clone(),
                datagram.clone(),
                next_group.clone(),
            ],
        );
        assert_eq!(
            &wire[..],
            &[
                0x1f, 0x03, 0x07, 0x05, 0x0a, 0x04, // first: all required fields
                0x21, 0x02, 0x02, 0x07, 0x00, // prior subgroup + Properties + zero payload
                0x44, 0x03, 0x02, // Datagram + Object ID delta + payload length
                0x1c, 0x01, 0x01, 0x14, 0x01, // group delta + absolute object ID
            ]
        );

        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        let mut wire = wire.freeze();
        for expected in [first, second, datagram, next_group] {
            assert_eq!(decoder.decode(&mut wire).unwrap(), expected);
        }
        assert!(wire.is_empty());
    }

    #[test]
    fn full_fetch_stream_vector_keeps_payload_outside_object_headers() {
        let header = FetchHeader {
            header_type: StreamHeaderType::Fetch,
            request_id: 9,
        };
        let first = object(1, 0, FetchForwardingPreference::Subgroup(0), 2, 3);
        let range = FetchItem::EndOfRange(FetchRangeEnd {
            kind: FetchRangeKind::NonExistent,
            location: Location::new(1, 4),
        });

        let mut wire = BytesMut::new();
        header.encode(&mut wire).unwrap();
        let mut encoder = FetchObjectEncoder::new(GroupOrder::Ascending).unwrap();
        encoder.encode(&first, &mut wire).unwrap();
        wire.put_slice(b"abc");
        encoder.encode(&range, &mut wire).unwrap();

        assert_eq!(
            &wire[..],
            &[
                0x05, 0x09, // FETCH_HEADER
                0x1c, 0x01, 0x00, 0x02, 0x03, b'a', b'b', b'c', // Object and payload
                0x80, 0x8c, 0x01, 0x04, // non-existent range marker
            ]
        );

        let header_type = StreamHeaderType::decode(&mut wire).unwrap();
        assert_eq!(FetchHeader::decode(header_type, &mut wire).unwrap(), header);
        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        let decoded = decoder.decode(&mut wire).unwrap();
        assert_eq!(decoded, first);
        let FetchItem::Object(object) = decoded else {
            panic!("expected Object")
        };
        assert_eq!(wire.copy_to_bytes(object.payload_length), &b"abc"[..]);
        assert_eq!(decoder.decode(&mut wire).unwrap(), range);
        assert!(wire.is_empty());
    }

    #[test]
    fn both_end_range_markers_are_absolute_and_preserve_object_metadata() {
        let first = object(3, 5, FetchForwardingPreference::Subgroup(7), 10, 1);
        let non_existent = FetchItem::EndOfRange(FetchRangeEnd {
            kind: FetchRangeKind::NonExistent,
            location: Location::new(3, 8),
        });
        let after = object(3, 9, FetchForwardingPreference::Subgroup(7), 10, 1);
        let unknown = FetchItem::EndOfRange(FetchRangeEnd {
            kind: FetchRangeKind::Unknown,
            location: Location::new(4, 2),
        });

        let wire = encode_items(
            GroupOrder::Ascending,
            &[
                first.clone(),
                non_existent.clone(),
                after.clone(),
                unknown.clone(),
            ],
        );
        assert_eq!(
            &wire[..],
            &[
                0x1f, 0x03, 0x07, 0x05, 0x0a, 0x01, // first Object
                0x80, 0x8c, 0x03, 0x08, // absolute non-existent range end
                0x01, 0x01, // prior subgroup/priority and sequential Object
                0x81, 0x0c, 0x04, 0x02, // absolute unknown range end
            ]
        );

        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        let mut wire = wire.freeze();
        for expected in [first, non_existent, after, unknown] {
            assert_eq!(decoder.decode(&mut wire).unwrap(), expected);
        }
    }

    #[test]
    fn descending_group_deltas_round_trip() {
        let first = object(10, 3, FetchForwardingPreference::Subgroup(0), 8, 1);
        let second = object(7, 0, FetchForwardingPreference::Subgroup(1), 8, 1);
        let wire = encode_items(GroupOrder::Descending, &[first.clone(), second.clone()]);
        assert_eq!(
            &wire[..],
            &[
                0x1c, 0x0a, 0x03, 0x08, 0x01, // first
                0x0e, 0x02, 0x00, 0x01, // group delta=2, prior subgroup+1, object=0
            ]
        );

        let mut decoder = FetchObjectDecoder::new(GroupOrder::Descending).unwrap();
        let mut wire = wire.freeze();
        assert_eq!(decoder.decode(&mut wire).unwrap(), first);
        assert_eq!(decoder.decode(&mut wire).unwrap(), second);
    }

    #[test]
    fn datagram_ignores_subgroup_mode_bits() {
        // Datagram + group/object/priority, with both ignored subgroup bits set.
        let mut wire = Bytes::from_static(&[0x5f, 0x01, 0x02, 0x03, 0x00]);
        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        assert_eq!(
            decoder.decode(&mut wire).unwrap(),
            object(1, 2, FetchForwardingPreference::Datagram, 3, 0)
        );
    }

    #[test]
    fn invalid_flags_and_first_object_references_are_rejected() {
        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        let mut reserved = Bytes::from_static(&[0x80, 0x80]);
        assert!(matches!(
            decoder.decode(&mut reserved),
            Err(DecodeError::InvalidValue)
        ));

        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        let mut missing_absolute_ids = Bytes::from_static(&[0x10, 0x01, 0x00]);
        assert!(matches!(
            decoder.decode(&mut missing_absolute_ids),
            Err(DecodeError::InvalidValue)
        ));

        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        // Required IDs and priority are present, but subgroup mode references
        // a prior Object which does not exist.
        let mut prior_subgroup = Bytes::from_static(&[0x1d, 0x00, 0x00, 0x01, 0x00]);
        assert!(matches!(
            decoder.decode(&mut prior_subgroup),
            Err(DecodeError::InvalidValue)
        ));
    }

    #[test]
    fn checked_group_object_and_subgroup_arithmetic_rejects_overflow() {
        let first_max_group = object(u64::MAX, 0, FetchForwardingPreference::Subgroup(0), 1, 0);
        let mut wire = encode_items(GroupOrder::Ascending, &[first_max_group]).freeze();
        // Group delta 0 would require MAX + 1.
        wire = {
            let mut combined = BytesMut::from(&wire[..]);
            combined.extend_from_slice(&[0x0c, 0x00, 0x00, 0x00]);
            combined.freeze()
        };
        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        decoder.decode(&mut wire).unwrap();
        assert!(matches!(
            decoder.decode(&mut wire),
            Err(DecodeError::InvalidValue)
        ));

        let first_max_object = object(1, u64::MAX, FetchForwardingPreference::Subgroup(0), 1, 0);
        let mut wire = encode_items(GroupOrder::Ascending, &[first_max_object]).freeze();
        let mut combined = BytesMut::from(&wire[..]);
        // Same group, implicit next Object and prior priority.
        combined.extend_from_slice(&[0x00, 0x00]);
        wire = combined.freeze();
        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        decoder.decode(&mut wire).unwrap();
        assert!(matches!(
            decoder.decode(&mut wire),
            Err(DecodeError::InvalidValue)
        ));

        let first_max_subgroup = object(1, 0, FetchForwardingPreference::Subgroup(u64::MAX), 1, 0);
        let mut wire = encode_items(GroupOrder::Ascending, &[first_max_subgroup]).freeze();
        let mut combined = BytesMut::from(&wire[..]);
        // Prior subgroup + 1, implicit next Object/priority.
        combined.extend_from_slice(&[0x02, 0x00]);
        wire = combined.freeze();
        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        decoder.decode(&mut wire).unwrap();
        assert!(matches!(
            decoder.decode(&mut wire),
            Err(DecodeError::InvalidValue)
        ));
    }

    #[test]
    fn encoder_rejects_wrong_order_without_advancing_state() {
        let first = object(4, 2, FetchForwardingPreference::Subgroup(0), 1, 0);
        let wrong = object(4, 1, FetchForwardingPreference::Subgroup(0), 1, 0);
        let valid = object(4, 3, FetchForwardingPreference::Subgroup(0), 1, 0);

        let mut encoder = FetchObjectEncoder::new(GroupOrder::Ascending).unwrap();
        let mut wire = BytesMut::new();
        encoder.encode(&first, &mut wire).unwrap();
        assert!(matches!(
            encoder.encode(&wrong, &mut wire),
            Err(EncodeError::InvalidValue)
        ));
        encoder.encode(&valid, &mut wire).unwrap();

        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        let mut wire = wire.freeze();
        assert_eq!(decoder.decode(&mut wire).unwrap(), first);
        assert_eq!(decoder.decode(&mut wire).unwrap(), valid);
    }

    #[test]
    fn publisher_group_order_is_not_valid_for_fetch_codec() {
        assert!(matches!(
            FetchObjectEncoder::new(GroupOrder::Publisher),
            Err(EncodeError::InvalidValue)
        ));
        assert!(matches!(
            FetchObjectDecoder::new(GroupOrder::Publisher),
            Err(DecodeError::InvalidGroupOrder)
        ));
    }

    #[test]
    fn truncated_properties_are_reported_without_advancing_state() {
        // First Object, Properties present, but the declared three property
        // bytes are absent.
        let mut truncated = Bytes::from_static(&[0x3c, 0x00, 0x00, 0x01, 0x03]);
        let mut decoder = FetchObjectDecoder::new(GroupOrder::Ascending).unwrap();
        assert!(matches!(
            decoder.decode(&mut truncated),
            Err(DecodeError::More(3))
        ));

        // A failed partial decode must not establish delta state.
        let mut valid = Bytes::from_static(&[0x1c, 0x00, 0x00, 0x01, 0x00]);
        assert_eq!(
            decoder.decode(&mut valid).unwrap(),
            object(0, 0, FetchForwardingPreference::Subgroup(0), 1, 0)
        );
    }
}
