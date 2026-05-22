#!/usr/bin/env python3
"""Parse sipp stat CSVs and emit a markdown comparison table.

Usage:
    analyze.py <results_dir> [output.md]

Each CSV is one run; we take the last data row (cumulative final
totals) and pull the fields that matter for a head-to-head
comparison: success rate, achieved CPS, INVITE→200 OK latency,
retransmissions.
"""

from __future__ import annotations

import csv
import os
import re
import sys
from collections import Counter
from pathlib import Path


RESPONSE_BUCKETS_MS = [10, 25, 50, 75, 100, 150, 200, 300, 500, 1000, 2000]
RESPONSE_BUCKET_LABELS = [f"<{upper}" for upper in RESPONSE_BUCKETS_MS] + [
    f">={RESPONSE_BUCKETS_MS[-1]}"
]


def parse_elapsed(elapsed: str) -> float:
    """Convert HH:MM:SS or HH:MM:SS:uuuuuu into seconds."""
    parts = elapsed.split(":")
    if len(parts) == 3:
        h, m, s = parts
        return int(h) * 3600 + int(m) * 60 + float(s)
    if len(parts) == 4:
        h, m, s, us = parts
        return int(h) * 3600 + int(m) * 60 + int(s) + int(us) / 1_000_000
    return float(elapsed)


def parse_int(value) -> int:
    return int(value or 0)


def response_buckets(by_name: dict[str, str], prefix: str) -> dict[str, int]:
    buckets = {
        f"<{upper}": parse_int(by_name.get(f"{prefix}_<{upper}"))
        for upper in RESPONSE_BUCKETS_MS
    }
    buckets[f">={RESPONSE_BUCKETS_MS[-1]}"] = parse_int(
        by_name.get(f"{prefix}_>={RESPONSE_BUCKETS_MS[-1]}")
    )
    return buckets


def percentile_from_buckets(buckets: dict[str, int], quantile: float) -> str:
    ordered = [(label, buckets.get(label, 0)) for label in RESPONSE_BUCKET_LABELS]
    sample_count = sum(count for _, count in ordered)
    if sample_count == 0:
        return "n/a"

    target = max(1, int(sample_count * quantile + 0.999999))
    seen = 0
    for label, count in ordered:
        seen += count
        if seen >= target:
            return label
    return ordered[-1][0]


def parse_csv(path: Path) -> dict:
    with path.open() as f:
        reader = csv.reader(f, delimiter=";")
        rows = list(reader)
    if not rows:
        return {}
    header = rows[0]
    last = rows[-1] if len(rows) > 1 else rows[0]
    by_name = dict(zip(header, last))

    elapsed_s = parse_elapsed(by_name.get("ElapsedTime(C)", "0"))
    total = parse_int(by_name.get("TotalCallCreated"))
    success = parse_int(by_name.get("SuccessfulCall(C)"))
    failed = parse_int(by_name.get("FailedCall(C)"))
    current = parse_int(by_name.get("CurrentCall"))
    retrans = parse_int(by_name.get("Retransmissions(C)"))
    target_rate = float(by_name.get("TargetRate", 0) or 0)

    # ResponseTime1 is INVITE → 200 OK; HH:MM:SS:uuuuuu.
    rt_ms = parse_elapsed(by_name.get("ResponseTime1(C)", "0")) * 1000.0
    rt_stddev_ms = parse_elapsed(by_name.get("ResponseTime1StDev(C)", "0")) * 1000.0
    call_len_ms = parse_elapsed(by_name.get("CallLength(C)", "0")) * 1000.0
    rt_buckets = response_buckets(by_name, "ResponseTimeRepartition1")
    rt_p95 = percentile_from_buckets(rt_buckets, 0.95)
    rt_p99 = percentile_from_buckets(rt_buckets, 0.99)
    rt_p99_9 = percentile_from_buckets(rt_buckets, 0.999)
    sample_count = sum(rt_buckets.values()) or success or total

    achieved_cps = (success / elapsed_s) if elapsed_s > 0 else 0.0
    success_rate = (success / total * 100.0) if total > 0 else 0.0

    return {
        "shards": 1,
        "elapsed_s": elapsed_s,
        "total": total,
        "success": success,
        "failed": failed,
        "current": current,
        "retrans": retrans,
        "target_rate": target_rate,
        "achieved_cps": achieved_cps,
        "success_rate": success_rate,
        "rt_ms": rt_ms,
        "rt_stddev_ms": rt_stddev_ms,
        "rt_p95": rt_p95,
        "rt_p99": rt_p99,
        "rt_p99_9": rt_p99_9,
        "rt_buckets": rt_buckets,
        "sample_count": sample_count,
        "call_len_ms": call_len_ms,
    }


def weighted_avg(runs: list[dict], key: str) -> float:
    total_weight = sum(run.get("sample_count", 0) for run in runs)
    if total_weight <= 0:
        return 0.0
    return sum(run.get(key, 0.0) * run.get("sample_count", 0) for run in runs) / total_weight


def aggregate_runs(runs: list[dict]) -> dict:
    if len(runs) == 1:
        return runs[0]

    elapsed_s = max(run["elapsed_s"] for run in runs)
    total = sum(run["total"] for run in runs)
    success = sum(run["success"] for run in runs)
    failed = sum(run["failed"] for run in runs)
    current = sum(run["current"] for run in runs)
    retrans = sum(run["retrans"] for run in runs)
    target_rate = sum(run["target_rate"] for run in runs)
    buckets = Counter()
    for run in runs:
        buckets.update(run.get("rt_buckets", {}))

    achieved_cps = (success / elapsed_s) if elapsed_s > 0 else 0.0
    success_rate = (success / total * 100.0) if total > 0 else 0.0

    return {
        "shards": len(runs),
        "elapsed_s": elapsed_s,
        "total": total,
        "success": success,
        "failed": failed,
        "current": current,
        "retrans": retrans,
        "target_rate": target_rate,
        "achieved_cps": achieved_cps,
        "success_rate": success_rate,
        "rt_ms": weighted_avg(runs, "rt_ms"),
        "rt_stddev_ms": weighted_avg(runs, "rt_stddev_ms"),
        "rt_p95": percentile_from_buckets(dict(buckets), 0.95),
        "rt_p99": percentile_from_buckets(dict(buckets), 0.99),
        "rt_p99_9": percentile_from_buckets(dict(buckets), 0.999),
        "rt_buckets": dict(buckets),
        "sample_count": sum(run.get("sample_count", 0) for run in runs),
        "call_len_ms": weighted_avg(runs, "call_len_ms"),
    }


def collect(results_dir: Path) -> dict:
    pattern = re.compile(r"^(?P<tag>.+)_(?P<cps>\d+)cps(?:_s(?P<shard>\d+))?$")
    runs: dict[str, dict[int, list[dict]]] = {}
    for p in sorted(results_dir.iterdir()):
        m = pattern.match(p.name)
        if not m:
            continue
        tag = m.group("tag")
        cps = int(m.group("cps"))
        parsed = parse_csv(p)
        if parsed:
            runs.setdefault(tag, {}).setdefault(cps, []).append(parsed)
    out: dict[str, dict[int, dict]] = {}
    for tag, by_cps in runs.items():
        for cps, cps_runs in by_cps.items():
            out.setdefault(tag, {})[cps] = aggregate_runs(cps_runs)
    return out


def render(data: dict) -> str:
    tags = sorted(data.keys())
    all_cps = sorted({c for t in tags for c in data[t].keys()})

    lines = ["# rvoip vs Asterisk — sipp-driven performance comparison", ""]
    lines.append(
        "Driver: SIPp 3.7.3 from a sidecar Alpine container on the asterisk "
        "docker bridge. Each scenario drives `uac_perf.xml` (INVITE → 200 → "
        "ACK → 100 ms pause → BYE → 200). High-CPS points may be split across "
        "parallel SIPp shards and aggregated by target."
    )
    lines.append("")
    lines.append("## Summary table")
    lines.append("")
    header = [
        "Target",
        "Target CPS",
        "Shards",
        "Total calls",
        "Success",
        "Success %",
        "Achieved CPS",
        "Avg RTT (ms)",
        "P95 RTT (ms)",
        "P99 RTT (ms)",
        "Retrans",
    ]
    lines.append("| " + " | ".join(header) + " |")
    lines.append("|" + "|".join(["---"] * len(header)) + "|")
    for tag in tags:
        for cps in all_cps:
            d = data[tag].get(cps)
            if not d:
                continue
            lines.append(
                f"| {tag} | {cps} | {d['shards']} | {d['total']} | {d['success']} | "
                f"{d['success_rate']:.1f}% | {d['achieved_cps']:.1f} | "
                f"{d['rt_ms']:.1f} | {d['rt_p95']} | {d['rt_p99']} | {d['retrans']} |"
            )
    lines.append("")

    # Per-target detail
    for tag in tags:
        lines.append(f"## {tag}")
        lines.append("")
        for cps in all_cps:
            d = data[tag].get(cps)
            if not d:
                continue
            lines.append(f"### {tag} @ {cps} CPS")
            lines.append("")
            lines.append(f"- Elapsed: **{d['elapsed_s']:.1f} s**")
            lines.append(
                f"- Calls: {d['success']} success / {d['failed']} failed / "
                f"{d['total']} total ({d['success_rate']:.1f}% success)"
            )
            lines.append(f"- Achieved CPS: **{d['achieved_cps']:.1f}** (target {cps})")
            lines.append(
                f"- INVITE→200 OK latency: avg **{d['rt_ms']:.1f} ms** "
                f"(σ {d['rt_stddev_ms']:.1f} ms), "
                f"p95 **{d['rt_p95']} ms**, p99 **{d['rt_p99']} ms**, "
                f"p99.9 **{d['rt_p99_9']} ms**"
            )
            lines.append(f"- Call length: {d['call_len_ms']:.1f} ms")
            lines.append(f"- SIPp shards: {d['shards']}")
            lines.append(f"- Retransmissions: {d['retrans']}")
            if d["current"] > 0:
                lines.append(
                    f"- WARNING: {d['current']} calls still in-flight at sweep end "
                    f"(SUT stalled mid-run)"
                )
            lines.append("")

    return "\n".join(lines)


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    results_dir = Path(sys.argv[1])
    if not results_dir.is_dir():
        print(f"results dir not found: {results_dir}", file=sys.stderr)
        return 2
    data = collect(results_dir)
    md = render(data)
    if len(sys.argv) >= 3:
        out = Path(sys.argv[2])
        out.write_text(md)
        print(f"wrote {out}")
    else:
        print(md)
    return 0


if __name__ == "__main__":
    sys.exit(main())
