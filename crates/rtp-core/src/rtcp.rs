use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::Error;
use crate::{Result, RtpSsrc};

/// RTCP packet types as defined in RFC 3550
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RtcpPacketType {
    /// Sender Report (SR)
    SenderReport = 200,
    
    /// Receiver Report (RR)
    ReceiverReport = 201,
    
    /// Source Description (SDES)
    SourceDescription = 202,
    
    /// Goodbye (BYE)
    Goodbye = 203,
    
    /// Application-Defined (APP)
    ApplicationDefined = 204,
}

impl TryFrom<u8> for RtcpPacketType {
    type Error = Error;
    
    fn try_from(value: u8) -> Result<Self> {
        match value {
            200 => Ok(RtcpPacketType::SenderReport),
            201 => Ok(RtcpPacketType::ReceiverReport),
            202 => Ok(RtcpPacketType::SourceDescription),
            203 => Ok(RtcpPacketType::Goodbye),
            204 => Ok(RtcpPacketType::ApplicationDefined),
            _ => Err(Error::RtcpError(format!("Unknown RTCP packet type: {}", value))),
        }
    }
}

/// RTCP version (same as RTP, always 2)
pub const RTCP_VERSION: u8 = 2;

/// NTP timestamp representation (64 bits)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NtpTimestamp {
    /// Seconds since January 1, 1900
    pub seconds: u32,
    
    /// Fraction of a second
    pub fraction: u32,
}

impl NtpTimestamp {
    /// Create a new NTP timestamp from the current system time
    pub fn now() -> Self {
        // Get current time since UNIX epoch
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        
        // Convert to NTP timestamp (seconds since January 1, 1900)
        // NTP epoch starts 70 years before UNIX epoch (2208988800 seconds)
        let ntp_seconds = now.as_secs() + 2208988800;
        
        // Convert nanoseconds to NTP fraction (2^32 / 10^9)
        let nanos = now.subsec_nanos();
        let ntp_fraction = (nanos as u64 * 0x100000000u64 / 1_000_000_000) as u32;
        
        Self {
            seconds: ntp_seconds as u32,
            fraction: ntp_fraction,
        }
    }
    
    /// Convert to a 64-bit representation
    pub fn to_u64(&self) -> u64 {
        (self.seconds as u64) << 32 | (self.fraction as u64)
    }
    
    /// Convert from a 64-bit representation
    pub fn from_u64(value: u64) -> Self {
        Self {
            seconds: (value >> 32) as u32,
            fraction: value as u32,
        }
    }
}

/// Report block in RTCP SR/RR packets
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpReportBlock {
    /// SSRC identifier of the source this report is for
    pub ssrc: RtpSsrc,
    
    /// Fraction of packets lost since last report
    pub fraction_lost: u8,
    
    /// Cumulative number of packets lost
    pub cumulative_lost: u32,
    
    /// Extended highest sequence number received
    pub highest_seq: u32,
    
    /// Interarrival jitter estimate
    pub jitter: u32,
    
    /// Last SR timestamp from this source
    pub last_sr: u32,
    
    /// Delay since last SR from this source (in units of 1/65536 seconds)
    pub delay_since_last_sr: u32,
}

impl RtcpReportBlock {
    /// Create a new empty report block
    pub fn new(ssrc: RtpSsrc) -> Self {
        Self {
            ssrc,
            fraction_lost: 0,
            cumulative_lost: 0,
            highest_seq: 0,
            jitter: 0,
            last_sr: 0,
            delay_since_last_sr: 0,
        }
    }
    
    /// Size of a report block in bytes
    pub const SIZE: usize = 24;
    
    /// Parse a report block from bytes
    pub fn parse(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < Self::SIZE {
            return Err(Error::BufferTooSmall {
                required: Self::SIZE,
                available: buf.remaining(),
            });
        }
        
        let ssrc = buf.get_u32();
        
        // Fraction lost (8 bits) + cumulative lost (24 bits)
        let fraction_lost = buf.get_u8();
        let cumulative_lost = (buf.get_u8() as u32) << 16 | (buf.get_u8() as u32) << 8 | buf.get_u8() as u32;
        
        let highest_seq = buf.get_u32();
        let jitter = buf.get_u32();
        let last_sr = buf.get_u32();
        let delay_since_last_sr = buf.get_u32();
        
        Ok(Self {
            ssrc,
            fraction_lost,
            cumulative_lost,
            highest_seq,
            jitter,
            last_sr,
            delay_since_last_sr,
        })
    }
    
    /// Serialize a report block to bytes
    pub fn serialize(&self, buf: &mut BytesMut) -> Result<()> {
        if buf.remaining_mut() < Self::SIZE {
            buf.reserve(Self::SIZE - buf.remaining_mut());
        }
        
        buf.put_u32(self.ssrc);
        
        // Fraction lost (8 bits) + cumulative lost (24 bits)
        buf.put_u8(self.fraction_lost);
        buf.put_u8(((self.cumulative_lost >> 16) & 0xFF) as u8);
        buf.put_u8(((self.cumulative_lost >> 8) & 0xFF) as u8);
        buf.put_u8((self.cumulative_lost & 0xFF) as u8);
        
        buf.put_u32(self.highest_seq);
        buf.put_u32(self.jitter);
        buf.put_u32(self.last_sr);
        buf.put_u32(self.delay_since_last_sr);
        
        Ok(())
    }
}

/// RTCP Sender Report (SR) packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSenderReport {
    /// SSRC of the sender
    pub ssrc: RtpSsrc,
    
    /// NTP timestamp of this report
    pub ntp_timestamp: NtpTimestamp,
    
    /// RTP timestamp corresponding to the NTP timestamp
    pub rtp_timestamp: u32,
    
    /// Sender's packet count
    pub sender_packet_count: u32,
    
    /// Sender's octet count
    pub sender_octet_count: u32,
    
    /// Report blocks
    pub report_blocks: Vec<RtcpReportBlock>,
}

impl RtcpSenderReport {
    /// Create a new sender report
    pub fn new(ssrc: RtpSsrc) -> Self {
        Self {
            ssrc,
            ntp_timestamp: NtpTimestamp::now(),
            rtp_timestamp: 0,
            sender_packet_count: 0,
            sender_octet_count: 0,
            report_blocks: Vec::new(),
        }
    }
    
    /// Size of the sender info section in bytes
    pub const SENDER_INFO_SIZE: usize = 20;
}

/// RTCP Receiver Report (RR) packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpReceiverReport {
    /// SSRC of the receiver
    pub ssrc: RtpSsrc,
    
    /// Report blocks
    pub report_blocks: Vec<RtcpReportBlock>,
}

impl RtcpReceiverReport {
    /// Create a new receiver report
    pub fn new(ssrc: RtpSsrc) -> Self {
        Self {
            ssrc,
            report_blocks: Vec::new(),
        }
    }
}

/// RTCP Source Description (SDES) item types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RtcpSdesItemType {
    /// End of SDES item list
    End = 0,
    
    /// Canonical name (CNAME)
    CName = 1,
    
    /// User name (NAME)
    Name = 2,
    
    /// E-mail address (EMAIL)
    Email = 3,
    
    /// Phone number (PHONE)
    Phone = 4,
    
    /// Geographic location (LOC)
    Location = 5,
    
    /// Application or tool name (TOOL)
    Tool = 6,
    
    /// Notice/status (NOTE)
    Note = 7,
    
    /// Private extensions (PRIV)
    Private = 8,
}

impl TryFrom<u8> for RtcpSdesItemType {
    type Error = Error;
    
    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(RtcpSdesItemType::End),
            1 => Ok(RtcpSdesItemType::CName),
            2 => Ok(RtcpSdesItemType::Name),
            3 => Ok(RtcpSdesItemType::Email),
            4 => Ok(RtcpSdesItemType::Phone),
            5 => Ok(RtcpSdesItemType::Location),
            6 => Ok(RtcpSdesItemType::Tool),
            7 => Ok(RtcpSdesItemType::Note),
            8 => Ok(RtcpSdesItemType::Private),
            _ => Err(Error::RtcpError(format!("Unknown SDES item type: {}", value))),
        }
    }
}

/// RTCP Source Description (SDES) item
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSdesItem {
    /// Item type
    pub item_type: RtcpSdesItemType,
    
    /// Item value
    pub value: String,
}

/// RTCP Source Description (SDES) chunk
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSdesChunk {
    /// SSRC/CSRC identifier
    pub ssrc: RtpSsrc,
    
    /// SDES items
    pub items: Vec<RtcpSdesItem>,
}

/// RTCP Source Description (SDES) packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSourceDescription {
    /// SDES chunks
    pub chunks: Vec<RtcpSdesChunk>,
}

/// RTCP Goodbye (BYE) packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpGoodbye {
    /// SSRC/CSRC identifiers
    pub sources: Vec<RtpSsrc>,
    
    /// Reason for leaving (optional)
    pub reason: Option<String>,
}

/// RTCP Application-Defined (APP) packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpApplicationDefined {
    /// SSRC/CSRC identifier
    pub ssrc: RtpSsrc,
    
    /// Name (4 ASCII characters)
    pub name: [u8; 4],
    
    /// Application-dependent data
    pub data: Bytes,
}

/// RTCP packet variants
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtcpPacket {
    /// Sender Report (SR)
    SenderReport(RtcpSenderReport),
    
    /// Receiver Report (RR)
    ReceiverReport(RtcpReceiverReport),
    
    /// Source Description (SDES)
    SourceDescription(RtcpSourceDescription),
    
    /// Goodbye (BYE)
    Goodbye(RtcpGoodbye),
    
    /// Application-Defined (APP)
    ApplicationDefined(RtcpApplicationDefined),
}

impl RtcpPacket {
    /// Parse an RTCP packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut buf = Bytes::copy_from_slice(data);
        
        // Parse common header (first 4 bytes)
        if buf.remaining() < 4 {
            return Err(Error::BufferTooSmall {
                required: 4,
                available: buf.remaining(),
            });
        }
        
        let first_byte = buf.get_u8();
        // Check version (2 bits)
        let version = (first_byte >> 6) & 0x03;
        if version != RTCP_VERSION {
            return Err(Error::RtcpError(format!("Invalid RTCP version: {}", version)));
        }
        
        // Check padding flag (1 bit)
        let _padding = ((first_byte >> 5) & 0x01) != 0;
        
        // Get reception report count (5 bits)
        let report_count = first_byte & 0x1F;
        
        // Get packet type
        let packet_type = RtcpPacketType::try_from(buf.get_u8())?;
        
        // Get length in 32-bit words minus one (convert to bytes)
        let length = buf.get_u16() as usize * 4;
        
        if buf.remaining() < length {
            return Err(Error::BufferTooSmall {
                required: length,
                available: buf.remaining(),
            });
        }
        
        // Parse specific packet type
        match packet_type {
            RtcpPacketType::SenderReport => {
                // Parse SR-specific fields
                if buf.remaining() < 24 {  // SSRC (4) + Sender Info (20)
                    return Err(Error::BufferTooSmall {
                        required: 24,
                        available: buf.remaining(),
                    });
                }
                
                let ssrc = buf.get_u32();
                
                // NTP timestamp (64 bits)
                let ntp_seconds = buf.get_u32();
                let ntp_fraction = buf.get_u32();
                let ntp_timestamp = NtpTimestamp {
                    seconds: ntp_seconds,
                    fraction: ntp_fraction,
                };
                
                let rtp_timestamp = buf.get_u32();
                let sender_packet_count = buf.get_u32();
                let sender_octet_count = buf.get_u32();
                
                // Parse report blocks
                let mut report_blocks = Vec::with_capacity(report_count as usize);
                for _ in 0..report_count {
                    report_blocks.push(RtcpReportBlock::parse(&mut buf)?);
                }
                
                Ok(RtcpPacket::SenderReport(RtcpSenderReport {
                    ssrc,
                    ntp_timestamp,
                    rtp_timestamp,
                    sender_packet_count,
                    sender_octet_count,
                    report_blocks,
                }))
            }
            RtcpPacketType::ReceiverReport => {
                // Parse RR-specific fields
                if buf.remaining() < 4 {  // SSRC (4)
                    return Err(Error::BufferTooSmall {
                        required: 4,
                        available: buf.remaining(),
                    });
                }
                
                let ssrc = buf.get_u32();
                
                // Parse report blocks
                let mut report_blocks = Vec::with_capacity(report_count as usize);
                for _ in 0..report_count {
                    report_blocks.push(RtcpReportBlock::parse(&mut buf)?);
                }
                
                Ok(RtcpPacket::ReceiverReport(RtcpReceiverReport {
                    ssrc,
                    report_blocks,
                }))
            }
            // For simplicity, we'll return placeholders for other packet types
            // In a complete implementation, these would be fully parsed
            RtcpPacketType::SourceDescription => {
                Ok(RtcpPacket::SourceDescription(RtcpSourceDescription {
                    chunks: Vec::new(),
                }))
            }
            RtcpPacketType::Goodbye => {
                Ok(RtcpPacket::Goodbye(RtcpGoodbye {
                    sources: Vec::new(),
                    reason: None,
                }))
            }
            RtcpPacketType::ApplicationDefined => {
                Ok(RtcpPacket::ApplicationDefined(RtcpApplicationDefined {
                    ssrc: 0,
                    name: [0; 4],
                    data: Bytes::new(),
                }))
            }
        }
    }
}

// RTCP packet serialization is more complex and would be implemented
// in a complete implementation. This is a placeholder for now.
// fn serialize(...) -> Result<Bytes> { ... } 