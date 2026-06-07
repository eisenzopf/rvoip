//! Split-process soak receiver.
//!
//! Launched by `scripts/perf_soak_split.sh` alongside
//! `perf_soak_caller.rs`. This process owns the receiver endpoint, received
//! media counters, receiver RSS/CPU sampling, and receiver retention
//! diagnostics.

#![allow(clippy::needless_return)]

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;

#[path = "support/mod.rs"]
mod support;
use support::soak::{
    boot_receiver, endpoint_metric, endpoint_retention_summary, perf_config, read_required_u16_env,
    retention_drain_wait, round2, rss_result_metrics, EndpointRetentionSampler,
    ReceiverDiagnostics, RssGrowthGate, SoakLoadSettings, ALICE_PORT_ENV, BOB_PORT_ENV,
    READY_FILE_ENV, STOP_FILE_ENV,
};
use support::{LoadProfile, ResourceSampler, ScenarioReport};

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_soak_receiver() {
    let settings = SoakLoadSettings::from_env();
    let bob_port = read_required_u16_env(BOB_PORT_ENV);
    let alice_port = read_required_u16_env(ALICE_PORT_ENV);
    let ready_file = PathBuf::from(
        std::env::var(READY_FILE_ENV)
            .unwrap_or_else(|_| panic!("{READY_FILE_ENV} must be set for receiver")),
    );
    let stop_file = PathBuf::from(
        std::env::var(STOP_FILE_ENV)
            .unwrap_or_else(|_| panic!("{STOP_FILE_ENV} must be set for receiver")),
    );
    let receiver_cfg = perf_config("perf-soak-bob", bob_port);
    let caller_cfg = perf_config("perf-soak-alice", alice_port);
    let rss_gate = RssGrowthGate::resolve(&caller_cfg, &receiver_cfg);
    let app_event_capacity = receiver_cfg.global_event_channel_capacity;
    let session_event_dispatcher_capacity = receiver_cfg.session_event_dispatcher_channel_capacity;
    let sip_transaction_command_channel_capacity = receiver_cfg
        .sip_transaction_command_channel_capacity
        .unwrap_or(
            rvoip_sip::api::unified::Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
        );
    let retention_drain_wait = retention_drain_wait();
    let diagnostics = ReceiverDiagnostics::default();
    let receiver = boot_receiver(receiver_cfg, diagnostics.clone()).await;
    let sampler = ResourceSampler::start(Duration::from_secs(5));
    let retention_sampler = EndpointRetentionSampler::start(
        "receiver",
        receiver.coordinator.clone(),
        Duration::from_secs(5),
    );
    std::fs::write(&ready_file, "ready\n").expect("write receiver ready file");

    let started = std::time::Instant::now();
    let max_wait =
        settings.total() + retention_drain_wait + settings.call_timeout + Duration::from_secs(300);
    let mut stop_seen = false;
    loop {
        if stop_file.exists() {
            stop_seen = true;
            break;
        }
        if started.elapsed() >= max_wait {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let active_secs = started.elapsed().as_secs_f64();

    tokio::time::sleep(retention_drain_wait).await;
    let retention_samples = retention_sampler.stop().await;
    let final_retention = retention_samples
        .last()
        .cloned()
        .unwrap_or_else(|| json!({}));
    let retained_after_drain = final_retention["retained_total"].as_u64().unwrap_or(0);
    let active_audio_receivers = diagnostics.active_audio_receivers.load(Ordering::Relaxed);
    let completed_audio_receivers = diagnostics
        .completed_audio_receivers
        .load(Ordering::Relaxed);
    let received_frames = diagnostics.received_frames.load(Ordering::Relaxed);
    let resources = sampler.stop().await;
    let rss = rss_result_metrics(&resources, active_secs, retention_drain_wait.as_secs_f64());

    let load = LoadProfile {
        target_cps: 0.0,
        ramp_secs: 0,
        steady_secs: active_secs.round() as u64,
        cooldown_secs: retention_drain_wait.as_secs(),
    };
    let mut report = ScenarioReport::new("perf_soak_receiver", load);
    report
        .result("process_role", "receiver")
        .result("stop_seen", stop_seen)
        .result("active_secs", round2(active_secs))
        .result("configured_duration_secs", settings.duration_secs)
        .result("global_event_channel_capacity", app_event_capacity)
        .result(
            "session_event_dispatcher_channel_capacity",
            session_event_dispatcher_capacity,
        )
        .result(
            "sip_transaction_command_channel_capacity",
            sip_transaction_command_channel_capacity,
        )
        .result("retention_drain_wait_secs", retention_drain_wait.as_secs())
        .result("bob_received_frames", received_frames)
        .result("bob_active_audio_receivers", active_audio_receivers)
        .result("bob_completed_audio_receivers", completed_audio_receivers)
        .result("rss_growth_mb_per_hr", round2(rss.full_growth_mb_per_hr))
        .result(
            "rss_sustained_growth_mb_per_hr",
            round2(rss.sustained_growth_mb_per_hr),
        )
        .result(
            "rss_post_drain_growth_mb_per_hr",
            round2(rss.post_drain_growth_mb_per_hr),
        )
        .result(
            "rss_post_drain_sample_count",
            rss.post_drain_sample_count as u64,
        )
        .result(
            "rss_gate_growth_mb_per_hr",
            round2(rss.gate_growth_mb_per_hr),
        )
        .result("rss_gate_window", rss.gate_window)
        .result_block("rss_gate", rss_gate.to_json())
        .result("retained_objects_after_drain", retained_after_drain)
        .result(
            "transaction_manager_active_after_drain",
            endpoint_metric(&final_retention["receiver"], "/transaction_manager/total"),
        )
        .result(
            "transaction_runner_active_after_drain",
            endpoint_metric(
                &final_retention["receiver"],
                "/sip_dialog_diagnostics/transaction_runner/active",
            ),
        )
        .result(
            "lifecycle_expired_terminal_entries_after_drain",
            endpoint_metric(
                &final_retention["receiver"],
                "/lifecycle/expired_terminal_entries",
            ),
        )
        .result(
            "lifecycle_terminal_entries_after_drain",
            endpoint_metric(&final_retention["receiver"], "/lifecycle/terminal_entries"),
        )
        .result_block(
            "retention",
            endpoint_retention_summary(&retention_samples, retained_after_drain, "receiver"),
        )
        .diagnostic_block(
            "retention_samples",
            json!({
                "sample_count": retention_samples.len(),
                "samples": retention_samples,
                "final_retained_objects": retained_after_drain,
            }),
        )
        .diagnostic_block(
            "rss_windows",
            json!({
                "windows": rss.windows,
                "gate_window": rss.gate_window,
                "gate_growth_mb_per_hr": round2(rss.gate_growth_mb_per_hr),
            }),
        )
        .with_resources(resources);
    let json_path = report.write_json();
    report.print_summary(&json_path);

    receiver.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), receiver.task).await;

    let mut gate_failures = Vec::new();
    if !stop_seen {
        gate_failures.push("receiver stop file was not observed".to_string());
    }
    if rss.gate_growth_mb_per_hr > rss_gate.effective_mb_per_hr {
        gate_failures.push(format!(
            "receiver RSS gate growth {:.2} MB/hr over {} window exceeded effective threshold {:.2} MB/hr ({})",
            rss.gate_growth_mb_per_hr, rss.gate_window, rss_gate.effective_mb_per_hr, rss_gate.source
        ));
    }
    if retained_after_drain != 0 {
        gate_failures.push(format!(
            "receiver_retained_objects_after_drain={retained_after_drain}"
        ));
    }
    if active_audio_receivers != 0 {
        gate_failures.push(format!(
            "bob_active_audio_receivers={active_audio_receivers}"
        ));
    }
    assert!(
        gate_failures.is_empty(),
        "perf_soak_receiver gate failed:\n{}",
        gate_failures.join("\n")
    );
}
