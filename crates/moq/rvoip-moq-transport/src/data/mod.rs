// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

mod datagram;
mod extension_headers;
mod fetch;
mod header;
mod object_status;
mod subgroup;

use crate::coding::{Decode, DecodeError, Encode, EncodeError};

/// Default Publisher Priority when a Track does not declare property `0x0E`.
pub const DEFAULT_PUBLISHER_PRIORITY: u8 = 128;

/// Whether an object carries an explicit priority or inherits the Track value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublisherPriority {
    Inherited,
    Explicit(u8),
}

impl PublisherPriority {
    /// Resolve the effective value using the Track property or the protocol
    /// default when the Track omits it.
    pub const fn resolve(self, track_default: Option<u8>) -> u8 {
        match self {
            Self::Inherited => match track_default {
                Some(priority) => priority,
                None => DEFAULT_PUBLISHER_PRIORITY,
            },
            Self::Explicit(priority) => priority,
        }
    }
}

pub(crate) fn decode_payload_length<R: bytes::Buf>(r: &mut R) -> Result<usize, DecodeError> {
    let wire_length = u64::decode(r)?;
    usize::try_from(wire_length)
        .map_err(|_| DecodeError::FieldBoundsExceeded("ObjectPayloadLength".to_string()))
}

pub(crate) fn encode_payload_length<W: bytes::BufMut>(
    payload_length: usize,
    w: &mut W,
) -> Result<(), EncodeError> {
    let wire_length = u64::try_from(payload_length)
        .map_err(|_| EncodeError::FieldBoundsExceeded("ObjectPayloadLength".to_string()))?;
    wire_length.encode(w)
}

pub use datagram::*;
pub use extension_headers::*;
pub use fetch::*;
pub use header::*;
pub use object_status::*;
pub use subgroup::*;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn payload_length_uses_u64_wire_encoding_without_truncation() {
        let mut wire = BytesMut::new();
        encode_payload_length(usize::MAX, &mut wire).unwrap();
        assert_eq!(u64::decode(&mut wire).unwrap(), usize::MAX as u64);
        assert!(wire.is_empty());
    }

    #[test]
    fn payload_length_checks_the_implementation_limit() {
        let mut wire = BytesMut::new();
        u64::MAX.encode(&mut wire).unwrap();
        let decoded = decode_payload_length(&mut wire);

        if usize::BITS < u64::BITS {
            assert!(matches!(decoded, Err(DecodeError::FieldBoundsExceeded(_))));
        } else {
            assert_eq!(decoded.unwrap(), usize::MAX);
        }
    }
}
