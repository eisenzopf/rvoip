//! Media/RTP-only churn for memory attribution.
//!
//! This bypasses SIP entirely and repeatedly creates/stops media-core RTP
//! sessions using the same active-call phase model as the split soak. It helps
//! decide whether retained RSS belongs to media/RTP or higher SIP lifecycle
//! layers.

#[path = "support/mod.rs"]
mod support;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use rvoip_media_core::relay::controller::{MediaConfig, MediaSessionController};
use rvoip_media_core::DialogId;
use tokio::task::JoinSet;

use support::soak::{
    memory_diagnostic_interval, memory_diagnostic_summary, retention_drain_wait, round2,
    MemoryDiagnosticSampler, SoakLoadSettings,
};
use support::{LoadProfile, ScenarioReport};

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_media_churn() {
    let settings = SoakLoadSettings::from_env();
    let base_port = std::env::var("RVOIP_PERF_MEDIA_CHURN_BASE_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(30000);
    let port_span = (settings.max_active_calls().max(1) * 4)
        .try_into()
        .unwrap_or(u16::MAX - base_port);
    let max_port = base_port.saturating_add(port_span);
    let controller = std::sync::Arc::new(MediaSessionController::with_port_range_and_capacity(
        base_port,
        max_port,
        settings.max_active_calls() as usize,
    ));
    let memory_sampler =
        MemoryDiagnosticSampler::start("media_churn", &settings, memory_diagnostic_interval());

    let started = std::time::Instant::now();
    let active_deadline = started + settings.total();
    let mut tasks = JoinSet::new();
    for slot in 0..settings.max_active_calls() {
        let controller = controller.clone();
        let settings = settings.clone();
        tasks.spawn(async move {
            let mut cycle = 0u64;
            loop {
                let now = std::time::Instant::now();
                if now >= active_deadline {
                    break;
                }
                let elapsed = now.duration_since(started);
                if slot >= settings.active_calls_at(elapsed) {
                    let Some(next_activation_secs) =
                        settings.next_slot_activation_secs(slot, elapsed)
                    else {
                        break;
                    };
                    let wake_at =
                        (started + Duration::from_secs(next_activation_secs)).min(active_deadline);
                    let wait = wake_at.saturating_duration_since(std::time::Instant::now());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                    continue;
                }

                let dialog_id = DialogId::new(format!("media-churn-{slot}-{cycle}"));
                let config = MediaConfig {
                    local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                    remote_addr: None,
                    preferred_codec: Some("PCMU".to_string()),
                    parameters: HashMap::new(),
                };
                if controller
                    .start_media(dialog_id.clone(), config)
                    .await
                    .is_ok()
                {
                    let hold = media_churn_hold_duration(
                        slot,
                        cycle,
                        settings.min_hold_secs,
                        settings.max_hold_secs,
                    );
                    let mut hold_deadline = (std::time::Instant::now() + hold).min(active_deadline);
                    if let Some(deactivation_secs) =
                        settings.next_slot_deactivation_secs(slot, elapsed)
                    {
                        hold_deadline =
                            hold_deadline.min(started + Duration::from_secs(deactivation_secs));
                    }
                    let remaining =
                        hold_deadline.saturating_duration_since(std::time::Instant::now());
                    if !remaining.is_zero() {
                        tokio::time::sleep(remaining).await;
                    }
                    let _ = controller.stop_media(&dialog_id).await;
                } else {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                cycle += 1;
            }
        });
    }

    while let Some(result) = tasks.join_next().await {
        let _ = result;
    }

    let drain_wait = retention_drain_wait();
    tokio::time::sleep(drain_wait).await;
    let memory_series = match memory_sampler {
        Some(sampler) => Some(sampler.stop().await),
        None => None,
    };
    let final_counts = controller.diagnostic_counts();

    let mut report = ScenarioReport::new(
        "perf_media_churn",
        LoadProfile {
            target_cps: 0.0,
            ramp_secs: 0,
            steady_secs: settings.duration_secs,
            cooldown_secs: drain_wait.as_secs(),
        },
    );
    report
        .result("duration_secs", settings.duration_secs)
        .result("active_calls_target", settings.active_calls)
        .result("active_calls_initial", settings.initial_active_calls())
        .result("active_calls_final", settings.final_active_calls())
        .result_block("active_call_phases", settings.active_phases_json())
        .result("active_call_min_hold_secs", settings.min_hold_secs)
        .result("active_call_max_hold_secs", settings.max_hold_secs)
        .result("retention_drain_wait_secs", drain_wait.as_secs())
        .result(
            "media_sessions_after_drain",
            final_counts["sessions"].clone(),
        )
        .result(
            "rtp_sessions_after_drain",
            final_counts["rtp_sessions"].clone(),
        )
        .result(
            "rtp_streams_after_drain",
            final_counts["rtp_streams"].clone(),
        )
        .diagnostic_block("controller_after_drain", final_counts)
        .diagnostic_block(
            "memory_diagnostics",
            memory_diagnostic_summary(memory_series.as_ref()),
        )
        .result("memory_diagnostics_enabled", memory_series.is_some())
        .result("elapsed_secs", round2(started.elapsed().as_secs_f64()));
    let json_path = report.write_json();
    report.print_summary(&json_path);
}

fn media_churn_hold_duration(slot: u64, cycle: u64, min_secs: u64, max_secs: u64) -> Duration {
    if min_secs >= max_secs {
        return Duration::from_secs(min_secs);
    }
    let span = max_secs - min_secs + 1;
    let offset = slot
        .wrapping_mul(1_103_515_245)
        .wrapping_add(cycle.wrapping_mul(12_345))
        % span;
    Duration::from_secs(min_secs + offset)
}
