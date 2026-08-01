#!/usr/bin/env python3
"""Generate the version-neutral release catalog from the canonical 108-gate run."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import shlex
import subprocess
from typing import Any


SCHEMA = "rvoip-release-gate-catalog-v1"
SOURCE = Path(
    "crates/sip/rvoip-sip/docs/releases/beta/20260724T231400Z/gate-results.json"
)
OUTPUT = Path("scripts/release/gates.json")
LEGACY_UNSTRUCTURED = {
    "source.clean-start",
    "source.canonical-2k",
    "interop.proxy-descope",
    "perf.capture-boundary",
    "perf.literal-all-config",
    "report.regression-audit",
    "report.perf-evidence-capture",
    "report.performance-metrics",
    "source.final-capture",
    "source.canonical-2k-unchanged",
}
COLLECT_AGGREGATES = {
    "source.canonical-2k",
    "perf.capture-boundary",
    "perf.literal-all-config",
    "report.regression-audit",
    "report.perf-evidence-capture",
    "report.performance-metrics",
    "source.final-capture",
    "source.canonical-2k-unchanged",
    "interop.proxy-descope",
    "perf.media-burst-matrix",
}
BURST_SCENARIOS = [
    "carrier-smoke",
    "access-edge-microburst",
    "contact-center-flash",
    "shift-change-long-hold",
    "overload-recovery",
    "high-density-media-burst",
    "buffer-ab-legacy",
]
COMMAND_OVERRIDES = {
    "security.advisory-audit": [
        "cargo",
        "deny",
        "check",
        "advisories",
        "bans",
        "sources",
    ],
    "interop.asterisk-matrix": [
        "env",
        "PBX_OUT_ROOT={artifact_dir}",
        "PBX_REPORT_APPEND=1",
        "PBX_G729_PROFILES=g729a g729ab",
        "{workspace}/crates/sip/rvoip-sip/examples/pbx/run.sh",
        "--pbx",
        "asterisk",
        "--api",
        "all",
        "--scenario",
        "all",
    ],
    "interop.freeswitch-matrix": [
        "env",
        "PBX_OUT_ROOT={artifact_dir}",
        "PBX_REPORT_APPEND=1",
        "PBX_G729_PROFILES=g729a g729ab",
        "{workspace}/crates/sip/rvoip-sip/examples/pbx/run.sh",
        "--pbx",
        "freeswitch",
        "--api",
        "all",
        "--scenario",
        "all",
    ],
    "interop.sipp-matrix": [
        "env",
        "RVOIP_PERF_RESULTS={artifact_dir}",
        "RVOIP_PERF_CPS=30 100 300 1000 2000",
        "RVOIP_PERF_MIN_SUCCESS_PCT=99.9",
        "{workspace}/crates/sip/rvoip-sip/tests/perf/sipp_scenarios/run_comparison.sh",
        "127.0.0.1",
        "35060",
        "rvoip",
    ],
}

for _gate_id, _action in {
    "interop.freeswitch-down-before-asterisk": "freeswitch-down",
    "interop.asterisk-up": "asterisk-up",
    "interop.asterisk-down-after": "asterisk-down",
    "interop.asterisk-down-before-freeswitch": "asterisk-down",
    "interop.freeswitch-up": "freeswitch-up",
    "interop.freeswitch-down-after": "freeswitch-down",
    "interop.restore-asterisk-down": "restore-asterisk-down",
    "interop.restore-freeswitch-down": "restore-freeswitch-down",
    "interop.sipp-start": "sipp-start",
    "interop.sipp-stop": "sipp-stop",
}.items():
    COMMAND_OVERRIDES[_gate_id] = [
        "bash",
        "infra/release-runners/interop-lifecycle.sh",
        _action,
    ]

FUZZ_TARGETS = {
    "sip-message": "sip_message",
    "uri": "uri",
    "header": "header",
    "sdp": "sdp",
    "rtp": "rtp_packet",
    "rtcp": "rtcp_packet",
    "srtp": "srtp_unprotect",
    "dtls": "dtls_record",
    "stun": "stun_response",
    "g711": "g711_unpack",
}

for _suffix, _target in FUZZ_TARGETS.items():
    _fuzz_dir = (
        "{workspace}/crates/sip/fuzz"
        if _suffix in {"sip-message", "uri", "header", "sdp"}
        else "{workspace}/crates/media/fuzz"
    )
    COMMAND_OVERRIDES[f"security.fuzz-{_suffix}"] = [
        "cargo",
        "+nightly",
        "fuzz",
        "run",
        _target,
        "--fuzz-dir",
        _fuzz_dir,
        # GitHub's hosted environment may set a static-musl Cargo target.
        # libFuzzer's AddressSanitizer requires the runner's dynamically
        # linked GNU target instead.
        "--target",
        "x86_64-unknown-linux-gnu",
        "--",
        "-runs=1000",
        "-max_total_time=10",
    ]


def run_json(argv: list[str], root: Path) -> dict[str, Any]:
    return json.loads(subprocess.run(argv, cwd=root, text=True, capture_output=True, check=True).stdout)


def insert_locked(argv: list[str]) -> list[str]:
    try:
        cargo = argv.index("cargo")
    except ValueError:
        return argv
    if "--locked" not in argv and cargo + 1 < len(argv):
        return argv[: cargo + 2] + ["--locked"] + argv[cargo + 2 :]
    return argv


def normalized_command(record: dict[str, Any]) -> list[str] | None:
    gate_id = record["id"]
    if gate_id in COMMAND_OVERRIDES:
        return COMMAND_OVERRIDES[gate_id]
    if gate_id in LEGACY_UNSTRUCTURED:
        return None
    command = record.get("sanitized_argv", "")
    if not command or command.endswith("bash -c ") or command == "<local-path>":
        return None
    argv = shlex.split(command)
    if not argv or argv[0] in {
        "verify_clean_source_fingerprint",
        "prepare_perf_results_capture",
        "capture_current_perf_results",
        "write_performance_gate_metrics",
    }:
        return None
    return insert_locked(
        [
            value.replace("<workspace>", "{workspace}").replace(
                "<source-report>", "{artifact_dir}"
            )
            for value in argv
        ]
    )


def affected_crates(command: list[str] | None, all_packages: list[str]) -> list[str]:
    if not command:
        return []
    if "--workspace" in command:
        return all_packages
    result = []
    for index, value in enumerate(command[:-1]):
        if value in {"-p", "--package"}:
            result.append(command[index + 1])
    return sorted(set(result))


def affected_paths(record: dict[str, Any], command: list[str] | None) -> list[str]:
    gate_id = record["id"]
    if gate_id.startswith("perf."):
        return [
            "crates/sip/rvoip-sip/**",
            "crates/media/**",
            "crates/foundation/**",
        ]
    if gate_id.startswith("interop."):
        return ["crates/sip/**", "crates/media/**"]
    if gate_id.startswith("security."):
        return ["Cargo.lock", "deny.toml", "crates/**"]
    if gate_id.startswith("source.") or gate_id.startswith("report."):
        return ["**"]
    paths = []
    for value in command or []:
        expanded = value.replace("{workspace}/", "")
        if "/Cargo.toml" in expanded:
            paths.append(expanded.rsplit("/Cargo.toml", 1)[0] + "/**")
        elif expanded.startswith(("scripts/", "crates/", "examples/", "infra/")):
            path = expanded.split("=", 1)[-1]
            paths.append(path if "*" in path else path)
    return sorted(set(paths))


def resource_class(record: dict[str, Any]) -> str:
    gate_id = record["id"]
    if gate_id.startswith("perf."):
        if gate_id in {"perf.monolithic-soak", "perf.media-burst-matrix", "perf.soak-candidate"}:
            return "gcp-performance-soak"
        return "gcp-performance"
    if gate_id.startswith("interop."):
        return "gcp-interop"
    if gate_id.startswith("security.fuzz"):
        return "github-nightly"
    if gate_id.startswith(("source.", "report.")):
        return "github-evidence"
    return "github-standard"


def legacy_mode(record: dict[str, Any]) -> str:
    kind = record["kind"]
    if kind == "security":
        return "security"
    if kind == "interop":
        return "interop"
    if kind in {"performance", "reporting"}:
        return "perf"
    return "full"


def dependencies(record: dict[str, Any], records: list[dict[str, Any]]) -> list[str]:
    gate_id = record["id"]
    if gate_id == "source.clean-start":
        return []
    if gate_id == "source.canonical-2k":
        return [
            "source.clean-start",
            "perf.concurrent-calls",
            "perf.sipp-parity",
            "perf.media-burst-matrix",
        ]
    if gate_id == "source.final-capture":
        return [item["id"] for item in records if item["id"] != gate_id and not item["id"].startswith("source.canonical-2k-unchanged")]
    if gate_id == "source.canonical-2k-unchanged":
        return ["source.clean-start", "source.final-capture"]
    if gate_id == "interop.proxy-descope":
        return ["interop.remote-proxies"]
    if gate_id == "perf.media-burst-matrix":
        return [f"perf.media-burst.{scenario}" for scenario in BURST_SCENARIOS]
    if gate_id.startswith("report."):
        perf = [item["id"] for item in records if item["id"].startswith("perf.")]
        prior_reports = [
            item["id"]
            for item in records
            if item["id"].startswith("report.") and item["sequence"] < record["sequence"]
        ]
        return sorted(set(perf + prior_reports))
    interop_chain = [item["id"] for item in records if item["id"].startswith("interop.")]
    if gate_id in interop_chain:
        index = interop_chain.index(gate_id)
        return ["source.clean-start"] + ([interop_chain[index - 1]] if index else [])
    return ["source.clean-start"]


def legacy_gate(record: dict[str, Any], records: list[dict[str, Any]], packages: list[str]) -> dict[str, Any]:
    command = normalized_command(record)
    executor = "argv" if command else "legacy-group"
    if record["id"] in COLLECT_AGGREGATES:
        # These gates reconcile evidence produced by other shards. Running
        # them concurrently would race the evidence they are meant to audit.
        executor = "aggregate"
    if record["id"] == "source.clean-start":
        # The remote runner can verify this directly without invoking the
        # macOS-only beta wrapper.
        executor = "builtin"
    duration = int(record.get("duration_seconds", 0))
    return {
        "id": record["id"],
        "name": record["name"],
        "category": record["category"],
        "kind": record["kind"],
        "executor": executor,
        "command": command,
        "display_command": record.get("sanitized_argv"),
        "working_directory": ".",
        "dependencies": dependencies(record, records),
        "resource_class": resource_class(record),
        "timeout_minutes": max(5, math.ceil(duration / 60 * 2) + 5),
        "retry_on_exit_codes": [75],
        "max_infrastructure_retries": 1,
        "affected_crates": affected_crates(command, packages),
        "affected_paths": affected_paths(record, command),
        "expected_outputs": ["receipt.json", "command.log"],
        "estimated_seconds": max(1, duration),
        "always_fresh": record["kind"] in {"source", "reporting"},
        "legacy": {
            "sequence": record["sequence"],
            "mode": legacy_mode(record),
            "required": record["required"],
            "source_status": record["status"],
        },
    }


def core_gate(package: dict[str, Any], root: Path, weights: dict[str, int]) -> dict[str, Any]:
    name = package["name"]
    slug = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    crate_root = Path(package["manifest_path"]).resolve().parent.relative_to(root.resolve()).as_posix()
    return {
        "id": f"core.{slug}",
        "name": f"{name} unit, integration, example, and Clippy checks",
        "category": "Parallel 44-crate core",
        "kind": "cargo",
        "executor": "argv",
        "command": [
            "python3",
            "scripts/ci/run_checks.py",
            "shard",
            "--name",
            f"release-{slug}",
            "--packages",
            name,
            "--output",
            "{artifact_dir}/nested-ci-receipt.json",
        ],
        "display_command": f"test and lint {name}",
        "working_directory": ".",
        "dependencies": ["source.remote-clean"],
        "resource_class": "github-standard",
        "timeout_minutes": 60,
        "retry_on_exit_codes": [75],
        "max_infrastructure_retries": 1,
        "affected_crates": [name],
        "affected_paths": [f"{crate_root}/**"],
        "expected_outputs": ["receipt.json", "command.log", "nested-ci-receipt.json"],
        "estimated_seconds": int(weights.get(name, 2)) * 60,
        "always_fresh": False,
        "legacy": None,
    }


def synthetic_gate(
    gate_id: str,
    name: str,
    *,
    executor: str = "builtin",
    command: list[str] | None = None,
    resource: str = "github-evidence",
    dependencies: list[str] | None = None,
    paths: list[str] | None = None,
    always_fresh: bool = False,
) -> dict[str, Any]:
    return {
        "id": gate_id,
        "name": name,
        "category": "Remote release framework",
        "kind": "remote",
        "executor": executor,
        "command": command,
        "display_command": " ".join(command or [executor, gate_id]),
        "working_directory": ".",
        "dependencies": dependencies or [],
        "resource_class": resource,
        "timeout_minutes": 60,
        "retry_on_exit_codes": [75],
        "max_infrastructure_retries": 1,
        "affected_crates": [],
        "affected_paths": paths or ["**"],
        "expected_outputs": ["receipt.json", "command.log"],
        "estimated_seconds": 1,
        "always_fresh": always_fresh,
        "legacy": None,
    }


def security_gates() -> list[dict[str, Any]]:
    result = [
        synthetic_gate(
            "security.remote-advisories",
            "dependency advisory policy",
            executor="argv",
            command=["cargo", "deny", "check", "advisories", "bans", "sources"],
            resource="github-standard",
            dependencies=["source.remote-clean"],
            paths=["Cargo.lock", "deny.toml", "Cargo.toml", "crates/**/Cargo.toml"],
        )
    ]
    targets = {
        "sip-message": ("crates/sip/fuzz/Cargo.toml", "sip_message"),
        "uri": ("crates/sip/fuzz/Cargo.toml", "uri"),
        "header": ("crates/sip/fuzz/Cargo.toml", "header"),
        "sdp": ("crates/sip/fuzz/Cargo.toml", "sdp"),
        "rtp": ("crates/media/fuzz/Cargo.toml", "rtp_packet"),
        "rtcp": ("crates/media/fuzz/Cargo.toml", "rtcp_packet"),
        "srtp": ("crates/media/fuzz/Cargo.toml", "srtp_unprotect"),
        "dtls": ("crates/media/fuzz/Cargo.toml", "dtls_record"),
        "stun": ("crates/media/fuzz/Cargo.toml", "stun_response"),
        "g711": ("crates/media/fuzz/Cargo.toml", "g711_unpack"),
    }
    for suffix, (manifest, target) in targets.items():
        fuzz_dir = manifest.rsplit("/", 1)[0]
        result.append(
            synthetic_gate(
                f"security.remote-fuzz-{suffix}",
                f"bounded {target} fuzz smoke",
                executor="argv",
                command=[
                    "cargo",
                    "+nightly",
                    "fuzz",
                    "run",
                    target,
                    "--fuzz-dir",
                    f"{{workspace}}/{fuzz_dir}",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                    "--",
                    "-runs=1000",
                    "-max_total_time=10",
                ],
                resource="github-nightly",
                dependencies=["source.remote-clean"],
                paths=[manifest, fuzz_dir + "/**"],
            )
        )
    return result


def burst_scenario_gates() -> list[dict[str, Any]]:
    result = []
    for scenario in BURST_SCENARIOS:
        gate = synthetic_gate(
            f"perf.media-burst.{scenario}",
            f"media burst scenario: {scenario}",
            executor="argv",
            command=[
                "env",
                "RVOIP_PERF_RESULTS={artifact_dir}",
                f"RVOIP_PERF_BURST_SCENARIOS={scenario}",
                "bash",
                "{workspace}/crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh",
            ],
            resource="gcp-performance-soak",
            dependencies=["source.remote-clean"],
            paths=[
                "crates/sip/rvoip-sip/**",
                "crates/media/**",
                "crates/foundation/**",
            ],
        )
        gate["estimated_seconds"] = 900
        gate["timeout_minutes"] = 75
        result.append(gate)
    return result


def build_catalog(root: Path, source: Path) -> dict[str, Any]:
    source_payload = json.loads(source.read_text())
    records = source_payload["records"]
    if len(records) != 108:
        raise RuntimeError(f"canonical source must contain 108 records, found {len(records)}")
    metadata = run_json(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], root
    )
    members = set(metadata["workspace_members"])
    package_rows = sorted(
        (package for package in metadata["packages"] if package["id"] in members),
        key=lambda package: package["name"],
    )
    if len(package_rows) != 44:
        raise RuntimeError(f"expected 44 workspace packages, found {len(package_rows)}")
    package_names = [package["name"] for package in package_rows]
    weights = json.loads((root / "scripts/ci/policy.json").read_text()).get("package_weights", {})

    legacy = [legacy_gate(record, records, package_names) for record in records]
    synthetic = [
        synthetic_gate(
            "source.remote-clean",
            "clean candidate source fingerprint",
            always_fresh=True,
        ),
        synthetic_gate(
            "package.inventory",
            "44-package release inventory audit",
            executor="argv",
            command=["python3", "scripts/release.py", "audit"],
            dependencies=["source.remote-clean"],
            paths=["Cargo.toml", "Cargo.lock", "crates/**/Cargo.toml", "scripts/release.py"],
            always_fresh=True,
        ),
        synthetic_gate(
            "interop.remote-libsrtp",
            "pinned libSRTP bidirectional interoperability",
            executor="argv",
            command=["bash", "scripts/test_libsrtp_interop.sh"],
            resource="gcp-interop",
            dependencies=["source.remote-clean"],
            paths=["scripts/test_libsrtp_interop.sh", "crates/media/rtp-core/**"],
        ),
        synthetic_gate(
            "interop.remote-proxies",
            "Kamailio and OpenSIPS proxy matrix",
            executor="argv",
            command=["bash", "crates/sip/sip-proxy/tests/interop/scripts/beta_gate.sh"],
            resource="gcp-interop",
            dependencies=["source.remote-clean"],
            paths=["crates/sip/sip-proxy/**", "crates/sip/sip-transport/**", "crates/sip/sip-core/**"],
        ),
        synthetic_gate(
            "interop.browser-dtmf",
            "real Chromium outbound RFC 4733 interoperability (BridgeFu issue #54)",
            executor="argv",
            command=[
                "cargo",
                "test",
                "--locked",
                "-p",
                "rvoip-webrtc",
                "--features",
                "interop-browser,signaling-whip,signaling-ws",
                "--test",
                "browser_interop",
                "--",
                "--include-ignored",
                "--nocapture",
            ],
            resource="github-standard",
            dependencies=["source.remote-clean"],
            paths=[
                "crates/webrtc/rvoip-rtc/**",
                "crates/webrtc/rvoip-webrtc/**",
                "crates/core/rvoip-core/**",
            ],
        ),
    ]
    synthetic.extend(burst_scenario_gates())
    core = [core_gate(package, root, weights) for package in package_rows]
    security = security_gates()
    remote_without_final = [gate["id"] for gate in synthetic + core + security]
    final = [
        synthetic_gate(
            "report.remote-aggregate",
            "exact-candidate evidence reconciliation",
            executor="aggregate",
            dependencies=sorted(set(remote_without_final)),
            always_fresh=True,
        ),
        synthetic_gate(
            "source.remote-final",
            "final candidate source fingerprint",
            executor="aggregate",
            dependencies=["report.remote-aggregate"],
            always_fresh=True,
        ),
    ]
    gates = legacy + synthetic + core + security + final

    direct_legacy = [gate["id"] for gate in legacy if gate["executor"] == "argv"]
    structured_legacy = [
        gate["id"] for gate in legacy if gate["executor"] != "legacy-group"
    ]
    core_ids = [gate["id"] for gate in core]
    security_ids = [gate["id"] for gate in security]
    framework_ids = [
        "source.remote-clean",
        "source.clean-start",
        "package.inventory",
    ]
    lightweight_legacy = [
        gate_id
        for gate_id in direct_legacy
        if not gate_id.startswith(("perf.", "interop.", "report.", "source."))
    ]
    lightweight_legacy.append("source.clean-start")
    heavy_direct = [
        gate_id for gate_id in direct_legacy if gate_id.startswith(("perf.", "interop."))
    ]
    remote_core = sorted(set(framework_ids + core_ids + lightweight_legacy + [security_ids[0]]))
    remote_release = sorted(
        set(
            remote_core
            + security_ids[1:]
            + structured_legacy
            + [
                "interop.remote-libsrtp",
                "interop.remote-proxies",
                "interop.browser-dtmf",
                "report.remote-aggregate",
                "source.remote-final",
            ]
            + [f"perf.media-burst.{scenario}" for scenario in BURST_SCENARIOS]
        )
    )
    remote_release_legacy_coverage = sorted(
        set(record["id"] for record in records) - set(remote_release)
    )
    return {
        "schema": SCHEMA,
        "catalog_version": 1,
        "legacy_source": {
            "schema": source_payload["schema"],
            "run_id": source_payload["run_id"],
            "record_count": len(records),
            "path": source.relative_to(root).as_posix(),
        },
        "workspace_package_count": len(package_rows),
        "profiles": {
            "legacy-full": sorted(
                set(
                    [record["id"] for record in records]
                    + ["source.remote-clean", "interop.remote-proxies"]
                    + [f"perf.media-burst.{scenario}" for scenario in BURST_SCENARIOS]
                )
            ),
            "remote-core": remote_core,
            "remote-release": remote_release,
        },
        "remote_release_legacy_coverage": {
            "required_legacy_count": len(records),
            "profile_legacy_count": sum(
                gate.get("legacy") is not None
                for gate in legacy
                if gate["id"] in set(remote_release)
            ),
            "unautomated_legacy_ids": remote_release_legacy_coverage,
        },
        "gates": gates,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    source = args.source if args.source.is_absolute() else root / args.source
    output = args.output if args.output.is_absolute() else root / args.output
    catalog = build_catalog(root, source)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(catalog, indent=2, sort_keys=True) + "\n")
    print(
        f"wrote {len(catalog['gates'])} gates; "
        f"{catalog['legacy_source']['record_count']} canonical legacy mappings"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
