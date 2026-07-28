#!/usr/bin/env python3
"""Create and independently verify rvoip-sip beta report attestations."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import pathlib
import re
import shutil
import sys
from typing import Any, Iterable


SCHEMA_V1 = "rvoip-sip-beta-attestation-v1"
SCHEMA_V2 = "rvoip-sip-beta-attestation-v2"
SCHEMA = SCHEMA_V1
STRUCTURED_CONFIG_SCHEMA = "rvoip-sip-effective-gate-config-v1"
STRUCTURED_RESULTS_SCHEMA = "rvoip-sip-gate-results-v1"
HASH_ALGORITHM = "sha256"
ATTESTATION_NAME = "attestation.json"
CHECKSUM_NAME = "attestation.json.sha256"
EXCLUDED_FILES = {ATTESTATION_NAME, CHECKSUM_NAME}
VALID_MODES = {"local", "full", "interop", "perf", "security"}
VALID_GATE_STATUSES = {"PASS", "FAIL", "SKIP"}
REQUIRED_INPUTS = {
    "attestation-verifier",
    "burst-scenarios",
    "performance-recipe",
    "state-machine-yaml",
}
MODE_REQUIRED_INPUTS = {
    "full": {"performance-regression-baseline"},
}
PERFORMANCE_REGRESSION_BASELINE_SCHEMA = "rvoip-perf-regression-baseline-v1"
PERFORMANCE_REGRESSION_BASELINE_PACKAGE = "perf-regression-baseline"
PERFORMANCE_REGRESSION_BASELINE_ID = "20260706T181609Z"
PERFORMANCE_REGRESSION_BASELINE_SHA256 = (
    "739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9"
)
CANONICAL_2K_SCHEMA = "rvoip-canonical-2k-evidence-v2"
CANONICAL_2K_RUN_COUNT = 3
CANONICAL_2K_SCENARIO = "perf_call_setup_cps_pbx-media-server"
CANONICAL_2K_BASELINE_ID = "20260706T181609Z"
CANONICAL_2K_BASELINE_RELATIVE_PATH = (
    "perf_call_setup_cps_pbx-media-server/2000.json"
)
CANONICAL_2K_BASELINE_SHA256 = (
    "6d55df11e169ff22dd955c466c02cb86a506830559e3f9d2aa03fc2b86f417c3"
)
CANONICAL_2K_BASELINE_PACKAGED_PATH = (
    f"reviewed-baseline/{CANONICAL_2K_BASELINE_RELATIVE_PATH}"
)
CANONICAL_2K_BASELINE_ORIGINS = {"tracked-default", "explicit-override"}
DOCKER_PEER_SCHEMA = "rvoip-docker-peer-snapshot-v1"
VALID_STATE_TABLE_SOURCES = {
    "embedded-default",
    "configured-path",
    "configured-path-fallback",
}
VALID_STATE_TABLE_FALLBACK_REASONS = {
    "read-failed",
    "decode-failed",
    "load-failed",
    "validation-failed",
}
MODE_POINTERS = {
    "local": "latest-local.txt",
    "interop": "latest-interop.txt",
    "security": "latest-security.txt",
    "perf": "latest-perf.txt",
    "full": "latest-full-clean.txt",
}
PBX_EVIDENCE_PATHS = {
    "pbx/matrix.tsv",
    "pbx/summary.md",
}
INTEROP_EVIDENCE_PATHS = {
    "sipp/analysis.md",
    "sipp/run_summary.md",
    "strict-ua/matrix.tsv",
    "strict-ua/summary.md",
}
PERFORMANCE_GATE_METRICS_PATHS = {
    "performance-gate-metrics.json",
    "performance-gate-metrics.md",
}
REQUIRED_INTEROP_PEER_PRODUCTS = {"asterisk", "baresip", "freeswitch", "sipp"}
STANDARD_PERFORMANCE_RESULT_PATHS = {
    "perf-results/perf_backpressure_step.json",
    "perf-results/perf_call_setup_cps_endpoint.json",
    "perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json",
    "perf-results/perf_call_setup_cps_signaling-only-server-high-performance/_sweep.json",
    "perf-results/perf_concurrent_active_calls.json",
    "perf-results/perf_registration_throughput.json",
    "perf-results/perf_rtp_steady_state.json",
    "perf-results/perf_session_churn_leak.json",
    "perf-results/perf_soak_caller.json",
    "perf-results/perf_soak_receiver.json",
    "perf-results/perf_transport_recovery.json",
}
LITERAL_ALL_PERFORMANCE_RESULT_PATHS = {
    "perf-results/perf_ai_agent_load.json",
    "perf-results/perf_b2bua_forwarding.json",
    "perf-results/perf_contact_center_transfers.json",
    "perf-results/perf_mass_teardown_stress.json",
    "perf-results/perf_media_churn.json",
    "perf-results/perf_mid_call_signal_under_media.json",
    "perf-results/perf_mixed_workload.json",
    "perf-results/perf_pdd_with_180_first.json",
    "perf-results/perf_registrar_binding_scale.json",
    "perf-results/perf_sipp_parity.json",
    "perf-results/perf_soak_30min.json",
    "perf-results/perf_srtp_overhead.json",
    "perf-results/perf_sustained_long_duration_calls.json",
    "perf-results/perf_tls_overhead.json",
}
LOCAL_REQUIRED_GATES = {
    "format check",
    "beta evidence helper tests",
    "public API compatibility",
    "rvoip-sip all-target check",
    "claimed lower-crate check",
    "supporting SIP crate tests",
    "rtp-core tests",
    "rvoip-sip unit tests",
    "rvoip-sip integration tests",
    "rvoip-sip doctests",
    "rvoip-sip examples compile",
    "downstream rvoip default check",
    "downstream rvoip app check",
    "downstream rvoip-client default check",
    "downstream rvoip-client full check",
    "downstream rvoip-core check",
    "downstream rvoip-amazon-connect server check",
    "downstream rvoip-uctp check",
    "downstream rvoip-quic check",
    "downstream rvoip-webtransport check",
    "downstream rvoip-websocket media and TLS check",
    "downstream rvoip-webrtc interop check",
    "downstream rvoip-audio-device check",
    "PBX analyzer unit tests",
    "rvoip-sip rustdoc",
    "sip-core RFC 4475 torture tests",
    "sip-core generated message validation",
    "sip dialog generated validation",
} | {
    f"standalone example {example} tests"
    for example in (
        "01-quickstart-p2p",
        "02-softphone-audio",
        "03-register-to-pbx",
        "04-call-control",
        "05-blind-transfer",
        "06-attended-transfer",
        "07-secure-call-srtp",
        "08-tls-transport",
        "09-ivr-server",
        "10-call-center-b2bua",
        "11-ai-harness-demo",
        "12-customer-escalation-sip-webrtc",
        "13-sip-to-amazon-connect",
    )
}
SECURITY_REQUIRED_GATES = {
    "dependency advisory audit",
} | {
    f"parser fuzz smoke ({target})"
    for target in (
        "sip_message",
        "uri",
        "header",
        "sdp",
        "rtp_packet",
        "rtcp_packet",
        "srtp_unprotect",
        "dtls_record",
        "stun_response",
        "g711_unpack",
    )
}
INTEROP_REQUIRED_GATES = {
    "SIPp standalone matrix",
    "baresip strict-UA matrix",
    "Kamailio/OpenSIPS proxy de-scope audit",
}
STANDARD_PERFORMANCE_REQUIRED_GATES = {
    "perf results capture boundary",
    "perf regression baseline evidence",
    "perf call setup CPS (endpoint)",
    "perf call setup CPS (pbx-media-server)",
    "perf call setup CPS (signaling-only-server-high-performance)",
    "perf registration throughput",
    "perf concurrent active calls",
    "perf RTP steady state",
    "perf backpressure step",
    "perf transport recovery",
    "perf session churn leak",
    "perf soak candidate",
    "perf regression audit",
    "perf results evidence capture",
    "performance gate metrics report",
}
LITERAL_ALL_PERFORMANCE_REQUIRED_GATES = {
    "literal-all perf configuration",
    "all registered resiliency tests",
    "perf mid-call signaling under media",
    "perf TLS overhead",
    "perf SRTP overhead",
    "perf PDD with 180 first",
    "perf sustained long-duration calls",
    "perf registrar binding scale",
    "perf mixed workload",
    "perf B2BUA forwarding",
    "perf AI-agent load",
    "perf contact-center transfers",
    "perf SIPp parity",
    "perf soak target invariant tests",
    "perf media churn",
    "perf monolithic soak",
    "perf mass teardown stress",
}
FULL_NUMERIC_CONFIGURATION = {
    "beta_perf_high_density_burst_cps": ("exact", 160),
    "beta_perf_high_density_min_asr": ("exact", 0.995),
    "beta_perf_high_density_rss_limit_mb_per_hr": ("exact", 15),
    "beta_perf_media_churn_active_calls": ("minimum", 30),
    "beta_perf_media_churn_duration_secs": ("minimum", 120),
    "beta_perf_monolithic_soak_active_calls": ("minimum", 30),
    "beta_perf_monolithic_soak_duration_secs": ("minimum", 1800),
    "rvoip_perf_soak_duration_secs": ("minimum", 3600),
    "rvoip_perf_soak_active_calls": ("minimum", 500),
    "rvoip_perf_soak_min_hold_secs": ("exact", 10),
    "rvoip_perf_soak_max_hold_secs": ("exact", 360),
    "rvoip_perf_soak_cps": ("exact", 0),
    "rvoip_perf_soak_drain_cps": ("exact", 10),
    "rvoip_perf_retention_drain_wait_secs": ("minimum", 120),
    "rvoip_perf_mass_teardown_calls": ("minimum", 500),
    "rvoip_perf_mass_teardown_setup_cps": ("minimum", 30),
    "rvoip_perf_max_rss_growth_mb_per_hr": ("maximum", 15),
    "rvoip_perf_skip_audio_frame_delivery": ("exact", 0),
}
HEX_40 = re.compile(r"^[0-9a-f]{40}$")
HEX_64 = re.compile(r"^[0-9a-f]{64}$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")
GATE_ROW = re.compile(
    r"^\|\s*(PASS|FAIL|SKIP)\s*\|\s*(.*?)\s*\|\s*(.*?)\s*\|\s*`([^`]+)`\s*\|$"
)


class AttestationError(RuntimeError):
    """Raised when evidence cannot be attested or verified."""


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def parse_timestamp(value: Any, label: str) -> dt.datetime:
    if not isinstance(value, str):
        raise AttestationError(f"{label} must be an ISO-8601 timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise AttestationError(
            f"{label} is not a valid ISO-8601 timestamp: {value!r}"
        ) from error
    if parsed.tzinfo is None:
        raise AttestationError(f"{label} must include a timezone")
    return parsed


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise AttestationError(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def canonical_json_sha256(value: Any) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    return sha256_bytes(encoded)


def hash_frame(digest: Any, value: bytes) -> None:
    digest.update(len(value).to_bytes(8, "little"))
    digest.update(value)


def tree_sha256(root: pathlib.Path) -> str:
    digest = hashlib.sha256(b"rvoip-beta-evidence-tree-v1\0")
    if not root.is_dir():
        raise AttestationError(f"evidence tree does not exist: {root}")
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise AttestationError(f"evidence tree may not contain symlinks: {path}")
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix().encode("utf-8")
        hash_frame(digest, relative)
        hash_frame(digest, path.read_bytes())
    return digest.hexdigest()


def canonical_2k_tree_sha256(root: pathlib.Path) -> str:
    """Hash a canonical 2K run with the canonical packager's domain."""
    digest = hashlib.sha256(b"rvoip-canonical-evidence-tree-v1\0")
    if not root.is_dir():
        raise AttestationError(f"canonical evidence tree does not exist: {root}")
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise AttestationError(
                f"canonical evidence tree may not contain symlinks: {path}"
            )
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix().encode("utf-8")
        hash_frame(digest, relative)
        hash_frame(digest, path.read_bytes())
    return digest.hexdigest()


def load_json(path: pathlib.Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise AttestationError(f"cannot read {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise AttestationError(f"{label} must be a JSON object: {path}")
    return value


def relative_file(
    root: pathlib.Path, value: str | pathlib.Path, label: str
) -> pathlib.Path:
    candidate = (root / value).resolve()
    try:
        candidate.relative_to(root)
    except ValueError as error:
        raise AttestationError(f"{label} escapes report root: {value}") from error
    if not candidate.is_file() or candidate.is_symlink():
        raise AttestationError(f"{label} is not a regular report file: {value}")
    return candidate


def relative_path(value: Any, label: str) -> pathlib.PurePosixPath:
    if not isinstance(value, str) or not value:
        raise AttestationError(f"{label} is missing")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts:
        raise AttestationError(f"{label} must be a report-relative path: {value!r}")
    return path


def source_block(root: pathlib.Path, path: pathlib.Path) -> dict[str, Any]:
    source = load_json(path, "source fingerprint")
    for key, pattern in (
        ("git_commit", HEX_40),
        ("git_tree", HEX_40),
        ("source_fingerprint_sha256", HEX_64),
    ):
        value = source.get(key)
        if not isinstance(value, str) or pattern.fullmatch(value) is None:
            raise AttestationError(f"source fingerprint has invalid {key}: {value!r}")
    if not isinstance(source.get("git_dirty"), bool):
        raise AttestationError("source fingerprint git_dirty must be boolean")
    try:
        relative = path.resolve().relative_to(root).as_posix()
    except ValueError as error:
        raise AttestationError(
            f"source fingerprint must be copied under report root: {path}"
        ) from error
    return {
        "git_commit": source["git_commit"],
        "git_tree": source["git_tree"],
        "git_dirty": source["git_dirty"],
        "source_fingerprint_sha256": source["source_fingerprint_sha256"],
        "evidence_path": relative,
        "evidence_sha256": sha256_file(path),
    }


def safe_label(value: str) -> str:
    label = re.sub(r"[^a-z0-9_.-]+", "-", value.lower()).strip("-.")
    if not label:
        raise AttestationError(f"invalid input label: {value!r}")
    return label


def parse_named_path(value: str) -> tuple[str, pathlib.Path]:
    if "=" not in value:
        raise AttestationError(f"expected NAME=PATH, found {value!r}")
    name, raw_path = value.split("=", 1)
    if not raw_path:
        raise AttestationError(f"missing path in {value!r}")
    return safe_label(name), pathlib.Path(raw_path).expanduser().resolve()


def required_inputs_for_mode(mode: str) -> set[str]:
    return REQUIRED_INPUTS | MODE_REQUIRED_INPUTS.get(mode, set())


def copy_inputs(
    root: pathlib.Path, values: Iterable[str], mode: str
) -> dict[str, dict[str, Any]]:
    destination_root = root / "inputs"
    destination_root.mkdir(parents=True, exist_ok=True)
    inputs: dict[str, dict[str, Any]] = {}
    for encoded in values:
        label, source = parse_named_path(encoded)
        if label in inputs:
            raise AttestationError(f"duplicate attestation input: {label}")
        if not source.is_file() or source.is_symlink():
            raise AttestationError(f"attestation input does not exist: {source}")
        suffix = "".join(source.suffixes[-2:]) or ".input"
        destination = destination_root / f"{label}{suffix}"
        if source != destination.resolve():
            shutil.copyfile(source, destination)
        inputs[label] = {
            "path": destination.relative_to(root).as_posix(),
            "sha256": sha256_file(destination),
            "bytes": destination.stat().st_size,
            "source_name": source.name,
        }
    missing = sorted(required_inputs_for_mode(mode) - set(inputs))
    if missing:
        raise AttestationError(f"required attestation inputs are missing: {missing}")
    return inputs


def state_table_block(
    inputs: dict[str, dict[str, Any]],
    selected_source: str,
    fallback_reason: str | None,
    expected_sha256: str | None,
) -> dict[str, Any]:
    if selected_source not in VALID_STATE_TABLE_SOURCES:
        raise AttestationError(
            f"invalid selected state-table source: {selected_source!r}"
        )
    if selected_source == "configured-path-fallback":
        if fallback_reason not in VALID_STATE_TABLE_FALLBACK_REASONS:
            raise AttestationError(
                "configured state-table fallback requires a bounded fallback reason"
            )
    elif fallback_reason not in (None, ""):
        raise AttestationError(
            "state-table fallback reason is only valid for configured-path-fallback"
        )
    selected = inputs["state-machine-yaml"]
    if expected_sha256 is not None:
        if HEX_64.fullmatch(expected_sha256) is None:
            raise AttestationError("expected selected state-table hash is invalid")
        if selected["sha256"] != expected_sha256:
            raise AttestationError(
                "selected state-table YAML changed after the gate captured its hash"
            )
    return {
        "selected_source": selected_source,
        "fallback_reason": fallback_reason or None,
        "selected_yaml_path": selected["path"],
        "selected_yaml_source_name": selected["source_name"],
        "selected_yaml_sha256": selected["sha256"],
        "selected_yaml_bytes": selected["bytes"],
    }


def captured_lines(path: pathlib.Path) -> list[str]:
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as error:
        raise AttestationError(
            f"cannot read captured evidence {path}: {error}"
        ) from error
    if lines and lines[0].startswith("+ "):
        lines = lines[1:]
    return lines


def first_payload_line(path: pathlib.Path) -> str:
    return next(
        (line.strip() for line in captured_lines(path) if line.strip()), "unknown"
    )


def read_captured_json(path: pathlib.Path) -> Any:
    payload = "\n".join(captured_lines(path))
    try:
        return json.loads(payload)
    except ValueError as error:
        raise AttestationError(f"invalid captured JSON in {path}: {error}") from error


def cargo_package(cargo_metadata_path: pathlib.Path) -> tuple[str, str]:
    metadata = load_json(cargo_metadata_path, "Cargo metadata")
    for package in metadata.get("packages", []):
        if isinstance(package, dict) and package.get("name") == "rvoip-sip":
            version = package.get("version")
            if not isinstance(version, str) or SEMVER.fullmatch(version) is None:
                raise AttestationError(
                    f"Cargo metadata has invalid rvoip-sip version: {version!r}"
                )
            return "rvoip-sip", version
    raise AttestationError("Cargo metadata does not contain rvoip-sip")


def compiler_block(
    root: pathlib.Path, requested_target: str | None = None
) -> dict[str, Any]:
    rustc_path = root / "environment/rustc-version.txt"
    cargo_path = root / "environment/cargo-version.txt"
    rustc_lines = captured_lines(rustc_path)
    cargo_lines = captured_lines(cargo_path)
    host = next(
        (
            line.split(":", 1)[1].strip()
            for line in rustc_lines
            if line.startswith("host:")
        ),
        "unknown",
    )
    if host == "unknown":
        raise AttestationError("rustc target host is absent from environment evidence")
    target = requested_target.strip() if isinstance(requested_target, str) else ""
    return {
        "rustc": rustc_lines[0] if rustc_lines else "unknown",
        "cargo": cargo_lines[0] if cargo_lines else "unknown",
        "target": target or host,
        "target_source": "explicit" if target else "rustc-host",
        "rustc_evidence_path": rustc_path.relative_to(root).as_posix(),
        "rustc_evidence_sha256": sha256_file(rustc_path),
        "cargo_evidence_path": cargo_path.relative_to(root).as_posix(),
        "cargo_evidence_sha256": sha256_file(cargo_path),
    }


def source_fingerprints(value: Any) -> set[str]:
    found: set[str] = set()
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "source_fingerprint_sha256" and isinstance(child, str):
                found.add(child)
            else:
                found.update(source_fingerprints(child))
    elif isinstance(value, list):
        for child in value:
            found.update(source_fingerprints(child))
    return found


def copy_executables(
    root: pathlib.Path,
    fingerprint: str,
    target_directory: pathlib.Path | None,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    destination_root = root / "executables"
    executable_by_hash: dict[str, dict[str, Any]] = {}
    ignored: list[dict[str, Any]] = []

    def record(source: pathlib.Path, expected: str, provenance: str) -> None:
        actual = sha256_file(source)
        if actual != expected:
            raise AttestationError(
                f"executable hash mismatch for {source}: expected {expected}, found {actual}"
            )
        if expected not in executable_by_hash:
            destination_root.mkdir(parents=True, exist_ok=True)
            destination = destination_root / f"{expected}-{safe_label(source.name)}"
            if source != destination.resolve():
                shutil.copyfile(source, destination)
                destination.chmod(source.stat().st_mode & 0o777)
            executable_by_hash[expected] = {
                "sha256": expected,
                "bytes": destination.stat().st_size,
                "path": destination.relative_to(root).as_posix(),
                "provenance": [provenance],
            }
        elif provenance not in executable_by_hash[expected]["provenance"]:
            executable_by_hash[expected]["provenance"].append(provenance)

    for manifest_path in sorted(root.rglob("manifest.json")):
        if manifest_path.is_symlink():
            raise AttestationError(f"manifest may not be a symlink: {manifest_path}")
        try:
            manifest = load_json(manifest_path, "nested manifest")
        except AttestationError:
            continue
        expected = manifest.get("executable_sha256")
        executable = manifest.get("executable")
        if not isinstance(expected, str) or HEX_64.fullmatch(expected) is None:
            continue
        provenance = manifest_path.relative_to(root).as_posix()
        if fingerprint not in source_fingerprints(manifest):
            ignored.append(
                {
                    "sha256": expected,
                    "provenance": provenance,
                    "reason": "different-source",
                }
            )
            continue
        if not isinstance(executable, str) or not executable:
            raise AttestationError(
                f"current-source manifest lacks executable path: {provenance}"
            )
        source = pathlib.Path(executable).expanduser().resolve()
        if not source.is_file() or source.is_symlink():
            # Canonical imports rewrite the usable binary location into index.json.
            packaged = manifest_path.parent / "executable"
            if packaged.is_file() and not packaged.is_symlink():
                source = packaged
            else:
                raise AttestationError(
                    f"current-source executable is unavailable: {executable}"
                )
        record(source, expected, provenance)

    if target_directory is not None:
        target_directory = target_directory.expanduser().resolve()
        absolute_pattern = re.compile(
            r"(?:[A-Za-z]:)?/[^\s`\"'()]+/target/(?:debug|release)/[^\s`\"'()]+"
        )
        relative_pattern = re.compile(
            r"(?<![/A-Za-z0-9_.-])target/(?:debug|release)/[^\s`\"'()]+"
        )
        evidence_files = sorted(root.rglob("*.log")) + sorted(
            root.rglob("*_metadata.md")
        )
        for evidence_path in evidence_files:
            relative_evidence = evidence_path.relative_to(root).as_posix().lower()
            claim_bearing = relative_evidence.startswith(
                (
                    "pbx/",
                    "sipp/",
                    "strict-ua/",
                    "security/",
                    "perf-results/",
                    "canonical-2k/",
                )
            ) or any(
                marker in evidence_path.name.lower()
                for marker in (
                    "perf",
                    "pbx",
                    "sipp",
                    "strict",
                    "fuzz",
                    "soak",
                    "burst",
                    "canonical",
                )
            )
            if not claim_bearing:
                continue
            try:
                content = evidence_path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            candidates = absolute_pattern.findall(content) + relative_pattern.findall(
                content
            )
            for encoded in candidates:
                encoded = encoded.rstrip(".,:;]")
                source = pathlib.Path(encoded)
                if not source.is_absolute():
                    source = target_directory.parent / source
                source = source.resolve()
                try:
                    source.relative_to(target_directory)
                except ValueError:
                    continue
                if (
                    not source.is_file()
                    or source.is_symlink()
                    or not os.access(source, os.X_OK)
                ):
                    continue
                provenance = evidence_path.relative_to(root).as_posix()
                record(source, sha256_file(source), provenance)

    canonical_index = root / "canonical-2k/index.json"
    if canonical_index.is_file():
        index = load_json(canonical_index, "canonical 2K index")
        expected = index.get("common_executable_sha256")
        packaged_value = index.get("packaged_executable")
        if not isinstance(expected, str) or HEX_64.fullmatch(expected) is None:
            raise AttestationError("canonical 2K index executable hash is invalid")
        if not isinstance(packaged_value, str):
            raise AttestationError("canonical 2K index packaged executable is missing")
        packaged = relative_file(
            root, pathlib.Path("canonical-2k") / packaged_value, "canonical executable"
        )
        actual = sha256_file(packaged)
        if actual != expected:
            raise AttestationError("canonical 2K packaged executable hash mismatch")
        if expected in executable_by_hash:
            entry = executable_by_hash[expected]
            if "canonical-2k/index.json" not in entry["provenance"]:
                entry["provenance"].append("canonical-2k/index.json")
        else:
            # The canonical executable is already copied under canonical-2k;
            # retain that stable relative path instead of duplicating it.
            executable_by_hash[expected] = {
                "sha256": expected,
                "bytes": packaged.stat().st_size,
                "path": packaged.relative_to(root).as_posix(),
                "provenance": ["canonical-2k/index.json"],
            }

    return sorted(executable_by_hash.values(), key=lambda item: item["sha256"]), ignored


def markdown_field(path: pathlib.Path, field: str) -> str | None:
    prefix = f"- {field}:"
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    for line in lines:
        if line.startswith(prefix):
            return line[len(prefix) :].strip().strip("`")
    return None


def peers_block(
    root: pathlib.Path, inputs: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    peers: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()

    raw_inspect_paths = sorted(
        (root / "environment").glob("docker-*/rvoip-*-inspect.json")
    )
    if raw_inspect_paths:
        raise AttestationError(
            "raw Docker inspect evidence is forbidden; capture only an allowlisted "
            f"sanitized peer snapshot: {raw_inspect_paths[0]}"
        )

    for snapshot_path in sorted(
        (root / "environment").glob("docker-*/rvoip-*-peer.json")
    ):
        payload = read_captured_json(snapshot_path)
        if not isinstance(payload, dict):
            raise AttestationError(
                f"unexpected sanitized Docker peer snapshot: {snapshot_path}"
            )
        if payload.get("schema") != DOCKER_PEER_SCHEMA:
            raise AttestationError(
                f"unsupported sanitized Docker peer snapshot schema: {snapshot_path}"
            )
        product = payload.get("product")
        expected_product = (
            snapshot_path.name.removesuffix("-peer.json").removeprefix("rvoip-")
        )
        image = payload.get("image")
        config = payload.get("configuration")
        if not isinstance(product, str) or not product:
            raise AttestationError(f"Docker peer product missing: {snapshot_path}")
        if product != expected_product:
            raise AttestationError(
                "Docker peer product disagrees with its evidence filename: "
                f"{snapshot_path}"
            )
        if not isinstance(image, dict):
            raise AttestationError(f"Docker peer image missing: {snapshot_path}")
        image_digest = image.get("id")
        if not isinstance(image_digest, str) or not image_digest:
            raise AttestationError(
                f"Docker peer image digest missing: {snapshot_path}"
            )
        if not isinstance(config, dict):
            raise AttestationError(f"Docker peer config missing: {snapshot_path}")
        version = image.get("reference")
        if version is not None and not isinstance(version, str):
            raise AttestationError(
                f"Docker peer image reference is invalid: {snapshot_path}"
            )
        key = (product, image_digest)
        if key in seen:
            continue
        seen.add(key)
        peers.append(
            {
                "product": product,
                "version": version,
                "image_digest": image_digest,
                "config_sha256": canonical_json_sha256(config),
                "evidence_paths": [snapshot_path.relative_to(root).as_posix()],
            }
        )

    for product in ("asterisk", "freeswitch"):
        evidence_dir = root / "environment/local-pbx" / product
        revision_path = evidence_dir / "git-rev.txt"
        if not revision_path.is_file():
            continue
        version = first_payload_line(revision_path)
        key = (product, version)
        if key in seen:
            continue
        seen.add(key)
        peers.append(
            {
                "product": product,
                "version": version,
                "image_digest": None,
                "config_sha256": tree_sha256(evidence_dir),
                "evidence_paths": [evidence_dir.relative_to(root).as_posix()],
            }
        )

    strict_environment = root / "strict-ua/environment.md"
    if strict_environment.is_file():
        version = markdown_field(strict_environment, "baresip")
        config_dir = root / "strict-ua/baresip"
        if version and config_dir.is_dir():
            peers.append(
                {
                    "product": "baresip",
                    "version": version.splitlines()[0],
                    "image_digest": None,
                    "config_sha256": tree_sha256(config_dir),
                    "evidence_paths": [
                        strict_environment.relative_to(root).as_posix(),
                        config_dir.relative_to(root).as_posix(),
                    ],
                }
            )

    sipp_environment = root / "sipp/environment.md"
    sipp_input = inputs.get("sipp-scenario")
    if sipp_environment.is_file() and sipp_input:
        version = markdown_field(sipp_environment, "sipp")
        if version:
            peers.append(
                {
                    "product": "sipp",
                    "version": version,
                    "image_digest": None,
                    "config_sha256": sipp_input["sha256"],
                    "evidence_paths": [
                        sipp_environment.relative_to(root).as_posix(),
                        sipp_input["path"],
                    ],
                }
            )

    return sorted(
        peers,
        key=lambda item: (
            item["product"],
            item.get("version") or item.get("image_digest") or "",
        ),
    )


def parse_gates(root: pathlib.Path) -> list[dict[str, Any]]:
    summary_path = root / "summary.md"
    try:
        lines = summary_path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise AttestationError(f"cannot read beta summary: {error}") from error
    gates: list[dict[str, Any]] = []
    names: set[str] = set()
    in_gates = False
    for line in lines:
        if line == "## Gates":
            in_gates = True
            continue
        if in_gates and line.startswith("## "):
            break
        if not in_gates or not line.startswith("|"):
            continue
        if line.startswith("| Status ") or re.fullmatch(r"\|[-|]+\|", line):
            continue
        match = GATE_ROW.match(line)
        if not match:
            raise AttestationError(f"invalid or unsupported gate row: {line!r}")
        status, name, duration, relative_log = match.groups()
        if name in names:
            raise AttestationError(f"duplicate gate name in beta summary: {name}")
        names.add(name)
        log_path = relative_file(root, relative_log, f"gate log for {name}")
        duration_match = re.fullmatch(r"(\d+)s", duration)
        if duration_match is None:
            raise AttestationError(
                f"gate {name!r} lacks a numeric duration in seconds: {duration!r}"
            )
        gates.append(
            {
                "status": status,
                "name": name,
                "duration": duration,
                "duration_seconds": int(duration_match.group(1)),
                "log_path": pathlib.PurePosixPath(relative_log).as_posix(),
                "log_sha256": sha256_file(log_path),
            }
        )
    if not gates:
        raise AttestationError("beta summary contains no gate rows")
    return gates


def summary_result(root: pathlib.Path) -> dict[str, int]:
    summary_path = root / "summary.md"
    try:
        lines = summary_path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise AttestationError(f"cannot read beta summary result: {error}") from error
    in_result = False
    values: dict[str, int] = {}
    pattern = re.compile(r"^- (failures|skips): (\d+)$")
    for line in lines:
        if line == "## Result":
            in_result = True
            continue
        if in_result and line.startswith("## "):
            break
        if not in_result:
            continue
        match = pattern.fullmatch(line)
        if match:
            values[match.group(1)] = int(match.group(2))
    if set(values) != {"failures", "skips"}:
        raise AttestationError("summary Result must contain numeric failures and skips")
    return values


def enabled_config(value: str | None) -> bool:
    return (value or "").lower() in {"1", "true", "yes", "on"}


def reconcile_build_config(
    gate_config: dict[str, str],
    features: list[str],
    toolchain: dict[str, Any],
) -> None:
    encoded_features = gate_config.get("beta_attestation_features")
    if encoded_features is None:
        raise AttestationError("summary omits beta_attestation_features")
    expected_features = sorted(
        set(part.strip() for part in encoded_features.split(",") if part.strip())
    )
    if features != expected_features:
        raise AttestationError("build features disagree with summary.md")
    configured_target = gate_config.get("beta_attestation_target")
    if configured_target in (None, ""):
        raise AttestationError("summary omits beta_attestation_target")
    if configured_target == "rustc-host":
        if toolchain.get("target_source") != "rustc-host":
            raise AttestationError("build target source disagrees with summary.md")
    elif (
        toolchain.get("target_source") != "explicit"
        or toolchain.get("target") != configured_target
    ):
        raise AttestationError("build target disagrees with summary.md")


def reconcile_result_counts(
    root: pathlib.Path,
    gates: list[dict[str, Any]],
    gate_config: dict[str, str],
    failures: int,
    skips: int,
) -> dict[str, int]:
    summary = summary_result(root)
    if summary != {"failures": failures, "skips": skips}:
        raise AttestationError(
            "attestation failure/skip counts disagree with summary.md "
            f"(arguments={{'failures': {failures}, 'skips': {skips}}}, summary={summary})"
        )
    failed_gates = sum(gate["status"] == "FAIL" for gate in gates)
    skipped_gates = sum(gate["status"] == "SKIP" for gate in gates)
    if skips != skipped_gates:
        raise AttestationError(
            f"skip count {skips} disagrees with {skipped_gates} SKIP gate rows"
        )
    require_external = enabled_config(gate_config.get("beta_gate_require_external"))
    expected_failures = failed_gates + (skipped_gates if require_external else 0)
    if failures != expected_failures:
        raise AttestationError(
            f"failure count {failures} disagrees with gate rows; expected "
            f"{expected_failures} (require_external={require_external})"
        )
    return {
        "failed_gates": failed_gates,
        "skipped_gates": skipped_gates,
        "required_skips_counted_as_failures": skipped_gates if require_external else 0,
    }


def classify_artifact(relative: str) -> str:
    if relative.endswith(".log") or "/logs/" in relative:
        return "log"
    if relative.startswith("inputs/") or relative.startswith(
        "canonical-2k/reviewed-baseline/"
    ) or relative.startswith("perf-regression-baseline/"):
        return "input"
    if relative.startswith("executables/") or relative.startswith(
        "canonical-2k/executable"
    ):
        return "executable"
    if relative.endswith(".json"):
        return "json"
    if relative.endswith(".md"):
        return "report"
    return "artifact"


def artifact_inventory(root: pathlib.Path) -> list[dict[str, Any]]:
    files: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise AttestationError(f"report may not contain symlinks: {path}")
        if not path.is_file() or path.name in EXCLUDED_FILES:
            continue
        relative = path.relative_to(root).as_posix()
        files.append(
            {
                "path": relative,
                "sha256": sha256_file(path),
                "bytes": path.stat().st_size,
                "kind": classify_artifact(relative),
            }
        )
    return files


def canonical_2k_block(root: pathlib.Path, fingerprint: str) -> dict[str, Any]:
    index_path = root / "canonical-2k/index.json"
    if not index_path.is_file():
        return {
            "status": "absent",
            "absent_reason": "canonical-2k/index.json was not captured for this run",
            "index_path": None,
            "index_sha256": None,
            "reviewed_baseline": None,
            "runs": [],
        }

    index = load_json(index_path, "canonical 2K index")
    expected_fields = {
        "schema": CANONICAL_2K_SCHEMA,
        "status": "PASS",
        "scenario": CANONICAL_2K_SCENARIO,
        "run_count": CANONICAL_2K_RUN_COUNT,
        "common_source_fingerprint_sha256": fingerprint,
    }
    for key, expected in expected_fields.items():
        if index.get(key) != expected:
            raise AttestationError(
                f"canonical 2K index {key} must be {expected!r}, found {index.get(key)!r}"
            )
    common_executable = index.get("common_executable_sha256")
    require_hash(common_executable, "canonical 2K common executable hash")
    reviewed_baseline = index.get("reviewed_baseline")
    expected_baseline = {
        "id": CANONICAL_2K_BASELINE_ID,
        "relative_path": CANONICAL_2K_BASELINE_RELATIVE_PATH,
        "sha256": CANONICAL_2K_BASELINE_SHA256,
        "packaged_path": CANONICAL_2K_BASELINE_PACKAGED_PATH,
    }
    if reviewed_baseline != expected_baseline:
        raise AttestationError(
            "canonical 2K reviewed baseline identity does not match the approved input"
        )
    packaged_baseline_relative = f"canonical-2k/{CANONICAL_2K_BASELINE_PACKAGED_PATH}"
    packaged_baseline = root / packaged_baseline_relative
    if not packaged_baseline.is_file():
        raise AttestationError("canonical 2K reviewed baseline artifact is missing")
    if sha256_file(packaged_baseline) != CANONICAL_2K_BASELINE_SHA256:
        raise AttestationError("canonical 2K reviewed baseline artifact hash differs")
    indexed_runs = index.get("runs")
    if (
        not isinstance(indexed_runs, list)
        or len(indexed_runs) != CANONICAL_2K_RUN_COUNT
    ):
        raise AttestationError("canonical 2K index must describe exactly three runs")

    runs: list[dict[str, Any]] = []
    for sequence, indexed in enumerate(indexed_runs, start=1):
        if not isinstance(indexed, dict):
            raise AttestationError("canonical 2K run index entry must be an object")
        relative = f"canonical-2k/run-{sequence}"
        expected_run_fields = {
            "sequence": sequence,
            "packaged_run_dir": f"run-{sequence}",
            "source_fingerprint_sha256": fingerprint,
            "executable_sha256": common_executable,
            "reviewed_baseline_id": CANONICAL_2K_BASELINE_ID,
            "reviewed_baseline_relative_path": CANONICAL_2K_BASELINE_RELATIVE_PATH,
            "reviewed_baseline_sha256": CANONICAL_2K_BASELINE_SHA256,
        }
        for key, expected in expected_run_fields.items():
            if indexed.get(key) != expected:
                raise AttestationError(
                    f"canonical 2K run {sequence} {key} must be {expected!r}, "
                    f"found {indexed.get(key)!r}"
                )
        if indexed.get("reviewed_baseline_origin") not in CANONICAL_2K_BASELINE_ORIGINS:
            raise AttestationError(
                f"canonical 2K run {sequence} reviewed baseline origin is invalid"
            )
        expected_tree = indexed.get("packaged_tree_sha256")
        require_hash(expected_tree, f"canonical 2K run {sequence} tree hash")
        run_dir = root / relative
        actual_tree = canonical_2k_tree_sha256(run_dir)
        if actual_tree != expected_tree:
            raise AttestationError(
                f"canonical 2K run {sequence} tree hash disagrees with index"
            )
        captured_at = indexed.get("captured_at_utc")
        parse_timestamp(captured_at, f"canonical 2K run {sequence} captured_at_utc")
        runs.append(
            {
                "sequence": sequence,
                "path": relative,
                "captured_at_utc": captured_at,
                "tree_sha256": actual_tree,
                "source_fingerprint_sha256": fingerprint,
                "executable_sha256": common_executable,
                "reviewed_baseline_sha256": CANONICAL_2K_BASELINE_SHA256,
                "reviewed_baseline_origin": indexed["reviewed_baseline_origin"],
            }
        )

    return {
        "status": "captured",
        "absent_reason": None,
        "index_path": index_path.relative_to(root).as_posix(),
        "index_sha256": sha256_file(index_path),
        "common_executable_sha256": common_executable,
        "reviewed_baseline": {
            "id": CANONICAL_2K_BASELINE_ID,
            "relative_path": CANONICAL_2K_BASELINE_RELATIVE_PATH,
            "sha256": CANONICAL_2K_BASELINE_SHA256,
            "path": packaged_baseline_relative,
        },
        "runs": runs,
    }


def result_block(
    root: pathlib.Path,
    artifacts: list[dict[str, Any]],
    fingerprint: str,
) -> dict[str, Any]:
    json_results = [
        {"path": item["path"], "sha256": item["sha256"], "bytes": item["bytes"]}
        for item in artifacts
        if item["path"].endswith(".json")
        and not item["path"].startswith("environment/")
        and not item["path"].startswith("inputs/")
        and not item["path"].startswith("canonical-2k/reviewed-baseline/")
        and not item["path"].startswith("perf-regression-baseline/")
        and "/docker-" not in item["path"]
        and item["path"] not in PERFORMANCE_GATE_METRICS_PATHS
    ]
    performance_json_count = sum(
        item["path"].startswith("perf-results/") for item in json_results
    )
    return {
        "json_evidence": {
            "status": "captured" if json_results else "absent",
            "count": len(json_results),
            "absent_reason": (
                None if json_results else "no result JSON was produced for this mode"
            ),
        },
        "performance_json_evidence": evidence_status(
            performance_json_count,
            "no performance result JSON was captured for this mode",
        ),
        "json": json_results,
        "canonical_2k": canonical_2k_block(root, fingerprint),
    }


def effective_gate_config(root: pathlib.Path) -> dict[str, str]:
    summary_path = root / "summary.md"
    try:
        lines = summary_path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise AttestationError(
            f"cannot read effective gate configuration: {error}"
        ) from error
    in_environment = False
    values: dict[str, str] = {}
    pattern = re.compile(r"^- ([a-z0-9_]+): `([^`]*)`$")
    for line in lines:
        if line == "## Environment Snapshot":
            in_environment = True
            continue
        if in_environment and line.startswith("## "):
            break
        if not in_environment:
            continue
        match = pattern.match(line)
        if match:
            values[match.group(1)] = match.group(2)
    if not values:
        raise AttestationError("summary has no machine-readable Environment Snapshot")
    return values


def runtime_effective_configs(
    root: pathlib.Path, fingerprint: str
) -> list[dict[str, Any]]:
    runtime: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*.json")):
        if path.name in EXCLUDED_FILES:
            continue
        try:
            value = load_json(path, "runtime result")
        except AttestationError:
            continue
        if fingerprint not in source_fingerprints(value):
            continue
        effective = value.get("effective_config")
        if effective is not None:
            runtime.append(
                {
                    "evidence_path": path.relative_to(root).as_posix(),
                    "effective_config_sha256": canonical_json_sha256(effective),
                }
            )
    return runtime


def effective_config_block(
    root: pathlib.Path,
    fingerprint: str,
    gate_config: dict[str, str] | None = None,
    result_reconciliation: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    redacted = root / "environment/beta-env-redacted.txt"
    if not redacted.is_file():
        raise AttestationError("redacted beta environment is missing")
    runtime = runtime_effective_configs(root, fingerprint)
    gate_config = effective_gate_config(root) if gate_config is None else gate_config
    result_reconciliation = (
        performance_result_reconciliation(root, fingerprint, gate_config)
        if result_reconciliation is None
        else result_reconciliation
    )
    redacted_environment_sha256 = sha256_file(redacted)
    effective_gate_config_sha256 = canonical_json_sha256(gate_config)
    combined = {
        "effective_gate_config_sha256": effective_gate_config_sha256,
        "redacted_environment_sha256": redacted_environment_sha256,
        "runtime_effective_configs": runtime,
        "performance_result_reconciliation": result_reconciliation,
    }
    return {
        "effective_gate_config_source_path": "summary.md",
        "effective_gate_config": gate_config,
        "effective_gate_config_sha256": effective_gate_config_sha256,
        "effective_gate_config_keys": sorted(gate_config),
        "redacted_environment_path": redacted.relative_to(root).as_posix(),
        "redacted_environment_sha256": redacted_environment_sha256,
        "runtime_effective_configs": runtime,
        "performance_result_reconciliation": result_reconciliation,
        "effective_redacted_configuration_sha256": canonical_json_sha256(combined),
    }


def performance_inputs_block(inputs: dict[str, dict[str, Any]]) -> dict[str, Any]:
    recipe = inputs["performance-recipe"]
    burst = inputs["burst-scenarios"]
    return {
        "recipe_path": recipe["path"],
        "recipe_sha256": recipe["sha256"],
        "recipe_bytes": recipe["bytes"],
        "burst_scenarios_path": burst["path"],
        "burst_scenarios_sha256": burst["sha256"],
        "burst_scenarios_bytes": burst["bytes"],
    }


def performance_regression_baseline_block(
    root: pathlib.Path,
    inputs: dict[str, dict[str, Any]],
    mode: str,
) -> dict[str, Any]:
    baseline_input = inputs.get("performance-regression-baseline")
    package_root = root / PERFORMANCE_REGRESSION_BASELINE_PACKAGE
    packaged_manifest = package_root / "manifest.json"
    if baseline_input is None:
        if package_root.exists():
            raise AttestationError(
                "performance regression baseline package has no attested input"
            )
        return {
            "status": "absent",
            "absent_reason": (
                "performance regression baseline is not required for this mode"
            ),
            "baseline_id": None,
            "input_path": None,
            "input_sha256": None,
            "manifest_path": None,
            "manifest_sha256": None,
            "qualification": None,
            "source": None,
            "comparison_paths": [],
            "files": [],
        }
    if mode == "full" and not packaged_manifest.is_file():
        raise AttestationError(
            "full release evidence lacks perf-regression-baseline/manifest.json"
        )
    if not packaged_manifest.is_file() or packaged_manifest.is_symlink():
        if mode != "full" and not package_root.exists():
            input_manifest = relative_file(
                root,
                baseline_input["path"],
                "performance regression baseline input",
            )
            input_value = load_json(
                input_manifest, "performance regression baseline input"
            )
            return {
                "status": "absent",
                "absent_reason": (
                    "reviewed baseline input was recorded but was not exercised "
                    "or packaged for this mode"
                ),
                "baseline_id": input_value.get("baseline_id"),
                "input_path": input_manifest.relative_to(root).as_posix(),
                "input_sha256": baseline_input["sha256"],
                "manifest_path": None,
                "manifest_sha256": None,
                "qualification": input_value.get("qualification"),
                "source": input_value.get("source"),
                "comparison_paths": [],
                "files": [],
            }
        raise AttestationError(
            "performance regression baseline manifest is not a regular packaged file"
        )
    input_manifest = relative_file(
        root, baseline_input["path"], "performance regression baseline input"
    )
    if sha256_file(packaged_manifest) != baseline_input["sha256"]:
        raise AttestationError(
            "packaged performance regression baseline manifest disagrees with input"
        )
    manifest = load_json(packaged_manifest, "performance regression baseline manifest")
    if manifest.get("schema") != PERFORMANCE_REGRESSION_BASELINE_SCHEMA:
        raise AttestationError("performance regression baseline schema is unsupported")
    baseline_id = manifest.get("baseline_id")
    if not isinstance(baseline_id, str) or not baseline_id:
        raise AttestationError("performance regression baseline id is missing")
    qualification = manifest.get("qualification")
    if (
        not isinstance(qualification, dict)
        or qualification.get("release_evidence") is not False
        or qualification.get("permitted_use") in (None, "", [])
    ):
        raise AttestationError(
            "performance regression baseline qualification is invalid"
        )
    source = manifest.get("source")
    if (
        not isinstance(source, dict)
        or not isinstance(source.get("git_revision"), str)
        or not source["git_revision"]
        or not isinstance(source.get("git_status"), str)
        or not source["git_status"]
    ):
        raise AttestationError("performance regression baseline source is invalid")
    raw_files = manifest.get("files")
    if not isinstance(raw_files, list) or not raw_files:
        raise AttestationError("performance regression baseline file list is empty")
    files: list[dict[str, Any]] = []
    baseline_paths: set[str] = set()
    expected_packaged_paths = {"manifest.json"}
    for item in raw_files:
        if not isinstance(item, dict):
            raise AttestationError(
                "performance regression baseline file entry is invalid"
            )
        baseline_path = relative_path(
            item.get("path"), "performance regression baseline file path"
        ).as_posix()
        if baseline_path in baseline_paths:
            raise AttestationError(
                f"duplicate performance regression baseline path: {baseline_path}"
            )
        baseline_paths.add(baseline_path)
        expected_bytes = item.get("bytes")
        expected_sha256 = item.get("sha256")
        if not isinstance(expected_bytes, int) or expected_bytes < 0:
            raise AttestationError(
                f"performance regression baseline byte count is invalid: {baseline_path}"
            )
        require_hash(
            expected_sha256,
            f"performance regression baseline hash for {baseline_path}",
        )
        packaged_relative = pathlib.PurePosixPath("perf-results") / baseline_path
        packaged = relative_file(
            package_root,
            packaged_relative,
            f"packaged performance regression baseline file {baseline_path}",
        )
        if packaged.stat().st_size != expected_bytes:
            raise AttestationError(
                f"performance regression baseline byte count differs: {baseline_path}"
            )
        if sha256_file(packaged) != expected_sha256:
            raise AttestationError(
                f"performance regression baseline hash differs: {baseline_path}"
            )
        expected_packaged_paths.add(packaged_relative.as_posix())
        files.append(
            {
                "baseline_path": baseline_path,
                "path": packaged.relative_to(root).as_posix(),
                "bytes": expected_bytes,
                "sha256": expected_sha256,
            }
        )
    comparison_paths = manifest.get("comparison_paths")
    if not isinstance(comparison_paths, list) or not comparison_paths:
        raise AttestationError(
            "performance regression baseline comparison path list is empty"
        )
    normalized_comparison_paths = [
        relative_path(path, "performance regression comparison path").as_posix()
        for path in comparison_paths
    ]
    if (
        len(set(normalized_comparison_paths)) != len(normalized_comparison_paths)
        or not set(normalized_comparison_paths).issubset(baseline_paths)
    ):
        raise AttestationError(
            "performance regression comparison paths must uniquely reference packaged files"
        )
    actual_packaged_paths = {
        path.relative_to(package_root).as_posix()
        for path in package_root.rglob("*")
        if path.is_file()
    }
    for path in package_root.rglob("*"):
        if path.is_symlink():
            raise AttestationError(
                f"performance regression baseline package may not contain symlinks: {path}"
            )
    if actual_packaged_paths != expected_packaged_paths:
        missing = sorted(expected_packaged_paths - actual_packaged_paths)
        added = sorted(actual_packaged_paths - expected_packaged_paths)
        raise AttestationError(
            "performance regression baseline package inventory differs from manifest; "
            f"missing={missing}, added={added}"
        )
    return {
        "status": "captured",
        "absent_reason": None,
        "baseline_id": baseline_id,
        "input_path": input_manifest.relative_to(root).as_posix(),
        "input_sha256": baseline_input["sha256"],
        "manifest_path": packaged_manifest.relative_to(root).as_posix(),
        "manifest_sha256": sha256_file(packaged_manifest),
        "qualification": qualification,
        "source": source,
        "comparison_paths": normalized_comparison_paths,
        "files": files,
    }


def evidence_status(count: int, absent_reason: str) -> dict[str, Any]:
    return {
        "status": "captured" if count else "absent",
        "count": count,
        "absent_reason": None if count else absent_reason,
    }


def missing_paths(
    artifacts: list[dict[str, Any]], required: set[str]
) -> list[str]:
    captured = {item.get("path") for item in artifacts}
    return sorted(required - captured)


def performance_profile_matrix_is_complete(value: str | None) -> bool:
    required = {
        "endpoint": {"30"},
        "pbx-media-server": {"30", "100", "300", "1000", "2000"},
        "signaling-only-server-high-performance": {
            "30",
            "100",
            "300",
            "1000",
            "2000",
        },
    }
    captured: dict[str, set[str]] = {}
    for item in (value or "").split():
        if ":" not in item:
            continue
        profile, encoded_cps = item.split(":", 1)
        captured.setdefault(profile, set()).update(
            part for part in encoded_cps.split(",") if part
        )
    return all(expected.issubset(captured.get(profile, set())) for profile, expected in required.items())


def numeric_config(value: str | None) -> float | None:
    try:
        parsed = float(value) if value is not None else None
    except ValueError:
        return None
    if parsed is None or not math.isfinite(parsed):
        return None
    return parsed


def full_configuration_reasons(gate_config: dict[str, str]) -> list[str]:
    reasons: list[str] = []
    if not enabled_config(gate_config.get("rvoip_require_api_tools")):
        reasons.append(
            "full release evidence did not require pinned public API tools"
        )
    for key, (comparison, threshold) in FULL_NUMERIC_CONFIGURATION.items():
        actual = numeric_config(gate_config.get(key))
        valid = actual is not None
        if comparison == "minimum":
            valid = valid and actual >= threshold
        elif comparison == "maximum":
            valid = valid and 0 <= actual <= threshold
        else:
            valid = valid and actual == threshold
        if not valid:
            relation = {
                "minimum": f">= {threshold}",
                "maximum": f"between 0 and {threshold}",
                "exact": f"exactly {threshold}",
            }[comparison]
            reasons.append(
                f"full release configuration requires {key} {relation}; "
                f"found {gate_config.get(key)!r}"
            )
    return reasons


def required_gate_names(mode: str, gate_config: dict[str, str]) -> list[str]:
    required = {"beta final source fingerprint capture"}
    if mode in {"local", "full"}:
        required.update(LOCAL_REQUIRED_GATES)
    if mode in {"security", "full"}:
        required.update(SECURITY_REQUIRED_GATES)
    if mode in {"interop", "full"}:
        required.update(INTEROP_REQUIRED_GATES)
        if enabled_config(gate_config.get("beta_run_local_pbx")):
            provider = gate_config.get("beta_pbx_provider")
            if provider in {"all", "both", "ast", "asterisk"}:
                required.add("local Asterisk PBX matrix")
            if provider in {"all", "both", "fs", "free-switch", "freeswitch"}:
                required.add("local FreeSWITCH PBX matrix")
        else:
            required.add("PBX interop matrix")
    if mode in {"perf", "full"}:
        required.update(STANDARD_PERFORMANCE_REQUIRED_GATES)
        if enabled_config(gate_config.get("beta_run_perf_all")):
            required.update(LITERAL_ALL_PERFORMANCE_REQUIRED_GATES)
        if enabled_config(gate_config.get("beta_run_burst_matrix")):
            required.add("perf media burst matrix")
        else:
            required.add("perf media burst smoke")
    if mode == "full":
        required.update(
            {
                "clean beta source fingerprint",
                "canonical 2k three-pass evidence",
                "canonical 2k beta source unchanged",
            }
        )
    else:
        if enabled_config(gate_config.get("beta_require_clean_source")):
            required.update(
                {"clean beta source fingerprint", "beta source unchanged"}
            )
        if enabled_config(
            gate_config.get("beta_require_canonical_2k_evidence")
        ):
            required.update(
                {
                    "canonical 2k three-pass evidence",
                    "canonical 2k beta source unchanged",
                }
            )
    return sorted(required)


def gate_inventory_block(
    mode: str,
    gates: list[dict[str, Any]],
    gate_config: dict[str, str],
) -> dict[str, Any]:
    required = required_gate_names(mode, gate_config)
    captured = [gate["name"] for gate in gates]
    counts: dict[str, int] = {}
    for name in captured:
        counts[name] = counts.get(name, 0) + 1
    missing = sorted(set(required) - set(captured))
    duplicates = sorted(name for name, count in counts.items() if count != 1)
    return {
        "required": required,
        "captured": captured,
        "missing": missing,
        "duplicates": duplicates,
        "additional": sorted(set(captured) - set(required)),
        "complete": not missing and not duplicates,
    }


PERFORMANCE_RESULT_RECONCILIATION = {
    "perf-results/perf_session_churn_leak.json": (
        (
            "rvoip_perf_retention_drain_wait_secs",
            "results.retention_drain_wait_secs",
        ),
    ),
    "perf-results/perf_media_churn.json": (
        ("beta_perf_media_churn_duration_secs", "results.duration_secs"),
        ("beta_perf_media_churn_active_calls", "results.active_calls_target"),
        ("rvoip_perf_soak_min_hold_secs", "results.active_call_min_hold_secs"),
        ("rvoip_perf_soak_max_hold_secs", "results.active_call_max_hold_secs"),
        (
            "rvoip_perf_retention_drain_wait_secs",
            "results.retention_drain_wait_secs",
        ),
    ),
    "perf-results/perf_soak_30min.json": (
        ("beta_perf_monolithic_soak_duration_secs", "results.duration_secs"),
        ("beta_perf_monolithic_soak_active_calls", "results.active_calls_target"),
        ("rvoip_perf_soak_min_hold_secs", "results.active_call_min_hold_secs"),
        ("rvoip_perf_soak_max_hold_secs", "results.active_call_max_hold_secs"),
        ("rvoip_perf_soak_cps", "results.soak_cps"),
        ("rvoip_perf_soak_drain_cps", "results.controlled_drain_cps"),
        (
            "rvoip_perf_retention_drain_wait_secs",
            "results.retention_drain_wait_secs",
        ),
        (
            "rvoip_perf_max_rss_growth_mb_per_hr",
            "results.rss_gate.effective_mb_per_hr",
        ),
    ),
    "perf-results/perf_mass_teardown_stress.json": (
        ("rvoip_perf_mass_teardown_calls", "results.calls_requested"),
        (
            "rvoip_perf_mass_teardown_setup_cps",
            "results.mass_teardown_setup_cps",
        ),
        (
            "rvoip_perf_retention_drain_wait_secs",
            "results.retention_drain_wait_secs",
        ),
    ),
    "perf-results/perf_soak_caller.json": (
        ("rvoip_perf_soak_duration_secs", "results.duration_secs"),
        ("rvoip_perf_soak_active_calls", "results.active_calls_target"),
        ("rvoip_perf_soak_min_hold_secs", "results.active_call_min_hold_secs"),
        ("rvoip_perf_soak_max_hold_secs", "results.active_call_max_hold_secs"),
        ("rvoip_perf_soak_cps", "results.soak_cps"),
        (
            "rvoip_perf_retention_drain_wait_secs",
            "results.retention_drain_wait_secs",
        ),
        (
            "rvoip_perf_max_rss_growth_mb_per_hr",
            "results.rss_gate.effective_mb_per_hr",
        ),
    ),
    "perf-results/perf_soak_receiver.json": (
        ("rvoip_perf_soak_duration_secs", "results.configured_duration_secs"),
        ("rvoip_perf_soak_active_calls", "results.active_calls_target"),
        (
            "rvoip_perf_retention_drain_wait_secs",
            "results.retention_drain_wait_secs",
        ),
        (
            "rvoip_perf_max_rss_growth_mb_per_hr",
            "results.rss_gate.effective_mb_per_hr",
        ),
    ),
}


def dotted_value(value: Any, path: str) -> Any:
    current = value
    for part in path.split("."):
        if not isinstance(current, dict) or part not in current:
            return None
        current = current[part]
    return current


def same_numeric_value(left: Any, right: Any) -> bool:
    if isinstance(left, bool) or isinstance(right, bool):
        return False
    if not isinstance(left, (int, float)) or not isinstance(right, (int, float)):
        return False
    return math.isclose(float(left), float(right), rel_tol=0.0, abs_tol=1e-9)


def performance_result_reconciliation(
    root: pathlib.Path,
    fingerprint: str,
    gate_config: dict[str, str],
) -> list[dict[str, Any]]:
    reconciled: list[dict[str, Any]] = []
    for relative, specifications in PERFORMANCE_RESULT_RECONCILIATION.items():
        path = root / relative
        if not path.is_file():
            continue
        result = load_json(path, "performance result reconciliation evidence")
        current_source = fingerprint in source_fingerprints(result)
        checks: list[dict[str, Any]] = []
        for config_key, result_path in specifications:
            expected = numeric_config(gate_config.get(config_key))
            actual = dotted_value(result, result_path)
            checks.append(
                {
                    "config_key": config_key,
                    "result_path": result_path,
                    "expected": expected,
                    "actual": actual,
                    "matches": (
                        expected is not None
                        and current_source
                        and same_numeric_value(expected, actual)
                    ),
                }
            )
        reconciled.append(
            {
                "evidence_path": relative,
                "evidence_sha256": sha256_file(path),
                "current_source": current_source,
                "checks": checks,
                "matches": current_source
                and all(check["matches"] for check in checks),
            }
        )
    return reconciled


def append_missing_evidence_reason(
    reasons: list[str],
    artifacts: list[dict[str, Any]],
    required: set[str],
    label: str,
) -> None:
    missing = missing_paths(artifacts, required)
    if missing:
        reasons.append(f"{label} is incomplete; missing {', '.join(missing)}")


def append_missing_peer_reason(
    reasons: list[str], peers: list[dict[str, Any]], label: str
) -> None:
    captured = {peer.get("product") for peer in peers}
    missing = sorted(REQUIRED_INTEROP_PEER_PRODUCTS - captured)
    if missing:
        reasons.append(f"{label} is incomplete; missing {', '.join(missing)}")


def pointer_block(
    mode: str,
    clean: bool,
    unchanged: bool,
    failures: int,
    skips: int,
    executables: list[dict[str, Any]],
    peers: list[dict[str, Any]],
    results: dict[str, Any],
    gate_config: dict[str, str],
    artifacts: list[dict[str, Any]],
    gate_inventory: dict[str, Any],
    performance_baseline: dict[str, Any],
    result_reconciliation: list[dict[str, Any]],
) -> dict[str, Any]:
    reasons: list[str] = []
    if failures:
        reasons.append("one or more required gates failed")
    if skips:
        reasons.append("one or more gates were skipped")
    if not gate_inventory["complete"]:
        if gate_inventory["missing"]:
            reasons.append(
                "required gate inventory is incomplete; missing "
                + ", ".join(gate_inventory["missing"])
            )
        if gate_inventory["duplicates"]:
            reasons.append(
                "gate inventory contains duplicate names: "
                + ", ".join(gate_inventory["duplicates"])
            )
    if mode == "full":
        reasons.extend(full_configuration_reasons(gate_config))
        if not clean or not unchanged:
            reasons.append("full release evidence requires clean unchanged source")
        if not executables:
            reasons.append("full release evidence has no captured executable")
        if not peers:
            reasons.append(
                "full release evidence has no identified interoperability peer"
            )
        if results["performance_json_evidence"]["status"] != "captured":
            reasons.append("full release evidence has no performance result JSON")
        canonical = results["canonical_2k"]
        if canonical["status"] != "captured" or len(canonical["runs"]) != 3:
            reasons.append("full release evidence lacks three canonical 2K runs")
        if performance_baseline["status"] != "captured":
            reasons.append(
                "full release evidence lacks a reviewed performance regression baseline"
            )
        else:
            if (
                performance_baseline["baseline_id"]
                != PERFORMANCE_REGRESSION_BASELINE_ID
                or performance_baseline["manifest_sha256"]
                != PERFORMANCE_REGRESSION_BASELINE_SHA256
            ):
                reasons.append(
                    "full release evidence did not use the approved performance regression baseline"
                )
            if (
                gate_config.get("beta_perf_regression_baseline_id")
                != PERFORMANCE_REGRESSION_BASELINE_ID
            ):
                reasons.append(
                    "performance regression baseline id disagrees with the gate configuration"
                )
            if (
                gate_config.get("beta_perf_regression_baseline_manifest_sha256")
                != PERFORMANCE_REGRESSION_BASELINE_SHA256
            ):
                reasons.append(
                    "performance regression baseline hash disagrees with the gate configuration"
                )
        if not enabled_config(gate_config.get("beta_gate_require_external")):
            reasons.append(
                "full release evidence must fail closed on unavailable external gates"
            )
        if not enabled_config(gate_config.get("beta_run_perf_all")):
            reasons.append("full release evidence did not run literal-all performance gates")
        if not enabled_config(gate_config.get("beta_perf_regression_fail")):
            reasons.append("full release evidence did not hard-gate performance regressions")
        if not enabled_config(gate_config.get("beta_run_burst_matrix")) or gate_config.get(
            "beta_burst_matrix"
        ) != "all":
            reasons.append("full release evidence did not run the complete burst matrix")
        if not enabled_config(gate_config.get("beta_run_long_soak")):
            reasons.append("full release evidence did not run the split soak")
        if not (
            enabled_config(gate_config.get("beta_run_local_pbx"))
            or enabled_config(gate_config.get("beta_run_pbx"))
        ):
            reasons.append("full release evidence did not enable a PBX matrix")
        if gate_config.get("beta_pbx_provider") not in {"all", "both"}:
            reasons.append("full release evidence did not exercise both PBX products")
        if gate_config.get("beta_pbx_api") != "all":
            reasons.append("full release evidence did not exercise every PBX API")
        if gate_config.get("beta_pbx_scenario") != "all":
            reasons.append("full release evidence did not exercise every PBX scenario")
        g729_profiles = set((gate_config.get("beta_pbx_g729_profiles") or "").split())
        if not {"g729a", "g729ab"}.issubset(g729_profiles):
            reasons.append("full release evidence lacks G.729A/G.729AB PBX profiles")
        if not performance_profile_matrix_is_complete(
            gate_config.get("beta_profile_matrix")
        ):
            reasons.append("full release evidence lacks the complete performance profile matrix")
        append_missing_evidence_reason(
            reasons, artifacts, PBX_EVIDENCE_PATHS, "PBX report evidence"
        )
        append_missing_evidence_reason(
            reasons, artifacts, INTEROP_EVIDENCE_PATHS, "interop report evidence"
        )
        append_missing_peer_reason(reasons, peers, "interop peer identity evidence")
        append_missing_evidence_reason(
            reasons,
            artifacts,
            STANDARD_PERFORMANCE_RESULT_PATHS,
            "standard performance/soak result evidence",
        )
        append_missing_evidence_reason(
            reasons,
            artifacts,
            LITERAL_ALL_PERFORMANCE_RESULT_PATHS,
            "literal-all performance result evidence",
        )
        append_missing_evidence_reason(
            reasons,
            artifacts,
            PERFORMANCE_GATE_METRICS_PATHS,
            "performance gate metrics report",
        )
        if not any(
            item.get("path", "").startswith("perf-results/perf_burst_matrix/")
            and item.get("path", "").endswith(".json")
            for item in artifacts
        ):
            reasons.append("full release evidence lacks burst-matrix result JSON")
        for reconciliation in result_reconciliation:
            if reconciliation["matches"]:
                continue
            mismatches = [
                check["result_path"]
                for check in reconciliation["checks"]
                if not check["matches"]
            ]
            reasons.append(
                "performance result effective settings disagree with the gate: "
                f"{reconciliation['evidence_path']} ({', '.join(mismatches)})"
            )
    elif mode == "interop" and not peers:
        reasons.append("interop evidence has no identified peer")
    elif mode == "perf":
        if not executables:
            reasons.append("performance evidence has no captured executable")
        if results["performance_json_evidence"]["status"] != "captured":
            reasons.append("performance evidence has no performance result JSON")
        append_missing_evidence_reason(
            reasons,
            artifacts,
            STANDARD_PERFORMANCE_RESULT_PATHS,
            "performance/soak result evidence",
        )
        append_missing_evidence_reason(
            reasons,
            artifacts,
            PERFORMANCE_GATE_METRICS_PATHS,
            "performance gate metrics report",
        )
    if mode == "interop":
        append_missing_evidence_reason(
            reasons, artifacts, PBX_EVIDENCE_PATHS, "PBX report evidence"
        )
        append_missing_evidence_reason(
            reasons, artifacts, INTEROP_EVIDENCE_PATHS, "interop report evidence"
        )
        append_missing_peer_reason(reasons, peers, "interop peer identity evidence")
    return {
        "generic_informational_pointer": "latest.txt",
        "mode_specific_pointer": MODE_POINTERS[mode],
        "mode_specific_eligible": not reasons,
        "ineligibility_reasons": reasons,
    }


def qualification_block(mode: str, pointers: dict[str, Any]) -> dict[str, Any]:
    eligible = pointers["mode_specific_eligible"] is True
    release_candidate = mode == "full" and eligible
    if mode == "full":
        status = "RELEASE-CANDIDATE" if release_candidate else "NON-RC"
    else:
        status = "MODE-EVIDENCE" if eligible else "INCOMPLETE"
    return {
        "status": status,
        "release_candidate": release_candidate,
        "mode_evidence_eligible": eligible,
        "ineligibility_reasons": list(pointers["ineligibility_reasons"]),
    }


def structured_reporting_block(
    root: pathlib.Path, inputs: dict[str, dict[str, Any]]
) -> dict[str, Any]:
    config_path = relative_file(
        root, "effective-gate-config.json", "structured effective gate configuration"
    )
    results_path = relative_file(
        root, "gate-results.json", "structured gate results"
    )
    config = load_json(config_path, "structured effective gate configuration")
    results = load_json(results_path, "structured gate results")
    if config.get("schema") != STRUCTURED_CONFIG_SCHEMA:
        raise AttestationError("structured effective gate configuration schema is invalid")
    if results.get("schema") != STRUCTURED_RESULTS_SCHEMA:
        raise AttestationError("structured gate results schema is invalid")
    values = config.get("values")
    values_by_key = config.get("values_by_key")
    records = results.get("records")
    if (
        not isinstance(values, list)
        or not isinstance(values_by_key, dict)
        or len(values) != len(values_by_key)
    ):
        raise AttestationError("structured effective gate configuration is incomplete")
    if not isinstance(records, list) or not records:
        raise AttestationError("structured gate results are empty")
    ids = [item.get("id") for item in records if isinstance(item, dict)]
    sequences = [item.get("sequence") for item in records if isinstance(item, dict)]
    if (
        len(ids) != len(records)
        or len(ids) != len(set(ids))
        or sequences != list(range(1, len(records) + 1))
    ):
        raise AttestationError("structured gate results have duplicate or non-contiguous identities")
    expected_counts = {
        "required_count": len(records),
        "passed": sum(item.get("status") == "PASS" for item in records),
        "failed": sum(item.get("status") == "FAIL" for item in records),
        "skipped": sum(item.get("status") == "SKIP" for item in records),
    }
    for key, expected in expected_counts.items():
        if results.get(key) != expected:
            raise AttestationError(f"structured gate result count {key} is inconsistent")
    policy = inputs.get("beta-release-policy")
    generator = inputs.get("beta-release-report-generator")
    if not policy or not generator:
        raise AttestationError(
            "v2 attestation requires beta-release-policy and "
            "beta-release-report-generator inputs"
        )
    return {
        "binding_mode": "native-v2-input",
        "effective_gate_config": {
            "path": config_path.relative_to(root).as_posix(),
            "sha256": sha256_file(config_path),
            "schema": config["schema"],
            "value_count": len(values),
        },
        "gate_results": {
            "path": results_path.relative_to(root).as_posix(),
            "sha256": sha256_file(results_path),
            "schema": results["schema"],
            "mode": results.get("mode"),
            "ordered_gate_ids_sha256": canonical_json_sha256(ids),
            "ordered_gate_names_sha256": canonical_json_sha256(
                [item.get("name") for item in records]
            ),
            **expected_counts,
        },
        "policy_catalog": {
            "path": policy["path"],
            "sha256": policy["sha256"],
        },
        "report_generator": {
            "path": generator["path"],
            "sha256": generator["sha256"],
        },
    }


def reconcile_structured_reporting(
    structured: dict[str, Any],
    gates: list[dict[str, Any]],
    mode: str,
    failures: int,
    skips: int,
) -> None:
    gate_results = structured["gate_results"]
    names = [gate["name"] for gate in gates]
    if gate_results.get("mode") != mode:
        raise AttestationError("structured gate result mode disagrees with the run")
    if gate_results.get("ordered_gate_names_sha256") != canonical_json_sha256(names):
        raise AttestationError("structured gate result order disagrees with summary gates")
    if gate_results.get("required_count") != len(gates):
        raise AttestationError("structured gate result count disagrees with summary gates")
    if gate_results.get("failed") != failures or gate_results.get("skipped") != skips:
        raise AttestationError("structured gate result totals disagree with run totals")


def create_attestation(args: argparse.Namespace) -> pathlib.Path:
    root = pathlib.Path(args.report_root).expanduser().resolve()
    if not root.is_dir():
        raise AttestationError(f"report root does not exist: {root}")
    if args.mode not in VALID_MODES:
        raise AttestationError(f"invalid beta mode: {args.mode}")
    start_path = relative_file(root, args.source_start, "source-at-start")
    end_path = relative_file(root, args.source_end, "source-at-end")
    start = source_block(root, start_path)
    end = source_block(root, end_path)
    unchanged = start["source_fingerprint_sha256"] == end["source_fingerprint_sha256"]
    clean = start["git_dirty"] is False and end["git_dirty"] is False and unchanged

    cargo_metadata_path = relative_file(root, args.cargo_metadata, "Cargo metadata")
    package_name, package_version = cargo_package(cargo_metadata_path)
    cargo_metadata = load_json(cargo_metadata_path, "Cargo metadata")
    raw_target_directory = cargo_metadata.get("target_directory")
    target_directory = (
        pathlib.Path(raw_target_directory)
        if isinstance(raw_target_directory, str) and raw_target_directory
        else None
    )
    inputs = copy_inputs(root, args.input, args.mode)
    state_table = state_table_block(
        inputs,
        args.state_table_source,
        args.state_table_fallback_reason,
        args.state_table_sha256,
    )
    performance_baseline = performance_regression_baseline_block(
        root, inputs, args.mode
    )
    executables, ignored_executables = copy_executables(
        root, start["source_fingerprint_sha256"], target_directory
    )
    peers = peers_block(root, inputs)
    gates = parse_gates(root)
    artifacts = artifact_inventory(root)
    failures = int(args.failures)
    skips = int(args.skips)
    if failures < 0 or skips < 0:
        raise AttestationError("failure and skip counts must be nonnegative")
    gate_config = effective_gate_config(root)
    count_details = reconcile_result_counts(root, gates, gate_config, failures, skips)
    expected_state_config = {
        "beta_state_table_source": state_table["selected_source"],
        "beta_state_table_fallback_reason": state_table["fallback_reason"] or "none",
        "beta_state_table_sha256": state_table["selected_yaml_sha256"],
    }
    for key, expected in expected_state_config.items():
        if gate_config.get(key) != expected:
            raise AttestationError(
                f"selected state-table evidence disagrees with {key} in summary.md"
            )
    expected_overall = "PASS" if failures == 0 else "FAIL"
    if args.overall != expected_overall:
        raise AttestationError(
            f"overall status {args.overall} disagrees with failures={failures}"
        )
    started = parse_timestamp(args.started_at, "run.started_at_utc")
    ended = parse_timestamp(args.ended_at, "run.ended_at_utc")
    if ended < started:
        raise AttestationError("run end timestamp precedes start timestamp")
    features = sorted(
        set(part.strip() for part in args.features.split(",") if part.strip())
    )
    toolchain = compiler_block(root, getattr(args, "target", None))
    reconcile_build_config(gate_config, features, toolchain)
    results = result_block(root, artifacts, start["source_fingerprint_sha256"])
    gate_inventory = gate_inventory_block(args.mode, gates, gate_config)
    result_reconciliation = performance_result_reconciliation(
        root, start["source_fingerprint_sha256"], gate_config
    )
    configuration = effective_config_block(
        root,
        start["source_fingerprint_sha256"],
        gate_config,
        result_reconciliation,
    )
    pointers = pointer_block(
        args.mode,
        clean,
        unchanged,
        failures,
        skips,
        executables,
        peers,
        results,
        gate_config,
        artifacts,
        gate_inventory,
        performance_baseline,
        result_reconciliation,
    )

    schema = SCHEMA_V2 if getattr(args, "schema_version", 1) == 2 else SCHEMA_V1
    manifest: dict[str, Any] = {
        "schema": schema,
        "created_at_utc": utc_now(),
        "run": {
            "id": args.run_id,
            "mode": args.mode,
            "started_at_utc": args.started_at,
            "ended_at_utc": args.ended_at,
            "duration_seconds": int((ended - started).total_seconds()),
        },
        "package": {
            "name": package_name,
            "version": package_version,
            "cargo_metadata_path": cargo_metadata_path.relative_to(root).as_posix(),
            "cargo_metadata_sha256": sha256_file(cargo_metadata_path),
        },
        "source": {"start": start, "end": end, "unchanged": unchanged, "clean": clean},
        "build": {
            "toolchain": toolchain,
            "features": features,
            "features_source": "beta-gate-resolved",
            "executables": executables,
            "executable_evidence": evidence_status(
                len(executables),
                "no claim-bearing executable was available to package for this mode",
            ),
            "ignored_different_source_executables": ignored_executables,
        },
        "inputs": inputs,
        "state_table": state_table,
        "performance_inputs": performance_inputs_block(inputs),
        "performance_regression_baseline": performance_baseline,
        "configuration": configuration,
        "peers": peers,
        "peer_evidence": evidence_status(
            len(peers), "no external peer was exercised or identified for this mode"
        ),
        "gates": gates,
        "gate_inventory": gate_inventory,
        "results": results,
        "result": {
            "failures": failures,
            "skips": skips,
            "overall": args.overall,
            **count_details,
        },
        "pointers": pointers,
        "qualification": qualification_block(args.mode, pointers),
        "artifacts": {
            "hash_algorithm": HASH_ALGORITHM,
            "count": len(artifacts),
            "files": artifacts,
        },
        "verification": {
            "command": "python3 inputs/attestation-verifier.py verify --report-root .",
            "verifier_path": inputs.get("attestation-verifier", {}).get("path"),
            "verifier_sha256": inputs.get("attestation-verifier", {}).get("sha256"),
            "workspace_reads_required": False,
            "referenced_paths": "report-relative",
        },
        "assurance": {
            "kind": "integrity-and-reproducibility",
            "cryptographically_signed": False,
            "note": "SHA-256 binds copied evidence but does not provide third-party authenticity.",
        },
    }
    if schema == SCHEMA_V2:
        manifest["structured_reporting"] = structured_reporting_block(root, inputs)
        reconcile_structured_reporting(
            manifest["structured_reporting"], gates, args.mode, failures, skips
        )
    validate_structure(manifest)
    output = root / ATTESTATION_NAME
    temporary = root / f".{ATTESTATION_NAME}.tmp"
    temporary.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.replace(temporary, output)
    digest = sha256_file(output)
    checksum = root / CHECKSUM_NAME
    checksum_tmp = root / f".{CHECKSUM_NAME}.tmp"
    checksum_tmp.write_text(f"{digest}  {ATTESTATION_NAME}\n", encoding="ascii")
    os.replace(checksum_tmp, checksum)
    verify_report(root)
    return output


def require_hash(value: Any, label: str) -> None:
    if not isinstance(value, str) or HEX_64.fullmatch(value) is None:
        raise AttestationError(f"{label} must be a lowercase SHA-256 digest")


def validate_structure(manifest: dict[str, Any]) -> None:
    if manifest.get("schema") not in {SCHEMA_V1, SCHEMA_V2}:
        raise AttestationError(
            f"unsupported attestation schema: {manifest.get('schema')!r}"
        )
    run = manifest.get("run")
    if not isinstance(run, dict) or run.get("mode") not in VALID_MODES:
        raise AttestationError("attestation run mode is invalid")
    if (
        not isinstance(run.get("id"), str)
        or re.fullmatch(r"\d{8}T\d{6}Z", run["id"]) is None
    ):
        raise AttestationError("attestation run ID must be a UTC beta timestamp")
    created = parse_timestamp(manifest.get("created_at_utc"), "created_at_utc")
    started = parse_timestamp(run.get("started_at_utc"), "run.started_at_utc")
    ended = parse_timestamp(run.get("ended_at_utc"), "run.ended_at_utc")
    if ended < started:
        raise AttestationError("run end timestamp precedes start timestamp")
    if created < ended:
        raise AttestationError("attestation creation timestamp precedes run end")
    expected_duration = int((ended - started).total_seconds())
    if run.get("duration_seconds") != expected_duration:
        raise AttestationError("run duration disagrees with start/end timestamps")
    package = manifest.get("package")
    if not isinstance(package, dict) or package.get("name") != "rvoip-sip":
        raise AttestationError("attestation package must be rvoip-sip")
    if (
        not isinstance(package.get("version"), str)
        or SEMVER.fullmatch(package["version"]) is None
    ):
        raise AttestationError("attestation package version is invalid")
    if not package.get("cargo_metadata_path"):
        raise AttestationError("Cargo metadata evidence path is missing")
    require_hash(package.get("cargo_metadata_sha256"), "Cargo metadata hash")
    source = manifest.get("source")
    if not isinstance(source, dict):
        raise AttestationError("attestation source section is missing")
    for phase in ("start", "end"):
        block = source.get(phase)
        if not isinstance(block, dict):
            raise AttestationError(f"source.{phase} is missing")
        if (
            not isinstance(block.get("git_commit"), str)
            or HEX_40.fullmatch(block["git_commit"]) is None
        ):
            raise AttestationError(f"source.{phase}.git_commit is invalid")
        if (
            not isinstance(block.get("git_tree"), str)
            or HEX_40.fullmatch(block["git_tree"]) is None
        ):
            raise AttestationError(f"source.{phase}.git_tree is invalid")
        require_hash(
            block.get("source_fingerprint_sha256"),
            f"source.{phase}.source_fingerprint_sha256",
        )
        require_hash(block.get("evidence_sha256"), f"source.{phase}.evidence_sha256")
        relative_path(block.get("evidence_path"), f"source.{phase}.evidence_path")
        if not isinstance(block.get("git_dirty"), bool):
            raise AttestationError(f"source.{phase}.git_dirty must be boolean")
    expected_unchanged = (
        source["start"]["source_fingerprint_sha256"]
        == source["end"]["source_fingerprint_sha256"]
    )
    expected_clean = (
        source["start"]["git_dirty"] is False
        and source["end"]["git_dirty"] is False
        and expected_unchanged
    )
    if (
        source.get("unchanged") is not expected_unchanged
        or source.get("clean") is not expected_clean
    ):
        raise AttestationError("source cleanliness/unchanged flags are inconsistent")
    build = manifest.get("build")
    if not isinstance(build, dict) or not isinstance(build.get("features"), list):
        raise AttestationError("build feature evidence is missing")
    if build.get("features_source") != "beta-gate-resolved":
        raise AttestationError("build feature source is invalid")
    if build["features"] != sorted(set(build["features"])) or any(
        not isinstance(feature, str) or not feature for feature in build["features"]
    ):
        raise AttestationError("build features must be unique sorted nonempty strings")
    toolchain = build.get("toolchain")
    if not isinstance(toolchain, dict) or toolchain.get("target") in (
        None,
        "",
        "unknown",
    ):
        raise AttestationError("build target evidence is missing")
    if toolchain.get("target_source") not in {"explicit", "rustc-host"}:
        raise AttestationError("build target source is invalid")
    for name in ("rustc", "cargo"):
        if toolchain.get(name) in (None, "", "unknown"):
            raise AttestationError(f"{name} version evidence is missing")
        relative_path(toolchain.get(f"{name}_evidence_path"), f"{name} evidence path")
        require_hash(toolchain.get(f"{name}_evidence_sha256"), f"{name} evidence hash")
    executables = build.get("executables")
    if not isinstance(executables, list):
        raise AttestationError("build executable list is missing")
    for executable in executables:
        if not isinstance(executable, dict):
            raise AttestationError("executable evidence entry must be an object")
        require_hash(executable.get("sha256"), "build executable hash")
        relative_path(executable.get("path"), "build executable path")
        if not isinstance(executable.get("bytes"), int) or executable["bytes"] < 0:
            raise AttestationError("build executable byte count is invalid")
    executable_evidence = build.get("executable_evidence")
    expected_executable_status = "captured" if executables else "absent"
    if (
        not isinstance(executable_evidence, dict)
        or executable_evidence.get("status") != expected_executable_status
        or executable_evidence.get("count") != len(executables)
        or (executables and executable_evidence.get("absent_reason") is not None)
        or (not executables and not executable_evidence.get("absent_reason"))
    ):
        raise AttestationError("build executable evidence status is inconsistent")
    inputs = manifest.get("inputs")
    if not isinstance(inputs, dict) or not required_inputs_for_mode(
        run["mode"]
    ).issubset(inputs):
        raise AttestationError("required input evidence is missing")
    for name, entry in inputs.items():
        if not isinstance(entry, dict):
            raise AttestationError(f"input {name} is invalid")
        relative_path(entry.get("path"), f"input {name} path")
        require_hash(entry.get("sha256"), f"input {name} hash")
        if not isinstance(entry.get("bytes"), int) or entry["bytes"] < 0:
            raise AttestationError(f"input {name} byte count is invalid")
    state_table = manifest.get("state_table")
    if not isinstance(state_table, dict):
        raise AttestationError("selected state-table evidence is missing")
    selected_source = state_table.get("selected_source")
    if selected_source not in VALID_STATE_TABLE_SOURCES:
        raise AttestationError("selected state-table source is invalid")
    fallback_reason = state_table.get("fallback_reason")
    if selected_source == "configured-path-fallback":
        if fallback_reason not in VALID_STATE_TABLE_FALLBACK_REASONS:
            raise AttestationError("selected state-table fallback reason is invalid")
    elif fallback_reason is not None:
        raise AttestationError("unexpected selected state-table fallback reason")
    require_hash(
        state_table.get("selected_yaml_sha256"),
        "selected state-table YAML hash",
    )
    selected_input = inputs["state-machine-yaml"]
    expected_state_table = {
        "selected_yaml_path": selected_input.get("path"),
        "selected_yaml_source_name": selected_input.get("source_name"),
        "selected_yaml_sha256": selected_input.get("sha256"),
        "selected_yaml_bytes": selected_input.get("bytes"),
    }
    for key, expected in expected_state_table.items():
        if state_table.get(key) != expected:
            raise AttestationError(
                f"selected state-table {key} disagrees with copied YAML input"
            )
    performance_inputs = manifest.get("performance_inputs")
    if not isinstance(performance_inputs, dict):
        raise AttestationError("performance input evidence is missing")
    expected_performance_inputs = performance_inputs_block(inputs)
    if performance_inputs != expected_performance_inputs:
        raise AttestationError(
            "performance recipe/burst evidence disagrees with inputs"
        )
    performance_baseline = manifest.get("performance_regression_baseline")
    if not isinstance(performance_baseline, dict) or performance_baseline.get(
        "status"
    ) not in {"captured", "absent"}:
        raise AttestationError("performance regression baseline evidence is invalid")
    if run["mode"] == "full" and performance_baseline["status"] != "captured":
        raise AttestationError(
            "full release attestation requires a performance regression baseline"
        )
    if performance_baseline["status"] == "captured":
        for key in ("input_path", "manifest_path"):
            relative_path(
                performance_baseline.get(key),
                f"performance regression baseline {key}",
            )
        for key in ("input_sha256", "manifest_sha256"):
            require_hash(
                performance_baseline.get(key),
                f"performance regression baseline {key}",
            )
        if not performance_baseline.get("baseline_id"):
            raise AttestationError("performance regression baseline id is missing")
        if not isinstance(performance_baseline.get("files"), list) or not performance_baseline[
            "files"
        ]:
            raise AttestationError("performance regression baseline files are missing")
        for item in performance_baseline["files"]:
            if not isinstance(item, dict):
                raise AttestationError(
                    "performance regression baseline file evidence is invalid"
                )
            relative_path(item.get("baseline_path"), "baseline-relative result path")
            relative_path(item.get("path"), "packaged baseline result path")
            require_hash(item.get("sha256"), "packaged baseline result hash")
            if not isinstance(item.get("bytes"), int) or item["bytes"] < 0:
                raise AttestationError(
                    "packaged performance regression baseline byte count is invalid"
                )
    elif (
        not performance_baseline.get("absent_reason")
        or performance_baseline.get("files") != []
        or performance_baseline.get("comparison_paths") != []
    ):
        raise AttestationError(
            "absent performance regression baseline evidence is inconsistent"
        )
    verification = manifest.get("verification")
    if not isinstance(verification, dict) or not verification.get("verifier_path"):
        raise AttestationError("standalone verifier evidence is missing")
    relative_path(verification.get("verifier_path"), "standalone verifier path")
    require_hash(verification.get("verifier_sha256"), "standalone verifier hash")
    if (
        verification.get("workspace_reads_required") is not False
        or verification.get("referenced_paths") != "report-relative"
    ):
        raise AttestationError("standalone verification portability claim is invalid")
    verifier_input = inputs["attestation-verifier"]
    if (
        verification["verifier_path"] != verifier_input["path"]
        or verification["verifier_sha256"] != verifier_input["sha256"]
    ):
        raise AttestationError(
            "standalone verifier evidence disagrees with copied input"
        )
    configuration = manifest.get("configuration")
    if not isinstance(configuration, dict):
        raise AttestationError("configuration evidence is missing")
    require_hash(configuration.get("redacted_environment_sha256"), "configuration hash")
    require_hash(
        configuration.get("effective_gate_config_sha256"),
        "effective gate configuration hash",
    )
    require_hash(
        configuration.get("effective_redacted_configuration_sha256"),
        "combined effective redacted configuration hash",
    )
    if configuration.get("effective_gate_config_source_path") != "summary.md":
        raise AttestationError("effective gate configuration source must be summary.md")
    recorded_gate_config = configuration.get("effective_gate_config")
    if (
        not isinstance(recorded_gate_config, dict)
        or any(
            not isinstance(key, str) or not isinstance(value, str)
            for key, value in recorded_gate_config.items()
        )
        or canonical_json_sha256(recorded_gate_config)
        != configuration["effective_gate_config_sha256"]
        or sorted(recorded_gate_config)
        != configuration.get("effective_gate_config_keys")
    ):
        raise AttestationError("recorded effective gate configuration is inconsistent")
    if not configuration.get("redacted_environment_path"):
        raise AttestationError("redacted configuration path is missing")
    relative_path(
        configuration.get("redacted_environment_path"),
        "redacted configuration path",
    )
    runtime_configs = configuration.get("runtime_effective_configs")
    if not isinstance(runtime_configs, list):
        raise AttestationError("runtime effective configuration evidence is missing")
    for runtime_config in runtime_configs:
        if not isinstance(runtime_config, dict):
            raise AttestationError("runtime effective configuration entry is invalid")
        relative_path(
            runtime_config.get("evidence_path"),
            "runtime effective configuration evidence path",
        )
        require_hash(
            runtime_config.get("effective_config_sha256"),
            "runtime effective configuration hash",
        )
    result_reconciliation = configuration.get("performance_result_reconciliation")
    if not isinstance(result_reconciliation, list):
        raise AttestationError("performance result configuration reconciliation is missing")
    for reconciliation in result_reconciliation:
        if (
            not isinstance(reconciliation, dict)
            or not isinstance(reconciliation.get("current_source"), bool)
            or not isinstance(reconciliation.get("matches"), bool)
            or not isinstance(reconciliation.get("checks"), list)
        ):
            raise AttestationError(
                "performance result configuration reconciliation is invalid"
            )
        relative_path(
            reconciliation.get("evidence_path"),
            "performance result reconciliation evidence path",
        )
        require_hash(
            reconciliation.get("evidence_sha256"),
            "performance result reconciliation evidence hash",
        )
        expected_matches = reconciliation["current_source"]
        for check in reconciliation["checks"]:
            if (
                not isinstance(check, dict)
                or not isinstance(check.get("config_key"), str)
                or not isinstance(check.get("result_path"), str)
                or not isinstance(check.get("matches"), bool)
            ):
                raise AttestationError(
                    "performance result reconciliation check is invalid"
                )
            expected_matches = expected_matches and check["matches"]
        if reconciliation["matches"] is not expected_matches:
            raise AttestationError(
                "performance result reconciliation aggregate is inconsistent"
            )
    expected_combined_config_hash = canonical_json_sha256(
        {
            "effective_gate_config_sha256": configuration[
                "effective_gate_config_sha256"
            ],
            "redacted_environment_sha256": configuration["redacted_environment_sha256"],
            "runtime_effective_configs": runtime_configs,
            "performance_result_reconciliation": result_reconciliation,
        }
    )
    if (
        configuration["effective_redacted_configuration_sha256"]
        != expected_combined_config_hash
    ):
        raise AttestationError(
            "combined effective redacted configuration hash is inconsistent"
        )
    peers = manifest.get("peers")
    if not isinstance(peers, list):
        raise AttestationError("peer evidence must be a list")
    for peer in peers:
        if not isinstance(peer, dict) or not peer.get("product"):
            raise AttestationError("peer product is missing")
        if not peer.get("version") and not peer.get("image_digest"):
            raise AttestationError(
                f"peer version/image digest is missing for {peer.get('product')}"
            )
        require_hash(
            peer.get("config_sha256"), f"peer config hash for {peer.get('product')}"
        )
        evidence_paths = peer.get("evidence_paths")
        if not isinstance(evidence_paths, list) or not evidence_paths:
            raise AttestationError(
                f"peer evidence paths are missing for {peer.get('product')}"
            )
        for path in evidence_paths:
            relative_path(path, f"peer evidence path for {peer.get('product')}")
    peer_evidence = manifest.get("peer_evidence")
    expected_peer_status = "captured" if peers else "absent"
    if (
        not isinstance(peer_evidence, dict)
        or peer_evidence.get("status") != expected_peer_status
        or peer_evidence.get("count") != len(peers)
        or (peers and peer_evidence.get("absent_reason") is not None)
        or (not peers and not peer_evidence.get("absent_reason"))
    ):
        raise AttestationError("peer evidence status is inconsistent")
    gates = manifest.get("gates")
    if not isinstance(gates, list) or not gates:
        raise AttestationError("gate evidence is missing")
    for gate in gates:
        if not isinstance(gate, dict) or gate.get("status") not in VALID_GATE_STATUSES:
            raise AttestationError("gate status is invalid")
        if not isinstance(gate.get("name"), str) or not gate["name"]:
            raise AttestationError("gate name is missing")
        if (
            not isinstance(gate.get("duration_seconds"), int)
            or gate["duration_seconds"] < 0
            or gate.get("duration") != f"{gate['duration_seconds']}s"
        ):
            raise AttestationError(f"gate duration is invalid for {gate.get('name')}")
        relative_path(gate.get("log_path"), f"gate log path for {gate.get('name')}")
        require_hash(gate.get("log_sha256"), f"gate log hash for {gate.get('name')}")
    expected_gate_inventory = gate_inventory_block(
        run["mode"], gates, recorded_gate_config
    )
    if manifest.get("gate_inventory") != expected_gate_inventory:
        raise AttestationError("required gate inventory is inconsistent")
    result = manifest.get("result")
    if not isinstance(result, dict) or result.get("overall") not in {"PASS", "FAIL"}:
        raise AttestationError("result section is invalid")
    for key in (
        "failures",
        "skips",
        "failed_gates",
        "skipped_gates",
        "required_skips_counted_as_failures",
    ):
        if not isinstance(result.get(key), int) or result[key] < 0:
            raise AttestationError(f"result.{key} must be a nonnegative integer")
    expected_overall = "PASS" if result.get("failures") == 0 else "FAIL"
    if result.get("overall") != expected_overall:
        raise AttestationError("overall result disagrees with failure count")
    gate_failures = sum(gate["status"] == "FAIL" for gate in gates)
    gate_skips = sum(gate["status"] == "SKIP" for gate in gates)
    if (
        result["failed_gates"] != gate_failures
        or result["skipped_gates"] != gate_skips
        or result["skips"] != gate_skips
        or result["failures"]
        != gate_failures + result["required_skips_counted_as_failures"]
        or result["required_skips_counted_as_failures"] not in {0, gate_skips}
    ):
        raise AttestationError("result counts disagree with gate statuses")
    results = manifest.get("results")
    if not isinstance(results, dict) or not isinstance(results.get("json"), list):
        raise AttestationError("result JSON evidence is missing")
    for item in results["json"]:
        if not isinstance(item, dict) or not item.get("path"):
            raise AttestationError("result JSON entry is invalid")
        relative_path(item.get("path"), "result JSON path")
        require_hash(item.get("sha256"), f"result JSON hash for {item.get('path')}")
    json_evidence = results.get("json_evidence")
    expected_json_status = "captured" if results["json"] else "absent"
    if (
        not isinstance(json_evidence, dict)
        or json_evidence.get("status") != expected_json_status
        or json_evidence.get("count") != len(results["json"])
        or (results["json"] and json_evidence.get("absent_reason") is not None)
        or (not results["json"] and not json_evidence.get("absent_reason"))
    ):
        raise AttestationError("result JSON evidence status is inconsistent")
    performance_json_count = sum(
        item["path"].startswith("perf-results/") for item in results["json"]
    )
    performance_json_evidence = results.get("performance_json_evidence")
    expected_performance_json_status = (
        "captured" if performance_json_count else "absent"
    )
    if (
        not isinstance(performance_json_evidence, dict)
        or performance_json_evidence.get("status") != expected_performance_json_status
        or performance_json_evidence.get("count") != performance_json_count
        or (
            performance_json_count
            and performance_json_evidence.get("absent_reason") is not None
        )
        or (
            not performance_json_count
            and not performance_json_evidence.get("absent_reason")
        )
    ):
        raise AttestationError(
            "performance result JSON evidence status is inconsistent"
        )
    canonical = results.get("canonical_2k")
    if not isinstance(canonical, dict) or canonical.get("status") not in {
        "captured",
        "absent",
    }:
        raise AttestationError("canonical 2K evidence status is invalid")
    if canonical["status"] == "absent":
        if (
            not canonical.get("absent_reason")
            or canonical.get("index_path") is not None
            or canonical.get("index_sha256") is not None
            or canonical.get("reviewed_baseline") is not None
            or canonical.get("runs") != []
        ):
            raise AttestationError("absent canonical 2K evidence is inconsistent")
    else:
        if canonical.get("absent_reason") is not None:
            raise AttestationError(
                "captured canonical 2K evidence has an absent reason"
            )
        relative_path(canonical.get("index_path"), "canonical 2K index path")
        require_hash(canonical.get("index_sha256"), "canonical 2K index hash")
        require_hash(
            canonical.get("common_executable_sha256"),
            "canonical 2K common executable hash",
        )
        reviewed_baseline = canonical.get("reviewed_baseline")
        expected_reviewed_baseline = {
            "id": CANONICAL_2K_BASELINE_ID,
            "relative_path": CANONICAL_2K_BASELINE_RELATIVE_PATH,
            "sha256": CANONICAL_2K_BASELINE_SHA256,
            "path": f"canonical-2k/{CANONICAL_2K_BASELINE_PACKAGED_PATH}",
        }
        if reviewed_baseline != expected_reviewed_baseline:
            raise AttestationError(
                "captured canonical 2K reviewed baseline identity is invalid"
            )
        relative_path(
            reviewed_baseline["path"], "canonical 2K reviewed baseline path"
        )
        require_hash(
            reviewed_baseline["sha256"], "canonical 2K reviewed baseline hash"
        )
        runs = canonical.get("runs")
        if not isinstance(runs, list) or len(runs) != CANONICAL_2K_RUN_COUNT:
            raise AttestationError(
                "captured canonical 2K evidence must contain three runs"
            )
        for sequence, run_item in enumerate(runs, start=1):
            if not isinstance(run_item, dict) or run_item.get("sequence") != sequence:
                raise AttestationError("canonical 2K run sequence is invalid")
            relative_path(run_item.get("path"), "canonical 2K run path")
            require_hash(run_item.get("tree_sha256"), "canonical 2K run tree hash")
            require_hash(
                run_item.get("source_fingerprint_sha256"),
                "canonical 2K run source fingerprint",
            )
            require_hash(
                run_item.get("executable_sha256"),
                "canonical 2K run executable hash",
            )
            if run_item.get("reviewed_baseline_sha256") != CANONICAL_2K_BASELINE_SHA256:
                raise AttestationError(
                    "canonical 2K run reviewed baseline hash is invalid"
                )
            if run_item.get("reviewed_baseline_origin") not in CANONICAL_2K_BASELINE_ORIGINS:
                raise AttestationError(
                    "canonical 2K run reviewed baseline origin is invalid"
                )
            parse_timestamp(
                run_item.get("captured_at_utc"),
                "canonical 2K run captured timestamp",
            )
    pointers = manifest.get("pointers")
    mode = run["mode"]
    if (
        not isinstance(pointers, dict)
        or pointers.get("generic_informational_pointer") != "latest.txt"
        or pointers.get("mode_specific_pointer") != MODE_POINTERS[mode]
        or not isinstance(pointers.get("mode_specific_eligible"), bool)
        or not isinstance(pointers.get("ineligibility_reasons"), list)
        or pointers["mode_specific_eligible"] != (not pointers["ineligibility_reasons"])
    ):
        raise AttestationError("mode-specific pointer evidence is inconsistent")
    if manifest.get("qualification") != qualification_block(mode, pointers):
        raise AttestationError("release qualification evidence is inconsistent")
    if manifest["schema"] == SCHEMA_V2:
        structured = manifest.get("structured_reporting")
        if not isinstance(structured, dict) or structured.get("binding_mode") != "native-v2-input":
            raise AttestationError("v2 structured reporting binding is missing")
        for name in (
            "effective_gate_config",
            "gate_results",
            "policy_catalog",
            "report_generator",
        ):
            block = structured.get(name)
            if not isinstance(block, dict):
                raise AttestationError(f"v2 structured reporting block {name} is missing")
            relative_path(block.get("path"), f"v2 {name} path")
            require_hash(block.get("sha256"), f"v2 {name} hash")
    elif "structured_reporting" in manifest:
        raise AttestationError("v1 attestation must not contain a v2 structured binding")
    artifacts = manifest.get("artifacts")
    if (
        not isinstance(artifacts, dict)
        or artifacts.get("hash_algorithm") != HASH_ALGORITHM
    ):
        raise AttestationError("artifact inventory is invalid")
    files = artifacts.get("files")
    if not isinstance(files, list) or artifacts.get("count") != len(files):
        raise AttestationError("artifact count is inconsistent")
    seen_paths: set[str] = set()
    for item in files:
        if not isinstance(item, dict) or not item.get("path"):
            raise AttestationError("artifact entry is invalid")
        if item["path"] in seen_paths:
            raise AttestationError(f"duplicate artifact path: {item['path']}")
        seen_paths.add(item["path"])
        relative_path(item["path"], "artifact path")
        require_hash(item.get("sha256"), f"artifact hash for {item['path']}")
        if not isinstance(item.get("bytes"), int) or item["bytes"] < 0:
            raise AttestationError(f"artifact byte count is invalid for {item['path']}")

    packaged_executable_hashes = {
        item["sha256"] for item in files if item.get("kind") == "executable"
    }
    referenced_executable_hashes = {item["sha256"] for item in executables}
    if packaged_executable_hashes != referenced_executable_hashes:
        raise AttestationError(
            "packaged executable inventory disagrees with build executable evidence"
        )


def verify_report(
    report_root: str | pathlib.Path,
    *,
    require_clean: bool = False,
    require_unchanged_source: bool = False,
    require_no_skips: bool = False,
    require_pass: bool = False,
    require_mode_eligible: bool = False,
) -> dict[str, Any]:
    root = pathlib.Path(report_root).expanduser().resolve()
    attestation_path = root / ATTESTATION_NAME
    checksum_path = root / CHECKSUM_NAME
    manifest = load_json(attestation_path, "attestation")
    validate_structure(manifest)
    try:
        checksum_tokens = checksum_path.read_text(encoding="ascii").strip().split()
    except OSError as error:
        raise AttestationError(f"cannot read attestation checksum: {error}") from error
    if checksum_tokens != [sha256_file(attestation_path), ATTESTATION_NAME]:
        raise AttestationError("attestation checksum does not match attestation.json")

    expected_files = manifest["artifacts"]["files"]
    actual_files = artifact_inventory(root)
    expected_by_path = {item["path"]: item for item in expected_files}
    actual_by_path = {item["path"]: item for item in actual_files}
    missing = sorted(set(expected_by_path) - set(actual_by_path))
    added = sorted(set(actual_by_path) - set(expected_by_path))
    if missing:
        raise AttestationError(f"attested artifacts are missing: {missing}")
    if added:
        raise AttestationError(f"unattested artifacts were added: {added}")
    for relative, expected in expected_by_path.items():
        actual = actual_by_path[relative]
        if (
            expected["sha256"] != actual["sha256"]
            or expected["bytes"] != actual["bytes"]
        ):
            raise AttestationError(f"artifact changed after attestation: {relative}")

    for phase in ("start", "end"):
        block = manifest["source"][phase]
        path = relative_file(root, block["evidence_path"], f"source {phase} evidence")
        if sha256_file(path) != block["evidence_sha256"]:
            raise AttestationError(f"source {phase} evidence hash mismatch")
        if source_block(root, path) != block:
            raise AttestationError(
                f"source {phase} fields disagree with copied source evidence"
            )
    package = manifest["package"]
    metadata_path = relative_file(
        root, package["cargo_metadata_path"], "Cargo metadata"
    )
    if sha256_file(metadata_path) != package["cargo_metadata_sha256"]:
        raise AttestationError("Cargo metadata hash mismatch")
    package_name, package_version = cargo_package(metadata_path)
    if (package_name, package_version) != (package["name"], package["version"]):
        raise AttestationError(
            "attested package identity disagrees with Cargo metadata"
        )
    for name, entry in manifest["inputs"].items():
        path = relative_file(root, entry["path"], f"input {name}")
        if sha256_file(path) != entry["sha256"]:
            raise AttestationError(f"input changed after attestation: {name}")
        if path.stat().st_size != entry["bytes"]:
            raise AttestationError(
                f"input byte count changed after attestation: {name}"
            )
    for executable in manifest["build"]["executables"]:
        path = relative_file(root, executable["path"], "packaged executable")
        if sha256_file(path) != executable["sha256"]:
            raise AttestationError(f"packaged executable changed: {executable['path']}")
    for gate in manifest["gates"]:
        path = relative_file(root, gate["log_path"], f"gate log for {gate['name']}")
        if sha256_file(path) != gate["log_sha256"]:
            raise AttestationError(f"gate log changed: {gate['log_path']}")
    if parse_gates(root) != manifest["gates"]:
        raise AttestationError("gate evidence disagrees with summary.md")
    toolchain = manifest["build"]["toolchain"]
    requested_target = (
        toolchain["target"] if toolchain["target_source"] == "explicit" else None
    )
    if compiler_block(root, requested_target) != toolchain:
        raise AttestationError("toolchain evidence disagrees with captured environment")
    configuration = manifest["configuration"]
    config_path = relative_file(
        root,
        configuration["redacted_environment_path"],
        "redacted effective configuration",
    )
    if sha256_file(config_path) != configuration["redacted_environment_sha256"]:
        raise AttestationError("redacted effective configuration hash mismatch")
    gate_config = effective_gate_config(root)
    reconcile_build_config(gate_config, manifest["build"]["features"], toolchain)
    if (
        canonical_json_sha256(gate_config)
        != configuration["effective_gate_config_sha256"]
    ):
        raise AttestationError("effective gate configuration hash mismatch")
    if gate_config != configuration.get("effective_gate_config"):
        raise AttestationError("recorded effective gate configuration mismatch")
    if sorted(gate_config) != configuration.get("effective_gate_config_keys"):
        raise AttestationError("effective gate configuration key set mismatch")
    expected_configuration = effective_config_block(
        root,
        manifest["source"]["start"]["source_fingerprint_sha256"],
        gate_config,
    )
    if expected_configuration != configuration:
        raise AttestationError(
            "effective configuration evidence is incomplete or inconsistent"
        )
    if manifest["schema"] == SCHEMA_V2:
        expected_structured = structured_reporting_block(root, manifest["inputs"])
        if manifest.get("structured_reporting") != expected_structured:
            raise AttestationError(
                "v2 policy, generator, configuration, or gate-result binding changed"
            )
        reconcile_structured_reporting(
            expected_structured,
            manifest["gates"],
            manifest["run"]["mode"],
            manifest["result"]["failures"],
            manifest["result"]["skips"],
        )
    expected_state_config = {
        "beta_state_table_source": manifest["state_table"]["selected_source"],
        "beta_state_table_fallback_reason": manifest["state_table"]["fallback_reason"]
        or "none",
        "beta_state_table_sha256": manifest["state_table"]["selected_yaml_sha256"],
    }
    for key, expected in expected_state_config.items():
        if gate_config.get(key) != expected:
            raise AttestationError(
                f"selected state-table evidence disagrees with {key} in summary.md"
            )
    expected_performance_baseline = performance_regression_baseline_block(
        root, manifest["inputs"], manifest["run"]["mode"]
    )
    if expected_performance_baseline != manifest["performance_regression_baseline"]:
        raise AttestationError(
            "performance regression baseline evidence is incomplete or inconsistent"
        )
    expected_peers = peers_block(root, manifest["inputs"])
    if expected_peers != manifest["peers"]:
        raise AttestationError("peer evidence is incomplete or inconsistent")
    for result in manifest["results"]["json"]:
        path = relative_file(root, result["path"], "result JSON")
        if sha256_file(path) != result["sha256"]:
            raise AttestationError(f"result JSON changed: {result['path']}")
    expected_results = result_block(
        root,
        actual_files,
        manifest["source"]["start"]["source_fingerprint_sha256"],
    )
    if expected_results != manifest["results"]:
        raise AttestationError("result evidence is incomplete or inconsistent")
    reconcile_result_counts(
        root,
        manifest["gates"],
        gate_config,
        manifest["result"]["failures"],
        manifest["result"]["skips"],
    )
    expected_gate_inventory = gate_inventory_block(
        manifest["run"]["mode"], manifest["gates"], gate_config
    )
    if expected_gate_inventory != manifest["gate_inventory"]:
        raise AttestationError("required gate inventory is inconsistent")
    expected_pointers = pointer_block(
        manifest["run"]["mode"],
        manifest["source"]["clean"],
        manifest["source"]["unchanged"],
        manifest["result"]["failures"],
        manifest["result"]["skips"],
        manifest["build"]["executables"],
        manifest["peers"],
        manifest["results"],
        gate_config,
        actual_files,
        expected_gate_inventory,
        expected_performance_baseline,
        configuration["performance_result_reconciliation"],
    )
    if manifest["pointers"] != expected_pointers:
        raise AttestationError("mode-specific pointer eligibility is inconsistent")
    if manifest["qualification"] != qualification_block(
        manifest["run"]["mode"], expected_pointers
    ):
        raise AttestationError("release qualification evidence is inconsistent")

    if require_clean and manifest["source"]["clean"] is not True:
        raise AttestationError("release verification requires a clean source tree")
    if require_unchanged_source and manifest["source"]["unchanged"] is not True:
        raise AttestationError("release verification requires unchanged source")
    if require_no_skips and manifest["result"]["skips"] != 0:
        raise AttestationError("release verification requires zero skipped gates")
    if require_pass and manifest["result"]["overall"] != "PASS":
        raise AttestationError("release verification requires overall PASS")
    if (
        require_mode_eligible
        and manifest["pointers"]["mode_specific_eligible"] is not True
    ):
        reasons = "; ".join(manifest["pointers"]["ineligibility_reasons"])
        raise AttestationError(
            f"release verification requires mode-complete evidence: {reasons}"
        )
    return manifest


def atomic_pointer(path: pathlib.Path, run_id: str) -> None:
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(f"{run_id}\n", encoding="ascii")
    os.replace(temporary, path)


def update_latest_pointers(
    report_root: str | pathlib.Path, index_root: str | pathlib.Path
) -> list[pathlib.Path]:
    """Update informational and successful mode-specific report pointers."""
    report = pathlib.Path(report_root).expanduser().resolve()
    index = pathlib.Path(index_root).expanduser().resolve()
    manifest = verify_report(report)
    run_id = manifest["run"]["id"]
    if not isinstance(run_id, str) or re.fullmatch(r"\d{8}T\d{6}Z", run_id) is None:
        raise AttestationError(f"run ID is not a UTC beta timestamp: {run_id!r}")
    if report.name != run_id:
        raise AttestationError(
            f"report directory {report.name!r} does not match attested run ID {run_id!r}"
        )
    index.mkdir(parents=True, exist_ok=True)
    updated = [index / "latest.txt"]
    # This pointer intentionally means only "most recently packaged". It is
    # never sufficient to select release evidence.
    atomic_pointer(updated[0], run_id)
    if manifest["pointers"]["mode_specific_eligible"] is not True:
        return updated
    mode = manifest["run"]["mode"]
    mode_pointer = index / MODE_POINTERS[mode]
    atomic_pointer(mode_pointer, run_id)
    updated.append(mode_pointer)
    return updated


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)
    creator = subparsers.add_parser(
        "create", help="create and self-verify an attestation"
    )
    creator.add_argument("--report-root", required=True)
    creator.add_argument(
        "--schema-version",
        type=int,
        choices=(1, 2),
        default=1,
        help="v2 directly binds native policy/configuration/gate-result reporting inputs",
    )
    creator.add_argument(
        "--source-start", default="environment/source-at-beta-start.json"
    )
    creator.add_argument("--source-end", default="environment/source-at-beta-end.json")
    creator.add_argument("--cargo-metadata", default="environment/cargo-metadata.json")
    creator.add_argument("--mode", required=True, choices=sorted(VALID_MODES))
    creator.add_argument("--run-id", required=True)
    creator.add_argument("--started-at", required=True)
    creator.add_argument("--ended-at", required=True)
    creator.add_argument("--features", default="")
    creator.add_argument("--target")
    creator.add_argument("--input", action="append", default=[])
    creator.add_argument(
        "--state-table-source",
        choices=sorted(VALID_STATE_TABLE_SOURCES),
        default="embedded-default",
    )
    creator.add_argument(
        "--state-table-fallback-reason",
        choices=sorted(VALID_STATE_TABLE_FALLBACK_REASONS),
    )
    creator.add_argument("--state-table-sha256")
    creator.add_argument("--failures", required=True, type=int)
    creator.add_argument("--skips", required=True, type=int)
    creator.add_argument("--overall", required=True, choices=("PASS", "FAIL"))

    verifier = subparsers.add_parser(
        "verify", help="verify a copied report independently"
    )
    verifier.add_argument("--report-root", required=True)
    verifier.add_argument("--require-clean", action="store_true")
    verifier.add_argument("--require-unchanged-source", action="store_true")
    verifier.add_argument("--require-no-skips", action="store_true")
    verifier.add_argument("--require-pass", action="store_true")
    verifier.add_argument("--require-mode-eligible", action="store_true")

    pointers = subparsers.add_parser(
        "update-pointers", help="update generic and successful mode-specific pointers"
    )
    pointers.add_argument("--report-root", required=True)
    pointers.add_argument("--index-root", required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "create":
            output = create_attestation(args)
            manifest = verify_report(output.parent)
            print(
                f"beta attestation: created and verified {output}; "
                f"qualification={manifest['qualification']['status']}"
            )
        elif args.command == "verify":
            manifest = verify_report(
                args.report_root,
                require_clean=args.require_clean,
                require_unchanged_source=args.require_unchanged_source,
                require_no_skips=args.require_no_skips,
                require_pass=args.require_pass,
                require_mode_eligible=args.require_mode_eligible,
            )
            print(
                "beta attestation: verified "
                f"{manifest['run']['mode']} {manifest['run']['id']} "
                f"{manifest['result']['overall']} "
                f"mode_evidence={'ELIGIBLE' if manifest['pointers']['mode_specific_eligible'] else 'INELIGIBLE'}"
            )
        else:
            updated = update_latest_pointers(args.report_root, args.index_root)
            print(
                "beta attestation: updated pointers "
                + ", ".join(str(path) for path in updated)
            )
    except AttestationError as error:
        print(f"beta attestation: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
