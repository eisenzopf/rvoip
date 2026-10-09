use std::collections::HashMap;
use std::time::{Duration, Instant};

use rand::Rng;

use crate::packet::rtcp::{
    NtpTimestamp, RtcpReceiverReport, RtcpReportBlock, RtcpSdesChunk, RtcpSdesItem,
    RtcpSenderReport, RtcpSourceDescription,
};
use crate::stats::loss::PacketLossTracker;
use crate::{RtpSsrc, RtpTimestamp};

/// RFC 3550 §6.2 fixed minimum RTCP report interval.
pub const RTCP_MIN_INTERVAL: Duration = Duration::from_secs(5);
/// RFC 3550 §6.2: RTCP gets 5% of the session bandwidth.
pub const RTCP_BANDWIDTH_FRACTION: f64 = 0.05;
/// RFC 3550 §6.2: senders share 25% of the RTCP bandwidth when they are at
/// most a quarter of the members.
pub const RTCP_SENDER_BANDWIDTH_FRACTION: f64 = 0.25;
/// RFC 3550 §6.2: receivers share the remaining 75% in that case.
pub const RTCP_RECEIVER_BANDWIDTH_FRACTION: f64 = 0.75;
/// RFC 3550 §6.3.1 / Appendix A.7 compensation for timer reconsideration
/// converging below the intended average: `e - 3/2`.
pub const RTCP_COMPENSATION: f64 = std::f64::consts::E - 1.5;
/// UDP and IPv4 header octets added to every RTCP packet size when tracking
/// the average compound size (RFC 3550 §6.3.3).
pub const RTCP_LOWER_LAYER_OVERHEAD_OCTETS: usize = 28;
/// Session bandwidth used when nothing better is known: a 64 kbit/s audio
/// codec plus 16 kbit/s of IPv4/UDP/RTP headers at 50 packets per second.
pub const DEFAULT_SESSION_BANDWIDTH_BPS: u32 = 80_000;
/// Initial average compound RTCP size: SR with one report block plus an SDES
/// CNAME chunk, plus lower-layer headers.
const INITIAL_AVG_RTCP_SIZE_OCTETS: f64 = 100.0;

/// Nominal session bandwidth (RFC 3550 §6.2) for a static RTP payload type,
/// including IPv4/UDP/RTP header overhead at 20 ms packetisation.
///
/// Returns `None` for dynamic payload types, whose bitrate the payload type
/// alone does not determine.
pub fn session_bandwidth_for_payload_type(payload_type: u8) -> Option<u32> {
    // 40 octets of IPv4 + UDP + RTP headers, 50 packets per second.
    const HEADER_OVERHEAD_BPS: u32 = 16_000;
    let codec_bps = match payload_type {
        0 | 8 => 64_000, // PCMU / PCMA
        9 => 64_000,     // G.722
        3 => 13_200,     // GSM full rate
        4 => 6_300,      // G.723.1
        18 => 8_000,     // G.729
        13 => 0,         // Comfort noise rides alongside a codec
        _ => return None,
    };
    if codec_bps == 0 {
        return None;
    }
    Some(codec_bps + HEADER_OVERHEAD_BPS)
}

/// Inputs to the RFC 3550 §6.3.1 / Appendix A.7 report interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtcpIntervalParams {
    /// Session bandwidth in bits per second (RFC 3550 §6.2). Zero falls
    /// back to [`DEFAULT_SESSION_BANDWIDTH_BPS`].
    pub session_bandwidth_bps: u32,
    /// Session members, including this participant.
    pub members: u32,
    /// Members that sent RTP recently, including this participant when
    /// `we_sent` is set.
    pub senders: u32,
    /// Whether this participant sent RTP in the last two report intervals.
    pub we_sent: bool,
    /// Running average compound RTCP size in octets, lower-layer headers
    /// included.
    pub avg_rtcp_size: f64,
    /// No RTCP has been sent yet: the minimum is halved (RFC 3550 §6.2).
    pub initial: bool,
    /// Use the RFC 3550 §6.2 reduced minimum (360 / session kbit/s) instead
    /// of the fixed five seconds, when it is smaller.
    pub reduced_minimum: bool,
}

impl RtcpIntervalParams {
    fn bandwidth_bps(&self) -> f64 {
        if self.session_bandwidth_bps == 0 {
            f64::from(DEFAULT_SESSION_BANDWIDTH_BPS)
        } else {
            f64::from(self.session_bandwidth_bps)
        }
    }

    /// The minimum interval: five seconds, or the reduced minimum when
    /// enabled and smaller, halved before the first report.
    pub fn minimum(&self) -> Duration {
        let fixed = RTCP_MIN_INTERVAL.as_secs_f64();
        let mut minimum = if self.reduced_minimum {
            let reduced = 360.0 / (self.bandwidth_bps() / 1_000.0);
            reduced.min(fixed)
        } else {
            fixed
        };
        if self.initial {
            minimum /= 2.0;
        }
        Duration::from_secs_f64(minimum)
    }

    /// The deterministic interval `Td` before randomisation.
    pub fn deterministic(&self) -> Duration {
        let mut rtcp_bw = self.bandwidth_bps() * RTCP_BANDWIDTH_FRACTION / 8.0; // octets/s
        let members = self.members.max(1);
        let senders = self.senders.min(members);
        let mut n = f64::from(members);
        if f64::from(senders) <= f64::from(members) * RTCP_SENDER_BANDWIDTH_FRACTION {
            if self.we_sent {
                rtcp_bw *= RTCP_SENDER_BANDWIDTH_FRACTION;
                n = f64::from(senders.max(1));
            } else {
                rtcp_bw *= RTCP_RECEIVER_BANDWIDTH_FRACTION;
                n = f64::from((members - senders).max(1));
            }
        }
        let avg = if self.avg_rtcp_size.is_finite() && self.avg_rtcp_size > 0.0 {
            self.avg_rtcp_size
        } else {
            INITIAL_AVG_RTCP_SIZE_OCTETS
        };
        let t = avg * n / rtcp_bw;
        Duration::from_secs_f64(t.max(self.minimum().as_secs_f64()))
    }

    /// The transmission interval: `Td` scaled by a uniform factor in
    /// [0.5, 1.5] and divided by `e - 3/2` (RFC 3550 Appendix A.7).
    pub fn randomized<R: Rng + ?Sized>(&self, rng: &mut R) -> Duration {
        let factor: f64 = rng.gen_range(0.5..=1.5);
        Duration::from_secs_f64(self.deterministic().as_secs_f64() * factor / RTCP_COMPENSATION)
    }

    /// Smallest interval [`Self::randomized`] can return.
    pub fn lower_bound(&self) -> Duration {
        Duration::from_secs_f64(self.deterministic().as_secs_f64() * 0.5 / RTCP_COMPENSATION)
    }

    /// Largest interval [`Self::randomized`] can return.
    pub fn upper_bound(&self) -> Duration {
        Duration::from_secs_f64(self.deterministic().as_secs_f64() * 1.5 / RTCP_COMPENSATION)
    }
}

/// RTCP report generator
#[derive(Debug)]
pub struct RtcpReportGenerator {
    /// Local SSRC
    local_ssrc: RtpSsrc,

    /// CNAME for SDES reports
    cname: String,

    /// Packet loss statistics by SSRC
    loss_stats: HashMap<RtpSsrc, parking_lot::Mutex<PacketLossTracker>>,

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

    /// Sender packet totals when the previous two reports went out, oldest
    /// first. RFC 3550 §6.4: a participant reports as a sender only if it
    /// sent RTP since the report before the last one.
    packets_sent_at_reports: [u32; 2],

    /// Last RTCP interval
    last_interval: Duration,

    /// Session bandwidth in bits per second
    session_bandwidth: u32,

    /// Number of senders in the session
    senders: u32,

    /// Number of receivers in the session
    receivers: u32,

    /// Running average compound RTCP size, lower-layer headers included.
    avg_rtcp_size: f64,

    /// No report has been sent yet.
    initial: bool,

    /// Use the RFC 3550 §6.2 reduced minimum interval.
    reduced_minimum: bool,

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
            packets_sent_at_reports: [0, 0],
            last_interval: RTCP_MIN_INTERVAL,
            session_bandwidth: DEFAULT_SESSION_BANDWIDTH_BPS,
            senders: 1,
            receivers: 0,
            avg_rtcp_size: INITIAL_AVG_RTCP_SIZE_OCTETS,
            initial: true,
            reduced_minimum: false,
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

    /// Replace the cumulative RTP sender totals used by the next report.
    ///
    /// Session statistics are already cumulative. Using the additive update
    /// API on every periodic tick would count the same packets repeatedly.
    pub fn set_sent_totals(&mut self, packets: u32, octets: u32) {
        self.packets_sent = packets;
        self.octets_sent = octets;
    }

    /// Process a received RTP packet
    pub fn process_received_packet(&mut self, ssrc: RtpSsrc, seq: u16) {
        let tracker = self
            .loss_stats
            .entry(ssrc)
            .or_insert_with(|| parking_lot::Mutex::new(PacketLossTracker::new()));
        tracker.get_mut().process(seq);
    }

    fn take_report_blocks(&self) -> Vec<RtcpReportBlock> {
        self.loss_stats
            .iter()
            .take(31)
            .map(|(ssrc, tracker)| {
                let mut tracker = tracker.lock();
                RtcpReportBlock {
                    ssrc: *ssrc,
                    fraction_lost: tracker.take_interval_fraction_lost(),
                    cumulative_lost: tracker.get_cumulative_lost(),
                    highest_seq: tracker.highest_extended_sequence(),
                    jitter: 0,              // Jitter would be calculated separately
                    last_sr: 0,             // Would be from received SRs
                    delay_since_last_sr: 0, // Would be from received SRs
                }
            })
            .collect()
    }

    /// The SDES CNAME this generator reports.
    pub fn cname(&self) -> &str {
        &self.cname
    }

    /// Session bandwidth in bits per second used for the report interval.
    pub fn bandwidth(&self) -> u32 {
        self.session_bandwidth
    }

    /// Use the RFC 3550 §6.2 reduced minimum interval (360 / session
    /// kbit/s) instead of the fixed five seconds when it is smaller.
    pub fn set_reduced_minimum(&mut self, enabled: bool) {
        self.reduced_minimum = enabled;
    }

    /// Whether the reduced minimum interval is in use.
    pub fn reduced_minimum(&self) -> bool {
        self.reduced_minimum
    }

    /// Whether this participant counts as a sender (RFC 3550 §6.4): it sent
    /// RTP since the report before the previous one.
    pub fn we_sent(&self) -> bool {
        self.packets_sent != self.packets_sent_at_reports[0]
    }

    /// Whether no report has been sent yet.
    pub fn is_initial(&self) -> bool {
        self.initial
    }

    /// Record a compound report of `octets` (RTCP only) that was sent.
    ///
    /// Advances the sender history used by [`Self::we_sent`], clears the
    /// initial-interval state, and folds the size into the running average.
    pub fn on_report_sent(&mut self, octets: usize) {
        self.packets_sent_at_reports = [self.packets_sent_at_reports[1], self.packets_sent];
        self.initial = false;
        self.observe_rtcp_size(octets);
    }

    /// Fold a received compound RTCP size (RTCP only) into the running
    /// average (RFC 3550 §6.3.3).
    pub fn on_report_received(&mut self, octets: usize) {
        self.observe_rtcp_size(octets);
    }

    fn observe_rtcp_size(&mut self, octets: usize) {
        let size = (octets + RTCP_LOWER_LAYER_OVERHEAD_OCTETS) as f64;
        self.avg_rtcp_size = size / 16.0 + self.avg_rtcp_size * 15.0 / 16.0;
    }

    /// Interval inputs for the current session state.
    pub fn interval_params(&self) -> RtcpIntervalParams {
        RtcpIntervalParams {
            session_bandwidth_bps: self.session_bandwidth,
            members: self.senders + self.receivers,
            senders: self.senders,
            we_sent: self.we_sent(),
            avg_rtcp_size: self.avg_rtcp_size,
            initial: self.initial,
            reduced_minimum: self.reduced_minimum,
        }
    }

    /// Calculate the next RTCP interval (RFC 3550 §6.3.1, Appendix A.7):
    /// 5% of the session bandwidth shared by the members, a five-second
    /// minimum (halved before the first report), randomised over
    /// [0.5, 1.5] and divided by `e - 3/2`.
    pub fn calculate_interval(&mut self) -> Duration {
        let interval = self.interval_params().randomized(&mut rand::thread_rng());
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

        // RFC 3550 fraction loss covers the interval since the preceding
        // report, while cumulative loss and highest sequence remain lifetime
        // values.
        let report_blocks = self.take_report_blocks();

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
        let report_blocks = self.take_report_blocks();

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

    use rand::{rngs::StdRng, SeedableRng};

    fn two_party(we_sent: bool, initial: bool) -> RtcpIntervalParams {
        RtcpIntervalParams {
            session_bandwidth_bps: DEFAULT_SESSION_BANDWIDTH_BPS,
            members: 2,
            senders: if we_sent { 2 } else { 1 },
            we_sent,
            avg_rtcp_size: INITIAL_AVG_RTCP_SIZE_OCTETS,
            initial,
            reduced_minimum: false,
        }
    }

    #[test]
    fn rtcp_interval_statistics_follow_rfc3550_appendix_a7() {
        let mut rng = StdRng::seed_from_u64(0x3550);
        for (initial, minimum) in [(true, 2.5_f64), (false, 5.0_f64)] {
            let params = two_party(true, initial);
            // A two-party call needs far less than 5% of 80 kbit/s, so the
            // (possibly halved) five-second minimum is the deterministic Td.
            assert_eq!(params.deterministic(), Duration::from_secs_f64(minimum));
            let lower = minimum * 0.5 / RTCP_COMPENSATION;
            let upper = minimum * 1.5 / RTCP_COMPENSATION;
            let samples: Vec<f64> = (0..20_000)
                .map(|_| params.randomized(&mut rng).as_secs_f64())
                .collect();
            let mean = samples.iter().sum::<f64>() / samples.len() as f64;
            let expected_mean = minimum / RTCP_COMPENSATION;
            assert!(
                (mean - expected_mean).abs() < expected_mean * 0.01,
                "mean {mean} expected {expected_mean}"
            );
            let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = samples.iter().cloned().fold(0.0, f64::max);
            assert!(min >= lower - 1e-9 && max <= upper + 1e-9, "{min}..{max}");
            // Randomised over the whole [0.5, 1.5] span, not a fixed value.
            assert!(min < lower * 1.01 && max > upper * 0.99, "{min}..{max}");
            assert_eq!(params.lower_bound(), Duration::from_secs_f64(lower));
            assert_eq!(params.upper_bound(), Duration::from_secs_f64(upper));
        }
    }

    #[test]
    fn rtcp_interval_scales_with_members_and_bandwidth_share() {
        // 1000 receivers and one sender at 80 kbit/s: RTCP gets 500 octets/s,
        // receivers 75% of it, so Td = 100 * 999 / 375 seconds.
        let receivers = RtcpIntervalParams {
            members: 1_000,
            senders: 1,
            we_sent: false,
            ..two_party(false, false)
        };
        let expected = 100.0 * 999.0 / 375.0;
        assert!((receivers.deterministic().as_secs_f64() - expected).abs() < 1e-9);
        // The one sender shares 25% among senders only.
        let sender = RtcpIntervalParams {
            we_sent: true,
            ..receivers
        };
        assert_eq!(sender.deterministic(), Duration::from_secs(5));
    }

    #[test]
    fn reduced_minimum_is_360_over_session_kbps_and_never_raises_the_floor() {
        let mut params = two_party(true, false);
        params.reduced_minimum = true;
        // 80 kbit/s -> 4.5 s.
        assert_eq!(params.minimum(), Duration::from_secs_f64(4.5));
        params.session_bandwidth_bps = 1_000_000;
        assert_eq!(params.minimum(), Duration::from_secs_f64(0.36));
        params.initial = true;
        assert_eq!(params.minimum(), Duration::from_secs_f64(0.18));
        // Below 72 kbit/s the formula exceeds five seconds; keep five.
        params.initial = false;
        params.session_bandwidth_bps = 24_000;
        assert_eq!(params.minimum(), RTCP_MIN_INTERVAL);
        params.reduced_minimum = false;
        params.session_bandwidth_bps = 1_000_000;
        assert_eq!(params.minimum(), RTCP_MIN_INTERVAL);
    }

    #[test]
    fn sender_state_covers_the_last_two_report_intervals() {
        let mut generator = RtcpReportGenerator::new(1, "c".to_string());
        assert!(!generator.we_sent());
        assert!(generator.is_initial());
        generator.set_sent_totals(10, 1_600);
        assert!(generator.we_sent());
        generator.on_report_sent(80);
        assert!(!generator.is_initial());
        // Sent during the interval before the previous report: still a sender.
        assert!(generator.we_sent());
        generator.on_report_sent(80);
        // Nothing sent across two whole intervals: a receiver again.
        assert!(!generator.we_sent());
        generator.set_sent_totals(11, 1_760);
        assert!(generator.we_sent());
    }

    #[test]
    fn calculated_interval_respects_the_initial_half_minimum() {
        let mut generator = RtcpReportGenerator::new(1, "c".to_string());
        generator.update_members(1, 1);
        for _ in 0..1_000 {
            let interval = generator.calculate_interval();
            assert!(interval >= Duration::from_secs_f64(2.5 * 0.5 / RTCP_COMPENSATION));
            assert!(interval <= Duration::from_secs_f64(2.5 * 1.5 / RTCP_COMPENSATION));
        }
        generator.on_report_sent(80);
        for _ in 0..1_000 {
            let interval = generator.calculate_interval();
            assert!(interval >= Duration::from_secs_f64(5.0 * 0.5 / RTCP_COMPENSATION));
        }
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
    fn sender_and_receiver_reports_use_interval_fraction_loss() {
        let remote_ssrc = 0xabcdef01;

        let mut sender_generator =
            RtcpReportGenerator::new(0x12345678, "sender@example.com".to_string());
        sender_generator.process_received_packet(remote_ssrc, 10);
        sender_generator.process_received_packet(remote_ssrc, 12);
        let first_sender = sender_generator.generate_sender_report(160);
        assert_eq!(first_sender.report_blocks[0].fraction_lost, 85);
        assert_eq!(first_sender.report_blocks[0].cumulative_lost, 1);

        for sequence in 13..=20 {
            sender_generator.process_received_packet(remote_ssrc, sequence);
        }
        let second_sender = sender_generator.generate_sender_report(320);
        assert_eq!(second_sender.report_blocks[0].fraction_lost, 0);
        assert_eq!(second_sender.report_blocks[0].cumulative_lost, 1);

        let mut receiver_generator =
            RtcpReportGenerator::new(0x87654321, "receiver@example.com".to_string());
        receiver_generator.process_received_packet(remote_ssrc, 10);
        receiver_generator.process_received_packet(remote_ssrc, 12);
        let first_receiver = receiver_generator.generate_receiver_report();
        assert_eq!(first_receiver.report_blocks[0].fraction_lost, 85);
        assert_eq!(first_receiver.report_blocks[0].cumulative_lost, 1);

        for sequence in 13..=20 {
            receiver_generator.process_received_packet(remote_ssrc, sequence);
        }
        let second_receiver = receiver_generator.generate_receiver_report();
        assert_eq!(second_receiver.report_blocks[0].fraction_lost, 0);
        assert_eq!(second_receiver.report_blocks[0].cumulative_lost, 1);
    }

    #[test]
    fn setting_cumulative_sender_totals_does_not_double_count() {
        let mut generator = RtcpReportGenerator::new(0x12345678, "test".to_string());

        generator.set_sent_totals(3, 30);
        let first = generator.generate_sender_report(100);
        generator.set_sent_totals(3, 30);
        let second = generator.generate_sender_report(200);

        assert_eq!(first.sender_packet_count, 3);
        assert_eq!(first.sender_octet_count, 30);
        assert_eq!(second.sender_packet_count, 3);
        assert_eq!(second.sender_octet_count, 30);
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
