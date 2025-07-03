//! RTCP Feedback Packet Implementations
//!
//! This module implements the actual RTCP feedback packet structures and serialization/parsing
//! for Picture Loss Indication (PLI), Full Intra Request (FIR), Slice Loss Indication (SLI),
//! Temporal-Spatial Trade-off (TSTO), REMB, and Transport-wide Congestion Control.

use crate::{Result, RtpSsrc, Error};
use crate::feedback::{FeedbackPacketType, PayloadFeedbackFormat, TransportCcFormat};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::HashMap;

/// RTCP common header size (4 bytes)
pub const RTCP_HEADER_SIZE: usize = 4;

/// Picture Loss Indication (PLI) packet - RFC 4585
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PliPacket {
    /// SSRC of the sender of this feedback packet
    pub sender_ssrc: RtpSsrc,
    
    /// SSRC of the media source that should generate a keyframe
    pub media_ssrc: RtpSsrc,
}

impl PliPacket {
    /// Create a new PLI packet
    pub fn new(sender_ssrc: RtpSsrc, media_ssrc: RtpSsrc) -> Self {
        Self {
            sender_ssrc,
            media_ssrc,
        }
    }
    
    /// Parse PLI packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 8 {  // PLI payload is 8 bytes (2 SSRCs)
            return Err(Error::BufferTooSmall {
                required: 8,
                available: data.len(),
            });
        }
        
        let mut buf = Bytes::copy_from_slice(data);
        let sender_ssrc = buf.get_u32();
        let media_ssrc = buf.get_u32();
        
        Ok(Self {
            sender_ssrc,
            media_ssrc,
        })
    }
    
    /// Serialize PLI packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::with_capacity(12);  // 4 bytes header + 8 bytes payload
        
        // RTCP header: V=2, P=0, FMT=1, PT=206, length=2
        buf.put_u8(0x81);  // V=2, P=0, FMT=1
        buf.put_u8(FeedbackPacketType::PayloadSpecificFeedback as u8);
        buf.put_u16(2);    // Length in 32-bit words minus 1
        
        // PLI payload
        buf.put_u32(self.sender_ssrc);
        buf.put_u32(self.media_ssrc);
        
        Ok(buf.freeze())
    }
}

/// Full Intra Request (FIR) packet - RFC 5104
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirPacket {
    /// SSRC of the sender of this feedback packet
    pub sender_ssrc: RtpSsrc,
    
    /// SSRC of the media source that should generate a keyframe
    pub media_ssrc: RtpSsrc,
    
    /// FIR sequence number
    pub sequence_number: u8,
}

impl FirPacket {
    /// Create a new FIR packet
    pub fn new(sender_ssrc: RtpSsrc, media_ssrc: RtpSsrc, sequence_number: u8) -> Self {
        Self {
            sender_ssrc,
            media_ssrc,
            sequence_number,
        }
    }
    
    /// Parse FIR packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 12 {  // FIR payload is 12 bytes minimum
            return Err(Error::BufferTooSmall {
                required: 12,
                available: data.len(),
            });
        }
        
        let mut buf = Bytes::copy_from_slice(data);
        let sender_ssrc = buf.get_u32();
        let media_ssrc = buf.get_u32();
        let sequence_number = buf.get_u8();
        
        Ok(Self {
            sender_ssrc,
            media_ssrc,
            sequence_number,
        })
    }
    
    /// Serialize FIR packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::with_capacity(16);  // 4 bytes header + 12 bytes payload
        
        // RTCP header: V=2, P=0, FMT=4, PT=206, length=3
        buf.put_u8(0x84);  // V=2, P=0, FMT=4
        buf.put_u8(FeedbackPacketType::PayloadSpecificFeedback as u8);
        buf.put_u16(3);    // Length in 32-bit words minus 1
        
        // FIR payload
        buf.put_u32(self.sender_ssrc);
        buf.put_u32(self.media_ssrc);
        buf.put_u8(self.sequence_number);
        buf.put_u8(0);     // Reserved
        buf.put_u16(0);    // Reserved
        
        Ok(buf.freeze())
    }
}

/// Slice Loss Indication (SLI) entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliEntry {
    /// First macroblock address
    pub first: u16,
    
    /// Number of macroblocks
    pub number: u16,
    
    /// Picture ID
    pub picture_id: u8,
}

/// Slice Loss Indication (SLI) packet - RFC 4585
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliPacket {
    /// SSRC of the sender of this feedback packet
    pub sender_ssrc: RtpSsrc,
    
    /// SSRC of the media source
    pub media_ssrc: RtpSsrc,
    
    /// SLI entries
    pub entries: Vec<SliEntry>,
}

impl SliPacket {
    /// Create a new SLI packet
    pub fn new(sender_ssrc: RtpSsrc, media_ssrc: RtpSsrc) -> Self {
        Self {
            sender_ssrc,
            media_ssrc,
            entries: Vec::new(),
        }
    }
    
    /// Add an SLI entry
    pub fn add_entry(&mut self, first: u16, number: u16, picture_id: u8) {
        self.entries.push(SliEntry {
            first,
            number,
            picture_id,
        });
    }
    
    /// Serialize SLI packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let payload_size = 8 + self.entries.len() * 4;  // 8 bytes header + 4 bytes per entry
        let mut buf = BytesMut::with_capacity(4 + payload_size);
        
        // RTCP header: V=2, P=0, FMT=2, PT=206
        buf.put_u8(0x82);  // V=2, P=0, FMT=2
        buf.put_u8(FeedbackPacketType::PayloadSpecificFeedback as u8);
        buf.put_u16(((payload_size / 4) - 1) as u16);  // Length in 32-bit words minus 1
        
        // SLI payload
        buf.put_u32(self.sender_ssrc);
        buf.put_u32(self.media_ssrc);
        
        // SLI entries
        for entry in &self.entries {
            buf.put_u16(entry.first);
            buf.put_u16(entry.number);
            // Picture ID is encoded in upper 6 bits, lower 2 bits reserved
            buf.put_u8(entry.picture_id << 2);
            buf.put_u8(0);  // Reserved
            buf.put_u16(0); // Reserved
        }
        
        Ok(buf.freeze())
    }
}

/// Temporal-Spatial Trade-off (TSTO) packet - RFC 5104
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TstoPacket {
    /// SSRC of the sender of this feedback packet
    pub sender_ssrc: RtpSsrc,
    
    /// SSRC of the media source
    pub media_ssrc: RtpSsrc,
    
    /// Sequence number
    pub sequence_number: u8,
    
    /// Trade-off index (0-31)
    pub tradeoff_index: u8,
}

impl TstoPacket {
    /// Create a new TSTO packet
    pub fn new(sender_ssrc: RtpSsrc, media_ssrc: RtpSsrc, sequence_number: u8, tradeoff_index: u8) -> Self {
        Self {
            sender_ssrc,
            media_ssrc,
            sequence_number,
            tradeoff_index: tradeoff_index & 0x1F,  // Limit to 5 bits
        }
    }
    
    /// Serialize TSTO packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::with_capacity(16);  // 4 bytes header + 12 bytes payload
        
        // RTCP header: V=2, P=0, FMT=5, PT=206, length=3
        buf.put_u8(0x85);  // V=2, P=0, FMT=5
        buf.put_u8(FeedbackPacketType::PayloadSpecificFeedback as u8);
        buf.put_u16(3);    // Length in 32-bit words minus 1
        
        // TSTO payload
        buf.put_u32(self.sender_ssrc);
        buf.put_u32(self.media_ssrc);
        buf.put_u8(self.sequence_number);
        buf.put_u8(0);     // Reserved
        buf.put_u8(self.tradeoff_index);
        buf.put_u8(0);     // Reserved
        
        Ok(buf.freeze())
    }
}

/// Receiver Estimated Max Bitrate (REMB) packet - Google extension
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RembPacket {
    /// SSRC of the sender of this feedback packet
    pub sender_ssrc: RtpSsrc,
    
    /// Unused (set to 0)
    pub media_ssrc: RtpSsrc,
    
    /// Maximum bitrate in bits per second
    pub bitrate_bps: u32,
    
    /// SSRCs this bitrate applies to
    pub ssrcs: Vec<RtpSsrc>,
}

impl RembPacket {
    /// Create a new REMB packet
    pub fn new(sender_ssrc: RtpSsrc, bitrate_bps: u32, ssrcs: Vec<RtpSsrc>) -> Self {
        Self {
            sender_ssrc,
            media_ssrc: 0,  // Unused in REMB
            bitrate_bps,
            ssrcs,
        }
    }
    
    /// Parse REMB packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 16 {  // Minimum REMB size
            return Err(Error::BufferTooSmall {
                required: 16,
                available: data.len(),
            });
        }
        
        let mut buf = Bytes::copy_from_slice(data);
        let sender_ssrc = buf.get_u32();
        let media_ssrc = buf.get_u32();
        
        // Check for REMB identifier "REMB"
        let remb_id = buf.get_u32();
        if remb_id != 0x52454D42 {  // "REMB" in ASCII
            return Err(Error::RtcpError("Invalid REMB identifier".to_string()));
        }
        
        // Parse number of SSRCs and bitrate
        let num_ssrc_br = buf.get_u32();
        let num_ssrcs = (num_ssrc_br >> 24) as u8;
        let bitrate_bps = num_ssrc_br & 0x3FFFF;  // 18-bit exponential notation
        
        // Decode bitrate from exponential format
        let exp = (bitrate_bps >> 14) & 0x3F;
        let mantissa = bitrate_bps & 0x3FFF;
        let decoded_bitrate = mantissa << exp;
        
        // Parse SSRCs
        let mut ssrcs = Vec::with_capacity(num_ssrcs as usize);
        for _ in 0..num_ssrcs {
            if buf.remaining() < 4 {
                break;
            }
            ssrcs.push(buf.get_u32());
        }
        
        Ok(Self {
            sender_ssrc,
            media_ssrc,
            bitrate_bps: decoded_bitrate,
            ssrcs,
        })
    }
    
    /// Serialize REMB packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let payload_size = 16 + self.ssrcs.len() * 4;  // Base + SSRC list
        let mut buf = BytesMut::with_capacity(4 + payload_size);
        
        // RTCP header: V=2, P=0, FMT=15, PT=206 (Application Layer Feedback)
        buf.put_u8(0x8F);  // V=2, P=0, FMT=15
        buf.put_u8(FeedbackPacketType::PayloadSpecificFeedback as u8);
        buf.put_u16(((payload_size / 4) - 1) as u16);
        
        // REMB payload
        buf.put_u32(self.sender_ssrc);
        buf.put_u32(self.media_ssrc);
        buf.put_u32(0x52454D42);  // "REMB" identifier
        
        // Encode bitrate in exponential format (6-bit exponent, 14-bit mantissa)
        let (exp, mantissa) = Self::encode_bitrate(self.bitrate_bps);
        let encoded_bitrate = ((exp as u32) << 14) | (mantissa as u32);
        let num_ssrc_br = ((self.ssrcs.len() as u32) << 24) | encoded_bitrate;
        buf.put_u32(num_ssrc_br);
        
        // SSRC list
        for ssrc in &self.ssrcs {
            buf.put_u32(*ssrc);
        }
        
        Ok(buf.freeze())
    }
    
    /// Encode bitrate in exponential format
    fn encode_bitrate(bitrate: u32) -> (u8, u16) {
        if bitrate == 0 {
            return (0, 0);
        }
        
        // Find the highest bit position
        let mut exp = 0u8;
        let mut mantissa = bitrate;
        
        // Reduce mantissa to fit in 14 bits
        while mantissa > 0x3FFF && exp < 63 {
            mantissa >>= 1;
            exp += 1;
        }
        
        (exp, mantissa as u16)
    }
}

/// Transport-wide Congestion Control feedback packet entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportCcEntry {
    /// Sequence number
    pub sequence_number: u16,
    
    /// Receive delta (microseconds, can be negative)
    pub receive_delta: Option<i16>,
}

/// Transport-wide Congestion Control feedback packet - WebRTC extension
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportCcPacket {
    /// SSRC of the sender of this feedback packet
    pub sender_ssrc: RtpSsrc,
    
    /// SSRC of the media source (set to 0 for transport-wide)
    pub media_ssrc: RtpSsrc,
    
    /// Base sequence number
    pub base_sequence: u16,
    
    /// Packet status count
    pub packet_status_count: u16,
    
    /// Reference time (24-bit, 64ms resolution)
    pub reference_time: u32,
    
    /// Feedback packet count
    pub feedback_count: u8,
    
    /// Packet entries
    pub entries: Vec<TransportCcEntry>,
}

impl TransportCcPacket {
    /// Create a new Transport CC packet
    pub fn new(sender_ssrc: RtpSsrc, base_sequence: u16, reference_time: u32, feedback_count: u8) -> Self {
        Self {
            sender_ssrc,
            media_ssrc: 0,  // Set to 0 for transport-wide feedback
            base_sequence,
            packet_status_count: 0,
            reference_time: reference_time & 0xFFFFFF,  // 24-bit
            feedback_count,
            entries: Vec::new(),
        }
    }
    
    /// Add a packet entry
    pub fn add_entry(&mut self, sequence_number: u16, receive_delta: Option<i16>) {
        self.entries.push(TransportCcEntry {
            sequence_number,
            receive_delta,
        });
        self.packet_status_count = self.entries.len() as u16;
    }
    
    /// Serialize Transport CC packet to bytes (simplified version)
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::with_capacity(64);  // Conservative estimate
        
        // RTCP header: V=2, P=0, FMT=15, PT=205 (Generic feedback)
        buf.put_u8(0x8F);  // V=2, P=0, FMT=15
        buf.put_u8(FeedbackPacketType::GenericNack as u8);
        
        // We'll update length later
        let length_pos = buf.len();
        buf.put_u16(0);
        
        // Transport CC payload
        buf.put_u32(self.sender_ssrc);
        buf.put_u32(self.media_ssrc);
        buf.put_u16(self.base_sequence);
        buf.put_u16(self.packet_status_count);
        buf.put_u8((self.reference_time >> 16) as u8);
        buf.put_u8((self.reference_time >> 8) as u8);
        buf.put_u8(self.reference_time as u8);
        buf.put_u8(self.feedback_count);
        
        // For simplicity, encode all packets as "received" with small deltas
        // A full implementation would use the complex encoding specified in the draft
        for entry in &self.entries {
            if let Some(delta) = entry.receive_delta {
                buf.put_i16(delta);
            } else {
                buf.put_u16(0);  // Not received
            }
        }
        
        // Update length field
        let total_length = buf.len();
        let length_words = (total_length / 4) - 1;
        buf[length_pos..length_pos + 2].copy_from_slice(&(length_words as u16).to_be_bytes());
        
        Ok(buf.freeze())
    }
}

/// Combined feedback packet enum
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackPacket {
    /// Picture Loss Indication
    Pli(PliPacket),
    
    /// Full Intra Request
    Fir(FirPacket),
    
    /// Slice Loss Indication
    Sli(SliPacket),
    
    /// Temporal-Spatial Trade-off
    Tsto(TstoPacket),
    
    /// Receiver Estimated Max Bitrate
    Remb(RembPacket),
    
    /// Transport-wide Congestion Control
    TransportCc(TransportCcPacket),
}

impl FeedbackPacket {
    /// Parse a feedback packet from RTCP data
    pub fn parse_from_rtcp(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(Error::BufferTooSmall {
                required: 4,
                available: data.len(),
            });
        }
        
        let first_byte = data[0];
        let fmt = first_byte & 0x1F;
        let packet_type = data[1];
        
        match packet_type {
            206 => {  // Payload-specific feedback
                match fmt {
                    1 => Ok(FeedbackPacket::Pli(PliPacket::parse(&data[4..])?)),
                    4 => Ok(FeedbackPacket::Fir(FirPacket::parse(&data[4..])?)),
                    15 => {
                        // Check if it's REMB by looking for the identifier
                        if data.len() >= 16 {
                            let remb_check = &data[12..16];
                            if remb_check == b"REMB" {
                                return Ok(FeedbackPacket::Remb(RembPacket::parse(&data[4..])?));
                            }
                        }
                        Err(Error::RtcpError("Unsupported ALF format".to_string()))
                    }
                    _ => Err(Error::RtcpError(format!("Unsupported feedback format: {}", fmt))),
                }
            }
            205 => {  // Generic feedback (Transport CC)
                if fmt == 15 {
                    // This would be Transport CC, but parsing is complex
                    // For now, return an error or implement basic parsing
                    Err(Error::RtcpError("Transport CC parsing not fully implemented".to_string()))
                } else {
                    Err(Error::RtcpError(format!("Unsupported generic feedback format: {}", fmt)))
                }
            }
            _ => Err(Error::RtcpError(format!("Not a feedback packet type: {}", packet_type))),
        }
    }
    
    /// Serialize the feedback packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        match self {
            FeedbackPacket::Pli(pli) => pli.serialize(),
            FeedbackPacket::Fir(fir) => fir.serialize(),
            FeedbackPacket::Sli(sli) => sli.serialize(),
            FeedbackPacket::Tsto(tsto) => tsto.serialize(),
            FeedbackPacket::Remb(remb) => remb.serialize(),
            FeedbackPacket::TransportCc(transport_cc) => transport_cc.serialize(),
        }
    }
    
    /// Get the sender SSRC
    pub fn sender_ssrc(&self) -> RtpSsrc {
        match self {
            FeedbackPacket::Pli(pli) => pli.sender_ssrc,
            FeedbackPacket::Fir(fir) => fir.sender_ssrc,
            FeedbackPacket::Sli(sli) => sli.sender_ssrc,
            FeedbackPacket::Tsto(tsto) => tsto.sender_ssrc,
            FeedbackPacket::Remb(remb) => remb.sender_ssrc,
            FeedbackPacket::TransportCc(transport_cc) => transport_cc.sender_ssrc,
        }
    }
    
    /// Get the media SSRC
    pub fn media_ssrc(&self) -> RtpSsrc {
        match self {
            FeedbackPacket::Pli(pli) => pli.media_ssrc,
            FeedbackPacket::Fir(fir) => fir.media_ssrc,
            FeedbackPacket::Sli(sli) => sli.media_ssrc,
            FeedbackPacket::Tsto(tsto) => tsto.media_ssrc,
            FeedbackPacket::Remb(remb) => remb.media_ssrc,
            FeedbackPacket::TransportCc(transport_cc) => transport_cc.media_ssrc,
        }
    }
} 