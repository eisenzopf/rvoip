use bytes::{Buf, BufMut, BytesMut};

use crate::quality::e_model::{self, CodecImpairment, QualityInputs, G711};

use super::NtpTimestamp;
use crate::error::Error;
use crate::{Result, RtpSsrc};

const VOIP_METRICS_BLOCK_CONTENT_LENGTH: usize = 32;

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
            _ => Err(Error::RtcpError(format!(
                "Unknown XR block type: {}",
                value
            ))),
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

    /// Add a VoIP metrics block
    pub fn add_voip_metrics(&mut self, metrics: VoipMetricsBlock) {
        self.blocks.push(RtcpXrBlock::VoipMetrics(metrics));
    }

    /// Add a receiver reference time block
    pub fn add_receiver_reference_time(&mut self, ntp: NtpTimestamp) {
        self.blocks.push(RtcpXrBlock::ReceiverReferenceTimes(
            ReceiverReferenceTimeBlock { ntp },
        ));
    }

    /// Get the total size of the XR packet in bytes
    pub fn size(&self) -> usize {
        4 + self.blocks.iter().map(|b| b.size()).sum::<usize>()
    }

    /// Serialize the XR packet to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let total_size = self.size();
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
            }
            RtcpXrBlockType::DuplicateRle => {
                // Parse Duplicate RLE block
                RtcpXrBlock::DuplicateRle(parse_duplicate_rle_block(buf, block_length)?)
            }
            RtcpXrBlockType::PacketReceiptTimes => {
                // Parse Packet Receipt Times block
                RtcpXrBlock::PacketReceiptTimes(parse_packet_receipt_times_block(
                    buf,
                    block_length,
                )?)
            }
            RtcpXrBlockType::ReceiverReferenceTimes => {
                // Parse Receiver Reference Time block
                RtcpXrBlock::ReceiverReferenceTimes(parse_receiver_reference_time_block(buf)?)
            }
            RtcpXrBlockType::Dlrr => {
                // Parse DLRR block
                RtcpXrBlock::Dlrr(parse_dlrr_block(buf, block_length)?)
            }
            RtcpXrBlockType::StatisticsSummary => {
                // Parse Statistics Summary block
                RtcpXrBlock::StatisticsSummary(parse_statistics_summary_block(buf)?)
            }
            RtcpXrBlockType::VoipMetrics => {
                if block_length != VOIP_METRICS_BLOCK_CONTENT_LENGTH {
                    return Err(Error::RtcpError(format!(
                        "Invalid VoIP Metrics block length: expected {} bytes, got {}",
                        VOIP_METRICS_BLOCK_CONTENT_LENGTH, block_length
                    )));
                }

                // Restrict the parser to this report block so malformed input
                // can never consume bytes belonging to the following block.
                let mut block_buf = buf.take(block_length);
                let metrics = parse_voip_metrics_block(&mut block_buf)?;
                if block_buf.has_remaining() {
                    return Err(Error::RtcpError(format!(
                        "VoIP Metrics parser left {} bytes unread",
                        block_buf.remaining()
                    )));
                }
                RtcpXrBlock::VoipMetrics(metrics)
            }
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

fn parse_packet_receipt_times_block(
    buf: &mut impl Buf,
    length: usize,
) -> Result<PacketReceiptTimesBlock> {
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
    if buf.remaining() < length {
        return Err(Error::BufferTooSmall {
            required: length,
            available: buf.remaining(),
        });
    }

    let mut sub_blocks = Vec::new();
    let sub_block_len = 12; // Each DLRR sub-block is 12 bytes

    // Process all sub-blocks
    let num_sub_blocks = length / sub_block_len;
    for _ in 0..num_sub_blocks {
        if buf.remaining() < sub_block_len {
            break;
        }

        let ssrc = buf.get_u32();
        let last_rr = buf.get_u32();
        let delay = buf.get_u32();

        sub_blocks.push(DlrrSubBlock {
            ssrc,
            last_rr,
            delay,
        });
    }

    Ok(DlrrBlock { sub_blocks })
}

fn parse_statistics_summary_block(buf: &mut impl Buf) -> Result<StatisticsSummaryBlock> {
    // 18 bytes of mandatory fixed fields are read below (ssrc..dup_packets);
    // the optional jitter/ttl fields are length-checked individually. The
    // guard previously read 16, which allowed a 2-byte over-read (panic) on a
    // truncated block. See fuzz regression test `xr_statistics_summary_truncated`.
    if buf.remaining() < 18 {
        return Err(Error::BufferTooSmall {
            required: 18,
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
    // 32 bytes of mandatory fixed fields are read below (ssrc..jb_abs_max).
    // The guard previously read 24, allowing an 8-byte over-read (panic) on a
    // truncated block.
    if buf.remaining() < VOIP_METRICS_BLOCK_CONTENT_LENGTH {
        return Err(Error::BufferTooSmall {
            required: VOIP_METRICS_BLOCK_CONTENT_LENGTH,
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
            RtcpXrBlock::VoipMetrics(_) => 4 + VOIP_METRICS_BLOCK_CONTENT_LENGTH,
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
            RtcpXrBlock::LossRle(block) => {
                // Implementation incomplete
                buf.put_u32(block.ssrc);
                buf.put_u16(block.begin_seq);
                buf.put_u16(block.end_seq);
                // We would add the chunks here in a full implementation
            }
            RtcpXrBlock::DuplicateRle(block) => {
                // Implementation incomplete
                buf.put_u32(block.ssrc);
                buf.put_u16(block.begin_seq);
                buf.put_u16(block.end_seq);
                // We would add the chunks here in a full implementation
            }
            RtcpXrBlock::PacketReceiptTimes(block) => {
                // Implementation incomplete
                buf.put_u32(block.ssrc);
                buf.put_u16(block.begin_seq);
                buf.put_u16(block.end_seq);
                // We would add the receipt times here in a full implementation
            }
            RtcpXrBlock::ReceiverReferenceTimes(block) => {
                buf.put_u32(block.ntp.seconds);
                buf.put_u32(block.ntp.fraction);
            }
            RtcpXrBlock::Dlrr(block) => {
                // Serialize each sub-block
                for sub_block in &block.sub_blocks {
                    buf.put_u32(sub_block.ssrc);
                    buf.put_u32(sub_block.last_rr);
                    buf.put_u32(sub_block.delay);
                }
            }
            RtcpXrBlock::StatisticsSummary(block) => {
                buf.put_u32(block.ssrc);

                // Construct flags byte
                let mut flags = 0u8;
                if block.loss_report {
                    flags |= 0x01;
                }
                if block.duplicate_report {
                    flags |= 0x02;
                }
                if block.jitter_report {
                    flags |= 0x04;
                }
                if block.ttr_report {
                    flags |= 0x08;
                }

                buf.put_u8(flags);
                buf.put_u8(0); // Reserved
                buf.put_u16(block.begin_seq);
                buf.put_u16(block.end_seq);
                buf.put_u32(block.lost_packets);
                buf.put_u32(block.dup_packets);

                // Add optional jitter fields if present
                if block.jitter_report {
                    if let Some(min_jitter) = block.min_jitter {
                        buf.put_u32(min_jitter);
                    } else {
                        buf.put_u32(0);
                    }

                    if let Some(max_jitter) = block.max_jitter {
                        buf.put_u32(max_jitter);
                    } else {
                        buf.put_u32(0);
                    }

                    if let Some(mean_jitter) = block.mean_jitter {
                        buf.put_u32(mean_jitter);
                    } else {
                        buf.put_u32(0);
                    }

                    if let Some(dev_jitter) = block.dev_jitter {
                        buf.put_u32(dev_jitter);
                    } else {
                        buf.put_u32(0);
                    }
                }
            }
            RtcpXrBlock::VoipMetrics(block) => {
                buf.put_u32(block.ssrc);
                buf.put_u8(block.loss_rate);
                buf.put_u8(block.discard_rate);
                buf.put_u8(block.burst_density);
                buf.put_u8(block.gap_density);
                buf.put_u16(block.burst_duration);
                buf.put_u16(block.gap_duration);
                buf.put_u16(block.round_trip_delay);
                buf.put_u16(block.end_system_delay);
                buf.put_u8(block.signal_level);
                buf.put_u8(block.noise_level);
                buf.put_u8(block.rerl);
                buf.put_u8(block.gmin);
                buf.put_u8(block.r_factor);
                buf.put_u8(block.ext_r_factor);
                buf.put_u8(block.mos_lq);
                buf.put_u8(block.mos_cq);
                buf.put_u8(block.rx_config);
                buf.put_u8(0); // Reserved
                buf.put_u16(block.jb_nominal);
                buf.put_u16(block.jb_maximum);
                buf.put_u16(block.jb_abs_max);
            }
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

    /// R factor, encoded directly from 0 to 100 or 127 when unavailable.
    pub r_factor: u8,

    /// External R factor, encoded directly from 0 to 100 or 127 when unavailable.
    pub ext_r_factor: u8,

    /// MOS-LQ multiplied by 10, or 127 when unavailable.
    pub mos_lq: u8,

    /// MOS-CQ multiplied by 10, or 127 when unavailable.
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

impl VoipMetricsBlock {
    /// Create a new VoIP metrics block
    pub fn new(ssrc: RtpSsrc) -> Self {
        Self {
            ssrc,
            loss_rate: 0,
            discard_rate: 0,
            burst_density: 0,
            gap_density: 0,
            burst_duration: 0,
            gap_duration: 0,
            round_trip_delay: 0,
            end_system_delay: 0,
            signal_level: 127,
            noise_level: 127,
            rerl: 127,
            gmin: 16, // Default value from RFC 3611
            r_factor: 127,
            ext_r_factor: 127,
            mos_lq: 127,
            mos_cq: 127,
            rx_config: 0,
            jb_nominal: 0,
            jb_maximum: 0,
            jb_abs_max: 0,
        }
    }

    /// Calculate RFC 3611 quality fields using G.711 impairment parameters.
    ///
    /// This compatibility method preserves the existing API. New code that
    /// negotiates another codec should use [`Self::calculate_r_factor_with_codec`].
    /// Jitter is treated as additional playout delay. Jitter-buffer discards
    /// must already be included in `packet_loss_percent`.
    ///
    /// # Arguments
    ///
    /// * `packet_loss_percent` - Effective network loss plus buffer discards.
    /// * `round_trip_ms` - Round-trip delay in milliseconds.
    /// * `jitter_ms` - Additional jitter-buffer playout delay in milliseconds.
    pub fn calculate_r_factor(
        &mut self,
        packet_loss_percent: f32,
        round_trip_ms: u16,
        jitter_ms: f32,
    ) {
        self.calculate_r_factor_with_codec(packet_loss_percent, round_trip_ms, jitter_ms, G711);
    }

    /// Calculate RFC 3611 quality fields for a negotiated codec.
    ///
    /// Values are encoded according to RFC 3611 section 4.7. Jitter is
    /// treated as additional playout delay, while jitter-buffer discards must
    /// be included in `packet_loss_percent`.
    ///
    /// # Arguments
    ///
    /// * `packet_loss_percent` - Effective network loss plus buffer discards.
    /// * `round_trip_ms` - Round-trip delay in milliseconds.
    /// * `jitter_ms` - Additional jitter-buffer playout delay in milliseconds.
    /// * `codec` - ITU-T G.113 equipment impairment parameters.
    pub fn calculate_r_factor_with_codec(
        &mut self,
        packet_loss_percent: f32,
        round_trip_ms: u16,
        jitter_ms: f32,
        codec: CodecImpairment,
    ) {
        let one_way_delay_ms = round_trip_ms as f32 / 2.0 + jitter_ms.max(0.0);
        let scores = e_model::evaluate(QualityInputs {
            one_way_delay_ms,
            loss_percent: packet_loss_percent,
            codec,
        });

        self.r_factor = scores.r_factor.round() as u8;
        self.ext_r_factor = 127;
        self.mos_lq = (scores.mos_lq * 10.0).round() as u8;
        self.mos_cq = (scores.mos_cq * 10.0).round() as u8;
    }
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
        assert_eq!(
            xr_block.block_type(),
            RtcpXrBlockType::ReceiverReferenceTimes
        );

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
    fn test_voip_metrics_block() {
        // Create a VoIP metrics block
        let mut voip_metrics = VoipMetricsBlock::new(0x12345678);

        // Set some metrics
        voip_metrics.loss_rate = 5; // 5% loss
        voip_metrics.discard_rate = 2; // 2% discard
        voip_metrics.round_trip_delay = 150; // 150ms RTT
        voip_metrics.end_system_delay = 20; // 20ms end system delay

        // Calculate R-factor and MOS scores
        voip_metrics.calculate_r_factor(5.0, 150, 30.0);

        // Wrap in an XR block
        let xr_block = RtcpXrBlock::VoipMetrics(voip_metrics);

        // Check block type
        assert_eq!(xr_block.block_type(), RtcpXrBlockType::VoipMetrics);

        // Serialize
        let buf = BytesMut::with_capacity(100);
        let mut buf_clone = buf.clone();
        xr_block.serialize(&mut buf_clone).unwrap();

        // The actual serialized size is what matters for the test
        let serialized_size = buf_clone.len();
        assert_eq!(serialized_size, 36);

        assert_eq!(buf_clone[0], RtcpXrBlockType::VoipMetrics as u8);
        assert_eq!(buf_clone[1], 0); // Reserved
        assert_eq!(buf_clone[2], 0); // Length high byte
        assert_eq!(buf_clone[3], 8); // Length low byte (8 words = 32 bytes)
        assert!((10..=50).contains(&buf_clone[26]));
        assert!((10..=50).contains(&buf_clone[27]));

        // Parse back
        let mut read_buf = buf_clone.freeze();
        let block_type = RtcpXrBlockType::try_from(read_buf[0]).unwrap();
        read_buf.advance(4); // Skip header

        assert_eq!(block_type, RtcpXrBlockType::VoipMetrics);
        let parsed_metrics = parse_voip_metrics_block(&mut read_buf).unwrap();

        // Check parsed fields
        assert_eq!(parsed_metrics.ssrc, 0x12345678);
        assert_eq!(parsed_metrics.loss_rate, 5);
        assert_eq!(parsed_metrics.discard_rate, 2);
        assert_eq!(parsed_metrics.round_trip_delay, 150);
        assert_eq!(parsed_metrics.end_system_delay, 20);
        assert!(parsed_metrics.r_factor > 0);
        assert!(parsed_metrics.mos_lq > 0);
        assert!(parsed_metrics.mos_cq > 0);
    }

    #[test]
    fn voip_metrics_defaults_quality_fields_to_unavailable() {
        let metrics = VoipMetricsBlock::new(0x12345678);
        let block = RtcpXrBlock::VoipMetrics(metrics);
        let mut serialized = BytesMut::new();

        block.serialize(&mut serialized).unwrap();

        assert_eq!(serialized[20], 127); // signal level
        assert_eq!(serialized[21], 127); // noise level
        assert_eq!(serialized[22], 127); // RERL
        assert_eq!(serialized[24], 127); // R factor
        assert_eq!(serialized[25], 127); // external R factor
        assert_eq!(serialized[26], 127); // MOS-LQ
        assert_eq!(serialized[27], 127); // MOS-CQ
    }

    #[test]
    fn voip_metrics_quality_fields_survive_round_trip() {
        let mut metrics = VoipMetricsBlock::new(0x12345678);
        metrics.calculate_r_factor_with_codec(1.0, 50, 5.0, G711);
        let expected = metrics.clone();
        let block = RtcpXrBlock::VoipMetrics(metrics);
        let mut serialized = BytesMut::new();
        block.serialize(&mut serialized).unwrap();
        serialized.advance(4);

        let parsed = parse_voip_metrics_block(&mut serialized).unwrap();

        assert_eq!(parsed, expected);
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
            ReceiverReferenceTimeBlock { ntp },
        ));

        // Add a VoIP metrics block
        let mut voip_metrics = VoipMetricsBlock::new(0x87654321);
        voip_metrics.loss_rate = 3;
        voip_metrics.round_trip_delay = 120;
        voip_metrics.calculate_r_factor(3.0, 120, 25.0);

        xr.add_block(RtcpXrBlock::VoipMetrics(voip_metrics));

        // Serialize
        let buf = xr.serialize().unwrap();

        // Calculate the size based on actual blocks
        let expected_size = 4 + // SSRC
            xr.blocks.iter().map(|b| b.size()).sum::<usize>();

        println!("XR packet blocks: {}", xr.blocks.len());
        for (i, block) in xr.blocks.iter().enumerate() {
            println!(
                "Block {} type: {:?}, size: {}",
                i,
                block.block_type(),
                block.size()
            );
        }
        println!("XR packet expected size: {}", expected_size);
        println!("XR packet buffer size: {}", buf.len());

        assert_eq!(buf.len(), 52);
        assert_eq!(&buf[0..4], &0x12345678u32.to_be_bytes());

        // Parse back
        let mut read_buf = buf.freeze();
        let parsed_xr = parse_xr(&mut read_buf).unwrap();

        assert_eq!(read_buf.remaining(), 0);
        assert_eq!(parsed_xr, xr);

        match &parsed_xr.blocks[0] {
            RtcpXrBlock::ReceiverReferenceTimes(block) => {
                assert_eq!(block.ntp.seconds, 0x12345678);
                assert_eq!(block.ntp.fraction, 0xabcdef01);
            }
            _ => panic!("Expected ReferenceTimeBlock"),
        }

        match &parsed_xr.blocks[1] {
            RtcpXrBlock::VoipMetrics(block) => {
                assert_eq!(block.ssrc, 0x87654321);
                assert_eq!(block.loss_rate, 3);
                assert_eq!(block.round_trip_delay, 120);
            }
            _ => panic!("Expected VoipMetricsBlock"),
        }
    }

    #[test]
    fn parse_xr_rejects_short_voip_metrics_without_consuming_the_next_block() {
        let mut serialized = BytesMut::new();
        serialized.put_u32(0x12345678); // XR sender SSRC
        serialized.put_u8(RtcpXrBlockType::VoipMetrics as u8);
        serialized.put_u8(0);
        serialized.put_u16(7); // Invalid: 28 content bytes instead of 32
        serialized.extend_from_slice(&[0; 28]);

        let next_block_offset = serialized.len();
        RtcpXrBlock::ReceiverReferenceTimes(ReceiverReferenceTimeBlock {
            ntp: NtpTimestamp {
                seconds: 0x11223344,
                fraction: 0x55667788,
            },
        })
        .serialize(&mut serialized)
        .unwrap();

        let expected_next_block = serialized[next_block_offset..].to_vec();
        let mut read_buf = serialized.freeze();
        let error = parse_xr(&mut read_buf).unwrap_err();

        assert!(matches!(error, Error::RtcpError(_)));
        assert_eq!(read_buf.remaining(), 28 + 12);
        assert_eq!(
            &read_buf.chunk()[28..],
            expected_next_block.as_slice(),
            "the following XR block must remain untouched"
        );
    }

    #[test]
    fn public_rtcp_api_round_trips_a_complete_voip_metrics_report() {
        let mut metrics = VoipMetricsBlock::new(0x87654321);
        metrics.loss_rate = 5;
        metrics.discard_rate = 2;
        metrics.round_trip_delay = 80;
        metrics.signal_level = (-18_i8) as u8;
        metrics.noise_level = (-65_i8) as u8;
        metrics.rerl = 42;
        metrics.calculate_r_factor_with_codec(7.0, 80, 20.0, G711);

        let mut xr = RtcpExtendedReport::new(0x12345678);
        xr.add_voip_metrics(metrics);
        let expected = crate::RtcpPacket::ExtendedReport(xr);

        let serialized = expected.serialize().unwrap();
        let parsed = crate::RtcpPacket::parse(&serialized).unwrap();

        assert_eq!(serialized.len(), 44); // 8-byte XR header + 36-byte report block
        assert_eq!(parsed, expected);
    }
}
