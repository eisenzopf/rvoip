// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! REQUEST_OK message (draft-ietf-moq-transport-19 §9.7).
//!
//! Sent in response to REQUEST_UPDATE, TRACK_STATUS, SUBSCRIBE_NAMESPACE,
//! and PUBLISH_NAMESPACE requests. On a request stream the response omits its
//! Request ID; `id` below is retained as the local routing association.

use crate::coding::{Decode, DecodeError, Encode, EncodeError, KeyValuePairs};
use crate::message::{params::decode_request_parameters, TrackProperties};

/// Sent to acknowledge a successful request update or status query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestOk {
    /// Local Request ID association; omitted from request-stream wire bodies.
    pub id: u64,

    /// Optional parameters (e.g. LARGEST_OBJECT for TRACK_STATUS responses).
    pub params: KeyValuePairs,

    /// Track Properties. These are populated only for TRACK_STATUS_OK and
    /// otherwise must be empty.
    pub track_properties: TrackProperties,
}

impl Decode for RequestOk {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let id = u64::decode(r)?;
        let params = decode_request_parameters(r)?;
        let track_properties = TrackProperties::decode(r)?;
        Ok(Self {
            id,
            params,
            track_properties,
        })
    }
}

impl Encode for RequestOk {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.id.encode(w)?;
        self.params.encode(w)?;
        self.track_properties.encode(w)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn encode_decode_no_params() {
        let mut buf = BytesMut::new();
        let msg = RequestOk {
            id: 42,
            params: KeyValuePairs::default(),
            track_properties: TrackProperties::default(),
        };
        msg.encode(&mut buf).unwrap();
        let decoded = RequestOk::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn encode_decode_with_params() {
        let mut buf = BytesMut::new();
        let mut params = KeyValuePairs::new();
        params.set_intvalue(0x08, 3600); // EXPIRES example
        let msg = RequestOk {
            id: 100,
            params,
            track_properties: TrackProperties::default(),
        };
        msg.encode(&mut buf).unwrap();
        let decoded = RequestOk::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn encode_decode_with_track_properties() {
        let mut buf = BytesMut::new();
        let mut track_properties = TrackProperties::default();
        track_properties.set_int_extension(0x78, 7);
        let msg = RequestOk {
            id: 2,
            params: KeyValuePairs::default(),
            track_properties,
        };

        msg.encode(&mut buf).unwrap();
        assert_eq!(&buf[..], &[0x02, 0x00, 0x78, 0x07]);
        assert_eq!(RequestOk::decode(&mut buf).unwrap(), msg);
    }
}
