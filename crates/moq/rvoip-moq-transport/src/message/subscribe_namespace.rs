// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! SUBSCRIBE_NAMESPACE message from draft-ietf-moq-transport-19.

use crate::coding::{
    Decode, DecodeError, Encode, EncodeError, KeyValuePairs, TrackNamespacePrefix,
};
use crate::message::params::decode_request_parameters;

/// Requests namespace discovery below a namespace prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscribeNamespace {
    /// The subscription request ID
    pub id: u64,

    /// The track namespace prefix
    pub track_namespace_prefix: TrackNamespacePrefix,

    /// Optional parameters
    pub params: KeyValuePairs,
}

impl Decode for SubscribeNamespace {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let id = u64::decode(r)?;
        let track_namespace_prefix = TrackNamespacePrefix::decode(r)?;
        let params = decode_request_parameters(r)?;

        Ok(Self {
            id,
            track_namespace_prefix,
            params,
        })
    }
}

impl Encode for SubscribeNamespace {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.id.encode(w)?;
        self.track_namespace_prefix.encode(w)?;
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

        // One parameter for testing
        let mut kvps = KeyValuePairs::new();
        kvps.set_bytesvalue(123, vec![0x00, 0x01, 0x02, 0x03]);

        let msg = SubscribeNamespace {
            id: 12345,
            track_namespace_prefix: TrackNamespacePrefix::from_utf8_path("path/prefix"),
            params: kvps,
        };
        msg.encode(&mut buf).unwrap();
        let decoded = SubscribeNamespace::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }
}
