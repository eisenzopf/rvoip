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
usage/IO error. The Markdown report and stdout summary are always produced.

Stdlib only — safe to invoke from beta_gate.sh with the system python3.
"""

import argparse
import glob
import json
import os
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
    ("resources.rss_tail_growth_mb_per_min", "RSS tail growth (MB/min)", "higher_worse", 0.5, True),
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


def collect_metrics(base, cur, throughput_tol, latency_tol):
    """Yield comparison rows for one scenario (only metrics present on both sides)."""
    rows = []
    for key, label, direction, abs_floor, gate in SCALAR_METRICS:
        b = dotted_get(base, key)
        c = dotted_get(cur, key)
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
        # Not a regression — nothing to compare against.
        with open(args.out, "w") as fh:
            fh.write("# Perf Regression Audit\n\nstatus: NO_BASELINE\n\n"
                     f"No comparable perf JSON in baseline `{args.baseline}`.\n")
        print("perf_audit: NO_BASELINE (nothing to compare)")
        return 0

    def git_rev(idx):
        for d in idx.values():
            r = d.get("environment", {}).get("git_rev")
            if r:
                return r
        return "?"

    shared = sorted(set(base_idx) & set(cur_idx))
    only_cur = sorted(set(cur_idx) - set(base_idx))
    only_base = sorted(set(base_idx) - set(cur_idx))

    all_rows = []       # (scenario, label, base, cur, delta, regressed, gated)
    regressions = []
    for rel in shared:
        for label, b, c, delta, regressed, gated in collect_metrics(
            base_idx[rel], cur_idx[rel], args.tolerance_pct, args.latency_tolerance_pct
        ):
            scen = base_idx[rel].get("scenario") or rel
            all_rows.append((scen, label, b, c, delta, regressed, gated))
            if regressed:
                regressions.append((scen, label, b, c, delta))

    status = "REGRESSION" if regressions else "OK"
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

    if regressions and args.fail_on_regression:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
