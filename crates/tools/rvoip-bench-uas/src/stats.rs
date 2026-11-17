//! Counters, the per-interval snapshot and its two renderings: one
//! `stats.jsonl` line and the Prometheus text exposition.
//!
//! Every gauge comes from state that is decremented on every terminal path:
//! the call table in [`crate::calls`] (one entry per call, removed exactly
//! once) or a structure rvoip owns (its dialog stores, its RTP port
//! reservations). Nothing here is an estimate from rates.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use serde::Serialize;

/// Prefix of every Prometheus metric.
pub const METRIC_PREFIX: &str = "rvoip_bench_uas_";

/// Counters the DUT maintains itself. Monotonic totals are `AtomicU64`;
/// gauges are `AtomicI64` so a transient decrement-before-increment race
/// cannot wrap (they are clamped at zero when read).
#[derive(Debug, Default)]
pub struct Counters {
    /// Inbound calls rvoip answered with 200 OK (each `IncomingCall` under
    /// fast auto-accept).
    pub incoming_calls: AtomicU64,
    /// Calls whose ACK arrived (`CallEstablished`).
    pub calls_established: AtomicU64,
    /// Calls the caller ended with BYE (answered 200 by rvoip's dialog
    /// layer before the event is published).
    pub byes_received: AtomicU64,
    /// Calls that ended any other way after being answered (ACK timeout,
    /// transport loss, local teardown).
    pub calls_ended_other: AtomicU64,
    /// Calls that failed before establishment (`CallFailed`).
    pub calls_failed: AtomicU64,
    /// Calls cancelled before answer (`CallCancelled`).
    pub calls_cancelled: AtomicU64,
    /// Terminal events whose call never produced `IncomingCall` within the
    /// tombstone horizon.
    pub unmatched_terminal_events: AtomicU64,
    /// Live calls answered but not yet ACKed (gauge).
    pub pending_setups: AtomicI64,
    /// Live established calls (gauge).
    pub active_calls: AtomicI64,
    /// Echo media streams bound and not yet closed (gauge).
    pub echo_streams_open: AtomicI64,
    /// Echo media streams of ended calls that are still closing (gauge).
    pub echo_streams_closing: AtomicI64,
    /// Echo streams that could not be bound for a live call.
    pub echo_bind_failures: AtomicU64,
    /// Inbound RTP media frames rvoip handed to the echo.
    pub rtp_frames_received: AtomicU64,
    /// Frames the echo handed back to rvoip that rvoip accepted for sending.
    pub rtp_frames_sent: AtomicU64,
    /// Of those, frames sent on the media lane's synchronous path.
    pub rtp_frames_sent_inline: AtomicU64,
    /// Frames the echo could not send back.
    pub echo_frames_dropped: AtomicU64,
}

impl Counters {
    /// Clamp a gauge at zero.
    pub fn gauge(value: &AtomicI64) -> u64 {
        u64::try_from(value.load(Ordering::Relaxed)).unwrap_or(0)
    }

    fn total(value: &AtomicU64) -> u64 {
        value.load(Ordering::Relaxed)
    }
}

/// What rvoip and the operating system report, gathered once per snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct External {
    /// New-call gate: INVITEs rejected (rate limit or drain).
    pub gate_rejected: u64,
    /// New-call gate: 503s that could not be written within their budget.
    pub gate_reject_sends_dropped: u64,
    /// Transaction layer: INVITEs rejected at `max_concurrent_calls`.
    pub capacity_rejections: u64,
    /// Transaction layer: INVITEs rejected while the SIP stack drained.
    pub draining_rejections: u64,
    /// Session layer: INVITEs shed because no RTP port was free.
    pub media_port_rejections: u64,
    /// Session layer: INVITEs answered 503 after their setup failed.
    pub setup_failure_rejections: u64,
    /// Of those, 503s that never reached the wire.
    pub setup_failure_unanswered: u64,
    /// Dialogs held by rvoip's dialog stores right now.
    pub live_dialogs: u64,
    /// RTP port reservations held by rvoip right now.
    pub media_ports: u64,
    /// Inbound SRTP packets rvoip dropped for failing authentication.
    pub srtp_unprotect_failures: u64,
    /// Established TCP connections on the SIP TLS listen port.
    pub tls_connections: u64,
    /// Distinct caller addresses with at least one live call.
    pub udp_flows: u64,
    /// Process CPU time (user + system), seconds.
    pub process_cpu_seconds: f64,
    /// Process resident set size, bytes.
    pub process_resident_memory_bytes: u64,
    /// Process thread count.
    pub process_threads: u64,
}

/// One point-in-time view of every metric. Serialized as a `stats.jsonl`
/// line (plus `"t"`) and as `summary.json`'s `counters`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Snapshot {
    /// INVITEs that reached a decision: answered, or rejected with 503.
    pub invites_received_total: u64,
    /// Final responses to INVITEs, by status code.
    pub responses_sent_total: BTreeMap<String, u64>,
    /// Calls whose ACK arrived.
    pub calls_established_total: u64,
    /// BYEs received for live calls.
    pub byes_received_total: u64,
    /// Responses to those BYEs, by status code.
    pub byes_answered_total: BTreeMap<String, u64>,
    /// Established calls not yet ended.
    pub active_calls: u64,
    /// Answered calls whose ACK has not arrived.
    pub pending_setups: u64,
    /// Resources still held for calls that already ended (see README).
    pub pending_cleanups: u64,
    /// Dialogs rvoip holds.
    pub open_dialogs: u64,
    /// Established TLS connections on the SIP port (TLS only).
    pub connections: u64,
    /// Distinct UDP caller addresses with a live call (UDP only).
    pub udp_flows: u64,
    /// INVITEs shed by admission (max-cps, drain, or max_concurrent_calls).
    pub admission_rejected_total: u64,
    /// RTP frames received for echo.
    pub rtp_packets_received_total: u64,
    /// RTP frames sent back.
    pub rtp_packets_sent_total: u64,
    /// SRTP packets that failed authentication.
    pub srtp_unprotect_failures_total: u64,
    /// Process CPU seconds.
    pub process_cpu_seconds_total: f64,
    /// Process RSS bytes.
    pub process_resident_memory_bytes: u64,
    /// Process threads.
    pub process_threads: u64,
    /// RTP port reservations rvoip holds (extension).
    pub media_ports: u64,
    /// Calls that failed before establishment (extension).
    pub calls_failed_total: u64,
    /// Calls cancelled before answer (extension).
    pub calls_cancelled_total: u64,
    /// Answered calls that ended without a BYE from the caller (extension).
    pub calls_ended_other_total: u64,
    /// Echoed frames sent inline on the media lane (extension).
    pub rtp_packets_sent_inline_total: u64,
    /// Echo frames that could not be sent back (extension).
    pub echo_frames_dropped_total: u64,
    /// Echo streams that failed to bind for a live call (extension).
    pub echo_bind_failures_total: u64,
    /// Whether the DUT is draining (extension).
    pub draining: bool,
}

impl Snapshot {
    /// Combine the DUT's own counters with what rvoip and the OS report.
    pub fn compute(counters: &Counters, external: &External, draining: bool) -> Self {
        let answered = Counters::total(&counters.incoming_calls);
        let active = Counters::gauge(&counters.active_calls);
        let pending_setups = Counters::gauge(&counters.pending_setups);
        let live_calls = active + pending_setups;
        let rejected_503 = (external.gate_rejected
            + external.capacity_rejections
            + external.draining_rejections
            + external.media_port_rejections
            + external.setup_failure_rejections)
            .saturating_sub(external.gate_reject_sends_dropped + external.setup_failure_unanswered);
        let decided = answered
            + external.gate_rejected
            + external.capacity_rejections
            + external.draining_rejections
            + external.media_port_rejections
            + external.setup_failure_rejections;
        let byes = Counters::total(&counters.byes_received);

        let mut responses = BTreeMap::new();
        responses.insert("200".to_string(), answered);
        responses.insert("503".to_string(), rejected_503);
        let mut bye_responses = BTreeMap::new();
        bye_responses.insert("200".to_string(), byes);

        Self {
            invites_received_total: decided,
            responses_sent_total: responses,
            calls_established_total: Counters::total(&counters.calls_established),
            byes_received_total: byes,
            byes_answered_total: bye_responses,
            active_calls: active,
            pending_setups,
            pending_cleanups: external.live_dialogs.saturating_sub(live_calls)
                + Counters::gauge(&counters.echo_streams_closing),
            open_dialogs: external.live_dialogs,
            connections: external.tls_connections,
            udp_flows: external.udp_flows,
            admission_rejected_total: external.gate_rejected
                + external.capacity_rejections
                + external.draining_rejections,
            rtp_packets_received_total: Counters::total(&counters.rtp_frames_received),
            rtp_packets_sent_total: Counters::total(&counters.rtp_frames_sent),
            srtp_unprotect_failures_total: external.srtp_unprotect_failures,
            process_cpu_seconds_total: external.process_cpu_seconds,
            process_resident_memory_bytes: external.process_resident_memory_bytes,
            process_threads: external.process_threads,
            media_ports: external.media_ports,
            calls_failed_total: Counters::total(&counters.calls_failed),
            calls_cancelled_total: Counters::total(&counters.calls_cancelled),
            calls_ended_other_total: Counters::total(&counters.calls_ended_other),
            rtp_packets_sent_inline_total: Counters::total(&counters.rtp_frames_sent_inline),
            echo_frames_dropped_total: Counters::total(&counters.echo_frames_dropped),
            echo_bind_failures_total: Counters::total(&counters.echo_bind_failures),
            draining,
        }
    }

    /// Every call and every per-call resource is gone: no active call, no
    /// pending setup or cleanup, no dialog and no RTP port reservation.
    pub fn is_true_zero(&self) -> bool {
        self.active_calls == 0
            && self.pending_setups == 0
            && self.pending_cleanups == 0
            && self.open_dialogs == 0
            && self.media_ports == 0
    }

    /// One `stats.jsonl` line (no trailing newline) stamped with `t`, unix
    /// seconds.
    pub fn json_line(&self, t: f64) -> String {
        let mut value = serde_json::to_value(self).expect("snapshot serializes");
        if let serde_json::Value::Object(map) = &mut value {
            map.insert("t".to_string(), serde_json::json!(t));
        }
        value.to_string()
    }

    /// The Prometheus text exposition (format 0.0.4).
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4096);
        let mut metric = |name: &str, kind: &str, help: &str, samples: &[(String, String)]| {
            let _ = writeln!(out, "# HELP {METRIC_PREFIX}{name} {help}");
            let _ = writeln!(out, "# TYPE {METRIC_PREFIX}{name} {kind}");
            for (labels, value) in samples {
                let _ = writeln!(out, "{METRIC_PREFIX}{name}{labels} {value}");
            }
        };
        let plain = |value: u64| vec![(String::new(), value.to_string())];
        let by_code = |codes: &BTreeMap<String, u64>| -> Vec<(String, String)> {
            codes
                .iter()
                .map(|(code, value)| (format!("{{code=\"{code}\"}}"), value.to_string()))
                .collect()
        };
        let responses = by_code(&self.responses_sent_total);
        let bye_responses = by_code(&self.byes_answered_total);

        metric(
            "invites_received_total",
            "counter",
            "Initial INVITEs answered or rejected.",
            &plain(self.invites_received_total),
        );
        metric(
            "responses_sent_total",
            "counter",
            "Final responses to initial INVITEs by status code.",
            &responses,
        );
        metric(
            "calls_established_total",
            "counter",
            "Calls whose ACK arrived.",
            &plain(self.calls_established_total),
        );
        metric(
            "byes_received_total",
            "counter",
            "BYEs received for live calls.",
            &plain(self.byes_received_total),
        );
        metric(
            "byes_answered_total",
            "counter",
            "Responses to received BYEs by status code.",
            &bye_responses,
        );
        metric(
            "active_calls",
            "gauge",
            "Established calls not yet ended.",
            &plain(self.active_calls),
        );
        metric(
            "pending_setups",
            "gauge",
            "Answered calls waiting for their ACK.",
            &plain(self.pending_setups),
        );
        metric(
            "pending_cleanups",
            "gauge",
            "Resources still held for calls that already ended.",
            &plain(self.pending_cleanups),
        );
        metric(
            "open_dialogs",
            "gauge",
            "Dialogs held by the SIP stack.",
            &plain(self.open_dialogs),
        );
        metric(
            "connections",
            "gauge",
            "Established TLS connections on the SIP port.",
            &plain(self.connections),
        );
        metric(
            "udp_flows",
            "gauge",
            "Distinct UDP caller addresses with a live call.",
            &plain(self.udp_flows),
        );
        metric(
            "admission_rejected_total",
            "counter",
            "INVITEs shed with 503 by admission control.",
            &plain(self.admission_rejected_total),
        );
        metric(
            "rtp_packets_received_total",
            "counter",
            "RTP media packets received for echo.",
            &plain(self.rtp_packets_received_total),
        );
        metric(
            "rtp_packets_sent_total",
            "counter",
            "RTP media packets echoed back.",
            &plain(self.rtp_packets_sent_total),
        );
        metric(
            "srtp_unprotect_failures_total",
            "counter",
            "Inbound SRTP packets that failed authentication.",
            &plain(self.srtp_unprotect_failures_total),
        );
        metric(
            "media_ports",
            "gauge",
            "RTP port reservations held.",
            &plain(self.media_ports),
        );
        metric(
            "calls_failed_total",
            "counter",
            "Calls that failed before establishment.",
            &plain(self.calls_failed_total),
        );
        metric(
            "calls_cancelled_total",
            "counter",
            "Calls cancelled before answer.",
            &plain(self.calls_cancelled_total),
        );
        metric(
            "calls_ended_other_total",
            "counter",
            "Answered calls that ended without a BYE from the caller.",
            &plain(self.calls_ended_other_total),
        );
        metric(
            "rtp_packets_sent_inline_total",
            "counter",
            "Echoed RTP packets sent synchronously on the media lane.",
            &plain(self.rtp_packets_sent_inline_total),
        );
        metric(
            "echo_frames_dropped_total",
            "counter",
            "Echo frames that could not be sent back.",
            &plain(self.echo_frames_dropped_total),
        );
        metric(
            "echo_bind_failures_total",
            "counter",
            "Echo streams that failed to bind for a live call.",
            &plain(self.echo_bind_failures_total),
        );
        metric(
            "draining",
            "gauge",
            "1 while the DUT drains after the finish file appeared.",
            &plain(u64::from(self.draining)),
        );
        metric(
            "process_cpu_seconds_total",
            "counter",
            "User and system CPU time of the process, seconds.",
            &[(
                String::new(),
                format!("{:.3}", self.process_cpu_seconds_total),
            )],
        );
        metric(
            "process_resident_memory_bytes",
            "gauge",
            "Resident set size of the process, bytes.",
            &plain(self.process_resident_memory_bytes),
        );
        metric(
            "process_threads",
            "gauge",
            "Threads of the process.",
            &plain(self.process_threads),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters() -> Counters {
        let counters = Counters::default();
        counters.incoming_calls.store(100, Ordering::Relaxed);
        counters.calls_established.store(95, Ordering::Relaxed);
        counters.byes_received.store(80, Ordering::Relaxed);
        counters.active_calls.store(15, Ordering::Relaxed);
        counters.pending_setups.store(5, Ordering::Relaxed);
        counters.echo_streams_closing.store(2, Ordering::Relaxed);
        counters.rtp_frames_received.store(1_000, Ordering::Relaxed);
        counters.rtp_frames_sent.store(990, Ordering::Relaxed);
        counters
    }

    fn external() -> External {
        External {
            gate_rejected: 7,
            gate_reject_sends_dropped: 1,
            capacity_rejections: 2,
            draining_rejections: 0,
            media_port_rejections: 1,
            setup_failure_rejections: 1,
            setup_failure_unanswered: 0,
            live_dialogs: 23,
            media_ports: 20,
            srtp_unprotect_failures: 3,
            tls_connections: 32,
            udp_flows: 0,
            process_cpu_seconds: 12.5,
            process_resident_memory_bytes: 1 << 20,
            process_threads: 24,
        }
    }

    #[test]
    fn snapshot_combines_own_counters_and_rvoip_state() {
        let snapshot = Snapshot::compute(&counters(), &external(), false);
        // 100 answered + 7 gate + 2 capacity + 1 port + 1 setup failure.
        assert_eq!(snapshot.invites_received_total, 111);
        assert_eq!(snapshot.responses_sent_total["200"], 100);
        // 11 rejections minus the one 503 that never left.
        assert_eq!(snapshot.responses_sent_total["503"], 10);
        assert_eq!(snapshot.admission_rejected_total, 9);
        assert_eq!(snapshot.active_calls, 15);
        assert_eq!(snapshot.pending_setups, 5);
        // 23 dialogs for 20 live calls: 3 still being cleaned up, plus 2
        // closing echo streams.
        assert_eq!(snapshot.pending_cleanups, 5);
        assert_eq!(snapshot.open_dialogs, 23);
        assert_eq!(snapshot.byes_answered_total["200"], 80);
        assert!(!snapshot.is_true_zero());
    }

    #[test]
    fn gauges_never_wrap_below_zero() {
        let counters = Counters::default();
        counters.active_calls.store(-1, Ordering::Relaxed);
        let snapshot = Snapshot::compute(&counters, &External::default(), true);
        assert_eq!(snapshot.active_calls, 0);
        assert!(snapshot.is_true_zero());
        assert!(snapshot.draining);
    }

    #[test]
    fn true_zero_needs_every_gauge_and_resource_at_zero() {
        let quiet = Snapshot::default();
        assert!(quiet.is_true_zero());
        for broken in [
            Snapshot {
                active_calls: 1,
                ..Snapshot::default()
            },
            Snapshot {
                pending_setups: 1,
                ..Snapshot::default()
            },
            Snapshot {
                pending_cleanups: 1,
                ..Snapshot::default()
            },
            Snapshot {
                open_dialogs: 1,
                ..Snapshot::default()
            },
            Snapshot {
                media_ports: 1,
                ..Snapshot::default()
            },
        ] {
            assert!(!broken.is_true_zero(), "{broken:?}");
        }
    }

    #[test]
    fn json_line_carries_t_and_every_counter() {
        let snapshot = Snapshot::compute(&counters(), &external(), false);
        let line = snapshot.json_line(1_700_000_000.25);
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["t"], 1_700_000_000.25);
        assert_eq!(value["invites_received_total"], 111);
        assert_eq!(value["responses_sent_total"]["503"], 10);
        assert_eq!(value["rtp_packets_sent_total"], 990);
        assert_eq!(value["srtp_unprotect_failures_total"], 3);
        assert_eq!(value["connections"], 32);
        assert_eq!(value["draining"], false);
    }

    #[test]
    fn prometheus_exposition_names_every_contract_metric() {
        let text = Snapshot::compute(&counters(), &external(), false).render_prometheus();
        for name in [
            "invites_received_total",
            "responses_sent_total",
            "calls_established_total",
            "byes_received_total",
            "byes_answered_total",
            "active_calls",
            "pending_setups",
            "pending_cleanups",
            "open_dialogs",
            "connections",
            "udp_flows",
            "admission_rejected_total",
            "rtp_packets_received_total",
            "rtp_packets_sent_total",
            "srtp_unprotect_failures_total",
            "process_cpu_seconds_total",
            "process_resident_memory_bytes",
            "process_threads",
        ] {
            assert!(
                text.contains(&format!("# TYPE {METRIC_PREFIX}{name} ")),
                "missing {name}"
            );
        }
        assert!(text.contains("rvoip_bench_uas_responses_sent_total{code=\"200\"} 100\n"));
        assert!(text.contains("rvoip_bench_uas_responses_sent_total{code=\"503\"} 10\n"));
        assert!(text.contains("rvoip_bench_uas_byes_answered_total{code=\"200\"} 80\n"));
        assert!(text.contains("# TYPE rvoip_bench_uas_active_calls gauge\n"));
        assert!(text.contains("# TYPE rvoip_bench_uas_invites_received_total counter\n"));
        assert!(text.contains("rvoip_bench_uas_process_cpu_seconds_total 12.500\n"));
        // Every sample line is `name[labels] value`.
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let (name, value) = line.rsplit_once(' ').expect("sample");
            assert!(name.starts_with(METRIC_PREFIX), "{line}");
            assert!(value.parse::<f64>().is_ok(), "{line}");
        }
    }
}
