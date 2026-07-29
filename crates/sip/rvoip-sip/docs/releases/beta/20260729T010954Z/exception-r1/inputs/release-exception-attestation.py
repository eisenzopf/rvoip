#!/usr/bin/env python3
"""Create and verify an explicit owner-approved release exception.

This does not reinterpret a failed beta gate as PASS. It binds an operator
decision to the immutable strict attestation, the complete gate inventory, and
the exact performance evidence that motivated the exception.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
from typing import Any


SCHEMA = "rvoip-release-exception-attestation-v1"
SOURCE_SCHEMA = "rvoip-sip-beta-attestation-v2"
GATE_SCHEMA = "rvoip-sip-gate-results-v1"
EXPECTED_FAILED_GATES = {
    "perf.media-burst-matrix",
    "report.performance-metrics",
}
ROOT_FAILED_GATE = "perf.media-burst-matrix"
DERIVED_FAILED_GATE = "report.performance-metrics"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
PERSONAL_PATH_RE = re.compile(
    r"(?:/(?:Users|home)/[^/\\s\"']+|[A-Za-z]:\\\\Users\\\\[^\\\\\\s\"']+)"
)


class ExceptionAttestationError(RuntimeError):
    """Raised when exception evidence is incomplete or inconsistent."""


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ExceptionAttestationError(f"cannot read JSON {path}: {error}") from error
    if not isinstance(value, dict):
        raise ExceptionAttestationError(f"expected JSON object in {path}")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ExceptionAttestationError(message)


def require_number(value: Any, label: str) -> float:
    require(
        isinstance(value, (int, float)) and not isinstance(value, bool),
        f"{label} must be numeric",
    )
    return float(value)


def write_text(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value)


def write_json(path: Path, value: dict[str, Any]) -> None:
    write_text(path, json.dumps(value, indent=2, sort_keys=True) + "\n")


def sanitized_source_bytes(
    source_path: Path,
    report_root: Path,
    workspace_root: Path,
) -> bytes:
    try:
        text = source_path.read_text()
    except UnicodeDecodeError as error:
        raise ExceptionAttestationError(
            f"source evidence must be UTF-8 text: {source_path}"
        ) from error
    replacements = (
        (str(report_root), "<source-report>"),
        (str(workspace_root), "<workspace>"),
    )
    for original, replacement in replacements:
        text = text.replace(original, replacement)
    return text.encode()


def require_no_personal_paths(path: Path) -> None:
    require(
        PERSONAL_PATH_RE.search(path.read_text(errors="replace")) is None,
        f"personal absolute path leaked into snapshot: {path}",
    )


def raw_artifact_hashes(source: dict[str, Any]) -> dict[str, str]:
    files = source.get("artifacts", {}).get("files")
    require(isinstance(files, list), "source attestation lacks artifact inventory")
    result: dict[str, str] = {}
    for item in files:
        require(isinstance(item, dict), "source artifact record must be an object")
        path = item.get("path")
        digest = item.get("sha256")
        if isinstance(path, str) and isinstance(digest, str):
            result[path] = digest
    return result


def find_one(root: Path, pattern: str) -> Path:
    matches = sorted(root.glob(pattern))
    require(len(matches) == 1, f"expected one {pattern!r} below {root}, found {len(matches)}")
    return matches[0]


def source_layout(report_root: Path) -> dict[str, Path]:
    burst_root = find_one(
        report_root,
        "perf-results/perf_burst_matrix/*/high-density-media-burst",
    )
    return {
        "source/attestation.json": report_root / "attestation.json",
        "source/attestation.json.sha256": report_root / "attestation.json.sha256",
        "source/gate-results.json": report_root / "gate-results.json",
        "source/effective-gate-config.json": report_root / "effective-gate-config.json",
        "source/summary.md": report_root / "summary.md",
        "source/performance-gate-metrics.log": (
            report_root / "performance_gate_metrics_report.log"
        ),
        "evidence/canonical-2k-index.json": report_root / "canonical-2k/index.json",
        "evidence/canonical-2k-run-1.json": report_root / "canonical-2k/run-1/report.json",
        "evidence/canonical-2k-run-2.json": report_root / "canonical-2k/run-2/report.json",
        "evidence/canonical-2k-run-3.json": report_root / "canonical-2k/run-3/report.json",
        "evidence/high-density-caller.json": (
            burst_root / "perf_burst_caller_high-density-media-burst.json"
        ),
        "evidence/high-density-receiver.json": (
            burst_root / "perf_burst_receiver_high-density-media-burst.json"
        ),
        "evidence/host-udp-delta.txt": burst_root / "host_udp_delta.txt",
        "evidence/monolithic-soak.json": report_root / "perf-results/perf_soak_30min.json",
        "evidence/split-soak-caller.json": report_root / "perf-results/perf_soak_caller.json",
        "evidence/split-soak-receiver.json": (
            report_root / "perf-results/perf_soak_receiver.json"
        ),
        "inputs/beta-release-policy.yaml": report_root / "inputs/beta-release-policy.yaml",
    }


def validate_source_package(
    report_root: Path, expected_version: str
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Path]]:
    layout = source_layout(report_root)
    for source_path in layout.values():
        require(source_path.is_file(), f"missing source evidence: {source_path}")

    source_attestation = load_json(layout["source/attestation.json"])
    digest_line = layout["source/attestation.json.sha256"].read_text().strip().split()
    require(len(digest_line) >= 1, "source attestation checksum is empty")
    require(
        digest_line[0] == sha256(layout["source/attestation.json"]),
        "source attestation checksum mismatch",
    )
    require(source_attestation.get("schema") == SOURCE_SCHEMA, "unexpected source schema")
    require(
        source_attestation.get("package", {}).get("version") == expected_version,
        "source attestation package version mismatch",
    )
    require(
        source_attestation.get("run", {}).get("mode") == "full",
        "exception requires a full beta run",
    )
    require(
        source_attestation.get("source", {}).get("clean") is True
        and source_attestation.get("source", {}).get("unchanged") is True,
        "exception requires clean and unchanged tested source",
    )
    require(
        source_attestation.get("result", {}).get("overall") == "FAIL"
        and source_attestation.get("qualification", {}).get("status") == "NON-RC",
        "source attestation must preserve its strict NON-RC result",
    )

    gate_results = load_json(layout["source/gate-results.json"])
    require(gate_results.get("schema") == GATE_SCHEMA, "unexpected gate-results schema")
    records = gate_results.get("records")
    require(isinstance(records, list), "gate-results records must be a list")
    require(
        gate_results.get("required_count") == 108
        and len(records) == 108
        and gate_results.get("passed") == 106
        and gate_results.get("failed") == 2
        and gate_results.get("skipped") == 0,
        "exception is limited to the recorded 106/108, zero-skip result",
    )
    failed_ids = {
        item.get("id")
        for item in records
        if isinstance(item, dict) and item.get("status") == "FAIL"
    }
    require(failed_ids == EXPECTED_FAILED_GATES, f"unexpected failed gates: {failed_ids}")

    source_hashes = raw_artifact_hashes(source_attestation)
    for snapshot_path, source_path in layout.items():
        if snapshot_path in {
            "source/attestation.json",
            "source/attestation.json.sha256",
        }:
            continue
        relative = source_path.relative_to(report_root).as_posix()
        expected_hash = source_hashes.get(relative)
        if expected_hash is None:
            if relative == "gate-results.json":
                expected_hash = source_attestation["structured_reporting"]["gate_results"][
                    "sha256"
                ]
            elif relative == "effective-gate-config.json":
                expected_hash = source_attestation["structured_reporting"][
                    "effective_gate_config"
                ]["sha256"]
            elif relative == "canonical-2k/index.json":
                expected_hash = source_attestation["results"]["canonical_2k"][
                    "index_sha256"
                ]
        require(expected_hash == sha256(source_path), f"unbound source evidence: {relative}")
    return source_attestation, gate_results, layout


def parse_udp_delta(path: Path) -> dict[str, int]:
    values: dict[str, int] = {}
    for line in path.read_text().splitlines():
        if "=" not in line:
            continue
        key, raw = line.split("=", 1)
        try:
            values[key] = int(raw)
        except ValueError:
            continue
    return values


def derive_facts(root: Path, expected_version: str) -> dict[str, Any]:
    source = load_json(root / "source/attestation.json")
    gates = load_json(root / "source/gate-results.json")
    caller = load_json(root / "evidence/high-density-caller.json")
    receiver = load_json(root / "evidence/high-density-receiver.json")
    canonical = load_json(root / "evidence/canonical-2k-index.json")
    monolithic = load_json(root / "evidence/monolithic-soak.json")
    split_caller = load_json(root / "evidence/split-soak-caller.json")
    split_receiver = load_json(root / "evidence/split-soak-receiver.json")
    udp = parse_udp_delta(root / "evidence/host-udp-delta.txt")

    require(source.get("package", {}).get("version") == expected_version, "version mismatch")
    records = gates.get("records")
    require(isinstance(records, list), "missing gate records")
    failed = [
        {
            "id": record["id"],
            "name": record["name"],
            "category": record["category"],
            "duration_seconds": record["duration_seconds"],
        }
        for record in records
        if record.get("status") == "FAIL"
    ]
    require({item["id"] for item in failed} == EXPECTED_FAILED_GATES, "failed gates changed")

    caller_results = caller.get("results", {})
    receiver_results = receiver.get("results", {})
    errors = caller_results.get("errors", {})
    offered = caller_results.get("calls_offered")
    succeeded = caller_results.get("calls_succeeded")
    calls_failed = caller_results.get("calls_failed")
    asr = require_number(caller_results.get("asr"), "burst ASR")
    minimum_asr = 0.995
    require(
        offered == 18000
        and succeeded == 17871
        and calls_failed == 129
        and errors.get("timeout") == 129,
        "burst call accounting differs from the approved exception",
    )
    require(
        all(
            errors.get(key) == 0
            for key in (
                "answer_failed",
                "invite_send_failed",
                "media_setup_failed",
                "overload_rejected",
                "teardown_failed",
            )
        ),
        "the exception does not cover non-timeout burst errors",
    )
    require(asr == 0.9928 and asr < minimum_asr, "unexpected burst ASR")
    require(
        caller_results.get("retained_objects_after_drain") == 0
        and caller_results.get("transaction_manager_active_after_drain") == 0
        and receiver_results.get("retained_objects_after_drain") == 0
        and receiver_results.get("bob_active_audio_receivers") == 0
        and receiver_results.get("transaction_manager_active_after_drain") == 0,
        "the exception does not cover cleanup or retention failures",
    )
    require(
        require_number(caller_results.get("rss_gate_growth_mb_per_hr"), "caller RSS") <= 15
        and require_number(
            receiver_results.get("rss_gate_growth_mb_per_hr"), "receiver RSS"
        )
        <= 15,
        "the exception does not cover RSS failures",
    )
    require(
        receiver_results.get("bob_received_frames", 0) > 0,
        "full-media delivery evidence is missing",
    )
    require(
        udp.get("udp_dropped_full_socket_buffers_delta") == 0,
        "the exception does not cover host UDP full-buffer drops",
    )

    require(
        canonical.get("status") == "PASS"
        and canonical.get("run_count") == 3
        and len(canonical.get("runs", [])) == 3,
        "canonical 2K evidence must contain three passing runs",
    )
    canonical_runs: list[dict[str, Any]] = []
    for sequence in range(1, 4):
        report = load_json(root / f"evidence/canonical-2k-run-{sequence}.json")
        result = report.get("results", {})
        latency = report.get("latency_ns", {})
        canonical_runs.append(
            {
                "sequence": sequence,
                "target_cps": report.get("load", {}).get("target_cps"),
                "achieved_cps": result.get("achieved_cps"),
                "asr": result.get("asr"),
                "calls_offered": result.get("calls_offered"),
                "calls_succeeded": result.get("calls_succeeded"),
                "setup_p99_ms": round(
                    require_number(
                        latency.get("setup_latency", {}).get("p99"),
                        f"canonical run {sequence} setup p99",
                    )
                    / 1_000_000,
                    3,
                ),
                "full_cycle_p99_ms": round(
                    require_number(
                        latency.get("full_cycle", {}).get("p99"),
                        f"canonical run {sequence} full-cycle p99",
                    )
                    / 1_000_000,
                    3,
                ),
            }
        )
    require(
        all(
            run["target_cps"] == 2000
            and run["asr"] == 1
            and run["calls_offered"] == 65000
            and run["calls_succeeded"] == 65000
            for run in canonical_runs
        ),
        "canonical 2K call accounting changed",
    )

    mono = monolithic.get("results", {})
    require(
        mono.get("duration_secs") == 3600
        and mono.get("calls_offered") == mono.get("calls_succeeded")
        and mono.get("retained_objects_after_drain") == 0
        and mono.get("bob_active_audio_receivers") == 0
        and mono.get("controlled_drain_failed") == 0
        and all(value == 0 for value in mono.get("errors", {}).values()),
        "monolithic soak evidence is not clean",
    )
    require(
        split_caller.get("results", {}).get("retained_objects_after_drain") == 0
        and split_receiver.get("results", {}).get("retained_objects_after_drain") == 0,
        "split-soak retention evidence is not clean",
    )

    source_info = source["source"]
    return {
        "strict_result": {
            "overall": source["result"]["overall"],
            "qualification": source["qualification"]["status"],
            "release_candidate": source["qualification"]["release_candidate"],
        },
        "run": source["run"],
        "package": source["package"],
        "source": {
            "tested_commit": source_info["start"]["git_commit"],
            "tested_tree": source_info["start"]["git_tree"],
            "source_fingerprint_sha256": source_info["start"][
                "source_fingerprint_sha256"
            ],
            "clean": source_info["clean"],
            "unchanged": source_info["unchanged"],
        },
        "gates": {
            "required": gates["required_count"],
            "passed": gates["passed"],
            "failed": gates["failed"],
            "skipped": gates["skipped"],
            "failed_records": failed,
            "root_failed_gate": ROOT_FAILED_GATE,
            "derived_failed_gate": DERIVED_FAILED_GATE,
            "root_deviation_count": 1,
        },
        "exception": {
            "scope": "high-density-media-burst answer timeout rate",
            "offered_calls": offered,
            "succeeded_calls": succeeded,
            "timeout_calls": errors["timeout"],
            "observed_asr": asr,
            "required_asr": minimum_asr,
            "absolute_shortfall": round(minimum_asr - asr, 4),
            "observed_timeout_percent": round(errors["timeout"] * 100 / offered, 6),
            "allowed_timeout_percent": 0.5,
            "non_timeout_errors": 0,
        },
        "preserved_invariants": {
            "caller_retained_after_drain": caller_results[
                "retained_objects_after_drain"
            ],
            "receiver_retained_after_drain": receiver_results[
                "retained_objects_after_drain"
            ],
            "receiver_active_audio_receivers_after_drain": receiver_results[
                "bob_active_audio_receivers"
            ],
            "caller_transaction_manager_after_drain": caller_results[
                "transaction_manager_active_after_drain"
            ],
            "receiver_transaction_manager_after_drain": receiver_results[
                "transaction_manager_active_after_drain"
            ],
            "caller_rss_gate_mb_per_hr": caller_results[
                "rss_gate_growth_mb_per_hr"
            ],
            "receiver_rss_gate_mb_per_hr": receiver_results[
                "rss_gate_growth_mb_per_hr"
            ],
            "delivered_audio_frames": receiver_results["bob_received_frames"],
            "host_udp_full_socket_buffer_drops": udp[
                "udp_dropped_full_socket_buffers_delta"
            ],
        },
        "canonical_2k": {
            "status": canonical["status"],
            "common_executable_sha256": canonical["common_executable_sha256"],
            "common_source_fingerprint_sha256": canonical[
                "common_source_fingerprint_sha256"
            ],
            "runs": canonical_runs,
        },
        "monolithic_soak": {
            "duration_secs": mono["duration_secs"],
            "calls_offered": mono["calls_offered"],
            "calls_succeeded": mono["calls_succeeded"],
            "retained_after_drain": mono["retained_objects_after_drain"],
            "active_audio_receivers_after_drain": mono[
                "bob_active_audio_receivers"
            ],
            "rss_gate_growth_mb_per_hr": mono["rss_gate_growth_mb_per_hr"],
        },
    }


def artifact_manifest(root: Path) -> list[dict[str, Any]]:
    excluded = {"exception-attestation.json", "exception-attestation.json.sha256"}
    artifacts = []
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        relative = path.relative_to(root).as_posix()
        if relative in excluded:
            continue
        artifacts.append(
            {
                "path": relative,
                "bytes": path.stat().st_size,
                "sha256": sha256(path),
            }
        )
    return artifacts


def report_release(
    facts: dict[str, Any], approval_actor: str, approval_basis: str
) -> str:
    version = facts["package"]["version"]
    gates = facts["gates"]
    exception = facts["exception"]
    source = facts["source"]
    return f"""# rvoip {version} Release Exception Attestation

> Owner-approved exception derived from strict full-run package `{facts["run"]["id"]}`.
> No failed gate was rewritten as PASS. The strict source attestation remains
> `FAIL` / `NON-RC`; this document records the separate release decision.

## Decision

| Field | Value |
|---|---|
| Release | `{version}` |
| Disposition | **APPROVED-WITH-EXCEPTION** |
| Approval actor | `{approval_actor}` |
| Approval basis | {approval_basis} |
| Strict automated qualification | **{facts["strict_result"]["qualification"]}** |
| Tested commit | `{source["tested_commit"]}` |
| Tested tree | `{source["tested_tree"]}` |
| Clean and unchanged source | `{source["clean"] and source["unchanged"]}` |
| Gate inventory | `{gates["passed"]}/{gates["required"]}` PASS, `{gates["failed"]}` FAIL, `{gates["skipped"]}` SKIP |
| Root policy deviations | `{gates["root_deviation_count"]}` |

## Accepted deviation

The high-density full-media burst delivered `{exception["succeeded_calls"]}` of
`{exception["offered_calls"]}` calls. ASR was `{exception["observed_asr"]:.4f}`
against the release requirement of `{exception["required_asr"]:.4f}`, an
absolute shortfall of `{exception["absolute_shortfall"]:.4f}`. All
`{exception["timeout_calls"]}` failures were answer timeouts; non-timeout errors
were zero.

The second failed record, `report.performance-metrics`, is the reporting
roll-up of the same ASR miss. It is not a second independent product failure.

## Preserved invariants

- Media setup, overload rejection, teardown, and other non-timeout errors: zero.
- Caller and receiver retained resources after drain: zero.
- Active receiver audio resources after drain: zero.
- Caller and receiver transaction managers after drain: zero.
- Host UDP full-socket-buffer drops: zero.
- Delivered application audio frames: `{facts["preserved_invariants"]["delivered_audio_frames"]}`.
- Caller/receiver RSS gate values: `{facts["preserved_invariants"]["caller_rss_gate_mb_per_hr"]}` / `{facts["preserved_invariants"]["receiver_rss_gate_mb_per_hr"]}` MB/hour.
- Canonical 2K qualification: three source-identical PASS runs.
- Monolithic and split soaks: clean completion and zero post-drain retention.

## Release meaning

This decision permits preparation and publication of `{version}` with the accepted
burst-risk disclosure. It does **not** convert the run into a strict beta
release candidate and does not authorize broader production, carrier-SBC, or
untested-topology claims.

Tracked evidence replaces personal absolute host paths with `<workspace>` or
`<source-report>`. Each source binding records both the original source
SHA-256 and the sanitized snapshot SHA-256.

See [the complete gate record](BETA_GATE_REPORT.md), [performance details](BETA_PERFORMANCE_REPORT.md),
and the machine-verifiable [`exception-attestation.json`](exception-attestation.json).
"""


def report_gates(facts: dict[str, Any], records: list[dict[str, Any]]) -> str:
    version = facts["package"]["version"]
    category: dict[str, dict[str, int]] = {}
    rows = []
    for record in records:
        totals = category.setdefault(
            record["category"], {"required": 0, "passed": 0, "failed": 0, "skipped": 0}
        )
        totals["required"] += 1
        status = {
            "PASS": "passed",
            "FAIL": "failed",
            "SKIP": "skipped",
        }[record["status"]]
        totals[status] += 1
        evidence = ", ".join(item["path"] for item in record.get("evidence", []))
        rows.append(
            f'| {record["sequence"]:03d} | `{record["id"]}` | {record["status"]} | '
            f'{record["duration_seconds"]} | {evidence} |'
        )
    category_rows = [
        f"| {name} | {value['required']} | {value['passed']} | "
        f"{value['failed']} | {value['skipped']} |"
        for name, value in sorted(category.items())
    ]
    gates = facts["gates"]
    return f"""# {version} Complete Beta Gate Record

> Status and structured results from `source/gate-results.json` for full run
> `{facts["run"]["id"]}`. Original statuses and source evidence hashes are
> preserved; personal absolute host paths are deterministically redacted.

## Result

**Strict result: FAIL — {gates["passed"]}/{gates["required"]} passed,
{gates["failed"]} failed, {gates["skipped"]} skipped.**

The owner-approved release exception covers one root policy deviation:
`{gates["root_failed_gate"]}`. `{gates["derived_failed_gate"]}` is its derived
reporting roll-up.

## Category totals

| Category | Required | Passed | Failed | Skipped |
|---|---:|---:|---:|---:|
{chr(10).join(category_rows)}

## Complete ordered inventory

| Seq | Gate ID | Status | Seconds | Evidence |
|---:|---|---|---:|---|
{chr(10).join(rows)}

The full structured checks, commands, timestamps, and SHA-256 evidence bindings
are retained in [`source/gate-results.json`](source/gate-results.json) and
[`source/attestation.json`](source/attestation.json).
"""


def report_performance(facts: dict[str, Any]) -> str:
    version = facts["package"]["version"]
    canonical_rows = [
        f"| {run['sequence']} | {run['target_cps']} | {run['achieved_cps']} | "
        f"{run['asr']:.4f} | {run['calls_succeeded']}/{run['calls_offered']} | "
        f"{run['setup_p99_ms']:.3f} | {run['full_cycle_p99_ms']:.3f} |"
        for run in facts["canonical_2k"]["runs"]
    ]
    exception = facts["exception"]
    invariants = facts["preserved_invariants"]
    soak = facts["monolithic_soak"]
    return f"""# {version} Beta Performance Evidence

> Evidence from full run `{facts["run"]["id"]}`. The high-density burst retains
> its strict FAIL status and is released only under the adjacent owner exception.

## Canonical 2K three-pass evidence

| Run | Target CPS | Achieved CPS | ASR | Calls | Setup p99 ms | Cycle p99 ms |
|---:|---:|---:|---:|---:|---:|---:|
{chr(10).join(canonical_rows)}

All three runs share source fingerprint
`{facts["canonical_2k"]["common_source_fingerprint_sha256"]}` and executable
SHA-256 `{facts["canonical_2k"]["common_executable_sha256"]}`.

## High-density full-media burst

| Metric | Requirement | Observed | Strict result |
|---|---:|---:|---|
| Offered CPS | 160 | 160 | PASS |
| ASR | >= {exception["required_asr"]:.4f} | {exception["observed_asr"]:.4f} | **FAIL / WAIVED** |
| Calls | 18,000 | {exception["succeeded_calls"]} succeeded, {exception["timeout_calls"]} timed out | **FAIL / WAIVED** |
| Non-timeout errors | 0 | {exception["non_timeout_errors"]} | PASS |
| Caller retained after drain | 0 | {invariants["caller_retained_after_drain"]} | PASS |
| Receiver retained after drain | 0 | {invariants["receiver_retained_after_drain"]} | PASS |
| Receiver active audio resources | 0 | {invariants["receiver_active_audio_receivers_after_drain"]} | PASS |
| Delivered audio frames | > 0 | {invariants["delivered_audio_frames"]} | PASS |
| UDP full-buffer drops | 0 | {invariants["host_udp_full_socket_buffer_drops"]} | PASS |
| Caller RSS MB/hour | <= 15 | {invariants["caller_rss_gate_mb_per_hr"]} | PASS |
| Receiver RSS MB/hour | <= 15 | {invariants["receiver_rss_gate_mb_per_hr"]} | PASS |

## Monolithic soak

| Metric | Observed |
|---|---:|
| Duration | {soak["duration_secs"]} seconds |
| Calls | {soak["calls_succeeded"]}/{soak["calls_offered"]} |
| Retained after drain | {soak["retained_after_drain"]} |
| Active audio receivers after drain | {soak["active_audio_receivers_after_drain"]} |
| RSS gate | {soak["rss_gate_growth_mb_per_hr"]} MB/hour |

The tracked evidence files in `evidence/` are the machine-readable authority.
Focused follow-up controls are not used as formal evidence here because their
artifacts were removed by the subsequently requested `cargo clean`.
"""


def create(
    report_root: Path,
    output_dir: Path,
    version: str,
    approval_actor: str,
    approval_basis: str,
) -> Path:
    require(approval_actor.strip() != "", "approval actor must not be empty")
    require(approval_basis.strip() != "", "approval basis must not be empty")
    source, gate_results, layout = validate_source_package(report_root, version)
    require(
        not output_dir.exists() or not any(output_dir.iterdir()),
        f"output directory must be absent or empty: {output_dir}",
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    workspace_root = Path(__file__).resolve().parent.parent
    for relative, source_path in layout.items():
        destination = output_dir / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(
            sanitized_source_bytes(source_path, report_root, workspace_root)
        )
        shutil.copystat(source_path, destination)
    source_checksum = output_dir / "source/attestation.json.sha256"
    write_text(
        source_checksum,
        f"{sha256(output_dir / 'source/attestation.json')}  attestation.json\n",
    )
    verifier_copy = output_dir / "inputs/release-exception-attestation.py"
    shutil.copy2(Path(__file__).resolve(), verifier_copy)

    facts = derive_facts(output_dir, version)
    write_text(
        output_dir / "BETA_RELEASE_REPORT.md",
        report_release(facts, approval_actor, approval_basis),
    )
    write_text(
        output_dir / "BETA_GATE_REPORT.md",
        report_gates(facts, gate_results["records"]),
    )
    write_text(
        output_dir / "BETA_PERFORMANCE_REPORT.md",
        report_performance(facts),
    )

    source_bindings = []
    for snapshot_path, source_path in sorted(layout.items()):
        source_bindings.append(
            {
                "snapshot_path": snapshot_path,
                "source_path": source_path.relative_to(report_root).as_posix(),
                "source_sha256": sha256(source_path),
                "snapshot_sha256": sha256(output_dir / snapshot_path),
                "transformation": "personal-path-redaction-v1",
            }
        )
    payload = {
        "schema": SCHEMA,
        "created_at_utc": utc_now(),
        "release": {
            "version": version,
            "disposition": "APPROVED-WITH-EXCEPTION",
            "strict_automated_status": "NON-RC",
        },
        "approval": {
            "actor": approval_actor,
            "basis": approval_basis,
            "method": "explicit project-owner/operator instruction",
            "cryptographically_signed": False,
        },
        "facts": facts,
        "source_attestation": {
            "path": "source/attestation.json",
            "sha256": sha256(output_dir / "source/attestation.json"),
            "original_sha256": sha256(layout["source/attestation.json"]),
            "schema": source["schema"],
            "strict_overall": source["result"]["overall"],
            "strict_qualification": source["qualification"]["status"],
        },
        "source_bindings": source_bindings,
        "artifacts": artifact_manifest(output_dir),
        "assurance": {
            "kind": "integrity-and-explicit-risk-acceptance",
            "hash_algorithm": "SHA-256",
            "cryptographically_signed": False,
            "note": (
                "The digest binds the exception decision and copied evidence; "
                "it is not third-party signing and does not alter strict gate results."
            ),
        },
    }
    for path in output_dir.rglob("*"):
        if path.is_file():
            require_no_personal_paths(path)
    attestation_path = output_dir / "exception-attestation.json"
    write_json(attestation_path, payload)
    write_text(
        output_dir / "exception-attestation.json.sha256",
        f"{sha256(attestation_path)}  exception-attestation.json\n",
    )
    verify(attestation_path, version)
    return attestation_path


def verify(attestation_path: Path, expected_version: str) -> dict[str, Any]:
    require(attestation_path.is_file(), f"missing attestation: {attestation_path}")
    root = attestation_path.parent
    checksum_path = root / "exception-attestation.json.sha256"
    require(checksum_path.is_file(), f"missing checksum: {checksum_path}")
    checksum_fields = checksum_path.read_text().strip().split()
    require(
        len(checksum_fields) == 2
        and checksum_fields[1] == "exception-attestation.json"
        and checksum_fields[0] == sha256(attestation_path),
        "exception attestation checksum mismatch",
    )
    payload = load_json(attestation_path)
    require(payload.get("schema") == SCHEMA, "unexpected exception schema")
    release = payload.get("release", {})
    require(release.get("version") == expected_version, "release version mismatch")
    require(
        release.get("disposition") == "APPROVED-WITH-EXCEPTION"
        and release.get("strict_automated_status") == "NON-RC",
        "invalid exception disposition",
    )
    approval = payload.get("approval", {})
    require(
        isinstance(approval.get("actor"), str)
        and approval["actor"].strip()
        and isinstance(approval.get("basis"), str)
        and approval["basis"].strip()
        and approval.get("cryptographically_signed") is False,
        "exception approval is incomplete",
    )
    artifacts = payload.get("artifacts")
    require(isinstance(artifacts, list) and artifacts, "artifact manifest is missing")
    expected_paths: set[str] = set()
    for item in artifacts:
        require(isinstance(item, dict), "artifact entry must be an object")
        relative = item.get("path")
        require(
            isinstance(relative, str)
            and relative not in expected_paths
            and not Path(relative).is_absolute()
            and ".." not in Path(relative).parts,
            f"invalid artifact path: {relative!r}",
        )
        expected_paths.add(relative)
        path = root / relative
        require(path.is_file(), f"missing attested artifact: {relative}")
        require(path.stat().st_size == item.get("bytes"), f"size mismatch: {relative}")
        require(sha256(path) == item.get("sha256"), f"hash mismatch: {relative}")
    actual_paths = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file()
    } - {"exception-attestation.json", "exception-attestation.json.sha256"}
    require(actual_paths == expected_paths, "snapshot contains unattested or missing files")
    for path in root.rglob("*"):
        if path.is_file():
            require_no_personal_paths(path)

    bindings = payload.get("source_bindings")
    require(
        isinstance(bindings, list) and bindings,
        "source bindings are missing",
    )
    bound_paths: set[str] = set()
    for binding in bindings:
        require(isinstance(binding, dict), "source binding must be an object")
        snapshot_path = binding.get("snapshot_path")
        source_path = binding.get("source_path")
        source_digest = binding.get("source_sha256")
        snapshot_digest = binding.get("snapshot_sha256")
        require(
            isinstance(snapshot_path, str)
            and snapshot_path in expected_paths
            and snapshot_path not in bound_paths
            and isinstance(source_path, str)
            and not Path(source_path).is_absolute()
            and ".." not in Path(source_path).parts
            and isinstance(source_digest, str)
            and SHA256_RE.fullmatch(source_digest) is not None
            and isinstance(snapshot_digest, str)
            and snapshot_digest == sha256(root / snapshot_path)
            and binding.get("transformation") == "personal-path-redaction-v1",
            f"invalid source binding: {snapshot_path!r}",
        )
        bound_paths.add(snapshot_path)
    expected_bound_paths = expected_paths - {
        "BETA_RELEASE_REPORT.md",
        "BETA_GATE_REPORT.md",
        "BETA_PERFORMANCE_REPORT.md",
        "inputs/release-exception-attestation.py",
    }
    require(
        bound_paths == expected_bound_paths,
        "source binding inventory is incomplete or unexpected",
    )

    source_checksum = root / "source/attestation.json.sha256"
    source_fields = source_checksum.read_text().strip().split()
    require(
        source_fields
        and source_fields[0] == sha256(root / "source/attestation.json"),
        "copied source attestation checksum mismatch",
    )
    facts = derive_facts(root, expected_version)
    require(payload.get("facts") == facts, "derived exception facts changed")
    require(
        payload.get("source_attestation", {}).get("sha256")
        == sha256(root / "source/attestation.json"),
        "source attestation binding changed",
    )
    original_attestation_hash = next(
        binding["source_sha256"]
        for binding in bindings
        if binding["snapshot_path"] == "source/attestation.json"
    )
    require(
        payload.get("source_attestation", {}).get("original_sha256")
        == original_attestation_hash,
        "original source attestation binding changed",
    )
    return payload


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        description="Create or verify an explicit release-exception attestation."
    )
    commands = result.add_subparsers(dest="command", required=True)
    create_command = commands.add_parser("create")
    create_command.add_argument("--report-root", type=Path, required=True)
    create_command.add_argument("--output-dir", type=Path, required=True)
    create_command.add_argument("--version", required=True)
    create_command.add_argument("--approval-actor", required=True)
    create_command.add_argument("--approval-basis", required=True)
    verify_command = commands.add_parser("verify")
    verify_command.add_argument("--attestation", type=Path, required=True)
    verify_command.add_argument("--version", required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "create":
            path = create(
                args.report_root.resolve(),
                args.output_dir.resolve(),
                args.version,
                args.approval_actor,
                args.approval_basis,
            )
            print(f"release exception attestation created: {path}")
        else:
            verify(args.attestation.resolve(), args.version)
            print(
                "release exception attestation: PASS "
                f"({args.version}, {args.attestation})"
            )
    except ExceptionAttestationError as error:
        print(f"release exception attestation: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
