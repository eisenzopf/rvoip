//! Minimal SCTP chunk encoding and decoding.
//!
//! Only the chunk types needed for WebRTC Data Channel operation are
//! implemented: INIT, INIT-ACK, DATA, SACK, SHUTDOWN, SHUTDOWN-ACK,
//! and COOKIE-ECHO / COOKIE-ACK for the four-way handshake.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use crate::error::Error;

// ---------- Chunk type constants ----------

/// DATA chunk
pub const CHUNK_DATA: u8 = 0x00;
/// INIT chunk
pub const CHUNK_INIT: u8 = 0x01;
/// INIT-ACK chunk
pub const CHUNK_INIT_ACK: u8 = 0x02;
/// SACK chunk
pub const CHUNK_SACK: u8 = 0x03;
/// COOKIE-ECHO chunk
pub const CHUNK_COOKIE_ECHO: u8 = 0x0A;
/// COOKIE-ACK chunk
pub const CHUNK_COOKIE_ACK: u8 = 0x0B;
/// SHUTDOWN chunk
pub const CHUNK_SHUTDOWN: u8 = 0x07;
/// SHUTDOWN-ACK chunk
pub const CHUNK_SHUTDOWN_ACK: u8 = 0x08;

/// WebRTC Data Channel Establishment Protocol (DCEP) payload protocol ID
pub const PPID_DCEP: u32 = 50;
/// WebRTC String payload protocol ID (UTF-8)
pub const PPID_STRING: u32 = 51;
/// WebRTC Binary payload protocol ID
pub const PPID_BINARY: u32 = 53;
/// WebRTC String Empty payload protocol ID
pub const PPID_STRING_EMPTY: u32 = 56;
/// WebRTC Binary Empty payload protocol ID
pub const PPID_BINARY_EMPTY: u32 = 57;

// ---------- SCTP common header (12 bytes) ----------

/// Minimal SCTP packet header (source port, dest port, verification tag, checksum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SctpHeader {
    pub source_port: u16,
    pub dest_port: u16,
    pub verification_tag: u32,
    pub checksum: u32,
}

impl SctpHeader {
    /// Serialise to 12 bytes.
    pub fn serialize(&self) -> BytesMut {
        let mut buf = BytesMut::with_capacity(12);
        buf.put_u16(self.source_port);
        buf.put_u16(self.dest_port);
        buf.put_u32(self.verification_tag);
        buf.put_u32(self.checksum);
        buf
    }

    /// Parse from at least 12 bytes.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 12 {
            return Err(Error::SctpError("SCTP header too short".to_string()));
        }
        let mut cursor = &data[..12];
        let source_port = cursor.get_u16();
        let dest_port = cursor.get_u16();
        let verification_tag = cursor.get_u32();
        let checksum = cursor.get_u32();
        Ok(Self { source_port, dest_port, verification_tag, checksum })
    }
}

// ---------- Chunk envelope ----------

/// Generic SCTP chunk (type + flags + length + value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunk {
    pub chunk_type: u8,
    pub flags: u8,
    pub value: Bytes,
}

impl RawChunk {
    /// Serialize a chunk (4-byte header + value, padded to 4-byte boundary).
    pub fn serialize(&self) -> BytesMut {
        let value_len = self.value.len();
        let chunk_len = 4 + value_len;
        let padded = (chunk_len + 3) & !3;
        let mut buf = BytesMut::with_capacity(padded);
        buf.put_u8(self.chunk_type);
        buf.put_u8(self.flags);
        buf.put_u16(chunk_len as u16);
        buf.put_slice(&self.value);
        // Pad to 4-byte boundary
        for _ in 0..(padded - chunk_len) {
            buf.put_u8(0);
        }
        buf
    }

    /// Parse one chunk from `data`. Returns the chunk and the number of bytes consumed.
    pub fn parse(data: &[u8]) -> Result<(Self, usize), Error> {
        if data.len() < 4 {
            return Err(Error::SctpError("Chunk header too short".to_string()));
        }
        let chunk_type = data[0];
        let flags = data[1];
        let chunk_len = u16::from_be_bytes([data[2], data[3]]) as usize;
        if chunk_len < 4 || data.len() < chunk_len {
            return Err(Error::SctpError(format!(
                "Invalid chunk length {}, data len {}",
                chunk_len,
                data.len()
            )));
        }
        let value = Bytes::copy_from_slice(&data[4..chunk_len]);
        let consumed = (chunk_len + 3) & !3; // account for padding
        let consumed = consumed.min(data.len()); // don't overshoot
        Ok((Self { chunk_type, flags, value }, consumed))
    }

    /// Parse all chunks from a payload (after the 12-byte SCTP header).
    pub fn parse_all(data: &[u8]) -> Result<Vec<Self>, Error> {
        let mut chunks = Vec::new();
        let mut offset = 0;
        while offset < data.len() {
            if data.len() - offset < 4 {
                break; // trailing padding
            }
            let (chunk, consumed) = Self::parse(&data[offset..])?;
            chunks.push(chunk);
            offset += consumed;
        }
        Ok(chunks)
    }
}

// ---------- Typed chunk helpers ----------

/// INIT / INIT-ACK parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitChunk {
    pub initiate_tag: u32,
    pub a_rwnd: u32,
    pub num_outbound_streams: u16,
    pub num_inbound_streams: u16,
    pub initial_tsn: u32,
    /// Opaque cookie (only present in INIT-ACK).
    pub cookie: Option<Bytes>,
}

impl InitChunk {
    /// Encode to a `RawChunk` value (without the cookie for INIT, with cookie for INIT-ACK).
    pub fn to_raw(&self, is_ack: bool) -> RawChunk {
        let mut buf = BytesMut::with_capacity(20);
        buf.put_u32(self.initiate_tag);
        buf.put_u32(self.a_rwnd);
        buf.put_u16(self.num_outbound_streams);
        buf.put_u16(self.num_inbound_streams);
        buf.put_u32(self.initial_tsn);
        // Append cookie as optional TLV parameter (type 0x0007)
        if let Some(ref cookie) = self.cookie {
            let param_len = 4 + cookie.len();
            let padded = (param_len + 3) & !3;
            buf.put_u16(0x0007); // State Cookie parameter type
            buf.put_u16(param_len as u16);
            buf.put_slice(cookie);
            for _ in 0..(padded - param_len) {
                buf.put_u8(0);
            }
        }
        RawChunk {
            chunk_type: if is_ack { CHUNK_INIT_ACK } else { CHUNK_INIT },
            flags: 0,
            value: buf.freeze(),
        }
    }

    /// Parse from `RawChunk` value bytes.
    pub fn from_raw(value: &[u8]) -> Result<Self, Error> {
        if value.len() < 16 {
            return Err(Error::SctpError("INIT chunk too short".to_string()));
        }
        let mut cursor = &value[..];
        let initiate_tag = cursor.get_u32();
        let a_rwnd = cursor.get_u32();
        let num_outbound_streams = cursor.get_u16();
        let num_inbound_streams = cursor.get_u16();
        let initial_tsn = cursor.get_u32();

        // Look for State Cookie parameter (type 0x0007) in remaining bytes
        let mut cookie = None;
        let mut offset = 16;
        while offset + 4 <= value.len() {
            let param_type = u16::from_be_bytes([value[offset], value[offset + 1]]);
            let param_len = u16::from_be_bytes([value[offset + 2], value[offset + 3]]) as usize;
            if param_len < 4 || offset + param_len > value.len() {
                break;
            }
            if param_type == 0x0007 {
                cookie = Some(Bytes::copy_from_slice(&value[offset + 4..offset + param_len]));
            }
            offset += (param_len + 3) & !3;
        }

        Ok(Self {
            initiate_tag,
            a_rwnd,
            num_outbound_streams,
            num_inbound_streams,
            initial_tsn,
            cookie,
        })
    }
}

/// DATA chunk header fields (after the generic chunk header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataChunkHeader {
    pub tsn: u32,
    pub stream_id: u16,
    pub stream_seq: u16,
    pub protocol_id: u32,
}

impl DataChunkHeader {
    /// Flags for a complete, unfragmented message.
    pub const FLAG_BEGIN: u8 = 0x02;
    pub const FLAG_END: u8 = 0x01;
    pub const FLAG_UNORDERED: u8 = 0x04;

    /// Build a `RawChunk` for a DATA chunk carrying `payload`.
    pub fn to_raw(&self, payload: &[u8], ordered: bool) -> RawChunk {
        let mut buf = BytesMut::with_capacity(12 + payload.len());
        buf.put_u32(self.tsn);
        buf.put_u16(self.stream_id);
        buf.put_u16(self.stream_seq);
        buf.put_u32(self.protocol_id);
        buf.put_slice(payload);
        let flags = Self::FLAG_BEGIN
            | Self::FLAG_END
            | if ordered { 0 } else { Self::FLAG_UNORDERED };
        RawChunk {
            chunk_type: CHUNK_DATA,
            flags,
            value: buf.freeze(),
        }
    }

    /// Parse from `RawChunk` value bytes. Returns the header and the user payload.
    pub fn from_raw(value: &[u8]) -> Result<(Self, Bytes), Error> {
        if value.len() < 12 {
            return Err(Error::SctpError("DATA chunk value too short".to_string()));
        }
        let mut cursor = &value[..12];
        let tsn = cursor.get_u32();
        let stream_id = cursor.get_u16();
        let stream_seq = cursor.get_u16();
        let protocol_id = cursor.get_u32();
        let payload = Bytes::copy_from_slice(&value[12..]);
        Ok((
            Self { tsn, stream_id, stream_seq, protocol_id },
            payload,
        ))
    }
}

/// SACK chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SackChunk {
    pub cumulative_tsn_ack: u32,
    pub a_rwnd: u32,
    pub gap_ack_blocks: Vec<(u16, u16)>,
    pub duplicate_tsns: Vec<u32>,
}

impl SackChunk {
    /// Build a `RawChunk`.
    pub fn to_raw(&self) -> RawChunk {
        let num_gap = self.gap_ack_blocks.len() as u16;
        let num_dup = self.duplicate_tsns.len() as u16;
        let len = 12 + 4 * num_gap as usize + 4 * num_dup as usize;
        let mut buf = BytesMut::with_capacity(len);
        buf.put_u32(self.cumulative_tsn_ack);
        buf.put_u32(self.a_rwnd);
        buf.put_u16(num_gap);
        buf.put_u16(num_dup);
        for &(start, end) in &self.gap_ack_blocks {
            buf.put_u16(start);
            buf.put_u16(end);
        }
        for &tsn in &self.duplicate_tsns {
            buf.put_u32(tsn);
        }
        RawChunk {
            chunk_type: CHUNK_SACK,
            flags: 0,
            value: buf.freeze(),
        }
    }

    /// Parse from `RawChunk` value bytes.
    pub fn from_raw(value: &[u8]) -> Result<Self, Error> {
        if value.len() < 12 {
            return Err(Error::SctpError("SACK chunk too short".to_string()));
        }
        let mut cursor = &value[..];
        let cumulative_tsn_ack = cursor.get_u32();
        let a_rwnd = cursor.get_u32();
        let num_gap = cursor.get_u16() as usize;
        let num_dup = cursor.get_u16() as usize;
        let expected = 12 + 4 * num_gap + 4 * num_dup;
        if value.len() < expected {
            return Err(Error::SctpError("SACK chunk too short for gap/dup counts".to_string()));
        }
        let mut gap_ack_blocks = Vec::with_capacity(num_gap);
        for _ in 0..num_gap {
            let start = cursor.get_u16();
            let end = cursor.get_u16();
            gap_ack_blocks.push((start, end));
        }
        let mut duplicate_tsns = Vec::with_capacity(num_dup);
        for _ in 0..num_dup {
            duplicate_tsns.push(cursor.get_u32());
        }
        Ok(Self { cumulative_tsn_ack, a_rwnd, gap_ack_blocks, duplicate_tsns })
    }
}

/// Encode a full SCTP packet (header + chunks).
pub fn encode_packet(header: &SctpHeader, chunks: &[RawChunk]) -> Bytes {
    let mut buf = header.serialize();
    for chunk in chunks {
        buf.extend_from_slice(&chunk.serialize());
    }
    // CRC32c checksum -- for DTLS transport the checksum can be zero per
    // RFC 8261 Section 4.1 ("When SCTP packets are sent over DTLS the
    // SCTP checksum ... SHOULD be set to zero"). We leave it zero for now.
    buf.freeze()
}

/// Parse a full SCTP packet into header + chunks.
pub fn decode_packet(data: &[u8]) -> Result<(SctpHeader, Vec<RawChunk>), Error> {
    let header = SctpHeader::parse(data)?;
    let chunks = if data.len() > 12 {
        RawChunk::parse_all(&data[12..])?
    } else {
        Vec::new()
    };
    Ok((header, chunks))
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sctp_header_roundtrip() {
        let hdr = SctpHeader {
            source_port: 5000,
            dest_port: 5000,
            verification_tag: 0xDEADBEEF,
            checksum: 0,
        };
        let bytes = hdr.serialize();
        let parsed = SctpHeader::parse(&bytes).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(hdr, parsed);
    }

    #[test]
    fn test_raw_chunk_roundtrip() {
        let chunk = RawChunk {
            chunk_type: CHUNK_DATA,
            flags: 0x03,
            value: Bytes::from_static(b"hello"),
        };
        let encoded = chunk.serialize();
        let (decoded, consumed) = RawChunk::parse(&encoded).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(decoded.chunk_type, chunk.chunk_type);
        assert_eq!(decoded.flags, chunk.flags);
        assert_eq!(decoded.value, chunk.value);
        // 4 header + 5 value = 9, padded to 12
        assert_eq!(consumed, 12);
    }

    #[test]
    fn test_init_chunk_roundtrip() {
        let init = InitChunk {
            initiate_tag: 0x12345678,
            a_rwnd: 65535,
            num_outbound_streams: 1,
            num_inbound_streams: 1,
            initial_tsn: 1,
            cookie: None,
        };
        let raw = init.to_raw(false);
        assert_eq!(raw.chunk_type, CHUNK_INIT);
        let parsed = InitChunk::from_raw(&raw.value).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(parsed.initiate_tag, init.initiate_tag);
        assert_eq!(parsed.initial_tsn, init.initial_tsn);
    }

    #[test]
    fn test_init_ack_with_cookie_roundtrip() {
        let init_ack = InitChunk {
            initiate_tag: 0xAABBCCDD,
            a_rwnd: 65535,
            num_outbound_streams: 1,
            num_inbound_streams: 1,
            initial_tsn: 100,
            cookie: Some(Bytes::from_static(b"secret-cookie")),
        };
        let raw = init_ack.to_raw(true);
        assert_eq!(raw.chunk_type, CHUNK_INIT_ACK);
        let parsed = InitChunk::from_raw(&raw.value).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(parsed.cookie.as_deref(), Some(b"secret-cookie".as_slice()));
    }

    #[test]
    fn test_data_chunk_roundtrip() {
        let hdr = DataChunkHeader {
            tsn: 42,
            stream_id: 1,
            stream_seq: 0,
            protocol_id: PPID_STRING,
        };
        let payload = b"hello world";
        let raw = hdr.to_raw(payload, true);
        assert_eq!(raw.chunk_type, CHUNK_DATA);
        assert_eq!(raw.flags & DataChunkHeader::FLAG_BEGIN, DataChunkHeader::FLAG_BEGIN);
        assert_eq!(raw.flags & DataChunkHeader::FLAG_END, DataChunkHeader::FLAG_END);
        assert_eq!(raw.flags & DataChunkHeader::FLAG_UNORDERED, 0);

        let (parsed_hdr, parsed_payload) =
            DataChunkHeader::from_raw(&raw.value).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(parsed_hdr.tsn, 42);
        assert_eq!(parsed_hdr.stream_id, 1);
        assert_eq!(parsed_hdr.protocol_id, PPID_STRING);
        assert_eq!(parsed_payload.as_ref(), payload);
    }

    #[test]
    fn test_sack_chunk_roundtrip() {
        let sack = SackChunk {
            cumulative_tsn_ack: 10,
            a_rwnd: 65535,
            gap_ack_blocks: vec![(2, 3)],
            duplicate_tsns: vec![5],
        };
        let raw = sack.to_raw();
        let parsed = SackChunk::from_raw(&raw.value).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(parsed.cumulative_tsn_ack, 10);
        assert_eq!(parsed.gap_ack_blocks, vec![(2, 3)]);
        assert_eq!(parsed.duplicate_tsns, vec![5]);
    }

    #[test]
    fn test_full_packet_roundtrip() {
        let header = SctpHeader {
            source_port: 5000,
            dest_port: 5000,
            verification_tag: 0x12345678,
            checksum: 0,
        };
        let data_hdr = DataChunkHeader {
            tsn: 1,
            stream_id: 0,
            stream_seq: 0,
            protocol_id: PPID_BINARY,
        };
        let chunk = data_hdr.to_raw(b"test", true);
        let packet = encode_packet(&header, &[chunk]);
        let (parsed_header, parsed_chunks) =
            decode_packet(&packet).unwrap_or_else(|e| panic!("decode failed: {e}"));
        assert_eq!(parsed_header.source_port, 5000);
        assert_eq!(parsed_chunks.len(), 1);
        assert_eq!(parsed_chunks[0].chunk_type, CHUNK_DATA);
    }

    #[test]
    fn test_multiple_chunks_parse() {
        let c1 = RawChunk { chunk_type: CHUNK_DATA, flags: 0x03, value: Bytes::from_static(b"ab") };
        let c2 = SackChunk {
            cumulative_tsn_ack: 1,
            a_rwnd: 65535,
            gap_ack_blocks: vec![],
            duplicate_tsns: vec![],
        }.to_raw();
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&c1.serialize());
        buf.extend_from_slice(&c2.serialize());
        let chunks = RawChunk::parse_all(&buf).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chunk_type, CHUNK_DATA);
        assert_eq!(chunks[1].chunk_type, CHUNK_SACK);
    }
}
