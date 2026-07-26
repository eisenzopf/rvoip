// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! REQUEST_UPDATE message (draft-ietf-moq-transport-19 §9.12).
//!
//! The sender writes this message on the same bidirectional stream as the
//! request it modifies. The stream identifies the existing request; the ID in
//! this message identifies the update itself and consumes a new Request ID.

use crate::coding::{Decode, DecodeError, Encode, EncodeError, KeyValuePairs};
use crate::message::params::decode_request_parameters;

/// Sent to modify an existing request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestUpdate {
    /// Request ID consumed by this update message.
    pub id: u64,

    /// Parameters to update. Absent parameters retain their current values.
    pub params: KeyValuePairs,
}

impl Decode for RequestUpdate {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let id = u64::decode(r)?;
        let params = decode_request_parameters(r)?;
        Ok(Self { id, params })
    }
}

impl Encode for RequestUpdate {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.id.encode(w)?;
        self.params.encode(w)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn encode_decode() {
        let mut buf = BytesMut::new();
        let mut params = KeyValuePairs::new();
        params.set_intvalue(0x10, 1); // FORWARD=1
        let msg = RequestUpdate { id: 4, params };
        msg.encode(&mut buf).unwrap();
        let decoded = RequestUpdate::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn encode_decode_no_params() {
        let mut buf = BytesMut::new();
        let msg = RequestUpdate {
            id: 6,
            params: KeyValuePairs::default(),
        };
        msg.encode(&mut buf).unwrap();
        let decoded = RequestUpdate::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn draft_19_golden_body_omits_existing_request_id() {
        let mut buf = BytesMut::new();
        RequestUpdate {
            id: 4,
            params: KeyValuePairs::default(),
        }
        .encode(&mut buf)
        .unwrap();

        assert_eq!(&buf[..], &[0x04, 0x00]);
    }
}
