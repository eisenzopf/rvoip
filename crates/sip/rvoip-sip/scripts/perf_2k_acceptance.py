#!/usr/bin/env python3
"""Enforce the absolute rvoip-sip 2,000-CPS beta acceptance limits."""

import argparse
import hashlib
import json
import math
import numbers
import pathlib
import sys


CLEANUP_RSS_INTENT_MB_PER_HOUR = 10.0

LIMITS = (
    ("results.achieved_cps", ">=", 1578.53),
    ("results.asr", ">=", 0.999),
    ("results.ner", ">=", 0.999),
    ("latency_ns.setup_latency.p50", "<=", 13.97e6),
    ("latency_ns.setup_latency.p95", "<=", 15.36e6),
    ("latency_ns.setup_latency.p99", "<=", 16.69e6),
    ("latency_ns.full_cycle.p50", "<=", 154.66e6),
    ("latency_ns.full_cycle.p95", "<=", 156.88e6),
    ("latency_ns.full_cycle.p99", "<=", 159.66e6),
    ("resources.peak_rss_mb", "<=", 3202.26),
    # The reviewed 2,378.44 limit was measured during the 2,000-CPS active
    # point after four shared-peer conditioning points. It is not a 60-second
    # idle-tail limit; keep the number and give the measurement its honest
    # phase name.
    ("resources.rss_active_growth_mb_per_min", "<=", 2378.44),
    # Short-window OLS slopes are unstable when projected to an hour. Gate the
    # robust endpoint-median delta after normalizing it by the actual median
    # timestamp separation of those endpoint bands. Using the outer 95-second
    # window here would silently relax the 10 MB/hour intent because the
    # representative samples are closer together than the window boundaries.
    (
        "resources.rss_cleanup_endpoint_growth_mb_per_hour",
        "<=",
        CLEANUP_RSS_INTENT_MB_PER_HOUR,
    ),
)

CANONICAL_SCENARIO = "perf_call_setup_cps_pbx-media-server"
CANONICAL_CALLS = 65_000
CANONICAL_SWEEP = [30.0, 100.0, 300.0, 1000.0, 2000.0]
CANONICAL_CONDITIONING = [
    {"target_cps": 30.0, "calls_offered": 975, "calls_succeeded": 975},
    {"target_cps": 100.0, "calls_offered": 3250, "calls_succeeded": 3250},
    {"target_cps": 300.0, "calls_offered": 9750, "calls_succeeded": 9750},
    {"target_cps": 1000.0, "calls_offered": 32500, "calls_succeeded": 32500},
]


def bundled_recipe_sha256():
    recipe = pathlib.Path(__file__).resolve().parent.parent / "config" / "performance-recipes.yaml"
    return hashlib.sha256(recipe.read_bytes()).hexdigest()


def canonical_exact_values():
    """Fields that make a passing result the reviewed workload, not just a fast run."""
    runtime = "diagnostics.effective_config.runtime_switches.environment"
    effective = "diagnostics.effective_config"
    bob = f"{effective}.bob"
    return (
        ("scenario", CANONICAL_SCENARIO),
        ("load.target_cps", 2000.0),
        ("load.ramp_secs", 5),
        ("load.steady_secs", 30),
        ("load.cooldown_secs", 5),
        ("results.calls_offered", CANONICAL_CALLS),
        ("results.calls_succeeded", CANONICAL_CALLS),
        ("environment.build_profile", "release"),
        ("environment.global_allocator", "mimalloc"),
        ("environment.mimalloc_enabled", True),
        ("environment.cargo_features", ["perf-tests"]),
        ("environment.requested_cargo_features", "perf-tests"),
        ("resources.rss_tail_window_requested_secs", 60.0),
        (f"{effective}.profile", "pbx-media-server"),
        (f"{effective}.report_scenario", CANONICAL_SCENARIO),
        (f"{effective}.client_profile", "endpoint"),
        (f"{effective}.alice_shards", 4),
        (f"{effective}.channel_capacity", 8000),
        (f"{effective}.alice_channel_capacity_per_shard", 2000),
        (f"{effective}.recipe_file", None),
        (f"{effective}.recipe.source", "bundled"),
        (f"{effective}.recipe.sha256", bundled_recipe_sha256()),
        (f"{effective}.max_in_flight_override", None),
        (f"{effective}.retained_lifecycle_sizing.max_offered_cps", 2000),
        (f"{effective}.retained_lifecycle_sizing.anti_reuse_horizon_secs", 64),
        (f"{effective}.retained_lifecycle_sizing.churn_headroom_percent", 25),
        (f"{runtime}.RVOIP_PERF_BUILD_FEATURES", "perf-tests"),
        (f"{runtime}.RVOIP_PERF_RUN_MODE", "clean"),
        (f"{runtime}.RVOIP_PERF_SWEEP_CPS", "30,100,300,1000,2000"),
        (f"{runtime}.RVOIP_PERF_TARGET_CPS", None),
        (f"{runtime}.RVOIP_PERF_RAMP_SECS", "5"),
        (f"{runtime}.RVOIP_PERF_STEADY_SECS", "30"),
        (f"{runtime}.RVOIP_PERF_COOLDOWN_SECS", "5"),
        (f"{runtime}.RVOIP_PERF_CALL_TIMEOUT_SECS", "15"),
        (f"{runtime}.RVOIP_PERF_WORKER_THREADS", "8"),
        (f"{runtime}.RVOIP_PERF_PROFILE", "pbx-media-server"),
        (f"{runtime}.RVOIP_PERF_CLIENT_PROFILE", "endpoint"),
        (f"{runtime}.RVOIP_PERF_ALICE_SHARDS", "4"),
        (f"{runtime}.RVOIP_PERF_RECIPE_FILE", None),
        (f"{runtime}.RVOIP_PERF_MAX_IN_FLIGHT", None),
        (f"{runtime}.RVOIP_PERF_SCHED_TICK_MS", "1"),
        (f"{runtime}.RVOIP_PERF_REPORT_SCENARIO", CANONICAL_SCENARIO),
        (f"{runtime}.RVOIP_PERF_MIN_ASR", "0.999"),
        (f"{runtime}.RVOIP_PERF_REQUIRE_ALL_POINTS", "1"),
        (f"{runtime}.RVOIP_PERF_REQUIRE_ZERO_ERRORS", "1"),
        (f"{runtime}.RVOIP_PERF_RETENTION_SNAPSHOT", "0"),
        (f"{runtime}.RVOIP_PERF_BOUNDARY_SNAPSHOT", "0"),
        (f"{runtime}.RVOIP_PERF_EMBED_RESOURCE_SAMPLES", "0"),
        (f"{runtime}.RVOIP_PERF_RSS_TAIL_WINDOW_SECS", "60"),
        (f"{runtime}.RVOIP_PERF_POST_DRAIN_SAMPLE_SECS", "95"),
        (f"{runtime}.RVOIP_PERF_CALL_SETUP_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_PERF_MEMORY_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_PERF_ALLOCATOR_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_PERF_SYSTEM_ALLOCATOR", "0"),
        (f"{runtime}.RVOIP_PERF_DHAT", "0"),
        (f"{runtime}.RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY", "0"),
        (f"{runtime}.RVOIP_MEDIA_AUDIO_TX_PACING", "0"),
        (f"{runtime}.RVOIP_MEDIA_AUDIO_TX_PACING_TARGET_ACTIVE", None),
        (f"{runtime}.RVOIP_MEDIA_AUDIO_TX_SHARED_SCHEDULER", "0"),
        (f"{runtime}.RVOIP_MEDIA_AUDIO_TX_SHARED_BATCH_SIZE", None),
        (f"{runtime}.RVOIP_MEDIA_AUDIO_QUALITY_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_MEDIA_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_RTP_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_SIP_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_SRTP_DIAGNOSTICS", "0"),
        (f"{runtime}.RVOIP_TEST", None),
        (f"{runtime}.RVOIP_TEST_TRANSACTION_TIMEOUT_MS", None),
        (f"{effective}.runtime_switches.effective.audio_tx_pacing", False),
        (f"{effective}.runtime_switches.effective.audio_tx_shared_scheduler", False),
        (f"{effective}.runtime_switches.effective.skip_audio_frame_delivery", False),
        (f"{effective}.runtime_switches.effective.retention_snapshot", False),
        (f"{effective}.runtime_switches.effective.boundary_snapshot", False),
        (f"{effective}.runtime_switches.effective.call_setup_diagnostics", False),
        (f"{effective}.runtime_switches.effective.memory_diagnostics", False),
        (f"{effective}.runtime_switches.effective.allocator_diagnostics", False),
        (f"{effective}.runtime_switches.effective.compiled_diagnostic_features.call_setup", False),
        (f"{effective}.runtime_switches.effective.compiled_diagnostic_features.infra_memory", False),
        (f"{effective}.runtime_switches.effective.compiled_diagnostic_features.media", False),
        (f"{effective}.runtime_switches.effective.compiled_diagnostic_features.media_memory", False),
        (f"{effective}.runtime_switches.effective.compiled_diagnostic_features.rtp_memory", False),
        (f"{bob}.media_mode.kind", "enabled"),
        (f"{bob}.incoming_call_channel_capacity", 8000),
        (f"{bob}.state_event_channel_capacity", 8000),
        (f"{bob}.sip_transport_channel_capacity", 80_000),
        (f"{bob}.sip_udp_recv_buffer_size", 8_388_608),
        (f"{bob}.sip_udp_send_buffer_size", 8_388_608),
        (f"{bob}.sip_udp_parse_workers", 4),
        (f"{bob}.sip_udp_parse_queue_capacity", 8000),
        (f"{bob}.sip_udp_parse_dispatch", "RoundRobin"),
        (f"{bob}.transaction_event_channel_capacity", 80_000),
        (f"{bob}.sip_transaction_dispatch_workers", 2),
        (f"{bob}.sip_transaction_command_channel_capacity", 128),
        (f"{bob}.effective_sip_transaction_command_channel_capacity", 128),
        (f"{bob}.sip_transaction_dispatch_priority_burst_max", None),
        (f"{bob}.effective_sip_transaction_dispatch_priority_burst_max", 64),
        (f"{bob}.sip_invite_2xx_retransmit_max_due_per_tick", None),
        (f"{bob}.effective_sip_invite_2xx_retransmit_max_due_per_tick", 2048),
        (f"{bob}.sip_dialog_dispatch_workers", 4),
        (f"{bob}.global_event_channel_capacity", 80_000),
        (f"{bob}.session_event_dispatcher_workers", 4),
        (f"{bob}.session_event_dispatcher_channel_capacity", 80_000),
        (f"{bob}.server_call_capacity", 8000),
        (f"{bob}.server_retained_lifecycle_capacity", 168_000),
        (f"{bob}.server_call_admission_limit", 8000),
        (f"{bob}.server_call_admission_soft_limit", 7200),
        (f"{bob}.server_call_admission_pacing_delay_ms", 1),
        (f"{bob}.server_overload_retry_after_secs", 1),
        (f"{bob}.media_port_start", 16_384),
        (f"{bob}.media_port_end", 40_999),
        (f"{bob}.media_port_capacity", 24_616),
        (f"{bob}.media_session_capacity", 8000),
        (f"{bob}.auto_180_ringing", False),
        (f"{bob}.auto_100_trying", False),
        (f"{bob}.fast_auto_accept_incoming_calls", True),
        (f"{bob}.diagnostics.sip_udp", False),
        (f"{bob}.diagnostics.transaction_timing", False),
        (f"{bob}.diagnostics.dialog_timing", False),
        (f"{bob}.diagnostics.media_setup", False),
        (f"{bob}.diagnostics.cleanup", False),
    )


def dotted_get(value, path):
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            raise KeyError(path)
        value = value[part]
    return value


def is_number(value):
    return (
        isinstance(value, numbers.Number)
        and not isinstance(value, bool)
        and math.isfinite(float(value))
    )


def add_exact_check(checks, report, path, expected):
    try:
        actual = dotted_get(report, path)
        passed = actual == expected
        error = None
    except KeyError as exc:
        actual = None
        passed = False
        error = str(exc)
    checks.append(
        {
            "metric": path,
            "operator": "==",
            "limit": expected,
            "actual": actual,
            "passed": passed,
            "error": error,
        }
    )


def alice_shard_identity(report):
    """Return a compact identity summary for the four historical endpoint shards."""
    try:
        shards = dotted_get(report, "diagnostics.effective_config.alice_shard_configs")
    except KeyError:
        return None, "missing alice_shard_configs"
    if not isinstance(shards, list) or len(shards) != 4:
        return shards, "alice_shard_configs must contain exactly four entries"

    expected_ranges = (
        (51_000, 54_633),
        (54_634, 58_267),
        (58_268, 61_901),
        (61_902, 65_535),
    )
    summary = []
    for index, (entry, (start, end)) in enumerate(zip(shards, expected_ranges)):
        if not isinstance(entry, dict) or not isinstance(entry.get("config"), dict):
            return shards, f"alice shard {index} is not a config object"
        config = entry["config"]
        from_uri = entry.get("from")
        expected = {
            "incoming_call_channel_capacity": 1000,
            "state_event_channel_capacity": 1000,
            "sip_transport_channel_capacity": 10_000,
            "transaction_event_channel_capacity": 10_000,
            "effective_sip_transaction_command_channel_capacity": 32,
            "effective_sip_transaction_dispatch_priority_burst_max": 64,
            "effective_sip_invite_2xx_retransmit_max_due_per_tick": 2048,
            "global_event_channel_capacity": 256,
            "session_event_dispatcher_workers": 16,
            "server_call_capacity": None,
            "server_retained_lifecycle_capacity": None,
            "media_port_start": start,
            "media_port_end": end,
            "media_port_capacity": 3634,
            "media_session_capacity": 2000,
            "auto_180_ringing": True,
            "auto_100_trying": True,
            "fast_auto_accept_incoming_calls": False,
        }
        for key, expected_value in expected.items():
            if config.get(key) != expected_value:
                return (
                    shards,
                    f"alice shard {index} {key}={config.get(key)!r}, expected {expected_value!r}",
                )
        if config.get("media_mode") != {"kind": "enabled"}:
            return shards, f"alice shard {index} media_mode is not enabled"
        if config.get("diagnostics") != {
            "sip_udp": False,
            "transaction_timing": False,
            "dialog_timing": False,
            "media_setup": False,
            "cleanup": False,
        }:
            return shards, f"alice shard {index} diagnostics are not all disabled"
        prefix = f"sip:alice{index}@127.0.0.1:"
        if not isinstance(from_uri, str) or not from_uri.startswith(prefix):
            return shards, f"alice shard {index} From URI does not start with {prefix!r}"
        summary.append(
            {
                "index": index,
                "media_port_start": start,
                "media_port_end": end,
                "media_session_capacity": config["media_session_capacity"],
            }
        )
    return summary, None


def phase_marker_identity(report):
    try:
        markers = dotted_get(report, "diagnostics.phase_markers")
    except KeyError:
        return None, "missing phase_markers"
    if not isinstance(markers, list):
        return markers, "phase_markers must be an array"
    identities = {
        (marker.get("phase"), marker.get("kind"), marker.get("elapsed_ms"))
        for marker in markers
        if isinstance(marker, dict)
    }
    required_planned = {
        ("ramp_end", "planned", 5_000),
        ("steady_end", "planned", 35_000),
        ("cooldown_end", "planned", 40_000),
    }
    missing_planned = sorted(required_planned - identities)
    actual_phases = {
        marker.get("phase")
        for marker in markers
        if isinstance(marker, dict) and marker.get("kind") == "actual"
    }
    required_actual = {
        "point_start",
        "dispatch_complete",
        "calls_drained",
        "cooldown_end",
        "post_drain_cleanup_start",
        "post_drain_cleanup_end",
        "resource_sampling_stopped",
    }
    retention_markers = [
        marker
        for marker in markers
        if isinstance(marker, dict) and marker.get("kind") == "retention_snapshot_actual"
    ]
    if missing_planned:
        return markers, f"missing planned phase markers: {missing_planned!r}"
    if not required_actual.issubset(actual_phases):
        return markers, f"missing actual phase markers: {sorted(required_actual - actual_phases)!r}"
    if retention_markers:
        return markers, "clean report contains diagnostic retention snapshots"
    return markers, None


def canonical_measurement_identity_checks(checks, report):
    expected = (
        ("diagnostics.measurement_identity.schema", "rvoip-sip-perf-measurement-identity-v1"),
        ("diagnostics.measurement_identity.peer_lifecycle", "shared_for_entire_sweep"),
        ("diagnostics.measurement_identity.sweep_points_cps", CANONICAL_SWEEP),
        ("diagnostics.measurement_identity.point_index", 4),
        ("diagnostics.measurement_identity.measured_point_cps", 2000.0),
        ("diagnostics.measurement_identity.conditioning.points", CANONICAL_CONDITIONING),
        ("diagnostics.measurement_identity.conditioning.point_count", 4),
        ("diagnostics.measurement_identity.conditioning.calls_offered", 46_475),
        ("diagnostics.measurement_identity.conditioning.calls_succeeded", 46_475),
        ("diagnostics.measurement_identity.resource_window.metric", "resources.rss_active_growth_mb_per_min"),
        ("diagnostics.measurement_identity.resource_window.kind", "active_load"),
        ("diagnostics.measurement_identity.resource_window.start_phase", "point_start"),
        ("diagnostics.measurement_identity.resource_window.end_phase", "calls_drained"),
        ("diagnostics.measurement_identity.resource_window.sample_interval_ms", 500),
        ("diagnostics.measurement_identity.post_drain_cleanup.requested_secs", 95),
        ("diagnostics.measurement_identity.post_drain_cleanup.rss_metric", "resources.rss_cleanup_endpoint_growth_mb_per_hour"),
        ("diagnostics.measurement_identity.post_drain_cleanup.rss_retained_delta_metric", "resources.rss_cleanup_retained_growth_mb"),
        ("diagnostics.measurement_identity.post_drain_cleanup.rss_trend_metric", "resources.rss_cleanup_growth_mb_per_hour"),
        ("diagnostics.measurement_identity.post_drain_cleanup.rss_intent_mb_per_hour", CLEANUP_RSS_INTENT_MB_PER_HOUR),
        ("diagnostics.measurement_identity.post_drain_cleanup.rss_endpoint_estimator", "median_first_last_sixth_capped_15s"),
        ("diagnostics.measurement_identity.post_drain_cleanup.structural_metric", "diagnostics.cleanup_convergence.converged"),
    )
    for path, value in expected:
        add_exact_check(checks, report, path, value)


def add_numeric_floor_check(checks, report, path, minimum):
    try:
        actual = dotted_get(report, path)
        passed = is_number(actual) and float(actual) >= minimum
        error = None if is_number(actual) else f"required metric is not numeric: {actual!r}"
    except KeyError as exc:
        actual = None
        passed = False
        error = str(exc)
    checks.append(
        {
            "metric": path,
            "operator": ">=",
            "limit": minimum,
            "actual": actual,
            "passed": passed,
            "error": error,
        }
    )


def add_cleanup_estimator_consistency_check(checks, report):
    scalar_path = "resources.rss_cleanup_retained_growth_mb"
    window_path = "resources.rss_windows.post_drain_cleanup"
    try:
        scalar = dotted_get(report, scalar_path)
        window = dotted_get(report, window_path)
        start = window["rss_start_median_mb"]
        end = window["rss_end_median_mb"]
        retained = window["rss_retained_growth_mb"]
        start_time = window["rss_start_representative_secs"]
        end_time = window["rss_end_representative_secs"]
        separation = window["rss_endpoint_separation_secs"]
        endpoint_rate = window["rss_endpoint_growth_mb_per_hour"]
        scalar_rate = dotted_get(report, "resources.rss_cleanup_endpoint_growth_mb_per_hour")
        first_sample = window["first_sample_secs"]
        last_sample = window["last_sample_secs"]
        actual_coverage = window["actual_coverage_secs"]
        endpoint_band = window["rss_endpoint_band_secs"]
        numeric = all(
            is_number(value)
            for value in (
                scalar,
                start,
                end,
                retained,
                start_time,
                end_time,
                separation,
                endpoint_rate,
                scalar_rate,
                first_sample,
                last_sample,
                actual_coverage,
                endpoint_band,
            )
        )
        passed = numeric and math.isclose(float(retained), float(end) - float(start), abs_tol=1e-9)
        passed = passed and math.isclose(float(scalar), float(retained), abs_tol=1e-9)
        passed = passed and float(separation) > 0.0
        passed = passed and math.isclose(
            float(separation), float(end_time) - float(start_time), abs_tol=1e-9
        )
        passed = passed and math.isclose(
            float(actual_coverage), float(last_sample) - float(first_sample), abs_tol=1e-6
        )
        passed = passed and float(first_sample) <= float(start_time) <= (
            float(first_sample) + float(endpoint_band)
        )
        passed = passed and (float(last_sample) - float(endpoint_band)) <= float(end_time) <= float(last_sample)
        expected_rate = float(retained) * 3600.0 / float(separation) if passed else None
        passed = passed and math.isclose(float(endpoint_rate), expected_rate, abs_tol=1e-9)
        passed = passed and math.isclose(float(scalar_rate), float(endpoint_rate), abs_tol=1e-9)
        error = None if numeric else "cleanup endpoint estimator and time-basis fields must all be numeric"
        actual = {
            "scalar_retained_growth_mb": scalar,
            "start_median_mb": start,
            "end_median_mb": end,
            "window_retained_growth_mb": retained,
            "start_representative_secs": start_time,
            "end_representative_secs": end_time,
            "endpoint_separation_secs": separation,
            "window_endpoint_growth_mb_per_hour": endpoint_rate,
            "scalar_endpoint_growth_mb_per_hour": scalar_rate,
            "first_sample_secs": first_sample,
            "last_sample_secs": last_sample,
            "actual_coverage_secs": actual_coverage,
            "endpoint_band_secs": endpoint_band,
        }
    except (KeyError, TypeError) as exc:
        actual = None
        passed = False
        error = str(exc)
    checks.append(
        {
            "metric": "resources.rss_cleanup_endpoint_estimator_consistency",
            "operator": "delta/time-basis == window rate == scalar rate",
            "limit": True,
            "actual": actual,
            "passed": passed,
            "error": error,
        }
    )


def canonical_resource_coverage_checks(checks, report):
    exact = (
        ("resources.rss_tail_window_complete", True),
        ("resources.rss_windows.active_load.name", "active_load"),
        ("resources.rss_windows.active_load.start_phase", "point_start"),
        ("resources.rss_windows.active_load.end_phase", "calls_drained"),
        ("resources.rss_windows.active_load.complete", True),
        ("resources.rss_windows.post_drain_cleanup.name", "post_drain_cleanup"),
        ("resources.rss_windows.post_drain_cleanup.start_phase", "calls_drained"),
        ("resources.rss_windows.post_drain_cleanup.end_phase", "post_drain_cleanup_end"),
        ("resources.rss_windows.post_drain_cleanup.requested_coverage_secs", 95.0),
        ("resources.rss_windows.post_drain_cleanup.complete", True),
        ("resources.rss_windows.post_drain_cleanup.rss_endpoint_band_secs", 15.0),
        ("diagnostics.cleanup_convergence.schema", "rvoip-sip-cleanup-convergence-v1"),
        ("diagnostics.cleanup_convergence.endpoint_count", 5),
        ("diagnostics.cleanup_convergence.retained_total", 0),
        ("diagnostics.cleanup_convergence.missing_count", 0),
        ("diagnostics.cleanup_convergence.converged", True),
    )
    for path, value in exact:
        add_exact_check(checks, report, path, value)
    for path, minimum in (
        ("resources.rss_tail_window_secs", 59.0),
        ("resources.rss_tail_sample_count", 110),
        ("resources.rss_windows.active_load.actual_coverage_secs", 34.0),
        ("resources.rss_windows.active_load.sample_count", 60),
        ("resources.rss_windows.post_drain_cleanup.actual_coverage_secs", 94.0),
        ("resources.rss_windows.post_drain_cleanup.sample_count", 180),
        ("resources.rss_windows.post_drain_cleanup.rss_start_sample_count", 25),
        ("resources.rss_windows.post_drain_cleanup.rss_end_sample_count", 25),
    ):
        add_numeric_floor_check(checks, report, path, minimum)
    add_cleanup_estimator_consistency_check(checks, report)


def validate_error_tree(value, prefix="results.errors"):
    """Return (numeric leaf count, nonzero leaves, invalid leaves)."""
    if isinstance(value, dict):
        numeric_count = 0
        nonzero = []
        invalid = []
        for key, child in value.items():
            count, child_nonzero, child_invalid = validate_error_tree(
                child, f"{prefix}.{key}"
            )
            numeric_count += count
            nonzero.extend(child_nonzero)
            invalid.extend(child_invalid)
        return numeric_count, nonzero, invalid
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        return 0, [], [{"metric": prefix, "actual": value}]
    return 1, ([{"metric": prefix, "actual": value}] if value else []), []


def evaluate(report, expected_scenario, report_path):
    checks = [
        {
            "metric": "scenario",
            "operator": "==",
            "limit": expected_scenario,
            "actual": report.get("scenario") if isinstance(report, dict) else None,
            "passed": (
                isinstance(report, dict)
                and report.get("scenario") == expected_scenario
            ),
            "error": None,
        }
    ]

    for path, expected in canonical_exact_values():
        add_exact_check(checks, report, path, expected)
    canonical_measurement_identity_checks(checks, report)
    canonical_resource_coverage_checks(checks, report)

    output_root_path = (
        "diagnostics.effective_config.runtime_switches.environment."
        "RVOIP_PERF_OUTPUT_ROOT"
    )
    try:
        output_root = dotted_get(report, output_root_path)
        output_root_ok = isinstance(output_root, str) and pathlib.Path(output_root).is_absolute()
        output_root_error = None
    except KeyError as exc:
        output_root = None
        output_root_ok = False
        output_root_error = str(exc)
    checks.append(
        {
            "metric": output_root_path,
            "operator": "is_absolute_path",
            "limit": True,
            "actual": output_root,
            "passed": output_root_ok,
            "error": output_root_error,
        }
    )

    phase_markers, phase_error = phase_marker_identity(report)
    checks.append(
        {
            "metric": "diagnostics.phase_markers",
            "operator": "matches_canonical_clean_phases",
            "limit": "planned ramp/steady/cooldown plus actual lifecycle; no retention scans",
            "actual": phase_markers,
            "passed": phase_error is None,
            "error": phase_error,
        }
    )

    diagnostics = report.get("diagnostics") if isinstance(report, dict) else None
    boundary_output_present = isinstance(diagnostics, dict) and "boundary_snapshot" in diagnostics
    checks.append(
        {
            "metric": "diagnostics.boundary_snapshot",
            "operator": "is_absent",
            "limit": True,
            "actual": boundary_output_present,
            "passed": not boundary_output_present,
            "error": None,
        }
    )

    alice_identity, alice_error = alice_shard_identity(report)
    checks.append(
        {
            "metric": "diagnostics.effective_config.alice_shard_configs",
            "operator": "matches_canonical_endpoint_shards",
            "limit": 4,
            "actual": alice_identity,
            "passed": alice_error is None,
            "error": alice_error,
        }
    )

    for path, operator, limit in LIMITS:
        try:
            actual = dotted_get(report, path)
            if not is_number(actual):
                raise TypeError(f"required metric is not numeric: {actual!r}")
            passed = actual >= limit if operator == ">=" else actual <= limit
            error = None
        except (KeyError, TypeError) as exc:
            actual = None
            passed = False
            error = str(exc)
        checks.append(
            {
                "metric": path,
                "operator": operator,
                "limit": limit,
                "actual": actual,
                "passed": passed,
                "error": error,
            }
        )

    results = report.get("results") if isinstance(report, dict) else None
    results = results if isinstance(results, dict) else {}
    offered = results.get("calls_offered")
    succeeded = results.get("calls_succeeded")
    valid_call_counts = (
        isinstance(offered, int)
        and not isinstance(offered, bool)
        and offered > 0
        and isinstance(succeeded, int)
        and not isinstance(succeeded, bool)
        and succeeded == offered
    )
    checks.append(
        {
            "metric": "results.calls_succeeded == results.calls_offered",
            "operator": "==",
            "limit": offered,
            "actual": succeeded,
            "passed": valid_call_counts,
            "error": None if valid_call_counts else "missing, invalid, or unequal call counts",
        }
    )

    errors = results.get("errors")
    if isinstance(errors, dict):
        numeric_error_count, nonzero_errors, invalid_errors = validate_error_tree(errors)
    else:
        numeric_error_count = 0
        nonzero_errors = []
        invalid_errors = [{"metric": "results.errors", "actual": errors}]
    checks.append(
        {
            "metric": "results.errors.*",
            "operator": "==",
            "limit": 0,
            "actual": {
                "numeric_leaf_count": numeric_error_count,
                "nonzero": nonzero_errors,
                "invalid": invalid_errors,
            },
            "passed": (
                numeric_error_count > 0 and not nonzero_errors and not invalid_errors
            ),
            "error": None,
        }
    )

    passed = all(check["passed"] for check in checks)
    return {
        "schema": "rvoip-sip-2k-acceptance-v2",
        "status": "PASS" if passed else "FAIL",
        "report": str(report_path),
        "checks": checks,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--scenario", required=True)
    args = parser.parse_args()

    report_path = pathlib.Path(args.report)
    out_path = pathlib.Path(args.out)
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        print(f"perf-2k acceptance: cannot read {report_path}: {exc}", file=sys.stderr)
        return 2

    result = evaluate(report, args.scenario, report_path)
    try:
        out_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    except OSError as exc:
        print(f"perf-2k acceptance: cannot write {out_path}: {exc}", file=sys.stderr)
        return 2

    if result["status"] == "PASS":
        print(f"perf-2k acceptance: PASS ({report_path})")
        return 0

    for check in result["checks"]:
        if not check["passed"]:
            print(
                f"perf-2k acceptance: FAIL {check['metric']} "
                f"actual={check['actual']!r} {check['operator']} {check['limit']!r}",
                file=sys.stderr,
            )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
