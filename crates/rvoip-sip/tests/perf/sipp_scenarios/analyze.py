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
from pathlib import Path


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
    total = int(by_name.get("TotalCallCreated", 0) or 0)
    success = int(by_name.get("SuccessfulCall(C)", 0) or 0)
    failed = int(by_name.get("FailedCall(C)", 0) or 0)
    current = int(by_name.get("CurrentCall", 0) or 0)
    retrans = int(by_name.get("Retransmissions(C)", 0) or 0)
    target_rate = float(by_name.get("TargetRate", 0) or 0)

    # ResponseTime1 is INVITE → 200 OK; HH:MM:SS:uuuuuu.
    rt_ms = parse_elapsed(by_name.get("ResponseTime1(C)", "0")) * 1000.0
    rt_stddev_ms = parse_elapsed(by_name.get("ResponseTime1StDev(C)", "0")) * 1000.0
    call_len_ms = parse_elapsed(by_name.get("CallLength(C)", "0")) * 1000.0

    achieved_cps = (success / elapsed_s) if elapsed_s > 0 else 0.0
    success_rate = (success / total * 100.0) if total > 0 else 0.0

    return {
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
        "call_len_ms": call_len_ms,
    }


def collect(results_dir: Path) -> dict:
    pattern = re.compile(r"^(?P<tag>[a-z_]+)_(?P<cps>\d+)cps$")
    out: dict[str, dict[int, dict]] = {}
    for p in sorted(results_dir.iterdir()):
        m = pattern.match(p.name)
        if not m:
            continue
        tag = m.group("tag")
        cps = int(m.group("cps"))
        out.setdefault(tag, {})[cps] = parse_csv(p)
    return out


def render(data: dict) -> str:
    tags = sorted(data.keys())
    all_cps = sorted({c for t in tags for c in data[t].keys()})

    lines = ["# rvoip vs Asterisk — sipp-driven performance comparison", ""]
    lines.append(
        "Driver: SIPp 3.7.3 from a sidecar Alpine container on the asterisk "
        "docker bridge. Each scenario drives `uac_perf.xml` (INVITE → 200 → "
        "ACK → 100 ms pause → BYE → 200) at the target CPS for "
        "15 s of steady load (`15 × CPS` total calls)."
    )
    lines.append("")
    lines.append("## Summary table")
    lines.append("")
    header = ["Target", "Target CPS", "Total calls", "Success",
              "Success %", "Achieved CPS", "Avg RTT (ms)", "Retrans"]
    lines.append("| " + " | ".join(header) + " |")
    lines.append("|" + "|".join(["---"] * len(header)) + "|")
    for tag in tags:
        for cps in all_cps:
            d = data[tag].get(cps)
            if not d:
                continue
            lines.append(
                f"| {tag} | {cps} | {d['total']} | {d['success']} | "
                f"{d['success_rate']:.1f}% | {d['achieved_cps']:.1f} | "
                f"{d['rt_ms']:.1f} | {d['retrans']} |"
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
                f"(σ {d['rt_stddev_ms']:.1f} ms)"
            )
            lines.append(f"- Call length: {d['call_len_ms']:.1f} ms")
            lines.append(f"- Retransmissions: {d['retrans']}")
            if d["current"] > 0:
                lines.append(
                    f"- ⚠️ {d['current']} calls still in-flight at sweep end "
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
