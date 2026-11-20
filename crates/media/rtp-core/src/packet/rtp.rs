#[cfg(test)]
use super::header::RTP_MIN_HEADER_SIZE;
use bytes::{Bytes, BytesMut};
use std::fmt;
use tracing::debug;

use super::header::RtpHeader;
use crate::{Result, RtpSequenceNumber, RtpSsrc, RtpTimestamp};

/// An RTP packet with header and payload
#[derive(Clone, PartialEq, Eq)]
pub struct RtpPacket {
    /// RTP header
    pub header: RtpHeader,

    /// Payload data
    pub payload: Bytes,

    /// Number of RTP padding octets on the wire.
    ///
    /// Padding is not included in [`Self::payload`]. A non-zero value must
    /// agree with [`RtpHeader::padding`]; the final serialized padding octet
    /// contains this value as required by RFC 3550.
    pub padding_size: u8,
}

impl RtpPacket {
    /// Create a new RTP packet with the given header and payload
    pub fn new(header: RtpHeader, payload: Bytes) -> Self {
        Self {
            header,
            payload,
            padding_size: 0,
        }
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
        Self {
            header,
            payload,
            padding_size: 0,
        }
    }

    /// Configure the number of padding octets to write on the wire.
    pub fn set_padding(&mut self, padding_size: u8) {
        self.padding_size = padding_size;
        self.header.padding = padding_size != 0;
    }

    /// Remove RTP padding from the packet.
    pub fn clear_padding(&mut self) {
        self.set_padding(0);
    }

    /// Get the total size of the packet in bytes
    pub fn size(&self) -> usize {
        self.header.size() + self.payload.len() + self.padding_size as usize
    }

    fn payload_bounds(data: &[u8], header_size: usize, has_padding: bool) -> Result<(usize, u8)> {
        if !has_padding {
            return Ok((data.len(), 0));
        }

        if data.len() <= header_size {
            return Err(crate::Error::InvalidPacket(
                "RTP padding flag is set but the packet has no padding octets".to_string(),
            ));
        }

        let padding_size = data[data.len() - 1];
        if padding_size == 0 {
            return Err(crate::Error::InvalidPacket(
                "RTP padding length must be non-zero".to_string(),
            ));
        }

        let available = data.len() - header_size;
        if padding_size as usize > available {
            return Err(crate::Error::InvalidPacket(format!(
                "RTP padding length {} exceeds {} available payload octets",
                padding_size, available
            )));
        }

        Ok((data.len() - padding_size as usize, padding_size))
    }

    fn validate_padding(&self) -> Result<()> {
        if self.header.padding != (self.padding_size != 0) {
            return Err(crate::Error::InvalidParameter(format!(
                "RTP padding flag ({}) does not match padding length ({})",
                self.header.padding, self.padding_size
            )));
        }
        Ok(())
    }

    fn serialize_padding(&self, buf: &mut BytesMut) {
        if self.padding_size == 0 {
            return;
        }

        for _ in 1..self.padding_size {
            buf.extend_from_slice(&[0]);
        }
        buf.extend_from_slice(&[self.padding_size]);
    }

    /// Parse an RTP packet from bytes.
    ///
    /// Allocates a fresh `Bytes` for the payload (one `copy_from_slice`).
    pub fn parse(data: &[u8]) -> Result<Self> {
        debug!("Parsing RTP packet from {} bytes", data.len());

        // Parse the header without consuming the buffer
        let (header, header_size) = RtpHeader::parse_without_consuming(data)?;
        debug!("Parsed header of size {}", header_size);

        let (payload_end, padding_size) = Self::payload_bounds(data, header_size, header.padding)?;

        // Extract the payload without the RTP padding.
        let payload = if payload_end > header_size {
            Bytes::copy_from_slice(&data[header_size..payload_end])
        } else {
            Bytes::new()
        };
        debug!("Extracted payload of size {}", payload.len());

        Ok(Self {
            header,
            payload,
            padding_size,
        })
    }

    /// Parse an RTP packet from an owned `Bytes`, slicing the payload as a
    /// refcounted view without copying.
    pub fn parse_from_bytes(data: Bytes) -> Result<Self> {
        debug!("Parsing RTP packet from {} bytes (zero-copy)", data.len());

        let (header, header_size) = RtpHeader::parse_without_consuming(&data)?;
        debug!("Parsed header of size {}", header_size);

        let (payload_end, padding_size) = Self::payload_bounds(&data, header_size, header.padding)?;

        // Zero-copy slice: `Bytes::slice` only bumps the underlying
        // refcount, no allocation.
        let payload = if payload_end > header_size {
            data.slice(header_size..payload_end)
        } else {
            Bytes::new()
        };
        debug!("Sliced payload of size {}", payload.len());

        Ok(Self {
            header,
            payload,
            padding_size,
        })
    }

    /// Serialize the packet to bytes.
    ///
    /// Allocates a fresh `BytesMut` per call and freezes it directly.
    /// Hot paths that send many packets should prefer
    /// [`Self::serialize_into`] with a per-task buffer to amortise the
    /// allocation across calls.
    pub fn serialize(&self) -> Result<Bytes> {
        self.validate_padding()?;
        let total_size = self.size();
        let mut buf = BytesMut::with_capacity(total_size);
        self.header.serialize(&mut buf)?;
        buf.extend_from_slice(&self.payload);
        self.serialize_padding(&mut buf);
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
        self.validate_padding()?;
        let total_size = self.size();
        buf.reserve(total_size);

        // Serialize the header
        self.header.serialize(buf)?;

        // Add the payload
        buf.extend_from_slice(&self.payload);

        // Add RFC 3550 padding, with the count in the final octet.
        self.serialize_padding(buf);

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
            "RtpPacket {{ header: {:?}, payload_len: {}, padding_size: {} }}",
            self.header,
            self.payload.len(),
            self.padding_size
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_padding_roundtrip_and_payload_stripping() {
        let mut packet = RtpPacket::new_with_payload(
            96,
            1000,
            12345,
            0xabcdef01,
            Bytes::from_static(b"payload"),
        );
        packet.set_padding(4);

        let serialized = packet.serialize().unwrap();
        assert_eq!(&serialized[serialized.len() - 4..], &[0, 0, 0, 4]);

        let parsed = RtpPacket::parse(&serialized).unwrap();
        assert!(parsed.header.padding);
        assert_eq!(parsed.padding_size, 4);
        assert_eq!(parsed.payload, Bytes::from_static(b"payload"));
        assert_eq!(parsed, packet);
    }

    #[test]
    fn test_parse_rejects_malformed_padding() {
        let packet =
            RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, Bytes::from_static(b"x"));
        let mut serialized = packet.serialize().unwrap().to_vec();
        serialized[0] |= 0x20;

        serialized.push(0);
        assert!(RtpPacket::parse(&serialized).is_err());

        *serialized.last_mut().unwrap() = 3;
        assert!(RtpPacket::parse(&serialized).is_err());

        serialized.truncate(RTP_MIN_HEADER_SIZE);
        assert!(RtpPacket::parse(&serialized).is_err());
    }

    #[test]
    fn test_serialize_rejects_inconsistent_padding_state() {
        let mut packet = RtpPacket::new_with_payload(96, 1000, 12345, 0xabcdef01, Bytes::new());
        packet.header.padding = true;
        assert!(packet.serialize().is_err());
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
}
