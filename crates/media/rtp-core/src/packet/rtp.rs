#[cfg(test)]
use super::header::RTP_MIN_HEADER_SIZE;
use bytes::{Bytes, BytesMut};
use std::fmt;
use tracing::debug;

use super::header::RtpHeader;
use crate::error::Error;
use crate::{Result, RtpSequenceNumber, RtpSsrc, RtpTimestamp};

/// Given the full packet bytes and the size of the RTP header already
/// parsed out of them, returns how many of the remaining bytes are actual
/// payload, accounting for RTP padding (RFC 3550 section 5.1).
///
/// When `header.padding` is false, this is just every byte after the
/// header. When it's true, at least one byte must follow the header: the
/// padding octet count, which is itself included in that count, must be
/// nonzero, and must not exceed the bytes available. A packet with P=1 and
/// nothing after the header, or a header.padding value that doesn't
/// satisfy those constraints, is malformed.
fn payload_len_without_padding(
    header: &RtpHeader,
    data: &[u8],
    header_size: usize,
) -> Result<usize> {
    let raw_len = data.len().saturating_sub(header_size);
    if !header.padding {
        return Ok(raw_len);
    }

    if raw_len == 0 {
        return Err(Error::InvalidPacket(
            "RTP padding bit set but no bytes follow the header".to_string(),
        ));
    }

    let padding_len = data[data.len() - 1] as usize;
    if padding_len == 0 {
        return Err(Error::InvalidPacket(
            "RTP padding bit set but padding octet count is zero".to_string(),
        ));
    }
    if padding_len > raw_len {
        return Err(Error::InvalidPacket(format!(
            "RTP padding length {padding_len} exceeds available payload bytes {raw_len}"
        )));
    }

    Ok(raw_len - padding_len)
}

/// An RTP packet with header and payload
#[derive(Clone, PartialEq, Eq)]
pub struct RtpPacket {
    /// RTP header
    pub header: RtpHeader,

    /// Payload data. When parsed from the wire, this never includes RTP
    /// padding (RFC 3550 section 5.1): [`Self::parse`] and
    /// [`Self::parse_from_bytes`] strip the padding octets and the trailing
    /// padding-length octet before returning.
    pub payload: Bytes,
}

impl RtpPacket {
    /// Create a new RTP packet with the given header and payload
    pub fn new(header: RtpHeader, payload: Bytes) -> Self {
        Self { header, payload }
    }

    /// Create a new RTP packet with the standard header fields and payload
    pub fn new_with_payload(
        payload_type: u8,
        sequence_number: RtpSequenceNumber,
        timestamp: RtpTimestamp,
        ssrc: RtpSsrc,
        payload: Bytes,
    ) -> Self {
        let header = RtpHeader::new(payload_type, sequence_number, timestamp, ssrc);
        Self { header, payload }
    }

    /// Get the total size of the packet in bytes
    pub fn size(&self) -> usize {
        self.header.size() + self.payload.len()
    }

    /// Parse an RTP packet from bytes.
    ///
    /// Allocates a fresh `Bytes` for the payload (one `copy_from_slice`).
    pub fn parse(data: &[u8]) -> Result<Self> {
        debug!("Parsing RTP packet from {} bytes", data.len());

        // Parse the header without consuming the buffer
        let (mut header, header_size) = RtpHeader::parse_without_consuming(data)?;
        debug!("Parsed header of size {}", header_size);

        // Extract the payload
        let payload_len = payload_len_without_padding(&header, data, header_size)?;
        header.padding = false; // payload below has any padding stripped, so the header should say so
        let payload = Bytes::copy_from_slice(&data[header_size..header_size + payload_len]);
        debug!("Extracted payload of size {}", payload.len());

        Ok(Self { header, payload })
    }

    /// Parse an RTP packet from an owned `Bytes`, slicing the payload as a
    /// refcounted view without copying.
    pub fn parse_from_bytes(data: Bytes) -> Result<Self> {
        debug!("Parsing RTP packet from {} bytes (zero-copy)", data.len());

        let (mut header, header_size) = RtpHeader::parse_without_consuming(&data)?;
        debug!("Parsed header of size {}", header_size);

        // Zero-copy slice: `Bytes::slice` only bumps the underlying
        // refcount, no allocation.
        let payload_len = payload_len_without_padding(&header, &data, header_size)?;
        header.padding = false; // payload below has any padding stripped, so the header should say so
        let payload = data.slice(header_size..header_size + payload_len);
        debug!("Sliced payload of size {}", payload.len());

        Ok(Self { header, payload })
    }

    /// Serialize the packet to bytes.
    ///
    /// Allocates a fresh `BytesMut` per call and freezes it directly.
    /// Hot paths that send many packets should prefer
    /// [`Self::serialize_into`] with a per-task buffer to amortise the
    /// allocation across calls.
    pub fn serialize(&self) -> Result<Bytes> {
        let total_size = self.size();
        let mut buf = BytesMut::with_capacity(total_size);
        self.header.serialize(&mut buf)?;
        buf.extend_from_slice(&self.payload);
        Ok(buf.freeze())
    }

    /// Serialize the packet into the caller-supplied `BytesMut`.
    ///
    /// Returns a `Bytes` view over just the freshly written region by
    /// splitting `buf`. The remaining capacity stays with `buf` and is
    /// reusable on the next call — when nobody holds the returned
    /// `Bytes` any more, `BytesMut` can reclaim the backing
    /// allocation, so a per-task `BytesMut` amortises the allocation
    /// across many packets. This is the zero-alloc-steady-state shape
    /// we want on the UDP send hot path.
    ///
    /// The buffer is grown if it does not already have enough capacity
    /// for the packet. For single-shot use, prefer the allocating
    /// [`Self::serialize`] — `split` on an unshared `BytesMut`
    /// performs an internal reallocation that only pays off when the
    /// buffer is reused across repeated calls.
    pub fn serialize_into(&self, buf: &mut BytesMut) -> Result<Bytes> {
        let total_size = self.size();
        buf.reserve(total_size);

        // Serialize the header
        self.header.serialize(buf)?;

        // Add the payload
        buf.extend_from_slice(&self.payload);

        // Split off exactly the bytes we wrote and freeze them into an
        // immutable Bytes view. `buf` retains any leftover capacity for
        // the next packet.
        Ok(buf.split().freeze())
    }
}

impl fmt::Debug for RtpPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RtpPacket {{ header: {:?}, payload_len: {} }}",
            self.header,
            self.payload.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::extension::RtpHeaderExtensions;
    use bytes::Bytes;

    #[test]
    fn test_new_with_payload() {
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new_with_payload(
            96,         // Payload type
            1000,       // Sequence number
            12345,      // Timestamp
            0xabcdef01, // SSRC
            payload.clone(),
        );

        assert_eq!(packet.header.payload_type, 96);
        assert_eq!(packet.header.sequence_number, 1000);
        assert_eq!(packet.header.timestamp, 12345);
        assert_eq!(packet.header.ssrc, 0xabcdef01);
        assert_eq!(packet.payload, payload);
    }

    #[test]
    fn test_size() {
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, payload);

        assert_eq!(packet.size(), RTP_MIN_HEADER_SIZE + 12); // 12 bytes payload
    }

    #[test]
    fn test_serialize_parse_roundtrip() {
        let payload = Bytes::from_static(b"test payload data");
        let original = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, payload);

        // Serialize
        let serialized = original.serialize().unwrap();

        // Parse
        let parsed = RtpPacket::parse(&serialized).unwrap();

        // Verify
        assert_eq!(parsed.header.payload_type, original.header.payload_type);
        assert_eq!(
            parsed.header.sequence_number,
            original.header.sequence_number
        );
        assert_eq!(parsed.header.timestamp, original.header.timestamp);
        assert_eq!(parsed.header.ssrc, original.header.ssrc);
        assert_eq!(parsed.payload, original.payload);
    }

    #[test]
    fn test_parse_from_bytes_matches_slice_parse() {
        let payload = Bytes::from_static(b"test payload data");
        let original = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, payload);
        let serialized = original.serialize().unwrap();

        let parsed_from_slice = RtpPacket::parse(&serialized).unwrap();
        let parsed_from_bytes = RtpPacket::parse_from_bytes(serialized).unwrap();

        assert_eq!(parsed_from_bytes, parsed_from_slice);
    }

    #[test]
    fn test_serialize_into_writes_one_payload() {
        let payload = Bytes::from_static(b"abc123");
        let packet = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, payload.clone());
        let mut reusable = BytesMut::with_capacity(1500);

        let serialized = packet.serialize_into(&mut reusable).unwrap();

        assert_eq!(serialized.len(), RTP_MIN_HEADER_SIZE + payload.len());
        assert_eq!(&serialized[RTP_MIN_HEADER_SIZE..], payload.as_ref());
    }

    #[test]
    fn test_debug_format() {
        let packet = RtpPacket::new_with_payload(
            96,
            1000,
            12345,
            0xabcdef01,
            Bytes::from_static(b"test payload"),
        );

        let debug_str = format!("{:?}", packet);
        assert!(debug_str.contains("payload_len: 12"));
        assert!(debug_str.contains("header:"));
    }

    /// Serializes `header` followed by `media`, and, if `padding_octets` is
    /// `Some`, RFC 3550 section 5.1 padding: `padding_octets - 1` zero bytes
    /// followed by the count byte itself (`padding_octets`). Sets
    /// `header.padding` to match. `padding_octets` must be >= 1 when given,
    /// since the count byte counts itself.
    fn build_raw_packet(mut header: RtpHeader, media: &[u8], padding_octets: Option<u8>) -> Bytes {
        header.padding = padding_octets.is_some();
        let mut buf = BytesMut::new();
        header.serialize(&mut buf).unwrap();
        buf.extend_from_slice(media);
        if let Some(count) = padding_octets {
            assert!(count >= 1, "padding octet count must include itself");
            buf.extend(std::iter::repeat(0u8).take((count - 1) as usize));
            buf.extend_from_slice(&[count]);
        }
        buf.freeze()
    }

    fn plain_header() -> RtpHeader {
        RtpHeader::new(96, 1000, 12345, 0xabcdef01)
    }

    #[test]
    fn parse_packet_without_padding_bit_returns_full_payload_unchanged() {
        let raw = build_raw_packet(plain_header(), b"media bytes", None);
        let packet = RtpPacket::parse(&raw).unwrap();

        assert_eq!(packet.payload, Bytes::from_static(b"media bytes"));
        assert!(!packet.header.padding);
    }

    #[test]
    fn parse_strips_valid_padding_from_the_payload() {
        let raw = build_raw_packet(plain_header(), b"media", Some(4));
        let packet = RtpPacket::parse(&raw).unwrap();

        assert_eq!(packet.payload, Bytes::from_static(b"media"));
        assert!(
            !packet.header.padding,
            "padding has been stripped, header should no longer claim it's present"
        );
    }

    #[test]
    fn parse_from_bytes_strips_valid_padding_the_same_way_as_parse() {
        let raw = build_raw_packet(plain_header(), b"media", Some(4));

        let via_slice = RtpPacket::parse(&raw).unwrap();
        let via_bytes = RtpPacket::parse_from_bytes(raw).unwrap();

        assert_eq!(via_bytes, via_slice);
    }

    #[test]
    fn parse_rejects_padding_length_larger_than_the_payload() {
        // Hand-craft a P=1 packet whose count byte claims more padding than
        // bytes are actually available, without going through
        // build_raw_packet's own bookkeeping.
        let mut header = plain_header();
        header.padding = true;
        let mut buf = BytesMut::new();
        header.serialize(&mut buf).unwrap();
        buf.extend_from_slice(&[0xAA, 0xBB, 200]); // claims 200 bytes of padding, only 3 present
        let raw = buf.freeze();

        assert!(RtpPacket::parse(&raw).is_err());
        assert!(RtpPacket::parse_from_bytes(raw).is_err());
    }

    #[test]
    fn parse_rejects_padding_bit_set_with_zero_count() {
        let mut header = plain_header();
        header.padding = true;
        let mut buf = BytesMut::new();
        header.serialize(&mut buf).unwrap();
        buf.extend_from_slice(&[0xAA, 0xBB, 0]); // P=1 but count byte is 0
        let raw = buf.freeze();

        assert!(RtpPacket::parse(&raw).is_err());
        assert!(RtpPacket::parse_from_bytes(raw).is_err());
    }

    #[test]
    fn parse_rejects_padding_bit_without_padding_count_octet() {
        // P=1 requires at least one byte after the header (the padding
        // count, itself included). A packet that ends exactly at the
        // header boundary with P=1 is malformed, not a valid empty payload.
        let mut header = plain_header();
        header.padding = true;

        let mut raw = BytesMut::new();
        header.serialize(&mut raw).unwrap();

        assert!(RtpPacket::parse(&raw).is_err());
        assert!(RtpPacket::parse_from_bytes(raw.freeze()).is_err());
    }

    #[test]
    fn parse_handles_csrc_and_extension_and_padding_together() {
        let mut header = plain_header();
        header.csrc = vec![0x1111_1111, 0x2222_2222];
        header.cc = header.csrc.len() as u8;
        header.extension = true;
        let mut extensions = RtpHeaderExtensions::new_one_byte();
        extensions
            .add_extension(1, Bytes::from_static(&[0xAA]))
            .unwrap();
        header.extensions = Some(extensions);

        let raw = build_raw_packet(header, b"media", Some(4));
        let packet = RtpPacket::parse(&raw).unwrap();

        assert_eq!(packet.header.csrc, vec![0x1111_1111, 0x2222_2222]);
        assert!(packet.header.extension);
        assert!(packet.header.extensions.is_some());
        assert_eq!(packet.payload, Bytes::from_static(b"media"));
        assert!(!packet.header.padding);
    }

    #[test]
    fn padded_packet_round_trips_through_serialize_as_an_unpadded_packet() {
        // parse() strips padding and clears header.padding, so the
        // resulting RtpPacket is self-consistent: serializing it again
        // produces a packet with P=0 and just the media bytes, not a
        // packet that claims padding it no longer carries.
        let raw = build_raw_packet(plain_header(), b"media", Some(4));
        let packet = RtpPacket::parse(&raw).unwrap();

        let reserialized = packet.serialize().unwrap();
        let reparsed = RtpPacket::parse(&reserialized).unwrap();

        assert!(!reparsed.header.padding);
        assert_eq!(reparsed.payload, Bytes::from_static(b"media"));
    }
}
