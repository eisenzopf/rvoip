#!/usr/bin/env python3
"""Render and validate the beta report's release-critical performance metrics."""

from __future__ import annotations

import argparse
import json
import math
import pathlib
from typing import Any


SCHEMA = "rvoip-sip-beta-performance-gate-metrics-v1"


class MetricsError(RuntimeError):
    pass


def load_json(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise MetricsError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise MetricsError(f"{path} is not a JSON object")
    return value


def one(root: pathlib.Path, pattern: str, label: str) -> pathlib.Path | None:
    matches = sorted(root.glob(pattern))
    if not matches:
        return None
    if len(matches) != 1:
        raise MetricsError(f"expected one {label} artifact, found {len(matches)}")
    return matches[0]


def finite_number(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    parsed = float(value)
    return parsed if math.isfinite(parsed) else None


def add_check(
    checks: list[dict[str, Any]],
    metric: str,
    requirement: str,
    observed: Any,
    passed: bool,
) -> None:
    checks.append(
        {
            "metric": metric,
            "requirement": requirement,
            "observed": observed,
            "passed": bool(passed),
        }
    )


def error_total(errors: Any, names: tuple[str, ...]) -> int | None:
    if not isinstance(errors, dict):
        return None
    values = [errors.get(name) for name in names]
    if any(isinstance(value, bool) or not isinstance(value, int) for value in values):
        return None
    return sum(values)


def high_density_metrics(
    perf_root: pathlib.Path,
    expected_cps: float,
    expected_min_asr: float,
    expected_rss_limit: float,
    required: bool,
) -> dict[str, Any]:
    caller_path = one(
        perf_root,
        "perf_burst_matrix/burst_*/high-density-media-burst/"
        "perf_burst_caller_high-density-media-burst.json",
        "high-density caller",
    )
    receiver_path = one(
        perf_root,
        "perf_burst_matrix/burst_*/high-density-media-burst/"
        "perf_burst_receiver_high-density-media-burst.json",
        "high-density receiver",
    )
    if caller_path is None or receiver_path is None:
        if required:
            raise MetricsError("required high-density caller/receiver artifacts are missing")
        return {"enabled": False, "required": False, "passed": True}

    caller = load_json(caller_path)
    receiver = load_json(receiver_path)
    caller_results = caller.get("results", {})
    receiver_results = receiver.get("results", {})
    definition = caller_results.get("scenario_definition", {})
    phases = definition.get("phases", []) if isinstance(definition, dict) else []
    acceptance = (
        definition.get("acceptance", {}) if isinstance(definition, dict) else {}
    )
    burst_cps = (
        phases[1].get("cps")
        if isinstance(phases, list)
        and len(phases) > 1
        and isinstance(phases[1], dict)
        else None
    )
    min_asr = acceptance.get("minAsr") if isinstance(acceptance, dict) else None
    rss_limit = (
        acceptance.get("maxRssGrowthMbPerHr")
        if isinstance(acceptance, dict)
        else None
    )
    caller_skip = (
        caller.get("diagnostics", {})
        .get("media_receive", {})
        .get("skip_audio_frame_delivery")
    )
    receiver_skip = (
        receiver.get("diagnostics", {})
        .get("media_receive", {})
        .get("skip_audio_frame_delivery")
    )
    errors = caller_results.get("errors")
    exact_errors = error_total(
        errors,
        (
            "answer_failed",
            "invite_send_failed",
            "media_setup_failed",
            "overload_rejected",
            "teardown_failed",
        ),
    )
    offered = caller_results.get("calls_offered")
    succeeded = caller_results.get("calls_succeeded")
    failed = caller_results.get("calls_failed")
    timeout_errors = errors.get("timeout") if isinstance(errors, dict) else None
    timeout_pct = (
        100.0 * timeout_errors / offered
        if isinstance(timeout_errors, int)
        and not isinstance(timeout_errors, bool)
        and isinstance(offered, int)
        and not isinstance(offered, bool)
        and offered > 0
        else None
    )
    timeout_accounting_matches = (
        isinstance(offered, int)
        and not isinstance(offered, bool)
        and offered > 0
        and isinstance(succeeded, int)
        and not isinstance(succeeded, bool)
        and isinstance(failed, int)
        and not isinstance(failed, bool)
        and isinstance(timeout_errors, int)
        and not isinstance(timeout_errors, bool)
        and offered == succeeded + failed
        and failed == timeout_errors
    )
    checks: list[dict[str, Any]] = []
    add_check(
        checks,
        "media_burst_cps",
        f"exactly {expected_cps:g}",
        burst_cps,
        finite_number(burst_cps) == expected_cps,
    )
    add_check(
        checks,
        "minimum_asr",
        f"exactly {expected_min_asr:g}",
        min_asr,
        finite_number(min_asr) == expected_min_asr,
    )
    add_check(
        checks,
        "rss_limit_mb_per_hr",
        f"exactly {expected_rss_limit:g}",
        rss_limit,
        finite_number(rss_limit) == expected_rss_limit,
    )
    add_check(
        checks,
        "full_audio_frame_delivery",
        "enabled for caller and receiver",
        {"caller_skip": caller_skip, "receiver_skip": receiver_skip},
        caller_skip is False and receiver_skip is False,
    )
    asr = finite_number(caller_results.get("asr"))
    add_check(
        checks,
        "asr",
        f">= {expected_min_asr:g}",
        asr,
        asr is not None and asr >= expected_min_asr,
    )
    add_check(
        checks,
        "timeout_failures",
        f"<= {(1.0 - expected_min_asr) * 100.0:g}% and exactly reconciled",
        {"count": timeout_errors, "percent": timeout_pct},
        timeout_accounting_matches
        and timeout_pct is not None
        and timeout_pct <= (1.0 - expected_min_asr) * 100.0 + 1e-9,
    )
    add_check(
        checks,
        "non_timeout_errors",
        "0",
        exact_errors,
        exact_errors == 0,
    )
    for metric, observed in (
        ("caller_retained_after_drain", caller_results.get("retained_objects_after_drain")),
        (
            "receiver_retained_after_drain",
            receiver_results.get("retained_objects_after_drain"),
        ),
        (
            "receiver_active_audio_receivers_after_drain",
            receiver_results.get("bob_active_audio_receivers"),
        ),
        (
            "caller_transaction_manager_after_drain",
            caller_results.get("transaction_manager_active_after_drain"),
        ),
        (
            "receiver_transaction_manager_after_drain",
            receiver_results.get("transaction_manager_active_after_drain"),
        ),
    ):
        add_check(checks, metric, "0", observed, observed == 0)
    delivered_frames = receiver_results.get("bob_received_frames")
    add_check(
        checks,
        "delivered_audio_frames",
        "> 0",
        delivered_frames,
        isinstance(delivered_frames, int) and delivered_frames > 0,
    )
    for metric, observed in (
        ("caller_rss_gate_mb_per_hr", caller_results.get("rss_gate_growth_mb_per_hr")),
        (
            "receiver_rss_gate_mb_per_hr",
            receiver_results.get("rss_gate_growth_mb_per_hr"),
        ),
    ):
        value = finite_number(observed)
        add_check(
            checks,
            metric,
            f"<= {expected_rss_limit:g}",
            value,
            value is not None and value <= expected_rss_limit,
        )

    return {
        "enabled": True,
        "required": required,
        "passed": all(check["passed"] for check in checks),
        "evidence": {
            "caller": caller_path.relative_to(perf_root).as_posix(),
            "receiver": receiver_path.relative_to(perf_root).as_posix(),
        },
        "policy": {
            "media_burst_cps": expected_cps,
            "minimum_asr": expected_min_asr,
            "rss_limit_mb_per_hr": expected_rss_limit,
            "full_audio_frame_delivery": True,
        },
        "observed": {
            "calls_offered": offered,
            "calls_succeeded": succeeded,
            "calls_failed": failed,
            "asr": asr,
            "errors": errors,
            "peak_active_calls": caller_results.get("active_call_occupancy", {}).get(
                "peak_active_calls"
            ),
            "peak_pending_setups": caller_results.get(
                "active_call_occupancy", {}
            ).get("peak_pending_setups"),
            "delivered_audio_frames": delivered_frames,
            "completed_audio_receivers": receiver_results.get(
                "bob_completed_audio_receivers"
            ),
            "caller_rss_gate_mb_per_hr": caller_results.get(
                "rss_gate_growth_mb_per_hr"
            ),
            "receiver_rss_gate_mb_per_hr": receiver_results.get(
                "rss_gate_growth_mb_per_hr"
            ),
        },
        "checks": checks,
    }


def monolithic_metrics(
    perf_root: pathlib.Path,
    expected_duration: int,
    expected_active_calls: int,
    expected_rss_limit: float,
    required: bool,
) -> dict[str, Any]:
    path = perf_root / "perf_soak_30min.json"
    if not path.is_file():
        if required:
            raise MetricsError("required monolithic-soak artifact is missing")
        return {"enabled": False, "required": False, "passed": True}
    report = load_json(path)
    results = report.get("results", {})
    errors = results.get("errors")
    all_errors = (
        sum(errors.values())
        if isinstance(errors, dict)
        and all(
            isinstance(value, int) and not isinstance(value, bool)
            for value in errors.values()
        )
        else None
    )
    gate = results.get("rss_gate", {})
    effective_rss_limit = (
        gate.get("effective_mb_per_hr") if isinstance(gate, dict) else None
    )
    checks: list[dict[str, Any]] = []
    for metric, requirement, observed, passed in (
        (
            "duration_secs",
            f"exactly {expected_duration}",
            results.get("duration_secs"),
            results.get("duration_secs") == expected_duration,
        ),
        (
            "active_calls_target",
            f"exactly {expected_active_calls}",
            results.get("active_calls_target"),
            results.get("active_calls_target") == expected_active_calls,
        ),
        (
            "rss_limit_mb_per_hr",
            f"exactly {expected_rss_limit:g}",
            effective_rss_limit,
            finite_number(effective_rss_limit) == expected_rss_limit,
        ),
        ("errors", "0", all_errors, all_errors == 0),
        (
            "retained_after_drain",
            "0",
            results.get("retained_objects_after_drain"),
            results.get("retained_objects_after_drain") == 0,
        ),
        (
            "active_audio_receivers_after_drain",
            "0",
            results.get("bob_active_audio_receivers"),
            results.get("bob_active_audio_receivers") == 0,
        ),
        (
            "transaction_manager_after_drain",
            "0",
            results.get("transaction_manager_active_after_drain"),
            results.get("transaction_manager_active_after_drain") == 0,
        ),
        (
            "transaction_runner_after_drain",
            "0",
            results.get("transaction_runner_active_after_drain"),
            results.get("transaction_runner_active_after_drain") == 0,
        ),
        (
            "controlled_drain_failed",
            "0",
            results.get("controlled_drain_failed"),
            results.get("controlled_drain_failed") == 0,
        ),
    ):
        add_check(checks, metric, requirement, observed, passed)
    offered = results.get("calls_offered")
    succeeded = results.get("calls_succeeded")
    add_check(
        checks,
        "call_completion",
        "all offered calls succeed",
        {"offered": offered, "succeeded": succeeded},
        isinstance(offered, int) and offered > 0 and succeeded == offered,
    )
    frames = results.get("bob_received_frames")
    add_check(
        checks,
        "delivered_audio_frames",
        "> 0",
        frames,
        isinstance(frames, int) and frames > 0,
    )
    rss_growth = finite_number(results.get("rss_gate_growth_mb_per_hr"))
    add_check(
        checks,
        "rss_gate_growth_mb_per_hr",
        f"<= {expected_rss_limit:g}",
        rss_growth,
        rss_growth is not None and rss_growth <= expected_rss_limit,
    )
    return {
        "enabled": True,
        "required": required,
        "passed": all(check["passed"] for check in checks),
        "evidence": path.relative_to(perf_root).as_posix(),
        "policy": {
            "duration_secs": expected_duration,
            "active_calls_target": expected_active_calls,
            "rss_limit_mb_per_hr": expected_rss_limit,
            "full_audio_frame_delivery": True,
        },
        "observed": {
            "calls_offered": offered,
            "calls_succeeded": succeeded,
            "asr": results.get("asr"),
            "errors": errors,
            "media_calls_held": results.get("media_calls_held"),
            "delivered_audio_frames": frames,
            "rss_gate_growth_mb_per_hr": rss_growth,
            "rss_gate_window": results.get("rss_gate_window"),
            "rss_active_tail_window_secs": results.get("rss_active_tail_window_secs"),
            "rss_post_drain_growth_mb_per_hr": results.get(
                "rss_post_drain_growth_mb_per_hr"
            ),
        },
        "checks": checks,
    }


def markdown(metrics: dict[str, Any]) -> str:
    lines = [
        "## Performance Gate Metrics",
        "",
        "This table is generated from the packaged JSON artifacts. `PASS` means",
        "the recorded policy and every related tracking metric agree.",
        "",
    ]
    for key, title in (
        ("high_density_media_burst", "High-density media burst"),
        ("monolithic_soak", "Monolithic soak"),
    ):
        section = metrics[key]
        lines.extend([f"### {title}", ""])
        if not section["enabled"]:
            lines.extend(["Not enabled in this run.", ""])
            continue
        evidence = section["evidence"]
        if isinstance(evidence, dict):
            evidence_text = ", ".join(f"`{value}`" for value in evidence.values())
        else:
            evidence_text = f"`{evidence}`"
        lines.extend(
            [
                f"- result: `{'PASS' if section['passed'] else 'FAIL'}`",
                f"- evidence: {evidence_text}",
                "",
                "| Metric | Requirement | Observed | Result |",
                "|--------|-------------|----------|--------|",
            ]
        )
        for check in section["checks"]:
            observed = json.dumps(check["observed"], sort_keys=True)
            lines.append(
                f"| {check['metric']} | {check['requirement']} | `{observed}` | "
                f"{'PASS' if check['passed'] else 'FAIL'} |"
            )
        lines.append("")
    return "\n".join(lines)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--perf-root", required=True)
    result.add_argument("--output-json", required=True)
    result.add_argument("--output-markdown", required=True)
    result.add_argument("--high-density-cps", type=float, default=160.0)
    result.add_argument("--high-density-min-asr", type=float, default=0.995)
    result.add_argument("--rss-limit-mb-per-hr", type=float, default=15.0)
    result.add_argument("--monolithic-duration-secs", type=int, default=3600)
    result.add_argument("--monolithic-active-calls", type=int, default=30)
    result.add_argument("--require-high-density", action="store_true")
    result.add_argument("--require-monolithic", action="store_true")
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        root = pathlib.Path(args.perf_root).resolve()
        metrics = {
            "schema": SCHEMA,
            "high_density_media_burst": high_density_metrics(
                root,
                args.high_density_cps,
                args.high_density_min_asr,
                args.rss_limit_mb_per_hr,
                args.require_high_density,
            ),
            "monolithic_soak": monolithic_metrics(
                root,
                args.monolithic_duration_secs,
                args.monolithic_active_calls,
                args.rss_limit_mb_per_hr,
                args.require_monolithic,
            ),
        }
        metrics["passed"] = all(
            metrics[key]["passed"]
            for key in ("high_density_media_burst", "monolithic_soak")
        )
        output_json = pathlib.Path(args.output_json)
        output_markdown = pathlib.Path(args.output_markdown)
        output_json.write_text(
            json.dumps(metrics, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        output_markdown.write_text(markdown(metrics), encoding="utf-8")
        if not metrics["passed"]:
            raise MetricsError("one or more performance gate metrics failed")
    except MetricsError as error:
        print(f"beta performance gate metrics: FAIL: {error}")
        return 1
    print("beta performance gate metrics: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
