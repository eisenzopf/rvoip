// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::coding::{Decode, DecodeError, Encode, EncodeError, SessionUri};

/// Sent by the server to indicate that the client should connect to a different server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoAway {
    pub uri: SessionUri,

    /// Milliseconds the sender intends to wait for graceful closure.
    pub timeout: u64,
}

impl Decode for GoAway {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let uri = SessionUri::decode(r)?;
        let timeout = u64::decode(r)?;
        Ok(Self { uri, timeout })
    }
}

impl Encode for GoAway {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.uri.encode(w)?;
        self.timeout.encode(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn encode_decode() {
        let mut buf = BytesMut::new();

        let msg = GoAway {
            uri: SessionUri("moq://example.com:1234".to_string()),
            timeout: 5_000,
        };
        msg.encode(&mut buf).unwrap();
        let decoded = GoAway::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn draft_19_golden_encoding() {
        let mut buf = BytesMut::new();
        GoAway {
            uri: SessionUri(String::new()),
            timeout: 300,
        }
        .encode(&mut buf)
        .unwrap();

        assert_eq!(buf.to_vec(), vec![0x00, 0x81, 0x2c]);
    }
}
