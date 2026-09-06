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
PROXY_INTEROP_PEERS = ["kamailio", "opensips"]
PROXY_INTEROP_ORDERS = ["rvoip-first", "peer-first"]
PROXY_INTEROP_TRANSPORTS = ["udp", "tcp", "tls"]
PROXY_INTEROP_GATE_IDS = [
    f"interop.remote-proxies.{peer}.{order}.{transport}"
    for peer in PROXY_INTEROP_PEERS
    for order in PROXY_INTEROP_ORDERS
    for transport in PROXY_INTEROP_TRANSPORTS
]
PROXY_PBX_PEERS = ["kamailio", "opensips"]
PROXY_PBX_GATE_IDS = [
    f"interop.proxy-pbx.{peer}.{stage}"
    for peer in PROXY_PBX_PEERS
    for stage in ("up", "matrix", "down")
]
# One cell per (variant, transport); each sweeps every mode that variant has.
AMR_RATE_SWEEP_CELLS = [
    ("amrnb", "UDP"),
    ("amrwb", "UDP"),
    ("amrnb", "TLS"),
    ("amrwb", "TLS"),
]
AMR_RATE_SWEEP_CELL_IDS = [
    f"interop.amr-rate-sweep.{profile}-{transport.lower()}"
    for profile, transport in AMR_RATE_SWEEP_CELLS
]
AMR_RATE_SWEEP_GATE_IDS = [
    "interop.amr-rate-sweep.up",
    *AMR_RATE_SWEEP_CELL_IDS,
    "interop.amr-rate-sweep.down",
]
JAMBONZ_GATE_IDS = [
    "interop.jambonz-latest",
    "interop.jambonz-up",
    "interop.jambonz-matrix",
    "interop.jambonz-down",
]
CURRENT_PERFORMANCE_EVALUATION_GATE_IDS = ["perf.canonical-2k-current"]
COMMAND_OVERRIDES = {
    "security.advisory-audit": [
        "cargo",
        "deny",
        "check",
        "advisories",
        "bans",
        "sources",
    ],
    # Keep the environment assignment as one argv element. The historical
    # shell-rendered command records it as `RUSTDOCFLAGS=-D warnings`, which
    # shlex would otherwise split into an attempted `warnings` executable.
    "build.rustdoc": [
        "env",
        "RUSTDOCFLAGS=-D warnings",
        "cargo",
        "doc",
        "--locked",
        "--workspace",
        "--all-features",
        "--no-deps",
    ],
    "interop.asterisk-matrix": [
        "env",
        "PBX_OUT_ROOT={artifact_dir}",
        "PBX_REPORT_APPEND=1",
        "PBX_G729_PROFILES=g729a g729ab",
        # The runner PBX images build AMR from source (Asterisk: pinned
        # traud/asterisk-amr patches; FreeSWITCH: mod_amr/mod_amrwb against
        # the Apache-2.0 codec libraries), so the probe must FIND the codec:
        # a lab losing it fails the AMR rows loudly instead of skipping.
        "PBX_REQUIRE_AMR=1",
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
        # The runner PBX images build AMR from source (Asterisk: pinned
        # traud/asterisk-amr patches; FreeSWITCH: mod_amr/mod_amrwb against
        # the Apache-2.0 codec libraries), so the probe must FIND the codec:
        # a lab losing it fails the AMR rows loudly instead of skipping.
        "PBX_REQUIRE_AMR=1",
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
# AMR: the RFC 4867 depacketizer on attacker bytes, the decoder DSP behind it,
# and the encoders on arbitrary PCM. Separate because these have no legacy
# record and are synthesized by `amr_fuzz_gates`.
AMR_FUZZ_TARGETS = {
    "amr-unpack": "amr_unpack",
    "amr-decode": "amr_decode",
    "amr-encode": "amr_encode",
}

for _suffix, _target in {**FUZZ_TARGETS, **AMR_FUZZ_TARGETS}.items():
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
    subcommand = cargo + 1
    if subcommand < len(argv) and argv[subcommand].startswith("+"):
        subcommand += 1
    if subcommand < len(argv) and argv[subcommand] == "fmt":
        # `cargo fmt` is a rustfmt proxy and does not accept Cargo's --locked.
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
        return [
            "crates/sip/**",
            "crates/media/**",
            "infra/release-runners/interop-lifecycle.sh",
            "infra/release-runners/pbx/**",
        ]
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
        if gate_id in {"perf.monolithic-soak", "perf.soak-candidate"}:
            return "gcp-performance-soak-long"
        if gate_id == "perf.media-burst-matrix":
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
        return ["source.clean-start", *CURRENT_PERFORMANCE_EVALUATION_GATE_IDS]
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
        if gate_id in {"report.perf-evidence-capture", "report.performance-metrics"}:
            perf.extend(CURRENT_PERFORMANCE_EVALUATION_GATE_IDS)
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
    timeout_minutes = max(5, math.ceil(duration / 60 * 2) + 5)
    if record["kind"] == "cargo":
        # Historical durations measure the command after a warm local build.
        # Release shards can begin cold, so leave enough time to compile and
        # then execute the required test instead of timing out mid-build.
        timeout_minutes = max(timeout_minutes, 20)
    if resource_class(record).startswith("gcp-"):
        # GCP workers start without a warm target directory. Preserve enough
        # headroom for the first release build as well as the measured gate.
        timeout_minutes = max(timeout_minutes, 20)
    expected_outputs = ["receipt.json", "command.log"]
    if record["id"] == "report.performance-metrics":
        expected_outputs.extend(
            [
                "current-performance-evaluation.json",
                "current-performance-evaluation.md",
                "current-performance-artifact-index.json",
            ]
        )
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
        "timeout_minutes": timeout_minutes,
        "retry_on_exit_codes": [75],
        "max_infrastructure_retries": 1,
        "affected_crates": affected_crates(command, packages),
        "affected_paths": affected_paths(record, command),
        "expected_outputs": expected_outputs,
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
        "category": "Parallel 45-crate core",
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
        "affected_paths": [f"{crate_root}/**", "scripts/ci/run_checks.py"],
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
    timeout_minutes: int = 60,
    estimated_seconds: int = 1,
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
        "timeout_minutes": timeout_minutes,
        "retry_on_exit_codes": [75],
        "max_infrastructure_retries": 1,
        "affected_crates": [],
        "affected_paths": paths or ["**"],
        "expected_outputs": ["receipt.json", "command.log"],
        "estimated_seconds": estimated_seconds,
        "always_fresh": always_fresh,
        "legacy": None,
    }


def amr_fuzz_gates() -> list[dict[str, Any]]:
    """AMR fuzz targets, which post-date the canonical 108-gate run.

    The other fuzz gates ride in as legacy records with a COMMAND_OVERRIDES
    entry; these have no legacy record to attach to, so they are synthesized
    with the same command shape. Three targets: the RFC 4867 depacketizer on
    attacker-controlled bytes, the decoder DSP behind it (where a corrupted
    field indexes a table), and the encoders on arbitrary PCM.
    """
    result = []
    for suffix, target in AMR_FUZZ_TARGETS.items():
        gate = synthetic_gate(
            f"security.fuzz-{suffix}",
            f"{target} libFuzzer target",
            executor="argv",
            command=COMMAND_OVERRIDES[f"security.fuzz-{suffix}"],
            dependencies=["source.remote-clean"],
            paths=[
                "crates/media/fuzz/**",
                "crates/media/codec-core/src/codecs/amr/**",
            ],
        )
        result.append(gate)
    return result


def proxy_pbx_gates() -> list[dict[str, Any]]:
    """Kamailio/OpenSIPS registrar-proxy labs with an rtpengine media relay.

    A different oracle class from the B2BUA matrices: the proxy stays in the
    signalling path via Record-Route while rtpengine relays RTP verbatim, so
    these are what prove AMR crosses a relay that never re-encodes it. Media
    flows, unlike the signalling-only `interop.remote-proxies` family, which
    this deliberately does not replace.

    Each peer is a three-gate chain — up, matrix, down — because the lab has
    to be torn down even when the matrix fails, and an aggregate cannot
    express that ordering.
    """
    paths = [
        "crates/sip/rvoip-sip/examples/pbx/**",
        "crates/media/codec-core/**",
        "crates/media/media-core/**",
        "infra/release-runners/pbx/**",
        "infra/release-runners/interop-lifecycle.sh",
    ]
    result = []
    for peer in PROXY_PBX_PEERS:
        up = synthetic_gate(
            f"interop.proxy-pbx.{peer}.up",
            f"{peer} + rtpengine lab up",
            executor="argv",
            command=[
                "bash",
                "infra/release-runners/interop-lifecycle.sh",
                f"{peer}-up",
            ],
            resource="gcp-proxy-interop",
            dependencies=["source.remote-clean"],
            paths=paths,
        )
        up["estimated_seconds"] = 120
        matrix = synthetic_gate(
            f"interop.proxy-pbx.{peer}.matrix",
            f"{peer} + rtpengine AMR passthrough matrix",
            executor="argv",
            command=[
                "env",
                "PBX_OUT_ROOT={artifact_dir}",
                "PBX_REPORT_APPEND=1",
                # The relay forwards payloads verbatim, so one lab config
                # covers all four AMR framings; the harness sweeps them.
                "PBX_REQUIRE_AMR=1",
                "{workspace}/crates/sip/rvoip-sip/examples/pbx/run.sh",
                "--pbx",
                peer,
                "--api",
                "endpoint",
                "--scenario",
                "all",
            ],
            resource="gcp-proxy-interop",
            dependencies=[f"interop.proxy-pbx.{peer}.up"],
            paths=paths,
        )
        matrix["estimated_seconds"] = 600
        matrix["timeout_minutes"] = 45
        down = synthetic_gate(
            f"interop.proxy-pbx.{peer}.down",
            f"{peer} + rtpengine lab down",
            executor="argv",
            command=[
                "bash",
                "infra/release-runners/interop-lifecycle.sh",
                f"{peer}-down",
            ],
            resource="gcp-proxy-interop",
            dependencies=[f"interop.proxy-pbx.{peer}.matrix"],
            paths=paths,
            always_fresh=True,
        )
        result.extend([up, matrix, down])

    result.append(
        synthetic_gate(
            "interop.proxy-pbx",
            "complete registrar-proxy media matrix (Kamailio and OpenSIPS + rtpengine)",
            executor="aggregate",
            dependencies=PROXY_PBX_GATE_IDS,
            paths=paths,
        )
    )
    return result


def amr_rate_sweep_gates() -> list[dict[str, Any]]:
    """Every AMR mode against a live PBX, one cell per rate.

    The ordinary AMR rows in `interop.asterisk-matrix` all run at whichever
    mode the encoder opens at, which is the highest the negotiation permits.
    That leaves the other sixteen rates with conformance-vector evidence and
    no third-party evidence at all -- so a regression that broke, say, AMR-WB
    at 12.65 kbit/s would pass the entire release gate.

    Each cell pins one rate the standard way: `Config::amr_mode_set` puts an
    RFC 4867 `mode-set` in the INVITE naming exactly that mode. `rate-sweep.sh`
    then fails the gate unless the codec was *built* at the pinned mode --
    read from media-core's `codec generation built` line, not from the
    environment variable meant to cause it -- because a cell pinned to one
    rate and a cell that ignored the pin both carry clean audio, and the
    status column alone attests to nothing about the rate.

    Both transports, because they are different claims: UDP carries RTP, TLS
    carries SDES-SRTP, and AMR payload length varies per mode, so the
    encrypted path exercises frame boundaries that a fixed-size codec never
    moves.

    Its own up/down chain rather than a link in the shared interop sequence:
    that chain tears the Asterisk lab down after `interop.asterisk-matrix`,
    and a sweep appended afterwards would find no lab.
    """
    paths = [
        "crates/sip/rvoip-sip/examples/pbx/**",
        "crates/media/codec-core/**",
        "crates/media/media-core/**",
        "crates/sip/rvoip-sip/src/adapters/media_adapter.rs",
        "infra/release-runners/interop-lifecycle.sh",
    ]
    up = synthetic_gate(
        "interop.amr-rate-sweep.up",
        "Asterisk lab up for the AMR per-rate sweep",
        executor="argv",
        command=["bash", "infra/release-runners/interop-lifecycle.sh", "asterisk-up"],
        resource="gcp-interop",
        dependencies=["source.remote-clean"],
        paths=paths,
    )
    up["estimated_seconds"] = 120

    cells = []
    for profile, transport in AMR_RATE_SWEEP_CELLS:
        cell = synthetic_gate(
            f"interop.amr-rate-sweep.{profile}-{transport.lower()}",
            f"AMR {profile} every mode over {transport}, each pinned by mode-set",
            executor="argv",
            command=[
                "env",
                "PBX_OUT_ROOT={artifact_dir}",
                "PBX_REPORT_APPEND=1",
                "PBX_REQUIRE_AMR=1",
                "{workspace}/crates/sip/rvoip-sip/examples/pbx/rate-sweep.sh",
                "--pbx",
                "asterisk",
                "--profile",
                profile,
                "--transport",
                transport,
            ],
            resource="gcp-interop",
            dependencies=["interop.amr-rate-sweep.up"],
            paths=paths,
        )
        # Eight or nine cells, each a full call plus teardown.
        cell["estimated_seconds"] = 420
        cell["timeout_minutes"] = 45
        cells.append(cell)

    down = synthetic_gate(
        "interop.amr-rate-sweep.down",
        "Asterisk lab down after the AMR per-rate sweep",
        executor="argv",
        command=["bash", "infra/release-runners/interop-lifecycle.sh", "asterisk-down"],
        resource="gcp-interop",
        dependencies=[cell["id"] for cell in cells],
        paths=paths,
        # Runs even when a sweep fails: a lab left up holds its ports and the
        # next suite to want them fails for a reason that looks unrelated.
        always_fresh=True,
    )

    aggregate = synthetic_gate(
        "interop.amr-rate-sweep",
        "complete AMR per-rate matrix (17 modes x UDP and TLS+SRTP)",
        executor="aggregate",
        dependencies=[cell["id"] for cell in cells],
        paths=paths,
    )
    return [up, *cells, down, aggregate]


def jambonz_interop_gates() -> list[dict[str, Any]]:
    """Latest stable OSS Jambonz as a first-class release B2BUA peer."""
    paths = [
        "crates/sip/rvoip-sip/examples/pbx/**",
        "crates/media/**",
        "infra/release-runners/interop-lifecycle.sh",
        "infra/release-runners/pbx/jambonz/**",
    ]
    latest = synthetic_gate(
        "interop.jambonz-latest",
        "Jambonz latest stable open-source release pin",
        executor="argv",
        command=[
            "python3",
            "infra/release-runners/pbx/jambonz/verify-latest.py",
            "--receipt",
            "{artifact_dir}/latest.json",
        ],
        resource="gcp-interop",
        dependencies=["source.remote-clean", "interop.freeswitch-down-after"],
        paths=paths,
        always_fresh=True,
    )
    up = synthetic_gate(
        "interop.jambonz-up",
        "latest pinned Jambonz OSS lab up",
        executor="argv",
        command=[
            "env",
            "JAMBONZ_RECEIPT_DIR={artifact_dir}",
            "bash",
            "infra/release-runners/interop-lifecycle.sh",
            "jambonz-up",
        ],
        resource="gcp-interop",
        dependencies=["interop.jambonz-latest"],
        paths=paths,
    )
    up["estimated_seconds"] = 240
    up["timeout_minutes"] = 30
    matrix = synthetic_gate(
        "interop.jambonz-matrix",
        "Jambonz OSS PBX/B2BUA matrix",
        executor="argv",
        command=[
            "env",
            "PBX_OUT_ROOT={artifact_dir}",
            "PBX_REPORT_APPEND=1",
            # The pinned Jambonz profile admits the two mandatory G.711
            # encodings. Sweep them separately so a combined offer cannot
            # satisfy both assertions by selecting PCMU twice.
            "PBX_G711_PROFILES=pcmu pcma",
            "{workspace}/crates/sip/rvoip-sip/examples/pbx/run.sh",
            "--pbx",
            "jambonz",
            "--api",
            "all",
            "--scenario",
            "all",
            "--transport",
            "UDP",
        ],
        resource="gcp-interop",
        dependencies=["interop.jambonz-up"],
        paths=paths,
    )
    matrix["estimated_seconds"] = 600
    matrix["timeout_minutes"] = 45
    down = synthetic_gate(
        "interop.jambonz-down",
        "Jambonz OSS lab teardown and residue proof",
        executor="argv",
        command=[
            "env",
            "JAMBONZ_RECEIPT_DIR={artifact_dir}",
            "bash",
            "infra/release-runners/interop-lifecycle.sh",
            "jambonz-down",
        ],
        resource="gcp-interop",
        dependencies=["interop.jambonz-matrix"],
        paths=paths,
        always_fresh=True,
    )
    return [latest, up, matrix, down]


def proxy_interop_gates() -> list[dict[str, Any]]:
    paths = [
        "crates/sip/sip-proxy/**",
        "crates/sip/sip-transport/**",
        "crates/sip/sip-core/**",
    ]
    result = []
    for peer in PROXY_INTEROP_PEERS:
        for order in PROXY_INTEROP_ORDERS:
            for transport in PROXY_INTEROP_TRANSPORTS:
                gate = synthetic_gate(
                    f"interop.remote-proxies.{peer}.{order}.{transport}",
                    f"{peer} {order} {transport} proxy interoperability",
                    executor="argv",
                    command=[
                        "env",
                        "PROXY_INTEROP_ARTIFACT_DIR={artifact_dir}/proxy-interop",
                        "bash",
                        "crates/sip/sip-proxy/tests/interop/scripts/beta_gate.sh",
                        peer,
                        order,
                        transport,
                    ],
                    resource="gcp-proxy-interop",
                    dependencies=["source.remote-clean"],
                    paths=paths,
                )
                gate["estimated_seconds"] = 240
                gate["timeout_minutes"] = 30
                result.append(gate)

    result.append(
        synthetic_gate(
            "interop.remote-proxies",
            "complete Kamailio and OpenSIPS proxy matrix",
            executor="aggregate",
            dependencies=PROXY_INTEROP_GATE_IDS,
            paths=paths,
        )
    )
    return result


def infrastructure_preflight_gates() -> list[dict[str, Any]]:
    """Build the full-shape, short-running GCP orchestration acceptance profile.

    The release profile uses six short-performance workers, seven burst/soak
    workers, two long-soak workers, one stateful interoperability worker, and
    two proxy-interoperability workers. Keep the same 100-vCPU fanout here so a
    controller, quota, startup, evidence, or cleanup defect is found before a
    real hour-long qualification begins.
    """
    paths = [
        ".github/workflows/release-qualify.yml",
        "infra/release-runners/gcp-release-startup.sh",
        "infra/release-runners/release-infrastructure-preflight.sh",
        "scripts/release/build_gate_catalog.py",
        "scripts/release/gates.py",
        "scripts/release/gates.json",
        "scripts/release/gcp_fanout.py",
    ]
    result = []
    for resource, count in (
        ("gcp-performance", 6),
        ("gcp-performance-soak", 7),
        ("gcp-performance-soak-long", 2),
        ("gcp-interop", 1),
        ("gcp-proxy-interop", 2),
    ):
        suffix = resource.removeprefix("gcp-")
        for index in range(1, count + 1):
            result.append(
                synthetic_gate(
                    f"preflight.{suffix}-{index:02d}",
                    f"{resource} worker {index:02d} infrastructure acceptance",
                    executor="argv",
                    command=[
                        "bash",
                        "infra/release-runners/release-infrastructure-preflight.sh",
                        resource,
                        "{artifact_dir}",
                    ],
                    resource=resource,
                    paths=paths,
                    always_fresh=True,
                )
            )
    return result


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


def facade_feature_bundle_gate() -> dict[str, Any]:
    """Compile every public facade bundle as part of exact-candidate qualification."""
    gate = synthetic_gate(
        "build.facade-feature-bundles",
        "rvoip facade deployment bundle matrix",
        executor="argv",
        command=[
            "python3",
            "scripts/ci/run_checks.py",
            "specialty",
            "--name",
            "release-facade-feature-bundles",
            "--gate",
            "facade-feature-bundles",
            "--output",
            "{artifact_dir}/nested-ci-receipt.json",
        ],
        resource="github-standard",
        dependencies=["source.remote-clean"],
        paths=[
            "Cargo.lock",
            "crates/rvoip/Cargo.toml",
            "crates/foundation/rvoip-core/Cargo.toml",
            "crates/foundation/rvoip-core/src/media_graph.rs",
            "docs/FEATURE_BUNDLES.md",
            "scripts/ci/check_facade_feature_bundles.py",
            "scripts/ci/run_checks.py",
        ],
    )
    gate["expected_outputs"].append("nested-ci-receipt.json")
    # The six checks share one target directory and heavily reuse compilation;
    # this is the observed warm-run cost used by the hosted shard packer.
    gate["estimated_seconds"] = 600
    return gate


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
    if len(package_rows) != 45:
        raise RuntimeError(f"expected 45 workspace packages, found {len(package_rows)}")
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
            "45-package release inventory audit",
            executor="argv",
            command=["python3", "scripts/release.py", "audit"],
            dependencies=["source.remote-clean"],
            paths=["Cargo.toml", "Cargo.lock", "crates/**/Cargo.toml", "scripts/release.py"],
            always_fresh=True,
        ),
        synthetic_gate(
            "interop.remote-libsrtp",
            "pinned SRTP and DTLS-SRTP interoperability",
            executor="argv",
            command=[
                "python3",
                "scripts/ci/run_checks.py",
                "specialty",
                "--name",
                "release-rtp-interop",
                "--gate",
                "rtp-interop",
                "--output",
                "{artifact_dir}/nested-ci-receipt.json",
            ],
            resource="gcp-interop",
            dependencies=["source.remote-clean"],
            paths=[
                "Cargo.lock",
                "scripts/test_libsrtp_interop.sh",
                "crates/media/media-core/src/relay/**",
                "crates/media/rtp-core/**",
                "crates/sip/rvoip-sip/Cargo.toml",
                "crates/sip/rvoip-sip/src/adapters/dtls_negotiator.rs",
                "crates/sip/rvoip-sip/src/adapters/media_adapter.rs",
                "crates/sip/rvoip-sip/src/api/events.rs",
                "crates/sip/rvoip-sip/src/api/unified.rs",
                "crates/sip/rvoip-sip/tests/dtls_srtp_call_integration.rs",
                "scripts/ci/run_checks.py",
            ],
        ),
        facade_feature_bundle_gate(),
        synthetic_gate(
            "perf.canonical-2k-current",
            "three-pass exact-candidate 2,000-CPS performance evaluation",
            executor="argv",
            command=[
                "env",
                "RVOIP_CANONICAL_EVAL_OUTPUT={artifact_dir}",
                "{workspace}/crates/sip/rvoip-sip/scripts/canonical_2k_release_eval.sh",
            ],
            resource="gcp-performance-soak-long",
            dependencies=["source.remote-clean"],
            paths=[
                "Cargo.lock",
                "Cargo.toml",
                "crates/foundation/**",
                "crates/media/**",
                "crates/sip/rvoip-sip/**",
                "scripts/release/**",
            ],
            always_fresh=True,
            timeout_minutes=120,
            estimated_seconds=4_500,
        ),
        *proxy_interop_gates(),
        *proxy_pbx_gates(),
        *jambonz_interop_gates(),
        *amr_rate_sweep_gates(),
        *amr_fuzz_gates(),
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
                # `crates/core/` has not existed since the directory reorg that
                # moved this crate to `crates/foundation/`; the glob matched
                # nothing, so rvoip-core changes never selected this gate.
                "crates/foundation/rvoip-core/**",
            ],
        ),
    ]
    synthetic.extend(burst_scenario_gates())
    preflight = infrastructure_preflight_gates()
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
    gates = legacy + synthetic + preflight + core + security + final

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
                *PROXY_INTEROP_GATE_IDS,
                "interop.proxy-pbx",
                *PROXY_PBX_GATE_IDS,
                *JAMBONZ_GATE_IDS,
                *CURRENT_PERFORMANCE_EVALUATION_GATE_IDS,
                "interop.amr-rate-sweep",
                *AMR_RATE_SWEEP_GATE_IDS,
                *[f"security.fuzz-{suffix}" for suffix in AMR_FUZZ_TARGETS],
                "interop.browser-dtmf",
                "build.facade-feature-bundles",
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
                    + CURRENT_PERFORMANCE_EVALUATION_GATE_IDS
                    + PROXY_INTEROP_GATE_IDS
                    + [f"perf.media-burst.{scenario}" for scenario in BURST_SCENARIOS]
                )
            ),
            "remote-preflight": sorted(gate["id"] for gate in preflight),
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
