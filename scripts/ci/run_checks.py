#!/usr/bin/env python3
"""Run CI commands and always write a machine-readable command receipt."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
import tomllib
from typing import Any, NamedTuple

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fetch_webex_byova_protos as webex_byova_protos  # noqa: E402
import sip_test_partitions  # noqa: E402


SCHEMA = "rvoip-ci-command-receipt-v1"
PACKAGE = re.compile(r"^[A-Za-z0-9_-]+$")
GATE = re.compile(r"^[A-Za-z0-9_-]+$")
TARGET = re.compile(r"^[A-Za-z0-9_-]+$")

# The optional-codec gates, one per package. Keep in step with the matching
# `specialty_rules` entries in `policy.json` and the `--specialty-gate` flags
# in `.github/workflows/main-ci.yml`: a gate named in only one of the three
# either never runs or fails the plan.
CODEC_FEATURE_GATES = {
    "codec-features-codec-core": "rvoip-codec-core",
    "codec-features-media-core": "rvoip-media-core",
    "codec-features-sip": "rvoip-sip",
    "codec-features-core": "rvoip-core",
}

# `codec-features-sip` was the PR Gate's long pole (46-55 min). Cold, its
# test build took 23 min, its test run 14 min (8.4 of them one target,
# audio_roundtrip_integration) and its clippy 16.5 min, all in one job. It now
# runs as four concurrent jobs over the same commands and features: clippy
# alone, and the test targets in three duration-balanced partitions, each job
# compiling only its own targets. The first partition also runs the library,
# binary and example tests. Every listed gate shares codec-features-sip's
# `policy.json` patterns and main-ci flag, so any change that selected the
# old gate selects all four.
CODEC_FEATURE_SIP_SHARDS = {
    "codec-features-sip": ("test", 0),
    "codec-features-sip-tests-2": ("test", 1),
    "codec-features-sip-tests-3": ("test", 2),
    "codec-features-sip-clippy": ("clippy", None),
}
CODEC_FEATURE_SIP_TEST_PARTITIONS = 3
# Every gate that runs optional-codec tests, including the rvoip-sip shards.
ALL_CODEC_FEATURE_GATES = tuple(sorted({*CODEC_FEATURE_GATES, *CODEC_FEATURE_SIP_SHARDS}))

# Feature-gated targets no other gate builds, one gate per crate family. Keep
# in step with the matching `specialty_rules` entries in `policy.json` and the
# `--specialty-gate` flags in `.github/workflows/main-ci.yml`, exactly like the
# codec gates above; `test_feature_gate_coverage.py` fails when a target with
# `required-features`, or a test file behind `#![cfg(feature = ...)]`, is
# reached by no gate at all.
#
# The shards build default features, so `cargo test --tests` skips a target
# whose `required-features` are off without a word, and a `#![cfg(feature)]`
# test compiles to an empty binary that passes. `rvoip-webrtc`'s
# `sip_webrtc_acceptance` sat in that hole from September and rotted. Each
# entry below is one `cargo test` for one package. By default it runs the
# package's library, binary, example and integration tests with the union of
# the features its feature-gated targets need, so the library's own
# feature-gated unit tests run too. An entry names `tests` only when the
# features would break the rest of the package (a build with no crypto
# provider, say). Packages that share most of a dependency tree share a gate,
# and so one job and one cache.
class FeatureRun(NamedTuple):
    package: str
    features: str
    no_default_features: bool = False
    tests: tuple[str, ...] = ()


FEATURE_TARGET_GATES: dict[str, tuple[FeatureRun, ...]] = {
    "feature-targets-amazon-connect": (FeatureRun("rvoip-amazon-connect", "server"),),
    "feature-targets-moq": (
        FeatureRun("rvoip-moq", "relay-runtime"),
        FeatureRun("rvoip-redis", "moq"),
    ),
    # rvoip-rtc's crypto-provider contract: both built-in providers side by
    # side, and a build with neither that must fail with an actionable error.
    # Only those targets: the rest of the crate needs exactly one provider.
    "feature-targets-rtc": (
        FeatureRun(
            "rvoip-rtc",
            "crypto-aws-lc-rs",
            tests=("crypto_provider_peer_connections",),
        ),
        FeatureRun(
            "rvoip-rtc",
            "",
            no_default_features=True,
            tests=("no_builtin_crypto_provider",),
        ),
    ),
    "feature-targets-rtp-core": (FeatureRun("rvoip-rtp-core", "dtls-webrtc"),),
    "feature-targets-sip-stack": (
        FeatureRun("rvoip-sip-core", "generated-validation,lenient_parsing"),
        FeatureRun("rvoip-sip-dialog", "generated-validation"),
        FeatureRun("rvoip-sip-transport", "dns"),
    ),
    "feature-targets-users-core": (
        FeatureRun("rvoip-users-core", "auth-core,client,postgres"),
    ),
    # Every rvoip-webrtc feature but `interop-browser` (nightly-interop.yml
    # runs it against a real Chromium) and the three no target needs:
    # `client-cpal` (an audio device), `test-hooks` and `turn-fork-candidate`.
    # `soak-1h` is a 60-second smoke unless SOAK_SECS says otherwise.
    "feature-targets-webrtc": (
        FeatureRun(
            "rvoip-webrtc",
            "bridge-quic,client-video-h264,client-video-vp8,comprehensive,"
            "signaling-whip,signaling-ws,soak-1h,tls-rustls",
        ),
    ),
    "feature-targets-websocket": (FeatureRun("rvoip-websocket", "media-webrtc,wss"),),
}
# `client-video-vp8` generates libvpx bindings against the installed headers
# with env-libvpx-sys's bindgen 0.68, which needs libclang 18 or older
# (rust-bindgen#2824): these gates install libvpx-dev and run on ubuntu-24.04,
# like rustdoc's `--all-features` build.
LIBVPX_GATES = ("rustdoc", "feature-targets-webrtc", "workspace-all-features")


def feature_run_argv(run: FeatureRun) -> list[str]:
    argv = ["cargo", "test", "--locked", "-p", run.package]
    if run.no_default_features:
        argv.append("--no-default-features")
    if run.features:
        argv.extend(("--features", run.features))
    if run.tests:
        argv.extend(arg for name in run.tests for arg in ("--test", name))
    else:
        argv.extend(("--lib", "--tests", "--bins", "--examples"))
    return argv


def feature_target_commands(
    gate: str,
) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    return [(feature_run_argv(run), None, None) for run in FEATURE_TARGET_GATES[gate]]


# Every `rvoip-sip` feature except the release-only `perf-*` harness. Written
# out rather than computed so the command is inspectable from here; the
# accompanying test derives the same set from the manifest and fails if a new
# feature is added without a decision about this gate.
NON_PERF_SIP_FEATURES = (
    "all-codecs,amr,amr-nb,amr-wb,dev-insecure-tls,dhat,dtls-srtp,event-history,g729,"
    "generated-validation,opus,opus-sim,persistence,test-hooks,tokio-console"
)


# Every rvoip-sip lane builds with `test-hooks`. Integration tests and process
# fixtures that need test-only knobs (second-scale RFC 4028 session timers via
# `Config::session_timer_allow_short_intervals_for_testing`) are compiled out
# of a default-feature build, so a lane without the feature would pass while
# running none of them. The feature only adds test hooks; it never changes
# production behavior, so enabling it everywhere loses no default coverage.
SIP_LANE_FEATURES = ["--features", "test-hooks"]

# The release catalog's `build.rustdoc` gate (scripts/release/gates.json),
# verbatim. No shard builds documentation, so before this gate a broken
# intra-doc link passed every PR check and first failed the 0.4.0 release
# qualification (#315). The accompanying test fails if this argv drifts from
# the release gate's, so a PR runs exactly what qualification will run.
RUSTDOC_COMMAND = (
    "env",
    "RUSTDOCFLAGS=-D warnings",
    "cargo",
    "doc",
    "--locked",
    "--workspace",
    "--all-features",
    "--no-deps",
)
# Cargo unifies features across every package a command selects, so a
# workspace-wide `--all-features` build compiles code that no per-package
# build ever sees: one crate's optional dependency adds trait impls another
# crate's tests then trip over. #424 broke this build on main -- rkyv,
# reached through dtls 0.17, adds `PartialEq<ArchivedSocketAddr> for
# SocketAddr`, which made a `"...".parse().unwrap()` comparison in
# rvoip-bench-uas's tests ambiguous -- and no gate noticed, because every
# other gate selects packages. This gate must stay workspace-wide: splitting
# it by package would lose exactly the unification it exists to build.
# rustdoc's `--all-features` build is the other workspace-wide one, but it
# never compiles test, example or bench targets.
WORKSPACE_ALL_FEATURES_COMMAND = (
    "cargo",
    "clippy",
    "--locked",
    "--workspace",
    "--all-features",
    "--all-targets",
    "--",
    "-D",
    "warnings",
)
WORKSPACE_ALL_FEATURES_GATE = "workspace-all-features"
SIP_SHARD_FEATURES = ["--features", "rvoip-sip/test-hooks"]


class CheckError(RuntimeError):
    """Invalid CI input or environment."""


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def capture_version(argv: list[str], root: Path) -> str:
    completed = subprocess.run(argv, cwd=root, text=True, capture_output=True, check=False)
    output = (completed.stdout or completed.stderr).splitlines()
    return output[0].strip() if output else "unavailable"


def command_record(
    argv: list[str],
    *,
    root: Path,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
) -> dict[str, Any]:
    started = utc_now()
    start = time.monotonic()
    print(f"::group::{' '.join(argv)}", flush=True)
    try:
        completed = subprocess.run(
            argv,
            cwd=cwd or root,
            env=env,
            check=False,
        )
        exit_code = completed.returncode
    except OSError as error:
        print(f"cannot execute {argv[0]}: {error}", file=sys.stderr)
        exit_code = 127
    finally:
        print("::endgroup::", flush=True)
    environment_overrides = {
        key: value for key, value in (env or {}).items() if os.environ.get(key) != value
    }
    return {
        "argv": argv,
        "working_directory": (cwd or root).relative_to(root).as_posix()
        if (cwd or root) != root
        else ".",
        "started_at": started,
        "duration_seconds": round(time.monotonic() - start, 3),
        "exit_code": exit_code,
        "environment_overrides": environment_overrides,
    }


def package_args(packages_csv: str) -> list[str]:
    packages = sorted(set(filter(None, packages_csv.split(","))))
    if not packages or any(not PACKAGE.fullmatch(package) for package in packages):
        raise CheckError(f"invalid package selection: {packages_csv!r}")
    selected = [value for package in packages for value in ("-p", package)]
    if "rvoip-sip" in packages:
        selected.extend(SIP_SHARD_FEATURES)
    return selected


# The examples workspace has its own lockfile, which no PR shard resolves:
# a crate manifest change left it stale until release preparation refreshed
# it, hiding the drift until release day. Every PR now checks it with the
# same command release.py uses to validate release lockfiles.
EXAMPLES_LOCK_COMMAND = (
    "bash",
    "-c",
    "cargo metadata --manifest-path examples/Cargo.toml --format-version 1 --locked > /dev/null",
)


def policy_commands() -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    return [
        (["cargo", "fmt", "--all", "--", "--check"], None, None),
        (list(EXAMPLES_LOCK_COMMAND), None, None),
        (
            [
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "scripts/ci",
                "-p",
                "test_*.py",
            ],
            None,
            None,
        ),
    ]


def facade_bundle_names(root: Path) -> list[str]:
    manifest = root / "crates/rvoip/Cargo.toml"
    with manifest.open("rb") as handle:
        data = tomllib.load(handle)
    bundles = data["package"]["metadata"]["rvoip"]["feature-bundles"]
    names = [bundle["feature"] for bundle in bundles]
    if not names or any(not GATE.fullmatch(name) for name in names):
        raise CheckError("invalid or empty rvoip facade feature-bundle catalog")
    return names


def shard_commands(packages_csv: str) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    selected = package_args(packages_csv)
    return [
        (
            [
                "cargo",
                "test",
                "--locked",
                "--lib",
                "--tests",
                "--bins",
                "--examples",
                *selected,
            ],
            None,
            None,
        ),
        (["cargo", "clippy", "--locked", "--all-targets", *selected], None, None),
    ]


def shard_test_commands(
    packages_csv: str,
) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    selected = package_args(packages_csv)
    return [
        (
            [
                "cargo",
                "test",
                "--locked",
                "--lib",
                "--tests",
                "--bins",
                "--examples",
                *selected,
            ],
            None,
            None,
        )
    ]


def shard_clippy_commands(
    packages_csv: str,
) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    selected = package_args(packages_csv)
    return [(["cargo", "clippy", "--locked", "--all-targets", *selected], None, None)]


def sip_core_commands() -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    return [
        (
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "rvoip-sip",
                *SIP_LANE_FEATURES,
                "--lib",
                "--bins",
                "--examples",
            ],
            None,
            None,
        )
    ]


def sip_clippy_commands() -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    return [
        (
            [
                "cargo",
                "clippy",
                "--locked",
                "-p",
                "rvoip-sip",
                *SIP_LANE_FEATURES,
                "--all-targets",
            ],
            None,
            None,
        )
    ]


def sip_integration_commands(
    targets_csv: str,
) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    targets = sorted(set(filter(None, targets_csv.split(","))))
    if not targets or any(not TARGET.fullmatch(target) for target in targets):
        raise CheckError(f"invalid SIP integration target selection: {targets_csv!r}")
    argv = ["cargo", "test", "--locked", "-p", "rvoip-sip", *SIP_LANE_FEATURES]
    for target in targets:
        argv.extend(("--test", target))
    return [(argv, None, None)]


def sip_fixture_commands(
    targets_csv: str,
    examples_csv: str,
    root: Path,
) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    targets = sorted(set(filter(None, targets_csv.split(","))))
    examples = sorted(set(filter(None, examples_csv.split(","))))
    if not targets or any(not TARGET.fullmatch(target) for target in targets):
        raise CheckError(f"invalid SIP process-fixture target selection: {targets_csv!r}")
    if not examples or any(not TARGET.fullmatch(example) for example in examples):
        raise CheckError(f"invalid SIP process-fixture example selection: {examples_csv!r}")

    target_root = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    if not target_root.is_absolute():
        target_root = root / target_root
    prebuilt_dir = target_root / "debug" / "examples"
    fixture_env = {
        **os.environ,
        "RVOIP_SIP_PREBUILT_EXAMPLE_DIR": str(prebuilt_dir),
    }

    build = ["cargo", "build", "--locked", "-p", "rvoip-sip", *SIP_LANE_FEATURES]
    for example in examples:
        build.extend(("--example", example))
    test = ["cargo", "test", "--locked", "-p", "rvoip-sip", *SIP_LANE_FEATURES]
    for target in targets:
        test.extend(("--test", target))
    return [(build, None, None), (test, None, fixture_env)]


def codec_sip_test_targets(root: Path) -> list[str]:
    """rvoip-sip test targets that `cargo test --tests` builds with the codec features."""

    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            str(root / "crates/sip/rvoip-sip/Cargo.toml"),
        ],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode:
        raise CheckError(completed.stderr.strip() or "cargo metadata failed")
    enabled = set(NON_PERF_SIP_FEATURES.split(","))
    packages = [
        package
        for package in json.loads(completed.stdout)["packages"]
        if package["name"] == "rvoip-sip"
    ]
    if len(packages) != 1:
        raise CheckError("expected exactly one rvoip-sip package")
    targets = sorted(
        target["name"]
        for target in packages[0]["targets"]
        if "test" in target["kind"]
        and set(target.get("required-features", [])) <= enabled
    )
    if not targets or any(not TARGET.fullmatch(name) for name in targets):
        raise CheckError("rvoip-sip test target inventory is empty or unsafe")
    return targets


def codec_sip_shard_commands(
    gate: str, root: Path
) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    kind, partition = CODEC_FEATURE_SIP_SHARDS[gate]
    selection = ["--features", NON_PERF_SIP_FEATURES]
    if kind == "clippy":
        return [
            (
                ["cargo", "clippy", "--locked", *selection, "--all-targets", "-p", "rvoip-sip"],
                None,
                None,
            )
        ]
    partitions = sip_test_partitions.partition_targets(
        codec_sip_test_targets(root), CODEC_FEATURE_SIP_TEST_PARTITIONS
    )
    tests = [arg for name in partitions[partition]["targets"] for arg in ("--test", name)]
    scope = ["--lib", "--bins", "--examples"] if partition == 0 else []
    return [
        (
            ["cargo", "test", "--locked", *selection, *scope, *tests, "-p", "rvoip-sip"],
            None,
            None,
        )
    ]


def specialty_commands(
    gate: str, root: Path
) -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    if not GATE.fullmatch(gate):
        raise CheckError(f"invalid specialty gate: {gate!r}")
    if gate.startswith("example--"):
        example = gate.removeprefix("example--")
        manifest = root / "examples" / example / "Cargo.toml"
        if not manifest.is_file():
            raise CheckError(f"unknown example project: {example}")
        return [
            (["cargo", "build", "--manifest-path", str(manifest), "--locked"], None, None)
        ]
    smoke_examples = (
        "01-quickstart-p2p",
        "02-softphone-audio",
        "06-attended-transfer",
        "07-secure-call-srtp",
        "10-call-center-b2bua",
    )
    if gate.startswith("example-smoke--"):
        example = gate.removeprefix("example-smoke--")
        if example not in smoke_examples:
            raise CheckError(f"unknown example smoke project: {example}")
        directory = root / "examples" / example
        return [
            (["cargo", "build", "--release", "--locked"], directory, None),
            (["timeout", "120", "./run_demo.sh"], directory, None),
        ]
    if gate == "examples-smoke":
        commands: list[tuple[list[str], Path | None, dict[str, str] | None]] = []
        for example in smoke_examples:
            directory = root / "examples" / example
            commands.extend(
                [
                    (["cargo", "build", "--release", "--locked"], directory, None),
                    (["timeout", "120", "./run_demo.sh"], directory, None),
                ]
            )
        return commands
    if gate == "release-tooling":
        return [
            (
                [
                    "python3",
                    "-m",
                    "unittest",
                    "scripts/test_release.py",
                    "scripts/test_release_carry_forward_attestation.py",
                    "scripts/test_release_exception_attestation.py",
                    "scripts/test_release_aws_fanout.py",
                    "scripts/test_release_gates.py",
                    "scripts/test_release_notes.py",
                    "scripts/test_release_prebuilt_performance.py",
                    "scripts/test_render_qualification_reports.py",
                ],
                None,
                None,
            ),
            (
                [
                    "python3",
                    "-m",
                    "unittest",
                    "crates/sip/rvoip-sip/scripts/test_beta_attestation.py",
                    "crates/sip/rvoip-sip/scripts/test_beta_performance_gate_metrics.py",
                    "crates/sip/rvoip-sip/scripts/test_beta_gate_source.py",
                    "crates/sip/rvoip-sip/scripts/test_perf_audit.py",
                    "crates/sip/rvoip-sip/scripts/test_canonical_2k_evidence.py",
                    "crates/sip/rvoip-sip/scripts/test_perf_2k_acceptance.py",
                    "crates/sip/rvoip-sip/scripts/test_perf_2k_baseline.py",
                    "crates/sip/rvoip-sip/scripts/test_perf_regression_baseline.py",
                    "crates/sip/rvoip-sip/scripts/test_perf_cargo_artifact.py",
                    "crates/sip/rvoip-sip/scripts/test_docker_peer_snapshot.py",
                ],
                None,
                None,
            ),
            (
                [
                    "python3",
                    "-m",
                    "unittest",
                    "discover",
                    "-s",
                    "crates/sip/sip-proxy/tests/interop/scripts",
                    "-p",
                    "test_*.py",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "+nightly",
                    "fuzz",
                    "check",
                    "--fuzz-dir",
                    "crates/sip/fuzz",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "+nightly",
                    "fuzz",
                    "check",
                    "--fuzz-dir",
                    "crates/media/fuzz",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                ],
                None,
                None,
            ),
        ]
    facade_part = re.fullmatch(r"facade-feature-bundles(?:--part-([12]))?", gate)
    if facade_part:
        # The whole gate, or one of the two parts the PR and main plans run
        # in parallel (pr_plan.SPECIALTY_PARTS): bundles interleaved so each
        # part gets one of the two full bundles.
        part = int(facade_part.group(1)) if facade_part.group(1) else None
        bundles = facade_bundle_names(root)
        if part is not None:
            bundles = bundles[part - 1 :: 2]
        commands: list[tuple[list[str], Path | None, dict[str, str] | None]] = []
        if part in (None, 1):
            commands.append(
                (
                    [
                        "python3",
                        "scripts/ci/check_facade_feature_bundles.py",
                        "--verify-resolved",
                    ],
                    None,
                    None,
                )
            )
        for feature in bundles:
            commands.append(
                (
                    [
                        "cargo",
                        "test",
                        "--locked",
                        "-p",
                        "rvoip",
                        "--lib",
                        "--no-default-features",
                        "--features",
                        feature,
                    ],
                    None,
                    None,
                )
            )
        return commands
    if gate in FEATURE_TARGET_GATES:
        return feature_target_commands(gate)
    if gate in CODEC_FEATURE_SIP_SHARDS:
        return codec_sip_shard_commands(gate, root)
    if gate in CODEC_FEATURE_GATES:
        # The shards run `cargo test -p <crate>` with default features only.
        # `rvoip-codec-core` defaults to `["g711"]` and `rvoip-media-core` to
        # `["pcmu", "pcma"]`, so every test behind `g729`, `opus`, `amr-nb` or
        # `amr-wb` is compiled out on every pull request -- 104 tests run where
        # `--all-features` runs 740. The skipped ones are the expensive ones to
        # be wrong about: the AMR bit-exactness suites against TS 26.073 and
        # TS 26.173. This gate is where they actually execute.
        #
        # Both crates and not just the codec, because media-core's own codec
        # adapters sit behind the same feature names and inherit the identical
        # hole: 290 tests run there by default and 316 with every feature on,
        # and 14 of the 26 missing are the RFC 4867 AMR payload-format tests
        # the relay's framing guard depends on.
        # `rvoip-sip` too, and for the same reason one step further out: its
        # defaults are empty, so its AMR SDP-negotiation test -- the one that
        # checks a peer's offer reaches media-core with its framing intact and
        # builds a codec that works -- is compiled out on every pull request.
        #
        # `rvoip-core` last, and it is the worst of the four to omit. Its
        # `amr-nb`/`amr-wb` features gate the media-graph tests that prove an
        # AMR frame crosses the graph and comes out as PCMU. Without them the
        # default run still executes the two tests asserting the graph
        # *refuses* AMR -- so the shard prints green while demonstrating the
        # opposite of what is true. A hole that reports the inverse of reality
        # is worse than one that reports nothing.
        #
        # One gate per package rather than one gate over four. Each package
        # needs `--all-features` for both a test build and a clippy build, and
        # sharing a job made that eight dependency-tree builds against a single
        # 45-minute specialty budget: the combined gate timed out with the last
        # package still compiling, having run no tests at all. Worse, it could
        # not recover on a retry -- `Swatinem/rust-cache` keys on the gate name
        # and does not save a cache from a job that failed, so the one gate
        # that needed a warm cache was the one that could never populate it.
        # Split, the packages build concurrently, each against its own budget
        # and its own cache key. The patterns in `policy.json` stay identical
        # across the four, so any change that selected the combined gate still
        # verifies all four packages.
        package = CODEC_FEATURE_GATES[gate]
        # `--all-features` everywhere except `rvoip-sip`, whose feature set
        # includes the `perf-*` harness. Those targets are release-only by
        # contract -- `EnvironmentBlock::capture` panics under
        # `debug_assertions` rather than publish a number nobody should cite,
        # and the release catalog runs them as `--release --features
        # perf-tests,... --test <name>`. Turning them on in a debug gate does
        # not measure anything, it just fails. Every other feature stays on,
        # so the codec surface this gate exists for is unchanged.
        selection = (
            ["--features", NON_PERF_SIP_FEATURES]
            if package == "rvoip-sip"
            else ["--all-features"]
        )
        return [
            (
                [
                    "cargo",
                    "test",
                    "--locked",
                    *selection,
                    "--lib",
                    "--tests",
                    "--bins",
                    "--examples",
                    "-p",
                    package,
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "clippy",
                    "--locked",
                    *selection,
                    "--all-targets",
                    "-p",
                    package,
                ],
                None,
                None,
            ),
        ]
    if gate == "rtp-interop":
        return [
            (["bash", "scripts/test_libsrtp_interop.sh"], None, None),
            (
                [
                    "cargo",
                    "test",
                    "--locked",
                    "-p",
                    "rvoip-media-core",
                    "--features",
                    "dtls-srtp",
                    "--lib",
                    "dtls_fingerprint_mismatch_fails_before_context_installation",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "test",
                    "--locked",
                    "-p",
                    "rvoip-rtp-core",
                    "--features",
                    "dtls-webrtc",
                    "--test",
                    "dtls_srtp_handshake_test",
                    "--test",
                    "dtls_srtp_transport_bridge_test",
                    "--test",
                    "srtp_interop_webrtc_srtp",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "test",
                    "--locked",
                    "-p",
                    "rvoip-sip",
                    "--features",
                    "dtls-srtp",
                    "--test",
                    "dtls_srtp_call_integration",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "clippy",
                    "--locked",
                    "-p",
                    "rvoip-sip",
                    "--features",
                    "dtls-srtp",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
                None,
                None,
            ),
        ]
    if gate == "rustdoc":
        return [(list(RUSTDOC_COMMAND), None, None)]
    if gate == WORKSPACE_ALL_FEATURES_GATE:
        return [(list(WORKSPACE_ALL_FEATURES_COMMAND), None, None)]
    if gate == "amazon-connect-aws-control":
        return [
            (
                [
                    "cargo",
                    "check",
                    "--locked",
                    "-p",
                    "rvoip-amazon-connect",
                    "--features",
                    "aws-control",
                    "--all-targets",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "check",
                    "--manifest-path",
                    str(root / "examples/13-sip-to-amazon-connect/Cargo.toml"),
                    "--locked",
                    "--all-features",
                    "--all-targets",
                ],
                None,
                None,
            ),
        ]
    if gate == "infra-otel":
        return [
            (
                [
                    "cargo",
                    "check",
                    "--locked",
                    "-p",
                    "rvoip-infra-common",
                    "--features",
                    "otel",
                    "--all-targets",
                ],
                None,
                None,
            )
        ]
    if gate == "webrtc-runtime":
        commands = []
        for feature_args in ([], ["--no-default-features"], ["--all-features"]):
            commands.append(
                (
                    [
                        "cargo",
                        "test",
                        "-p",
                        "rvoip-webrtc-stack",
                        "--locked",
                        *feature_args,
                        "--test",
                        "tokio_only_runtime",
                    ],
                    None,
                    None,
                )
            )
        commands.append(([
            "cargo", "test", "--locked", "-p", "rvoip-webrtc",
            "--test", "turn_relay_e2e",
        ], None, None))
        for example_args in ([], ["--direct"]):
            commands.append(([
                "cargo", "run", "--locked", "-p", "rvoip-webrtc",
                "--example", "turn_ice", "--", *example_args,
            ], None, None))
        commands.append((["python3", "scripts/ci/check_runtime_dependencies.py"], None, None))
        return commands
    if gate == "browser-smoke":
        browser = root / "tests" / "browser-smoke"
        return [
            (
                ["cargo", "build", "-p", "rvoip-uctp", "--locked", "--example", "orchestrator_bridge"],
                None,
                None,
            ),
            (["npm", "ci"], browser, None),
            (["npx", "playwright", "install", "--with-deps", "chromium"], browser, None),
            (
                ["npm", "test"],
                browser,
                {
                    **os.environ,
                    "RUST_LOG": "warn",
                    "RVOIP_WT_SMOKE": "1",
                },
            ),
        ]
    if gate == "vcon-postgres":
        env = {
            **os.environ,
            "DATABASE_URL": "postgres://postgres:postgres@127.0.0.1:5432/rvoip_vcon_test",
        }
        return [
            (
                [
                    "cargo",
                    "test",
                    "-p",
                    "rvoip-vcon",
                    "--all-targets",
                    "--locked",
                ],
                None,
                None,
            ),
            (
                ["cargo", "test", "-p", "rvoip-core", "--lib", "vcon", "--locked"],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "test",
                    "-p",
                    "rvoip-core",
                    "--test",
                    "vcon_emission",
                    "--locked",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "check",
                    "-p",
                    "rvoip-core",
                    "--no-default-features",
                    "--all-targets",
                    "--locked",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "check",
                    "-p",
                    "rvoip-core",
                    "--all-features",
                    "--all-targets",
                    "--locked",
                ],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "test",
                    "-p",
                    "rvoip-vcon-postgres",
                    "--all-targets",
                    "--features",
                    "core-store,live-tests",
                    "--locked",
                ],
                None,
                env,
            ),
            (
                [
                    "cargo",
                    "test",
                    "-p",
                    "rvoip-quic",
                    "--test",
                    "e2e_full_stack",
                    "--locked",
                ],
                None,
                None,
            ),
            (
                ["cargo", "check", "-p", "rvoip", "--features", "voip-3", "--locked"],
                None,
                None,
            ),
            (
                [
                    "cargo",
                    "check",
                    "--manifest-path",
                    str(root / "examples/11-ai-harness-demo/Cargo.toml"),
                    "--locked",
                ],
                None,
                None,
            ),
        ]
    raise CheckError(f"unsupported specialty gate: {gate}")


def start_postgres(root: Path, records: list[dict[str, Any]]) -> str:
    suffix = re.sub(r"[^A-Za-z0-9_.-]", "-", f"{os.getenv('GITHUB_RUN_ID', 'local')}-{os.getenv('GITHUB_RUN_ATTEMPT', '1')}")
    name = f"rvoip-ci-postgres-{suffix}"
    start = command_record(
        [
            "docker",
            "run",
            "--detach",
            "--rm",
            "--name",
            name,
            "-e",
            "POSTGRES_DB=rvoip_vcon_test",
            "-e",
            "POSTGRES_USER=postgres",
            "-e",
            "POSTGRES_PASSWORD=postgres",
            "-p",
            "5432:5432",
            "postgres:17",
        ],
        root=root,
    )
    records.append(start)
    if start["exit_code"]:
        return name
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        ready = subprocess.run(
            ["docker", "exec", name, "pg_isready", "-U", "postgres", "-d", "rvoip_vcon_test"],
            cwd=root,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if ready.returncode == 0:
            return name
        time.sleep(2)
    records.append(
        {
            "argv": ["docker", "exec", name, "pg_isready"],
            "working_directory": ".",
            "started_at": utc_now(),
            "duration_seconds": 60,
            "exit_code": 1,
            "error": "PostgreSQL did not become ready",
        }
    )
    return name


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "kind",
        choices=(
            "policy",
            "shard",
            "shard-test",
            "shard-clippy",
            "specialty",
            "doctest",
            "sip-core",
            "sip-clippy",
            "sip-integration",
            "sip-fixtures",
        ),
    )
    parser.add_argument("--name", required=True)
    parser.add_argument("--packages", default="")
    parser.add_argument("--gate", default="")
    parser.add_argument("--targets", default="")
    parser.add_argument("--examples", default="")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[2]
    records: list[dict[str, Any]] = []
    postgres_name: str | None = None
    setup_error: str | None = None
    try:
        if args.kind == "policy":
            commands = policy_commands()
        elif args.kind == "shard":
            commands = shard_commands(args.packages)
        elif args.kind == "shard-test":
            commands = shard_test_commands(args.packages)
        elif args.kind == "shard-clippy":
            commands = shard_clippy_commands(args.packages)
        elif args.kind == "doctest":
            commands = [
                (["cargo", "test", "--workspace", "--doc", "--locked"], None, None)
            ]
        elif args.kind == "sip-core":
            commands = sip_core_commands()
        elif args.kind == "sip-clippy":
            commands = sip_clippy_commands()
        elif args.kind == "sip-integration":
            commands = sip_integration_commands(args.targets)
        elif args.kind == "sip-fixtures":
            commands = sip_fixture_commands(args.targets, args.examples, root)
        else:
            if args.gate == "vcon-postgres":
                postgres_name = start_postgres(root, records)
            commands = specialty_commands(args.gate, root)
        # Cisco's Webex BYOVA protobuf contract is not committed. Any command
        # that builds `rvoip-webex-byova/grpc` (every `--all-features`
        # workspace build, including the rustdoc gate) first fetches the
        # pinned, checksum-verified files; the fetch is recorded like any
        # other command and fails the run closed.
        if any(webex_byova_protos.needed(command) for command, _, _ in commands):
            commands = [
                ([sys.executable, "scripts/ci/fetch_webex_byova_protos.py"], None, None),
                *commands,
            ]
        for command, cwd, env in commands:
            records.append(command_record(command, root=root, cwd=cwd, env=env))
    except (CheckError, OSError, ValueError) as error:
        setup_error = str(error)
        print(setup_error, file=sys.stderr)
    finally:
        if postgres_name:
            subprocess.run(
                ["docker", "rm", "--force", postgres_name],
                cwd=root,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )

    failed = setup_error is not None or not records or any(record["exit_code"] for record in records)
    receipt = {
        "schema": SCHEMA,
        "name": args.name,
        "kind": args.kind,
        "gate": args.gate or None,
        "packages": sorted(filter(None, args.packages.split(","))),
        "targets": sorted(filter(None, args.targets.split(","))),
        "git_commit": os.getenv("GITHUB_SHA")
        or capture_version(["git", "rev-parse", "HEAD"], root),
        "recorded_at": utc_now(),
        "environment": {
            "os": platform.platform(),
            "architecture": platform.machine(),
            "python": platform.python_version(),
            "rustc": capture_version(["rustc", "--version"], root),
            "cargo": capture_version(["cargo", "--version"], root),
            "runner_image": os.getenv("ImageOS", "local"),
        },
        "commands": records,
        "status": "FAIL" if failed else "PASS",
        "error": setup_error,
    }
    output = args.output if args.output.is_absolute() else root / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
