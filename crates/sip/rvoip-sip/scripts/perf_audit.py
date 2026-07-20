#!/usr/bin/env python3
"""Compare two beta-gate perf-results directories and flag performance regressions.

Matches perf JSON files by their path relative to each perf-results root (so a
scenario is compared against the same scenario), extracts a fixed set of
comparable metrics, computes per-metric deltas with a direction and tolerance,
writes a Markdown report, and optionally exits non-zero when a regression beyond
tolerance is found.

Usage:
    perf_audit.py --baseline <dir> --current <dir> --out <file.md>
                  [--tolerance-pct 15] [--latency-tolerance-pct 25]
                  [--fail-on-regression]

Exit status: 0 if no regression (or --fail-on-regression not set); 1 if a
regression beyond tolerance was found AND --fail-on-regression was passed; 2 on
usage/IO/coverage error or when conditioning/window identity is non-comparable.
The Markdown report and stdout summary are always produced.

Stdlib only — safe to invoke from beta_gate.sh with the system python3.
"""

import argparse
import glob
import json
import os
import pathlib
import sys

# Scalar metrics compared per scenario file.
#   key:        dotted path into the JSON
#   label:      human label
#   direction:  "lower_worse" (throughput) or "higher_worse" (latency/RSS/CPU)
#   abs_floor:  if the baseline value is below this, skip the % rule and instead
#               only flag when the current value ALSO exceeds abs_floor and has
#               more than doubled (guards near-zero baselines from % blow-ups).
#               None => always use the % rule.
#   gate:       True => can count as a regression; False => reported only.
SCALAR_METRICS = [
    ("results.achieved_cps", "achieved CPS", "lower_worse", None, True),
    ("results.cps_per_core", "CPS/core", "lower_worse", None, True),
    ("results.asr", "ASR (answer-seizure ratio)", "lower_worse", None, True),
    ("results.ner", "NER (non-error ratio)", "lower_worse", None, True),
    ("resources.peak_rss_mb", "peak RSS (MB)", "higher_worse", None, True),
    ("resources.rss_active_growth_mb_per_min", "RSS active-load growth (MB/min)", "higher_worse", 0.5, True),
    ("resources.avg_cpu_pct", "avg CPU (%)", "higher_worse", None, False),  # too noisy to gate
]

# Success-ratio metrics get a tighter tolerance than throughput (they should sit
# at ~1.0; any real drop matters).
STRICT_TOLERANCE = {"results.asr": 2.0, "results.ner": 2.0}

LATENCY_PERCENTILES = ["p50", "p95", "p99"]


def dotted_get(obj, path):
    cur = obj
    for part in path.split("."):
        if not isinstance(cur, dict) or part not in cur:
            return None
        cur = cur[part]
    return cur if isinstance(cur, (int, float)) else None


def load_json(path):
    try:
        with open(path) as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return None


def index_dir(root):
    """Map relative-path -> parsed JSON for every *.json under root."""
    out = {}
    for path in glob.glob(os.path.join(root, "**", "*.json"), recursive=True):
        data = load_json(path)
        if isinstance(data, dict):
            out[os.path.relpath(path, root)] = data
    return out


def _normalise_conditioning(points):
    normalised = []
    if not isinstance(points, list):
        return None
    for point in points:
        if not isinstance(point, dict):
            return None
        target = point.get("target_cps")
        offered = point.get("calls_offered")
        succeeded = point.get("calls_succeeded")
        if not isinstance(target, (int, float)) or not isinstance(offered, int) or not isinstance(succeeded, int):
            return None
        normalised.append(
            {
                "target_cps": float(target),
                "calls_offered": offered,
                "calls_succeeded": succeeded,
            }
        )
    return normalised


def explicit_measurement_identity(report):
    identity = report.get("diagnostics", {}).get("measurement_identity")
    if not isinstance(identity, dict):
        return None
    conditioning = _normalise_conditioning(
        (identity.get("conditioning") or {}).get("points")
    )
    window = identity.get("resource_window")
    sweep = identity.get("sweep_points_cps")
    if conditioning is None or not isinstance(window, dict) or not isinstance(sweep, list):
        return {"invalid": "explicit measurement identity is incomplete"}
    try:
        sweep = [float(point) for point in sweep]
        target = float(identity["measured_point_cps"])
    except (KeyError, TypeError, ValueError):
        return {"invalid": "explicit sweep/target identity is invalid"}
    return {
        "peer_lifecycle": identity.get("peer_lifecycle"),
        "sweep_points_cps": sweep,
        "measured_point_cps": target,
        "conditioning": conditioning,
        "resource_window": {
            "kind": window.get("kind"),
            "start_phase": window.get("start_phase"),
            "end_phase": window.get("end_phase"),
            "sample_interval_ms": window.get("sample_interval_ms"),
        },
        "source": "explicit",
    }


def legacy_sweep_measurement_identity(index, relative_path, report):
    """Infer the reviewed pre-identity sweep shape from its complete tree.

    Old reports did not record phase markers, but the per-point reports and
    aggregate sweep prove call conditioning and shared-peer ordering. Their
    historical `rss_tail_growth` was sampled from point start until calls
    drained; it is therefore the predecessor of the explicit active window.
    """
    path = pathlib.PurePath(relative_path)
    if path.suffix != ".json" or path.stem.startswith("_"):
        return None
    scenario = report.get("scenario")
    if not isinstance(scenario, str) or not scenario.startswith("perf_call_setup_cps"):
        return None
    aggregate = index.get(str(path.parent / "_sweep.json"))
    points = None
    if isinstance(aggregate, dict):
        points = (aggregate.get("sweep_summary") or {}).get("points")
    if not isinstance(points, list):
        # Legacy single-point runs used a flat `<scenario>.json` file. There
        # was no prior point and therefore no conditioning ambiguity.
        target = report.get("load", {}).get("target_cps")
        if path.parent != pathlib.PurePath(".") or not isinstance(target, (int, float)):
            return None
        points = [float(target)]
    try:
        points = [float(point) for point in points]
        target = float(report.get("load", {}).get("target_cps"))
        point_index = points.index(target)
    except (TypeError, ValueError):
        return None
    conditioning = []
    for point in points[:point_index]:
        sibling = index.get(str(path.parent / f"{point:g}.json"))
        if not isinstance(sibling, dict):
            return None
        results = sibling.get("results") or {}
        offered = results.get("calls_offered")
        succeeded = results.get("calls_succeeded")
        if not isinstance(offered, int) or not isinstance(succeeded, int):
            return None
        conditioning.append(
            {
                "target_cps": point,
                "calls_offered": offered,
                "calls_succeeded": succeeded,
            }
        )
    resources = report.get("resources") or {}
    if not isinstance(resources.get("rss_sample_count"), int):
        return None
    return {
        "peer_lifecycle": "shared_for_entire_sweep",
        "sweep_points_cps": points,
        "measured_point_cps": target,
        "conditioning": conditioning,
        "resource_window": {
            "kind": "active_load",
            "start_phase": "point_start",
            "end_phase": "calls_drained",
            "sample_interval_ms": 500,
        },
        "source": "legacy_complete_sweep_inference",
    }


def measurement_identity(index, relative_path, report):
    return explicit_measurement_identity(report) or legacy_sweep_measurement_identity(
        index, relative_path, report
    )


def identity_comparison_value(identity):
    if not isinstance(identity, dict) or "invalid" in identity:
        return identity
    return {key: value for key, value in identity.items() if key != "source"}


def explicit_window_coverage_issue(identity, report):
    if not isinstance(identity, dict) or identity.get("source") != "explicit":
        return None
    resources = report.get("resources") or {}
    active = (resources.get("rss_windows") or {}).get("active_load")
    if not isinstance(active, dict):
        return "explicit identity has no resources.rss_windows.active_load evidence"
    if active.get("complete") is not True:
        return "explicit active-load resource window is incomplete"
    if not isinstance(active.get("sample_count"), int) or active["sample_count"] < 2:
        return "explicit active-load resource window has fewer than two samples"
    coverage = active.get("actual_coverage_secs")
    if not isinstance(coverage, (int, float)) or coverage <= 0:
        return "explicit active-load resource window has no positive actual coverage"
    if dotted_get(report, "resources.rss_active_growth_mb_per_min") is None:
        return "explicit active-load resource slope is missing"
    return None


def resource_window_evidence(report):
    resources = report.get("resources") or {}
    active = (resources.get("rss_windows") or {}).get("active_load") or {}
    has_explicit_tail_coverage = "rss_tail_window_requested_secs" in resources
    return {
        "sample_count": active.get("sample_count", resources.get("rss_sample_count")),
        "actual_coverage_secs": active.get("actual_coverage_secs"),
        "complete": active.get("complete"),
        "tail_requested_secs": resources.get(
            "rss_tail_window_requested_secs", resources.get("rss_tail_window_secs")
        ),
        "tail_actual_secs": (
            resources.get("rss_tail_window_secs")
            if has_explicit_tail_coverage
            else None
        ),
        "legacy_reported_tail_window_secs": (
            None
            if has_explicit_tail_coverage
            else resources.get("rss_tail_window_secs")
        ),
        "tail_sample_count": resources.get("rss_tail_sample_count", resources.get("rss_sample_count")),
    }


def pct_change(base, cur):
    if base == 0:
        return float("inf") if cur else 0.0
    return (cur - base) / abs(base) * 100.0


def is_regression(base, cur, direction, tol_pct, abs_floor):
    """Return (regressed: bool, delta_pct: float)."""
    delta = pct_change(base, cur)
    if abs_floor is not None and abs(base) < abs_floor:
        # Near-zero baseline: only a real, above-floor doubling counts.
        regressed = (
            direction == "higher_worse" and cur > abs_floor and cur > base * 2.0
        ) or (
            direction == "lower_worse" and base > abs_floor and cur < base / 2.0
        )
        return regressed, delta
    if direction == "higher_worse":
        return (delta > tol_pct), delta
    return (delta < -tol_pct), delta


def metric_get(report, key, identity):
    value = dotted_get(report, key)
    if value is not None:
        return value
    if (
        key == "resources.rss_active_growth_mb_per_min"
        and isinstance(identity, dict)
        and identity.get("source") == "legacy_complete_sweep_inference"
    ):
        return dotted_get(report, "resources.rss_tail_growth_mb_per_min")
    return None


def collect_metrics(base, cur, base_identity, cur_identity, throughput_tol, latency_tol):
    """Yield comparison rows for one scenario (only metrics present on both sides)."""
    rows = []
    for key, label, direction, abs_floor, gate in SCALAR_METRICS:
        b = metric_get(base, key, base_identity)
        c = metric_get(cur, key, cur_identity)
        if b is None or c is None:
            continue
        tol = STRICT_TOLERANCE.get(key, throughput_tol)
        regressed, delta = is_regression(b, c, direction, tol, abs_floor)
        rows.append((label, b, c, delta, gate and regressed, gate))
    # Latency percentiles: latency_ns.<name>.<pXX> (higher is worse).
    b_lat = base.get("latency_ns") if isinstance(base, dict) else None
    c_lat = cur.get("latency_ns") if isinstance(cur, dict) else None
    if isinstance(b_lat, dict) and isinstance(c_lat, dict):
        for name in sorted(set(b_lat) & set(c_lat)):
            for pct in LATENCY_PERCENTILES:
                b = dotted_get(b_lat[name], pct)
                c = dotted_get(c_lat[name], pct)
                if b is None or c is None:
                    continue
                # Sub-millisecond latencies are timer-quantization noise; don't
                # gate below a 1 ms floor (values here are in nanoseconds).
                regressed, delta = is_regression(b, c, "higher_worse", latency_tol, 1_000_000.0)
                rows.append((f"{name} {pct} (ms)", b / 1e6, c / 1e6, delta, regressed, True))
    return rows


def fmt(v):
    if isinstance(v, float):
        return f"{v:.3f}".rstrip("0").rstrip(".") if v else "0"
    return str(v)


def main():
    ap = argparse.ArgumentParser(description="Beta-gate perf regression audit.")
    ap.add_argument("--baseline", required=True, help="previous run's perf-results dir")
    ap.add_argument("--current", required=True, help="current run's perf-results dir")
    ap.add_argument("--out", required=True, help="Markdown report output path")
    ap.add_argument("--tolerance-pct", type=float, default=15.0,
                    help="allowed throughput/RSS change before flagging (default 15)")
    ap.add_argument("--latency-tolerance-pct", type=float, default=25.0,
                    help="allowed latency increase before flagging (default 25)")
    ap.add_argument("--fail-on-regression", action="store_true",
                    help="exit non-zero if any gated regression is found")
    args = ap.parse_args()

    if not os.path.isdir(args.current):
        print(f"perf_audit: no current perf-results dir: {args.current}", file=sys.stderr)
        return 2

    base_idx = index_dir(args.baseline)
    cur_idx = index_dir(args.current)
    if not base_idx:
        print(f"perf_audit: baseline has no perf JSON: {args.baseline}", file=sys.stderr)
        with open(args.out, "w") as fh:
            fh.write("# Perf Regression Audit\n\nstatus: COVERAGE_ERROR\n\n"
                     f"No perf JSON exists in baseline `{args.baseline}`.\n")
        print("perf_audit: COVERAGE_ERROR (baseline is empty)", file=sys.stderr)
        return 2

    def git_rev(idx):
        for d in idx.values():
            r = d.get("environment", {}).get("git_rev")
            if r:
                return r
        return "?"

    shared = sorted(set(base_idx) & set(cur_idx))
    only_cur = sorted(set(cur_idx) - set(base_idx))
    only_base = sorted(set(base_idx) - set(cur_idx))

    if not shared:
        lines = [
            "# Perf Regression Audit",
            "",
            "status: COVERAGE_ERROR",
            f"baseline: `{args.baseline}`",
            f"current:  `{args.current}`",
            "",
            "The two result trees contain no JSON files at matching relative paths.",
            "This is a coverage error, not a successful performance comparison.",
            "",
        ]
        try:
            with open(args.out, "w") as fh:
                fh.write("\n".join(lines))
        except OSError as exc:
            print(f"perf_audit: cannot write {args.out}: {exc}", file=sys.stderr)
            return 2
        print("perf_audit: COVERAGE_ERROR (no shared scenario paths)", file=sys.stderr)
        print(f"perf_audit: report written to {args.out}", file=sys.stderr)
        return 2

    all_rows = []       # (scenario, label, base, cur, delta, regressed, gated)
    regressions = []
    noncomparable = []
    comparison_evidence = []
    for rel in shared:
        base_identity = measurement_identity(base_idx, rel, base_idx[rel])
        cur_identity = measurement_identity(cur_idx, rel, cur_idx[rel])
        base_coverage_issue = explicit_window_coverage_issue(base_identity, base_idx[rel])
        cur_coverage_issue = explicit_window_coverage_issue(cur_identity, cur_idx[rel])
        strict_identity = base_identity is not None or cur_identity is not None
        if strict_identity and (
            base_coverage_issue
            or cur_coverage_issue
            or identity_comparison_value(base_identity) != identity_comparison_value(cur_identity)
        ):
            if base_coverage_issue:
                base_identity = {"identity": base_identity, "coverage_error": base_coverage_issue}
            if cur_coverage_issue:
                cur_identity = {"identity": cur_identity, "coverage_error": cur_coverage_issue}
            noncomparable.append(
                (
                    rel,
                    base_identity,
                    cur_identity,
                    resource_window_evidence(base_idx[rel]),
                    resource_window_evidence(cur_idx[rel]),
                )
            )
            continue
        if strict_identity:
            comparison_evidence.append(
                (
                    rel,
                    base_identity,
                    cur_identity,
                    resource_window_evidence(base_idx[rel]),
                    resource_window_evidence(cur_idx[rel]),
                )
            )
        for label, b, c, delta, regressed, gated in collect_metrics(
            base_idx[rel],
            cur_idx[rel],
            base_identity,
            cur_identity,
            args.tolerance_pct,
            args.latency_tolerance_pct,
        ):
            scen = base_idx[rel].get("scenario") or rel
            all_rows.append((scen, label, b, c, delta, regressed, gated))
            if regressed:
                regressions.append((scen, label, b, c, delta))

    if not all_rows and not noncomparable:
        lines = [
            "# Perf Regression Audit",
            "",
            "status: COVERAGE_ERROR",
            f"baseline: `{args.baseline}`",
            f"current:  `{args.current}`",
            "",
            "The result trees share JSON paths, but none contains comparable metrics.",
            "This is a coverage error, not a successful performance comparison.",
            "",
        ]
        try:
            with open(args.out, "w") as fh:
                fh.write("\n".join(lines))
        except OSError as exc:
            print(f"perf_audit: cannot write {args.out}: {exc}", file=sys.stderr)
            return 2
        print("perf_audit: COVERAGE_ERROR (zero comparable metrics)", file=sys.stderr)
        print(f"perf_audit: report written to {args.out}", file=sys.stderr)
        return 2

    status = "NON_COMPARABLE" if noncomparable else ("REGRESSION" if regressions else "OK")
    lines = []
    lines.append("# Perf Regression Audit")
    lines.append("")
    lines.append(f"status: {status}")
    lines.append(f"baseline: `{args.baseline}` (git_rev {git_rev(base_idx)})")
    lines.append(f"current:  `{args.current}` (git_rev {git_rev(cur_idx)})")
    lines.append(f"tolerances: throughput/RSS ±{args.tolerance_pct:g}%, "
                 f"latency +{args.latency_tolerance_pct:g}%, ASR/NER +2%")
    lines.append(f"scenarios compared: {len(shared)}"
                 + (f" (baseline-only: {len(only_base)}, current-only: {len(only_cur)})"
                    if (only_base or only_cur) else ""))
    lines.append("")

    if noncomparable:
        lines.append(f"## Refused comparisons ({len(noncomparable)})")
        lines.append("")
        lines.append(
            "These scenarios have different or incomplete conditioning/resource-window identities. "
            "No scalar comparison was performed; this is not a passing audit."
        )
        lines.append("")
        for rel, base_identity, cur_identity, base_evidence, cur_evidence in noncomparable:
            lines.append(f"### `{rel}`")
            lines.append("")
            lines.append(f"- baseline identity: `{json.dumps(base_identity, sort_keys=True)}`")
            lines.append(f"- current identity: `{json.dumps(cur_identity, sort_keys=True)}`")
            lines.append(f"- baseline window evidence: `{json.dumps(base_evidence, sort_keys=True)}`")
            lines.append(f"- current window evidence: `{json.dumps(cur_evidence, sort_keys=True)}`")
            lines.append("")

    if comparison_evidence:
        lines.append("## Measurement identity and coverage")
        lines.append("")
        for rel, base_identity, cur_identity, base_evidence, cur_evidence in comparison_evidence:
            lines.append(f"- `{rel}`")
            lines.append(
                f"  - conditioning/window identity: comparable "
                f"(baseline `{base_identity.get('source')}`, current `{cur_identity.get('source')}`)"
            )
            lines.append(f"  - baseline coverage: `{json.dumps(base_evidence, sort_keys=True)}`")
            lines.append(f"  - current coverage: `{json.dumps(cur_evidence, sort_keys=True)}`")
        lines.append("")

    if regressions:
        lines.append(f"## Regressions ({len(regressions)})")
        lines.append("")
        lines.append("| scenario | metric | baseline | current | delta |")
        lines.append("|---|---|---|---|---|")
        for scen, label, b, c, delta in regressions:
            lines.append(f"| {scen} | {label} | {fmt(b)} | {fmt(c)} | {delta:+.1f}% |")
        lines.append("")

    lines.append("## All metrics")
    lines.append("")
    lines.append("| scenario | metric | baseline | current | delta | verdict |")
    lines.append("|---|---|---|---|---|---|")
    for scen, label, b, c, delta, regressed, gated in all_rows:
        verdict = "REGRESSION" if regressed else ("ok" if gated else "info")
        lines.append(f"| {scen} | {label} | {fmt(b)} | {fmt(c)} | {delta:+.1f}% | {verdict} |")
    lines.append("")

    if only_base or only_cur:
        lines.append("## Coverage notes")
        lines.append("")
        for rel in only_base:
            lines.append(f"- baseline-only (not in current): `{rel}`")
        for rel in only_cur:
            lines.append(f"- current-only (new since baseline): `{rel}`")
        lines.append("")

    report = "\n".join(lines)
    try:
        with open(args.out, "w") as fh:
            fh.write(report)
    except OSError as exc:
        print(f"perf_audit: cannot write {args.out}: {exc}", file=sys.stderr)
        return 2

    # Console summary (captured in the gate log).
    print(f"perf_audit: status={status}  scenarios={len(shared)}  "
          f"metrics={len(all_rows)}  regressions={len(regressions)}")
    print(f"perf_audit: baseline git_rev {git_rev(base_idx)} -> current {git_rev(cur_idx)}")
    for scen, label, b, c, delta in regressions:
        print(f"perf_audit:  ⚠ REGRESSION {scen} / {label}: {fmt(b)} -> {fmt(c)} ({delta:+.1f}%)")
    print(f"perf_audit: report written to {args.out}")

    if noncomparable:
        print(
            f"perf_audit: NON_COMPARABLE ({len(noncomparable)} conditioning/window identity mismatch(es))",
            file=sys.stderr,
        )
        return 2
    if regressions and args.fail_on_regression:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
