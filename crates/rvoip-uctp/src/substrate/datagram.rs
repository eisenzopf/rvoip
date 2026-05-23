//! UCTP media datagram framing per CONVERSATION_PROTOCOL.md §10.1.
//!
//! 8-byte header `ver | flags | stream_local_id (u16 BE) | datagram_seq (u32 BE)`
//! followed by the RTP packet body. `pack`/`unpack` do not parse the RTP
//! body — they treat it as opaque bytes.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::errors::SubstrateError;

/// In-memory shape of a UCTP media datagram. Wire layout above.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaDatagram {
    pub flags: u8,
    pub stream_local_id: u16,
    pub seq: u32,
    pub payload: Bytes,
}

/// Serialize a [`MediaDatagram`] to its wire bytes.
pub fn pack(d: &MediaDatagram) -> Bytes {
    let mut buf = BytesMut::with_capacity(8 + d.payload.len());
    buf.put_u8(1); // ver
    buf.put_u8(d.flags);
    buf.put_u16(d.stream_local_id);
    buf.put_u32(d.seq);
    buf.put(&d.payload[..]);
    buf.freeze()
}

/// Parse a wire-bytes datagram back to [`MediaDatagram`].
///
/// Returns [`SubstrateError::InvalidDatagram`] on:
/// - length < 8 bytes
/// - `ver` byte != 1
pub fn unpack(input: &[u8]) -> Result<MediaDatagram, SubstrateError> {
    if input.len() < 8 {
        return Err(SubstrateError::InvalidDatagram("length < 8"));
    }
    let mut b = input;
    let ver = b.get_u8();
    if ver != 1 {
        return Err(SubstrateError::InvalidDatagram("ver != 1"));
    }
    let flags = b.get_u8();
    let stream_local_id = b.get_u16();
    let seq = b.get_u32();
    let payload = Bytes::copy_from_slice(b);
    Ok(MediaDatagram {
        flags,
        stream_local_id,
        seq,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let d = MediaDatagram {
            flags: 0,
            stream_local_id: 7,
            seq: 12345,
            payload: Bytes::from_static(b"\x80\x60\x00\x01rtp-body"),
        };
        let bytes = pack(&d);
        let d2 = unpack(&bytes).unwrap();
        assert_eq!(d, d2);
    }

    #[test]
    fn unpack_rejects_short_input() {
        let err = unpack(b"abc").unwrap_err();
        matches!(err, SubstrateError::InvalidDatagram(_));
    }

    #[test]
    fn unpack_rejects_wrong_version() {
        let mut bad = vec![0u8; 8];
        bad[0] = 9; // bad ver
        let err = unpack(&bad).unwrap_err();
        matches!(err, SubstrateError::InvalidDatagram(_));
    }
}
