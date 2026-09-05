#!/usr/bin/env python3
"""Render public release reports from one exact remote qualification bundle.

The remote release pipeline deliberately stores machine evidence separately from
the source tree.  This tool verifies that bundle, derives human-readable reports,
and writes a small checksummed summary suitable for committing after a release.
It never runs, repairs, or infers a gate result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import sys
from collections import Counter
from pathlib import Path
from typing import Any


CATALOG_SCHEMA = "rvoip-release-gate-catalog-v1"
PLAN_SCHEMA = "rvoip-release-gate-plan-v1"
AGGREGATE_SCHEMA = "rvoip-release-qualification-v1"
RECEIPT_SCHEMA = "rvoip-release-gate-receipt-v1"
SUMMARY_SCHEMA = "rvoip-release-qualification-summary-v1"
ATTESTATION_SCHEMA = "rvoip-release-qualification-report-attestation-v1"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
REPORT_FILES = (
    "BETA_RELEASE_REPORT.md",
    "BETA_GATE_REPORT.md",
    "BETA_PERFORMANCE_REPORT.md",
    "QUALIFICATION_SUMMARY.json",
)
ATTESTATION_FILES = (
    "QUALIFICATION_REPORT_ATTESTATION.json",
    "QUALIFICATION_REPORT_ATTESTATION.json.sha256",
)


class ReportError(RuntimeError):
    """The supplied qualification evidence is incomplete or inconsistent."""


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def pretty_bytes(value: Any) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_path(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def read_object(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ReportError(f"cannot read {label} at {path}: {error}") from error
    if not isinstance(value, dict):
        raise ReportError(f"{label} must be a JSON object")
    return value


def safe_relative(root: Path, value: str, label: str) -> Path:
    relative = Path(value)
    if relative.is_absolute() or ".." in relative.parts:
        raise ReportError(f"unsafe {label} path: {value!r}")
    path = root / relative
    try:
        path.resolve().relative_to(root.resolve())
    except ValueError as error:
        raise ReportError(f"{label} escapes its evidence root: {value!r}") from error
    return path


def md_cell(value: Any) -> str:
    if value is None:
        return "—"
    return str(value).replace("|", "\\|").replace("\n", " ")


def number(value: Any, digits: int = 2) -> str:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return "—"
    return f"{value:.{digits}f}".rstrip("0").rstrip(".")


def milliseconds(value: Any) -> str:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return "—"
    return number(value / 1_000_000, 3)


def load_bundle(
    *,
    catalog_path: Path,
    plan_path: Path,
    aggregate_path: Path,
    evidence_root: Path,
    version: str,
) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, Any]]]:
    catalog = read_object(catalog_path, "gate catalog")
    plan = read_object(plan_path, "qualification plan")
    aggregate = read_object(aggregate_path, "qualification aggregate")

    if catalog.get("schema") != CATALOG_SCHEMA:
        raise ReportError("unsupported gate catalog schema")
    if plan.get("schema") != PLAN_SCHEMA:
        raise ReportError("unsupported qualification plan schema")
    if aggregate.get("schema") != AGGREGATE_SCHEMA:
        raise ReportError("unsupported qualification aggregate schema")
    candidate = aggregate.get("candidate_sha")
    if not isinstance(candidate, str) or not COMMIT_RE.fullmatch(candidate):
        raise ReportError("qualification aggregate has no exact candidate commit")
    if (
        aggregate.get("status") != "PASS"
        or aggregate.get("failures") != []
        or aggregate.get("profile") != "remote-release"
        or aggregate.get("publishing_attempted") is not False
    ):
        raise ReportError("qualification aggregate is not a clean remote-release PASS")
    expected_catalog_hash = sha256_bytes(canonical_bytes(catalog))
    if aggregate.get("catalog_sha256") != expected_catalog_hash:
        raise ReportError("qualification aggregate does not bind this gate catalog")
    if plan.get("candidate_sha") != candidate or plan.get("profile") != "remote-release":
        raise ReportError("qualification plan and aggregate identify different candidates")

    catalog_gates = catalog.get("gates")
    profile_ids = catalog.get("profiles", {}).get("remote-release")
    plan_gates = plan.get("gates")
    accepted = aggregate.get("accepted_gates")
    if not all(isinstance(value, list) for value in (catalog_gates, profile_ids, plan_gates, accepted)):
        raise ReportError("qualification bundle lacks gate lists")
    definitions = {
        gate.get("id"): gate
        for gate in catalog_gates
        if isinstance(gate, dict) and isinstance(gate.get("id"), str)
    }
    plan_by_id = {
        gate.get("id"): gate
        for gate in plan_gates
        if isinstance(gate, dict) and isinstance(gate.get("id"), str)
    }
    accepted_by_id = {
        gate.get("gate_id"): gate
        for gate in accepted
        if isinstance(gate, dict) and isinstance(gate.get("gate_id"), str)
    }
    expected_ids = list(profile_ids)
    if (
        len(expected_ids) != len(set(expected_ids))
        or set(plan_by_id) != set(expected_ids)
        or set(accepted_by_id) != set(expected_ids)
        or aggregate.get("gate_count") != len(expected_ids)
        or aggregate.get("fresh_count", 0) + aggregate.get("reused_count", 0) != len(expected_ids)
    ):
        raise ReportError("qualification bundle does not cover the exact remote-release profile")
    coverage = catalog.get("remote_release_legacy_coverage", {})
    if (
        coverage.get("required_legacy_count") != 108
        or coverage.get("profile_legacy_count") != 108
        or coverage.get("unautomated_legacy_ids") != []
    ):
        raise ReportError("gate catalog no longer proves all 108 legacy release requirements")

    receipt_paths: dict[str, list[Path]] = {}
    for path in evidence_root.rglob("receipt.json"):
        receipt = read_object(path, "gate receipt")
        if receipt.get("candidate_sha") == candidate and isinstance(receipt.get("gate_id"), str):
            receipt_paths.setdefault(receipt["gate_id"], []).append(path)

    gate_rows: list[dict[str, Any]] = []
    starts: list[str] = []
    ends: list[str] = []
    for sequence, gate_id in enumerate(expected_ids, start=1):
        definition = definitions.get(gate_id)
        planned = plan_by_id[gate_id]
        accepted_gate = accepted_by_id[gate_id]
        source = accepted_gate.get("source")
        if not isinstance(definition, dict) or source not in {"fresh", "reused"}:
            raise ReportError(f"{gate_id}: accepted gate has no catalog definition or source")
        if source == "fresh":
            paths = receipt_paths.get(gate_id, [])
            if len(paths) != 1:
                raise ReportError(f"{gate_id}: expected exactly one exact-candidate receipt")
            receipt = read_object(paths[0], f"receipt for {gate_id}")
            log_verification = "rehashed"
        else:
            receipt = planned.get("reuse_receipt")
            if not isinstance(receipt, dict) or sha256_bytes(canonical_bytes(receipt)) != planned.get(
                "reuse_receipt_sha256"
            ):
                raise ReportError(f"{gate_id}: reused receipt does not match the plan")
            log_verification = "collector-accepted-reuse"
        if receipt.get("schema") != RECEIPT_SCHEMA:
            raise ReportError(f"{gate_id}: unsupported receipt schema")
        expected_receipt_hash = sha256_bytes(
            canonical_bytes({key: value for key, value in receipt.items() if not key.startswith("_")})
        )
        if (
            receipt.get("status") != "PASS"
            or accepted_gate.get("receipt_sha256") != expected_receipt_hash
            or accepted_gate.get("receipt_candidate_sha") != receipt.get("candidate_sha")
            or accepted_gate.get("input_sha256") != planned.get("input_sha256")
            or receipt.get("gate_definition_sha256") != planned.get("gate_definition_sha256")
            or receipt.get("environment_sha256") != planned.get("environment_sha256")
        ):
            raise ReportError(f"{gate_id}: receipt is not the accepted PASS record")
        log_record = receipt.get("log")
        if not isinstance(log_record, dict) or not isinstance(log_record.get("path"), str):
            raise ReportError(f"{gate_id}: receipt has no command-log identity")
        if source == "fresh":
            log_path = safe_relative(evidence_root, log_record["path"], "command log")
            if not log_path.is_file() or sha256_path(log_path) != log_record.get("sha256"):
                raise ReportError(f"{gate_id}: command-log hash mismatch")
        started = receipt.get("started_at")
        ended = receipt.get("ended_at")
        if source == "fresh" and isinstance(started, str):
            starts.append(started)
        if source == "fresh" and isinstance(ended, str):
            ends.append(ended)
        gate_rows.append(
            {
                "sequence": sequence,
                "gate_id": gate_id,
                "name": definition.get("name"),
                "category": definition.get("category"),
                "kind": definition.get("kind"),
                "executor": definition.get("executor"),
                "resource_class": definition.get("resource_class"),
                "display_command": definition.get("display_command"),
                "source": source,
                "receipt_candidate_sha": receipt.get("candidate_sha"),
                "command_log_verification": log_verification,
                "duration_seconds": receipt.get("duration_seconds"),
                "receipt_sha256": expected_receipt_hash,
                "command_log_sha256": log_record.get("sha256"),
            }
        )

    measurements: list[dict[str, Any]] = []
    perf_root = evidence_root / "_perf-results"
    if not perf_root.is_dir():
        raise ReportError("qualification bundle has no archived performance results")
    for path in sorted(perf_root.rglob("*.json")):
        payload = read_object(path, "performance result")
        scenario = payload.get("scenario")
        environment = payload.get("environment")
        if not isinstance(scenario, str) or not isinstance(environment, dict):
            continue
        if not ("results" in payload or "headline" in payload):
            continue
        if (
            environment.get("git_commit") != candidate
            or environment.get("git_dirty") is not False
            or environment.get("rvoip_sip_version") != version
        ):
            raise ReportError(f"{path}: performance result is not exact-candidate {version} evidence")
        results = payload.get("results") if isinstance(payload.get("results"), dict) else {}
        headline = payload.get("headline") if isinstance(payload.get("headline"), dict) else {}
        load = payload.get("load") if isinstance(payload.get("load"), dict) else {}
        latency = payload.get("latency_ns") if isinstance(payload.get("latency_ns"), dict) else {}
        setup = latency.get("setup_latency") if isinstance(latency.get("setup_latency"), dict) else {}
        resources = payload.get("resources") if isinstance(payload.get("resources"), dict) else {}
        relative = path.relative_to(evidence_root).as_posix()
        measurements.append(
            {
                "path": relative,
                "sha256": sha256_path(path),
                "scenario": scenario,
                "kind": "sweep" if "headline" in payload else "point",
                "target_cps": load.get("target_cps", headline.get("operating_point")),
                "achieved_cps": results.get("achieved_cps", headline.get("achieved")),
                "asr": results.get("asr", headline.get("ratio") if headline.get("ratio_label") == "ASR" else None),
                "calls_offered": results.get("calls_offered"),
                "calls_succeeded": results.get("calls_succeeded"),
                "setup_p99_ms": setup.get("p99") / 1_000_000 if isinstance(setup.get("p99"), (int, float)) else None,
                "peak_rss_mb": resources.get("peak_rss_mb"),
                "duration_secs": payload.get("duration_secs"),
            }
        )
    reused_performance_gates = sum(
        1
        for row in gate_rows
        if row["category"] == "Performance and resiliency" and row["source"] == "reused"
    )
    if not measurements and reused_performance_gates == 0:
        raise ReportError("qualification bundle has no exact-candidate performance measurements")

    summary = {
        "schema": SUMMARY_SCHEMA,
        "release": {"version": version, "candidate_sha": candidate},
        "qualification": {
            "profile": "remote-release",
            "status": "PASS",
            "environment_id": aggregate.get("environment_id"),
            "generated_at": aggregate.get("generated_at"),
            "started_at": min(starts) if starts else None,
            "ended_at": max(ends) if ends else None,
            "gate_count": len(gate_rows),
            "fresh_count": aggregate.get("fresh_count"),
            "reused_count": aggregate.get("reused_count"),
            "legacy_required_count": coverage.get("required_legacy_count"),
            "legacy_covered_count": coverage.get("profile_legacy_count"),
            "reused_performance_gate_count": reused_performance_gates,
        },
        "inputs": {
            "catalog_sha256": expected_catalog_hash,
            "plan_sha256": sha256_path(plan_path),
            "aggregate_sha256": sha256_path(aggregate_path),
        },
        "categories": dict(sorted(Counter(row["category"] for row in gate_rows).items())),
        "gates": gate_rows,
        "performance_measurements": measurements,
    }
    return summary, gate_rows, measurements


def render_release(summary: dict[str, Any], provenance: dict[str, str]) -> str:
    release = summary["release"]
    qualification = summary["qualification"]
    lines = [
        f"# RVoIP {release['version']} Release Qualification Report",
        "",
        f"> Generated from the protected `remote-release` run [{provenance['run_id']}]({provenance['run_url']}). No gate was rerun and no measurement was edited during report generation.",
        "",
        "## Qualification",
        "",
        "| Field | Value |",
        "|---|---|",
        "| Status | **PASS — RELEASE-CANDIDATE** |",
        f"| Workspace release | `{release['version']}` |",
        f"| Tested commit | `{release['candidate_sha']}` |",
        f"| Profile | `{qualification['profile']}` |",
        f"| Gates | **{qualification['gate_count']}/{qualification['gate_count']} passed** |",
        f"| Fresh / reused | `{qualification['fresh_count']}` / `{qualification['reused_count']}` |",
        f"| Legacy release requirements | **{qualification['legacy_covered_count']}/{qualification['legacy_required_count']} covered** |",
        f"| Run window | `{qualification['started_at']}` to `{qualification['ended_at']}` |",
        f"| Environment | `{qualification['environment_id']}` |",
        f"| Evidence artifact | `{provenance['artifact_id']}` / `{provenance['artifact_digest']}` |",
        "",
        "The [complete gate record](BETA_GATE_REPORT.md), [performance observations](BETA_PERFORMANCE_REPORT.md), and [machine summary](QUALIFICATION_SUMMARY.json) are derived from the same accepted receipts.",
        "",
        "## Category totals",
        "",
        "| Category | Passed |",
        "|---|---:|",
    ]
    for category, count in summary["categories"].items():
        lines.append(f"| {md_cell(category)} | {count} |")
    lines.extend(
        [
            "",
            "## Evidence integrity",
            "",
            f"- Gate catalog: `{summary['inputs']['catalog_sha256']}`",
            f"- Qualification plan: `{summary['inputs']['plan_sha256']}`",
            f"- Qualification aggregate: `{summary['inputs']['aggregate_sha256']}`",
            f"- GitHub artifact archive: `{provenance['artifact_digest']}`",
            "- Every fresh gate receipt and command log was rehashed before rendering; any reused receipt remains explicitly identified and was input-bound by the qualification collector.",
            "- Every published performance row is bound to the tested commit, a clean tree, and rvoip-sip at the release version.",
            "",
            "## Claim boundary",
            "",
            "PASS applies only to the exact source commit, gate catalog, commands, feature bundles, peer images, environments, limits, and measurements recorded by this run. It is not a general carrier certification or a performance SLA. Production remote-endpoint NAT/TLS/SDES qualification remains separately tracked until live two-UA evidence is recorded.",
            "",
        ]
    )
    return "\n".join(lines)


def render_gates(summary: dict[str, Any], rows: list[dict[str, Any]], provenance: dict[str, str]) -> str:
    count = len(rows)
    lines = [
        "# RVoIP Release Gate Report",
        "",
        f"> Exact accepted-gate ledger for [{provenance['run_id']}]({provenance['run_url']}) at `{summary['release']['candidate_sha']}`.",
        "",
        "## Result",
        "",
        f"**PASS — {count}/{count} remote-release gates passed; 0 failed.** All 108 requirements inherited from the strict beta ledger are covered, and the expanded profile adds current crate, feature-bundle, security, interoperability, and remote-host checks.",
        "",
        "| # | Gate | Category | Executor | Source | Receipt candidate | Seconds | Receipt SHA-256 | Command-log SHA-256 |",
        "|---:|---|---|---|---|---|---:|---|---|",
    ]
    for row in rows:
        lines.append(
            f"| {row['sequence']} | `{md_cell(row['gate_id'])}` — {md_cell(row['name'])} | {md_cell(row['category'])} | `{md_cell(row['executor'])}` | `{md_cell(row['source'])}` | `{row['receipt_candidate_sha']}` | {number(row['duration_seconds'], 3)} | `{row['receipt_sha256']}` | `{row['command_log_sha256']}` |"
        )
    lines.extend(
        [
            "",
            "## Command scope",
            "",
            "The catalog display command names the reviewed scope. The immutable command log named by each receipt is authoritative for the exact invocation.",
            "",
            "| Gate | Reviewed command/scope | Resource class |",
            "|---|---|---|",
        ]
    )
    for row in rows:
        lines.append(
            f"| `{md_cell(row['gate_id'])}` | `{md_cell(row['display_command'])}` | `{md_cell(row['resource_class'])}` |"
        )
    lines.append("")
    return "\n".join(lines)


def render_performance(summary: dict[str, Any], rows: list[dict[str, Any]], provenance: dict[str, str]) -> str:
    lines = [
        "# RVoIP Performance Qualification Report",
        "",
        f"> Exact-candidate observations archived by protected run [{provenance['run_id']}]({provenance['run_url']}). A blank cell means the scenario does not emit that metric; it does not mean zero. Measurements belonging only to an accepted prior-run receipt are not copied into this table.",
        "",
        f"Release: `{summary['release']['version']}` · commit: `{summary['release']['candidate_sha']}` · qualification: **PASS**.",
        "",
        "## Archived measurements",
        "",
        "| Scenario | Kind | Target CPS | Achieved CPS | ASR | Calls | Setup p99 ms | Peak RSS MB | Duration s | Evidence SHA-256 |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---|",
    ]
    for row in rows:
        offered = row["calls_offered"]
        succeeded = row["calls_succeeded"]
        calls = "—" if offered is None and succeeded is None else f"{md_cell(succeeded)}/{md_cell(offered)}"
        lines.append(
            f"| `{md_cell(row['scenario'])}` | `{row['kind']}` | {number(row['target_cps'])} | {number(row['achieved_cps'])} | {number(row['asr'], 4)} | {calls} | {number(row['setup_p99_ms'], 3)} | {number(row['peak_rss_mb'])} | {number(row['duration_secs'])} | `{row['sha256']}` |"
        )
    lines.extend(
        [
            "",
            "## Interpretation",
            "",
            "- The rows are observations, not individually invented PASS verdicts. Their governing performance, soak, regression, cleanup, and evidence-integrity gates are PASS in the complete gate report.",
            "- The supported general full-media beta claim remains up to 2,000 CPS with media enabled. Results above 2,000 CPS remain tuned or experimental and require their own topology, hardware, and qualification evidence.",
            "- Call-setup sweeps use loopback networking on the recorded GCP qualification host. They establish repeatable release regression evidence, not public-network latency or carrier capacity.",
            "- Full JSON, resource windows, diagnostics, and scenario-specific counters remain in the GitHub evidence artifact; this report intentionally avoids flattening non-equivalent metrics into one score.",
            "",
            "## Evidence paths",
            "",
        ]
    )
    for row in rows:
        lines.append(f"- `{row['path']}` — `{row['sha256']}`")
    lines.append("")
    return "\n".join(lines)


def write_bundle(
    *,
    output_dir: Path,
    summary: dict[str, Any],
    rows: list[dict[str, Any]],
    measurements: list[dict[str, Any]],
    provenance: dict[str, str],
    generator: Path,
) -> None:
    run_id = provenance.get("run_id", "")
    run_url = provenance.get("run_url", "")
    artifact_id = provenance.get("artifact_id", "")
    artifact_digest = provenance.get("artifact_digest", "")
    if not run_id.isdigit() or run_url != f"https://github.com/eisenzopf/rvoip/actions/runs/{run_id}":
        raise ReportError("qualification provenance has an invalid canonical run URL")
    if artifact_id != "pending-upload" and not artifact_id.isdigit():
        raise ReportError("qualification provenance has an invalid artifact ID")
    if artifact_digest != "pending-upload" and not (
        artifact_digest.startswith("sha256:") and SHA256_RE.fullmatch(artifact_digest[7:])
    ):
        raise ReportError("qualification provenance has an invalid artifact digest")
    summary = {
        **summary,
        "provenance": {
            "run_id": run_id,
            "run_url": run_url,
            "artifact_id": artifact_id,
            "artifact_digest": artifact_digest,
        },
    }
    output_dir.mkdir(parents=True, exist_ok=True)
    rendered = {
        "BETA_RELEASE_REPORT.md": render_release(summary, provenance).encode(),
        "BETA_GATE_REPORT.md": render_gates(summary, rows, provenance).encode(),
        "BETA_PERFORMANCE_REPORT.md": render_performance(summary, measurements, provenance).encode(),
        "QUALIFICATION_SUMMARY.json": pretty_bytes(summary),
    }
    for name, payload in rendered.items():
        (output_dir / name).write_bytes(payload)
    attestation = {
        "schema": ATTESTATION_SCHEMA,
        "generated_at": summary["qualification"]["generated_at"],
        "release": summary["release"],
        "qualification": {
            "run_id": provenance["run_id"],
            "run_url": provenance["run_url"],
            "artifact_id": provenance["artifact_id"],
            "artifact_digest": provenance["artifact_digest"],
            "aggregate_sha256": summary["inputs"]["aggregate_sha256"],
            "plan_sha256": summary["inputs"]["plan_sha256"],
            "catalog_sha256": summary["inputs"]["catalog_sha256"],
        },
        "generator": {
            "path": "scripts/release/render_qualification_reports.py",
            "sha256": sha256_path(generator),
        },
        "files": {
            name: {"sha256": sha256_bytes(payload), "bytes": len(payload)}
            for name, payload in sorted(rendered.items())
        },
    }
    attestation_path = output_dir / ATTESTATION_FILES[0]
    attestation_path.write_bytes(pretty_bytes(attestation))
    digest = sha256_path(attestation_path)
    (output_dir / ATTESTATION_FILES[1]).write_text(f"{digest}  {ATTESTATION_FILES[0]}\n")


def verify_bundle(directory: Path, generator: Path) -> None:
    attestation_path = directory / ATTESTATION_FILES[0]
    checksum_path = directory / ATTESTATION_FILES[1]
    attestation = read_object(attestation_path, "report attestation")
    if attestation.get("schema") != ATTESTATION_SCHEMA:
        raise ReportError("unsupported report attestation schema")
    expected_checksum = f"{sha256_path(attestation_path)}  {ATTESTATION_FILES[0]}"
    if checksum_path.read_text().strip() != expected_checksum:
        raise ReportError("report attestation checksum mismatch")
    if attestation.get("generator", {}).get("sha256") != sha256_path(generator):
        raise ReportError("report generator hash mismatch")
    files = attestation.get("files")
    if not isinstance(files, dict) or set(files) != set(REPORT_FILES):
        raise ReportError("report attestation has an incomplete file inventory")
    for name in REPORT_FILES:
        record = files[name]
        path = directory / name
        if (
            not path.is_file()
            or not isinstance(record, dict)
            or record.get("sha256") != sha256_path(path)
            or record.get("bytes") != path.stat().st_size
        ):
            raise ReportError(f"report file mismatch: {name}")
    summary = read_object(directory / "QUALIFICATION_SUMMARY.json", "qualification summary")
    if summary.get("schema") != SUMMARY_SCHEMA or summary.get("qualification", {}).get("status") != "PASS":
        raise ReportError("report summary is not a qualified PASS")


def promote(source: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    for name in REPORT_FILES + ATTESTATION_FILES:
        shutil.copy2(source / name, destination / name)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)
    generate = subparsers.add_parser("generate")
    generate.add_argument("--catalog", type=Path, required=True)
    generate.add_argument("--plan", type=Path, required=True)
    generate.add_argument("--aggregate", type=Path, required=True)
    generate.add_argument("--evidence", type=Path, required=True)
    generate.add_argument("--version", required=True)
    generate.add_argument("--run-id", required=True)
    generate.add_argument("--run-url", required=True)
    generate.add_argument("--artifact-id", default="pending-upload")
    generate.add_argument("--artifact-digest", default="pending-upload")
    generate.add_argument("--output-dir", type=Path, required=True)
    generate.add_argument("--promote-to", type=Path)
    verify = subparsers.add_parser("verify")
    verify.add_argument("--directory", type=Path, required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    generator = Path(__file__).resolve()
    try:
        if args.command == "generate":
            if not SEMVER_RE.fullmatch(args.version):
                raise ReportError("release version must be stable SemVer")
            summary, rows, measurements = load_bundle(
                catalog_path=args.catalog,
                plan_path=args.plan,
                aggregate_path=args.aggregate,
                evidence_root=args.evidence,
                version=args.version,
            )
            provenance = {
                "run_id": args.run_id,
                "run_url": args.run_url,
                "artifact_id": args.artifact_id,
                "artifact_digest": args.artifact_digest,
            }
            write_bundle(
                output_dir=args.output_dir,
                summary=summary,
                rows=rows,
                measurements=measurements,
                provenance=provenance,
                generator=generator,
            )
            verify_bundle(args.output_dir, generator)
            if args.promote_to:
                promote(args.output_dir, args.promote_to)
                verify_bundle(args.promote_to, generator)
        else:
            verify_bundle(args.directory, generator)
    except (ReportError, OSError) as error:
        print(f"qualification report error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
