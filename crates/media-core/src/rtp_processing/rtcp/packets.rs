//! RTCP Feedback Packet Structures (moved from rtp-core)
//!
//! This module defines the actual RTCP feedback packet structures
//! and provides serialization/deserialization functionality.

use bytes::{Bytes, BytesMut, BufMut};
use crate::api::error::MediaError;
use super::feedback::{RtpSsrc, PayloadFeedbackFormat, FeedbackPacketType};

/// RTCP feedback packet header
#[derive(Debug, Clone, PartialEq)]
pub struct RtcpFeedbackHeader {
    /// Packet type (PT)
    pub packet_type: FeedbackPacketType,
    /// Format (FMT) field
    pub format: u8,
    /// Length field (in 32-bit words minus one)
    pub length: u16,
    /// Sender SSRC
    pub sender_ssrc: RtpSsrc,
    /// Media source SSRC
    pub media_ssrc: RtpSsrc,
}

impl RtcpFeedbackHeader {
    /// Create a new RTCP feedback header
    pub fn new(
        packet_type: FeedbackPacketType,
        format: u8,
        sender_ssrc: RtpSsrc,
        media_ssrc: RtpSsrc,
    ) -> Self {
        Self {
            packet_type,
            format,
            length: 2, // Base length (header only)
            sender_ssrc,
            media_ssrc,
        }
    }
    
    /// Serialize header to bytes
    pub fn to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(12);
        
        // First byte: Version (2) + Padding (0) + Format (5 bits)
        buf.put_u8(0x80 | (self.format & 0x1F));
        
        // Second byte: Packet Type
        buf.put_u8(self.packet_type as u8);
        
        // Length (16 bits)
        buf.put_u16(self.length);
        
        // Sender SSRC (32 bits)
        buf.put_u32(self.sender_ssrc);
        
        // Media SSRC (32 bits)
        buf.put_u32(self.media_ssrc);
        
        buf.freeze()
    }
    
    /// Parse header from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, MediaError> {
        if data.len() < 12 {
            return Err(MediaError::FormatError("RTCP header too short".to_string()));
        }
        
        let first_byte = data[0];
        let version = (first_byte >> 6) & 0x3;
        if version != 2 {
            return Err(MediaError::FormatError("Invalid RTCP version".to_string()));
        }
        
        let format = first_byte & 0x1F;
        let packet_type = match data[1] {
            205 => FeedbackPacketType::GenericNack,
            206 => FeedbackPacketType::PayloadSpecificFeedback,
            _ => return Err(MediaError::FormatError("Unknown RTCP feedback type".to_string())),
        };
        
        let length = u16::from_be_bytes([data[2], data[3]]);
        let sender_ssrc = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let media_ssrc = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        
        Ok(Self {
            packet_type,
            format,
            length,
            sender_ssrc,
            media_ssrc,
        })
    }
}

/// Picture Loss Indication (PLI) packet
#[derive(Debug, Clone, PartialEq)]
pub struct PliPacket {
    /// RTCP header
    pub header: RtcpFeedbackHeader,
}

impl PliPacket {
    /// Create a new PLI packet
    pub fn new(sender_ssrc: RtpSsrc, media_ssrc: RtpSsrc) -> Self {
        let header = RtcpFeedbackHeader::new(
            FeedbackPacketType::PayloadSpecificFeedback,
            PayloadFeedbackFormat::PictureLossIndication as u8,
            sender_ssrc,
            media_ssrc,
        );
        
        Self { header }
    }
    
    /// Serialize PLI packet to bytes
    pub fn to_bytes(&self) -> Bytes {
        // PLI has no additional payload beyond the header
        self.header.to_bytes()
    }
    
    /// Parse PLI packet from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, MediaError> {
        let header = RtcpFeedbackHeader::from_bytes(data)?;
        
        if header.format != PayloadFeedbackFormat::PictureLossIndication as u8 {
            return Err(MediaError::FormatError("Not a PLI packet".to_string()));
        }
        
        Ok(Self { header })
    }
}

/// Full Intra Request (FIR) packet
#[derive(Debug, Clone, PartialEq)]
pub struct FirPacket {
    /// RTCP header
    pub header: RtcpFeedbackHeader,
    /// FIR entries
    pub entries: Vec<FirEntry>,
}

/// FIR entry
#[derive(Debug, Clone, PartialEq)]
pub struct FirEntry {
    /// SSRC of the media sender
    pub ssrc: RtpSsrc,
    /// Sequence number
    pub sequence_number: u8,
    /// Reserved bytes (must be zero)
    pub reserved: [u8; 3],
}

impl FirPacket {
    /// Create a new FIR packet
    pub fn new(sender_ssrc: RtpSsrc, media_ssrc: RtpSsrc, sequence_number: u8) -> Self {
        let mut header = RtcpFeedbackHeader::new(
            FeedbackPacketType::PayloadSpecificFeedback,
            PayloadFeedbackFormat::FullIntraRequest as u8,
            sender_ssrc,
            media_ssrc,
        );
        
        // Update length to include FIR entry (2 additional 32-bit words)
        header.length = 4;
        
        let entry = FirEntry {
            ssrc: media_ssrc,
            sequence_number,
            reserved: [0; 3],
        };
        
        Self {
            header,
            entries: vec![entry],
        }
    }
    
    /// Serialize FIR packet to bytes
    pub fn to_bytes(&self) -> Bytes {
        let mut header_bytes = self.header.to_bytes();
        let mut buf = BytesMut::from(header_bytes.as_ref());
        
        // Add FIR entries
        for entry in &self.entries {
            buf.put_u32(entry.ssrc);
            buf.put_u8(entry.sequence_number);
            buf.put_slice(&entry.reserved);
        }
        
        buf.freeze()
    }
    
    /// Parse FIR packet from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, MediaError> {
        let header = RtcpFeedbackHeader::from_bytes(data)?;
        
        if header.format != PayloadFeedbackFormat::FullIntraRequest as u8 {
            return Err(MediaError::FormatError("Not a FIR packet".to_string()));
        }
        
        if data.len() < 16 {
            return Err(MediaError::FormatError("FIR packet too short".to_string()));
        }
        
        let mut entries = Vec::new();
        let mut offset = 12; // After header
        
        while offset + 8 <= data.len() {
            let ssrc = u32::from_be_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
            let sequence_number = data[offset + 4];
            let reserved = [data[offset + 5], data[offset + 6], data[offset + 7]];
            
            entries.push(FirEntry {
                ssrc,
                sequence_number,
                reserved,
            });
            
            offset += 8;
        }
        
        Ok(Self { header, entries })
    }
}

/// Receiver Estimated Maximum Bitrate (REMB) packet
#[derive(Debug, Clone, PartialEq)]
pub struct RembPacket {
    /// RTCP header
    pub header: RtcpFeedbackHeader,
    /// Bitrate in bits per second
    pub bitrate_bps: u32,
    /// SSRCs that this bitrate applies to
    pub ssrcs: Vec<RtpSsrc>,
}

impl RembPacket {
    /// Create a new REMB packet
    pub fn new(sender_ssrc: RtpSsrc, bitrate_bps: u32, ssrcs: Vec<RtpSsrc>) -> Self {
        let mut header = RtcpFeedbackHeader::new(
            FeedbackPacketType::PayloadSpecificFeedback,
            PayloadFeedbackFormat::ApplicationLayerFeedback as u8,
            sender_ssrc,
            0, // Media SSRC not used for REMB
        );
        
        // Calculate length: header (3 words) + identifier (1 word) + bitrate (1 word) + SSRCs
        let ssrc_words = (ssrcs.len() + 3) / 4; // Round up to 32-bit word boundary
        header.length = 4 + ssrc_words as u16;
        
        Self {
            header,
            bitrate_bps,
            ssrcs,
        }
    }
    
    /// Serialize REMB packet to bytes
    pub fn to_bytes(&self) -> Bytes {
        let header_bytes = self.header.to_bytes();
        let mut buf = BytesMut::from(header_bytes.as_ref());
        
        // REMB identifier "REMB"
        buf.put_slice(b"REMB");
        
        // Number of SSRCs (1 byte) + BR Exp (6 bits) + BR Mantissa (18 bits)
        let num_ssrcs = self.ssrcs.len() as u8;
        let (exp, mantissa) = Self::encode_bitrate(self.bitrate_bps);
        
        buf.put_u8(num_ssrcs);
        buf.put_u8(exp << 2 | ((mantissa >> 16) & 0x3) as u8);
        buf.put_u16((mantissa & 0xFFFF) as u16);
        
        // SSRC list
        for &ssrc in &self.ssrcs {
            buf.put_u32(ssrc);
        }
        
        // Pad to 32-bit boundary
        while buf.len() % 4 != 0 {
            buf.put_u8(0);
        }
        
        buf.freeze()
    }
    
    /// Encode bitrate using exponential notation
    fn encode_bitrate(bitrate_bps: u32) -> (u8, u32) {
        if bitrate_bps == 0 {
            return (0, 0);
        }
        
        let mut exp = 0u8;
        let mut mantissa = bitrate_bps;
        
        // Find the appropriate exponent
        while mantissa > 0x3FFFF && exp < 63 {
            mantissa >>= 1;
            exp += 1;
        }
        
        (exp, mantissa)
    }
    
    /// Decode bitrate from exponential notation
    fn decode_bitrate(exp: u8, mantissa: u32) -> u32 {
        if mantissa == 0 {
            return 0;
        }
        
        mantissa << exp
    }
    
    /// Parse REMB packet from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, MediaError> {
        let header = RtcpFeedbackHeader::from_bytes(data)?;
        
        if header.format != PayloadFeedbackFormat::ApplicationLayerFeedback as u8 {
            return Err(MediaError::FormatError("Not an ALF packet".to_string()));
        }
        
        if data.len() < 20 {
            return Err(MediaError::FormatError("REMB packet too short".to_string()));
        }
        
        // Check REMB identifier
        if &data[12..16] != b"REMB" {
            return Err(MediaError::FormatError("Not a REMB packet".to_string()));
        }
        
        let num_ssrcs = data[16];
        let exp = data[17] >> 2;
        let mantissa = (((data[17] & 0x3) as u32) << 16) | ((data[18] as u32) << 8) | (data[19] as u32);
        
        let bitrate_bps = Self::decode_bitrate(exp, mantissa);
        
        let mut ssrcs = Vec::new();
        let mut offset = 20;
        
        for _ in 0..num_ssrcs {
            if offset + 4 > data.len() {
                return Err(MediaError::FormatError("REMB packet truncated".to_string()));
            }
            
            let ssrc = u32::from_be_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
            ssrcs.push(ssrc);
            offset += 4;
        }
        
        Ok(Self {
            header,
            bitrate_bps,
            ssrcs,
        })
    }
}

/// Generic NACK packet for requesting retransmission
#[derive(Debug, Clone, PartialEq)]
pub struct NackPacket {
    /// RTCP header
    pub header: RtcpFeedbackHeader,
    /// Lost packet sequence numbers
    pub lost_packets: Vec<u16>,
}

impl NackPacket {
    /// Create a new NACK packet
    pub fn new(sender_ssrc: RtpSsrc, media_ssrc: RtpSsrc, lost_packets: Vec<u16>) -> Self {
        let mut header = RtcpFeedbackHeader::new(
            FeedbackPacketType::GenericNack,
            1, // Generic NACK format
            sender_ssrc,
            media_ssrc,
        );
        
        // Calculate length based on number of packet entries
        let num_entries = (lost_packets.len() + 16) / 17; // NACK entries can pack up to 17 sequence numbers
        header.length = 2 + num_entries as u16;
        
        Self {
            header,
            lost_packets,
        }
    }
    
    /// Serialize NACK packet to bytes
    pub fn to_bytes(&self) -> Bytes {
        let header_bytes = self.header.to_bytes();
        let mut buf = BytesMut::from(header_bytes.as_ref());
        
        // Encode lost packets as NACK entries
        let mut remaining_packets = self.lost_packets.clone();
        remaining_packets.sort_unstable();
        
        while !remaining_packets.is_empty() {
            let pid = remaining_packets.remove(0);
            let mut blp = 0u16; // Bitmask of additional lost packets
            
            // Check for consecutive lost packets within 16 of the PID
            for i in 1..=16 {
                let seq = pid.wrapping_add(i);
                if let Some(pos) = remaining_packets.iter().position(|&x| x == seq) {
                    blp |= 1 << (i - 1);
                    remaining_packets.remove(pos);
                }
            }
            
            buf.put_u16(pid);
            buf.put_u16(blp);
        }
        
        buf.freeze()
    }
    
    /// Parse NACK packet from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, MediaError> {
        let header = RtcpFeedbackHeader::from_bytes(data)?;
        
        if header.packet_type != FeedbackPacketType::GenericNack {
            return Err(MediaError::FormatError("Not a NACK packet".to_string()));
        }
        
        let mut lost_packets = Vec::new();
        let mut offset = 12; // After header
        
        while offset + 4 <= data.len() {
            let pid = u16::from_be_bytes([data[offset], data[offset + 1]]);
            let blp = u16::from_be_bytes([data[offset + 2], data[offset + 3]]);
            
            lost_packets.push(pid);
            
            // Decode bitmask
            for i in 0..16 {
                if (blp & (1 << i)) != 0 {
                    lost_packets.push(pid.wrapping_add(i + 1));
                }
            }
            
            offset += 4;
        }
        
        Ok(Self {
            header,
            lost_packets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_pli_packet() {
        let pli = PliPacket::new(0x12345678, 0x87654321);
        let bytes = pli.to_bytes();
        let parsed = PliPacket::from_bytes(&bytes).unwrap();
        
        assert_eq!(pli, parsed);
        assert_eq!(pli.header.sender_ssrc, 0x12345678);
        assert_eq!(pli.header.media_ssrc, 0x87654321);
    }
    
    #[test]
    fn test_fir_packet() {
        let fir = FirPacket::new(0x12345678, 0x87654321, 42);
        let bytes = fir.to_bytes();
        let parsed = FirPacket::from_bytes(&bytes).unwrap();
        
        assert_eq!(fir, parsed);
        assert_eq!(fir.entries[0].sequence_number, 42);
    }
    
    #[test]
    fn test_remb_packet() {
        let ssrcs = vec![0x12345678, 0x87654321];
        let remb = RembPacket::new(0x11111111, 1_000_000, ssrcs.clone());
        let bytes = remb.to_bytes();
        let parsed = RembPacket::from_bytes(&bytes).unwrap();
        
        assert_eq!(remb.bitrate_bps, parsed.bitrate_bps);
        assert_eq!(remb.ssrcs, parsed.ssrcs);
    }
    
    #[test]
    fn test_nack_packet() {
        let lost_packets = vec![100, 101, 103, 105];
        let nack = NackPacket::new(0x12345678, 0x87654321, lost_packets.clone());
        let bytes = nack.to_bytes();
        let parsed = NackPacket::from_bytes(&bytes).unwrap();
        
        // Verify all lost packets are preserved
        for &packet in &lost_packets {
            assert!(parsed.lost_packets.contains(&packet));
        }
    }
    
    #[test]
    fn test_bitrate_encoding() {
        // Test various bitrate values
        let test_cases = vec![
            0,
            1000,
            64000,
            1_000_000,
            10_000_000,
            100_000_000,
        ];
        
        for bitrate in test_cases {
            let (exp, mantissa) = RembPacket::encode_bitrate(bitrate);
            let decoded = RembPacket::decode_bitrate(exp, mantissa);
            
            // Allow for some rounding in the encoding
            let diff = if bitrate > decoded { bitrate - decoded } else { decoded - bitrate };
            assert!(diff <= bitrate / 100 || diff <= 1000); // Within 1% or 1kbps
        }
    }
}