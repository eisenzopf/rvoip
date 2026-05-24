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


def parse_optional_int(value) -> int | None:
    if value is None or value == "":
        return None
    return int(value)


def fmt_optional(value) -> str:
    return "n/a" if value is None else str(value)


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


def default_supplemental() -> dict:
    return {
        "sipp_rc": None,
        "udp_full_socket_drops_delta": None,
        "listener_accepted_total": None,
        "listener_cleaned_total": None,
        "dead_200_total": 0,
        "dead_200_by_cseq": Counter(),
        "sip_udp_diag": {},
        "sip_retrans_diag": {},
        "sample_artifact": None,
        "samply_profile": None,
        "samply_log": None,
    }


def dead_call_200_by_cseq(path: Path) -> Counter:
    counts = Counter()
    text = path.read_text(errors="replace")
    for chunk in text.split("Dead call ")[1:]:
        if "received 'SIP/2.0 200 OK" not in chunk:
            continue
        match = re.search(r"(?im)^CSeq:\s*\d+\s+([A-Z]+)\s*$", chunk)
        method = match.group(1).upper() if match else "UNKNOWN"
        counts[method] += 1
    return counts


def parse_key_value_file(path: Path) -> dict[str, str]:
    values = {}
    for line in path.read_text(errors="replace").splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    return values


def extract_int(line: str, key: str) -> int | None:
    match = re.search(rf"\b{re.escape(key)}=(\d+)", line)
    return parse_optional_int(match.group(1)) if match else None


def extract_bracket(line: str, key: str) -> str | None:
    marker = f"{key}=["
    start = line.find(marker)
    if start < 0:
        return None
    value_start = start + len(marker)
    depth = 1
    idx = value_start
    while idx < len(line):
        ch = line[idx]
        if ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
            if depth == 0:
                return line[value_start:idx]
        idx += 1
    return None


def parse_sip_udp_diag(line: str | None) -> dict:
    if not line:
        return {}
    keys = [
        "queue_full",
        "send_errors",
        "raw_sends",
        "resp_2xx",
        "manager_backpressure_events",
        "transport_backpressure_events",
    ]
    out = {key: extract_int(line, key) for key in keys}
    out["transport_manager_to_transaction"] = extract_bracket(
        line, "transport_manager_to_transaction"
    )
    return out


def parse_sip_retrans_diag(line: str | None) -> dict:
    if not line:
        return {}
    keys = [
        "dup_invite_cache_miss",
        "ack_unmatched",
        "worker_mismatch",
        "invite_2xx_proactive_retx",
        "bye_200_sent",
    ]
    out = {key: extract_int(line, key) for key in keys}
    for bracket_key in [
        "ok_200_source",
        "bye_path",
        "bye_tombstone",
        "transaction_dispatch_queue",
        "transaction_dispatch_queue_by_kind",
        "transaction_dispatch_queue_by_worker",
        "transaction_dispatch_queue_depth",
        "transaction_dispatch_backpressure",
        "dialog_event_dispatch_queue",
        "dialog_to_session_queue",
        "bye_receive_to_200",
    ]:
        out[bracket_key] = extract_bracket(line, bracket_key)
    return out


def parse_listener_log(path: Path) -> dict:
    last_udp_diag = None
    last_retrans_diag = None
    accepted_total = None
    cleaned_total = None
    for line in path.read_text(errors="replace").splitlines():
        if "[sip_udp_diag]" in line:
            last_udp_diag = line
        elif "[sip_retrans_diag]" in line:
            last_retrans_diag = line
        if "[perf_listener]" in line and "accepted_total=" in line:
            match = re.search(r"accepted_total=(\d+).*cleaned_total=(\d+)", line)
            if match:
                accepted_total = int(match.group(1))
                cleaned_total = int(match.group(2))

    return {
        "listener_accepted_total": accepted_total,
        "listener_cleaned_total": cleaned_total,
        "sip_udp_diag": parse_sip_udp_diag(last_udp_diag),
        "sip_retrans_diag": parse_sip_retrans_diag(last_retrans_diag),
    }


def collect_supplemental(results_dir: Path) -> dict[str, dict[int, dict]]:
    by_run: dict[str, dict[int, dict]] = {}

    def get(tag: str, cps: int) -> dict:
        return by_run.setdefault(tag, {}).setdefault(cps, default_supplemental())

    patterns = {
        "errors": re.compile(r"^(?P<tag>.+)_(?P<cps>\d+)cps(?:_s\d+)?_errors\.log$"),
        "host_udp": re.compile(r"^(?P<tag>.+)_(?P<cps>\d+)cps_host_udp_netstat\.txt$"),
        "listener": re.compile(r"^(?P<tag>.+)_(?P<cps>\d+)cps_listener\.log$"),
        "sample": re.compile(r"^(?P<tag>.+)_(?P<cps>\d+)cps_sample\.txt$"),
        "samply_profile": re.compile(
            r"^(?P<tag>.+)_(?P<cps>\d+)cps_samply_profile\.json\.gz$"
        ),
        "samply_log": re.compile(r"^(?P<tag>.+)_(?P<cps>\d+)cps_samply\.log$"),
    }

    for path in sorted(results_dir.iterdir()):
        name = path.name
        if match := patterns["errors"].match(name):
            tag, cps = match.group("tag"), int(match.group("cps"))
            counts = dead_call_200_by_cseq(path)
            entry = get(tag, cps)
            entry["dead_200_by_cseq"].update(counts)
            entry["dead_200_total"] += sum(counts.values())
        elif match := patterns["host_udp"].match(name):
            tag, cps = match.group("tag"), int(match.group("cps"))
            values = parse_key_value_file(path)
            entry = get(tag, cps)
            entry["sipp_rc"] = parse_optional_int(values.get("rc"))
            entry["udp_full_socket_drops_delta"] = parse_optional_int(
                values.get("udp_full_socket_drops_delta")
            )
        elif match := patterns["listener"].match(name):
            tag, cps = match.group("tag"), int(match.group("cps"))
            get(tag, cps).update(parse_listener_log(path))
        elif match := patterns["sample"].match(name):
            tag, cps = match.group("tag"), int(match.group("cps"))
            get(tag, cps)["sample_artifact"] = path.as_posix()
        elif match := patterns["samply_profile"].match(name):
            tag, cps = match.group("tag"), int(match.group("cps"))
            get(tag, cps)["samply_profile"] = path.as_posix()
        elif match := patterns["samply_log"].match(name):
            tag, cps = match.group("tag"), int(match.group("cps"))
            get(tag, cps)["samply_log"] = path.as_posix()

    for by_cps in by_run.values():
        for entry in by_cps.values():
            entry["dead_200_by_cseq"] = dict(entry["dead_200_by_cseq"])

    return by_run


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
    supplemental = collect_supplemental(results_dir)
    out: dict[str, dict[int, dict]] = {}
    for tag, by_cps in runs.items():
        for cps, cps_runs in by_cps.items():
            aggregated = aggregate_runs(cps_runs)
            aggregated.update(default_supplemental())
            aggregated.update(supplemental.get(tag, {}).get(cps, {}))
            out.setdefault(tag, {})[cps] = aggregated
    return out


def dead_count(run: dict, method: str) -> int:
    return int(run.get("dead_200_by_cseq", {}).get(method, 0))


def format_dead_counts(run: dict) -> str:
    counts = run.get("dead_200_by_cseq", {})
    if not counts:
        return "none"
    return ", ".join(f"{method}={count}" for method, count in sorted(counts.items()))


def diag_value(run: dict, group: str, key: str):
    return run.get(group, {}).get(key)


def format_artifact(value) -> str:
    return value if value else "n/a"


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
        "SIPp rc",
        "UDP drops",
        "Dead 200",
        "Dead INVITE 200",
        "Dead BYE 200",
        "dup miss",
        "ack unmatched",
        "worker mismatch",
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
                f"{d['rt_ms']:.1f} | {d['rt_p95']} | {d['rt_p99']} | {d['retrans']} | "
                f"{fmt_optional(d.get('sipp_rc'))} | "
                f"{fmt_optional(d.get('udp_full_socket_drops_delta'))} | "
                f"{d.get('dead_200_total', 0)} | {dead_count(d, 'INVITE')} | "
                f"{dead_count(d, 'BYE')} | "
                f"{fmt_optional(diag_value(d, 'sip_retrans_diag', 'dup_invite_cache_miss'))} | "
                f"{fmt_optional(diag_value(d, 'sip_retrans_diag', 'ack_unmatched'))} | "
                f"{fmt_optional(diag_value(d, 'sip_retrans_diag', 'worker_mismatch'))} |"
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
            udp_diag = d.get("sip_udp_diag", {})
            retrans_diag = d.get("sip_retrans_diag", {})
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
            lines.append(
                f"- SIPp rc: {fmt_optional(d.get('sipp_rc'))}; "
                f"host UDP full-socket drops: "
                f"{fmt_optional(d.get('udp_full_socket_drops_delta'))}"
            )
            lines.append(
                f"- Listener final: accepted "
                f"{fmt_optional(d.get('listener_accepted_total'))}, cleaned "
                f"{fmt_optional(d.get('listener_cleaned_total'))}"
            )
            lines.append(
                f"- Dead-call 200 OK: {d.get('dead_200_total', 0)} "
                f"({format_dead_counts(d)})"
            )
            lines.append(
                "- Final sip_udp_diag: "
                f"queue_full={fmt_optional(udp_diag.get('queue_full'))}, "
                f"send_errors={fmt_optional(udp_diag.get('send_errors'))}, "
                f"raw_sends={fmt_optional(udp_diag.get('raw_sends'))}, "
                f"resp_2xx={fmt_optional(udp_diag.get('resp_2xx'))}, "
                f"transport_manager_to_transaction=["
                f"{udp_diag.get('transport_manager_to_transaction') or 'n/a'}]"
            )
            lines.append(
                "- Final sip_retrans_diag: "
                f"dup_invite_cache_miss="
                f"{fmt_optional(retrans_diag.get('dup_invite_cache_miss'))}, "
                f"ack_unmatched={fmt_optional(retrans_diag.get('ack_unmatched'))}, "
                f"worker_mismatch={fmt_optional(retrans_diag.get('worker_mismatch'))}, "
                f"invite_2xx_proactive_retx="
                f"{fmt_optional(retrans_diag.get('invite_2xx_proactive_retx'))}, "
                f"bye_200_sent={fmt_optional(retrans_diag.get('bye_200_sent'))}"
            )
            if retrans_diag.get("ok_200_source"):
                lines.append(
                    f"- 200 OK sources: [{retrans_diag['ok_200_source']}]"
                )
            for metric_name, label in [
                ("transaction_dispatch_queue", "Transaction dispatch queue"),
                ("transaction_dispatch_queue_by_kind", "Transaction dispatch queue by kind"),
                ("transaction_dispatch_queue_by_worker", "Transaction dispatch queue by worker"),
                ("transaction_dispatch_queue_depth", "Transaction dispatch queue depth"),
                ("transaction_dispatch_backpressure", "Transaction dispatch backpressure"),
                ("dialog_event_dispatch_queue", "Dialog event dispatch queue"),
                ("dialog_to_session_queue", "Dialog-to-session queue"),
                ("bye_receive_to_200", "BYE receive-to-200"),
                ("bye_path", "BYE path"),
                ("bye_tombstone", "BYE tombstone"),
            ]:
                if retrans_diag.get(metric_name):
                    lines.append(f"- {label}: [{retrans_diag[metric_name]}]")
            lines.append(
                f"- Profile artifacts: sample={format_artifact(d.get('sample_artifact'))}; "
                f"samply_profile={format_artifact(d.get('samply_profile'))}; "
                f"samply_log={format_artifact(d.get('samply_log'))}"
            )
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
