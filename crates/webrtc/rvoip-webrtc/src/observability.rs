//! Operational observability helpers — Prometheus text-format exporter for
//! [`crate::adapter::WebRtcMetrics`].
//!
//! Self-contained: emits `# HELP` / `# TYPE` lines + one sample per series.
//! No `prometheus` crate dependency — for richer features (histograms, labels)
//! the caller is free to wire `metrics::Recorder` / `prometheus_client`
//! manually using `adapter.metrics()`.

use std::fmt::Write;

use crate::adapter::WebRtcMetrics;
use crate::media::WebRtcStatsSnapshot;

/// Render `metrics` as a Prometheus text exposition body.
///
/// Series names are prefixed with `rvoip_webrtc_`. Counters are monotonic
/// (`reaped_total`, `inbound_total`, `outbound_total`, `signaling_errors_total`,
/// `sessions_rejected_over_cap`); `active_sessions` is a gauge.
pub fn render_prometheus(metrics: &WebRtcMetrics) -> String {
    let mut out = String::with_capacity(512);
    emit_counter(
        &mut out,
        "rvoip_webrtc_inbound_total",
        "Total inbound WebRTC sessions accepted (counter).",
        metrics.inbound_total,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_outbound_total",
        "Total outbound WebRTC sessions initiated by originate (counter).",
        metrics.outbound_total,
    );
    emit_gauge(
        &mut out,
        "rvoip_webrtc_active_sessions",
        "Currently live WebRTC sessions (gauge).",
        metrics.active_sessions as u64,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_signaling_errors_total",
        "Total signaling-layer errors observed (counter).",
        metrics.signaling_errors_total,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_sessions_rejected_over_cap",
        "Sessions rejected because the concurrent-session cap was reached (counter).",
        metrics.sessions_rejected_over_cap,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_reaped_total",
        "Sessions reaped by the idle/failure reaper (counter).",
        metrics.reaped_total,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_data_messages_dropped_total",
        "Data messages dropped because the bounded adapter event queue was full (counter).",
        metrics.data_messages_dropped_total,
    );
    emit_gauge(
        &mut out,
        "rvoip_webrtc_active_http_resources",
        "Live WHIP/WHEP HTTP resources with retained mutation state (gauge).",
        metrics.active_http_resources as u64,
    );
    emit_gauge(
        &mut out,
        "rvoip_webrtc_http_resource_tasks",
        "Live WHEP HTTP resource expiry supervisors (gauge).",
        metrics.http_resource_tasks as u64,
    );
    emit_gauge(
        &mut out,
        "rvoip_webrtc_peer_session_tasks",
        "Live adapter-owned tasks supervised by WebRTC peer routes (gauge).",
        metrics.peer_session_tasks as u64,
    );
    emit_gauge(
        &mut out,
        "rvoip_webrtc_media_tasks",
        "Live RTP pump and stats tasks retained by WebRTC media streams (gauge).",
        metrics.media_tasks as u64,
    );
    emit_gauge(
        &mut out,
        "rvoip_webrtc_inbound_ws_connection_tasks",
        "Live bounded inbound WS/WSS connection tasks (gauge).",
        metrics.inbound_ws_connection_tasks as u64,
    );
    emit_gauge(
        &mut out,
        "rvoip_webrtc_inbound_admission_tasks",
        "Live inbound admission-confirmation tasks awaiting an application decision (gauge).",
        metrics.inbound_admission_tasks as u64,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_inbound_ws_connections_rejected_total",
        "Inbound WS/WSS connections rejected because the connection-task budget was exhausted (counter).",
        metrics.inbound_ws_connections_rejected_total,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_legacy_whep_sessions_total",
        "WHEP sessions created through explicit legacy server-offer mode (counter).",
        metrics.legacy_whep_sessions_total,
    );
    out
}

/// G4 — Prometheus body extended with the aggregated per-stream snapshot
/// (outbound + inbound + candidate pair). Wires straight into
/// [`crate::adapter::WebRtcAdapter::aggregated_stats`].
///
/// New series (counter):
///   * `rvoip_webrtc_inbound_packets_total`
///   * `rvoip_webrtc_inbound_bytes_total`
///   * `rvoip_webrtc_packets_lost_total`
///   * `rvoip_webrtc_outbound_packets_total`
///   * `rvoip_webrtc_outbound_bytes_total`
///   * `rvoip_webrtc_retransmitted_packets_total`
///   * `rvoip_webrtc_nack_count_total`
///   * `rvoip_webrtc_pli_count_total`
///   * `rvoip_webrtc_fir_count_total`
///   * `rvoip_webrtc_frames_dropped_total`
///
/// New series (gauge):
///   * `rvoip_webrtc_jitter_ms` (avg across streams)
///   * `rvoip_webrtc_packet_loss_pct` (avg across streams)
///   * `rvoip_webrtc_mos_estimate` (avg across streams)
///   * `rvoip_webrtc_selected_pair_rtt_ms` (only emitted when known)
///   * `rvoip_webrtc_available_outgoing_bitrate_bps` (only when known)
pub fn render_prometheus_with_stats(
    metrics: &WebRtcMetrics,
    snapshot: &WebRtcStatsSnapshot,
) -> String {
    let mut out = render_prometheus(metrics);
    emit_counter(
        &mut out,
        "rvoip_webrtc_inbound_packets_total",
        "Total RTP packets received across all streams (counter, sum).",
        snapshot.packets_received,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_inbound_bytes_total",
        "Total RTP bytes received across all streams (counter, sum).",
        snapshot.bytes_received,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_packets_lost_total",
        "Total RTP packets lost across all streams (counter, sum).",
        snapshot.packets_lost,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_frames_dropped_total",
        "Frames dropped by the inbound pump due to slow downstream consumers (counter).",
        snapshot.frames_dropped,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_outbound_packets_total",
        "Total RTP packets sent across all streams (counter, sum).",
        snapshot.outbound.packets_sent,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_outbound_bytes_total",
        "Total RTP bytes sent across all streams (counter, sum).",
        snapshot.outbound.bytes_sent,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_retransmitted_packets_total",
        "RTX packets sent (counter, sum).",
        snapshot.outbound.retransmitted_packets,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_nack_count_total",
        "NACK feedback messages sent (counter, sum).",
        snapshot.outbound.nack_count,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_pli_count_total",
        "Picture-Loss-Indication messages sent (counter, sum).",
        snapshot.outbound.pli_count,
    );
    emit_counter(
        &mut out,
        "rvoip_webrtc_fir_count_total",
        "Full-Intra-Refresh messages sent (counter, sum).",
        snapshot.outbound.fir_count,
    );
    emit_gauge_f(
        &mut out,
        "rvoip_webrtc_jitter_ms",
        "Average inter-arrival jitter across active streams (gauge).",
        snapshot.jitter_ms as f64,
    );
    emit_gauge_f(
        &mut out,
        "rvoip_webrtc_packet_loss_pct",
        "Average packet loss percentage across active streams (gauge).",
        snapshot.packet_loss_pct as f64,
    );
    emit_gauge_f(
        &mut out,
        "rvoip_webrtc_mos_estimate",
        "Average MOS estimate across active streams (gauge, 1.0-4.5).",
        snapshot.mos as f64,
    );
    if let Some(pair) = &snapshot.selected_pair {
        if let Some(rtt) = pair.current_round_trip_time_ms {
            emit_gauge_f(
                &mut out,
                "rvoip_webrtc_selected_pair_rtt_ms",
                "Most recent STUN RTT on the nominated candidate pair (gauge).",
                rtt,
            );
        }
        if let Some(bw) = pair.available_outgoing_bitrate_bps {
            emit_gauge(
                &mut out,
                "rvoip_webrtc_available_outgoing_bitrate_bps",
                "Estimated outgoing bitrate from GCC/TWCC (gauge, bits/sec).",
                bw,
            );
        }
    }
    out
}

fn emit_gauge_f(out: &mut String, name: &str, help: &str, value: f64) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} gauge");
    let _ = writeln!(out, "{name} {value:.4}");
}

fn emit_counter(out: &mut String, name: &str, help: &str, value: u64) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} counter");
    let _ = writeln!(out, "{name} {value}");
}

fn emit_gauge(out: &mut String, name: &str, help: &str, value: u64) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} gauge");
    let _ = writeln!(out, "{name} {value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_each_series_with_help_and_type() {
        let m = WebRtcMetrics {
            inbound_total: 7,
            outbound_total: 3,
            active_sessions: 2,
            signaling_errors_total: 1,
            sessions_rejected_over_cap: 0,
            reaped_total: 4,
            data_messages_dropped_total: 5,
            active_http_resources: 6,
            http_resource_tasks: 1,
            peer_session_tasks: 3,
            media_tasks: 4,
            inbound_ws_connection_tasks: 5,
            inbound_admission_tasks: 7,
            inbound_ws_connections_rejected_total: 6,
            legacy_whep_sessions_total: 2,
        };
        let body = render_prometheus(&m);

        for series in [
            "rvoip_webrtc_inbound_total",
            "rvoip_webrtc_outbound_total",
            "rvoip_webrtc_active_sessions",
            "rvoip_webrtc_signaling_errors_total",
            "rvoip_webrtc_sessions_rejected_over_cap",
            "rvoip_webrtc_reaped_total",
            "rvoip_webrtc_data_messages_dropped_total",
            "rvoip_webrtc_active_http_resources",
            "rvoip_webrtc_http_resource_tasks",
            "rvoip_webrtc_peer_session_tasks",
            "rvoip_webrtc_media_tasks",
            "rvoip_webrtc_inbound_ws_connection_tasks",
            "rvoip_webrtc_inbound_admission_tasks",
            "rvoip_webrtc_inbound_ws_connections_rejected_total",
            "rvoip_webrtc_legacy_whep_sessions_total",
        ] {
            assert!(
                body.contains(&format!("# HELP {series}")),
                "missing HELP for {series}"
            );
            assert!(
                body.contains(&format!("# TYPE {series}")),
                "missing TYPE for {series}"
            );
        }
        assert!(body.contains("rvoip_webrtc_inbound_total 7"));
        assert!(body.contains("rvoip_webrtc_active_sessions 2"));
        assert!(body.contains("rvoip_webrtc_reaped_total 4"));
        assert!(body.contains("rvoip_webrtc_active_http_resources 6"));
        assert!(body.contains("rvoip_webrtc_inbound_admission_tasks 7"));
        assert!(body.contains("rvoip_webrtc_legacy_whep_sessions_total 2"));
    }

    #[test]
    fn counter_and_gauge_types_are_correct() {
        let body = render_prometheus(&WebRtcMetrics::default());
        assert!(body.contains("# TYPE rvoip_webrtc_active_sessions gauge"));
        assert!(body.contains("# TYPE rvoip_webrtc_inbound_total counter"));
    }
}
