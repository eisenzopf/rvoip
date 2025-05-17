//! Example demonstrating RTCP Extended Reports (XR) and Compound Packets
//!
//! This example shows how to create, serialize, and parse RTCP XR
//! packets with VoIP metrics and include them in compound RTCP packets.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use bytes::{Bytes, BytesMut};
use tracing::{info, debug};

use rvoip_rtp_core::{
    RtpSsrc, RtcpSenderReport, RtcpReceiverReport, 
    RtcpReportBlock, NtpTimestamp, RtcpGoodbye,
    RtcpExtendedReport, RtcpXrBlock, VoipMetricsBlock,
    RtcpCompoundPacket, RtcpPacket,
    transport::UdpRtpTransport,
    session::{RtpSession, RtpSessionConfig},
};

fn calculate_ntp_timestamp() -> NtpTimestamp {
    // Get current time since UNIX epoch
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0));
    
    // NTP timestamp starts at Jan 1, 1900, which is 70 years before UNIX epoch
    // (plus 17 leap days)
    let ntp_sec = now.as_secs() + 2_208_988_800;
    
    // Convert nanoseconds to NTP fraction (2^32 fractions per second)
    let nanos = now.subsec_nanos() as u64;
    let ntp_frac = (nanos << 32) / 1_000_000_000;
    
    NtpTimestamp {
        seconds: ntp_sec as u32,
        fraction: ntp_frac as u32,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("RTCP Extended Reports (XR) Example");
    
    // Create an RTCP XR packet with VoIP metrics
    let mut xr = RtcpExtendedReport::new(0x12345678);
    
    // Add a receiver reference time block with current time
    let ntp = calculate_ntp_timestamp();
    xr.add_receiver_reference_time(ntp);
    
    // Create VoIP metrics block
    let mut voip_metrics = VoipMetricsBlock::new(0x87654321);
    
    // Set some example metrics
    voip_metrics.loss_rate = 5;       // 5% packet loss
    voip_metrics.discard_rate = 2;    // 2% discard rate
    voip_metrics.burst_density = 10;  // 10% of lost packets are in bursts
    voip_metrics.gap_density = 3;     // 3% of packets in gaps are lost
    voip_metrics.burst_duration = 120; // 120ms average burst duration
    voip_metrics.gap_duration = 5000;  // 5000ms average gap duration
    voip_metrics.round_trip_delay = 150; // 150ms round-trip delay
    voip_metrics.end_system_delay = 40;  // 40ms end system delay
    voip_metrics.signal_level = 30;    // -30 dBm signal level
    voip_metrics.noise_level = 70;     // -70 dBm noise level
    voip_metrics.rerl = 25;           // 25 dB residual echo return loss
    voip_metrics.jb_nominal = 60;     // 60ms nominal jitter buffer
    voip_metrics.jb_maximum = 120;    // 120ms maximum jitter buffer
    
    // Calculate R-factor and MOS scores
    voip_metrics.calculate_r_factor(5.0, 150, 30.0);
    
    info!("Generated VoIP metrics with R-factor={} (MOS-LQ={}, MOS-CQ={})",
          voip_metrics.r_factor, voip_metrics.mos_lq, voip_metrics.mos_cq);
    
    // Add VoIP metrics to XR packet
    xr.add_block(RtcpXrBlock::VoipMetrics(voip_metrics));
    
    // Create a sender report for a compound packet
    let sr = RtcpSenderReport {
        ssrc: 0x12345678,
        ntp_timestamp: ntp,
        rtp_timestamp: 0x87654321,
        packet_count: 1000,
        octet_count: 128000,
        report_blocks: Vec::new(),
    };
    
    // Create a compound packet
    let mut compound = RtcpCompoundPacket::new_with_sr(sr);
    
    // Add the XR packet to the compound packet
    compound.add_xr(xr.clone());
    
    // Add a BYE packet as well
    let bye = RtcpGoodbye {
        sources: vec![0x12345678],
        reason: Some("Example complete".to_string()),
    };
    
    compound.add_bye(bye);
    
    // Serialize the compound packet
    let compound_bytes = compound.serialize()?;
    
    info!("Serialized compound packet of {} bytes containing SR, XR, and BYE", 
          compound_bytes.len());
    
    // Parse the compound packet back
    let parsed_compound = RtcpCompoundPacket::parse(&compound_bytes)?;
    
    info!("Successfully parsed compound packet with {} RTCP packets", 
          parsed_compound.packets.len());
    
    // Examine the parsed packets
    for (i, packet) in parsed_compound.packets.iter().enumerate() {
        match packet {
            RtcpPacket::SenderReport(sr) => {
                info!("Packet {}: Sender Report from SSRC 0x{:08x}", i, sr.ssrc);
            },
            RtcpPacket::ReceiverReport(rr) => {
                info!("Packet {}: Receiver Report from SSRC 0x{:08x}", i, rr.ssrc);
            },
            RtcpPacket::ExtendedReport(xr) => {
                info!("Packet {}: Extended Report from SSRC 0x{:08x} with {} blocks", 
                      i, xr.ssrc, xr.blocks.len());
                
                // Examine XR blocks
                for (j, block) in xr.blocks.iter().enumerate() {
                    match block {
                        RtcpXrBlock::VoipMetrics(metrics) => {
                            info!("  Block {}: VoIP Metrics for SSRC 0x{:08x}", j, metrics.ssrc);
                            info!("    Loss Rate: {}%, R-Factor: {}, MOS-LQ: {}, MOS-CQ: {}", 
                                  metrics.loss_rate, metrics.r_factor, metrics.mos_lq, metrics.mos_cq);
                        },
                        RtcpXrBlock::ReceiverReferenceTimes(ref_time) => {
                            info!("  Block {}: Receiver Reference Time (NTP: {}.{})",
                                  j, ref_time.ntp.seconds, ref_time.ntp.fraction);
                        },
                        _ => {
                            info!("  Block {}: {:?}", j, block.block_type());
                        }
                    }
                }
            },
            RtcpPacket::Goodbye(bye) => {
                info!("Packet {}: Goodbye from {} sources (Reason: {:?})", 
                      i, bye.sources.len(), bye.reason);
            },
            _ => {
                info!("Packet {}: {:?}", i, packet.packet_type());
            }
        }
    }
    
    info!("RTCP XR Example completed successfully");
    
    Ok(())
} 