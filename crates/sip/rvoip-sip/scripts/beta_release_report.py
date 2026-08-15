#!/usr/bin/env python3
"""Generate, verify, and promote evidence-complete beta release reports.

The current candidate is a post-run derivation from a v1 attestation. This
tool never executes a beta gate. Future beta runs also use its structured
configuration and per-gate recording commands so Markdown is not an input.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import hashlib
import importlib.util
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


SCHEMA_EVIDENCE_LEGACY = "rvoip-sip-release-evidence-v1"
SCHEMA_EVIDENCE = "rvoip-sip-release-evidence-v2"
SCHEMA_CONFIG = "rvoip-sip-effective-gate-config-v1"
SCHEMA_RESULTS = "rvoip-sip-gate-results-v1"
SCHEMA_REPORT_ATTESTATION_LEGACY = "rvoip-sip-report-attestation-v1"
SCHEMA_REPORT_ATTESTATION = "rvoip-sip-report-attestation-v2"
SCHEMA_INTEROP_ATTESTATION = "rvoip-sip-interop-peer-attestation-v1"
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
PROXY_INTEROP_GATE_NAME = "Kamailio/OpenSIPS stateful-proxy interoperability matrix"
INTEROP_ATTESTATION_SPECS = {
    "asterisk": {
        "display_name": "Asterisk",
        "gate_id": "interop.asterisk-matrix",
        "identity_evidence_prefix": "environment/local-pbx/asterisk",
        "scope": (
            "PBX call-control and RTP media interoperability for the recorded "
            "API, scenario, codec, and security matrix"
        ),
    },
    "freeswitch": {
        "display_name": "FreeSWITCH",
        "gate_id": "interop.freeswitch-matrix",
        "identity_evidence_prefix": "environment/local-pbx/freeswitch",
        "scope": (
            "PBX call-control and RTP media interoperability for the recorded "
            "API, scenario, codec, and security matrix"
        ),
    },
    "kamailio": {
        "display_name": "Kamailio",
        "gate_id": "interop.proxy-stateful-matrix",
        "identity_evidence_prefix": "proxy-interop/",
        "scope": (
            "RFC 3261 transaction-stateful proxy interoperability in both hop "
            "orders over UDP, TCP, and TLS"
        ),
    },
    "opensips": {
        "display_name": "OpenSIPS",
        "gate_id": "interop.proxy-stateful-matrix",
        "identity_evidence_prefix": "proxy-interop/",
        "scope": (
            "RFC 3261 transaction-stateful proxy interoperability in both hop "
            "orders over UDP, TCP, and TLS"
        ),
    },
}
INTEROP_ATTESTATION_NON_CLAIM = (
    "PASS is limited to the exact peer versions, configurations, transports, "
    "codecs, scenarios, source tree, and evidence recorded here."
)
PROXY_INTEROP_ANCILLARY_EVIDENCE = (
    "proxy-interop/cargo-build.log",
    "proxy-interop/cargo-build-command.txt",
    "proxy-interop/environment.txt",
    "proxy-interop/matrix.tsv",
    "proxy-interop/proxy-binary-check.txt",
    "proxy-interop/proxy-binary.path",
    "proxy-interop/proxy-binary.sha256",
    "proxy-interop/runtime-state-check.json",
    "proxy-interop/runtime-state-end.json",
    "proxy-interop/runtime-state-start.json",
    "proxy-interop/source-check.txt",
    "proxy-interop/summary.json",
    "proxy-interop/summary.md",
)
PROXY_INTEROP_IMAGES = {
    "kamailio": (
        "ghcr.io/kamailio/kamailio:6.1.3-bookworm@"
        "sha256:26b26c61801d679ffbe54ea3597c38964a46c4bfe60fb6537c7eeacc576b0c92"
    ),
    "opensips": (
        "opensips/opensips:3.6@"
        "sha256:eba1396b438a7f8a9d33c17017aae4670cb43361eb7130359240cf85fc3e6979"
    ),
}
PROXY_INTEROP_PEERS = frozenset(PROXY_INTEROP_IMAGES)
PROXY_INTEROP_PEER_VERSION_RE = {
    "kamailio": re.compile(r"(?im)^version:\s*kamailio\s+6\.1\.3(?:\s|$|\()"),
    "opensips": re.compile(r"(?im)^version:\s*opensips\s+3\.6\.7(?:\s|$|\()"),
}
PROXY_INTEROP_BINARY_PATH = "target/release/examples/stateful_proxy_interop"
PROXY_INTEROP_ENVIRONMENT_TOOL_KEYS = frozenset(
    {
        "cargo",
        "cargo_path",
        "docker",
        "docker_path",
        "rustc",
        "rustc_path",
        "sipp",
        "sipp_path",
        "tcpdump",
        "tcpdump_path",
        "tshark",
        "tshark_path",
    }
)
PROXY_INTEROP_ORDERS = frozenset({"rvoip-first", "peer-first"})
PROXY_INTEROP_TRANSPORTS = frozenset({"udp", "tcp", "tls"})
PROXY_INTEROP_SCENARIOS = frozenset(
    {
        "options-readiness",
        "invite-success",
        "matched-cancel-before-provisional",
        "matched-cancel-after-provisional",
        "cancel-retransmission",
        "unmatched-cancel",
        "ack-non2xx",
        "sequential-fork",
        "parallel-fork",
        "multiple-2xx",
        "late-2xx",
        "sixxx-cancel",
        "timer-c-calling",
        "timer-c-proceeding",
        "transport-failure",
        "rfc3263-failover",
        "via-response-destination",
        "route-strict",
        "route-loose-record-route",
        "sips-routing",
        "auth-aggregation",
        "message-body-content-length",
        "stray-response-drop",
        "capacity-overload",
        "retention-cleanup",
    }
)
PROXY_INTEROP_EVERY_ROW_SCENARIOS = frozenset(
    {
        "options-readiness",
        "invite-success",
        "matched-cancel-before-provisional",
        "matched-cancel-after-provisional",
        "cancel-retransmission",
        "unmatched-cancel",
        "ack-non2xx",
        "via-response-destination",
        "message-body-content-length",
        "retention-cleanup",
    }
)
PROXY_INTEROP_UDP_ADVANCED_SCENARIOS = frozenset(
    {
        "sequential-fork",
        "parallel-fork",
        "multiple-2xx",
        "late-2xx",
        "sixxx-cancel",
        "stray-response-drop",
    }
)
PROXY_INTEROP_TCP_ADVANCED_SCENARIOS = frozenset(
    {
        "timer-c-calling",
        "timer-c-proceeding",
        "transport-failure",
        "rfc3263-failover",
        "capacity-overload",
        "route-strict",
        "route-loose-record-route",
        "auth-aggregation",
    }
)
PROXY_INTEROP_SCENARIO_SCHEMA = "rvoip-sip-proxy-interop-scenario-v1"
PROXY_INTEROP_EXTERNAL_EVIDENCE_KINDS = frozenset(
    {
        "external-sipp-and-packet-observation",
        "external-raw-wire-and-packet-observation",
        "retention-phase-snapshots",
        "verified-external-tls",
    }
)
PROXY_INTEROP_RAW_EVIDENCE_KIND = "external-raw-wire-and-packet-observation"
PROXY_INTEROP_RAW_SCHEMAS = {
    "udp": "rvoip-sip-proxy-interop-raw-wire-v1",
    "tcp": "rvoip-sip-proxy-interop-advanced-raw-wire-v1",
}
PROXY_INTEROP_PACKET_REQUIREMENTS = {
    "options-readiness": ({"OPTIONS"}, {200}),
    "invite-success": ({"INVITE", "ACK", "BYE"}, {180, 200}),
    "matched-cancel-before-provisional": (
        {"INVITE", "CANCEL", "ACK"},
        {200, 487},
    ),
    "matched-cancel-after-provisional": (
        {"INVITE", "CANCEL", "ACK"},
        {180, 200, 487},
    ),
    "cancel-retransmission": (
        {"INVITE", "CANCEL", "ACK"},
        {180, 200, 487},
    ),
    "unmatched-cancel": ({"CANCEL"}, {481}),
    "ack-non2xx": ({"INVITE", "ACK"}, {486}),
    "via-response-destination": ({"OPTIONS"}, {200}),
    "message-body-content-length": ({"MESSAGE"}, {200}),
    "sequential-fork": ({"INVITE", "ACK"}, {180, 200, 486}),
    "parallel-fork": ({"INVITE", "ACK"}, {180, 200, 480, 486}),
    "multiple-2xx": ({"INVITE", "ACK"}, {200, 486}),
    "late-2xx": ({"INVITE", "ACK"}, {200, 480, 486}),
    "sixxx-cancel": (
        {"INVITE", "CANCEL", "ACK"},
        {180, 200, 487, 603},
    ),
    "stray-response-drop": ({"OPTIONS"}, {200}),
    "timer-c-calling": ({"INVITE"}, {408}),
    "timer-c-proceeding": ({"INVITE", "CANCEL"}, {180, 200, 487}),
    "transport-failure": ({"OPTIONS", "INVITE"}, {200, 503}),
    "rfc3263-failover": ({"INVITE"}, {200}),
    "route-strict": ({"INVITE"}, {200}),
    "route-loose-record-route": ({"INVITE"}, {200}),
    "auth-aggregation": ({"INVITE"}, {401, 407}),
    "capacity-overload": ({"INVITE"}, {486, 503}),
    "sips-routing": ({"OPTIONS"}, {200}),
}
PROXY_INTEROP_SIP_PACKET_ASSERTIONS = frozenset(
    {
        "scenario-call-id-observed",
        "required-methods-observed",
        "required-statuses-observed",
        "rvoip-via-observed",
        "peer-via-observed",
    }
)
PROXY_INTEROP_TLS_PACKET_ASSERTIONS = frozenset(
    {
        "tls-packets-observed",
        "tls-application-data-on-every-encrypted-hop",
        "tls-handshake-sni-valid-when-observed",
        "tls-handshake-certificates-observed-when-initiated",
    }
)
PROXY_INTEROP_INVITE_DIALOG_PACKET_ASSERTIONS = frozenset(
    {
        "invite-dialog-response-contact-and-record-route-set",
        "invite-dialog-uac-ack-bye-use-contact-and-reversed-route-set",
        "invite-dialog-downstream-ack-bye-reach-contact-with-routes-consumed",
    }
)
PROXY_INTEROP_STRAY_PACKET_ASSERTIONS = frozenset(
    {
        "one-true-stray-call-observed",
        "true-stray-arrived-at-rvoip",
        "true-stray-had-zero-rvoip-egress",
    }
)
PROXY_INTEROP_SIPS_PACKET_ASSERTIONS = frozenset(
    {
        "sips-request-uri-at-uac-boundary",
        "sips-request-uri-at-uas-boundary",
        "sips-request-preserved-end-to-end",
        "sips-both-proxy-vias-observed",
        "no-plaintext-sip-on-external-tls-ports",
        "sips-options-success-observed",
    }
)
PROXY_INTEROP_SIPS_RESULT_ASSERTIONS = frozenset(
    {
        "actual-sips-request-on-boundary-plaintext",
        "sips-request-uri-preserved",
        "both-real-proxy-vias",
        "sipp-full-path-success",
        "external-proxy-hops-mtls-only",
        "independent-tls-verifier",
        "scenario-owned-packet-capture",
    }
)
PROXY_INTEROP_SUPPLEMENTAL_EVIDENCE_KIND = "in-process-rust-conformance-test"
PROXY_INTEROP_RETENTION_PHASES = (
    "pre_zero",
    "activity",
    "cooldown",
    "post_retention",
    "pre_shutdown",
    "post_shutdown",
)
PROXY_INTEROP_TLS_RESULT_SCHEMA = "rvoip-sip-proxy-interop-tls-evidence-v2"
PROXY_INTEROP_TLS_PACKET_SCENARIOS = (
    "options-readiness",
    "invite-success",
    "matched-cancel-before-provisional",
    "matched-cancel-after-provisional",
    "cancel-retransmission",
    "unmatched-cancel",
    "ack-non2xx",
    "via-response-destination",
    "message-body-content-length",
    "sips-routing",
)
PROXY_INTEROP_RUNTIME_STATE_SCHEMA = "rvoip-sip-proxy-interop-runtime-state-v1"
PROXY_INTEROP_PROHIBITED_RAW_FILES = {
    "container-inspect.json",
    "image-inspect.json",
}
PROXY_INTEROP_PRIVATE_KEY_PEM = re.compile(
    rb"-----BEGIN (?:RSA |EC |ENCRYPTED |OPENSSH )?PRIVATE KEY-----"
)
PROXY_INTEROP_PEER_RUNTIME_SCHEMA = "rvoip-docker-peer-snapshot-v1"
PROXY_INTEROP_OPENSIPS_TLS_REFERENCE = "rvoip/opensips-tls-interop:3.6.7-1"
PROXY_INTEROP_OPENSIPS_TLS_PROVENANCE_SCHEMA = (
    "rvoip-sip-proxy-interop-opensips-tls-image-v1"
)
PROXY_INTEROP_OPENSIPS_TLS_DOCKERFILE = (
    "crates/sip/sip-proxy/tests/interop/images/opensips-tls/Dockerfile"
)
PROXY_INTEROP_OPENSIPS_TLS_DOCKERFILE_SHA256 = (
    "2ad332bdbb48e707c3995e0e95500053857a908c178de025d8180f6133d4501c"
)
PROXY_INTEROP_OPENSIPS_TLS_PACKAGES = {
    "opensips": {"version": "3.6.7-1"},
    "opensips-tls-module": {
        "version": "3.6.7-1",
        "deb_sha256": (
            "685f704faf2ce6b9015a0c059a32333bf789cabd8a467c7068aa1cea363de799"
        ),
    },
    "opensips-tlsmgm-module": {
        "version": "3.6.7-1",
        "deb_sha256": (
            "20d83193399a9ee02b8c3f1cc2fbe311231ec22c0b43656add63f77110a68545"
        ),
    },
    "opensips-tls-openssl-module": {
        "version": "3.6.7-1",
        "deb_sha256": (
            "690c52e06b9d0f8d76483900a4185f3a3d7cc3ec30a752ff762bdca254750209"
        ),
    },
}
PROXY_INTEROP_OPENSIPS_TLS_MODULES = {
    "proto_tls.so": {
        "path": "/usr/lib/x86_64-linux-gnu/opensips/modules/proto_tls.so",
        "sha256": ("c0a3b92a2d64fc58c8d015338f8016d8fe59e9cc131b91f2774f07281349f7f3"),
    },
    "tls_mgm.so": {
        "path": "/usr/lib/x86_64-linux-gnu/opensips/modules/tls_mgm.so",
        "sha256": ("f2726bc731dbdf840bd40dbc7209eded8f16899034177b8dabed481da60662d2"),
    },
    "tls_openssl.so": {
        "path": "/usr/lib/x86_64-linux-gnu/opensips/modules/tls_openssl.so",
        "sha256": ("77714d25e26933b4a18408b3ef65bad75400d5840d1bcf58efde99292e21ba6f"),
    },
}


class ReportError(RuntimeError):
    """A fail-closed reporting or verification error."""


def canonical_json(value: Any) -> bytes:
    return (
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    ).encode()


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
    with tempfile.NamedTemporaryFile(
        dir=path.parent, prefix=f".{path.name}.", delete=False
    ) as tmp:
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
            raise ReportError(
                f"unknown or missing validators for {gate['id']}: {validators}"
            )
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
                raise ReportError(
                    f"catalog gate {gate['id']} uses unknown condition {key}"
                )
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
    raw_attested = dict(
        attestation.get("configuration", {}).get("effective_gate_config", {})
    )
    raw_environment = parse_redacted_environment(
        report_root / "environment/beta-env-redacted.txt"
    )
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
            raise ReportError(
                f"release configuration {key}={actual!r}, expected {expected!r}"
            )
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
        prefixes = (
            "beta_pbx_",
            "beta_proxy_interop_",
            "beta_run_",
            "beta_restore_",
            "beta_sipp_",
        )
    elif (
        category == "Performance and resiliency"
        or category == "Reporting and regression"
    ):
        prefixes = (
            "beta_perf_",
            "beta_profile_",
            "beta_run_",
            "beta_burst_",
            "rvoip_perf_",
        )
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
    if name == PROXY_INTEROP_GATE_NAME:
        return list(PROXY_INTEROP_ANCILLARY_EVIDENCE)
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


def _proxy_interop_artifact_dir(root: Path, relative: Any) -> Path:
    if not isinstance(relative, str) or not safe_relative(relative):
        raise ReportError(
            f"unsafe stateful proxy interoperability artifact: {relative!r}"
        )
    proxy_root = (root / "proxy-interop").resolve()
    candidate = (proxy_root / relative).resolve()
    try:
        candidate.relative_to(proxy_root)
    except ValueError as exc:
        raise ReportError(
            f"stateful proxy interoperability artifact escapes its root: {relative!r}"
        ) from exc
    if not candidate.is_dir() or candidate.is_symlink():
        raise ReportError(
            f"stateful proxy interoperability row artifact is missing: {relative}"
        )
    return candidate


def _required_proxy_interop_file(path: Path, label: str) -> Path:
    if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
        raise ReportError(f"stateful proxy interoperability {label} is missing: {path}")
    return path


def _proxy_interop_relative_file(directory: Path, relative: Any, label: str) -> Path:
    if not isinstance(relative, str) or not safe_relative(relative):
        raise ReportError(
            f"stateful proxy interoperability {label} has an unsafe path: {relative!r}"
        )
    unresolved = directory / relative
    current = directory
    for part in Path(relative).parts:
        current /= part
        if current.is_symlink():
            raise ReportError(
                f"stateful proxy interoperability {label} may not use symlinks: "
                f"{relative!r}"
            )
    root = directory.resolve()
    candidate = unresolved.resolve()
    try:
        candidate.relative_to(root)
    except ValueError as exc:
        raise ReportError(
            f"stateful proxy interoperability {label} escapes its root: {relative!r}"
        ) from exc
    return _required_proxy_interop_file(candidate, label)


def _proxy_interop_exact_keys(
    value: Any, expected: set[str], label: str
) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        actual = sorted(value) if isinstance(value, dict) else type(value).__name__
        raise ReportError(
            f"stateful proxy interoperability {label} fields are invalid; "
            f"expected={sorted(expected)}, actual={actual}"
        )
    return value


def _validate_proxy_interop_assertions(value: Any, label: str) -> None:
    if not isinstance(value, list) or not value:
        raise ReportError(f"stateful proxy interoperability {label} has no assertions")
    names: set[str] = set()
    for item in value:
        assertion = _proxy_interop_exact_keys(
            item, {"name", "passed", "observed"}, f"{label} assertion"
        )
        name = assertion.get("name")
        if (
            not isinstance(name, str)
            or not name
            or name in names
            or assertion.get("passed") is not True
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} assertion failed: {name!r}"
            )
        names.add(name)


def _proxy_interop_assertions_by_name(
    value: Any, required: set[str] | frozenset[str], label: str
) -> dict[str, dict[str, Any]]:
    _validate_proxy_interop_assertions(value, label)
    assertions = {item["name"]: item for item in value}
    missing = set(required) - set(assertions)
    if missing:
        raise ReportError(
            f"stateful proxy interoperability {label} is missing required "
            f"assertions: {sorted(missing)}"
        )
    return assertions


def _validate_proxy_interop_inputs(
    scenario_dir: Path, value: Any, label: str
) -> dict[str, Path]:
    if not isinstance(value, dict) or not value:
        raise ReportError(f"stateful proxy interoperability {label} has no inputs")
    validated: dict[str, Path] = {}
    for relative, raw_record in value.items():
        if not isinstance(relative, str):
            raise ReportError(
                f"stateful proxy interoperability {label} input name is invalid"
            )
        record = _proxy_interop_exact_keys(
            raw_record, {"sha256", "bytes"}, f"{label} input {relative!r}"
        )
        expected_hash = record.get("sha256")
        expected_bytes = record.get("bytes")
        if (
            not isinstance(expected_hash, str)
            or re.fullmatch(r"[0-9a-f]{64}", expected_hash) is None
            or not isinstance(expected_bytes, int)
            or isinstance(expected_bytes, bool)
            or expected_bytes <= 0
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} input metadata is invalid: "
                f"{relative!r}"
            )
        path = _proxy_interop_relative_file(scenario_dir, relative, f"{label} input")
        if path.stat().st_size != expected_bytes or sha256_path(path) != expected_hash:
            raise ReportError(
                f"stateful proxy interoperability {label} input hash drift: "
                f"{relative!r}"
            )
        validated[relative] = path
    return validated


def _validate_proxy_interop_identity(
    payload: dict[str, Any],
    scenario: str,
    combination: tuple[str, str, str],
    label: str,
) -> None:
    if (
        payload.get("scenario") != scenario
        or (
            payload.get("peer"),
            payload.get("order"),
            payload.get("transport"),
        )
        != combination
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} identity mismatch: "
            f"{combination!r}/{scenario}"
        )


def _expected_proxy_interop_scenarios(
    combination: tuple[str, str, str],
) -> frozenset[str]:
    _peer, order, transport = combination
    expected = set(PROXY_INTEROP_EVERY_ROW_SCENARIOS)
    if order == "peer-first" and transport == "udp":
        expected.update(PROXY_INTEROP_UDP_ADVANCED_SCENARIOS)
    if order == "peer-first" and transport == "tcp":
        expected.update(PROXY_INTEROP_TCP_ADVANCED_SCENARIOS)
    if transport == "tls":
        expected.add("sips-routing")
    return frozenset(expected)


def _validate_proxy_interop_retention(
    payload: dict[str, Any], label: str, log_path: Path
) -> None:
    phases = payload.get("phases")
    if not isinstance(phases, dict) or set(phases) != set(
        PROXY_INTEROP_RETENTION_PHASES
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} retention phases are incomplete"
        )
    parsed: dict[str, dict[str, int]] = {}
    for phase in PROXY_INTEROP_RETENTION_PHASES:
        phase_record = _proxy_interop_exact_keys(
            phases[phase], {"counters", "nonzero"}, f"{label} phase {phase}"
        )
        counters = phase_record.get("counters")
        nonzero = phase_record.get("nonzero")
        if not isinstance(counters, dict) or not isinstance(nonzero, dict):
            raise ReportError(
                f"stateful proxy interoperability {label} phase {phase} is invalid"
            )
        for field, values in (("counters", counters), ("nonzero", nonzero)):
            if any(
                not isinstance(name, str)
                or not name
                or not isinstance(value, int)
                or isinstance(value, bool)
                or value < 0
                for name, value in values.items()
            ):
                raise ReportError(
                    f"stateful proxy interoperability {label} phase {phase} "
                    f"{field} is invalid"
                )
        expected_nonzero = {
            name: value for name, value in counters.items() if value != 0
        }
        if nonzero != expected_nonzero:
            raise ReportError(
                f"stateful proxy interoperability {label} phase {phase} "
                "nonzero counters disagree"
            )
        parsed[phase] = counters
    if (
        any(parsed["pre_zero"].values())
        or not any(parsed["activity"].values())
        or any(parsed["post_retention"].values())
        or any(parsed["pre_shutdown"].values())
        or any(parsed["post_shutdown"].values())
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} retention did not converge"
        )
    observed_phases: list[str] = []
    observed_counters: dict[str, dict[str, int]] = {}
    for raw_line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not raw_line.startswith("RVOIP_PROXY_RETENTION "):
            continue
        fields: dict[str, str] = {}
        for raw_field in raw_line.split()[1:]:
            key, separator, value = raw_field.partition("=")
            if not separator or not key or key in fields:
                raise ReportError(
                    f"stateful proxy interoperability {label} retention log "
                    "contains an invalid field"
                )
            fields[key] = value
        phase = fields.pop("phase", None)
        if phase not in PROXY_INTEROP_RETENTION_PHASES or phase in observed_counters:
            raise ReportError(
                f"stateful proxy interoperability {label} retention log "
                "contains an invalid phase"
            )
        try:
            counters = {name: int(value) for name, value in fields.items()}
        except ValueError as exc:
            raise ReportError(
                f"stateful proxy interoperability {label} retention log "
                "contains a non-integer counter"
            ) from exc
        if any(value < 0 for value in counters.values()):
            raise ReportError(
                f"stateful proxy interoperability {label} retention log "
                "contains a negative counter"
            )
        observed_phases.append(phase)
        observed_counters[phase] = counters
    if (
        tuple(observed_phases) != PROXY_INTEROP_RETENTION_PHASES
        or observed_counters != parsed
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} retention phases are "
            "not bound to the raw log"
        )


def _validate_proxy_interop_packet_evidence(
    scenario_dir: Path,
    scenario: str,
    combination: tuple[str, str, str],
    inputs: dict[str, Path],
    label: str,
    claimed_captures: dict[tuple[str, Any], str],
) -> None:
    packet_entries = [
        (relative, path)
        for relative, path in inputs.items()
        if path.name == "packet-evidence.json"
    ]
    if len(packet_entries) != 1:
        raise ReportError(
            f"stateful proxy interoperability {label} is not packet-evidence bound"
        )
    _relative, packet_path = packet_entries[0]
    payload = read_json(packet_path)
    common_keys = {
        "schema",
        "scenario",
        "status",
        "peer",
        "order",
        "transport",
        "analyzer",
        "captures",
        "display_filter",
        "selected_packet_count",
        "assertions",
    }
    is_sips_routing = scenario == "sips-routing"
    if combination[2] == "tls":
        expected_keys = common_keys | {
            "selected_call_ids",
            "selected_tls_packet_count",
            "selected_sip_packet_count",
            "observed_methods",
            "observed_statuses",
            "via_sent_by_addresses",
            "via_sent_by_ports",
            "observed_sni",
            "observed_handshake_types",
            "observed_certificate_sha256",
            "observed_tls_application_listener_ports",
            "observed_alerts",
        }
        if is_sips_routing:
            expected_keys |= {
                "observed_sips_request_uris",
                "plaintext_sip_endpoints",
                "insecure_external_sip_packet_count",
            }
    else:
        expected_keys = common_keys | {
            "selected_call_ids",
            "observed_methods",
            "observed_statuses",
            "via_sent_by_addresses",
            "via_sent_by_ports",
        }
    packet = _proxy_interop_exact_keys(
        payload, expected_keys, f"{label} packet evidence"
    )
    analyzer = packet.get("analyzer")
    captures = packet.get("captures")
    packet_count = packet.get("selected_packet_count")
    if (
        packet.get("schema") != "rvoip-sip-proxy-interop-packet-evidence-v1"
        or packet.get("scenario") != scenario
        or packet.get("status") != "PASS"
        or (
            packet.get("peer"),
            packet.get("order"),
            packet.get("transport"),
        )
        != combination
        or packet.get("display_filter")
        != ("sip or tls" if combination[2] == "tls" else "sip")
        or not isinstance(analyzer, dict)
        or set(analyzer) != {"tshark", "libpcap"}
        or any(
            not isinstance(analyzer.get(name), str) or not analyzer[name]
            for name in analyzer
        )
        or not isinstance(captures, list)
        or not captures
        or not isinstance(packet_count, int)
        or isinstance(packet_count, bool)
        or packet_count <= 0
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} packet evidence is invalid"
        )
    required_assertions = (
        PROXY_INTEROP_TLS_PACKET_ASSERTIONS | PROXY_INTEROP_SIP_PACKET_ASSERTIONS
        if combination[2] == "tls"
        else PROXY_INTEROP_SIP_PACKET_ASSERTIONS
    )
    if is_sips_routing:
        required_assertions = (
            required_assertions
            | PROXY_INTEROP_SIP_PACKET_ASSERTIONS
            | PROXY_INTEROP_SIPS_PACKET_ASSERTIONS
        )
    if scenario == "stray-response-drop":
        required_assertions = (
            required_assertions | PROXY_INTEROP_STRAY_PACKET_ASSERTIONS
        )
    if combination[2] == "tls" and scenario == "invite-success":
        required_assertions = (
            required_assertions | PROXY_INTEROP_INVITE_DIALOG_PACKET_ASSERTIONS
        )
    packet_assertions = _proxy_interop_assertions_by_name(
        packet.get("assertions"),
        required_assertions,
        f"{label} packet evidence",
    )
    row_dir = scenario_dir.parents[1]
    observed_captures: set[str] = set()
    for raw_capture in captures:
        capture = _proxy_interop_exact_keys(
            raw_capture,
            {"filename", "sha256", "bytes"},
            f"{label} packet capture",
        )
        filename = capture.get("filename")
        expected_hash = capture.get("sha256")
        expected_bytes = capture.get("bytes")
        if (
            not isinstance(filename, str)
            or Path(filename).name != filename
            or filename in observed_captures
            or not filename.startswith(f"{scenario}--")
            or not filename.endswith(".pcap")
            or not isinstance(expected_hash, str)
            or re.fullmatch(r"[0-9a-f]{64}", expected_hash) is None
            or not isinstance(expected_bytes, int)
            or isinstance(expected_bytes, bool)
            or expected_bytes <= 24
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} packet capture is invalid"
            )
        capture_path = _proxy_interop_relative_file(
            row_dir, filename, f"{label} row packet capture"
        )
        if (
            capture_path.stat().st_size != expected_bytes
            or sha256_path(capture_path) != expected_hash
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} packet capture hash drift"
            )
        capture_identity = capture_path.stat()
        ownership_keys: tuple[tuple[str, Any], ...] = (
            ("path", str(capture_path.resolve())),
            ("inode", (capture_identity.st_dev, capture_identity.st_ino)),
            ("sha256", expected_hash),
        )
        for ownership_key in ownership_keys:
            owner = claimed_captures.get(ownership_key)
            if owner is not None and owner != label:
                raise ReportError(
                    "stateful proxy interoperability packet capture is reused "
                    f"across scenarios: {owner!r} and {label!r}"
                )
            claimed_captures[ownership_key] = label
        observed_captures.add(filename)
    list_fields = (
        {
            "observed_sni",
            "observed_handshake_types",
            "observed_certificate_sha256",
            "observed_tls_application_listener_ports",
            "observed_alerts",
            "selected_call_ids",
            "observed_methods",
            "observed_statuses",
            "via_sent_by_addresses",
            "via_sent_by_ports",
        }
        if combination[2] == "tls"
        else {
            "selected_call_ids",
            "observed_methods",
            "observed_statuses",
            "via_sent_by_addresses",
            "via_sent_by_ports",
        }
    )
    if is_sips_routing:
        list_fields |= {
            "selected_call_ids",
            "observed_methods",
            "observed_statuses",
            "via_sent_by_addresses",
            "via_sent_by_ports",
            "observed_sips_request_uris",
            "plaintext_sip_endpoints",
        }
    if any(not isinstance(packet.get(field), list) for field in list_fields):
        raise ReportError(
            f"stateful proxy interoperability {label} packet observations are invalid"
        )
    if combination[2] == "tls":
        handshake_types = packet["observed_handshake_types"]
        snis = packet["observed_sni"]
        certificates = packet["observed_certificate_sha256"]
        application_ports = packet["observed_tls_application_listener_ports"]
        tls_packet_count = packet.get("selected_tls_packet_count")
        sip_packet_count = packet.get("selected_sip_packet_count")
        application_assertion = packet_assertions[
            "tls-application-data-on-every-encrypted-hop"
        ].get("observed")
        sni_assertion = packet_assertions["tls-handshake-sni-valid-when-observed"].get(
            "observed"
        )
        certificate_assertion = packet_assertions[
            "tls-handshake-certificates-observed-when-initiated"
        ].get("observed")
        if (
            not isinstance(application_assertion, dict)
            or set(application_assertion)
            != {
                "required_listener_ports",
                "observed_ports",
                "application_record_count",
            }
            or not isinstance(sni_assertion, dict)
            or set(sni_assertion) != {"allowed", "observed", "client_hello_observed"}
            or not isinstance(certificate_assertion, dict)
            or set(certificate_assertion)
            != {"client_hello_observed", "certificate_sha256"}
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} TLS packet "
                "assertions have invalid observations"
            )
        required_application_ports = application_assertion["required_listener_ports"]
        observed_application_ports = application_assertion["observed_ports"]
        application_record_count = application_assertion["application_record_count"]
        allowed_sni = sni_assertion["allowed"]
        assertion_sni = sni_assertion["observed"]
        client_hello_observed = sni_assertion["client_hello_observed"]
        assertion_certificates = certificate_assertion["certificate_sha256"]
        certificate_client_hello = certificate_assertion["client_hello_observed"]
        string_lists = (
            handshake_types,
            snis,
            certificates,
            application_ports,
            required_application_ports,
            observed_application_ports,
            allowed_sni,
            assertion_sni,
            assertion_certificates,
        )
        if (
            not isinstance(tls_packet_count, int)
            or isinstance(tls_packet_count, bool)
            or tls_packet_count <= 0
            or not isinstance(sip_packet_count, int)
            or isinstance(sip_packet_count, bool)
            or sip_packet_count <= 0
            or packet_count != tls_packet_count + sip_packet_count
            or any(not isinstance(items, list) for items in string_lists)
            or any(
                not isinstance(item, str) or not item
                for items in string_lists
                for item in items
            )
            or any(
                re.fullmatch(r"[0-9a-f]{64}", item) is None
                for item in (*certificates, *assertion_certificates)
            )
            or packet_assertions["tls-packets-observed"].get("observed")
            != tls_packet_count
            or not required_application_ports
            or not set(required_application_ports) <= set(observed_application_ports)
            or set(application_ports) != set(required_application_ports)
            or not isinstance(application_record_count, int)
            or isinstance(application_record_count, bool)
            or application_record_count <= 0
            or not allowed_sni
            or set(snis) - set(allowed_sni)
            or assertion_sni != snis
            or not isinstance(client_hello_observed, bool)
            or not isinstance(certificate_client_hello, bool)
            or certificate_client_hello != client_hello_observed
            or client_hello_observed != ("1" in handshake_types)
            or (client_hello_observed and not snis)
            or assertion_certificates != certificates
            or (client_hello_observed and not certificates)
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} TLS packet "
                "assertions disagree with packet observations"
            )

    selected_call_ids = packet["selected_call_ids"]
    observed_methods = packet["observed_methods"]
    observed_statuses = packet["observed_statuses"]
    if (
        any(not isinstance(item, str) or not item for item in selected_call_ids)
        or any(not isinstance(item, str) or not item for item in observed_methods)
        or any(
            not isinstance(item, int) or isinstance(item, bool) or item < 100
            for item in observed_statuses
        )
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} SIP packet observations "
            "have invalid value types"
        )
    via_observation = {
        "addresses": packet["via_sent_by_addresses"],
        "ports": packet["via_sent_by_ports"],
    }
    required_methods, required_statuses = PROXY_INTEROP_PACKET_REQUIREMENTS[scenario]
    method_assertion = packet_assertions["required-methods-observed"].get("observed")
    status_assertion = packet_assertions["required-statuses-observed"].get("observed")
    if (
        not selected_call_ids
        or packet_assertions["scenario-call-id-observed"].get("observed")
        != selected_call_ids
        or not isinstance(method_assertion, dict)
        or set(method_assertion) != {"required", "observed"}
        or not isinstance(method_assertion["required"], list)
        or not isinstance(method_assertion["observed"], list)
        or any(
            not isinstance(item, str) or not item
            for item in method_assertion["required"]
        )
        or set(method_assertion["required"]) != required_methods
        or method_assertion["observed"] != observed_methods
        or not required_methods <= set(observed_methods)
        or not isinstance(status_assertion, dict)
        or set(status_assertion) != {"required", "observed"}
        or not isinstance(status_assertion["required"], list)
        or not isinstance(status_assertion["observed"], list)
        or any(
            not isinstance(item, int) or isinstance(item, bool)
            for item in status_assertion["required"]
        )
        or set(status_assertion["required"]) != required_statuses
        or status_assertion["observed"] != observed_statuses
        or not required_statuses <= set(observed_statuses)
        or not via_observation["addresses"]
        or not via_observation["ports"]
        or packet_assertions["rvoip-via-observed"].get("observed") != via_observation
        or packet_assertions["peer-via-observed"].get("observed") != via_observation
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} SIP packet assertions "
            "disagree with scenario requirements or packet observations"
        )
    if is_sips_routing:
        sip_packet_count = packet.get("selected_sip_packet_count")
        request_uris = packet.get("observed_sips_request_uris")
        plaintext_endpoints = packet.get("plaintext_sip_endpoints")
        insecure_count = packet.get("insecure_external_sip_packet_count")
        if (
            not isinstance(sip_packet_count, int)
            or isinstance(sip_packet_count, bool)
            or sip_packet_count <= 0
            or packet_count != packet["selected_tls_packet_count"] + sip_packet_count
            or request_uris != ["sips:probe@example.test"]
            or not isinstance(plaintext_endpoints, list)
            or not plaintext_endpoints
            or any(
                not isinstance(endpoint_record, dict)
                or set(endpoint_record) != {"source", "destination"}
                or any(
                    not isinstance(endpoint_record.get(field), str)
                    or not endpoint_record[field]
                    for field in ("source", "destination")
                )
                for endpoint_record in plaintext_endpoints
            )
            or insecure_count != 0
            or packet_assertions["no-plaintext-sip-on-external-tls-ports"].get(
                "observed"
            )
            != 0
            or packet_assertions["sips-request-preserved-end-to-end"].get("observed")
            != request_uris
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} SIPS packet "
                "observations do not prove end-to-end secure routing"
            )
    if scenario == "stray-response-drop":
        true_stray = [
            item
            for item in selected_call_ids
            if isinstance(item, str) and item.startswith("true-stray-")
        ]
        if (
            len(true_stray) != 1
            or packet_assertions["one-true-stray-call-observed"].get("observed")
            != true_stray
            or not isinstance(
                packet_assertions["true-stray-arrived-at-rvoip"].get("observed"),
                int,
            )
            or isinstance(
                packet_assertions["true-stray-arrived-at-rvoip"].get("observed"),
                bool,
            )
            or packet_assertions["true-stray-arrived-at-rvoip"]["observed"] <= 0
            or packet_assertions["true-stray-had-zero-rvoip-egress"].get("observed")
            != 0
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} stray-response "
                "packet assertions are invalid"
            )


def _validate_proxy_interop_raw_evidence(
    scenario: str,
    combination: tuple[str, str, str],
    inputs: dict[str, Path],
    label: str,
) -> None:
    raw_entries = [path for path in inputs.values() if path.name == "raw-wire.json"]
    if len(raw_entries) != 1:
        raise ReportError(
            f"stateful proxy interoperability {label} is not raw-wire bound"
        )
    payload = read_json(raw_entries[0])
    transport = combination[2]
    external_key = (
        "external_peer_path_observed"
        if transport == "udp"
        else "external_peer_exercised"
    )
    if (
        not isinstance(payload, dict)
        or payload.get("schema") != PROXY_INTEROP_RAW_SCHEMAS.get(transport)
        or payload.get("scenario") != scenario
        or payload.get("status") != "PASS"
        or payload.get("transport") != transport
        or payload.get(external_key) is not True
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} raw-wire identity is invalid"
        )


def _validate_proxy_interop_scenario_result(
    scenario_dir: Path,
    scenario: str,
    combination: tuple[str, str, str],
    claimed_captures: dict[tuple[str, Any], str],
) -> dict[str, Any]:
    label = f"scenario {combination!r}/{scenario}"
    result_path = _required_proxy_interop_file(
        scenario_dir / "result.json", f"{label} result.json"
    )
    payload = read_json(result_path)
    common_keys = {
        "schema",
        "scenario",
        "status",
        "evidence_kind",
        "external_peer_exercised",
        "peer",
        "order",
        "transport",
        "assertions",
        "inputs",
    }
    evidence_kind = payload.get("evidence_kind") if isinstance(payload, dict) else None
    expected_keys = (
        common_keys | {"phases"}
        if evidence_kind == "retention-phase-snapshots"
        else common_keys
    )
    payload = _proxy_interop_exact_keys(payload, expected_keys, label)
    if (
        payload.get("schema") != PROXY_INTEROP_SCENARIO_SCHEMA
        or payload.get("status") != "PASS"
        or payload.get("external_peer_exercised") is not True
        or evidence_kind not in PROXY_INTEROP_EXTERNAL_EVIDENCE_KINDS
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} is not valid external evidence"
        )
    _validate_proxy_interop_identity(payload, scenario, combination, label)
    if scenario == "sips-routing":
        _proxy_interop_assertions_by_name(
            payload.get("assertions"),
            PROXY_INTEROP_SIPS_RESULT_ASSERTIONS,
            label,
        )
    else:
        _validate_proxy_interop_assertions(payload.get("assertions"), label)
    inputs = _validate_proxy_interop_inputs(scenario_dir, payload.get("inputs"), label)
    expected_evidence_kind = "external-sipp-and-packet-observation"
    if scenario in (
        PROXY_INTEROP_UDP_ADVANCED_SCENARIOS | PROXY_INTEROP_TCP_ADVANCED_SCENARIOS
    ):
        expected_evidence_kind = PROXY_INTEROP_RAW_EVIDENCE_KIND
    if scenario == "retention-cleanup":
        expected_evidence_kind = "retention-phase-snapshots"
    if scenario == "sips-routing":
        expected_evidence_kind = "verified-external-tls"
    if evidence_kind != expected_evidence_kind:
        raise ReportError(
            f"stateful proxy interoperability {label} has evidence kind "
            f"{evidence_kind!r}, expected {expected_evidence_kind!r}"
        )

    if (
        evidence_kind
        in {
            "external-sipp-and-packet-observation",
            PROXY_INTEROP_RAW_EVIDENCE_KIND,
        }
        or scenario == "sips-routing"
    ):
        _validate_proxy_interop_packet_evidence(
            scenario_dir,
            scenario,
            combination,
            inputs,
            label,
            claimed_captures,
        )
    if evidence_kind == PROXY_INTEROP_RAW_EVIDENCE_KIND:
        _validate_proxy_interop_raw_evidence(scenario, combination, inputs, label)

    if scenario == "retention-cleanup":
        if evidence_kind != "retention-phase-snapshots":
            raise ReportError(
                f"stateful proxy interoperability {label} lacks retention evidence"
            )
        retention_logs = [path for path in inputs.values() if path.name == "rvoip.log"]
        if len(retention_logs) != 1:
            raise ReportError(
                f"stateful proxy interoperability {label} lacks its retention log"
            )
        _validate_proxy_interop_retention(payload, label, retention_logs[0])
    elif evidence_kind == "retention-phase-snapshots":
        raise ReportError(
            f"stateful proxy interoperability {label} has unexpected retention evidence"
        )

    if scenario == "sips-routing":
        if evidence_kind != "verified-external-tls" or combination[2] != "tls":
            raise ReportError(
                f"stateful proxy interoperability {label} lacks verified TLS evidence"
            )
    elif evidence_kind == "verified-external-tls":
        raise ReportError(
            f"stateful proxy interoperability {label} has unexpected TLS evidence"
        )
    return payload


def _validate_proxy_interop_supplemental_result(
    result_path: Path,
    scenario: str,
    combination: tuple[str, str, str],
    source: dict[str, Any],
) -> None:
    label = f"supplemental scenario {combination!r}/{scenario}"
    payload = read_json(_required_proxy_interop_file(result_path, f"{label} result"))
    payload = _proxy_interop_exact_keys(
        payload,
        {
            "schema",
            "scenario",
            "status",
            "evidence_kind",
            "external_peer_exercised",
            "limitation",
            "peer_row",
            "source",
            "commands",
        },
        label,
    )
    peer_row = payload.get("peer_row")
    source_record = payload.get("source")
    commands = payload.get("commands")
    if (
        payload.get("schema") != PROXY_INTEROP_SCENARIO_SCHEMA
        or payload.get("scenario") != scenario
        or payload.get("status") != "PASS"
        or payload.get("evidence_kind") != PROXY_INTEROP_SUPPLEMENTAL_EVIDENCE_KIND
        or payload.get("external_peer_exercised") is not False
        or not isinstance(payload.get("limitation"), str)
        or not payload["limitation"].strip()
        or not isinstance(peer_row, dict)
        or set(peer_row) != {"peer", "order", "transport"}
        or (
            peer_row.get("peer"),
            peer_row.get("order"),
            peer_row.get("transport"),
        )
        != combination
        or not isinstance(source_record, dict)
        or set(source_record) != {"sha", "fingerprint_sha256"}
        or source_record.get("sha") != source.get("start_sha")
        or source_record.get("fingerprint_sha256")
        != source.get("start_fingerprint_sha256")
        or not isinstance(commands, list)
        or not commands
    ):
        raise ReportError(
            f"stateful proxy interoperability {label} contract is invalid"
        )

    scenario_dir = result_path.parent
    observed_logs: set[str] = set()
    for command in commands:
        record = _proxy_interop_exact_keys(
            command,
            {
                "argv",
                "exit_code",
                "one_exact_test_passed",
                "log",
                "log_sha256",
            },
            f"{label} command",
        )
        argv = record.get("argv")
        log = record.get("log")
        log_sha256 = record.get("log_sha256")
        if (
            not isinstance(argv, list)
            or len(argv) != 10
            or argv[:5] != ["cargo", "test", "--package", "rvoip-sip-proxy", "--test"]
            or not isinstance(argv[5], str)
            or not argv[5]
            or not isinstance(argv[6], str)
            or not argv[6]
            or argv[7:] != ["--", "--exact", "--test-threads=1"]
            or record.get("exit_code") != 0
            or isinstance(record.get("exit_code"), bool)
            or record.get("one_exact_test_passed") is not True
            or not isinstance(log, str)
            or Path(log).name != log
            or log in observed_logs
            or not isinstance(log_sha256, str)
            or re.fullmatch(r"[0-9a-f]{64}", log_sha256) is None
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} command is not exact"
            )
        log_path = _proxy_interop_relative_file(
            scenario_dir, log, f"{label} exact-test log"
        )
        log_text = log_path.read_text(encoding="utf-8", errors="replace")
        if (
            sha256_path(log_path) != log_sha256
            or re.search(r"test result: ok\.\s+1 passed;\s+0 failed", log_text) is None
        ):
            raise ReportError(
                f"stateful proxy interoperability {label} exact-test log is invalid"
            )
        observed_logs.add(log)
    expected_files = {"result.json", *observed_logs}
    actual_files: set[str] = set()
    for path in scenario_dir.iterdir():
        if not path.is_file() or path.is_symlink():
            raise ReportError(
                f"stateful proxy interoperability {label} has an undeclared entry"
            )
        actual_files.add(path.name)
    if actual_files != expected_files:
        raise ReportError(
            f"stateful proxy interoperability {label} file inventory is invalid"
        )


def _validate_proxy_interop_supplemental(
    row_dir: Path,
    combination: tuple[str, str, str],
    source: dict[str, Any],
) -> None:
    supplemental_root = row_dir / "supplemental"
    if not supplemental_root.exists():
        return
    if not supplemental_root.is_dir() or supplemental_root.is_symlink():
        raise ReportError(
            "stateful proxy interoperability supplemental evidence root is invalid: "
            f"{combination!r}"
        )
    observed: set[str] = set()
    for scenario_dir in sorted(supplemental_root.iterdir()):
        scenario = scenario_dir.name
        if (
            not scenario_dir.is_dir()
            or scenario_dir.is_symlink()
            or scenario not in PROXY_INTEROP_SCENARIOS
            or scenario in observed
        ):
            raise ReportError(
                "stateful proxy interoperability supplemental scenario is invalid: "
                f"{combination!r}/{scenario}"
            )
        observed.add(scenario)
        _validate_proxy_interop_supplemental_result(
            scenario_dir / "result.json", scenario, combination, source
        )


def _proxy_interop_tls_verifier(verifier_path: Path | None = None) -> Any:
    verifier_path = verifier_path or (
        Path(__file__).resolve().parents[2]
        / "sip-proxy/tests/interop/scripts/verify_tls_evidence.py"
    )
    if not verifier_path.is_file() or verifier_path.is_symlink():
        raise ReportError(
            "stateful proxy interoperability TLS verifier source is missing"
        )
    spec = importlib.util.spec_from_file_location(
        "rvoip_proxy_interop_tls_verifier", verifier_path
    )
    if spec is None or spec.loader is None:
        raise ReportError(
            "stateful proxy interoperability TLS verifier cannot be loaded"
        )
    module = importlib.util.module_from_spec(spec)
    previous_dont_write_bytecode = sys.dont_write_bytecode
    try:
        sys.dont_write_bytecode = True
        spec.loader.exec_module(module)
    except (ImportError, OSError, RuntimeError) as exc:
        raise ReportError(
            f"stateful proxy interoperability TLS verifier cannot be loaded: {exc}"
        ) from exc
    finally:
        sys.dont_write_bytecode = previous_dont_write_bytecode
    if not callable(getattr(module, "verify_row", None)):
        raise ReportError(
            "stateful proxy interoperability TLS verifier has no verify_row"
        )
    return module


def _validate_proxy_interop_peer_runtime(
    row_dir: Path,
    combination: tuple[str, str, str],
) -> dict[str, Any]:
    peer, _order, transport = combination
    runtime = _proxy_interop_exact_keys(
        read_json(
            _required_proxy_interop_file(
                row_dir / "peer-runtime.json", "peer runtime snapshot"
            )
        ),
        {
            "schema",
            "product",
            "container",
            "image",
            "configuration",
            "state",
            "network",
        },
        f"peer runtime {combination!r}",
    )
    container = runtime.get("container")
    image = runtime.get("image")
    configuration = runtime.get("configuration")
    state = runtime.get("state")
    network = runtime.get("network")
    expected_reference = (
        PROXY_INTEROP_OPENSIPS_TLS_REFERENCE
        if peer == "opensips" and transport == "tls"
        else PROXY_INTEROP_IMAGES[peer]
    )
    allowed_state = {
        "status",
        "running",
        "paused",
        "restarting",
        "oom_killed",
        "dead",
        "exit_code",
        "started_at",
        "finished_at",
        "health_status",
    }
    if (
        runtime.get("schema") != PROXY_INTEROP_PEER_RUNTIME_SCHEMA
        or runtime.get("product") != peer
        or not isinstance(container, dict)
        or set(container) != {"name", "id", "created", "platform"}
        or any(
            not isinstance(container.get(field), str) or not container[field]
            for field in container
        )
        or not container["platform"].startswith("linux")
        or not isinstance(image, dict)
        or set(image) != {"id", "reference"}
        or re.fullmatch(r"sha256:[0-9a-f]{64}", str(image.get("id", ""))) is None
        or image.get("reference") != expected_reference
        or not isinstance(configuration, dict)
        or set(configuration)
        != {
            "network_mode",
            "published_ports",
            "exposed_ports",
            "restart_policy",
        }
        or not isinstance(configuration.get("network_mode"), str)
        or not isinstance(configuration.get("published_ports"), dict)
        or not isinstance(configuration.get("exposed_ports"), list)
        or any(
            not isinstance(port, str) for port in configuration.get("exposed_ports", [])
        )
        or not isinstance(configuration.get("restart_policy"), dict)
        or set(configuration.get("restart_policy", {}))
        != {"name", "maximum_retry_count"}
        or not isinstance(configuration["restart_policy"].get("name"), str)
        or not isinstance(
            configuration["restart_policy"].get("maximum_retry_count"), int
        )
        or isinstance(configuration["restart_policy"].get("maximum_retry_count"), bool)
        or not isinstance(state, dict)
        or not {
            "status",
            "running",
            "paused",
            "restarting",
            "oom_killed",
            "dead",
            "exit_code",
        }
        <= set(state)
        or not set(state) <= allowed_state
        or state.get("status") != "running"
        or state.get("running") is not True
        or state.get("paused") is not False
        or state.get("restarting") is not False
        or state.get("oom_killed") is not False
        or state.get("dead") is not False
        or not isinstance(state.get("exit_code"), int)
        or isinstance(state.get("exit_code"), bool)
        or state.get("exit_code") != 0
        or not isinstance(network, dict)
        or set(network) != {"published_ports", "networks"}
        or not isinstance(network.get("published_ports"), dict)
        or not isinstance(network.get("networks"), dict)
        or not network.get("networks")
        or any(
            not isinstance(name, str) or not name or not isinstance(details, dict)
            for name, details in network.get("networks", {}).items()
        )
    ):
        raise ReportError(
            f"stateful proxy interoperability peer runtime is invalid: {combination!r}"
        )
    return runtime


def _validate_proxy_interop_opensips_provenance(
    row_dir: Path,
    runtime: dict[str, Any],
    combination: tuple[str, str, str],
) -> Path:
    provenance_path = _required_proxy_interop_file(
        row_dir / "opensips-tls-image-provenance.json",
        "OpenSIPS TLS image provenance",
    )
    provenance = _proxy_interop_exact_keys(
        read_json(provenance_path),
        {"schema", "result", "image", "dockerfile", "packages", "modules"},
        f"OpenSIPS TLS provenance {combination!r}",
    )
    image = provenance.get("image")
    dockerfile = provenance.get("dockerfile")
    workspace_dockerfile = (
        Path(__file__).resolve().parents[4] / PROXY_INTEROP_OPENSIPS_TLS_DOCKERFILE
    )
    if (
        provenance.get("schema") != PROXY_INTEROP_OPENSIPS_TLS_PROVENANCE_SCHEMA
        or provenance.get("result") != "PASS"
        or not isinstance(image, dict)
        or set(image) != {"reference", "id", "platform", "base_digest"}
        or image.get("reference") != PROXY_INTEROP_OPENSIPS_TLS_REFERENCE
        or image.get("id") != runtime.get("image", {}).get("id")
        or image.get("platform") != "linux/amd64"
        or image.get("base_digest") != PROXY_INTEROP_IMAGES["opensips"].split("@", 1)[1]
        or not isinstance(dockerfile, dict)
        or set(dockerfile) != {"relative_path", "sha256"}
        or dockerfile.get("relative_path") != PROXY_INTEROP_OPENSIPS_TLS_DOCKERFILE
        or dockerfile.get("sha256") != PROXY_INTEROP_OPENSIPS_TLS_DOCKERFILE_SHA256
        or not workspace_dockerfile.is_file()
        or workspace_dockerfile.is_symlink()
        or sha256_path(workspace_dockerfile)
        != PROXY_INTEROP_OPENSIPS_TLS_DOCKERFILE_SHA256
        or provenance.get("packages") != PROXY_INTEROP_OPENSIPS_TLS_PACKAGES
        or provenance.get("modules") != PROXY_INTEROP_OPENSIPS_TLS_MODULES
    ):
        raise ReportError(
            "stateful proxy interoperability OpenSIPS TLS provenance does not "
            f"match the reviewed image: {combination!r}"
        )
    return provenance_path


def _validate_proxy_interop_tls_packet_aggregate(
    result: dict[str, Any],
    combination: tuple[str, str, str],
) -> dict[str, Any]:
    peer, _order, _transport = combination
    aggregate = _proxy_interop_exact_keys(
        result.get("packet_aggregate"),
        {
            "scenarios",
            "expected_sni",
            "observed_sni",
            "expected_leaf_certificate_sha256",
            "observed_certificate_sha256",
            "capture_count",
        },
        f"TLS packet aggregate {combination!r}",
    )
    certificates = result.get("certificates")
    if not isinstance(certificates, dict):
        raise ReportError(
            f"stateful proxy interoperability TLS certificates are invalid: "
            f"{combination!r}"
        )
    expected_sni = sorted(
        {
            "rvoip.proxy.test",
            f"{peer}.proxy.test",
            "sipp.proxy.test",
        }
    )
    try:
        expected_leaf_hashes = sorted(
            certificates[name]["der_sha256"] for name in ("rvoip", "peer", "sipp")
        )
    except (KeyError, TypeError):
        raise ReportError(
            f"stateful proxy interoperability TLS leaf certificates are invalid: "
            f"{combination!r}"
        ) from None
    scenarios = aggregate.get("scenarios")
    expected_scenarios = set(PROXY_INTEROP_TLS_PACKET_SCENARIOS)
    if (
        aggregate.get("expected_sni") != expected_sni
        or aggregate.get("observed_sni") != expected_sni
        or aggregate.get("expected_leaf_certificate_sha256") != expected_leaf_hashes
        or not isinstance(aggregate.get("observed_certificate_sha256"), list)
        or not set(expected_leaf_hashes)
        <= set(aggregate["observed_certificate_sha256"])
        or not isinstance(aggregate.get("capture_count"), int)
        or isinstance(aggregate.get("capture_count"), bool)
        or aggregate["capture_count"] <= 0
        or not isinstance(scenarios, dict)
        or set(scenarios) != expected_scenarios
    ):
        raise ReportError(
            f"stateful proxy interoperability TLS packet aggregate is invalid: "
            f"{combination!r}"
        )

    observed_sni: set[str] = set()
    observed_certificates: set[str] = set()
    observed_captures: set[str] = set()
    for scenario in PROXY_INTEROP_TLS_PACKET_SCENARIOS:
        record = _proxy_interop_exact_keys(
            scenarios[scenario],
            {
                "packet_evidence_sha256",
                "observed_sni",
                "observed_certificate_sha256",
                "captures",
            },
            f"TLS packet aggregate {combination!r}/{scenario}",
        )
        packet_hash = record.get("packet_evidence_sha256")
        scenario_sni = record.get("observed_sni")
        scenario_certificates = record.get("observed_certificate_sha256")
        captures = record.get("captures")
        if (
            not isinstance(packet_hash, str)
            or re.fullmatch(r"[0-9a-f]{64}", packet_hash) is None
            or not isinstance(scenario_sni, list)
            or scenario_sni != sorted(set(scenario_sni))
            or any(
                not isinstance(value, str) or value not in expected_sni
                for value in scenario_sni
            )
            or not isinstance(scenario_certificates, list)
            or scenario_certificates != sorted(set(scenario_certificates))
            or any(
                not isinstance(value, str)
                or re.fullmatch(r"[0-9a-f]{64}", value) is None
                for value in scenario_certificates
            )
            or not isinstance(captures, list)
            or not captures
        ):
            raise ReportError(
                "stateful proxy interoperability TLS packet aggregate scenario "
                f"is invalid: {combination!r}/{scenario}"
            )
        observed_sni.update(scenario_sni)
        observed_certificates.update(scenario_certificates)
        for raw_capture in captures:
            capture = _proxy_interop_exact_keys(
                raw_capture,
                {"filename", "sha256", "bytes"},
                f"TLS packet aggregate capture {combination!r}/{scenario}",
            )
            filename = capture.get("filename")
            if (
                not isinstance(filename, str)
                or Path(filename).name != filename
                or not filename.startswith(f"{scenario}--")
                or not filename.endswith(".pcap")
                or filename in observed_captures
                or not isinstance(capture.get("sha256"), str)
                or re.fullmatch(r"[0-9a-f]{64}", capture["sha256"]) is None
                or not isinstance(capture.get("bytes"), int)
                or isinstance(capture.get("bytes"), bool)
                or capture["bytes"] <= 24
            ):
                raise ReportError(
                    "stateful proxy interoperability TLS packet aggregate "
                    f"capture is invalid: {combination!r}/{scenario}"
                )
            observed_captures.add(filename)
    if (
        sorted(observed_sni) != aggregate["observed_sni"]
        or sorted(observed_certificates) != aggregate["observed_certificate_sha256"]
        or len(observed_captures) != aggregate["capture_count"]
    ):
        raise ReportError(
            "stateful proxy interoperability TLS packet aggregate disagrees "
            f"with its scenario records: {combination!r}"
        )
    return {
        "expected_sni": expected_sni,
        "observed_sni": aggregate["observed_sni"],
        "expected_leaf_certificate_sha256": expected_leaf_hashes,
        "observed_certificate_sha256": aggregate["observed_certificate_sha256"],
        "capture_count": aggregate["capture_count"],
    }


def _validate_proxy_interop_tls(
    row_dir: Path,
    scenario_dir: Path,
    combination: tuple[str, str, str],
    scenario_payload: dict[str, Any],
    runtime: dict[str, Any],
) -> dict[str, Any]:
    peer, order, transport = combination
    if transport != "tls":
        raise ReportError(
            f"stateful proxy interoperability TLS row is invalid: {combination!r}"
        )
    stored_path = _required_proxy_interop_file(
        row_dir / "tls-verifier-result.json", "TLS verifier result"
    )
    stored = read_json(stored_path)
    if (
        not isinstance(stored, dict)
        or stored.get("schema") != PROXY_INTEROP_TLS_RESULT_SCHEMA
        or stored.get("result") != "PASS"
        or (
            stored.get("peer"),
            stored.get("order"),
            stored.get("transport"),
        )
        != combination
    ):
        raise ReportError(
            f"stateful proxy interoperability TLS result is invalid: {combination!r}"
        )
    packet_aggregate = _validate_proxy_interop_tls_packet_aggregate(stored, combination)
    inputs = scenario_payload.get("inputs")
    assert isinstance(inputs, dict)
    input_paths = {
        Path(relative).name: _proxy_interop_relative_file(
            scenario_dir,
            relative,
            f"TLS scenario {combination!r} input",
        )
        for relative in inputs
    }
    verifier_copy = input_paths.get("verify_tls_evidence.py")
    workspace_verifier = (
        Path(__file__).resolve().parents[2]
        / "sip-proxy/tests/interop/scripts/verify_tls_evidence.py"
    )
    if (
        verifier_copy is None
        or not workspace_verifier.is_file()
        or workspace_verifier.is_symlink()
        or sha256_path(verifier_copy) != sha256_path(workspace_verifier)
    ):
        raise ReportError(
            f"stateful proxy interoperability TLS verifier source is not bound: "
            f"{combination!r}"
        )
    verifier = _proxy_interop_tls_verifier(workspace_verifier)
    try:
        recomputed = verifier.verify_row(row_dir, peer, order)
    except Exception as exc:
        raise ReportError(
            f"stateful proxy interoperability TLS verification failed: "
            f"{combination!r}: {exc}"
        ) from exc
    if canonical_json(stored) != canonical_json(recomputed):
        raise ReportError(
            f"stateful proxy interoperability TLS verifier result drifted: "
            f"{combination!r}"
        )
    if (
        _validate_proxy_interop_tls_packet_aggregate(recomputed, combination)
        != packet_aggregate
    ):
        raise ReportError(
            f"stateful proxy interoperability TLS packet aggregate drifted: "
            f"{combination!r}"
        )

    required_logs = {
        "rvoip.log",
        "peer.log",
        "tls-rvoip-to-peer-positive.log",
        "tls-peer-to-rvoip-positive.log",
        "tls-rvoip-to-peer-wrong-name.log",
        "tls-rvoip-to-peer-wrong-ca.log",
        "tls-peer-to-rvoip-wrong-name.log",
        "tls-peer-to-rvoip-wrong-ca.log",
        "tls-peer-rejects-untrusted-client.log",
        "tls-rvoip-rejects-untrusted-client.log",
        "tls-boundary-client.log",
        "tls-boundary-server.log",
    }
    required_logs.add(f"tls-{peer}-outbound-boundary.log")
    required_live_sips = {
        "packet-evidence.json",
        "uac-messages.log",
        "uas-messages.log",
        "uac-stats.csv",
        "uas-stats.csv",
    }
    input_by_name = {Path(relative).name: record for relative, record in inputs.items()}
    runtime_path = _required_proxy_interop_file(
        row_dir / "peer-runtime.json", "peer runtime snapshot"
    )
    if (
        "tls-verifier-result.json" not in input_by_name
        or "verify_tls_evidence.py" not in input_by_name
        or "peer-runtime.json" not in input_by_name
        or not required_logs <= set(input_by_name)
        or not required_live_sips <= set(input_by_name)
        or input_by_name["tls-verifier-result.json"].get("sha256")
        != sha256_path(stored_path)
        or input_by_name["peer-runtime.json"].get("sha256") != sha256_path(runtime_path)
    ):
        raise ReportError(
            f"stateful proxy interoperability TLS scenario is not verifier-bound: "
            f"{combination!r}"
        )
    source_hashes = stored.get("source_files_sha256")
    if not isinstance(source_hashes, dict):
        raise ReportError(
            f"stateful proxy interoperability TLS source hashes are missing: "
            f"{combination!r}"
        )
    for basename in required_logs:
        expected = source_hashes.get(basename)
        if (
            not isinstance(expected, str)
            or input_by_name[basename].get("sha256") != expected
        ):
            raise ReportError(
                "stateful proxy interoperability TLS raw evidence is not bound: "
                f"{combination!r}/{basename}"
            )
    if peer == "opensips":
        provenance_name = "opensips-tls-image-provenance.json"
        provenance_path = _validate_proxy_interop_opensips_provenance(
            row_dir, runtime, combination
        )
        if (
            provenance_name not in input_by_name
            or input_by_name[provenance_name].get("sha256")
            != sha256_path(provenance_path)
            or source_hashes.get(provenance_name) != sha256_path(provenance_path)
        ):
            raise ReportError(
                "stateful proxy interoperability OpenSIPS TLS provenance is not "
                f"scenario-bound: {combination!r}"
            )
    return packet_aggregate


def _validate_proxy_interop_runtime_snapshot(value: Any, label: str) -> dict[str, Any]:
    snapshot = _proxy_interop_exact_keys(
        value,
        {
            "schema",
            "kind",
            "captured_at_utc",
            "ports",
            "collectors",
            "docker",
            "listeners",
        },
        f"runtime {label} snapshot",
    )
    ports = snapshot.get("ports")
    collectors = snapshot.get("collectors")
    docker = snapshot.get("docker")
    listeners = snapshot.get("listeners")
    if (
        snapshot.get("schema") != PROXY_INTEROP_RUNTIME_STATE_SCHEMA
        or snapshot.get("kind") != "snapshot"
        or not isinstance(snapshot.get("captured_at_utc"), str)
        or re.fullmatch(
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z",
            snapshot["captured_at_utc"],
        )
        is None
        or not isinstance(ports, list)
        or not ports
        or ports != sorted(set(ports))
        or any(
            not isinstance(port, int)
            or isinstance(port, bool)
            or port < 1
            or port > 65535
            for port in ports
        )
        or collectors
        not in (
            {"docker": "docker-list-json", "listeners": "lsof"},
            {"docker": "docker-list-json", "listeners": "ss"},
        )
        or not isinstance(docker, dict)
        or set(docker) != {"containers", "networks", "volumes"}
        or not isinstance(listeners, list)
    ):
        raise ReportError(
            f"stateful proxy interoperability runtime {label} snapshot is invalid"
        )

    record_contracts = {
        "containers": {"id", "name", "image", "state"},
        "networks": {"id", "name", "driver", "scope"},
        "volumes": {"name", "driver", "scope"},
    }
    identity_fields = {"containers": "id", "networks": "id", "volumes": "name"}
    for category, fields in record_contracts.items():
        records = docker.get(category)
        if not isinstance(records, list):
            raise ReportError(
                f"stateful proxy interoperability runtime {label} "
                f"{category} inventory is invalid"
            )
        identities: set[str] = set()
        for raw_record in records:
            record = _proxy_interop_exact_keys(
                raw_record,
                fields,
                f"runtime {label} {category} record",
            )
            if any(
                not isinstance(record.get(field), str) or not record[field]
                for field in fields
            ):
                raise ReportError(
                    f"stateful proxy interoperability runtime {label} "
                    f"{category} record is incomplete"
                )
            identity = record[identity_fields[category]]
            if identity in identities:
                raise ReportError(
                    f"stateful proxy interoperability runtime {label} "
                    f"{category} identity is duplicated"
                )
            identities.add(identity)

    listener_identities: set[tuple[Any, ...]] = set()
    for raw_record in listeners:
        record = _proxy_interop_exact_keys(
            raw_record,
            {"transport", "port", "pid", "process", "endpoint"},
            f"runtime {label} listener",
        )
        identity = (
            record.get("transport"),
            record.get("port"),
            record.get("pid"),
            record.get("process"),
            record.get("endpoint"),
        )
        if (
            record.get("transport") not in {"tcp", "udp"}
            or record.get("port") not in ports
            or not all(isinstance(value, str) for value in identity[2:])
            or not record.get("endpoint")
            or identity in listener_identities
        ):
            raise ReportError(
                f"stateful proxy interoperability runtime {label} listener is invalid"
            )
        listener_identities.add(identity)
    return snapshot


def _validate_proxy_interop_runtime_state(proxy_root: Path) -> None:
    start = _validate_proxy_interop_runtime_snapshot(
        read_json(
            _required_proxy_interop_file(
                proxy_root / "runtime-state-start.json", "runtime start snapshot"
            )
        ),
        "start",
    )
    end = _validate_proxy_interop_runtime_snapshot(
        read_json(
            _required_proxy_interop_file(
                proxy_root / "runtime-state-end.json", "runtime end snapshot"
            )
        ),
        "end",
    )
    check = _proxy_interop_exact_keys(
        read_json(
            _required_proxy_interop_file(
                proxy_root / "runtime-state-check.json", "runtime state check"
            )
        ),
        {
            "schema",
            "kind",
            "compared_at_utc",
            "ports",
            "clean",
            "preexisting_state_preserved",
            "no_added_leftovers",
            "added_leftovers",
            "removed_preexisting",
            "changed_preexisting",
            "differences",
        },
        "runtime state check",
    )
    differences = check.get("differences")
    if (
        start.get("ports") != end.get("ports")
        or check.get("ports") != start.get("ports")
        or start.get("docker") != end.get("docker")
        or start.get("listeners") != end.get("listeners")
        or check.get("schema") != PROXY_INTEROP_RUNTIME_STATE_SCHEMA
        or check.get("kind") != "comparison"
        or not isinstance(check.get("compared_at_utc"), str)
        or re.fullmatch(
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z",
            check["compared_at_utc"],
        )
        is None
        or check.get("clean") is not True
        or check.get("preexisting_state_preserved") is not True
        or check.get("no_added_leftovers") is not True
        or check.get("added_leftovers") != {}
        or check.get("removed_preexisting") != {}
        or check.get("changed_preexisting") != {}
        or not isinstance(differences, dict)
        or set(differences) != {"containers", "networks", "volumes", "listeners"}
    ):
        raise ReportError(
            "stateful proxy interoperability runtime state did not converge"
        )
    for category, raw_difference in differences.items():
        difference = _proxy_interop_exact_keys(
            raw_difference,
            {"added", "removed", "changed"},
            f"runtime state {category} difference",
        )
        if any(difference[field] != [] for field in difference):
            raise ReportError(
                "stateful proxy interoperability runtime difference is nonempty: "
                f"{category}"
            )


def proxy_interop_evidence_paths(report_root: Path) -> list[str]:
    proxy_root = report_root / "proxy-interop"
    if not proxy_root.is_dir() or proxy_root.is_symlink():
        raise ReportError("stateful proxy interoperability evidence root is missing")
    paths: list[str] = []
    for path in sorted(proxy_root.rglob("*")):
        if path.is_symlink():
            raise ReportError(
                f"stateful proxy interoperability evidence may not be a symlink: {path}"
            )
        if path.is_file():
            lowered = path.name.lower()
            if (
                lowered in PROXY_INTEROP_PROHIBITED_RAW_FILES
                or lowered.endswith((".key", ".key.pem", ".p12", ".pfx"))
                or "private-key" in lowered
                or (
                    path.stat().st_size <= 2 * 1024 * 1024
                    and PROXY_INTEROP_PRIVATE_KEY_PEM.search(path.read_bytes())
                    is not None
                )
            ):
                raise ReportError(
                    "stateful proxy interoperability evidence contains prohibited "
                    f"sensitive or raw state: {path}"
                )
            paths.append(path.relative_to(report_root).as_posix())
    if not paths:
        raise ReportError("stateful proxy interoperability evidence root is empty")
    return paths


def _validate_proxy_interop_build_evidence(
    proxy_root: Path, summary: dict[str, Any]
) -> None:
    _required_proxy_interop_file(proxy_root / "cargo-build.log", "Cargo build log")
    command_text = (
        _required_proxy_interop_file(
            proxy_root / "cargo-build-command.txt", "Cargo build command"
        )
        .read_text(encoding="utf-8", errors="strict")
        .strip()
    )
    binary_sha = (
        _required_proxy_interop_file(
            proxy_root / "proxy-binary.sha256", "proxy binary digest"
        )
        .read_text(encoding="utf-8", errors="strict")
        .strip()
    )
    if re.fullmatch(r"[0-9a-f]{64}", binary_sha) is None:
        raise ReportError(
            "stateful proxy interoperability proxy binary digest is invalid"
        )

    binary_path = (
        _required_proxy_interop_file(
            proxy_root / "proxy-binary.path", "proxy binary path"
        )
        .read_text(encoding="utf-8", errors="strict")
        .strip()
    )
    if binary_path != PROXY_INTEROP_BINARY_PATH:
        raise ReportError(
            "stateful proxy interoperability proxy binary path is not exact"
        )

    check_path = _required_proxy_interop_file(
        proxy_root / "proxy-binary-check.txt", "proxy binary revalidation"
    )
    check: dict[str, str] = {}
    for raw in check_path.read_text(encoding="utf-8", errors="strict").splitlines():
        if not raw or "=" not in raw:
            raise ReportError(
                "stateful proxy interoperability proxy binary revalidation is invalid"
            )
        key, value = raw.split("=", 1)
        if key in check:
            raise ReportError(
                "stateful proxy interoperability proxy binary revalidation "
                f"duplicates {key}"
            )
        check[key] = value
    if (
        set(check) != {"start_sha256", "end_sha256", "unchanged"}
        or check["start_sha256"] != binary_sha
        or check["end_sha256"] != binary_sha
        or check["unchanged"] != "true"
    ):
        raise ReportError(
            "stateful proxy interoperability proxy binary changed during the run"
        )

    environment_path = _required_proxy_interop_file(
        proxy_root / "environment.txt", "environment"
    )
    environment: dict[str, str] = {}
    for raw in environment_path.read_text(
        encoding="utf-8", errors="strict"
    ).splitlines():
        if not raw or ": " not in raw:
            raise ReportError(
                "stateful proxy interoperability environment provenance is invalid"
            )
        key, value = raw.split(": ", 1)
        if not key or not value or key in environment:
            raise ReportError(
                "stateful proxy interoperability environment provenance is invalid"
            )
        environment[key] = value
    missing_tools = PROXY_INTEROP_ENVIRONMENT_TOOL_KEYS - set(environment)
    if missing_tools:
        raise ReportError(
            "stateful proxy interoperability tool provenance is incomplete: "
            f"{sorted(missing_tools)}"
        )
    for key in PROXY_INTEROP_ENVIRONMENT_TOOL_KEYS:
        if key.endswith("_path") and not Path(environment[key]).is_absolute():
            raise ReportError(
                f"stateful proxy interoperability tool path is not absolute: {key}"
            )
    try:
        command = shlex.split(command_text)
    except ValueError as exc:
        raise ReportError(
            "stateful proxy interoperability Cargo build command is invalid"
        ) from exc
    if (
        len(command) != 9
        or command[0] != environment["cargo_path"]
        or command[1:3] != ["build", "--manifest-path"]
        or not Path(command[3]).is_absolute()
        or Path(command[3]).name != "Cargo.toml"
        or command[4:]
        != [
            "--release",
            "--package",
            "rvoip-sip-proxy",
            "--example",
            "stateful_proxy_interop",
        ]
    ):
        raise ReportError(
            "stateful proxy interoperability Cargo build command is not exact"
        )
    source = summary["source"]
    configuration = summary["configuration"]
    expected_environment = {
        "source_sha": source["start_sha"],
        "source_fingerprint": source["start_fingerprint_sha256"],
        "source_dirty": "false",
        "peers": "kamailio opensips",
        "orders": "rvoip-first peer-first",
        "transports": "udp tcp tls",
        "kamailio_image": configuration["kamailio_image"],
        "kamailio_platform": configuration["kamailio_platform"],
        "opensips_image": configuration["opensips_image"],
        "opensips_platform": configuration["opensips_platform"],
        "retention_drain_seconds": str(configuration["retention_drain_seconds"]),
    }
    if any(
        environment.get(key) != value for key, value in expected_environment.items()
    ):
        raise ReportError(
            "stateful proxy interoperability environment disagrees with its summary"
        )


def _validate_proxy_interop_peer_version(
    row_dir: Path, peer: str, combination: tuple[Any, Any, Any]
) -> None:
    version = _required_proxy_interop_file(
        row_dir / "peer-version.txt", "peer version"
    ).read_text(encoding="utf-8", errors="replace")
    if PROXY_INTEROP_PEER_VERSION_RE[peer].search(version) is None:
        raise ReportError(
            f"stateful proxy interoperability peer version drifted: {combination!r}"
        )


def validate_proxy_interop_result(report_root: Path) -> dict[str, Any]:
    proxy_root = report_root / "proxy-interop"
    proxy_interop_evidence_paths(report_root)
    summary = read_json(proxy_root / "summary.json")
    if summary.get("schema") != "rvoip-sip-proxy-interop-v1":
        raise ReportError("stateful proxy interoperability summary schema is invalid")
    result = summary.get("result")
    source = summary.get("source")
    configuration = summary.get("configuration")
    rows = summary.get("rows")
    if not all(
        isinstance(value, dict) for value in (result, source, configuration)
    ) or not isinstance(rows, list):
        raise ReportError("stateful proxy interoperability summary is incomplete")

    if (
        result.get("status") != "PASS"
        or result.get("passed_rows") != 12
        or result.get("failed_rows") != 0
        or result.get("gate_failures") != 0
    ):
        raise ReportError("stateful proxy interoperability result is not a 12-row PASS")
    if (
        source.get("dirty_at_start") is not False
        or source.get("unchanged") is not True
        or source.get("start_sha") != source.get("end_sha")
        or source.get("start_fingerprint_sha256")
        != source.get("end_fingerprint_sha256")
        or not re.fullmatch(r"[0-9a-f]{40}", str(source.get("start_sha", "")))
        or not re.fullmatch(
            r"[0-9a-f]{64}", str(source.get("start_fingerprint_sha256", ""))
        )
    ):
        raise ReportError(
            "stateful proxy interoperability source is not clean and unchanged"
        )

    for peer, expected_image in PROXY_INTEROP_IMAGES.items():
        if configuration.get(f"{peer}_image") != expected_image:
            raise ReportError(f"stateful proxy interoperability {peer} image drifted")
        if configuration.get(f"{peer}_platform") != "linux/amd64":
            raise ReportError(
                f"stateful proxy interoperability {peer} platform drifted"
            )
    drain = configuration.get("retention_drain_seconds")
    if not isinstance(drain, int) or isinstance(drain, bool) or drain < 130:
        raise ReportError(
            "stateful proxy interoperability retention drain is below 130 seconds"
        )
    _validate_proxy_interop_build_evidence(proxy_root, summary)

    expected_combinations = {
        (peer, order, transport)
        for peer in PROXY_INTEROP_PEERS
        for order in PROXY_INTEROP_ORDERS
        for transport in PROXY_INTEROP_TRANSPORTS
    }
    observed_combinations: set[tuple[str, str, str]] = set()
    observed_scenarios: set[str] = set()
    observed_scenarios_by_peer: defaultdict[str, set[str]] = defaultdict(set)
    row_artifacts: list[str] = []
    tls_packet_aggregates: dict[str, dict[str, Any]] = {}
    claimed_captures: dict[tuple[str, Any], str] = {}
    for row in rows:
        if not isinstance(row, dict):
            raise ReportError("stateful proxy interoperability row is not an object")
        combination = (
            row.get("peer"),
            row.get("order"),
            row.get("transport"),
        )
        if combination not in expected_combinations:
            raise ReportError(
                f"unexpected stateful proxy interoperability row: {combination!r}"
            )
        if combination in observed_combinations:
            raise ReportError(
                f"duplicate stateful proxy interoperability row: {combination!r}"
            )
        observed_combinations.add(combination)
        if row.get("status") != "PASS":
            raise ReportError(
                f"stateful proxy interoperability row did not pass: {combination!r}"
            )
        duration = row.get("duration_seconds")
        if (
            not isinstance(duration, (str, int))
            or isinstance(duration, bool)
            or not str(duration).isdigit()
            or int(duration) < drain
        ):
            raise ReportError(
                f"stateful proxy interoperability row lacks a duration: {combination!r}"
            )
        scenarios = row.get("scenarios")
        if (
            not isinstance(scenarios, list)
            or not scenarios
            or not all(isinstance(item, str) and item for item in scenarios)
            or len(scenarios) != len(set(scenarios))
        ):
            raise ReportError(
                f"stateful proxy interoperability row lacks scenario coverage: {combination!r}"
            )
        scenario_set = set(scenarios)
        expected_row_scenarios = _expected_proxy_interop_scenarios(combination)
        missing_row_scenarios = expected_row_scenarios - scenario_set
        misplaced_row_scenarios = scenario_set - expected_row_scenarios
        if missing_row_scenarios or misplaced_row_scenarios:
            raise ReportError(
                "stateful proxy interoperability row scenario contract failed: "
                f"{combination!r}; missing={sorted(missing_row_scenarios)}, "
                f"misplaced={sorted(misplaced_row_scenarios)}"
            )
        observed_scenarios.update(scenario_set)
        observed_scenarios_by_peer[str(row.get("peer"))].update(scenario_set)
        row_artifact = row.get("artifact")
        row_dir = _proxy_interop_artifact_dir(report_root, row_artifact)
        row_artifacts.append(str(row_artifact))
        peer_runtime = _validate_proxy_interop_peer_runtime(row_dir, combination)
        scenarios_root = row_dir / "scenarios"
        if not scenarios_root.is_dir() or scenarios_root.is_symlink():
            raise ReportError(
                f"stateful proxy interoperability scenarios root is missing: "
                f"{combination!r}"
            )
        actual_scenario_dirs: set[str] = set()
        for path in scenarios_root.iterdir():
            if not path.is_dir() or path.is_symlink():
                raise ReportError(
                    "stateful proxy interoperability scenarios root has an "
                    f"undeclared entry: {combination!r}/{path.name}"
                )
            actual_scenario_dirs.add(path.name)
        if actual_scenario_dirs != scenario_set:
            raise ReportError(
                "stateful proxy interoperability scenario inventory disagrees "
                f"with its row: {combination!r}"
            )
        scenario_results: dict[str, dict[str, Any]] = {}
        for scenario in scenarios:
            scenario_dir = row_dir / "scenarios" / scenario
            if not scenario_dir.is_dir() or scenario_dir.is_symlink():
                raise ReportError(
                    "stateful proxy interoperability scenario evidence is missing: "
                    f"{combination!r}/{scenario}"
                )
            scenario_results[scenario] = _validate_proxy_interop_scenario_result(
                scenario_dir,
                scenario,
                combination,
                claimed_captures,
            )
        _validate_proxy_interop_supplemental(row_dir, combination, source)
        _validate_proxy_interop_peer_version(row_dir, str(row.get("peer")), combination)
        retention = _required_proxy_interop_file(
            row_dir / "retention-check.txt", "retention check"
        ).read_text(encoding="utf-8", errors="replace")
        if "nonzero={}" not in retention:
            raise ReportError(
                f"stateful proxy interoperability retention did not converge: {combination!r}"
            )
        pcaps = [
            path
            for path in row_dir.glob("*.pcap")
            if path.is_file() and not path.is_symlink() and path.stat().st_size > 0
        ]
        if not pcaps:
            raise ReportError(
                f"stateful proxy interoperability packet capture is missing: {combination!r}"
            )
        if row.get("transport") == "tls":
            tls_packet_aggregates[str(row_artifact)] = _validate_proxy_interop_tls(
                row_dir,
                row_dir / "scenarios/sips-routing",
                combination,
                scenario_results["sips-routing"],
                peer_runtime,
            )

    if observed_combinations != expected_combinations:
        missing = sorted(expected_combinations - observed_combinations)
        raise ReportError(
            f"stateful proxy interoperability matrix is incomplete; missing={missing}"
        )
    if observed_scenarios != PROXY_INTEROP_SCENARIOS:
        raise ReportError(
            "stateful proxy interoperability scenario coverage is incomplete; "
            f"missing={sorted(PROXY_INTEROP_SCENARIOS - observed_scenarios)}, "
            f"unknown={sorted(observed_scenarios - PROXY_INTEROP_SCENARIOS)}"
        )
    for peer in sorted(PROXY_INTEROP_PEERS):
        peer_scenarios = observed_scenarios_by_peer[peer]
        if peer_scenarios != PROXY_INTEROP_SCENARIOS:
            raise ReportError(
                "stateful proxy peer scenario coverage is incomplete; "
                f"peer={peer!r}, "
                f"missing={sorted(PROXY_INTEROP_SCENARIOS - peer_scenarios)}, "
                f"unknown={sorted(peer_scenarios - PROXY_INTEROP_SCENARIOS)}"
            )

    with (proxy_root / "matrix.tsv").open(newline="", encoding="utf-8") as stream:
        matrix_reader = csv.DictReader(stream, delimiter="\t")
        matrix_rows = list(matrix_reader)
    if matrix_reader.fieldnames != [
        "peer",
        "order",
        "transport",
        "status",
        "duration_seconds",
        "artifact",
        "scenarios",
    ]:
        raise ReportError("stateful proxy interoperability TSV schema is invalid")
    matrix_combinations = {
        (row.get("peer"), row.get("order"), row.get("transport"))
        for row in matrix_rows
        if row.get("status") == "PASS"
    }
    if (
        len(matrix_rows) != 12
        or matrix_combinations != expected_combinations
        or any(row.get("status") != "PASS" for row in matrix_rows)
    ):
        raise ReportError(
            "stateful proxy interoperability TSV disagrees with the machine summary"
        )
    summary_by_combination = {
        (row["peer"], row["order"], row["transport"]): row for row in rows
    }
    for matrix_row in matrix_rows:
        combination = (
            matrix_row.get("peer"),
            matrix_row.get("order"),
            matrix_row.get("transport"),
        )
        summary_row = summary_by_combination[combination]
        if (
            matrix_row.get("status") != summary_row.get("status")
            or matrix_row.get("duration_seconds")
            != str(summary_row.get("duration_seconds"))
            or matrix_row.get("artifact") != summary_row.get("artifact")
            or [
                item for item in str(matrix_row.get("scenarios", "")).split(",") if item
            ]
            != summary_row.get("scenarios")
        ):
            raise ReportError(
                "stateful proxy interoperability TSV row drifted from its "
                f"machine summary: {combination!r}"
            )

    _validate_proxy_interop_runtime_state(proxy_root)

    return {
        "rows": len(rows),
        "peers": sorted(PROXY_INTEROP_PEERS),
        "orders": sorted(PROXY_INTEROP_ORDERS),
        "transports": sorted(PROXY_INTEROP_TRANSPORTS),
        "scenarios": sorted(observed_scenarios),
        "scenarios_by_peer": {
            peer: sorted(observed_scenarios_by_peer[peer])
            for peer in sorted(PROXY_INTEROP_PEERS)
        },
        "row_artifacts": sorted(row_artifacts),
        "tls_packet_aggregates": {
            artifact: tls_packet_aggregates[artifact]
            for artifact in sorted(tls_packet_aggregates)
        },
        "retention_drain_seconds": drain,
        "source_clean": True,
        "source_unchanged": True,
    }


# Gate name -> the `provider` column value its rows carry in pbx/matrix.tsv.
# The proxy labs (registrar-proxy + rtpengine media relay) append to the same
# matrix as the B2BUA providers, so their rows need the same row-level check:
# without one, a proxy gate that exits 0 having recorded nothing would pass.
PBX_MATRIX_PROVIDERS = {
    "local Asterisk PBX matrix": "asterisk",
    "local FreeSWITCH PBX matrix": "freeswitch",
    "local Kamailio PBX matrix": "kamailio",
    "local OpenSIPS PBX matrix": "opensips",
}


def interop_observed_check(report_root: Path, name: str) -> dict[str, Any] | None:
    if name in PBX_MATRIX_PROVIDERS:
        provider = PBX_MATRIX_PROVIDERS[name]
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
        # SKIP rows are the AMR capability probe declining cells the PBX
        # image cannot run (see examples/pbx/amr_probe.sh); they are evidence
        # of a deliberate non-run, not of success, so the gate needs at least
        # one genuine PASS alongside them and tolerates no FAIL.
        return {
            "check": f"{provider} PBX matrix rows",
            "observed": {
                "rows": len(statuses),
                "pass": statuses.count("PASS"),
                "skip": statuses.count("SKIP"),
            },
            "passed": statuses.count("PASS") > 0
            and all(status in {"PASS", "SKIP"} for status in statuses),
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
    if name == PROXY_INTEROP_GATE_NAME:
        observed = validate_proxy_interop_result(report_root)
        return {
            "check": "real Kamailio/OpenSIPS stateful-proxy matrix",
            "observed": observed,
            "passed": True,
        }
    return None


def _normalized_interop_identity(
    peer: dict[str, Any], display_name: str, *, require_image: bool
) -> dict[str, Any]:
    version = peer.get("version")
    image_digest = peer.get("image_digest")
    config_sha256 = peer.get("config_sha256")
    evidence_paths = peer.get("evidence_paths")
    if (
        not isinstance(config_sha256, str)
        or re.fullmatch(r"[0-9a-f]{64}", config_sha256) is None
        or not isinstance(evidence_paths, list)
        or not evidence_paths
        or not all(
            isinstance(path, str) and safe_relative(path) for path in evidence_paths
        )
        or not isinstance(version, str)
        or not version.strip()
    ):
        raise ReportError(
            f"{display_name} interoperability identity evidence is incomplete"
        )
    if require_image and (
        not isinstance(image_digest, str)
        or re.fullmatch(r"sha256:[0-9a-f]{64}", image_digest) is None
    ):
        raise ReportError(f"{display_name} interoperability image is not digest pinned")
    return {
        "version": version,
        "image_digest": image_digest,
        "config_sha256": config_sha256,
        "evidence_paths": sorted(evidence_paths),
    }


def _interop_attestation_identity(
    attestation: dict[str, Any],
    product: str,
    *,
    require_pbx_source: bool,
) -> dict[str, Any]:
    spec = INTEROP_ATTESTATION_SPECS[product]
    candidates = [
        peer
        for peer in attestation.get("peers", [])
        if isinstance(peer, dict) and peer.get("product") == product
    ]
    if product in PROXY_INTEROP_PEERS:
        runtime = [
            peer
            for peer in candidates
            if any(
                isinstance(path, str)
                and path.startswith(spec["identity_evidence_prefix"])
                for path in peer.get("evidence_paths", [])
            )
        ]
        if len(runtime) != 1:
            raise ReportError(
                f"{spec['display_name']} interoperability identity must resolve "
                f"to exactly one attested runtime peer; found {len(runtime)}"
            )
        return {
            "runtime": _normalized_interop_identity(
                runtime[0], spec["display_name"], require_image=True
            ),
            "source_checkout": None,
        }

    runtime = [
        peer
        for peer in candidates
        if isinstance(peer.get("image_digest"), str)
        and any(
            isinstance(path, str) and path.startswith("environment/docker-")
            for path in peer.get("evidence_paths", [])
        )
    ]
    source = [
        peer
        for peer in candidates
        if any(
            isinstance(path, str) and path.startswith(spec["identity_evidence_prefix"])
            for path in peer.get("evidence_paths", [])
        )
    ]
    if len(runtime) != 1:
        raise ReportError(
            f"{spec['display_name']} interoperability identity must resolve "
            f"to exactly one running image/configuration; found {len(runtime)}"
        )
    if len(source) > 1 or (require_pbx_source and len(source) != 1):
        raise ReportError(
            f"{spec['display_name']} interoperability identity must resolve "
            f"to exactly one local source revision; found {len(source)}"
        )
    source_identity = None
    if source:
        source_identity = _normalized_interop_identity(
            source[0], spec["display_name"], require_image=False
        )
        if (
            re.fullmatch(r"[0-9a-f]{40}", source_identity["version"]) is None
            or source_identity["image_digest"] is not None
        ):
            raise ReportError(
                f"{spec['display_name']} local source identity is not a full "
                "40-character Git revision"
            )
    return {
        "runtime": _normalized_interop_identity(
            runtime[0], spec["display_name"], require_image=True
        ),
        "source_checkout": source_identity,
    }


def _interop_attestation_evidence(
    gate: dict[str, Any], product: str
) -> list[dict[str, Any]]:
    evidence = gate.get("evidence")
    if not isinstance(evidence, list) or not evidence:
        raise ReportError(
            f"{INTEROP_ATTESTATION_SPECS[product]['display_name']} "
            "interoperability gate has no evidence"
        )
    selected: list[dict[str, Any]] = []
    for index, item in enumerate(evidence):
        if not isinstance(item, dict):
            raise ReportError("interoperability gate evidence entry is invalid")
        path = item.get("path")
        sha256 = item.get("sha256")
        if (
            not isinstance(path, str)
            or not safe_relative(path)
            or not isinstance(sha256, str)
            or re.fullmatch(r"[0-9a-f]{64}", sha256) is None
        ):
            raise ReportError("interoperability gate evidence binding is invalid")
        include = product in {"asterisk", "freeswitch"}
        if product in PROXY_INTEROP_PEERS:
            include = (
                index == 0
                or path in PROXY_INTEROP_ANCILLARY_EVIDENCE
                or path.startswith(f"proxy-interop/{product}/")
            )
        if include:
            selected.append(
                {
                    "path": path,
                    "sha256": sha256,
                    "bytes": item.get("bytes"),
                }
            )
    unique = {item["path"]: item for item in selected}
    if not unique:
        raise ReportError(
            f"{INTEROP_ATTESTATION_SPECS[product]['display_name']} "
            "interoperability evidence selection is empty"
        )
    return [unique[path] for path in sorted(unique)]


def _interop_attestation_coverage(gate: dict[str, Any], product: str) -> dict[str, Any]:
    spec = INTEROP_ATTESTATION_SPECS[product]
    if gate.get("status") != "PASS":
        raise ReportError(f"{spec['display_name']} interoperability gate is not PASS")
    observed_checks = gate.get("observed_checks")
    if not isinstance(observed_checks, list):
        raise ReportError(f"{spec['display_name']} interoperability checks are missing")
    if product in PROXY_INTEROP_PEERS:
        check_name = "real Kamailio/OpenSIPS stateful-proxy matrix"
    else:
        check_name = f"{product} PBX matrix rows"
    checks = [
        check
        for check in observed_checks
        if isinstance(check, dict) and check.get("check") == check_name
    ]
    if len(checks) != 1 or checks[0].get("passed") is not True:
        raise ReportError(
            f"{spec['display_name']} interoperability PASS check is missing"
        )
    observed = checks[0].get("observed")
    if not isinstance(observed, dict):
        raise ReportError(
            f"{spec['display_name']} interoperability coverage is invalid"
        )
    if product in PROXY_INTEROP_PEERS:
        if (
            product not in observed.get("peers", [])
            or set(observed.get("orders", [])) != PROXY_INTEROP_ORDERS
            or set(observed.get("transports", [])) != PROXY_INTEROP_TRANSPORTS
            or not isinstance(observed.get("scenarios_by_peer", {}).get(product), list)
            or not observed["scenarios_by_peer"][product]
        ):
            raise ReportError(
                f"{spec['display_name']} stateful-proxy coverage is incomplete"
            )
        return {
            "kind": "stateful-proxy-matrix",
            "matrix_rows": len(PROXY_INTEROP_ORDERS) * len(PROXY_INTEROP_TRANSPORTS),
            "orders": sorted(PROXY_INTEROP_ORDERS),
            "transports": sorted(PROXY_INTEROP_TRANSPORTS),
            "scenarios": observed["scenarios_by_peer"][product],
            "retention_drain_seconds": observed.get("retention_drain_seconds"),
        }
    rows = observed.get("rows")
    passed = observed.get("pass")
    if (
        not isinstance(rows, int)
        or isinstance(rows, bool)
        or rows <= 0
        or passed != rows
    ):
        raise ReportError(f"{spec['display_name']} PBX matrix coverage is incomplete")
    return {
        "kind": "pbx-call-and-media-matrix",
        "matrix_rows": rows,
        "passed_rows": passed,
    }


def build_interop_peer_attestation(
    attestation: dict[str, Any], gates: dict[str, Any]
) -> dict[str, Any]:
    records = gates.get("records")
    if not isinstance(records, list):
        raise ReportError("gate results are missing interoperability records")
    by_id = {
        record.get("id"): record
        for record in records
        if isinstance(record, dict) and isinstance(record.get("id"), str)
    }
    required_products = sorted(
        product
        for product, spec in INTEROP_ATTESTATION_SPECS.items()
        if spec["gate_id"] in by_id
    )
    if not required_products:
        raise ReportError("release report has no peer interoperability gates")
    if "interop.proxy-stateful-matrix" in by_id and required_products != sorted(
        INTEROP_ATTESTATION_SPECS
    ):
        raise ReportError(
            "stateful-proxy release reporting requires the Asterisk, FreeSWITCH, "
            "Kamailio, and OpenSIPS interoperability gates"
        )
    require_pbx_source = "interop.proxy-stateful-matrix" in by_id

    peer_records: list[dict[str, Any]] = []
    for product in required_products:
        spec = INTEROP_ATTESTATION_SPECS[product]
        gate = by_id[spec["gate_id"]]
        coverage = _interop_attestation_coverage(gate, product)
        peer_record = {
            "product": product,
            "display_name": spec["display_name"],
            "status": "PASS",
            "scope": spec["scope"],
            "gate": {"id": gate["id"], "name": gate["name"]},
            "identity": _interop_attestation_identity(
                attestation,
                product,
                require_pbx_source=require_pbx_source,
            ),
            "coverage": coverage,
            "evidence": _interop_attestation_evidence(gate, product),
        }
        peer_record["attestation_sha256"] = sha256_bytes(canonical_json(peer_record))
        peer_records.append(peer_record)

    attested_products = [record["product"] for record in peer_records]
    complete = attested_products == required_products
    if not complete:
        raise ReportError(
            "peer interoperability attestation does not cover every required product"
        )
    source = attestation.get("source", {}).get("start", {})
    source_binding = {
        "git_commit": source.get("git_commit"),
        "git_tree": source.get("git_tree"),
        "source_fingerprint_sha256": source.get("source_fingerprint_sha256"),
    }
    if (
        not isinstance(source_binding["git_commit"], str)
        or re.fullmatch(r"[0-9a-f]{40}", source_binding["git_commit"]) is None
        or not isinstance(source_binding["git_tree"], str)
        or re.fullmatch(r"[0-9a-f]{40}", source_binding["git_tree"]) is None
        or not isinstance(source_binding["source_fingerprint_sha256"], str)
        or re.fullmatch(r"[0-9a-f]{64}", source_binding["source_fingerprint_sha256"])
        is None
    ):
        raise ReportError("peer interoperability attestation lacks a source binding")
    return {
        "schema": SCHEMA_INTEROP_ATTESTATION,
        "status": "PASS",
        "complete": True,
        "required_products": required_products,
        "attested_products": attested_products,
        "source": source_binding,
        "records": peer_records,
        "non_claim": INTEROP_ATTESTATION_NON_CLAIM,
    }


def validate_interop_peer_attestation(
    value: Any,
    gates: dict[str, Any],
    peers: list[dict[str, Any]] | None = None,
    binding: dict[str, Any] | None = None,
) -> None:
    if not isinstance(value, dict) or value.get("schema") != SCHEMA_INTEROP_ATTESTATION:
        raise ReportError("peer interoperability attestation schema is invalid")
    if value.get("status") != "PASS" or value.get("complete") is not True:
        raise ReportError("peer interoperability attestation is not a complete PASS")
    if value.get("non_claim") != INTEROP_ATTESTATION_NON_CLAIM:
        raise ReportError("peer interoperability attestation non-claim is invalid")
    records = value.get("records")
    if not isinstance(records, list) or not records:
        raise ReportError("peer interoperability attestation has no records")
    products = [record.get("product") for record in records if isinstance(record, dict)]
    expected = sorted(
        product
        for product, spec in INTEROP_ATTESTATION_SPECS.items()
        if any(
            gate.get("id") == spec["gate_id"]
            for gate in gates.get("records", [])
            if isinstance(gate, dict)
        )
    )
    if (
        products != expected
        or value.get("required_products") != expected
        or value.get("attested_products") != expected
        or len(products) != len(set(products))
    ):
        raise ReportError("peer interoperability attestation product set is invalid")
    proxy_gate_present = "interop.proxy-stateful-matrix" in {
        gate.get("id") for gate in gates.get("records", []) if isinstance(gate, dict)
    }
    if proxy_gate_present and products != sorted(INTEROP_ATTESTATION_SPECS):
        raise ReportError(
            "stateful-proxy release attestation must cover Asterisk, FreeSWITCH, "
            "Kamailio, and OpenSIPS"
        )
    if binding is not None:
        expected_source = {
            "git_commit": binding.get("tested_commit"),
            "git_tree": binding.get("tested_tree"),
            "source_fingerprint_sha256": binding.get("source_fingerprint_sha256"),
        }
        if value.get("source") != expected_source:
            raise ReportError(
                "peer interoperability attestation source binding is invalid"
            )
    gates_by_id = {
        gate.get("id"): gate
        for gate in gates.get("records", [])
        if isinstance(gate, dict) and isinstance(gate.get("id"), str)
    }
    for record in records:
        product = record.get("product")
        if product not in INTEROP_ATTESTATION_SPECS:
            raise ReportError(
                f"peer interoperability attestation product is invalid: {product!r}"
            )
        if (
            record.get("status") != "PASS"
            or record.get("display_name")
            != INTEROP_ATTESTATION_SPECS[product]["display_name"]
            or record.get("scope") != INTEROP_ATTESTATION_SPECS[product]["scope"]
            or record.get("gate", {}).get("id")
            != INTEROP_ATTESTATION_SPECS[product]["gate_id"]
            or record.get("gate", {}).get("name")
            != gates_by_id.get(INTEROP_ATTESTATION_SPECS[product]["gate_id"], {}).get(
                "name"
            )
            or not record.get("coverage")
            or not record.get("evidence")
            or not record.get("identity", {}).get("runtime", {}).get("config_sha256")
        ):
            raise ReportError(
                f"peer interoperability attestation record is incomplete: "
                f"{record.get('product')!r}"
            )
        gate = gates_by_id.get(INTEROP_ATTESTATION_SPECS[product]["gate_id"])
        if (
            gate is None
            or record.get("coverage") != _interop_attestation_coverage(gate, product)
            or record.get("evidence") != _interop_attestation_evidence(gate, product)
        ):
            raise ReportError(
                f"peer interoperability attestation disagrees with its gate: {product}"
            )
        if peers is not None and record.get(
            "identity"
        ) != _interop_attestation_identity(
            {"peers": peers},
            product,
            require_pbx_source=proxy_gate_present,
        ):
            raise ReportError(
                f"peer interoperability attestation identity drifted: {product}"
            )
        unsigned_record = {
            key: value for key, value in record.items() if key != "attestation_sha256"
        }
        if record.get("attestation_sha256") != sha256_bytes(
            canonical_json(unsigned_record)
        ):
            raise ReportError(
                f"peer interoperability attestation hash is invalid: {product}"
            )


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
        raise ReportError(
            f"gate selection mismatch; missing={missing}, unselected={extra}"
        )
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
            if recorded["name"] == PROXY_INTEROP_GATE_NAME:
                captured = {item["path"] for item in evidence}
                for relative in proxy_interop_evidence_paths(report_root):
                    if relative not in captured:
                        evidence.append(
                            verify_artifact(report_root, artifacts, relative)
                        )
                        captured.add(relative)
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
                "relevant_configuration": relevant_configuration(
                    gate["category"], config
                ),
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
        raise ReportError(
            f"native gate selection mismatch; missing={missing}, extra={extra}"
        )
    if native.get("failed") or native.get("skipped"):
        raise ReportError(
            "native release reporting requires zero failed and zero skipped gates"
        )
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
        captured_evidence = {item["path"] for item in evidence}
        for relative in ancillary_gate_evidence(gate["name"]):
            if relative not in captured_evidence:
                evidence.append(verify_artifact(report_root, artifacts, relative))
                captured_evidence.add(relative)
        argv = source.get("sanitized_argv")
        if not isinstance(argv, list) or not argv:
            raise ReportError(f"native gate {gate['id']} has no structured argv")
        checks = source.get("checks", [])
        if not checks or not all(check.get("passed") for check in checks):
            raise ReportError(f"native gate {gate['id']} has missing or failed checks")
        checks = list(checks)
        interop_check = interop_observed_check(report_root, gate["name"])
        if interop_check:
            if not interop_check["passed"]:
                raise ReportError(f"interop matrix did not pass for {gate['name']}")
            checks.append(interop_check)
            if gate["name"] == PROXY_INTEROP_GATE_NAME:
                for relative in proxy_interop_evidence_paths(report_root):
                    if relative not in captured_evidence:
                        evidence.append(
                            verify_artifact(report_root, artifacts, relative)
                        )
                        captured_evidence.add(relative)
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
                "relevant_configuration": relevant_configuration(
                    gate["category"], config
                ),
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


def latency_percentiles(
    report: dict[str, Any], context: str
) -> dict[str, dict[str, int | float]]:
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
            if (
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or value < 0
            ):
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
            raise ReportError(
                f"policy is missing canonical latency limits for {family}"
            )
        family_limits: dict[str, float] = {}
        for percentile in LATENCY_PERCENTILES:
            value = family_source.get(percentile)
            if (
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or value <= 0
            ):
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
                    "calls_succeeded": endpoint.get("results", {}).get(
                        "calls_succeeded"
                    ),
                    "latency_ns": latency_percentiles(endpoint, endpoint_relative),
                }
            ],
            "evidence": verify_artifact(report_root, artifacts, endpoint_relative),
        },
    )
    tolerance = effective.get("values_by_key", {}).get(
        "beta_perf_latency_tolerance_pct"
    )
    if (
        not isinstance(tolerance, (int, float))
        or isinstance(tolerance, bool)
        or tolerance < 0
    ):
        raise ReportError(
            "effective configuration has invalid latency regression tolerance"
        )
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
        (
            index
            for index, name in enumerate(header)
            if name.lower() in {"status", "result"}
        ),
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
    interop_peer_attestation = build_interop_peer_attestation(attestation, gates)
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
                "json_evidence_count": len(
                    attestation.get("results", {}).get("json", [])
                ),
                "performance_json_count": performance["json_artifact_count"],
            },
            "peers": attestation.get("peers", []),
            "interop": {
                "peer_attestation": interop_peer_attestation,
                "pbx_matrix": {
                    "results": count_tsv_results(report_root / "pbx/matrix.tsv"),
                    "evidence": [
                        verify_artifact(
                            report_root, indexed_artifacts, "pbx/matrix.tsv"
                        ),
                        verify_artifact(
                            report_root, indexed_artifacts, "pbx/summary.md"
                        ),
                    ],
                },
                "sipp_matrix": {
                    "results": count_tsv_results(report_root / "sipp/runs.tsv"),
                    "evidence": [
                        verify_artifact(
                            report_root, indexed_artifacts, "sipp/runs.tsv"
                        ),
                        verify_artifact(
                            report_root, indexed_artifacts, "sipp/run_summary.md"
                        ),
                    ],
                },
                "strict_ua_matrix": {
                    "results": count_tsv_results(report_root / "strict-ua/matrix.tsv"),
                    "evidence": [
                        verify_artifact(
                            report_root, indexed_artifacts, "strict-ua/matrix.tsv"
                        ),
                        verify_artifact(
                            report_root, indexed_artifacts, "strict-ua/summary.md"
                        ),
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


def render_interop_attestation_rows(value: dict[str, Any]) -> list[str]:
    lines: list[str] = []
    for record in value["records"]:
        identity = record["identity"]
        runtime = identity["runtime"]
        identity_value = runtime.get("version") or runtime.get("image_digest")
        if identity.get("source_checkout"):
            identity_value += f"; source {identity['source_checkout']['version']}"
        coverage = record["coverage"]
        if coverage["kind"] == "stateful-proxy-matrix":
            coverage_text = (
                f"{coverage['matrix_rows']} rows; "
                f"{'/'.join(coverage['orders'])}; "
                f"{'/'.join(coverage['transports'])}; "
                f"{len(coverage['scenarios'])} scenarios"
            )
        else:
            coverage_text = (
                f"{coverage['passed_rows']}/{coverage['matrix_rows']} matrix rows"
            )
        lines.append(
            f"| {record['display_name']} | **{record['status']}** | "
            f"{record['scope']} | `{md_escape(identity_value)}` | "
            f"`{md_escape(coverage_text)}` | `{record['gate']['id']}` | "
            f"`{record['attestation_sha256']}` |"
        )
    return lines


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
    interop_attestation = evidence["interop"]["peer_attestation"]
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
        "## Interoperability attestation",
        "",
        "Each row is a source-, peer-identity-, configuration-, and evidence-bound "
        "attestation. A missing, skipped, ambiguous, unpinned, or failing required "
        "peer prevents report generation.",
        "",
        "| Peer | Status | Qualified scope | Version/image | Coverage | Gate | Attestation SHA-256 |",
        "|---|---|---|---|---|---|---|",
    ]
    lines.extend(render_interop_attestation_rows(interop_attestation))
    lines += [
        "",
        f"Attested source: commit `{interop_attestation['source']['git_commit']}`, "
        f"tree `{interop_attestation['source']['git_tree']}`, fingerprint "
        f"`{interop_attestation['source']['source_fingerprint_sha256']}`.",
        "",
        f"> {interop_attestation['non_claim']}",
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
            family_checks = [
                checks[(family, percentile)] for percentile in LATENCY_PERCENTILES
            ]
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
        label = (
            "Call setup" if check["family"] == "setup_latency" else "Full call cycle"
        )
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
            raise ReportError(
                "source run ID does not match the current policy candidate"
            )
        if (
            attestation.get("source", {}).get("start", {}).get("git_commit")
            != expected["tested_commit"]
        ):
            raise ReportError(
                "tested commit does not match the current policy candidate"
            )
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
        raise ReportError(
            "native report generation requires a clean, unchanged, zero-skip full PASS"
        )
    if not attestation.get("qualification", {}).get("release_candidate"):
        raise ReportError("source attestation is not release-candidate qualified")
    return attestation


def generate(report_root: Path, policy_path: Path, output_dir: Path) -> None:
    source_verifier(report_root)
    policy = load_policy(policy_path)
    native = (report_root / "effective-gate-config.json").is_file() and (
        report_root / "gate-results.json"
    ).is_file()
    attestation = validate_source_attestation(
        report_root, policy, current_only=not native
    )
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
        name: {
            "sha256": sha256_path(output_dir / name),
            "bytes": (output_dir / name).stat().st_size,
        }
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
        "interop_attestation": {
            "schema": SCHEMA_INTEROP_ATTESTATION,
            "status": evidence["interop"]["peer_attestation"]["status"],
            "products": evidence["interop"]["peer_attestation"]["attested_products"],
            "release_evidence_sha256": bindings["release-evidence.json"]["sha256"],
        },
        "assurance": {
            "kind": "integrity-and-reproducibility",
            "cryptographically_signed": False,
            "note": "SHA-256 integrity evidence; not third-party signing.",
        },
    }
    write_atomic(
        output_dir / "report-attestation.json", canonical_json(report_attestation)
    )
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


def validate_report_schema_scope(report_schema: str, gates: dict[str, Any]) -> None:
    if report_schema == SCHEMA_REPORT_ATTESTATION_LEGACY and any(
        isinstance(record, dict) and record.get("id") == "interop.proxy-stateful-matrix"
        for record in gates.get("records", [])
    ):
        raise ReportError(
            "legacy report schema cannot attest the stateful-proxy release gate; "
            "four-peer interoperability attestation v2 is required"
        )


def verify_generated(
    directory: Path, expected_policy: Path | None = None
) -> dict[str, Any]:
    for name in ALL_GENERATED_FILES:
        if not (directory / name).is_file():
            raise ReportError(f"missing generated report artifact {directory / name}")
    attestation = read_json(directory / "report-attestation.json")
    report_schema = attestation.get("schema")
    if report_schema not in {
        SCHEMA_REPORT_ATTESTATION_LEGACY,
        SCHEMA_REPORT_ATTESTATION,
    }:
        raise ReportError("unsupported report attestation schema")
    legacy_report = report_schema == SCHEMA_REPORT_ATTESTATION_LEGACY
    checksum_line = (directory / "report-attestation.json.sha256").read_text().strip()
    expected_checksum = (
        f"{sha256_path(directory / 'report-attestation.json')}  report-attestation.json"
    )
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
    validate_report_schema_scope(report_schema, gates)
    expected_evidence_schema = (
        SCHEMA_EVIDENCE_LEGACY if legacy_report else SCHEMA_EVIDENCE
    )
    if evidence.get("schema") != expected_evidence_schema:
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
        raise ReportError(
            "generated gate totals are not an all-required, zero-skip PASS"
        )
    if (
        len(records) != required_count
        or len({item.get("id") for item in records}) != required_count
    ):
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
        raise ReportError(
            "performance inventory does not account for 59 JSON artifacts"
        )
    if not legacy_report:
        validate_interop_peer_attestation(
            evidence.get("interop", {}).get("peer_attestation"),
            gates,
            evidence.get("peers"),
            evidence.get("binding"),
        )
        interop_summary = attestation.get("interop_attestation")
        peer_attestation = evidence["interop"]["peer_attestation"]
        if (
            not isinstance(interop_summary, dict)
            or interop_summary.get("schema") != SCHEMA_INTEROP_ATTESTATION
            or interop_summary.get("status") != "PASS"
            or interop_summary.get("products") != peer_attestation["attested_products"]
            or interop_summary.get("release_evidence_sha256")
            != sha256_path(directory / "release-evidence.json")
        ):
            raise ReportError("report interoperability attestation binding is invalid")
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
        raise ReportError(
            "current candidate report_revision must be a positive integer"
        )
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


def promote_docs(report_root: Path, policy_path: Path, docs_root: Path) -> None:
    source_verifier(report_root)
    policy = load_policy(policy_path)
    run_id = policy["current_candidate"]["run_id"]
    releases_root = docs_root / "releases/beta"
    snapshot_relative = current_snapshot_relative(policy)
    snapshot = releases_root / snapshot_relative
    with tempfile.TemporaryDirectory(
        prefix=".beta-report-", dir=docs_root.parent
    ) as tmp:
        generated = Path(tmp)
        generate(report_root, policy_path, generated)
        verify_generated(generated, policy_path)
        if snapshot.exists():
            for name in ALL_GENERATED_FILES:
                existing = snapshot / name
                if (
                    not existing.is_file()
                    or existing.read_bytes() != (generated / name).read_bytes()
                ):
                    raise ReportError(
                        f"immutable snapshot {snapshot} exists with different content"
                    )
        else:
            releases_root.mkdir(parents=True, exist_ok=True)
            staged_snapshot = releases_root / f".{run_id}.tmp"
            if staged_snapshot.exists():
                raise ReportError(
                    f"stale promotion directory exists: {staged_snapshot}"
                )
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
    if (
        not index.is_file()
        or f"Current candidate: `{run_id}`." not in index.read_text()
    ):
        raise ReportError("release index/current-candidate marker is missing")
    verify_markdown_links(index.parent, index)
    assert_no_sensitive_or_absolute_paths(index)
    if report_root:
        source_verifier(report_root)
        source = validate_source_attestation(report_root, policy)
        if sha256_path(report_root / "attestation.json") != report_attestation.get(
            "source_attestation_sha256"
        ):
            raise ReportError(
                "promoted report is not bound to the supplied source attestation"
            )
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
                    (
                        line.strip()
                        for line in path.read_text().splitlines()
                        if line.strip()
                    ),
                    None,
                )
        status_path = environment_dir / "git-status.txt"
        if status_path.is_file():
            derived["git_status"] = (
                "dirty" if status_path.read_text().strip() else "clean"
            )
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
    validate_proxy = commands.add_parser("validate-proxy-interop")
    validate_proxy.add_argument("--report-root", required=True, type=Path)
    capture = commands.add_parser("capture-config")
    capture.add_argument("--policy", type=Path, default=default_policy)
    capture.add_argument("--output", required=True, type=Path)
    capture.add_argument(
        "--mode",
        required=True,
        choices=["local", "full", "interop", "perf", "security"],
    )
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
            generate(
                args.report_root.resolve(),
                args.policy.resolve(),
                args.output_dir.resolve(),
            )
            verify_generated(args.output_dir.resolve(), args.policy.resolve())
            print(f"generated and verified beta reports in {args.output_dir}")
        elif args.command == "promote-docs":
            promote_docs(
                args.report_root.resolve(),
                args.policy.resolve(),
                args.docs_root.resolve(),
            )
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
                attestation = validate_source_attestation(
                    args.report_root.resolve(), policy
                )
                effective = effective_configuration(
                    args.report_root.resolve(), attestation, policy
                )
                build_gate_results(
                    args.report_root.resolve(), attestation, policy, effective
                )
            print("beta release policy validation: PASS")
        elif args.command == "validate-proxy-interop":
            result = validate_proxy_interop_result(args.report_root.resolve())
            print(
                "stateful proxy interoperability validation: PASS "
                f"({result['rows']} rows, {len(result['scenarios'])} scenarios)"
            )
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
