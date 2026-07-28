#!/usr/bin/env python3
"""Resolve and attest exact Cargo-produced performance test executables.

Performance runners must never guess which hashed executable Cargo just built by
examining ``target/*/deps`` mtimes.  Cargo's JSON ``compiler-artifact`` message
is the authority for the current invocation.  This helper also captures a
content fingerprint of the source tree so a run can reject source changes made
while its executable is built or exercised.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import sys
from typing import Any, Iterable


MANIFEST_SCHEMA = "rvoip-perf-cargo-artifact-v1"
SOURCE_FINGERPRINT_DOMAIN = b"rvoip-source-fingerprint-v1\0"


class ArtifactResolutionError(RuntimeError):
    """Raised when Cargo did not identify one exact executable artifact."""


def _sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _git_bytes(root: pathlib.Path, *args: str) -> bytes:
    return subprocess.run(
        ["git", *args],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout


def _frame(digest: Any, value: bytes) -> None:
    digest.update(len(value).to_bytes(8, "little"))
    digest.update(value)


def capture_source_provenance(workspace_root: pathlib.Path) -> dict[str, Any]:
    """Return the same content-based source identity used by the 2k gate."""

    root = workspace_root.resolve()
    try:
        commit = _git_bytes(root, "rev-parse", "HEAD").decode().strip()
        short = _git_bytes(root, "rev-parse", "--short", "HEAD").decode().strip()
        status = _git_bytes(
            root, "status", "--porcelain=v1", "-z", "--untracked-files=all"
        )
        tracked_diff = _git_bytes(root, "diff", "--binary", "HEAD", "--", ".")
        untracked = sorted(
            part
            for part in _git_bytes(
                root, "ls-files", "--others", "--exclude-standard", "-z"
            ).split(b"\0")
            if part
        )
        digest = hashlib.sha256(SOURCE_FINGERPRINT_DOMAIN)
        _frame(digest, commit.encode())
        _frame(digest, status)
        _frame(digest, tracked_diff)
        for raw_path in untracked:
            _frame(digest, raw_path)
            try:
                content = (root / os.fsdecode(raw_path)).read_bytes()
            except OSError as error:
                content = f"unreadable:{error.__class__.__name__}".encode()
            _frame(digest, content)
        return {
            "git_commit": commit,
            "git_rev": short,
            "git_dirty": bool(status),
            "source_fingerprint_sha256": digest.hexdigest(),
        }
    except (OSError, subprocess.CalledProcessError, UnicodeError) as error:
        raise ArtifactResolutionError(
            f"could not capture source provenance for {root}: {error}"
        ) from error


def write_source_provenance(
    workspace_root: pathlib.Path, output_path: pathlib.Path
) -> dict[str, Any]:
    provenance = capture_source_provenance(workspace_root)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    return provenance


def _valid_fingerprint(value: Any) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def assert_same_source(
    expected_path: pathlib.Path, actual_path: pathlib.Path, label: str
) -> str:
    expected = json.loads(expected_path.read_text(encoding="utf-8"))
    actual = json.loads(actual_path.read_text(encoding="utf-8"))
    expected_fingerprint = expected.get("source_fingerprint_sha256")
    actual_fingerprint = actual.get("source_fingerprint_sha256")
    if not _valid_fingerprint(expected_fingerprint):
        raise ArtifactResolutionError(
            f"{expected_path} has no valid source fingerprint"
        )
    if not _valid_fingerprint(actual_fingerprint):
        raise ArtifactResolutionError(f"{actual_path} has no valid source fingerprint")
    if expected_fingerprint != actual_fingerprint:
        raise ArtifactResolutionError(
            f"source changed {label}: expected {expected_fingerprint}, "
            f"found {actual_fingerprint}"
        )
    return expected_fingerprint


def _iter_json_messages(messages_path: pathlib.Path) -> Iterable[dict[str, Any]]:
    with messages_path.open("r", encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, start=1):
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                # Cargo can coexist with wrappers that write ordinary text.
                # Non-JSON lines are not artifact authority and are ignored.
                continue
            if not isinstance(value, dict):
                raise ArtifactResolutionError(
                    f"invalid Cargo JSON object at {messages_path}:{line_number}"
                )
            yield value


def resolve_cargo_test_artifact(
    messages_path: pathlib.Path,
    expected_name: str,
    expected_source: pathlib.Path | None = None,
) -> tuple[pathlib.Path, dict[str, Any]]:
    """Resolve exactly one executable for an integration-test target."""

    expected_source_resolved = expected_source.resolve() if expected_source else None
    candidates: list[tuple[pathlib.Path, dict[str, Any]]] = []
    for message in _iter_json_messages(messages_path):
        target = message.get("target") or {}
        executable = message.get("executable")
        if not (
            message.get("reason") == "compiler-artifact"
            and target.get("name") == expected_name
            and "test" in (target.get("kind") or [])
            and executable
        ):
            continue
        if expected_source_resolved is not None:
            source = target.get("src_path")
            if (
                source is None
                or pathlib.Path(source).resolve() != expected_source_resolved
            ):
                continue
        candidates.append((pathlib.Path(executable).resolve(), message))

    unique: dict[pathlib.Path, dict[str, Any]] = {}
    for executable, message in candidates:
        unique.setdefault(executable, message)
    if len(unique) != 1:
        rendered = [str(path) for path in unique]
        raise ArtifactResolutionError(
            f"expected exactly one Cargo executable for integration test "
            f"{expected_name!r}, found {rendered!r}"
        )

    executable, message = next(iter(unique.items()))
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ArtifactResolutionError(
            f"Cargo artifact is missing or not executable: {executable}"
        )
    return executable, message


def write_artifact_manifest(
    *,
    messages_path: pathlib.Path,
    manifest_path: pathlib.Path,
    workspace_root: pathlib.Path,
    source_at_build_path: pathlib.Path,
    expected_name: str,
    expected_source: pathlib.Path,
    package: str,
    profile: str,
    features: str,
    default_features: bool,
) -> pathlib.Path:
    executable, message = resolve_cargo_test_artifact(
        messages_path, expected_name, expected_source
    )
    source_at_build = json.loads(source_at_build_path.read_text(encoding="utf-8"))
    fingerprint = source_at_build.get("source_fingerprint_sha256")
    if not _valid_fingerprint(fingerprint):
        raise ArtifactResolutionError(
            f"{source_at_build_path} has no valid source fingerprint"
        )

    command = [
        "cargo",
        "test",
        "-p",
        package,
        "--release" if profile == "release" else f"--profile={profile}",
        "--features",
        features,
        "--test",
        expected_name,
        "--no-run",
        "--message-format=json-render-diagnostics",
    ]
    if not default_features:
        command.insert(5, "--no-default-features")

    target = message.get("target") or {}
    cargo_profile = message.get("profile") or {}
    selected_environment_names = (
        "CARGO_BUILD_TARGET",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_INCREMENTAL",
        "CARGO_TARGET_DIR",
        "RUSTFLAGS",
        "RUSTUP_TOOLCHAIN",
    )
    manifest = {
        "schema": MANIFEST_SCHEMA,
        "captured_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "workspace_root": str(workspace_root.resolve()),
        "source_at_build": source_at_build,
        "cargo_invocation": {
            "command": command,
            "package": package,
            "profile": profile,
            "features_requested": [item for item in features.split(",") if item],
            "default_features": default_features,
            "environment": {
                name: os.environ[name]
                for name in selected_environment_names
                if name in os.environ
            },
        },
        "cargo_artifact": {
            "target_name": target.get("name"),
            "target_kind": target.get("kind"),
            "target_crate_types": target.get("crate_types"),
            "target_src_path": target.get("src_path"),
            "package_id": message.get("package_id"),
            "profile": cargo_profile,
            "fresh": message.get("fresh"),
        },
        "cargo_messages_path": str(messages_path.resolve()),
        "cargo_messages_sha256": _sha256_file(messages_path),
        "executable": str(executable),
        "executable_sha256": _sha256_file(executable),
    }
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return executable


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    source = subparsers.add_parser("capture-source")
    source.add_argument("--workspace-root", type=pathlib.Path, required=True)
    source.add_argument("--output", type=pathlib.Path, required=True)

    check = subparsers.add_parser("assert-source")
    check.add_argument("--expected", type=pathlib.Path, required=True)
    check.add_argument("--actual", type=pathlib.Path, required=True)
    check.add_argument("--label", required=True)

    resolve = subparsers.add_parser("resolve")
    resolve.add_argument("--messages", type=pathlib.Path, required=True)
    resolve.add_argument("--manifest", type=pathlib.Path, required=True)
    resolve.add_argument("--workspace-root", type=pathlib.Path, required=True)
    resolve.add_argument("--source-at-build", type=pathlib.Path, required=True)
    resolve.add_argument("--target", required=True)
    resolve.add_argument("--target-source", type=pathlib.Path, required=True)
    resolve.add_argument("--package", required=True)
    resolve.add_argument("--profile", required=True)
    resolve.add_argument("--features", required=True)
    resolve.add_argument(
        "--default-features", choices=("enabled", "disabled"), required=True
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "capture-source":
            provenance = write_source_provenance(args.workspace_root, args.output)
            print(provenance["source_fingerprint_sha256"])
        elif args.command == "assert-source":
            print(assert_same_source(args.expected, args.actual, args.label))
        elif args.command == "resolve":
            executable = write_artifact_manifest(
                messages_path=args.messages,
                manifest_path=args.manifest,
                workspace_root=args.workspace_root,
                source_at_build_path=args.source_at_build,
                expected_name=args.target,
                expected_source=args.target_source,
                package=args.package,
                profile=args.profile,
                features=args.features,
                default_features=args.default_features == "enabled",
            )
            print(executable)
        else:  # pragma: no cover - argparse enforces the choices.
            raise AssertionError(args.command)
    except (
        ArtifactResolutionError,
        OSError,
        ValueError,
        json.JSONDecodeError,
    ) as error:
        print(f"perf Cargo artifact error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
