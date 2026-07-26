// SPDX-FileCopyrightText: 2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! SUBSCRIBE_TRACKS message from draft-ietf-moq-transport-19.

use crate::coding::{
    Decode, DecodeError, Encode, EncodeError, KeyValuePairs, TrackNamespacePrefix,
};
use crate::message::params::decode_request_parameters;

/// Requests PUBLISH messages for tracks below a namespace prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscribeTracks {
    /// The request ID allocated for this request stream.
    pub id: u64,

    /// The namespace prefix whose tracks should be published.
    pub track_namespace_prefix: TrackNamespacePrefix,

    /// Default subscription parameters copied to resulting PUBLISH requests.
    pub params: KeyValuePairs,
}

impl Decode for SubscribeTracks {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        Ok(Self {
            id: u64::decode(r)?,
            track_namespace_prefix: TrackNamespacePrefix::decode(r)?,
            params: decode_request_parameters(r)?,
        })
    }
}

impl Encode for SubscribeTracks {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.id.encode(w)?;
        self.track_namespace_prefix.encode(w)?;
        self.params.encode(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn encode_decode() {
        let msg = SubscribeTracks {
            id: 14,
            track_namespace_prefix: TrackNamespacePrefix::from_utf8_path("live/audio"),
            params: KeyValuePairs::default(),
        };
        let mut buf = BytesMut::new();
        msg.encode(&mut buf).unwrap();
        assert_eq!(SubscribeTracks::decode(&mut buf).unwrap(), msg);
    }
}
