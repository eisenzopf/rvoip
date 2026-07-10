//! UCTP media datagram framing per CONVERSATION_PROTOCOL.md §10.1.
//!
//! 8-byte header `ver | flags | stream_local_id (u16 BE) | datagram_seq (u32 BE)`
//! followed by the RTP packet body. `pack`/`unpack` do not parse the RTP
//! body — they treat it as opaque bytes.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::errors::SubstrateError;

/// RTP fields recovered from a UCTP media datagram body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtpMediaPayload {
    pub payload: Bytes,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

/// Construct the complete RTP packet required by UCTP §10.1.
pub fn pack_rtp(
    payload: Bytes,
    payload_type: u8,
    sequence_number: u16,
    timestamp: u32,
    ssrc: u32,
) -> Result<Bytes, SubstrateError> {
    rvoip_rtp_core::RtpPacket::new_with_payload(
        payload_type,
        sequence_number,
        timestamp,
        ssrc,
        payload,
    )
    .serialize()
    .map_err(|_| SubstrateError::InvalidDatagram("RTP serialization failed"))
}

/// Parse and validate the complete RTP packet carried after the UCTP header.
pub fn unpack_rtp(payload: Bytes) -> Result<RtpMediaPayload, SubstrateError> {
    let packet = rvoip_rtp_core::RtpPacket::parse_from_bytes(payload)
        .map_err(|_| SubstrateError::InvalidDatagram("invalid RTP packet"))?;
    Ok(RtpMediaPayload {
        payload: packet.payload,
        payload_type: packet.header.payload_type,
        sequence_number: packet.header.sequence_number,
        timestamp: packet.header.timestamp,
        ssrc: packet.header.ssrc,
    })
}

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
    fn rtp_body_roundtrip_is_a_complete_packet() {
        let payload = Bytes::from_static(b"opus-frame");
        let rtp = pack_rtp(payload.clone(), 111, 7, 9_600, 0x1234_5678).unwrap();
        assert_eq!(rtp[0] >> 6, 2, "RTP version must be two");
        let parsed = unpack_rtp(rtp).unwrap();
        assert_eq!(parsed.payload, payload);
        assert_eq!(parsed.payload_type, 111);
        assert_eq!(parsed.sequence_number, 7);
        assert_eq!(parsed.timestamp, 9_600);
        assert_eq!(parsed.ssrc, 0x1234_5678);
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
