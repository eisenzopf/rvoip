#!/usr/bin/env python3
"""Generate, verify, and promote evidence-complete beta release reports.

The current candidate is a post-run derivation from a v1 attestation. This
tool never executes a beta gate. Future beta runs also use its structured
configuration and per-gate recording commands so Markdown is not an input.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


SCHEMA_EVIDENCE = "rvoip-sip-release-evidence-v1"
SCHEMA_CONFIG = "rvoip-sip-effective-gate-config-v1"
SCHEMA_RESULTS = "rvoip-sip-gate-results-v1"
SCHEMA_REPORT_ATTESTATION = "rvoip-sip-report-attestation-v1"
EXPECTED_SOURCE_SCHEMAS = {
    "rvoip-sip-beta-attestation-v1",
    "rvoip-sip-beta-attestation-v2",
}
KNOWN_VALIDATORS = {
    "status-pass",
    "evidence-hash",
    "source-fingerprint",
    "command-exit-zero",
    "interop-result",
    "performance-result",
    "reporting-check",
}
LEGACY_LIFECYCLE_GATES = {
    "SIPp standalone target start",
    "SIPp standalone target stop",
}
REPORT_FILES = (
    "BETA_RELEASE_REPORT.md",
    "BETA_GATE_REPORT.md",
    "BETA_PERFORMANCE_REPORT.md",
)
MACHINE_FILES = (
    "effective-gate-config.json",
    "gate-results.json",
    "release-evidence.json",
)
ATTESTATION_FILES = ("report-attestation.json", "report-attestation.json.sha256")
INPUT_FILES = (
    "inputs/beta-release-policy.yaml",
    "inputs/beta_release_report.py",
)
ALL_GENERATED_FILES = REPORT_FILES + MACHINE_FILES + INPUT_FILES + ATTESTATION_FILES
SECRET_RE = re.compile(
    r"(?i)(-----BEGIN [A-Z ]*PRIVATE KEY-----|"
    r"(?:password|passwd|secret|access[_-]?token|api[_-]?key)\s*[=:]\s*\S+)"
)
ABSOLUTE_USER_PATH_RE = re.compile(r"/Users/[^/\s`\"']+")
MARKDOWN_LINK_RE = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
LATENCY_FAMILIES = ("setup_latency", "full_cycle")
LATENCY_PERCENTILES = ("p50", "p95", "p99")


class ReportError(RuntimeError):
    """A fail-closed reporting or verification error."""


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n").encode()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_path(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise ReportError(f"cannot read JSON {path}: {exc}") from exc


def write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as tmp:
        tmp.write(data)
        temporary = Path(tmp.name)
    os.replace(temporary, path)


def sanitize_text(value: Any) -> Any:
    """Remove user-specific absolute paths recursively."""
    if isinstance(value, dict):
        return {str(key): sanitize_text(item) for key, item in value.items()}
    if isinstance(value, list):
        return [sanitize_text(item) for item in value]
    if not isinstance(value, str):
        return value
    value = re.sub(
        r"/Users/[^/\s`\"']+/Developer/rvoip-beta-evidence/[^\s`\"']+",
        "<source-report>",
        value,
    )
    value = re.sub(
        r"/Users/[^/\s`\"']+/Developer/rvoip(?=[/\s:`\"']|$)",
        "<workspace>",
        value,
    )
    value = re.sub(r"/Users/[^/\s`\"']+(?:/[^\s`\"']*)?", "<local-path>", value)
    return value


def safe_relative(path: str) -> bool:
    candidate = Path(path)
    return (
        bool(path)
        and not candidate.is_absolute()
        and ".." not in candidate.parts
        and "\x00" not in path
    )


def load_policy(path: Path) -> dict[str, Any]:
    policy = read_json(path)
    if policy.get("schema") != "rvoip-sip-beta-release-policy-v1":
        raise ReportError(f"unsupported policy schema in {path}")
    validate_policy(policy)
    return policy


def expand_catalog(policy: dict[str, Any]) -> list[dict[str, Any]]:
    gates: list[dict[str, Any]] = []
    for group in policy.get("gate_groups", []):
        kind_name = group.get("kind")
        kind = policy["gate_kinds"].get(kind_name)
        if not kind:
            raise ReportError(f"unknown gate kind {kind_name!r}")
        for raw in group.get("gates", []):
            if not isinstance(raw, list) or len(raw) not in (2, 3):
                raise ReportError(f"invalid gate catalog entry: {raw!r}")
            condition = raw[2] if len(raw) == 3 else {}
            if not isinstance(condition, dict):
                raise ReportError(f"invalid gate condition: {raw!r}")
            gate = {
                "id": raw[0],
                "name": raw[1],
                "category": group["category"],
                "kind": kind_name,
                "modes": list(group.get("modes", [])),
                "condition": condition,
                **kind,
            }
            gate["purpose"] = f"{kind['purpose']} Named scope: {raw[1]}."
            gates.append(gate)
    return gates


def validate_policy(policy: dict[str, Any]) -> None:
    definitions = policy.get("configuration", {})
    if not definitions:
        raise ReportError("policy has no configuration definitions")
    gates = expand_catalog(policy)
    ids = [gate["id"] for gate in gates]
    names = [gate["name"] for gate in gates]
    if len(ids) != len(set(ids)):
        raise ReportError(f"duplicate catalog gate IDs: {_duplicates(ids)}")
    if len(names) != len(set(names)):
        raise ReportError(f"duplicate catalog gate names: {_duplicates(names)}")
    valid_modes = {"local", "full", "interop", "perf", "security"}
    for gate in gates:
        if not gate["id"] or not re.fullmatch(r"[a-z0-9][a-z0-9.-]*", gate["id"]):
            raise ReportError(f"invalid stable gate ID {gate['id']!r}")
        if not gate["modes"] or not set(gate["modes"]) <= valid_modes:
            raise ReportError(f"invalid or unreachable modes for {gate['id']}")
        validators = set(gate.get("validators", []))
        if not validators or not validators <= KNOWN_VALIDATORS:
            raise ReportError(f"unknown or missing validators for {gate['id']}: {validators}")
        for key in (
            "purpose",
            "components",
            "configuration_scope",
            "required_evidence",
            "pass_meaning",
            "non_claims",
        ):
            if not gate.get(key):
                raise ReportError(f"catalog gate {gate['id']} is missing {key}")
        unknown_configuration = set(gate["configuration_scope"]) - set(definitions)
        if unknown_configuration:
            raise ReportError(
                f"catalog gate {gate['id']} has unknown configuration scope "
                f"{sorted(unknown_configuration)}"
            )
        for operator in ("when", "unless"):
            key = gate["condition"].get(operator)
            if key and key not in definitions:
                raise ReportError(f"catalog gate {gate['id']} uses unknown condition {key}")
    current = policy.get("current_candidate", {})
    if current.get("expected_selected_gate_count", 0) <= 0:
        raise ReportError("policy lacks a current-candidate gate count")


def _duplicates(values: Iterable[str]) -> list[str]:
    counts = Counter(values)
    return sorted(value for value, count in counts.items() if count > 1)


def parse_redacted_environment(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ReportError(f"missing redacted environment evidence: {path}")
    values: dict[str, str] = {}
    for line in path.read_text().splitlines():
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        normalized = key.strip().lower()
        if normalized in values:
            raise ReportError(f"duplicate redacted environment key {key}")
        values[normalized] = value.strip()
    return values


def convert_typed(value: Any, definition: dict[str, Any] | None) -> Any:
    if definition is None:
        if isinstance(value, str) and value in {"0", "1"}:
            return value == "1"
        if isinstance(value, str) and re.fullmatch(r"-?\d+", value):
            return int(value)
        if isinstance(value, str) and re.fullmatch(r"-?\d+\.\d+", value):
            return float(value)
        return sanitize_text(value)
    kind = definition["type"]
    if value is None:
        return None
    if kind == "boolean":
        if isinstance(value, bool):
            return value
        lowered = str(value).strip().lower()
        if lowered in {"1", "true", "yes", "on"}:
            return True
        if lowered in {"0", "false", "no", "off"}:
            return False
        raise ReportError(f"invalid boolean value {value!r}")
    if kind == "integer":
        return int(value)
    if kind == "number":
        return float(value)
    if kind == "string-list":
        if isinstance(value, list):
            return sanitize_text(value)
        return [item for item in re.split(r"[\s,]+", str(value).strip()) if item]
    if kind == "integer-list":
        if isinstance(value, list):
            items = value
        else:
            items = [item for item in re.split(r"[\s,]+", str(value).strip()) if item]
        return [int(item) for item in items]
    if kind == "path-list":
        if isinstance(value, list):
            items = value
        else:
            items = str(value).split(":") if value else []
        return sanitize_text(items)
    return sanitize_text(value)


def effective_configuration(
    report_root: Path, attestation: dict[str, Any], policy: dict[str, Any]
) -> dict[str, Any]:
    raw_attested = dict(attestation.get("configuration", {}).get("effective_gate_config", {}))
    raw_environment = parse_redacted_environment(report_root / "environment/beta-env-redacted.txt")
    raw_attested["beta_gate_mode"] = attestation.get("run", {}).get("mode")
    definitions = policy["configuration"]
    keys = set(raw_attested) | set(raw_environment) | set(definitions)
    values: list[dict[str, Any]] = []
    typed_by_key: dict[str, Any] = {}
    mode = raw_attested["beta_gate_mode"]
    for key in sorted(keys):
        definition = definitions.get(key)
        if key in raw_environment:
            raw = raw_environment[key]
            origin = "environment-override"
        elif key in raw_attested:
            raw = raw_attested[key]
            origin = "derived-from-v1-attestation"
        elif definition and mode == "full" and "full_default" in definition:
            raw = definition["full_default"]
            origin = "policy-default"
        elif definition and "default" in definition:
            raw = definition["default"]
            origin = "policy-default"
        else:
            continue
        typed = convert_typed(raw, definition)
        typed_by_key[key] = typed
        values.append(
            {
                "key": key,
                "type": definition["type"] if definition else type(typed).__name__,
                "value": typed,
                "source": origin,
            }
        )
    required_release_values = {
        "rvoip_perf_skip_audio_frame_delivery": False,
        "beta_perf_high_density_burst_cps": 160,
        "beta_perf_high_density_min_asr": 0.995,
        "beta_perf_high_density_rss_limit_mb_per_hr": 15.0,
    }
    for key, expected in required_release_values.items():
        actual = typed_by_key.get(key)
        if actual != expected:
            raise ReportError(f"release configuration {key}={actual!r}, expected {expected!r}")
    return {
        "schema": SCHEMA_CONFIG,
        "policy_version": policy["policy_version"],
        "mode": mode,
        "binding_mode": "post-run-v1-backfill",
        "values": values,
        "values_by_key": typed_by_key,
    }


def gate_selected(gate: dict[str, Any], mode: str, config: dict[str, Any]) -> bool:
    if mode not in gate["modes"]:
        return False
    condition = gate["condition"]
    if condition.get("when") and not bool(config.get(condition["when"])):
        return False
    if condition.get("unless") and bool(config.get(condition["unless"])):
        return False
    return True


def artifact_index(attestation: dict[str, Any]) -> dict[str, dict[str, Any]]:
    files = attestation.get("artifacts", {}).get("files", [])
    index: dict[str, dict[str, Any]] = {}
    for artifact in files:
        path = artifact.get("path")
        if not safe_relative(str(path)):
            raise ReportError(f"unsafe attested artifact path {path!r}")
        if path in index:
            raise ReportError(f"duplicate attested artifact path {path}")
        index[path] = artifact
    return index


def verify_artifact(
    report_root: Path, index: dict[str, dict[str, Any]], relative: str
) -> dict[str, Any]:
    if not safe_relative(relative):
        raise ReportError(f"unsafe evidence path {relative!r}")
    artifact = index.get(relative)
    if artifact is None:
        raise ReportError(f"evidence is absent from source attestation: {relative}")
    path = report_root / relative
    if not path.is_file():
        raise ReportError(f"evidence file is missing: {relative}")
    actual = sha256_path(path)
    if actual != artifact.get("sha256"):
        raise ReportError(f"evidence hash mismatch: {relative}")
    return {
        "path": relative,
        "sha256": actual,
        "bytes": path.stat().st_size,
        "kind": artifact.get("kind", "artifact"),
    }


def parse_log_metadata(path: Path) -> dict[str, str]:
    metadata: dict[str, str] = {}
    if not path.is_file():
        return metadata
    for line in path.read_text(errors="replace").splitlines():
        if ": " in line:
            key, value = line.split(": ", 1)
            if key in {
                "gate",
                "started_at_utc",
                "ended_at_utc",
                "workspace",
                "command",
                "duration_seconds",
                "exit_status",
            }:
                metadata[key] = value
    return metadata


def relevant_configuration(category: str, config: dict[str, Any]) -> list[str]:
    keys = sorted(config)
    if category == "Source integrity":
        prefixes = ("beta_require_", "beta_canonical_", "beta_state_", "git_")
    elif category == "Security":
        prefixes = ("beta_gate_require_external", "beta_fuzz", "beta_run_fuzz")
    elif category == "PBX and interoperability":
        prefixes = ("beta_pbx_", "beta_run_", "beta_restore_", "beta_sipp_")
    elif category == "Performance and resiliency" or category == "Reporting and regression":
        prefixes = ("beta_perf_", "beta_profile_", "beta_run_", "beta_burst_", "rvoip_perf_")
    else:
        prefixes = ("beta_attestation_", "beta_deny_", "rvoip_require_api_tools")
    selected = [key for key in keys if key.startswith(prefixes)]
    return selected[:24]


def gate_reason(gate: dict[str, Any]) -> str:
    condition = gate["condition"]
    if condition.get("when"):
        return (
            f"Required in full mode because `{condition['when']}` was enabled; "
            "scheduled conditional gates are release-blocking."
        )
    if condition.get("unless"):
        return (
            f"Required in full mode unless `{condition['unless']}` is enabled; "
            "the recorded configuration selected this path."
        )
    return "Unconditionally required by the full beta release profile."


def ancillary_gate_evidence(name: str) -> list[str]:
    if name in {"local Asterisk PBX matrix", "local FreeSWITCH PBX matrix"}:
        return ["pbx/matrix.tsv", "pbx/summary.md"]
    if name == "SIPp standalone matrix":
        return ["sipp/runs.tsv", "sipp/run_summary.md"]
    if name == "baresip strict-UA matrix":
        return ["strict-ua/matrix.tsv", "strict-ua/summary.md"]
    if name == "dependency advisory audit":
        return [
            "security/cargo-audit.txt",
            "security/cargo-audit.json",
            "security/accepted-advisories.md",
        ]
    match = re.fullmatch(r"parser fuzz smoke \(([^)]+)\)", name)
    if match:
        return [
            f"security/fuzz/{match.group(1)}.log",
            f"security/fuzz/{match.group(1)}.version.txt",
        ]
    return []


def interop_observed_check(report_root: Path, name: str) -> dict[str, Any] | None:
    if name in {"local Asterisk PBX matrix", "local FreeSWITCH PBX matrix"}:
        provider = "asterisk" if "Asterisk" in name else "freeswitch"
        path = report_root / "pbx/matrix.tsv"
        lines = path.read_text().splitlines()
        header = lines[0].split("\t")
        status_index = header.index("status")
        provider_index = header.index("provider")
        statuses = [
            cells[status_index]
            for line in lines[1:]
            if (cells := line.split("\t"))[provider_index] == provider
        ]
        return {
            "check": f"{provider} PBX matrix rows",
            "observed": {"rows": len(statuses), "pass": statuses.count("PASS")},
            "passed": bool(statuses) and all(status == "PASS" for status in statuses),
        }
    if name == "SIPp standalone matrix":
        counts = count_tsv_results(report_root / "sipp/runs.tsv")
        return {
            "check": "SIPp matrix rows",
            "observed": counts,
            "passed": counts.get("rows") == 5 and counts.get("PASS") == 5,
        }
    if name == "baresip strict-UA matrix":
        counts = count_tsv_results(report_root / "strict-ua/matrix.tsv")
        return {
            "check": "strict-UA matrix rows",
            "observed": counts,
            "passed": counts.get("rows") == 7 and counts.get("PASS") == 7,
        }
    return None


def build_gate_results(
    report_root: Path,
    attestation: dict[str, Any],
    policy: dict[str, Any],
    effective: dict[str, Any],
) -> dict[str, Any]:
    catalog = expand_catalog(policy)
    by_name = {gate["name"]: gate for gate in catalog}
    config = effective["values_by_key"]
    mode = effective["mode"]
    selected = [gate for gate in catalog if gate_selected(gate, mode, config)]
    selected_names = {gate["name"] for gate in selected}
    source_gates = attestation.get("gates", [])
    captured_names = [gate.get("name") for gate in source_gates]
    if len(captured_names) != len(set(captured_names)):
        raise ReportError(f"duplicate recorded gates: {_duplicates(captured_names)}")
    unknown = sorted(set(captured_names) - set(by_name))
    if unknown:
        raise ReportError(f"uncatalogued recorded gates: {unknown}")
    missing = sorted(selected_names - set(captured_names))
    extra = sorted(set(captured_names) - selected_names)
    if missing or extra:
        raise ReportError(f"gate selection mismatch; missing={missing}, unselected={extra}")
    expected = policy["current_candidate"]["expected_selected_gate_count"]
    if len(selected) != expected or len(source_gates) != expected:
        raise ReportError(
            f"current full configuration must select and record {expected} gates; "
            f"selected={len(selected)} recorded={len(source_gates)}"
        )
    artifacts = artifact_index(attestation)
    records: list[dict[str, Any]] = []
    for sequence, recorded in enumerate(source_gates, 1):
        gate = by_name[recorded["name"]]
        if recorded.get("status") != "PASS":
            raise ReportError(f"non-PASS recorded gate {recorded['name']}")
        log_path = recorded.get("log_path")
        evidence = [verify_artifact(report_root, artifacts, log_path)]
        for relative in ancillary_gate_evidence(recorded["name"]):
            if relative != log_path:
                evidence.append(verify_artifact(report_root, artifacts, relative))
        metadata = parse_log_metadata(report_root / log_path)
        legacy = recorded["name"] in LEGACY_LIFECYCLE_GATES
        if legacy:
            command = (
                "managed perf_listener lifecycle operation "
                "(command was not structured separately by the v1 runner)"
            )
            evidence_strength = (
                "legacy-v1-summary-and-shared-listener-log; future runs record "
                "the lifecycle result directly"
            )
            observed = [
                {"check": "recorded status", "observed": "PASS", "passed": True},
                {
                    "check": "shared listener log hash",
                    "observed": evidence[0]["sha256"],
                    "passed": True,
                },
            ]
            timestamps = {
                "started_at_utc": None,
                "ended_at_utc": None,
                "source": "v1 run envelope; per-operation timestamps unavailable",
            }
        else:
            command = metadata.get("command")
            if not command:
                raise ReportError(f"recorded command missing for {recorded['name']}")
            exit_status = metadata.get("exit_status")
            if exit_status != "0":
                raise ReportError(
                    f"zero exit status missing for {recorded['name']}: {exit_status!r}"
                )
            evidence_strength = "direct-v1-gate-log"
            observed = [
                {"check": "recorded status", "observed": "PASS", "passed": True},
                {"check": "command exit status", "observed": 0, "passed": True},
                {
                    "check": "evidence SHA-256",
                    "observed": evidence[0]["sha256"],
                    "passed": True,
                },
            ]
            timestamps = {
                "started_at_utc": metadata.get("started_at_utc"),
                "ended_at_utc": metadata.get("ended_at_utc"),
                "source": "direct-v1-gate-log",
            }
            if not timestamps["started_at_utc"] or not timestamps["ended_at_utc"]:
                raise ReportError(f"timestamps missing for {recorded['name']}")
        interop_check = interop_observed_check(report_root, recorded["name"])
        if interop_check:
            if not interop_check["passed"]:
                raise ReportError(f"interop matrix did not pass for {recorded['name']}")
            observed.append(interop_check)
        records.append(
            {
                "sequence": sequence,
                "id": gate["id"],
                "name": gate["name"],
                "category": gate["category"],
                "kind": gate["kind"],
                "required": True,
                "required_reason": gate_reason(gate),
                "status": "PASS",
                "duration_seconds": recorded.get("duration_seconds"),
                "timestamps": timestamps,
                "sanitized_argv": sanitize_text(command),
                "components": gate["components"],
                "purpose": gate["purpose"],
                "relevant_configuration": relevant_configuration(gate["category"], config),
                "expected_checks": gate["validators"],
                "observed_checks": observed,
                "evidence": evidence,
                "evidence_strength": evidence_strength,
                "pass_meaning": gate["pass_meaning"],
                "non_claims": gate["non_claims"],
            }
        )
    return {
        "schema": SCHEMA_RESULTS,
        "policy_version": policy["policy_version"],
        "run_id": attestation["run"]["id"],
        "mode": mode,
        "binding_mode": "post-run-v1-backfill",
        "required_count": len(records),
        "passed": len(records),
        "failed": 0,
        "skipped": 0,
        "records": records,
    }


def load_native_gate_results(
    report_root: Path,
    attestation: dict[str, Any],
    policy: dict[str, Any],
    effective: dict[str, Any],
) -> dict[str, Any]:
    """Validate and enrich future structured results without reading Markdown."""
    native = read_json(report_root / "gate-results.json")
    if native.get("schema") != SCHEMA_RESULTS:
        raise ReportError("native gate-results.json has an unsupported schema")
    if native.get("binding_mode") != "native-v2-input":
        raise ReportError("native gate-results.json has an invalid binding mode")
    catalog = expand_catalog(policy)
    by_id = {gate["id"]: gate for gate in catalog}
    config = effective["values_by_key"]
    mode = effective["mode"]
    selected = [gate for gate in catalog if gate_selected(gate, mode, config)]
    selected_ids = {gate["id"] for gate in selected}
    source_records = native.get("records", [])
    recorded_ids = [record.get("id") for record in source_records]
    if len(recorded_ids) != len(set(recorded_ids)):
        raise ReportError(f"duplicate native gate IDs: {_duplicates(recorded_ids)}")
    missing = sorted(selected_ids - set(recorded_ids))
    extra = sorted(set(recorded_ids) - selected_ids)
    if missing or extra:
        raise ReportError(f"native gate selection mismatch; missing={missing}, extra={extra}")
    if native.get("failed") or native.get("skipped"):
        raise ReportError("native release reporting requires zero failed and zero skipped gates")
    artifacts = artifact_index(attestation)
    records: list[dict[str, Any]] = []
    for sequence, source in enumerate(source_records, 1):
        gate = by_id.get(source.get("id"))
        if gate is None or source.get("name") != gate["name"]:
            raise ReportError(f"invalid native gate identity at sequence {sequence}")
        if source.get("sequence") != sequence:
            raise ReportError(f"non-contiguous native gate sequence at {gate['id']}")
        if source.get("status") != "PASS":
            raise ReportError(f"native gate {gate['id']} is not PASS")
        raw_evidence = source.get("evidence", [])
        if not raw_evidence:
            raise ReportError(f"native gate {gate['id']} has no evidence")
        evidence = []
        for item in raw_evidence:
            verified = verify_artifact(report_root, artifacts, item.get("path", ""))
            if item.get("sha256") != verified["sha256"]:
                raise ReportError(f"native gate evidence hash drift: {gate['id']}")
            evidence.append(verified)
        argv = source.get("sanitized_argv")
        if not isinstance(argv, list) or not argv:
            raise ReportError(f"native gate {gate['id']} has no structured argv")
        checks = source.get("checks", [])
        if not checks or not all(check.get("passed") for check in checks):
            raise ReportError(f"native gate {gate['id']} has missing or failed checks")
        records.append(
            {
                "sequence": sequence,
                "id": gate["id"],
                "name": gate["name"],
                "category": gate["category"],
                "kind": gate["kind"],
                "required": True,
                "required_reason": gate_reason(gate),
                "status": "PASS",
                "duration_seconds": source.get("duration_seconds"),
                "timestamps": {
                    "started_at_utc": source.get("started_at_utc"),
                    "ended_at_utc": source.get("ended_at_utc"),
                    "source": "native structured gate result",
                },
                "sanitized_argv": argv,
                "components": gate["components"],
                "purpose": gate["purpose"],
                "relevant_configuration": relevant_configuration(gate["category"], config),
                "expected_checks": gate["validators"],
                "observed_checks": checks,
                "evidence": evidence,
                "evidence_strength": "native-structured-gate-result",
                "pass_meaning": gate["pass_meaning"],
                "non_claims": gate["non_claims"],
            }
        )
    return {
        "schema": SCHEMA_RESULTS,
        "policy_version": policy["policy_version"],
        "run_id": attestation["run"]["id"],
        "mode": mode,
        "binding_mode": "native-v2-input",
        "required_count": len(records),
        "passed": len(records),
        "failed": 0,
        "skipped": 0,
        "records": records,
    }


def latency_percentiles(report: dict[str, Any], context: str) -> dict[str, dict[str, int | float]]:
    latency = report.get("latency_ns")
    if not isinstance(latency, dict):
        raise ReportError(f"{context} is missing latency_ns")
    result: dict[str, dict[str, int | float]] = {}
    for family in LATENCY_FAMILIES:
        source = latency.get(family)
        if not isinstance(source, dict):
            raise ReportError(f"{context} is missing latency_ns.{family}")
        values: dict[str, int | float] = {}
        for percentile in LATENCY_PERCENTILES:
            value = source.get(percentile)
            if not isinstance(value, (int, float)) or isinstance(value, bool) or value < 0:
                raise ReportError(
                    f"{context} has invalid latency_ns.{family}.{percentile}"
                )
            values[percentile] = value
        result[family] = values
    return result


def canonical_latency_limits(policy: dict[str, Any]) -> dict[str, dict[str, float]]:
    source = policy.get("release_profile", {}).get("canonical_2k_latency_limits_ms")
    if not isinstance(source, dict):
        raise ReportError("policy is missing canonical 2K latency limits")
    limits: dict[str, dict[str, float]] = {}
    for family in LATENCY_FAMILIES:
        family_source = source.get(family)
        if not isinstance(family_source, dict):
            raise ReportError(f"policy is missing canonical latency limits for {family}")
        family_limits: dict[str, float] = {}
        for percentile in LATENCY_PERCENTILES:
            value = family_source.get(percentile)
            if not isinstance(value, (int, float)) or isinstance(value, bool) or value <= 0:
                raise ReportError(
                    f"policy has invalid canonical latency limit for {family}.{percentile}"
                )
            family_limits[percentile] = float(value)
        limits[family] = family_limits
    return limits


def performance_inventory(
    report_root: Path,
    attestation: dict[str, Any],
    policy: dict[str, Any],
    effective: dict[str, Any],
) -> dict[str, Any]:
    artifacts = artifact_index(attestation)
    files: list[dict[str, Any]] = []
    perf_root = report_root / "perf-results"
    for path in sorted(perf_root.rglob("*.json")):
        relative = path.relative_to(report_root).as_posix()
        evidence = verify_artifact(report_root, artifacts, relative)
        if "/build/" in relative:
            role = "build-provenance"
        elif relative.endswith("/_sweep.json"):
            role = "matrix-summary"
        elif "/perf_call_setup_cps_" in relative:
            role = "matrix-point"
        elif "soak" in relative:
            role = "soak"
        elif "burst" in relative:
            role = "burst"
        else:
            role = "workload"
        files.append({**evidence, "role": role})
    if len(files) != 59:
        raise ReportError(f"expected 59 performance JSON artifacts, found {len(files)}")
    metrics_path = report_root / "performance-gate-metrics.json"
    metrics = read_json(metrics_path)
    if not metrics.get("passed"):
        raise ReportError("packaged performance gate metrics did not pass")
    metrics_evidence = verify_artifact(
        report_root, artifacts, "performance-gate-metrics.json"
    )
    latency_limits_ms = canonical_latency_limits(policy)
    canonical: list[dict[str, Any]] = []
    for sequence in range(1, 4):
        relative = f"canonical-2k/run-{sequence}/report.json"
        report = read_json(report_root / relative)
        result = report.get("results", {})
        cleanup = report.get("diagnostics", {}).get("cleanup_convergence", {})
        latency_ns = latency_percentiles(report, relative)
        latency_checks: list[dict[str, Any]] = []
        for family in LATENCY_FAMILIES:
            for percentile in LATENCY_PERCENTILES:
                observed_ms = float(latency_ns[family][percentile]) / 1_000_000
                limit_ms = latency_limits_ms[family][percentile]
                latency_checks.append(
                    {
                        "family": family,
                        "percentile": percentile,
                        "observed_ms": observed_ms,
                        "limit_ms": limit_ms,
                        "passed": observed_ms <= limit_ms,
                    }
                )
        if not all(check["passed"] for check in latency_checks):
            raise ReportError(f"{relative} exceeds a canonical 2K latency limit")
        canonical.append(
            {
                "sequence": sequence,
                "target_cps": report.get("load", {}).get("target_cps"),
                "achieved_cps": result.get("achieved_cps"),
                "asr": result.get("asr"),
                "calls_offered": result.get("calls_offered"),
                "calls_succeeded": result.get("calls_succeeded"),
                "retained_after_drain": cleanup.get("retained_total"),
                "latency_ns": latency_ns,
                "latency_checks": latency_checks,
                "evidence": verify_artifact(report_root, artifacts, relative),
            }
        )
    matrices: list[dict[str, Any]] = []
    for profile in ("pbx-media-server", "signaling-only-server-high-performance"):
        relative = f"perf-results/perf_call_setup_cps_{profile}/_sweep.json"
        sweep = read_json(report_root / relative)
        points: list[dict[str, Any]] = []
        for point in sweep.get("points", []):
            results = point.get("results", {})
            target_cps = point.get("load", {}).get("target_cps")
            points.append(
                {
                    "target_cps": target_cps,
                    "achieved_cps": results.get("achieved_cps"),
                    "asr": results.get("asr"),
                    "calls_offered": results.get("calls_offered"),
                    "calls_succeeded": results.get("calls_succeeded"),
                    "latency_ns": latency_percentiles(
                        point, f"{relative} point target_cps={target_cps}"
                    ),
                }
            )
        matrices.append(
            {
                "profile": profile,
                "headline": sweep.get("headline", {}),
                "points": points,
                "evidence": verify_artifact(report_root, artifacts, relative),
            }
        )
    endpoint_relative = "perf-results/perf_call_setup_cps_endpoint.json"
    endpoint = read_json(report_root / endpoint_relative)
    matrices.insert(
        0,
        {
            "profile": "endpoint",
            "headline": {},
            "points": [
                {
                    "target_cps": endpoint.get("load", {}).get("target_cps"),
                    "achieved_cps": endpoint.get("results", {}).get("achieved_cps"),
                    "asr": endpoint.get("results", {}).get("asr"),
                    "calls_offered": endpoint.get("results", {}).get("calls_offered"),
                    "calls_succeeded": endpoint.get("results", {}).get("calls_succeeded"),
                    "latency_ns": latency_percentiles(endpoint, endpoint_relative),
                }
            ],
            "evidence": verify_artifact(report_root, artifacts, endpoint_relative),
        },
    )
    tolerance = effective.get("values_by_key", {}).get(
        "beta_perf_latency_tolerance_pct"
    )
    if not isinstance(tolerance, (int, float)) or isinstance(tolerance, bool) or tolerance < 0:
        raise ReportError("effective configuration has invalid latency regression tolerance")
    regression_latency: list[dict[str, Any]] = []
    comparison_paths = attestation.get("performance_regression_baseline", {}).get(
        "comparison_paths", []
    )
    if not isinstance(comparison_paths, list) or not comparison_paths:
        raise ReportError("performance regression baseline has no comparison paths")
    for comparison_path in comparison_paths:
        if not isinstance(comparison_path, str) or not safe_relative(comparison_path):
            raise ReportError("performance regression comparison path is unsafe")
        baseline_relative = f"perf-regression-baseline/perf-results/{comparison_path}"
        current_relative = f"perf-results/{comparison_path}"
        baseline = read_json(report_root / baseline_relative)
        current = read_json(report_root / current_relative)
        baseline_latency = latency_percentiles(baseline, baseline_relative)
        current_latency = latency_percentiles(current, current_relative)
        for family in LATENCY_FAMILIES:
            for percentile in LATENCY_PERCENTILES:
                baseline_ms = float(baseline_latency[family][percentile]) / 1_000_000
                current_ms = float(current_latency[family][percentile]) / 1_000_000
                limit_ms = baseline_ms * (1 + float(tolerance) / 100)
                regression_latency.append(
                    {
                        "scenario": comparison_path.removesuffix(".json"),
                        "family": family,
                        "percentile": percentile,
                        "baseline_ms": baseline_ms,
                        "tolerance_percent": float(tolerance),
                        "limit_ms": limit_ms,
                        "observed_ms": current_ms,
                        "passed": current_ms <= limit_ms,
                    }
                )
        verify_artifact(report_root, artifacts, baseline_relative)
        verify_artifact(report_root, artifacts, current_relative)
    if not all(check["passed"] for check in regression_latency):
        raise ReportError("packaged latency regression evidence exceeds its limit")
    split: list[dict[str, Any]] = []
    for role in ("caller", "receiver"):
        relative = f"perf-results/perf_soak_{role}.json"
        report = read_json(report_root / relative)
        results = report.get("results", {})
        split.append(
            {
                "role": role,
                "duration_secs": results.get(
                    "duration_secs", results.get("configured_duration_secs")
                ),
                "calls_offered": results.get("calls_offered"),
                "completed": results.get(
                    "calls_succeeded", results.get("bob_completed_audio_receivers")
                ),
                "delivered_audio_frames": results.get("bob_received_frames"),
                "rss_gate_growth_mb_per_hr": results.get("rss_gate_growth_mb_per_hr"),
                "retained_after_drain": results.get("retained_objects_after_drain"),
                "skip_audio_frame_delivery": report.get("diagnostics", {})
                .get("media_receive", {})
                .get("skip_audio_frame_delivery"),
                "evidence": verify_artifact(report_root, artifacts, relative),
            }
        )
    return {
        "json_artifact_count": len(files),
        "json_artifacts": files,
        "gate_metrics": metrics,
        "gate_metrics_evidence": metrics_evidence,
        "canonical_latency_limits_ms": latency_limits_ms,
        "canonical_2k": canonical,
        "profile_matrix": matrices,
        "split_soak": split,
        "regression": {
            "status": "PASS",
            "latency_tolerance_percent": float(tolerance),
            "latency_checks": regression_latency,
            "baseline": attestation.get("performance_regression_baseline", {}),
            "audit_evidence": verify_artifact(report_root, artifacts, "perf-audit.md"),
        },
    }


def count_tsv_results(path: Path) -> dict[str, int]:
    counts: Counter[str] = Counter()
    if not path.is_file():
        return {}
    lines = [line for line in path.read_text().splitlines() if line.strip()]
    if len(lines) < 2:
        return {}
    header = lines[0].split("\t")
    status_index = next(
        (index for index, name in enumerate(header) if name.lower() in {"status", "result"}),
        None,
    )
    if status_index is None:
        return {"rows": len(lines) - 1}
    for line in lines[1:]:
        cells = line.split("\t")
        if len(cells) > status_index:
            counts[cells[status_index]] += 1
    counts["rows"] = len(lines) - 1
    return dict(sorted(counts.items()))


def build_evidence_model(
    report_root: Path,
    policy_path: Path,
    policy: dict[str, Any],
    attestation: dict[str, Any],
    effective: dict[str, Any],
    gates: dict[str, Any],
    binding_mode: str,
) -> dict[str, Any]:
    artifacts = attestation.get("artifacts", {})
    artifact_files = artifacts.get("files", [])
    indexed_artifacts = artifact_index(attestation)
    kinds = Counter(item.get("kind", "artifact") for item in artifact_files)
    performance = performance_inventory(report_root, attestation, policy, effective)
    correction = report_root / "ATTESTATION_CORRECTION.md"
    if binding_mode == "post-run-v1-backfill" and not correction.is_file():
        raise ReportError("current v1 backfill requires ATTESTATION_CORRECTION.md")
    source_hash = sha256_path(report_root / "attestation.json")
    correction_hash = sha256_path(correction) if correction.is_file() else None
    generator_hash = sha256_path(Path(__file__).resolve())
    catalog_hash = sha256_path(policy_path)
    categories = Counter(record["category"] for record in gates["records"])
    return sanitize_text(
        {
            "schema": SCHEMA_EVIDENCE,
            "binding": {
                "binding_mode": binding_mode,
                "source_attestation_schema": attestation["schema"],
                "source_attestation_sha256": source_hash,
                "correction_record_sha256": correction_hash,
                "catalog_sha256": catalog_hash,
                "generator_sha256": generator_hash,
                "tested_commit": attestation["source"]["start"]["git_commit"],
                "tested_tree": attestation["source"]["start"]["git_tree"],
                "source_fingerprint_sha256": attestation["source"]["start"][
                    "source_fingerprint_sha256"
                ],
            },
            "candidate": {
                "run_id": attestation["run"]["id"],
                "package": attestation["package"],
                "started_at_utc": attestation["run"]["started_at_utc"],
                "ended_at_utc": attestation["run"]["ended_at_utc"],
                "duration_seconds": attestation["run"]["duration_seconds"],
                "mode": attestation["run"]["mode"],
                "qualification": attestation["qualification"],
                "result": attestation["result"],
                "source_clean": attestation["source"]["clean"],
                "source_unchanged": attestation["source"]["unchanged"],
            },
            "configuration": effective,
            "gate_results": gates,
            "category_totals": dict(sorted(categories.items())),
            "artifacts": {
                "attested_count": artifacts.get("count"),
                "attested_bytes": sum(item.get("bytes", 0) for item in artifact_files),
                "counts_by_kind": dict(sorted(kinds.items())),
                "json_evidence_count": len(attestation.get("results", {}).get("json", [])),
                "performance_json_count": performance["json_artifact_count"],
            },
            "peers": attestation.get("peers", []),
            "interop": {
                "pbx_matrix": {
                    "results": count_tsv_results(report_root / "pbx/matrix.tsv"),
                    "evidence": [
                        verify_artifact(report_root, indexed_artifacts, "pbx/matrix.tsv"),
                        verify_artifact(report_root, indexed_artifacts, "pbx/summary.md"),
                    ],
                },
                "sipp_matrix": {
                    "results": count_tsv_results(report_root / "sipp/runs.tsv"),
                    "evidence": [
                        verify_artifact(report_root, indexed_artifacts, "sipp/runs.tsv"),
                        verify_artifact(report_root, indexed_artifacts, "sipp/run_summary.md"),
                    ],
                },
                "strict_ua_matrix": {
                    "results": count_tsv_results(report_root / "strict-ua/matrix.tsv"),
                    "evidence": [
                        verify_artifact(report_root, indexed_artifacts, "strict-ua/matrix.tsv"),
                        verify_artifact(report_root, indexed_artifacts, "strict-ua/summary.md"),
                    ],
                },
            },
            "performance": performance,
            "limitations": [
                (
                    "This is a deterministic post-run reporting derivation; no gate was rerun."
                    if binding_mode == "post-run-v1-backfill"
                    else "Reports were derived from native structured gate/configuration records; report generation did not rerun a gate."
                ),
                "Later reporting-only commits were not exercised by the candidate run.",
                (
                    "The SIPp start and stop entries are backed by the v1 summary and a shared listener log; future runs capture those lifecycle results directly."
                    if binding_mode == "post-run-v1-backfill"
                    else "SIPp lifecycle entries use direct, separately hashed structured gate results."
                ),
                "SHA-256 supplies integrity and reproducibility evidence, not third-party authenticity or signing.",
                "A PASS applies only to the recorded hardware, configuration, peer versions, test scopes, thresholds, and workloads.",
            ],
        }
    )


def md_value(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "not set"
    if isinstance(value, (list, dict)):
        return json.dumps(value, sort_keys=True, separators=(",", ":"))
    return str(value)


def md_escape(value: Any) -> str:
    return md_value(value).replace("|", "\\|").replace("\n", " ")


def format_ms(value: int | float) -> str:
    return f"{float(value):.3f}"


def render_release_report(evidence: dict[str, Any]) -> str:
    candidate = evidence["candidate"]
    binding = evidence["binding"]
    result = candidate["result"]
    lines = [
        "# Beta Release Candidate Report",
        "",
        f"> Reporting derivation from verified package `{candidate['run_id']}`. "
        "No gate was rerun by report generation. The candidate identity remains the "
        "tested source, not a later reporting commit.",
        "",
        "## Qualification",
        "",
        "| Field | Value |",
        "|---|---|",
        f"| Status | **{candidate['qualification']['status']}** |",
        f"| Package | `{candidate['package']['name']} {candidate['package']['version']}` |",
        f"| Run | `{candidate['run_id']}` |",
        f"| Tested commit | `{binding['tested_commit']}` |",
        f"| Tested tree | `{binding['tested_tree']}` |",
        f"| Source clean and unchanged | `{candidate['source_clean'] and candidate['source_unchanged']}` |",
        f"| Gates | **{result['overall']}: {evidence['gate_results']['passed']}/"
        f"{evidence['gate_results']['required_count']} passed, "
        f"{evidence['gate_results']['failed']} failed, "
        f"{evidence['gate_results']['skipped']} skipped** |",
        f"| Run window | `{candidate['started_at_utc']}` to `{candidate['ended_at_utc']}` |",
        "",
        "The complete gate-by-gate proof is in the [Beta Gate Report](BETA_GATE_REPORT.md). "
        "Performance observations are in the "
        "[Beta Performance Report](BETA_PERFORMANCE_REPORT.md).",
        "",
        "## Category totals",
        "",
        "| Category | Required | Passed |",
        "|---|---:|---:|",
    ]
    for category, count in evidence["category_totals"].items():
        lines.append(f"| {category} | {count} | {count} |")
    lines += [
        "",
        "## Effective configuration",
        "",
        "Values are typed. `environment-override` means the value was present in the "
        "redacted run environment; `policy-default` means it was supplied by the "
        "catalog; other v1 values were recovered from the source attestation.",
        "",
        "| Key | Type | Value | Provenance |",
        "|---|---|---|---|",
    ]
    for item in evidence["configuration"]["values"]:
        lines.append(
            f"| `{item['key']}` | `{item['type']}` | `{md_escape(item['value'])}` | "
            f"`{item['source']}` |"
        )
    lines += [
        "",
        "## Environment and peer coverage",
        "",
        f"- Runtime: `{evidence['configuration']['values_by_key'].get('rustc')}`",
        f"- Cargo: `{evidence['configuration']['values_by_key'].get('cargo')}`",
        f"- Host: `{evidence['configuration']['values_by_key'].get('host')}`",
        f"- State table: `{evidence['configuration']['values_by_key'].get('beta_state_table_source')}` "
        f"with SHA-256 `{evidence['configuration']['values_by_key'].get('beta_state_table_sha256')}`",
        "",
        "| Peer | Version | Image/config evidence |",
        "|---|---|---|",
    ]
    for peer in evidence["peers"]:
        digest = peer.get("image_digest") or peer.get("config_sha256")
        lines.append(
            f"| {peer.get('product')} | `{md_escape(peer.get('version'))}` | `{digest}` |"
        )
    artifact = evidence["artifacts"]
    lines += [
        "",
        "## Evidence package",
        "",
        f"- Attested artifacts: **{artifact['attested_count']}** "
        f"({artifact['attested_bytes']} bytes).",
        f"- Attested JSON evidence records: **{artifact['json_evidence_count']}**.",
        f"- Performance JSON files accounted for in the performance inventory: "
        f"**{artifact['performance_json_count']}**.",
        f"- Artifact kinds: `{md_escape(artifact['counts_by_kind'])}`.",
        f"- Original v1 attestation SHA-256: `{binding['source_attestation_sha256']}`.",
        f"- Correction record SHA-256: `{binding['correction_record_sha256']}`.",
        f"- Policy catalog SHA-256: `{binding['catalog_sha256']}`.",
        f"- Report generator SHA-256: `{binding['generator_sha256']}`.",
        "",
        "## Interoperability result counts",
        "",
        "| Evidence | Recorded results | Bound evidence |",
        "|---|---|---|",
        f"| PBX matrix | `{md_escape(evidence['interop']['pbx_matrix']['results'])}` | "
        f"`{evidence['interop']['pbx_matrix']['evidence'][0]['sha256']}` |",
        f"| SIPp matrix | `{md_escape(evidence['interop']['sipp_matrix']['results'])}` | "
        f"`{evidence['interop']['sipp_matrix']['evidence'][0]['sha256']}` |",
        f"| strict-UA matrix | `{md_escape(evidence['interop']['strict_ua_matrix']['results'])}` | "
        f"`{evidence['interop']['strict_ua_matrix']['evidence'][0]['sha256']}` |",
        "",
        "## Limitations and non-claims",
        "",
    ]
    lines.extend(f"- {item}" for item in evidence["limitations"])
    lines += [
        "",
        "## Verification",
        "",
        "From the repository root:",
        "",
        "```sh",
        "python3 crates/sip/rvoip-sip/scripts/beta_release_report.py verify \\",
        "  --docs-root crates/sip/rvoip-sip/docs \\",
        "  --report-root /path/to/reports/20260724T231400Z",
        "```",
        "",
    ]
    return "\n".join(lines)


def render_gate_report(evidence: dict[str, Any]) -> str:
    config = evidence["configuration"]["values_by_key"]
    records = evidence["gate_results"]["records"]
    lines = [
        "# Beta Gate Report",
        "",
        f"> Evidence-complete reporting derivation for candidate `{evidence['candidate']['run_id']}`. "
        f"All {len(records)} recorded entries are required under the effective full configuration; "
        "none is classified as merely additional.",
        "",
        "## Result",
        "",
        f"**PASS — {len(records)}/{len(records)} required gates passed; 0 failed; 0 skipped.**",
        "",
    ]
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        grouped[record["category"]].append(record)
    for category, category_records in grouped.items():
        lines += [
            f"## {category}",
            "",
            f"{len(category_records)} required gates; {len(category_records)} passed.",
            "",
        ]
        for record in category_records:
            lines += [
                f"### {record['sequence']:03d} · `{record['id']}` — {record['name']}",
                "",
                f"- Result: **{record['status']}** in {record['duration_seconds']} seconds.",
                f"- Purpose: {record['purpose']}",
                f"- Recorded component/command: `{md_escape(record['sanitized_argv'])}`.",
                f"- Why required: {record['required_reason']}",
                f"- Evidence strength: `{record['evidence_strength']}`.",
                "- Relevant configuration: "
                + ", ".join(
                    f"`{key}={md_escape(config.get(key))}`"
                    for key in record["relevant_configuration"]
                )
                + ".",
                "- Expected checks: "
                + ", ".join(f"`{item}`" for item in record["expected_checks"])
                + ".",
                "- Observed checks: "
                + "; ".join(
                    f"{item['check']}=`{md_escape(item['observed'])}` "
                    f"({'PASS' if item['passed'] else 'FAIL'})"
                    for item in record["observed_checks"]
                )
                + ".",
                "- Evidence: "
                + ", ".join(
                    f"`{item['path']}` (SHA-256 `{item['sha256']}`)"
                    for item in record["evidence"]
                )
                + ".",
                f"- PASS establishes: {record['pass_meaning']}",
                "- PASS does not establish: " + " ".join(record["non_claims"]),
                "",
            ]
    lines += [
        "## Interpretation",
        "",
        "A gate PASS is bounded by its recorded command, configuration, evidence, and "
        "explicit non-claims. The gate report does not turn component evidence into a "
        "broader protocol, security, portability, or capacity claim.",
        "",
    ]
    return "\n".join(lines)


def render_checks(checks: list[dict[str, Any]]) -> list[str]:
    lines = ["| Metric | Requirement | Observed | Result |", "|---|---|---|---|"]
    for check in checks:
        lines.append(
            f"| `{check['metric']}` | {md_escape(check['requirement'])} | "
            f"`{md_escape(check['observed'])}` | "
            f"{'PASS' if check['passed'] else 'FAIL'} |"
        )
    return lines


def render_performance_report(evidence: dict[str, Any]) -> str:
    performance = evidence["performance"]
    metrics = performance["gate_metrics"]
    lines = [
        "# Beta Performance Report",
        "",
        f"> Current canonical performance evidence for candidate `{evidence['candidate']['run_id']}` "
        f"(tested commit `{evidence['binding']['tested_commit'][:8]}`). This replaces historical current values. No "
        "performance or soak workload was rerun to generate this report.",
        "",
        f"Current release train and runtime crate version: `{evidence['candidate']['package']['version']}`.",
        "",
        "## Release performance policy",
        "",
        "- Full application audio-frame delivery was enabled "
        "(`RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0`).",
        "- High-density media burst: exactly 160 CPS, ASR at least 0.995, "
        "RSS slope at most 15 MB/hour.",
        "- Canonical 2K evidence: three clean passes from the tested source and common "
        "executable, including absolute setup and full-cycle p50/p95/p99 limits.",
        f"- Performance-regression latency tolerance: no more than "
        f"{performance['regression']['latency_tolerance_percent']:g}% above the reviewed baseline.",
        "- General beta performance claim: up to 2,000 CPS with media enabled under the recorded profile.",
        "- Monolithic and split soaks: full delivery, zero post-drain retention, "
        "and applicable RSS slope at most 15 MB/hour.",
        "",
        "## Canonical 2K three-pass evidence",
        "",
        "| Run | Target CPS | Achieved CPS | ASR | Offered | Succeeded | Retained after drain | Evidence SHA-256 |",
        "|---:|---:|---:|---:|---:|---:|---:|---|",
    ]
    for run in performance["canonical_2k"]:
        lines.append(
            f"| {run['sequence']} | {run['target_cps']} | {run['achieved_cps']} | "
            f"{run['asr']} | {run['calls_offered']} | {run['calls_succeeded']} | "
            f"{run['retained_after_drain']} | `{run['evidence']['sha256']}` |"
        )
    lines += [
        "",
        "These are three distinct canonical executions, all at 2,000 target CPS with "
        "media enabled and full delivery. Their common executable and source identity "
        "are bound by `canonical-2k/index.json` in the source package.",
        "",
        "### Canonical 2K latency acceptance",
        "",
        "All latency values and limits are milliseconds; lower is better.",
        "",
        "| Run | Measurement | p50 observed | p50 limit | p95 observed | p95 limit | p99 observed | p99 limit | Result |",
        "|---:|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    for run in performance["canonical_2k"]:
        checks = {
            (check["family"], check["percentile"]): check
            for check in run["latency_checks"]
        }
        for family, label in (
            ("setup_latency", "Call setup"),
            ("full_cycle", "Full call cycle"),
        ):
            family_checks = [checks[(family, percentile)] for percentile in LATENCY_PERCENTILES]
            lines.append(
                f"| {run['sequence']} | {label} | "
                f"{format_ms(family_checks[0]['observed_ms'])} | "
                f"≤ {format_ms(family_checks[0]['limit_ms'])} | "
                f"{format_ms(family_checks[1]['observed_ms'])} | "
                f"≤ {format_ms(family_checks[1]['limit_ms'])} | "
                f"{format_ms(family_checks[2]['observed_ms'])} | "
                f"≤ {format_ms(family_checks[2]['limit_ms'])} | "
                f"{'PASS' if all(check['passed'] for check in family_checks) else 'FAIL'} |"
            )
    lines += [
        "",
        "## Complete call-setup profile matrix",
        "",
        "Latency columns are observed milliseconds. Matrix points without a separate "
        "absolute latency limit remain subject to the recorded ASR, throughput, cleanup, "
        "and applicable regression gates; they are not silently presented as latency-SLA passes.",
        "",
        "| Profile | Target CPS | Achieved CPS | ASR | Setup p50 | Setup p95 | Setup p99 | Cycle p50 | Cycle p95 | Cycle p99 | Calls |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for matrix in performance["profile_matrix"]:
        for point in matrix["points"]:
            setup = point["latency_ns"]["setup_latency"]
            cycle = point["latency_ns"]["full_cycle"]
            lines.append(
                f"| `{matrix['profile']}` | {point['target_cps']} | "
                f"{point['achieved_cps']} | {point['asr']} | "
                f"{format_ms(setup['p50'] / 1_000_000)} | "
                f"{format_ms(setup['p95'] / 1_000_000)} | "
                f"{format_ms(setup['p99'] / 1_000_000)} | "
                f"{format_ms(cycle['p50'] / 1_000_000)} | "
                f"{format_ms(cycle['p95'] / 1_000_000)} | "
                f"{format_ms(cycle['p99'] / 1_000_000)} | "
                f"{point['calls_succeeded']}/{point['calls_offered']} |"
            )
    high = metrics["high_density_media_burst"]
    mono = metrics["monolithic_soak"]
    lines += [
        "",
        "## High-density full-delivery media burst",
        "",
        f"**{'PASS' if high['passed'] else 'FAIL'}** — "
        f"{high['observed']['calls_succeeded']}/{high['observed']['calls_offered']} calls, "
        f"ASR {high['observed']['asr']}, "
        f"{high['observed']['delivered_audio_frames']} application audio frames delivered, "
        f"peak {high['observed']['peak_active_calls']} active calls.",
        "",
    ]
    lines.extend(render_checks(high["checks"]))
    lines += [
        "",
        "## Monolithic soak",
        "",
        f"**{'PASS' if mono['passed'] else 'FAIL'}** — "
        f"{mono['observed']['calls_succeeded']}/{mono['observed']['calls_offered']} calls, "
        f"{mono['observed']['delivered_audio_frames']} application audio frames delivered, "
        f"RSS gate slope {mono['observed']['rss_gate_growth_mb_per_hr']} MB/hour.",
        "",
    ]
    lines.extend(render_checks(mono["checks"]))
    lines += [
        "",
        "## Split soak",
        "",
        "| Role | Configured duration seconds | Offered | Completed | Delivered frames | RSS gate MB/hour | Retained after drain | Full delivery | Evidence SHA-256 |",
        "|---|---:|---:|---:|---:|---:|---:|---|---|",
    ]
    for item in performance["split_soak"]:
        full_delivery = item["skip_audio_frame_delivery"] is False
        lines.append(
            f"| {item['role']} | {item['duration_secs']} | {md_value(item['calls_offered'])} | "
            f"{item['completed']} | {md_value(item['delivered_audio_frames'])} | "
            f"{item['rss_gate_growth_mb_per_hr']} | "
            f"{item['retained_after_drain']} | {'yes' if full_delivery else 'no'} | "
            f"`{item['evidence']['sha256']}` |"
        )
    lines += [
        "",
        "## Regression evidence",
        "",
        f"- Result: **{performance['regression']['status']}**.",
        f"- Latency limit: reviewed baseline plus at most "
        f"{performance['regression']['latency_tolerance_percent']:g}%.",
        "",
        "| Scenario | Measurement | Percentile | Baseline ms | Limit ms | Observed ms | Result |",
        "|---|---|---|---:|---:|---:|---|",
    ]
    for check in performance["regression"]["latency_checks"]:
        label = "Call setup" if check["family"] == "setup_latency" else "Full call cycle"
        lines.append(
            f"| `{check['scenario']}` | {label} | {check['percentile']} | "
            f"{format_ms(check['baseline_ms'])} | "
            f"≤ {format_ms(check['limit_ms'])} | "
            f"{format_ms(check['observed_ms'])} | "
            f"{'PASS' if check['passed'] else 'FAIL'} |"
        )
    lines += [
        "",
        f"- Reviewed baseline: `{md_escape(performance['regression']['baseline'])}`.",
        f"- Audit evidence: `perf-audit.md` (SHA-256 "
        f"`{performance['regression']['audit_evidence']['sha256']}`).",
        "",
        "## Performance JSON artifact inventory",
        "",
        f"All **{performance['json_artifact_count']}** JSON files under the packaged "
        "`perf-results/` tree are listed. Primary results and supporting build/source "
        "provenance are distinguished by role; none is silently omitted.",
        "",
        "| # | Role | Evidence | SHA-256 | Bytes |",
        "|---:|---|---|---|---:|",
    ]
    for index, item in enumerate(performance["json_artifacts"], 1):
        lines.append(
            f"| {index} | `{item['role']}` | `{item['path']}` | "
            f"`{item['sha256']}` | {item['bytes']} |"
        )
    lines += [
        "",
        "## Interpretation",
        "",
        "PASS establishes the recorded thresholds only for this source, executable, "
        "host, loopback topology, configurations, durations, and workloads. It does "
        "not claim untested Internet conditions, hardware, concurrency, codecs, peers, "
        "or sustained durations.",
        "",
    ]
    return "\n".join(lines)


def source_verifier(report_root: Path) -> None:
    verifier = report_root / "inputs/attestation-verifier.py"
    if not verifier.is_file():
        raise ReportError(f"packaged source verifier is missing: {verifier}")
    command = [
        sys.executable,
        str(verifier),
        "verify",
        "--report-root",
        str(report_root),
        "--require-clean",
        "--require-unchanged-source",
        "--require-no-skips",
        "--require-pass",
        "--require-mode-eligible",
    ]
    completed = subprocess.run(command, text=True, capture_output=True, check=False)
    if completed.returncode != 0:
        detail = (completed.stdout + completed.stderr).strip()
        raise ReportError(f"strict source attestation verification failed:\n{detail}")


def validate_source_attestation(
    report_root: Path, policy: dict[str, Any], *, current_only: bool = True
) -> dict[str, Any]:
    attestation = read_json(report_root / "attestation.json")
    if attestation.get("schema") not in EXPECTED_SOURCE_SCHEMAS:
        raise ReportError("unsupported source attestation schema")
    if current_only and attestation.get("schema") != "rvoip-sip-beta-attestation-v1":
        raise ReportError("current backfill requires a v1 source attestation")
    expected = policy["current_candidate"]
    if current_only:
        if attestation.get("run", {}).get("id") != expected["run_id"]:
            raise ReportError("source run ID does not match the current policy candidate")
        if attestation.get("source", {}).get("start", {}).get("git_commit") != expected[
            "tested_commit"
        ]:
            raise ReportError("tested commit does not match the current policy candidate")
    result = attestation.get("result", {})
    actual = (
        len(attestation.get("gates", [])),
        result.get("failed_gates"),
        result.get("skipped_gates"),
    )
    if current_only:
        required = (
            expected["expected_passed"],
            expected["expected_failed"],
            expected["expected_skipped"],
        )
        if actual != required:
            raise ReportError(f"source gate totals {actual}, expected {required}")
    elif (
        attestation.get("run", {}).get("mode") != "full"
        or result.get("overall") != "PASS"
        or result.get("failed_gates") != 0
        or result.get("skipped_gates") != 0
        or not attestation.get("source", {}).get("clean")
        or not attestation.get("source", {}).get("unchanged")
    ):
        raise ReportError("native report generation requires a clean, unchanged, zero-skip full PASS")
    if not attestation.get("qualification", {}).get("release_candidate"):
        raise ReportError("source attestation is not release-candidate qualified")
    return attestation


def generate(report_root: Path, policy_path: Path, output_dir: Path) -> None:
    source_verifier(report_root)
    policy = load_policy(policy_path)
    native = (report_root / "effective-gate-config.json").is_file() and (
        report_root / "gate-results.json"
    ).is_file()
    attestation = validate_source_attestation(report_root, policy, current_only=not native)
    if native:
        effective = read_json(report_root / "effective-gate-config.json")
        if (
            effective.get("schema") != SCHEMA_CONFIG
            or effective.get("binding_mode") != "native-v2-input"
            or not effective.get("values_by_key")
        ):
            raise ReportError("invalid native effective-gate-config.json")
        gates = load_native_gate_results(report_root, attestation, policy, effective)
        binding_mode = "native-v2-input"
    else:
        effective = effective_configuration(report_root, attestation, policy)
        gates = build_gate_results(report_root, attestation, policy, effective)
        binding_mode = "post-run-v1-backfill"
    evidence = build_evidence_model(
        report_root, policy_path, policy, attestation, effective, gates, binding_mode
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    payloads = {
        "effective-gate-config.json": canonical_json(effective),
        "gate-results.json": canonical_json(gates),
        "release-evidence.json": canonical_json(evidence),
        "BETA_RELEASE_REPORT.md": render_release_report(evidence).encode(),
        "BETA_GATE_REPORT.md": render_gate_report(evidence).encode(),
        "BETA_PERFORMANCE_REPORT.md": render_performance_report(evidence).encode(),
        "inputs/beta-release-policy.yaml": policy_path.read_bytes(),
        "inputs/beta_release_report.py": Path(__file__).resolve().read_bytes(),
    }
    for name, data in payloads.items():
        write_atomic(output_dir / name, data)
    bindings = {
        name: {"sha256": sha256_path(output_dir / name), "bytes": (output_dir / name).stat().st_size}
        for name in sorted(payloads)
    }
    report_attestation = {
        "schema": SCHEMA_REPORT_ATTESTATION,
        "binding_mode": binding_mode,
        "run_id": attestation["run"]["id"],
        "report_revision": policy["current_candidate"].get("report_revision", 1),
        "tested_commit": attestation["source"]["start"]["git_commit"],
        "source_attestation_sha256": sha256_path(report_root / "attestation.json"),
        "correction_record_sha256": (
            sha256_path(report_root / "ATTESTATION_CORRECTION.md")
            if (report_root / "ATTESTATION_CORRECTION.md").is_file()
            else None
        ),
        "policy_catalog_sha256": sha256_path(policy_path),
        "generator_sha256": sha256_path(Path(__file__).resolve()),
        "generated_files": bindings,
        "assurance": {
            "kind": "integrity-and-reproducibility",
            "cryptographically_signed": False,
            "note": "SHA-256 integrity evidence; not third-party signing.",
        },
    }
    write_atomic(output_dir / "report-attestation.json", canonical_json(report_attestation))
    checksum = sha256_path(output_dir / "report-attestation.json")
    write_atomic(
        output_dir / "report-attestation.json.sha256",
        f"{checksum}  report-attestation.json\n".encode(),
    )


def assert_no_sensitive_or_absolute_paths(path: Path) -> None:
    text = path.read_text(errors="replace")
    if ABSOLUTE_USER_PATH_RE.search(text):
        raise ReportError(f"user-specific absolute path leaked into {path}")
    if SECRET_RE.search(text):
        raise ReportError(f"secret-like material found in {path}")


def verify_markdown_links(directory: Path, path: Path) -> None:
    for target in MARKDOWN_LINK_RE.findall(path.read_text()):
        target = target.split("#", 1)[0]
        if not target or re.match(r"^[a-z]+://", target):
            continue
        if not safe_relative(target):
            raise ReportError(f"unsafe Markdown link {target!r} in {path.name}")
        if not (directory / target).is_file():
            raise ReportError(f"broken Markdown link {target!r} in {path.name}")


def verify_generated(directory: Path, expected_policy: Path | None = None) -> dict[str, Any]:
    for name in ALL_GENERATED_FILES:
        if not (directory / name).is_file():
            raise ReportError(f"missing generated report artifact {directory / name}")
    attestation = read_json(directory / "report-attestation.json")
    if attestation.get("schema") != SCHEMA_REPORT_ATTESTATION:
        raise ReportError("unsupported report attestation schema")
    checksum_line = (directory / "report-attestation.json.sha256").read_text().strip()
    expected_checksum = f"{sha256_path(directory / 'report-attestation.json')}  report-attestation.json"
    if checksum_line != expected_checksum:
        raise ReportError("report attestation checksum mismatch")
    for name, binding in attestation.get("generated_files", {}).items():
        if not safe_relative(name) or not (directory / name).is_file():
            raise ReportError(f"unsafe or missing generated binding {name!r}")
        if sha256_path(directory / name) != binding.get("sha256"):
            raise ReportError(f"generated file hash mismatch: {name}")
        if (directory / name).stat().st_size != binding.get("bytes"):
            raise ReportError(f"generated file size mismatch: {name}")
    if expected_policy and sha256_path(expected_policy) != attestation.get(
        "policy_catalog_sha256"
    ):
        raise ReportError("policy catalog hash mismatch")
    if sha256_path(directory / "inputs/beta_release_report.py") != attestation.get(
        "generator_sha256"
    ):
        raise ReportError("packaged report generator hash mismatch")
    if sha256_path(directory / "inputs/beta-release-policy.yaml") != attestation.get(
        "policy_catalog_sha256"
    ):
        raise ReportError("packaged policy catalog hash mismatch")
    evidence = read_json(directory / "release-evidence.json")
    config = read_json(directory / "effective-gate-config.json")
    gates = read_json(directory / "gate-results.json")
    if evidence.get("schema") != SCHEMA_EVIDENCE:
        raise ReportError("invalid release evidence schema")
    if config.get("schema") != SCHEMA_CONFIG or gates.get("schema") != SCHEMA_RESULTS:
        raise ReportError("invalid structured configuration or gate-results schema")
    records = gates.get("records", [])
    required_count = gates.get("required_count")
    if (
        not isinstance(required_count, int)
        or required_count <= 0
        or gates.get("passed") != required_count
        or gates.get("failed") != 0
        or gates.get("skipped") != 0
    ):
        raise ReportError("generated gate totals are not an all-required, zero-skip PASS")
    if len(records) != required_count or len({item.get("id") for item in records}) != required_count:
        raise ReportError("generated gates are missing or have duplicate IDs")
    if attestation.get("run_id") == "20260724T231400Z" and required_count != 108:
        raise ReportError("current candidate reporting must contain exactly 108 gates")
    for record in records:
        for key in (
            "id",
            "name",
            "category",
            "purpose",
            "sanitized_argv",
            "expected_checks",
            "observed_checks",
            "relevant_configuration",
            "evidence",
            "pass_meaning",
            "non_claims",
        ):
            if not record.get(key):
                raise ReportError(f"gate {record.get('id')} is missing {key}")
        if not record.get("required") or record.get("status") != "PASS":
            raise ReportError(f"gate {record.get('id')} is not required PASS")
        if not all(check.get("passed") for check in record["observed_checks"]):
            raise ReportError(f"gate {record.get('id')} has a failed observed check")
    if len(config.get("values", [])) < 50:
        raise ReportError("effective configuration is incomplete")
    if evidence.get("performance", {}).get("json_artifact_count") != 59:
        raise ReportError("performance inventory does not account for 59 JSON artifacts")
    for name in REPORT_FILES + MACHINE_FILES + ("report-attestation.json",):
        assert_no_sensitive_or_absolute_paths(directory / name)
    for name in REPORT_FILES:
        verify_markdown_links(directory, directory / name)
    return attestation


def current_snapshot_relative(policy: dict[str, Any]) -> Path:
    candidate = policy["current_candidate"]
    run_id = candidate["run_id"]
    revision = candidate.get("report_revision", 1)
    if not isinstance(revision, int) or isinstance(revision, bool) or revision < 1:
        raise ReportError("current candidate report_revision must be a positive integer")
    return Path(run_id) if revision == 1 else Path(run_id) / f"reporting-r{revision}"


def render_release_index(
    run_ids: list[str], current: str, current_relative: Path
) -> str:
    lines = [
        "# Beta release evidence reports",
        "",
        "Immutable, post-run-derived release documentation. The marker identifies the "
        "current candidate; each directory is content-bound by its report attestation.",
        "",
        "| Run | Current | Release report | Gate report | Performance report |",
        "|---|---|---|---|---|",
    ]
    for run_id in sorted(run_ids, reverse=True):
        marker = "**yes**" if run_id == current else "no"
        report_root = current_relative.as_posix() if run_id == current else run_id
        lines.append(
            f"| `{run_id}` | {marker} | "
            f"[release]({report_root}/BETA_RELEASE_REPORT.md) | "
            f"[gates]({report_root}/BETA_GATE_REPORT.md) | "
            f"[performance]({report_root}/BETA_PERFORMANCE_REPORT.md) |"
        )
    lines += [
        "",
        f"Current candidate: `{current}`.",
        f"Current reporting derivation: `{current_relative.as_posix()}`.",
        "",
    ]
    return "\n".join(lines)


def promote_docs(
    report_root: Path, policy_path: Path, docs_root: Path
) -> None:
    source_verifier(report_root)
    policy = load_policy(policy_path)
    run_id = policy["current_candidate"]["run_id"]
    releases_root = docs_root / "releases/beta"
    snapshot_relative = current_snapshot_relative(policy)
    snapshot = releases_root / snapshot_relative
    with tempfile.TemporaryDirectory(prefix=".beta-report-", dir=docs_root.parent) as tmp:
        generated = Path(tmp)
        generate(report_root, policy_path, generated)
        verify_generated(generated, policy_path)
        if snapshot.exists():
            for name in ALL_GENERATED_FILES:
                existing = snapshot / name
                if not existing.is_file() or existing.read_bytes() != (generated / name).read_bytes():
                    raise ReportError(
                        f"immutable snapshot {snapshot} exists with different content"
                    )
        else:
            releases_root.mkdir(parents=True, exist_ok=True)
            staged_snapshot = releases_root / f".{run_id}.tmp"
            if staged_snapshot.exists():
                raise ReportError(f"stale promotion directory exists: {staged_snapshot}")
            staged_snapshot.mkdir()
            for name in ALL_GENERATED_FILES:
                destination = staged_snapshot / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(generated / name, destination)
            os.replace(staged_snapshot, snapshot)
        for name in REPORT_FILES:
            write_atomic(docs_root / name, (generated / name).read_bytes())
        run_ids = [
            child.name
            for child in releases_root.iterdir()
            if child.is_dir() and not child.name.startswith(".")
        ]
        write_atomic(
            releases_root / "README.md",
            render_release_index(run_ids, run_id, snapshot_relative).encode(),
        )
    verify_promoted(docs_root, policy_path, report_root)


def verify_promoted(
    docs_root: Path, policy_path: Path, report_root: Path | None
) -> None:
    policy = load_policy(policy_path)
    run_id = policy["current_candidate"]["run_id"]
    snapshot = docs_root / "releases/beta" / current_snapshot_relative(policy)
    report_attestation = verify_generated(snapshot, policy_path)
    for name in REPORT_FILES:
        if (docs_root / name).read_bytes() != (snapshot / name).read_bytes():
            raise ReportError(f"current {name} does not match immutable snapshot")
    index = docs_root / "releases/beta/README.md"
    if not index.is_file() or f"Current candidate: `{run_id}`." not in index.read_text():
        raise ReportError("release index/current-candidate marker is missing")
    verify_markdown_links(index.parent, index)
    assert_no_sensitive_or_absolute_paths(index)
    if report_root:
        source_verifier(report_root)
        source = validate_source_attestation(report_root, policy)
        if sha256_path(report_root / "attestation.json") != report_attestation.get(
            "source_attestation_sha256"
        ):
            raise ReportError("promoted report is not bound to the supplied source attestation")
        if source["source"]["start"]["git_commit"] != report_attestation.get(
            "tested_commit"
        ):
            raise ReportError("promoted report tested-commit binding mismatch")


def capture_config(
    policy_path: Path,
    output: Path,
    mode: str,
    environment_dir: Path | None = None,
    derived_items: list[str] | None = None,
) -> None:
    """Capture typed configuration natively for a future beta run."""
    policy = load_policy(policy_path)
    definitions = policy["configuration"]
    values = []
    by_key: dict[str, Any] = {}
    for key, definition in sorted(definitions.items()):
        env_key = key.upper()
        if env_key in os.environ:
            raw = os.environ[env_key]
            source = "environment-override"
        elif mode == "full" and "full_default" in definition:
            raw = definition["full_default"]
            source = "policy-default"
        else:
            raw = definition.get("default")
            source = "policy-default"
        typed = convert_typed(raw, definition)
        by_key[key] = typed
        values.append(
            {"key": key, "type": definition["type"], "value": typed, "source": source}
        )
    by_key["beta_gate_mode"] = mode
    for item in values:
        if item["key"] == "beta_gate_mode":
            item["value"] = mode
            item["source"] = "derived-setting"
    derived: dict[str, Any] = {}
    if environment_dir:
        file_keys = {
            "git_revision": "git-rev.txt",
            "rustc": "rustc-version.txt",
            "cargo": "cargo-version.txt",
            "host": "host-uname.txt",
            "colima": "colima-status.txt",
            "docker": "docker-version.txt",
        }
        for key, filename in file_keys.items():
            path = environment_dir / filename
            if path.is_file():
                derived[key] = next(
                    (line.strip() for line in path.read_text().splitlines() if line.strip()),
                    None,
                )
        status_path = environment_dir / "git-status.txt"
        if status_path.is_file():
            derived["git_status"] = "dirty" if status_path.read_text().strip() else "clean"
        derived["cargo_metadata"] = "environment/cargo-metadata.json"
        derived["source_at_beta_start"] = "environment/source-at-beta-start.json"
        derived["source_at_beta_end"] = "environment/source-at-beta-end.json"
    for item in derived_items or []:
        if "=" not in item:
            raise ReportError(f"derived configuration must be key=value: {item!r}")
        key, raw = item.split("=", 1)
        if key not in definitions:
            raise ReportError(f"unknown derived configuration key {key!r}")
        derived[key] = raw
    value_entries = {item["key"]: item for item in values}
    for key, raw in derived.items():
        if raw is None:
            continue
        definition = definitions.get(key)
        typed = convert_typed(raw, definition)
        by_key[key] = typed
        value_entries[key] = {
            "key": key,
            "type": definition["type"] if definition else type(typed).__name__,
            "value": typed,
            "source": "derived-setting",
        }
    values = [value_entries[key] for key in sorted(value_entries)]
    payload = {
        "schema": SCHEMA_CONFIG,
        "policy_version": policy["policy_version"],
        "mode": mode,
        "binding_mode": "native-v2-input",
        "captured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "values": sanitize_text(values),
        "values_by_key": sanitize_text(by_key),
    }
    write_atomic(output, canonical_json(payload))


def record_gate(args: argparse.Namespace) -> None:
    """Write one native per-gate result atomically for future aggregation."""
    policy = load_policy(args.policy)
    catalog = {gate["name"]: gate for gate in expand_catalog(policy)}
    gate = catalog.get(args.name)
    if gate is None:
        raise ReportError(f"cannot record uncatalogued gate {args.name!r}")
    if args.status not in {"PASS", "FAIL", "SKIP"}:
        raise ReportError(f"invalid gate status {args.status}")
    if not safe_relative(args.log):
        raise ReportError(f"unsafe gate evidence path {args.log!r}")
    raw_argv = args.argv[1:] if args.argv and args.argv[0] == "--" else args.argv
    argv = sanitize_text(raw_argv)
    if not argv:
        raise ReportError("native gate record requires argv")
    payload = {
        "schema": "rvoip-sip-gate-result-v1",
        "sequence": args.sequence,
        "id": gate["id"],
        "name": gate["name"],
        "category": gate["category"],
        "kind": gate["kind"],
        "status": args.status,
        "started_at_utc": args.started,
        "ended_at_utc": args.ended,
        "duration_seconds": args.duration,
        "sanitized_argv": argv,
        "evidence": [{"path": args.log, "sha256": args.log_sha256}],
        "checks": [
            {
                "check": "command exit status",
                "observed": args.exit_status,
                "passed": args.exit_status == 0,
            }
        ],
    }
    args.results_dir.mkdir(parents=True, exist_ok=True)
    write_atomic(
        args.results_dir / f"{args.sequence:03d}-{gate['id']}.json",
        canonical_json(payload),
    )


def update_config(policy_path: Path, path: Path, derived_items: list[str]) -> None:
    policy = load_policy(policy_path)
    payload = read_json(path)
    if payload.get("schema") != SCHEMA_CONFIG:
        raise ReportError(f"cannot update invalid effective configuration {path}")
    entries = {item["key"]: item for item in payload.get("values", [])}
    values_by_key = payload.get("values_by_key", {})
    for item in derived_items:
        if "=" not in item:
            raise ReportError(f"derived configuration must be key=value: {item!r}")
        key, raw = item.split("=", 1)
        definition = policy["configuration"].get(key)
        if definition is None:
            raise ReportError(f"unknown derived configuration key {key!r}")
        typed = convert_typed(raw, definition)
        entries[key] = {
            "key": key,
            "type": definition["type"],
            "value": typed,
            "source": "derived-setting",
        }
        values_by_key[key] = typed
    payload["values"] = [entries[key] for key in sorted(entries)]
    payload["values_by_key"] = dict(sorted(values_by_key.items()))
    write_atomic(path, canonical_json(payload))


def finalize_gates(results_dir: Path, output: Path, mode: str) -> None:
    records = [read_json(path) for path in sorted(results_dir.glob("*.json"))]
    sequences = [item.get("sequence") for item in records]
    ids = [item.get("id") for item in records]
    if len(sequences) != len(set(sequences)) or len(ids) != len(set(ids)):
        raise ReportError("duplicate sequence or gate ID in native result fragments")
    records.sort(key=lambda item: item["sequence"])
    payload = {
        "schema": SCHEMA_RESULTS,
        "binding_mode": "native-v2-input",
        "mode": mode,
        "required_count": len(records),
        "passed": sum(item["status"] == "PASS" for item in records),
        "failed": sum(item["status"] == "FAIL" for item in records),
        "skipped": sum(item["status"] == "SKIP" for item in records),
        "records": records,
    }
    write_atomic(output, canonical_json(payload))


def default_paths() -> tuple[Path, Path]:
    crate = Path(__file__).resolve().parent.parent
    return crate / "config/beta-release-policy.yaml", crate / "docs"


def parser() -> argparse.ArgumentParser:
    default_policy, default_docs = default_paths()
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    for name in ("generate", "promote-docs"):
        command = commands.add_parser(name)
        command.add_argument("--report-root", required=True, type=Path)
        command.add_argument("--policy", type=Path, default=default_policy)
        if name == "generate":
            command.add_argument("--output-dir", required=True, type=Path)
        else:
            command.add_argument("--docs-root", type=Path, default=default_docs)
    verify = commands.add_parser("verify")
    target = verify.add_mutually_exclusive_group(required=True)
    target.add_argument("--generated-dir", type=Path)
    target.add_argument("--docs-root", type=Path)
    verify.add_argument("--policy", type=Path, default=default_policy)
    verify.add_argument("--report-root", type=Path)
    validate = commands.add_parser("validate-policy")
    validate.add_argument("--policy", type=Path, default=default_policy)
    validate.add_argument("--report-root", type=Path)
    capture = commands.add_parser("capture-config")
    capture.add_argument("--policy", type=Path, default=default_policy)
    capture.add_argument("--output", required=True, type=Path)
    capture.add_argument("--mode", required=True, choices=["local", "full", "interop", "perf", "security"])
    capture.add_argument("--environment-dir", type=Path)
    capture.add_argument("--derived", action="append", default=[])
    update = commands.add_parser("update-config")
    update.add_argument("--policy", type=Path, default=default_policy)
    update.add_argument("--config", required=True, type=Path)
    update.add_argument("--derived", action="append", required=True)
    record = commands.add_parser("record-gate")
    record.add_argument("--policy", type=Path, default=default_policy)
    record.add_argument("--results-dir", required=True, type=Path)
    record.add_argument("--sequence", required=True, type=int)
    record.add_argument("--name", required=True)
    record.add_argument("--status", required=True)
    record.add_argument("--started", required=True)
    record.add_argument("--ended", required=True)
    record.add_argument("--duration", required=True, type=int)
    record.add_argument("--exit-status", required=True, type=int)
    record.add_argument("--log", required=True)
    record.add_argument("--log-sha256", required=True)
    record.add_argument("argv", nargs=argparse.REMAINDER)
    finalize = commands.add_parser("finalize-gates")
    finalize.add_argument("--results-dir", required=True, type=Path)
    finalize.add_argument("--output", required=True, type=Path)
    finalize.add_argument("--mode", required=True)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        if args.command == "generate":
            generate(args.report_root.resolve(), args.policy.resolve(), args.output_dir.resolve())
            verify_generated(args.output_dir.resolve(), args.policy.resolve())
            print(f"generated and verified beta reports in {args.output_dir}")
        elif args.command == "promote-docs":
            promote_docs(args.report_root.resolve(), args.policy.resolve(), args.docs_root.resolve())
            print(f"promoted verified beta reports into {args.docs_root}")
        elif args.command == "verify":
            if args.generated_dir:
                verify_generated(args.generated_dir.resolve(), args.policy.resolve())
            else:
                verify_promoted(
                    args.docs_root.resolve(),
                    args.policy.resolve(),
                    args.report_root.resolve() if args.report_root else None,
                )
            print("beta release reporting verification: PASS")
        elif args.command == "validate-policy":
            policy = load_policy(args.policy.resolve())
            if args.report_root:
                attestation = validate_source_attestation(args.report_root.resolve(), policy)
                effective = effective_configuration(args.report_root.resolve(), attestation, policy)
                build_gate_results(args.report_root.resolve(), attestation, policy, effective)
            print("beta release policy validation: PASS")
        elif args.command == "capture-config":
            capture_config(
                args.policy.resolve(),
                args.output.resolve(),
                args.mode,
                args.environment_dir.resolve() if args.environment_dir else None,
                args.derived,
            )
        elif args.command == "record-gate":
            args.policy = args.policy.resolve()
            args.results_dir = args.results_dir.resolve()
            record_gate(args)
        elif args.command == "update-config":
            update_config(args.policy.resolve(), args.config.resolve(), args.derived)
        elif args.command == "finalize-gates":
            finalize_gates(args.results_dir.resolve(), args.output.resolve(), args.mode)
        return 0
    except (ReportError, OSError, ValueError) as exc:
        print(f"beta release reporting: FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
