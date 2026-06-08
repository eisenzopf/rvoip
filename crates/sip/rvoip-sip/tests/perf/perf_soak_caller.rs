//! Split-process soak caller.
//!
//! Launched by `scripts/perf_soak_split.sh` alongside
//! `perf_soak_receiver.rs`. This process owns the caller endpoint, call
//! generation, setup latency histograms, caller RSS/CPU sampling, and caller
//! retention diagnostics.

#![allow(clippy::needless_return)]

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[path = "support/mod.rs"]
mod support;
use support::soak::{
    boot_caller, diagnostic_sample_path, endpoint_metric, endpoint_retention_summary,
    in_process_resource_sampler_enabled, media_receive_diagnostics, memory_diagnostic_interval,
    memory_diagnostic_summary, perf_config, read_required_u16_env, resource_sampling_diagnostics,
    retention_drain_wait, round2, round4, rss_result_metrics, run_caller_load, DhatProfile,
    MemoryDiagnosticSampler, RssGrowthGate, SoakCounters, SoakLoadSettings, ALICE_PORT_ENV,
    BOB_PORT_ENV,
};
use support::{LatencyHistogram, LoadProfile, ResourceSampler, ResourceSummary, ScenarioReport};

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_soak_caller() {
    let dhat_profile = DhatProfile::start("caller");
    let settings = SoakLoadSettings::from_env();
    let bob_port = read_required_u16_env(BOB_PORT_ENV);
    let alice_port = read_required_u16_env(ALICE_PORT_ENV);
    let receiver_cfg = perf_config("perf-soak-bob", bob_port);
    let caller_cfg = perf_config("perf-soak-alice", alice_port);
    let rss_gate = RssGrowthGate::resolve(&caller_cfg, &receiver_cfg);
    let app_event_capacity = caller_cfg.global_event_channel_capacity;
    let session_event_dispatcher_capacity = caller_cfg.session_event_dispatcher_channel_capacity;
    let sip_transaction_command_channel_capacity = caller_cfg
        .sip_transaction_command_channel_capacity
        .unwrap_or(
            rvoip_sip::api::unified::Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
        );
    let retention_drain_wait = retention_drain_wait();

    let caller = boot_caller(caller_cfg).await;
    let from = format!("sip:alice@127.0.0.1:{alice_port}");
    let target_uri = format!("sip:bob@127.0.0.1:{bob_port}");
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let first_minute_hist = Arc::new(LatencyHistogram::new("setup_latency_minute_1"));
    let last_minute_hist = Arc::new(LatencyHistogram::new("setup_latency_last_minute"));
    let counters = Arc::new(SoakCounters::default());
    let in_process_resource_sampling = in_process_resource_sampler_enabled();
    let sampler = if in_process_resource_sampling {
        Some(ResourceSampler::start_with_output(
            Duration::from_secs(5),
            diagnostic_sample_path("caller", "resource"),
        ))
    } else {
        None
    };
    let retention_sampler = support::soak::EndpointRetentionSampler::start(
        "caller",
        Arc::clone(&caller),
        Duration::from_secs(5),
    );
    let memory_sampler =
        MemoryDiagnosticSampler::start("caller", &settings, memory_diagnostic_interval());

    run_caller_load(
        Arc::clone(&caller),
        from,
        target_uri,
        settings.clone(),
        Arc::clone(&counters),
        Arc::clone(&setup_hist),
        Arc::clone(&first_minute_hist),
        Arc::clone(&last_minute_hist),
    )
    .await;

    tokio::time::sleep(retention_drain_wait).await;
    let retention_series = retention_sampler.stop().await;
    let memory_series = match memory_sampler {
        Some(sampler) => Some(sampler.stop().await),
        None => None,
    };
    let final_retention = retention_series
        .final_sample
        .clone()
        .unwrap_or_else(|| json!({}));
    let retained_after_drain = retention_series.final_retained_objects;
    let mut resources = match sampler {
        Some(sampler) => sampler.stop().await,
        None => ResourceSummary::empty(),
    };
    let rss = rss_result_metrics(
        &resources,
        settings.duration_secs as f64,
        retention_drain_wait.as_secs_f64(),
    );
    resources.samples.clear();
    let dhat_diagnostics = dhat_profile.finish();

    let fm = first_minute_hist.snapshot();
    let lm = last_minute_hist.snapshot();
    let drift_pct = if fm.p99 > 0 && lm.p99 > 0 {
        ((lm.p99 as f64 - fm.p99 as f64) / fm.p99 as f64) * 100.0
    } else {
        0.0
    };
    let offered = counters.offered.load(Ordering::Relaxed);
    let succeeded = counters.succeeded.load(Ordering::Relaxed);
    let failed = counters.failed.load(Ordering::Relaxed);
    let media_setup_failed = counters.media_setup_failed.load(Ordering::Relaxed);
    let teardown_failed = counters.teardown_failed.load(Ordering::Relaxed);
    let active_offered = counters.active_offered.load(Ordering::Relaxed);
    let active_succeeded = counters.active_succeeded.load(Ordering::Relaxed);
    let churn_offered = counters.churn_offered.load(Ordering::Relaxed);
    let churn_succeeded = counters.churn_succeeded.load(Ordering::Relaxed);
    let asr = if offered > 0 {
        succeeded as f64 / offered as f64
    } else {
        0.0
    };

    let load = LoadProfile {
        target_cps: settings.soak_cps,
        ramp_secs: 0,
        steady_secs: settings.duration_secs,
        cooldown_secs: retention_drain_wait.as_secs(),
    };
    let mut report = ScenarioReport::new("perf_soak_caller", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let cps_per_core = if cores > 0.0 {
        (succeeded as f64 / settings.duration_secs as f64) / cores
    } else {
        0.0
    };
    report
        .result("process_role", "caller")
        .result("in_process_resource_sampling", in_process_resource_sampling)
        .result("memory_diagnostics_enabled", memory_series.is_some())
        .result("duration_secs", settings.duration_secs)
        .result("soak_cps", settings.soak_cps)
        .result("active_calls_target", settings.active_calls)
        .result("active_calls_initial", settings.initial_active_calls())
        .result("active_calls_final", settings.final_active_calls())
        .result_block("active_call_phases", settings.active_phases_json())
        .result("media_calls_held", settings.active_calls)
        .result("active_call_min_hold_secs", settings.min_hold_secs)
        .result("active_call_max_hold_secs", settings.max_hold_secs)
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
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result("active_calls_offered", active_offered)
        .result("active_calls_succeeded", active_succeeded)
        .result("churn_calls_offered", churn_offered)
        .result("churn_calls_succeeded", churn_succeeded)
        .result("cps_per_core", round2(cps_per_core))
        .result("asr", round4(asr))
        .result("latency_drift_pct", round2(drift_pct))
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
            endpoint_metric(&final_retention["caller"], "/transaction_manager/total"),
        )
        .result(
            "transaction_runner_active_after_drain",
            endpoint_metric(
                &final_retention["caller"],
                "/sip_dialog_diagnostics/transaction_runner/active",
            ),
        )
        .result(
            "lifecycle_expired_terminal_entries_after_drain",
            endpoint_metric(
                &final_retention["caller"],
                "/lifecycle/expired_terminal_entries",
            ),
        )
        .result(
            "lifecycle_terminal_entries_after_drain",
            endpoint_metric(&final_retention["caller"], "/lifecycle/terminal_entries"),
        )
        .result_block("retention", endpoint_retention_summary(&retention_series))
        .diagnostic_block(
            "retention_samples",
            json!({
                "sample_count": retention_series.sample_count,
                "samples_path": retention_series.samples_path.display().to_string(),
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
        .diagnostic_block(
            "memory_diagnostics",
            memory_diagnostic_summary(memory_series.as_ref()),
        )
        .diagnostic_block(
            "resource_sampling",
            resource_sampling_diagnostics("caller", in_process_resource_sampling),
        )
        .diagnostic_block("media_receive", media_receive_diagnostics())
        .diagnostic_block("dhat", dhat_diagnostics)
        .result(
            "errors",
            json!({
                "call_failed": failed,
                "media_setup_failed": media_setup_failed,
                "teardown_failed": teardown_failed,
            }),
        )
        .latency(&setup_hist)
        .latency(&first_minute_hist)
        .latency(&last_minute_hist)
        .with_resources(resources);
    let json_path = report.write_json();
    report.print_summary(&json_path);
    drop(caller);

    let mut gate_failures = Vec::new();
    if rss.gate_growth_mb_per_hr > rss_gate.effective_mb_per_hr {
        gate_failures.push(format!(
            "caller RSS gate growth {:.2} MB/hr over {} window exceeded effective threshold {:.2} MB/hr ({})",
            rss.gate_growth_mb_per_hr, rss.gate_window, rss_gate.effective_mb_per_hr, rss_gate.source
        ));
    }
    if asr < 0.999 {
        gate_failures.push(format!("ASR {:.4} below 0.999", asr));
    }
    if failed != 0 {
        gate_failures.push(format!("call_failed={failed}"));
    }
    if media_setup_failed != 0 {
        gate_failures.push(format!("media_setup_failed={media_setup_failed}"));
    }
    if teardown_failed != 0 {
        gate_failures.push(format!("teardown_failed={teardown_failed}"));
    }
    if retained_after_drain != 0 {
        gate_failures.push(format!(
            "caller_retained_objects_after_drain={retained_after_drain}"
        ));
    }
    assert!(
        gate_failures.is_empty(),
        "perf_soak_caller gate failed:\n{}",
        gate_failures.join("\n")
    );
}
