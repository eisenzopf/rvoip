// SPDX-FileCopyrightText: 2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! PUBLISH_SKIPPED message from draft-ietf-moq-transport-19.

use crate::coding::{Decode, DecodeError, Encode, EncodeError, TrackName, TrackNamespacePrefix};

/// Reports a track for which a SUBSCRIBE_TRACKS response will not send PUBLISH.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishSkipped {
    /// Namespace fields following the SUBSCRIBE_TRACKS prefix.
    pub track_namespace_suffix: TrackNamespacePrefix,

    /// The skipped track name.
    pub track_name: TrackName,
}

impl Decode for PublishSkipped {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        Ok(Self {
            track_namespace_suffix: TrackNamespacePrefix::decode(r)?,
            track_name: TrackName::decode(r)?,
        })
    }
}

impl Encode for PublishSkipped {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.track_namespace_suffix.encode(w)?;
        self.track_name.encode(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn encode_decode() {
        let msg = PublishSkipped {
            track_namespace_suffix: TrackNamespacePrefix::from_utf8_path("region/west"),
            track_name: "main".into(),
        };
        let mut buf = BytesMut::new();
        msg.encode(&mut buf).unwrap();
        assert_eq!(PublishSkipped::decode(&mut buf).unwrap(), msg);
    }
}
