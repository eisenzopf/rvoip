//! Passive RTP/RTCP quality observations for endpoints and media relays.
//!
//! The types here inspect packets only; they never delay, rewrite, or enqueue
//! media. This makes them suitable for a real-time forwarding path. All
//! counters exposed by RTCP remain protocol values. Application reporting
//! windows and telemetry are intentionally outside this module.
//!
//! References:
//! - RTP/RTCP fields and interarrival jitter: <https://www.rfc-editor.org/rfc/rfc3550#section-6.4.1>
//! - RTP payload clock rates: <https://www.rfc-editor.org/rfc/rfc3551>

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use crate::{
    stats::{JitterEstimator, PacketLossResult, PacketLossTracker},
    RtcpPacket, RtcpPacketItem, RtcpPacketIter, RtpHeader,
};

/// Number of fixed-point units per second in the RTCP DLSR field.
pub const RTCP_DELAY_UNITS_PER_SECOND: f64 = 65_536.0;
/// Number of fixed-point units representing 100% loss in an RTCP RR.
pub const RTCP_LOSS_FRACTION_UNITS: f64 = 256.0;

const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const DEFAULT_UNMATCHED_SENDER_REPORT_TTL: Duration = Duration::from_secs(120);
const DEFAULT_MAX_TRACKED_RTP_STREAMS: usize = 32;
const MAX_PENDING_SENDER_REPORTS: usize = 64;

/// Converts RFC 3550's 8-bit fixed-point loss fraction into percent.
pub fn fraction_lost_percent(fraction_lost: u8) -> f64 {
    f64::from(fraction_lost) * 100.0 / RTCP_LOSS_FRACTION_UNITS
}

/// Converts RTCP jitter from RTP timestamp units into milliseconds.
///
/// `None` is returned for an invalid zero clock rate rather than manufacturing
/// a measurement. For G.722 callers must pass its 8 kHz RTP clock, as required
/// by RFC 3551 section 4.5.2, not its 16 kHz audio sample rate.
pub fn jitter_timestamp_units_to_ms(jitter: u32, clock_rate_hz: u32) -> Option<f64> {
    (clock_rate_hz != 0)
        .then(|| f64::from(jitter) * MILLISECONDS_PER_SECOND / f64::from(clock_rate_hz))
}

/// Computes RTT from elapsed SR-to-RR time after removing the peer's DLSR.
///
/// A negative result can occur because of coarse clocks or malformed input and
/// is clamped to zero; propagating it would make a later quality score better
/// than a zero-delay call.
pub fn round_trip_ms(elapsed: Duration, delay_since_last_sr: u32) -> f64 {
    (elapsed.as_secs_f64() - f64::from(delay_since_last_sr) / RTCP_DELAY_UNITS_PER_SECOND).max(0.0)
        * MILLISECONDS_PER_SECOND
}

/// Loss and jitter inferred while passively observing one RTP packet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtpObservation {
    pub inferred_lost: u64,
    pub jitter_seconds: Option<f64>,
}

#[derive(Debug)]
struct RtpStreamState {
    loss: PacketLossTracker,
    jitter_clock_rate: Option<u32>,
    jitter: Option<JitterEstimator>,
}

/// Per-SSRC packet-loss and jitter tracker for a passive RTP observer.
#[derive(Debug)]
pub struct RtpQualityTracker {
    clock_rates: HashMap<u8, u32>,
    streams: HashMap<u32, RtpStreamState>,
    stream_order: VecDeque<u32>,
    stream_limit: usize,
}

impl RtpQualityTracker {
    /// Creates a tracker with RTP clock rates indexed by payload type.
    pub fn new(clock_rates: HashMap<u8, u32>) -> Self {
        Self::with_stream_limit(clock_rates, DEFAULT_MAX_TRACKED_RTP_STREAMS)
    }

    /// Creates a tracker with an explicit SSRC bound.
    ///
    /// The oldest SSRC is forgotten when the bound is reached. A packet that
    /// later reuses an evicted SSRC starts a fresh loss baseline, which is
    /// safer than deriving a large, false loss value across unrelated streams.
    pub fn with_stream_limit(clock_rates: HashMap<u8, u32>, stream_limit: usize) -> Self {
        Self {
            clock_rates,
            streams: HashMap::new(),
            stream_order: VecDeque::new(),
            stream_limit: stream_limit.max(1),
        }
    }

    /// Parses and records one RTP packet without modifying it.
    pub fn observe(&mut self, packet: &[u8]) -> Option<RtpObservation> {
        let (header, _) = RtpHeader::parse_without_consuming(packet).ok()?;
        let clock_rate = self.clock_rates.get(&header.payload_type).copied();
        if !self.streams.contains_key(&header.ssrc) {
            if self.streams.len() == self.stream_limit {
                if let Some(oldest) = self.stream_order.pop_front() {
                    self.streams.remove(&oldest);
                }
            }
            self.stream_order.push_back(header.ssrc);
        }
        let state = self
            .streams
            .entry(header.ssrc)
            .or_insert_with(new_rtp_stream_state);

        let inferred_lost = match state.loss.process(header.sequence_number) {
            PacketLossResult::Gap { lost, .. } => u64::from(lost),
            _ => 0,
        };
        let jitter_seconds = clock_rate.map(|rate| observe_jitter(state, rate, &header));

        Some(RtpObservation {
            inferred_lost,
            jitter_seconds,
        })
    }
}

fn new_rtp_stream_state() -> RtpStreamState {
    RtpStreamState {
        loss: PacketLossTracker::new(),
        jitter_clock_rate: None,
        jitter: None,
    }
}

fn observe_jitter(state: &mut RtpStreamState, clock_rate: u32, header: &RtpHeader) -> f64 {
    if state.jitter_clock_rate != Some(clock_rate) {
        // Jitter is expressed in payload timestamp units. A clock-rate change
        // invalidates the previous estimator instead of mixing two units.
        state.jitter_clock_rate = Some(clock_rate);
        state.jitter = None;
    }
    state
        .jitter
        .get_or_insert_with(|| JitterEstimator::new(clock_rate))
        .update(header.timestamp, Instant::now())
}

/// One RTCP receiver-report block, including RTT when its SR was observed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtcpReceiverObservation {
    pub ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: u32,
    pub highest_seq: u32,
    pub jitter: u32,
    pub rtt_seconds: Option<f64>,
}

/// Correlates SR and RR packets seen by a passive RTP relay.
///
/// Unlike [`crate::stats::RttEstimator`], this observer did not originate the
/// sender report. Matching entries are consumed and unmatched entries expire,
/// so a silent or malicious peer cannot grow memory without bound.
#[derive(Debug)]
pub struct RtcpRelayObserver {
    sender_reports: Vec<PendingSenderReport>,
    sender_report_ttl: Duration,
}

#[derive(Debug)]
struct PendingSenderReport {
    ssrc: u32,
    compact_ntp: u32,
    observed_at: Instant,
}

impl Default for RtcpRelayObserver {
    fn default() -> Self {
        Self::new(DEFAULT_UNMATCHED_SENDER_REPORT_TTL)
    }
}

impl RtcpRelayObserver {
    pub fn new(sender_report_ttl: Duration) -> Self {
        Self {
            sender_reports: Vec::with_capacity(MAX_PENDING_SENDER_REPORTS),
            sender_report_ttl,
        }
    }

    /// Observes RTCP at the current monotonic time.
    pub fn observe(&mut self, packet: &[u8]) -> Vec<RtcpReceiverObservation> {
        self.observe_at(packet, Instant::now())
    }

    /// Appends observations to caller-owned storage, avoiding one allocation
    /// per packet when a reactor already owns a reusable batch.
    pub fn observe_into(&mut self, packet: &[u8], observations: &mut Vec<RtcpReceiverObservation>) {
        self.observe_each_at(packet, Instant::now(), |value| observations.push(value));
    }

    /// Deterministic-time variant intended for tests and virtual clocks.
    pub fn observe_at(&mut self, packet: &[u8], now: Instant) -> Vec<RtcpReceiverObservation> {
        let mut observations = Vec::new();
        self.observe_each_at(packet, now, |value| observations.push(value));
        observations
    }

    fn observe_each_at(
        &mut self,
        packet: &[u8],
        now: Instant,
        mut emit: impl FnMut(RtcpReceiverObservation),
    ) {
        self.expire_sender_reports(now);
        for item in RtcpPacketIter::new(packet) {
            let Ok(RtcpPacketItem::Known(packet)) = item else {
                continue;
            };
            self.observe_rtcp_packet(packet, now, &mut emit);
        }
    }

    fn expire_sender_reports(&mut self, now: Instant) {
        self.sender_reports.retain(|report| {
            now.saturating_duration_since(report.observed_at) <= self.sender_report_ttl
        });
    }

    fn observe_rtcp_packet(
        &mut self,
        packet: RtcpPacket,
        now: Instant,
        emit: &mut impl FnMut(RtcpReceiverObservation),
    ) {
        match packet {
            RtcpPacket::SenderReport(report) => {
                self.remember_sender_report(report.ssrc, report.ntp_timestamp.to_u32(), now);
            }
            RtcpPacket::ReceiverReport(report) => {
                for block in report.report_blocks {
                    emit(self.receiver_observation(block, now));
                }
            }
            _ => {}
        }
    }

    fn remember_sender_report(&mut self, ssrc: u32, compact_ntp: u32, now: Instant) {
        if let Some(existing) = self
            .sender_reports
            .iter_mut()
            .find(|report| report.ssrc == ssrc && report.compact_ntp == compact_ntp)
        {
            existing.observed_at = now;
            return;
        }
        if self.sender_reports.len() == MAX_PENDING_SENDER_REPORTS {
            self.sender_reports.remove(0);
        }
        self.sender_reports.push(PendingSenderReport {
            ssrc,
            compact_ntp,
            observed_at: now,
        });
    }

    fn receiver_observation(
        &mut self,
        block: crate::RtcpReportBlock,
        now: Instant,
    ) -> RtcpReceiverObservation {
        let match_index = self
            .sender_reports
            .iter()
            .position(|report| report.ssrc == block.ssrc && report.compact_ntp == block.last_sr);
        let rtt_seconds = match_index.map(|index| {
            let sent_at = self.sender_reports.swap_remove(index).observed_at;
            round_trip_ms(
                now.saturating_duration_since(sent_at),
                block.delay_since_last_sr,
            ) / MILLISECONDS_PER_SECOND
        });

        RtcpReceiverObservation {
            ssrc: block.ssrc,
            fraction_lost: block.fraction_lost,
            cumulative_lost: block.cumulative_lost,
            highest_seq: block.highest_seq,
            jitter: block.jitter,
            rtt_seconds,
        }
    }
}

/// Non-negative loss/sequence deltas between consecutive reports for an SSRC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverReportDelta {
    pub packets_lost: u64,
    pub packets_expected: u64,
}

#[derive(Debug, Clone, Copy)]
struct ReceiverReportBaseline {
    cumulative_lost: i32,
    highest_seq: u32,
}

/// Converts one source's cumulative RTCP RR fields into reporting-window deltas.
///
/// The caller selects the report-block SSRC it owns. Keeping one baseline
/// makes memory constant and prevents unrelated or malicious SSRCs in a
/// compound RR from growing an application-level map.
#[derive(Debug, Default)]
pub struct ReceiverReportDeltaTracker {
    baseline: Option<ReceiverReportBaseline>,
}

impl ReceiverReportDeltaTracker {
    /// Stores the current report and returns its delta from the prior report.
    /// A counter reset/restart establishes a new baseline and returns `None`.
    pub fn observe(
        &mut self,
        cumulative_lost: u32,
        highest_seq: u32,
    ) -> Option<ReceiverReportDelta> {
        let cumulative_lost = signed_24_bit(cumulative_lost);
        let current = ReceiverReportBaseline {
            cumulative_lost,
            highest_seq,
        };
        let previous = self.baseline.replace(current)?;
        if cumulative_lost < previous.cumulative_lost || highest_seq < previous.highest_seq {
            return None;
        }
        Some(ReceiverReportDelta {
            packets_lost: (cumulative_lost - previous.cumulative_lost) as u64,
            packets_expected: u64::from(highest_seq - previous.highest_seq),
        })
    }
}

/// Sign-extends RFC 3550's signed 24-bit cumulative-loss field.
pub fn signed_24_bit(value: u32) -> i32 {
    let wire = value & 0x00ff_ffff;
    if wire & 0x0080_0000 == 0 {
        wire as i32
    } else {
        (wire | 0xff00_0000) as i32
    }
}
