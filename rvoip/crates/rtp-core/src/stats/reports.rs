use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::packet::rtcp::{
    NtpTimestamp, RtcpReportBlock, RtcpSenderReport, RtcpReceiverReport,
    RtcpSourceDescription, RtcpSdesChunk, RtcpSdesItem
};
use crate::{RtpSsrc, RtpTimestamp};
use crate::stats::loss::PacketLossTracker;

/// RTCP interval constants (RFC 3550)
pub const RTCP_MIN_INTERVAL: Duration = Duration::from_millis(5000); // 5 seconds
pub const RTCP_BANDWIDTH_FRACTION: f64 = 0.05; // 5% of session bandwidth
pub const RTCP_SENDER_BANDWIDTH_FRACTION: f64 = 0.25; // 25% of RTCP bandwidth to senders

/// RTCP report generator
#[derive(Debug)]
pub struct RtcpReportGenerator {
    /// Local SSRC
    local_ssrc: RtpSsrc,
    
    /// CNAME for SDES reports
    cname: String,
    
    /// Packet loss statistics by SSRC
    loss_stats: HashMap<RtpSsrc, PacketLossTracker>,
    
    /// Time of last SR sent
    last_sr_time: Option<Instant>,
    
    /// Last SR NTP timestamp sent
    last_sr_ntp: Option<NtpTimestamp>,
    
    /// Last RTP timestamp used in SR
    last_rtp_timestamp: Option<RtpTimestamp>,
    
    /// Total packets sent
    packets_sent: u32,
    
    /// Total octets sent
    octets_sent: u32,
    
    /// Last RTCP interval
    last_interval: Duration,
    
    /// Session bandwidth in bits per second
    session_bandwidth: u32,
    
    /// Number of senders in the session
    senders: u32,
    
    /// Number of receivers in the session
    receivers: u32,
    
    /// RTCP transmission enabled
    enabled: bool,
}

impl RtcpReportGenerator {
    /// Create a new RTCP report generator
    pub fn new(local_ssrc: RtpSsrc, cname: String) -> Self {
        Self {
            local_ssrc,
            cname,
            loss_stats: HashMap::new(),
            last_sr_time: None,
            last_sr_ntp: None,
            last_rtp_timestamp: None,
            packets_sent: 0,
            octets_sent: 0,
            last_interval: RTCP_MIN_INTERVAL,
            session_bandwidth: 64000, // Default 64 kbps
            senders: 1,
            receivers: 0,
            enabled: true,
        }
    }
    
    /// Set the session bandwidth
    pub fn set_bandwidth(&mut self, bandwidth_bps: u32) {
        self.session_bandwidth = bandwidth_bps;
    }
    
    /// Update statistics for sent packets
    pub fn update_sent_stats(&mut self, packets: u32, octets: u32) {
        self.packets_sent += packets;
        self.octets_sent += octets;
    }
    
    /// Process a received RTP packet
    pub fn process_received_packet(&mut self, ssrc: RtpSsrc, seq: u16) {
        let tracker = self.loss_stats.entry(ssrc)
            .or_insert_with(|| PacketLossTracker::new());
        tracker.process(seq);
    }
    
    /// Calculate RTCP interval based on session parameters
    pub fn calculate_interval(&mut self) -> Duration {
        // RFC 3550 algorithm
        let rtcp_bw = (self.session_bandwidth as f64 * RTCP_BANDWIDTH_FRACTION) as u32;
        let members = self.senders + self.receivers;
        
        if members == 0 {
            return RTCP_MIN_INTERVAL;
        }
        
        // Interval = packet size * members / RTCP bandwidth
        let avg_rtcp_size = 100; // Average RTCP packet size in bytes
        let interval_seconds = (avg_rtcp_size * 8 * members) as f64 / rtcp_bw as f64;
        
        // Apply randomization (0.5 to 1.5 of calculated interval)
        let randomizer = rand::random::<f64>() + 0.5; // Random value between 0.5 and 1.5
        let interval = Duration::from_secs_f64(interval_seconds * randomizer);
        
        // Enforce minimum interval
        let interval = if interval < RTCP_MIN_INTERVAL {
            RTCP_MIN_INTERVAL
        } else {
            interval
        };
        
        self.last_interval = interval;
        interval
    }
    
    /// Generate a Sender Report (SR)
    pub fn generate_sender_report(&mut self, rtp_timestamp: RtpTimestamp) -> RtcpSenderReport {
        // Create NTP timestamp for now
        let ntp = NtpTimestamp::now();
        self.last_sr_ntp = Some(ntp);
        self.last_sr_time = Some(Instant::now());
        self.last_rtp_timestamp = Some(rtp_timestamp);
        
        // Create report blocks for each source we're receiving from
        let mut report_blocks = Vec::new();
        
        for (ssrc, tracker) in &self.loss_stats {
            let stats = tracker.get_stats();
            
            let report_block = RtcpReportBlock {
                ssrc: *ssrc,
                fraction_lost: stats.fraction_lost,
                cumulative_lost: tracker.get_cumulative_lost(),
                highest_seq: stats.packets_expected as u32,
                jitter: 0, // Jitter would be calculated separately
                last_sr: 0, // Would be from received SRs
                delay_since_last_sr: 0, // Would be from received SRs
            };
            
            report_blocks.push(report_block);
            
            // Only include up to 31 report blocks (5-bit field in RTCP header)
            if report_blocks.len() >= 31 {
                break;
            }
        }
        
        // Create SR
        RtcpSenderReport {
            ssrc: self.local_ssrc,
            ntp_timestamp: ntp,
            rtp_timestamp,
            sender_packet_count: self.packets_sent,
            sender_octet_count: self.octets_sent,
            report_blocks,
        }
    }
    
    /// Generate a Receiver Report (RR)
    pub fn generate_receiver_report(&self) -> RtcpReceiverReport {
        // Create report blocks for each source we're receiving from
        let mut report_blocks = Vec::new();
        
        for (ssrc, tracker) in &self.loss_stats {
            let stats = tracker.get_stats();
            
            let report_block = RtcpReportBlock {
                ssrc: *ssrc,
                fraction_lost: stats.fraction_lost,
                cumulative_lost: tracker.get_cumulative_lost(),
                highest_seq: stats.packets_expected as u32,
                jitter: 0, // Jitter would be calculated separately
                last_sr: 0, // Would be from received SRs
                delay_since_last_sr: 0, // Would be from received SRs
            };
            
            report_blocks.push(report_block);
            
            // Only include up to 31 report blocks (5-bit field in RTCP header)
            if report_blocks.len() >= 31 {
                break;
            }
        }
        
        // Create RR
        RtcpReceiverReport {
            ssrc: self.local_ssrc,
            report_blocks,
        }
    }
    
    /// Generate SDES (Source Description) packet
    pub fn generate_sdes(&self) -> RtcpSourceDescription {
        let mut sdes = RtcpSourceDescription::new();
        
        // Create chunk for local source
        let mut chunk = RtcpSdesChunk::new(self.local_ssrc);
        
        // Add CNAME
        chunk.add_item(RtcpSdesItem::cname(self.cname.clone()));
        
        // Add optional items (could add more like NAME, TOOL, etc.)
        
        // Add chunk to SDES packet
        sdes.add_chunk(chunk);
        
        sdes
    }
    
    /// Whether it's time to send an RTCP report
    pub fn should_send_report(&self) -> bool {
        if !self.enabled {
            return false;
        }
        
        if let Some(last_time) = self.last_sr_time {
            Instant::now().duration_since(last_time) >= self.last_interval
        } else {
            // No reports sent yet, should send initial report
            true
        }
    }
    
    /// Enable or disable RTCP transmission
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    
    /// Update session members
    pub fn update_members(&mut self, senders: u32, receivers: u32) {
        self.senders = senders;
        self.receivers = receivers;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_rtcp_interval() {
        let mut generator = RtcpReportGenerator::new(0x12345678, "user@example.com".to_string());
        
        // Default interval should be at least the minimum
        let interval = generator.calculate_interval();
        assert!(interval >= RTCP_MIN_INTERVAL);
        
        // Interval should decrease with higher bandwidth
        generator.set_bandwidth(1000000); // 1 Mbps
        let high_bw_interval = generator.calculate_interval();
        
        generator.set_bandwidth(10000); // 10 kbps
        let low_bw_interval = generator.calculate_interval();
        
        // Higher bandwidth should result in shorter intervals
        assert!(high_bw_interval <= low_bw_interval);
    }
    
    #[test]
    fn test_sender_report_generation() {
        let mut generator = RtcpReportGenerator::new(0x12345678, "user@example.com".to_string());
        
        // Update stats
        generator.update_sent_stats(100, 10000);
        
        // Process some received packets
        let remote_ssrc = 0xabcdef01;
        for seq in 1000..1010 {
            generator.process_received_packet(remote_ssrc, seq);
        }
        
        // Generate SR
        let sr = generator.generate_sender_report(12345);
        
        // Verify SR fields
        assert_eq!(sr.ssrc, 0x12345678);
        assert_eq!(sr.rtp_timestamp, 12345);
        assert_eq!(sr.sender_packet_count, 100);
        assert_eq!(sr.sender_octet_count, 10000);
        
        // Should have one report block for the remote source
        assert_eq!(sr.report_blocks.len(), 1);
        assert_eq!(sr.report_blocks[0].ssrc, remote_ssrc);
        assert_eq!(sr.report_blocks[0].fraction_lost, 0); // No loss in our test
    }
    
    #[test]
    fn test_sdes_generation() {
        let generator = RtcpReportGenerator::new(0x12345678, "user@example.com".to_string());
        
        // Generate SDES
        let sdes = generator.generate_sdes();
        
        // Verify SDES
        assert_eq!(sdes.chunks.len(), 1);
        assert_eq!(sdes.chunks[0].ssrc, 0x12345678);
        assert_eq!(sdes.chunks[0].items.len(), 1);
        assert_eq!(sdes.chunks[0].items[0].value, "user@example.com");
    }
} 