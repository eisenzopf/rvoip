use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::{Result, RtpSsrc};
use super::NtpTimestamp;

/// RTCP XR Block Types as defined in RFC 3611
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RtcpXrBlockType {
    /// Loss RLE Report Block
    LossRle = 1,
    
    /// Duplicate RLE Report Block
    DuplicateRle = 2,
    
    /// Packet Receipt Times Report Block
    PacketReceiptTimes = 3,
    
    /// Receiver Reference Time Report Block
    ReceiverReferenceTimes = 4,
    
    /// DLRR Report Block
    Dlrr = 5,
    
    /// Statistics Summary Report Block
    StatisticsSummary = 6,
    
    /// VoIP Metrics Report Block
    VoipMetrics = 7,
}

impl TryFrom<u8> for RtcpXrBlockType {
    type Error = Error;
    
    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(RtcpXrBlockType::LossRle),
            2 => Ok(RtcpXrBlockType::DuplicateRle),
            3 => Ok(RtcpXrBlockType::PacketReceiptTimes),
            4 => Ok(RtcpXrBlockType::ReceiverReferenceTimes),
            5 => Ok(RtcpXrBlockType::Dlrr),
            6 => Ok(RtcpXrBlockType::StatisticsSummary),
            7 => Ok(RtcpXrBlockType::VoipMetrics),
            _ => Err(Error::RtcpError(format!("Unknown XR block type: {}", value))),
        }
    }
}

/// RTCP Extended Reports (XR) Packet
/// Defined in RFC 3611
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpExtendedReport {
    /// SSRC of the packet sender
    pub ssrc: RtpSsrc,
    
    /// Report blocks contained in the XR packet
    pub blocks: Vec<RtcpXrBlock>,
}

impl RtcpExtendedReport {
    /// Create a new XR packet
    pub fn new(ssrc: RtpSsrc) -> Self {
        Self {
            ssrc,
            blocks: Vec::new(),
        }
    }
    
    /// Add a report block
    pub fn add_block(&mut self, block: RtcpXrBlock) {
        self.blocks.push(block);
    }
    
    /// Serialize the XR packet to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let total_size = 4 + self.blocks.iter().map(|b| b.size()).sum::<usize>();
        let mut buf = BytesMut::with_capacity(total_size);
        
        // SSRC
        buf.put_u32(self.ssrc);
        
        // Report blocks
        for block in &self.blocks {
            block.serialize(&mut buf)?;
        }
        
        Ok(buf)
    }
}

/// Parse an XR packet
pub fn parse_xr(buf: &mut impl Buf) -> Result<RtcpExtendedReport> {
    if buf.remaining() < 4 {
        return Err(Error::BufferTooSmall {
            required: 4,
            available: buf.remaining(),
        });
    }
    
    // Parse SSRC
    let ssrc = buf.get_u32();
    
    let mut xr = RtcpExtendedReport::new(ssrc);
    
    // Parse report blocks
    while buf.has_remaining() {
        if buf.remaining() < 4 {
            return Err(Error::BufferTooSmall {
                required: 4,
                available: buf.remaining(),
            });
        }
        
        // Get block type
        let block_type_byte = buf.get_u8();
        let block_type = RtcpXrBlockType::try_from(block_type_byte)?;
        
        // Skip reserved byte
        buf.advance(1);
        
        // Get block length in 32-bit words
        let block_length = buf.get_u16() as usize * 4;
        
        if buf.remaining() < block_length {
            return Err(Error::BufferTooSmall {
                required: block_length,
                available: buf.remaining(),
            });
        }
        
        // Parse specific block based on type
        let block = match block_type {
            RtcpXrBlockType::LossRle => {
                // Parse Loss RLE block
                RtcpXrBlock::LossRle(parse_loss_rle_block(buf, block_length)?)
            },
            RtcpXrBlockType::DuplicateRle => {
                // Parse Duplicate RLE block
                RtcpXrBlock::DuplicateRle(parse_duplicate_rle_block(buf, block_length)?)
            },
            RtcpXrBlockType::PacketReceiptTimes => {
                // Parse Packet Receipt Times block
                RtcpXrBlock::PacketReceiptTimes(parse_packet_receipt_times_block(buf, block_length)?)
            },
            RtcpXrBlockType::ReceiverReferenceTimes => {
                // Parse Receiver Reference Time block
                RtcpXrBlock::ReceiverReferenceTimes(parse_receiver_reference_time_block(buf)?)
            },
            RtcpXrBlockType::Dlrr => {
                // Parse DLRR block
                RtcpXrBlock::Dlrr(parse_dlrr_block(buf, block_length)?)
            },
            RtcpXrBlockType::StatisticsSummary => {
                // Parse Statistics Summary block
                RtcpXrBlock::StatisticsSummary(parse_statistics_summary_block(buf)?)
            },
            RtcpXrBlockType::VoipMetrics => {
                // Parse VoIP Metrics block
                RtcpXrBlock::VoipMetrics(parse_voip_metrics_block(buf)?)
            },
        };
        
        xr.add_block(block);
    }
    
    Ok(xr)
}

/// Dummy placeholder parse functions for XR blocks
/// These would be replaced with proper implementations
fn parse_loss_rle_block(buf: &mut impl Buf, length: usize) -> Result<LossRleReportBlock> {
    // Skip for now
    buf.advance(length);
    Ok(LossRleReportBlock {
        ssrc: 0,
        begin_seq: 0,
        end_seq: 0,
        chunks: Vec::new(),
    })
}

fn parse_duplicate_rle_block(buf: &mut impl Buf, length: usize) -> Result<DuplicateRleReportBlock> {
    // Skip for now
    buf.advance(length);
    Ok(DuplicateRleReportBlock {
        ssrc: 0,
        begin_seq: 0,
        end_seq: 0,
        chunks: Vec::new(),
    })
}

fn parse_packet_receipt_times_block(buf: &mut impl Buf, length: usize) -> Result<PacketReceiptTimesBlock> {
    // Skip for now
    buf.advance(length);
    Ok(PacketReceiptTimesBlock {
        ssrc: 0,
        begin_seq: 0,
        end_seq: 0,
        receipt_times: Vec::new(),
    })
}

fn parse_receiver_reference_time_block(buf: &mut impl Buf) -> Result<ReceiverReferenceTimeBlock> {
    if buf.remaining() < 8 {
        return Err(Error::BufferTooSmall {
            required: 8,
            available: buf.remaining(),
        });
    }
    
    let ntp_sec = buf.get_u32();
    let ntp_frac = buf.get_u32();
    
    Ok(ReceiverReferenceTimeBlock {
        ntp: NtpTimestamp {
            seconds: ntp_sec,
            fraction: ntp_frac,
        },
    })
}

fn parse_dlrr_block(buf: &mut impl Buf, length: usize) -> Result<DlrrBlock> {
    // Skip for now
    buf.advance(length);
    Ok(DlrrBlock {
        sub_blocks: Vec::new(),
    })
}

fn parse_statistics_summary_block(buf: &mut impl Buf) -> Result<StatisticsSummaryBlock> {
    if buf.remaining() < 16 {
        return Err(Error::BufferTooSmall {
            required: 16,
            available: buf.remaining(),
        });
    }
    
    let ssrc = buf.get_u32();
    let flags = buf.get_u8();
    buf.advance(1); // Reserved
    let begin_seq = buf.get_u16();
    let end_seq = buf.get_u16();
    let lost_packets = buf.get_u32();
    let dup_packets = buf.get_u32();
    
    // Extract flag bits
    let loss_report = (flags & 0x01) != 0;
    let duplicate_report = (flags & 0x02) != 0;
    let jitter_report = (flags & 0x04) != 0;
    let ttr_report = (flags & 0x08) != 0;
    
    // Parse optional fields based on flags
    let min_jitter = if jitter_report && buf.remaining() >= 4 {
        Some(buf.get_u32())
    } else {
        None
    };
    
    let max_jitter = if jitter_report && buf.remaining() >= 4 {
        Some(buf.get_u32())
    } else {
        None
    };
    
    let mean_jitter = if jitter_report && buf.remaining() >= 4 {
        Some(buf.get_u32())
    } else {
        None
    };
    
    let dev_jitter = if jitter_report && buf.remaining() >= 4 {
        Some(buf.get_u32())
    } else {
        None
    };
    
    Ok(StatisticsSummaryBlock {
        ssrc,
        begin_seq,
        end_seq,
        lost_packets,
        dup_packets,
        loss_report,
        duplicate_report,
        jitter_report,
        ttr_report,
        min_jitter,
        max_jitter,
        mean_jitter,
        dev_jitter,
    })
}

fn parse_voip_metrics_block(buf: &mut impl Buf) -> Result<VoipMetricsBlock> {
    if buf.remaining() < 24 {
        return Err(Error::BufferTooSmall {
            required: 24,
            available: buf.remaining(),
        });
    }
    
    let ssrc = buf.get_u32();
    let loss_rate = buf.get_u8();
    let discard_rate = buf.get_u8();
    let burst_density = buf.get_u8();
    let gap_density = buf.get_u8();
    let burst_duration = buf.get_u16();
    let gap_duration = buf.get_u16();
    let round_trip_delay = buf.get_u16();
    let end_system_delay = buf.get_u16();
    let signal_level = buf.get_u8();
    let noise_level = buf.get_u8();
    let rerl = buf.get_u8();
    let gmin = buf.get_u8();
    let r_factor = buf.get_u8();
    let ext_r_factor = buf.get_u8();
    let mos_lq = buf.get_u8();
    let mos_cq = buf.get_u8();
    let rx_config = buf.get_u8();
    buf.advance(1); // Reserved
    let jb_nominal = buf.get_u16();
    let jb_maximum = buf.get_u16();
    let jb_abs_max = buf.get_u16();
    
    Ok(VoipMetricsBlock {
        ssrc,
        loss_rate,
        discard_rate,
        burst_density,
        gap_density,
        burst_duration,
        gap_duration,
        round_trip_delay,
        end_system_delay,
        signal_level,
        noise_level,
        rerl,
        gmin,
        r_factor,
        ext_r_factor,
        mos_lq,
        mos_cq,
        rx_config,
        jb_nominal,
        jb_maximum,
        jb_abs_max,
    })
}

/// RTCP XR Block variants
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtcpXrBlock {
    /// Loss RLE Report Block
    LossRle(LossRleReportBlock),
    
    /// Duplicate RLE Report Block
    DuplicateRle(DuplicateRleReportBlock),
    
    /// Packet Receipt Times Report Block
    PacketReceiptTimes(PacketReceiptTimesBlock),
    
    /// Receiver Reference Time Report Block
    ReceiverReferenceTimes(ReceiverReferenceTimeBlock),
    
    /// DLRR Report Block
    Dlrr(DlrrBlock),
    
    /// Statistics Summary Report Block
    StatisticsSummary(StatisticsSummaryBlock),
    
    /// VoIP Metrics Report Block
    VoipMetrics(VoipMetricsBlock),
}

impl RtcpXrBlock {
    /// Get the block type
    pub fn block_type(&self) -> RtcpXrBlockType {
        match self {
            RtcpXrBlock::LossRle(_) => RtcpXrBlockType::LossRle,
            RtcpXrBlock::DuplicateRle(_) => RtcpXrBlockType::DuplicateRle,
            RtcpXrBlock::PacketReceiptTimes(_) => RtcpXrBlockType::PacketReceiptTimes,
            RtcpXrBlock::ReceiverReferenceTimes(_) => RtcpXrBlockType::ReceiverReferenceTimes,
            RtcpXrBlock::Dlrr(_) => RtcpXrBlockType::Dlrr,
            RtcpXrBlock::StatisticsSummary(_) => RtcpXrBlockType::StatisticsSummary,
            RtcpXrBlock::VoipMetrics(_) => RtcpXrBlockType::VoipMetrics,
        }
    }
    
    /// Get the size of the block in bytes
    pub fn size(&self) -> usize {
        // Block header (4 bytes) + block specific size
        match self {
            RtcpXrBlock::LossRle(block) => 4 + block.size(),
            RtcpXrBlock::DuplicateRle(block) => 4 + block.size(),
            RtcpXrBlock::PacketReceiptTimes(block) => 4 + block.size(),
            RtcpXrBlock::ReceiverReferenceTimes(_) => 4 + 8, // NTP timestamp (8 bytes)
            RtcpXrBlock::Dlrr(block) => 4 + block.size(),
            RtcpXrBlock::StatisticsSummary(_) => 4 + 16, // Basic fields (16 bytes) + optional fields
            RtcpXrBlock::VoipMetrics(_) => 4 + 24, // 24 bytes of metrics
        }
    }
    
    /// Serialize the block to bytes
    pub fn serialize(&self, buf: &mut BytesMut) -> Result<()> {
        // Block type
        buf.put_u8(self.block_type() as u8);
        
        // Reserved byte
        buf.put_u8(0);
        
        // Block length in 32-bit words (excluding the header)
        let block_length = (self.size() - 4) / 4;
        buf.put_u16(block_length as u16);
        
        // Block specific serialization
        match self {
            RtcpXrBlock::LossRle(_) => {
                // Not implemented yet
            },
            RtcpXrBlock::DuplicateRle(_) => {
                // Not implemented yet
            },
            RtcpXrBlock::PacketReceiptTimes(_) => {
                // Not implemented yet
            },
            RtcpXrBlock::ReceiverReferenceTimes(block) => {
                buf.put_u32(block.ntp.seconds);
                buf.put_u32(block.ntp.fraction);
            },
            RtcpXrBlock::Dlrr(_) => {
                // Not implemented yet
            },
            RtcpXrBlock::StatisticsSummary(_) => {
                // Not implemented yet
            },
            RtcpXrBlock::VoipMetrics(_) => {
                // Not implemented yet
            },
        }
        
        Ok(())
    }
}

/// Loss RLE Report Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LossRleReportBlock {
    /// SSRC
    pub ssrc: RtpSsrc,
    
    /// Begin sequence number
    pub begin_seq: u16,
    
    /// End sequence number
    pub end_seq: u16,
    
    /// Run Length Chunks
    pub chunks: Vec<RleChunk>,
}

impl LossRleReportBlock {
    /// Get the size of the block in bytes
    pub fn size(&self) -> usize {
        8 + self.chunks.len() * 2 // Basic fields (8 bytes) + chunks (2 bytes each)
    }
}

/// Duplicate RLE Report Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateRleReportBlock {
    /// SSRC
    pub ssrc: RtpSsrc,
    
    /// Begin sequence number
    pub begin_seq: u16,
    
    /// End sequence number
    pub end_seq: u16,
    
    /// Run Length Chunks
    pub chunks: Vec<RleChunk>,
}

impl DuplicateRleReportBlock {
    /// Get the size of the block in bytes
    pub fn size(&self) -> usize {
        8 + self.chunks.len() * 2 // Basic fields (8 bytes) + chunks (2 bytes each)
    }
}

/// Run Length Chunk
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RleChunk {
    /// Run length chunk
    RunLength { run_type: bool, run_length: u16 },
    
    /// Bit vector chunk
    BitVector(u16),
}

/// Packet Receipt Times Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketReceiptTimesBlock {
    /// SSRC
    pub ssrc: RtpSsrc,
    
    /// Begin sequence number
    pub begin_seq: u16,
    
    /// End sequence number
    pub end_seq: u16,
    
    /// Receipt times
    pub receipt_times: Vec<u32>,
}

impl PacketReceiptTimesBlock {
    /// Get the size of the block in bytes
    pub fn size(&self) -> usize {
        8 + self.receipt_times.len() * 4 // Basic fields (8 bytes) + receipt times (4 bytes each)
    }
}

/// Receiver Reference Time Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverReferenceTimeBlock {
    /// NTP timestamp
    pub ntp: NtpTimestamp,
}

/// DLRR Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlrrBlock {
    /// Sub-blocks
    pub sub_blocks: Vec<DlrrSubBlock>,
}

impl DlrrBlock {
    /// Get the size of the block in bytes
    pub fn size(&self) -> usize {
        self.sub_blocks.len() * 12 // Sub-blocks (12 bytes each)
    }
}

/// DLRR Sub-Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlrrSubBlock {
    /// SSRC
    pub ssrc: RtpSsrc,
    
    /// Last RR timestamp
    pub last_rr: u32,
    
    /// Delay since last RR
    pub delay: u32,
}

/// Statistics Summary Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatisticsSummaryBlock {
    /// SSRC
    pub ssrc: RtpSsrc,
    
    /// Begin sequence number
    pub begin_seq: u16,
    
    /// End sequence number
    pub end_seq: u16,
    
    /// Lost packets
    pub lost_packets: u32,
    
    /// Duplicate packets
    pub dup_packets: u32,
    
    /// Whether loss report is included
    pub loss_report: bool,
    
    /// Whether duplicate report is included
    pub duplicate_report: bool,
    
    /// Whether jitter report is included
    pub jitter_report: bool,
    
    /// Whether TTR report is included
    pub ttr_report: bool,
    
    /// Minimum jitter
    pub min_jitter: Option<u32>,
    
    /// Maximum jitter
    pub max_jitter: Option<u32>,
    
    /// Mean jitter
    pub mean_jitter: Option<u32>,
    
    /// Standard deviation of jitter
    pub dev_jitter: Option<u32>,
}

/// VoIP Metrics Block
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoipMetricsBlock {
    /// SSRC
    pub ssrc: RtpSsrc,
    
    /// Loss rate
    pub loss_rate: u8,
    
    /// Discard rate
    pub discard_rate: u8,
    
    /// Burst density
    pub burst_density: u8,
    
    /// Gap density
    pub gap_density: u8,
    
    /// Burst duration
    pub burst_duration: u16,
    
    /// Gap duration
    pub gap_duration: u16,
    
    /// Round-trip delay
    pub round_trip_delay: u16,
    
    /// End system delay
    pub end_system_delay: u16,
    
    /// Signal level
    pub signal_level: u8,
    
    /// Noise level
    pub noise_level: u8,
    
    /// Residual Echo Return Loss
    pub rerl: u8,
    
    /// Gmin
    pub gmin: u8,
    
    /// R factor
    pub r_factor: u8,
    
    /// External R factor
    pub ext_r_factor: u8,
    
    /// MOS-LQ
    pub mos_lq: u8,
    
    /// MOS-CQ
    pub mos_cq: u8,
    
    /// Receiver configuration
    pub rx_config: u8,
    
    /// Jitter buffer nominal delay
    pub jb_nominal: u16,
    
    /// Jitter buffer maximum delay
    pub jb_maximum: u16,
    
    /// Jitter buffer absolute maximum delay
    pub jb_abs_max: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_receiver_reference_time_block() {
        // Create a block
        let ntp = NtpTimestamp {
            seconds: 0x12345678,
            fraction: 0xabcdef01,
        };
        
        let block = ReceiverReferenceTimeBlock { ntp };
        
        // Wrap in an XR block
        let xr_block = RtcpXrBlock::ReceiverReferenceTimes(block);
        
        // Check block type
        assert_eq!(xr_block.block_type(), RtcpXrBlockType::ReceiverReferenceTimes);
        
        // Check size
        assert_eq!(xr_block.size(), 12); // 4-byte header + 8-byte NTP timestamp
        
        // Serialize
        let mut buf = BytesMut::with_capacity(xr_block.size());
        xr_block.serialize(&mut buf).unwrap();
        
        // Check serialized data
        assert_eq!(buf.len(), 12);
        assert_eq!(buf[0], RtcpXrBlockType::ReceiverReferenceTimes as u8);
        assert_eq!(buf[1], 0); // Reserved
        assert_eq!(buf[2], 0); // Length high byte
        assert_eq!(buf[3], 2); // Length low byte (2 words = 8 bytes)
    }
    
    #[test]
    fn test_xr_packet() {
        // Create an XR packet
        let mut xr = RtcpExtendedReport::new(0x12345678);
        
        // Add a receiver reference time block
        let ntp = NtpTimestamp {
            seconds: 0x12345678,
            fraction: 0xabcdef01,
        };
        
        xr.add_block(RtcpXrBlock::ReceiverReferenceTimes(
            ReceiverReferenceTimeBlock { ntp }
        ));
        
        // Serialize
        let buf = xr.serialize().unwrap();
        
        // Check serialized data
        assert_eq!(buf.len(), 16); // 4-byte SSRC + 12-byte block
        assert_eq!(&buf[0..4], &0x12345678u32.to_be_bytes());
    }
} 