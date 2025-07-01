use bytes::{Buf, BufMut, BytesMut};

use crate::error::Error;
use crate::{Result, RtpSsrc};
use super::report_block::RtcpReportBlock;

/// RTCP Receiver Report (RR) packet
/// Defined in RFC 3550 Section 6.4.2
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
    
    /// Add a report block
    pub fn add_report_block(&mut self, block: RtcpReportBlock) {
        self.report_blocks.push(block);
    }
    
    /// Calculate the total size in bytes
    pub fn size(&self) -> usize {
        4 + // SSRC (4 bytes)
        self.report_blocks.len() * RtcpReportBlock::SIZE // Report blocks
    }
    
    /// Serialize the receiver report to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let mut buf = BytesMut::with_capacity(self.size());
        
        // SSRC
        buf.put_u32(self.ssrc);
        
        // Report blocks
        for block in &self.report_blocks {
            block.serialize(&mut buf)?;
        }
        
        Ok(buf)
    }
}

/// Parse a receiver report from RTCP packet data
pub fn parse_receiver_report(buf: &mut impl Buf, report_count: u8) -> Result<RtcpReceiverReport> {
    // Check if we have enough data for the SSRC (4 bytes)
    if buf.remaining() < 4 {
        return Err(Error::BufferTooSmall {
            required: 4,
            available: buf.remaining(),
        });
    }
    
    let ssrc = buf.get_u32();
    
    // Parse report blocks
    let mut report_blocks = Vec::with_capacity(report_count as usize);
    for _ in 0..report_count {
        report_blocks.push(RtcpReportBlock::parse(buf)?);
    }
    
    Ok(RtcpReceiverReport {
        ssrc,
        report_blocks,
    })
}

/// Serialize a receiver report
pub fn serialize_receiver_report(rr: &RtcpReceiverReport) -> Result<BytesMut> {
    rr.serialize()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_receiver_report_creation() {
        let rr = RtcpReceiverReport::new(0x12345678);
        
        assert_eq!(rr.ssrc, 0x12345678);
        assert!(rr.report_blocks.is_empty());
    }
    
    #[test]
    fn test_add_report_block() {
        let mut rr = RtcpReceiverReport::new(0x12345678);
        let block1 = RtcpReportBlock::new(0xabcdef01);
        let block2 = RtcpReportBlock::new(0x11223344);
        
        rr.add_report_block(block1);
        rr.add_report_block(block2);
        
        assert_eq!(rr.report_blocks.len(), 2);
        assert_eq!(rr.report_blocks[0].ssrc, 0xabcdef01);
        assert_eq!(rr.report_blocks[1].ssrc, 0x11223344);
    }
    
    #[test]
    fn test_size_calculation() {
        let mut rr = RtcpReceiverReport::new(0x12345678);
        
        // Base size (just SSRC)
        assert_eq!(rr.size(), 4);
        
        // Add two report blocks
        rr.add_report_block(RtcpReportBlock::new(0xabcdef01));
        rr.add_report_block(RtcpReportBlock::new(0x11223344));
        
        // Size should increase by the report block sizes
        assert_eq!(rr.size(), 4 + 2 * RtcpReportBlock::SIZE);
    }
    
    #[test]
    fn test_serialize_parse() {
        // Create a receiver report with two report blocks
        let mut original = RtcpReceiverReport::new(0x12345678);
        
        let block1 = RtcpReportBlock {
            ssrc: 0xabcdef01,
            fraction_lost: 42,
            cumulative_lost: 1000,
            highest_seq: 5000,
            jitter: 100,
            last_sr: 0x87654321,
            delay_since_last_sr: 1500,
        };
        
        let block2 = RtcpReportBlock {
            ssrc: 0x11223344,
            fraction_lost: 10,
            cumulative_lost: 500,
            highest_seq: 10000,
            jitter: 200,
            last_sr: 0x55667788,
            delay_since_last_sr: 2000,
        };
        
        original.add_report_block(block1);
        original.add_report_block(block2);
        
        // Serialize
        let serialized = original.serialize().unwrap();
        
        // Parse
        let parsed = parse_receiver_report(&mut serialized.freeze(), 2).unwrap();
        
        // Verify
        assert_eq!(parsed.ssrc, original.ssrc);
        assert_eq!(parsed.report_blocks.len(), original.report_blocks.len());
        
        // Verify first report block
        assert_eq!(parsed.report_blocks[0].ssrc, original.report_blocks[0].ssrc);
        assert_eq!(parsed.report_blocks[0].fraction_lost, original.report_blocks[0].fraction_lost);
        assert_eq!(parsed.report_blocks[0].cumulative_lost, original.report_blocks[0].cumulative_lost);
        assert_eq!(parsed.report_blocks[0].highest_seq, original.report_blocks[0].highest_seq);
        assert_eq!(parsed.report_blocks[0].jitter, original.report_blocks[0].jitter);
        assert_eq!(parsed.report_blocks[0].last_sr, original.report_blocks[0].last_sr);
        assert_eq!(parsed.report_blocks[0].delay_since_last_sr, original.report_blocks[0].delay_since_last_sr);
        
        // Verify second report block
        assert_eq!(parsed.report_blocks[1].ssrc, original.report_blocks[1].ssrc);
        assert_eq!(parsed.report_blocks[1].fraction_lost, original.report_blocks[1].fraction_lost);
        assert_eq!(parsed.report_blocks[1].cumulative_lost, original.report_blocks[1].cumulative_lost);
        assert_eq!(parsed.report_blocks[1].highest_seq, original.report_blocks[1].highest_seq);
        assert_eq!(parsed.report_blocks[1].jitter, original.report_blocks[1].jitter);
        assert_eq!(parsed.report_blocks[1].last_sr, original.report_blocks[1].last_sr);
        assert_eq!(parsed.report_blocks[1].delay_since_last_sr, original.report_blocks[1].delay_since_last_sr);
    }
} 