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
from typing import Any


SCHEMA = "rvoip-ci-command-receipt-v1"
PACKAGE = re.compile(r"^[A-Za-z0-9_-]+$")
GATE = re.compile(r"^[A-Za-z0-9_-]+$")
TARGET = re.compile(r"^[A-Za-z0-9_-]+$")


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
    return [value for package in packages for value in ("-p", package)]


def policy_commands() -> list[tuple[list[str], Path | None, dict[str, str] | None]]:
    return [
        (["cargo", "fmt", "--all", "--", "--check"], None, None),
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
            ["cargo", "clippy", "--locked", "-p", "rvoip-sip", "--all-targets"],
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
    argv = ["cargo", "test", "--locked", "-p", "rvoip-sip"]
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

    build = ["cargo", "build", "--locked", "-p", "rvoip-sip"]
    for example in examples:
        build.extend(("--example", example))
    test = ["cargo", "test", "--locked", "-p", "rvoip-sip"]
    for target in targets:
        test.extend(("--test", target))
    return [(build, None, None), (test, None, fixture_env)]


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
    if gate == "examples-smoke":
        smoke_examples = (
            "01-quickstart-p2p",
            "02-softphone-audio",
            "06-attended-transfer",
            "07-secure-call-srtp",
            "10-call-center-b2bua",
        )
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
                    "scripts/test_release_gcp_fanout.py",
                    "scripts/test_release_gates.py",
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
    if gate == "rtp-interop":
        return [(["bash", "scripts/test_libsrtp_interop.sh"], None, None)]
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
            (["npm", "test"], browser, {**os.environ, "RUST_LOG": "warn"}),
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
