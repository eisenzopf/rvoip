use bytes::{Buf, BufMut, BytesMut};

use crate::error::Error;
use crate::{Result, RtpSsrc};

/// Report block in RTCP SR/RR packets
/// Defined in RFC 3550 Section 6.4.1 and 6.4.2
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
    
    /// Calculate packet loss statistics
    pub fn calculate_packet_loss(&self, total_expected: u32, total_received: u32) -> (u8, u32) {
        // Calculate cumulative loss
        let cumulative_lost = if total_expected > total_received {
            total_expected - total_received
        } else {
            0
        };
        
        // Calculate fraction lost using the 8-bit fixed point format
        // where 0 = 0% and 255 = 100% loss
        let fraction_lost = if total_expected > 0 {
            ((cumulative_lost as f32 / total_expected as f32) * 256.0) as u8
        } else {
            0
        };
        
        (fraction_lost, cumulative_lost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_report_block_creation() {
        let block = RtcpReportBlock::new(0x12345678);
        
        assert_eq!(block.ssrc, 0x12345678);
        assert_eq!(block.fraction_lost, 0);
        assert_eq!(block.cumulative_lost, 0);
        assert_eq!(block.highest_seq, 0);
        assert_eq!(block.jitter, 0);
        assert_eq!(block.last_sr, 0);
        assert_eq!(block.delay_since_last_sr, 0);
    }
    
    #[test]
    fn test_report_block_serialize_parse() {
        let original = RtcpReportBlock {
            ssrc: 0x12345678,
            fraction_lost: 42,
            cumulative_lost: 1000,
            highest_seq: 5000,
            jitter: 100,
            last_sr: 0x87654321,
            delay_since_last_sr: 1500,
        };
        
        // Serialize
        let mut buf = BytesMut::with_capacity(RtcpReportBlock::SIZE);
        original.serialize(&mut buf).unwrap();
        
        assert_eq!(buf.len(), RtcpReportBlock::SIZE);
        
        // Parse
        let parsed = RtcpReportBlock::parse(&mut buf.clone().freeze()).unwrap();
        
        // Verify
        assert_eq!(parsed.ssrc, original.ssrc);
        assert_eq!(parsed.fraction_lost, original.fraction_lost);
        assert_eq!(parsed.cumulative_lost, original.cumulative_lost);
        assert_eq!(parsed.highest_seq, original.highest_seq);
        assert_eq!(parsed.jitter, original.jitter);
        assert_eq!(parsed.last_sr, original.last_sr);
        assert_eq!(parsed.delay_since_last_sr, original.delay_since_last_sr);
    }
    
    #[test]
    fn test_packet_loss_calculation() {
        let block = RtcpReportBlock::new(0x12345678);
        
        // Test no loss
        let (fraction, cumulative) = block.calculate_packet_loss(1000, 1000);
        assert_eq!(fraction, 0);
        assert_eq!(cumulative, 0);
        
        // Test 25% loss
        let (fraction, cumulative) = block.calculate_packet_loss(1000, 750);
        assert_eq!(fraction, 64); // 0.25 * 256 = 64
        assert_eq!(cumulative, 250);
        
        // Test 100% loss
        let (fraction, cumulative) = block.calculate_packet_loss(1000, 0);
        assert_eq!(fraction, 255); // Rounds to 255 for 100% loss
        assert_eq!(cumulative, 1000);
        
        // Test more received than expected (shouldn't happen in practice)
        let (fraction, cumulative) = block.calculate_packet_loss(1000, 1100);
        assert_eq!(fraction, 0);
        assert_eq!(cumulative, 0);
    }
} 