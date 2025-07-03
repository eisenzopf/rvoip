use bytes::{Buf, BufMut, BytesMut};

use crate::error::Error;
use crate::{Result, RtpSsrc};
use super::ntp::NtpTimestamp;
use super::report_block::RtcpReportBlock;

/// RTCP Sender Report (SR) packet
/// Defined in RFC 3550 Section 6.4.1
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
    
    /// Add a report block
    pub fn add_report_block(&mut self, block: RtcpReportBlock) {
        self.report_blocks.push(block);
    }
    
    /// Calculate total size in bytes
    pub fn size(&self) -> usize {
        4 + // SSRC
        Self::SENDER_INFO_SIZE + // Sender info (NTP timestamp, RTP timestamp, packet count, octet count)
        self.report_blocks.len() * RtcpReportBlock::SIZE // Report blocks
    }
    
    /// Serialize the sender report to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let mut buf = BytesMut::with_capacity(self.size());
        
        // SSRC
        buf.put_u32(self.ssrc);
        
        // NTP timestamp
        buf.put_u32(self.ntp_timestamp.seconds);
        buf.put_u32(self.ntp_timestamp.fraction);
        
        // RTP timestamp
        buf.put_u32(self.rtp_timestamp);
        
        // Packet count
        buf.put_u32(self.sender_packet_count);
        
        // Octet count
        buf.put_u32(self.sender_octet_count);
        
        // Report blocks
        for block in &self.report_blocks {
            block.serialize(&mut buf)?;
        }
        
        Ok(buf)
    }
}

/// Parse a sender report from RTCP packet data
pub fn parse_sender_report(buf: &mut impl Buf, report_count: u8) -> Result<RtcpSenderReport> {
    // Check if we have enough data for the sender info and SSRC (24 bytes total)
    if buf.remaining() < 24 {
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
        report_blocks.push(RtcpReportBlock::parse(buf)?);
    }
    
    Ok(RtcpSenderReport {
        ssrc,
        ntp_timestamp,
        rtp_timestamp,
        sender_packet_count,
        sender_octet_count,
        report_blocks,
    })
}

/// Serialize a sender report
pub fn serialize_sender_report(sr: &RtcpSenderReport) -> Result<BytesMut> {
    sr.serialize()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sender_report_creation() {
        let sr = RtcpSenderReport::new(0x12345678);
        
        assert_eq!(sr.ssrc, 0x12345678);
        assert!(sr.report_blocks.is_empty());
    }
    
    #[test]
    fn test_add_report_block() {
        let mut sr = RtcpSenderReport::new(0x12345678);
        let block = RtcpReportBlock::new(0xabcdef01);
        
        sr.add_report_block(block);
        
        assert_eq!(sr.report_blocks.len(), 1);
        assert_eq!(sr.report_blocks[0].ssrc, 0xabcdef01);
    }
    
    #[test]
    fn test_size_calculation() {
        let mut sr = RtcpSenderReport::new(0x12345678);
        
        // Base size (SSRC + sender info)
        assert_eq!(sr.size(), 4 + RtcpSenderReport::SENDER_INFO_SIZE);
        
        // Add a report block
        sr.add_report_block(RtcpReportBlock::new(0xabcdef01));
        
        // Size should increase by the report block size
        assert_eq!(sr.size(), 4 + RtcpSenderReport::SENDER_INFO_SIZE + RtcpReportBlock::SIZE);
    }
    
    #[test]
    fn test_serialize_parse() {
        // Create a sender report with one report block
        let mut original = RtcpSenderReport::new(0x12345678);
        original.ntp_timestamp = NtpTimestamp { seconds: 0x11223344, fraction: 0x55667788 };
        original.rtp_timestamp = 0x99aabbcc;
        original.sender_packet_count = 1000;
        original.sender_octet_count = 100000;
        
        let block = RtcpReportBlock {
            ssrc: 0xabcdef01,
            fraction_lost: 42,
            cumulative_lost: 1000,
            highest_seq: 5000,
            jitter: 100,
            last_sr: 0x87654321,
            delay_since_last_sr: 1500,
        };
        original.add_report_block(block);
        
        // Serialize
        let serialized = original.serialize().unwrap();
        
        // Parse
        let parsed = parse_sender_report(&mut serialized.freeze(), 1).unwrap();
        
        // Verify
        assert_eq!(parsed.ssrc, original.ssrc);
        assert_eq!(parsed.ntp_timestamp.seconds, original.ntp_timestamp.seconds);
        assert_eq!(parsed.ntp_timestamp.fraction, original.ntp_timestamp.fraction);
        assert_eq!(parsed.rtp_timestamp, original.rtp_timestamp);
        assert_eq!(parsed.sender_packet_count, original.sender_packet_count);
        assert_eq!(parsed.sender_octet_count, original.sender_octet_count);
        
        assert_eq!(parsed.report_blocks.len(), 1);
        assert_eq!(parsed.report_blocks[0].ssrc, original.report_blocks[0].ssrc);
        assert_eq!(parsed.report_blocks[0].fraction_lost, original.report_blocks[0].fraction_lost);
        assert_eq!(parsed.report_blocks[0].cumulative_lost, original.report_blocks[0].cumulative_lost);
        assert_eq!(parsed.report_blocks[0].highest_seq, original.report_blocks[0].highest_seq);
        assert_eq!(parsed.report_blocks[0].jitter, original.report_blocks[0].jitter);
        assert_eq!(parsed.report_blocks[0].last_sr, original.report_blocks[0].last_sr);
        assert_eq!(parsed.report_blocks[0].delay_since_last_sr, original.report_blocks[0].delay_since_last_sr);
    }
} 