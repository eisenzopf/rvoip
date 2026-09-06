#!/usr/bin/env python3
"""Build, install, and execute exact-candidate performance test binaries.

Release performance workers must measure the candidate, not spend most of their
life compiling it.  This helper turns the selected GCP performance gates into a
small set of Cargo build invocations, packages the resulting test executables,
and verifies every byte again before a runtime worker may execute it.
"""

from __future__ import annotations

import argparse
import datetime as dt
import fnmatch
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from typing import Any


MANIFEST_SCHEMA = "rvoip-performance-prebuilt-v1"
RESULT_SCHEMA = "rvoip-gcp-performance-prebuild-result-v1"
CACHE_KEY_SCHEMA = "rvoip-performance-prebuilt-cache-key-v1"
COMMIT_SHA = re.compile(r"^[0-9a-f]{40}$")
HEX_SHA256 = re.compile(r"^[0-9a-f]{64}$")
ENV_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=.*$", re.DOTALL)
PERFORMANCE_RESOURCES = {
    "gcp-performance",
    "gcp-performance-soak",
    "gcp-performance-soak-long",
}


class PrebuiltError(RuntimeError):
    """An exact-candidate prebuilt invariant failed closed."""


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def cache_key(*, candidate: str, environment_id: str, gate_ids: list[str]) -> str:
    """Bind a reusable bundle to one exact build request.

    Candidate identity remains part of the key even when two commits have the
    same Git tree. A future build script may legitimately consume commit
    metadata, so cross-commit reuse would not be fail-closed.
    """

    if not COMMIT_SHA.fullmatch(candidate):
        raise PrebuiltError("cache candidate must be a full lowercase commit SHA")
    if not environment_id:
        raise PrebuiltError("cache environment ID must be non-empty")
    selected = sorted({gate_id for gate_id in gate_ids if gate_id})
    if not selected:
        raise PrebuiltError("cache gate set must be non-empty")
    return sha256_bytes(
        canonical_bytes(
            {
                "schema": CACHE_KEY_SCHEMA,
                "candidate_sha": candidate,
                "environment_id": environment_id,
                "selected_gate_ids": selected,
            }
        )
    )


def load_json(path: Path, description: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PrebuiltError(f"cannot read {description} {path}: {error}") from error
    if not isinstance(value, dict):
        raise PrebuiltError(f"{description} must be a JSON object: {path}")
    return value


def git(root: Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", *args], cwd=root, text=True, capture_output=True, check=False
    )
    if completed.returncode:
        raise PrebuiltError(
            f"git {' '.join(args)} failed: {(completed.stderr or completed.stdout).strip()}"
        )
    return completed.stdout.strip()


def require_exact_source(root: Path, candidate: str) -> str:
    if not COMMIT_SHA.fullmatch(candidate):
        raise PrebuiltError("candidate must be a full lowercase commit SHA")
    head = git(root, "rev-parse", "HEAD")
    if head != candidate:
        raise PrebuiltError(f"checkout is {head}, not exact candidate {candidate}")
    status = git(root, "status", "--porcelain=v1", "--untracked-files=all")
    if status:
        raise PrebuiltError("exact-candidate checkout is not clean")
    return git(root, "rev-parse", "HEAD^{tree}")


def normalise_features(raw: str) -> tuple[str, ...]:
    features = sorted({item.strip() for item in raw.split(",") if item.strip()})
    if not features:
        raise PrebuiltError("performance build requires at least one Cargo feature")
    return tuple(features)


def split_environment(command: list[str]) -> tuple[dict[str, str], list[str]]:
    if not command:
        raise PrebuiltError("gate command is empty")
    if command[0] != "env":
        return {}, list(command)
    environment: dict[str, str] = {}
    index = 1
    while index < len(command) and ENV_ASSIGNMENT.fullmatch(command[index]):
        name, value = command[index].split("=", 1)
        environment[name] = value
        index += 1
    if index == len(command):
        raise PrebuiltError("env gate command has no executable")
    return environment, command[index:]


def cargo_test_definition(command: list[str]) -> dict[str, Any] | None:
    environment, argv = split_environment(command)
    if len(argv) < 2 or argv[0] != "cargo" or argv[1] != "test":
        return None

    package = None
    features = None
    targets: list[str] = []
    runner_prefix: list[str] = []
    runner_args: list[str] = []
    release = False
    default_features = True
    value_options = {
        "-p",
        "--package",
        "--features",
        "--test",
        "--profile",
        "--target",
        "--target-dir",
        "--manifest-path",
        "-j",
        "--jobs",
    }
    index = 2
    while index < len(argv):
        token = argv[index]
        if token == "--":
            runner_args = argv[index + 1 :]
            break
        if token in value_options:
            if index + 1 >= len(argv):
                raise PrebuiltError(f"Cargo option {token} lacks a value")
            value = argv[index + 1]
            if token in {"-p", "--package"}:
                package = value
            elif token == "--features":
                features = value
            elif token == "--test":
                targets.append(value)
            index += 2
            continue
        if token == "--release":
            release = True
        elif token == "--no-default-features":
            default_features = False
        elif token.startswith("-"):
            # Flags such as --locked do not change direct libtest arguments.
            pass
        else:
            runner_prefix.append(token)
        index += 1

    if package != "rvoip-sip" or not release or features is None or not targets:
        raise PrebuiltError(
            "prebuilt performance Cargo gates must select release rvoip-sip test targets"
        )
    if len(runner_prefix) > 1:
        raise PrebuiltError(f"unsupported Cargo test filters: {runner_prefix}")
    return {
        "environment": environment,
        "features": normalise_features(features),
        "targets": targets,
        "runner_args": runner_prefix + runner_args,
        "default_features": default_features,
    }


def shell_build_definition(command: list[str]) -> dict[str, Any] | None:
    environment, argv = split_environment(command)
    script = next(
        (
            Path(token).name
            for token in argv
            if token.endswith("perf_burst_matrix.sh")
            or token.endswith("perf_soak_split.sh")
            or token.endswith("canonical_2k_release_eval.sh")
        ),
        None,
    )
    if script is None:
        return None
    features = normalise_features(environment.get("RVOIP_PERF_FEATURES", "perf-tests"))
    targets = {
        "perf_burst_matrix.sh": ["perf_burst_receiver", "perf_burst_caller"],
        "perf_soak_split.sh": ["perf_soak_receiver", "perf_soak_caller"],
        "canonical_2k_release_eval.sh": ["perf_call_setup_cps"],
    }[script]
    return {
        "environment": environment,
        "features": features,
        "targets": targets,
        "runner_args": [],
        "default_features": script != "canonical_2k_release_eval.sh",
    }


def gate_definition(gate: dict[str, Any]) -> dict[str, Any] | None:
    if gate.get("resource_class") not in PERFORMANCE_RESOURCES:
        return None
    command = gate.get("command")
    if gate.get("executor") != "argv" or not isinstance(command, list):
        return None
    definition = cargo_test_definition(command)
    if definition is not None:
        definition["kind"] = "cargo"
        return definition
    definition = shell_build_definition(command)
    if definition is not None:
        definition["kind"] = "shell"
        return definition
    # Infrastructure and policy-only gates intentionally require no binary.
    return None


def available_test_targets(root: Path) -> list[str]:
    crate = root / "crates/sip/rvoip-sip/tests"
    return sorted(path.stem for path in crate.rglob("*.rs"))


def expand_targets(patterns: list[str], available: list[str]) -> list[str]:
    selected: set[str] = set()
    for pattern in patterns:
        matches = [
            target for target in available if fnmatch.fnmatchcase(target, pattern)
        ]
        if not matches:
            raise PrebuiltError(f"Cargo test target pattern matched nothing: {pattern}")
        selected.update(matches)
    return sorted(selected)


def selected_builds(
    root: Path, catalog: dict[str, Any], gate_ids: list[str]
) -> tuple[dict[tuple[tuple[str, ...], bool], set[str]], dict[str, dict[str, Any]]]:
    by_id = {gate.get("id"): gate for gate in catalog.get("gates", [])}
    unknown = sorted(set(gate_ids) - set(by_id))
    if unknown:
        raise PrebuiltError(f"unknown release gate IDs: {unknown}")
    available = available_test_targets(root)
    groups: dict[tuple[tuple[str, ...], bool], set[str]] = {}
    definitions: dict[str, dict[str, Any]] = {}
    for gate_id in sorted(set(gate_ids)):
        definition = gate_definition(by_id[gate_id])
        if definition is None:
            continue
        targets = expand_targets(definition["targets"], available)
        definition = {**definition, "resolved_targets": targets}
        definitions[gate_id] = definition
        build_key = (definition["features"], definition["default_features"])
        groups.setdefault(build_key, set()).update(targets)
    if not groups:
        raise PrebuiltError(
            "selected performance gates require no prebuilt executables"
        )
    return groups, definitions


def cargo_artifacts(messages: Path, expected: set[str]) -> dict[str, dict[str, Any]]:
    found: dict[str, list[dict[str, Any]]] = {target: [] for target in expected}
    with messages.open(encoding="utf-8") as handle:
        for line in handle:
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            target = item.get("target") or {}
            name = target.get("name")
            if (
                item.get("reason") == "compiler-artifact"
                and name in found
                and "test" in (target.get("kind") or [])
                and item.get("executable")
            ):
                found[name].append(item)
    result: dict[str, dict[str, Any]] = {}
    for target, records in found.items():
        unique = {Path(record["executable"]).resolve(): record for record in records}
        if len(unique) != 1:
            raise PrebuiltError(
                f"expected one Cargo executable for {target}, found {list(map(str, unique))}"
            )
        executable, record = next(iter(unique.items()))
        if not executable.is_file() or not os.access(executable, os.X_OK):
            raise PrebuiltError(f"Cargo executable is unavailable: {executable}")
        result[target] = {"executable": executable, "message": record}
    return result


def build_bundle(
    *,
    root: Path,
    catalog_path: Path,
    gate_ids: list[str],
    candidate: str,
    environment_id: str,
    output: Path,
) -> dict[str, Any]:
    tree = require_exact_source(root, candidate)
    catalog = load_json(catalog_path, "release gate catalog")
    groups, definitions = selected_builds(root, catalog, gate_ids)
    if output.exists():
        raise PrebuiltError(f"refusing to overwrite prebuilt output: {output}")
    output.mkdir(parents=True)
    binaries = output / "binaries"
    evidence = output / "build-evidence"
    binaries.mkdir()
    evidence.mkdir()
    entries: list[dict[str, Any]] = []
    build_commands: list[dict[str, Any]] = []

    for group_index, ((features, default_features), targets) in enumerate(
        sorted(groups.items()), start=1
    ):
        messages = evidence / f"cargo-{group_index}.jsonl"
        command = [
            "cargo",
            "test",
            "--locked",
            "-p",
            "rvoip-sip",
            "--release",
            "--features",
            ",".join(features),
        ]
        if not default_features:
            command.append("--no-default-features")
        for target in sorted(targets):
            command.extend(("--test", target))
        command.extend(("--no-run", "--message-format=json-render-diagnostics"))
        started = dt.datetime.now(dt.timezone.utc)
        with messages.open("w", encoding="utf-8") as handle:
            completed = subprocess.run(
                command,
                cwd=root,
                stdout=handle,
                stderr=None,
                check=False,
            )
        duration = (dt.datetime.now(dt.timezone.utc) - started).total_seconds()
        build_commands.append(
            {
                "argv": command,
                "features": list(features),
                "default_features": default_features,
                "targets": sorted(targets),
                "duration_seconds": round(duration, 3),
                "exit_code": completed.returncode,
                "cargo_messages_sha256": sha256_file(messages),
            }
        )
        if completed.returncode:
            raise PrebuiltError(
                f"Cargo prebuild failed with exit code {completed.returncode}"
            )
        for target, record in cargo_artifacts(messages, targets).items():
            source = record["executable"]
            executable_sha = sha256_file(source)
            destination = binaries / f"{executable_sha}-{target}"
            if not destination.exists():
                shutil.copy2(source, destination)
                destination.chmod(destination.stat().st_mode | stat.S_IXUSR)
            message = record["message"]
            entries.append(
                {
                    "target": target,
                    "features": list(features),
                    "default_features": default_features,
                    "path": destination.relative_to(output).as_posix(),
                    "sha256": executable_sha,
                    "bytes": destination.stat().st_size,
                    "cargo_artifact": {
                        "package_id": message.get("package_id"),
                        "target": message.get("target"),
                        "profile": message.get("profile"),
                        "fresh": message.get("fresh"),
                    },
                }
            )

    manifest = {
        "schema": MANIFEST_SCHEMA,
        "created_at": dt.datetime.now(dt.timezone.utc)
        .replace(microsecond=0)
        .isoformat(),
        "candidate_sha": candidate,
        "source_tree_sha": tree,
        "environment_id": environment_id,
        "selected_gate_ids": sorted(set(gate_ids)),
        "gate_definitions": {
            gate_id: {
                "kind": definition["kind"],
                "features": list(definition["features"]),
                "default_features": definition["default_features"],
                "targets": definition["resolved_targets"],
                "runner_args": definition["runner_args"],
            }
            for gate_id, definition in sorted(definitions.items())
        },
        "rustc_version": subprocess.run(
            ["rustc", "--version", "--verbose"],
            cwd=root,
            text=True,
            capture_output=True,
            check=True,
        ).stdout.strip(),
        "cargo_version": subprocess.run(
            ["cargo", "--version", "--verbose"],
            cwd=root,
            text=True,
            capture_output=True,
            check=True,
        ).stdout.strip(),
        "build_commands": build_commands,
        "executables": sorted(
            entries,
            key=lambda item: (
                item["features"],
                item["default_features"],
                item["target"],
            ),
        ),
    }
    (output / "manifest.json").write_bytes(canonical_bytes(manifest))
    validate_manifest(
        manifest=manifest,
        bundle_root=output,
        root=root,
        candidate=candidate,
        environment_id=environment_id,
    )
    return manifest


def safe_bundle_members(archive: Path) -> list[tarfile.TarInfo]:
    try:
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
    except (OSError, tarfile.TarError) as error:
        raise PrebuiltError(
            f"cannot read performance bundle {archive}: {error}"
        ) from error
    if not members:
        raise PrebuiltError("performance bundle is empty")
    for member in members:
        path = PurePosixPath(member.name)
        if (
            path.is_absolute()
            or ".." in path.parts
            or not path.parts
            or path.parts[0] != "performance-prebuilt"
            or not (member.isdir() or member.isfile())
        ):
            raise PrebuiltError(f"unsafe performance bundle member: {member.name!r}")
    return members


def install_bundle(
    *,
    archive: Path,
    archive_sha256: str,
    destination: Path,
    root: Path,
    candidate: str,
    environment_id: str,
) -> dict[str, Any]:
    if not HEX_SHA256.fullmatch(archive_sha256):
        raise PrebuiltError("bundle SHA-256 is invalid")
    actual = sha256_file(archive)
    if actual != archive_sha256:
        raise PrebuiltError(
            f"bundle digest mismatch: expected {archive_sha256}, got {actual}"
        )
    if destination.exists():
        raise PrebuiltError(f"refusing to overwrite bundle destination: {destination}")
    members = safe_bundle_members(archive)
    destination.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(
        tempfile.mkdtemp(prefix=".performance-prebuilt-", dir=destination.parent)
    )
    try:
        with tarfile.open(archive, "r:gz") as bundle:
            bundle.extractall(staging, members=members, filter="data")
        extracted = staging / "performance-prebuilt"
        manifest = load_json(extracted / "manifest.json", "performance manifest")
        validate_manifest(
            manifest=manifest,
            bundle_root=extracted,
            root=root,
            candidate=candidate,
            environment_id=environment_id,
        )
        extracted.rename(destination)
        shutil.rmtree(staging, ignore_errors=True)
        return manifest
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def validate_manifest(
    *,
    manifest: dict[str, Any],
    bundle_root: Path,
    root: Path,
    candidate: str,
    environment_id: str,
) -> None:
    tree = require_exact_source(root, candidate)
    expected = {
        "schema": MANIFEST_SCHEMA,
        "candidate_sha": candidate,
        "source_tree_sha": tree,
        "environment_id": environment_id,
    }
    mismatches = [
        f"{key}: expected {value!r}, got {manifest.get(key)!r}"
        for key, value in expected.items()
        if manifest.get(key) != value
    ]
    entries = manifest.get("executables")
    if not isinstance(entries, list) or not entries:
        mismatches.append("executables must be a non-empty list")
        entries = []
    seen: set[tuple[tuple[str, ...], bool, str]] = set()
    for entry in entries:
        if not isinstance(entry, dict):
            mismatches.append("executable entry is not an object")
            continue
        features = entry.get("features")
        default_features = entry.get("default_features")
        target = entry.get("target")
        relative = entry.get("path")
        expected_sha = entry.get("sha256")
        if not (
            isinstance(features, list)
            and all(isinstance(item, str) and item for item in features)
            and isinstance(default_features, bool)
            and isinstance(target, str)
            and target
            and isinstance(relative, str)
            and relative
            and isinstance(expected_sha, str)
            and HEX_SHA256.fullmatch(expected_sha)
        ):
            mismatches.append(f"invalid executable entry: {entry!r}")
            continue
        key = (tuple(features), default_features, target)
        if key in seen:
            mismatches.append(f"duplicate executable entry: {key}")
            continue
        seen.add(key)
        path = PurePosixPath(relative)
        if path.is_absolute() or ".." in path.parts:
            mismatches.append(f"unsafe executable path: {relative!r}")
            continue
        executable = bundle_root.joinpath(*path.parts)
        if (
            not executable.is_file()
            or executable.is_symlink()
            or not os.access(executable, os.X_OK)
        ):
            mismatches.append(f"executable is missing or unsafe: {relative}")
            continue
        if entry.get("bytes") != executable.stat().st_size:
            mismatches.append(f"executable size mismatch for {target}")
        actual_sha = sha256_file(executable)
        if actual_sha != expected_sha:
            mismatches.append(
                f"executable digest mismatch for {target}: expected {expected_sha}, got {actual_sha}"
            )
    if mismatches:
        raise PrebuiltError(
            "performance manifest verification failed:\n- " + "\n- ".join(mismatches)
        )


def resolve_entry(
    manifest_path: Path,
    *,
    root: Path,
    candidate: str,
    environment_id: str,
    features: tuple[str, ...],
    default_features: bool,
    target: str,
) -> tuple[Path, dict[str, Any], dict[str, Any]]:
    bundle_root = manifest_path.resolve().parent
    manifest = load_json(manifest_path, "performance manifest")
    validate_manifest(
        manifest=manifest,
        bundle_root=bundle_root,
        root=root,
        candidate=candidate,
        environment_id=environment_id,
    )
    entries = [
        entry
        for entry in manifest["executables"]
        if entry.get("target") == target
        and tuple(entry.get("features", [])) == features
        and entry.get("default_features") is default_features
    ]
    if len(entries) != 1:
        raise PrebuiltError(
            f"expected one prebuilt {target} executable with features {features} "
            f"and default_features={default_features}, found {len(entries)}"
        )
    entry = entries[0]
    executable = bundle_root / entry["path"]
    return executable, entry, manifest


def write_runtime_artifact_manifest(
    *,
    output: Path,
    bundle_manifest_path: Path,
    executable: Path,
    entry: dict[str, Any],
    manifest: dict[str, Any],
    source_at_build: Path,
    build_targets: list[str],
    default_features: bool,
) -> None:
    source = load_json(source_at_build, "runtime source provenance")
    payload = {
        "schema": "rvoip-perf-cargo-artifact-v1",
        "captured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "workspace_root": str(Path.cwd().resolve()),
        "source_at_build": source,
        "cargo_invocation": {
            "command": ["prebuilt-performance-bundle"],
            "package": "rvoip-sip",
            "profile": "release",
            "test_targets": build_targets,
            "features_requested": entry["features"],
            "default_features": default_features,
            "environment": {},
        },
        "cargo_artifact": entry.get("cargo_artifact"),
        "prebuilt_bundle": {
            "schema": manifest["schema"],
            "candidate_sha": manifest["candidate_sha"],
            "source_tree_sha": manifest["source_tree_sha"],
            "environment_id": manifest["environment_id"],
            "manifest_sha256": sha256_file(bundle_manifest_path),
        },
        "executable": str(executable),
        "executable_sha256": entry["sha256"],
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(canonical_bytes(payload))


def expand_runtime(value: str, root: Path, artifact: Path, candidate: str) -> str:
    return (
        value.replace("{workspace}", str(root))
        .replace("{artifact_dir}", str(artifact))
        .replace("{candidate}", candidate)
    )


def run_gate(
    *,
    manifest_path: Path,
    catalog_path: Path,
    gate_id: str,
    root: Path,
    artifact: Path,
    candidate: str,
    environment_id: str,
) -> int:
    catalog = load_json(catalog_path, "release gate catalog")
    matches = [gate for gate in catalog.get("gates", []) if gate.get("id") == gate_id]
    if len(matches) != 1:
        raise PrebuiltError(f"cannot resolve exact gate definition for {gate_id}")
    gate = matches[0]
    definition = cargo_test_definition(gate.get("command") or [])
    if definition is None:
        raise PrebuiltError(f"gate {gate_id} is not a direct Cargo performance test")
    manifest = load_json(manifest_path, "performance manifest")
    bundle_root = manifest_path.resolve().parent
    validate_manifest(
        manifest=manifest,
        bundle_root=bundle_root,
        root=root,
        candidate=candidate,
        environment_id=environment_id,
    )
    selected = manifest.get("gate_definitions", {}).get(gate_id)
    if not isinstance(selected, dict):
        raise PrebuiltError(f"bundle was not built for gate {gate_id}")
    targets = selected.get("targets")
    if not isinstance(targets, list) or not targets:
        raise PrebuiltError(f"bundle gate definition lacks targets for {gate_id}")
    environment = os.environ.copy()
    environment.setdefault("CARGO_MANIFEST_DIR", str(root / "crates/sip/rvoip-sip"))
    environment.update(
        {
            name: expand_runtime(value, root, artifact, candidate)
            for name, value in definition["environment"].items()
        }
    )
    print(
        f"running {gate_id} from verified exact-candidate performance bundle "
        f"({len(targets)} executable(s))",
        flush=True,
    )
    for target in targets:
        executable, entry, _ = resolve_entry(
            manifest_path,
            root=root,
            candidate=candidate,
            environment_id=environment_id,
            features=definition["features"],
            default_features=definition["default_features"],
            target=target,
        )
        argv = [str(executable), *definition["runner_args"]]
        print(f"prebuilt target {target}: sha256={entry['sha256']}", flush=True)
        # Cargo executes integration tests from the package directory. Preserve
        # that runtime contract even though Cargo itself is no longer involved.
        completed = subprocess.run(
            argv,
            cwd=root / "crates/sip/rvoip-sip",
            env=environment,
            check=False,
        )
        if completed.returncode:
            return completed.returncode
    return 0


def validate_result(
    result_path: Path,
    *,
    candidate: str,
    environment_id: str,
    gate_ids: list[str],
    expected_cache_key: str | None = None,
) -> dict[str, Any]:
    result = load_json(result_path, "GCP prebuild result")
    expected = {
        "schema": RESULT_SCHEMA,
        "candidate_sha": candidate,
        "environment_id": environment_id,
        "selected_gate_ids": sorted(set(gate_ids)),
        "status": "PASS",
        "exit_code": 0,
        "publishing_attempted": False,
    }
    failures = [
        f"{key}: expected {value!r}, got {result.get(key)!r}"
        for key, value in expected.items()
        if result.get(key) != value
    ]
    if expected_cache_key is not None:
        if not HEX_SHA256.fullmatch(expected_cache_key):
            raise PrebuiltError("expected performance cache key is invalid")
        if result.get("cache_key_sha256") != expected_cache_key:
            failures.append(
                "cache_key_sha256: expected "
                f"{expected_cache_key!r}, got {result.get('cache_key_sha256')!r}"
            )
    if not HEX_SHA256.fullmatch(str(result.get("bundle_sha256", ""))):
        failures.append("bundle_sha256 is missing or invalid")
    if not HEX_SHA256.fullmatch(str(result.get("manifest_sha256", ""))):
        failures.append("manifest_sha256 is missing or invalid")
    uri = result.get("bundle_uri")
    if not isinstance(uri, str) or not uri.startswith("gs://"):
        failures.append("bundle_uri is missing or invalid")
    elif not uri.endswith(f"/bundles/{result.get('bundle_sha256')}.tar.gz"):
        failures.append("bundle_uri is not content-addressed by bundle_sha256")
    manifest_uri = result.get("manifest_uri")
    if not isinstance(manifest_uri, str) or not manifest_uri.startswith("gs://"):
        failures.append("manifest_uri is missing or invalid")
    elif not manifest_uri.endswith(
        f"/manifests/{result.get('manifest_sha256')}.json"
    ):
        failures.append("manifest_uri is not content-addressed by manifest_sha256")
    if expected_cache_key is not None:
        required_fragment = f"/performance-prebuilt-v1/{expected_cache_key}/"
        for label, value in (("bundle_uri", uri), ("manifest_uri", manifest_uri)):
            if isinstance(value, str) and required_fragment not in value:
                failures.append(f"{label} is outside the expected cache namespace")
    if failures:
        raise PrebuiltError(
            "GCP prebuild result verification failed:\n- " + "\n- ".join(failures)
        )
    return result


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)

    select = commands.add_parser("select-gates")
    select.add_argument("--catalog", type=Path, required=True)
    select.add_argument("--gates", required=True)

    cache = commands.add_parser("cache-key")
    cache.add_argument("--candidate", required=True)
    cache.add_argument("--environment-id", required=True)
    cache.add_argument("--gates", required=True)

    build = commands.add_parser("build")
    build.add_argument("--workspace", type=Path, required=True)
    build.add_argument("--catalog", type=Path, required=True)
    build.add_argument("--gates", required=True)
    build.add_argument("--candidate", required=True)
    build.add_argument("--environment-id", required=True)
    build.add_argument("--output", type=Path, required=True)

    install = commands.add_parser("install-bundle")
    install.add_argument("--archive", type=Path, required=True)
    install.add_argument("--archive-sha256", required=True)
    install.add_argument("--destination", type=Path, required=True)
    install.add_argument("--workspace", type=Path, required=True)
    install.add_argument("--candidate", required=True)
    install.add_argument("--environment-id", required=True)

    resolve = commands.add_parser("resolve")
    resolve.add_argument("--manifest", type=Path, required=True)
    resolve.add_argument("--workspace", type=Path, required=True)
    resolve.add_argument("--candidate", required=True)
    resolve.add_argument("--environment-id", required=True)
    resolve.add_argument("--features", required=True)
    resolve.add_argument(
        "--default-features", choices=("enabled", "disabled"), default="enabled"
    )
    resolve.add_argument("--target", required=True)
    resolve.add_argument("--artifact-manifest", type=Path)
    resolve.add_argument("--source-at-build", type=Path)
    resolve.add_argument("--build-target", action="append", default=[])

    run = commands.add_parser("run-gate")
    run.add_argument("--manifest", type=Path, required=True)
    run.add_argument("--catalog", type=Path, required=True)
    run.add_argument("--gate-id", required=True)
    run.add_argument("--workspace", type=Path, required=True)
    run.add_argument("--artifact-dir", type=Path, required=True)
    run.add_argument("--candidate", required=True)
    run.add_argument("--environment-id", required=True)

    verify = commands.add_parser("verify-result")
    verify.add_argument("--result", type=Path, required=True)
    verify.add_argument("--candidate", required=True)
    verify.add_argument("--environment-id", required=True)
    verify.add_argument("--gates", required=True)
    verify.add_argument("--cache-key")
    verify.add_argument("--github-output", type=Path)
    return result


def csv_values(raw: str) -> list[str]:
    return sorted({item.strip() for item in raw.split(",") if item.strip()})


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "select-gates":
            catalog = load_json(args.catalog.resolve(), "release gate catalog")
            by_id = {gate.get("id"): gate for gate in catalog.get("gates", [])}
            selected = csv_values(args.gates)
            unknown = sorted(set(selected) - set(by_id))
            if unknown:
                raise PrebuiltError(f"unknown release gate IDs: {unknown}")
            print(
                ",".join(
                    gate_id
                    for gate_id in selected
                    if gate_definition(by_id[gate_id]) is not None
                )
            )
        elif args.command == "cache-key":
            print(
                cache_key(
                    candidate=args.candidate,
                    environment_id=args.environment_id,
                    gate_ids=csv_values(args.gates),
                )
            )
        elif args.command == "build":
            manifest = build_bundle(
                root=args.workspace.resolve(),
                catalog_path=args.catalog.resolve(),
                gate_ids=csv_values(args.gates),
                candidate=args.candidate,
                environment_id=args.environment_id,
                output=args.output.resolve(),
            )
            print(json.dumps({"executables": len(manifest["executables"])}))
        elif args.command == "install-bundle":
            manifest = install_bundle(
                archive=args.archive.resolve(),
                archive_sha256=args.archive_sha256,
                destination=args.destination.resolve(),
                root=args.workspace.resolve(),
                candidate=args.candidate,
                environment_id=args.environment_id,
            )
            print(args.destination / "manifest.json")
            if manifest["candidate_sha"] != args.candidate:  # defensive clarity
                raise PrebuiltError(
                    "installed bundle candidate changed during verification"
                )
        elif args.command == "resolve":
            features = normalise_features(args.features)
            executable, entry, manifest = resolve_entry(
                args.manifest,
                root=args.workspace.resolve(),
                candidate=args.candidate,
                environment_id=args.environment_id,
                features=features,
                default_features=args.default_features == "enabled",
                target=args.target,
            )
            if bool(args.artifact_manifest) != bool(args.source_at_build):
                raise PrebuiltError(
                    "artifact-manifest and source-at-build must be supplied together"
                )
            if args.artifact_manifest:
                write_runtime_artifact_manifest(
                    output=args.artifact_manifest,
                    bundle_manifest_path=args.manifest.resolve(),
                    executable=executable,
                    entry=entry,
                    manifest=manifest,
                    source_at_build=args.source_at_build,
                    build_targets=args.build_target or [args.target],
                    default_features=args.default_features == "enabled",
                )
            print(executable)
        elif args.command == "run-gate":
            return run_gate(
                manifest_path=args.manifest.resolve(),
                catalog_path=args.catalog.resolve(),
                gate_id=args.gate_id,
                root=args.workspace.resolve(),
                artifact=args.artifact_dir.resolve(),
                candidate=args.candidate,
                environment_id=args.environment_id,
            )
        elif args.command == "verify-result":
            result = validate_result(
                args.result,
                candidate=args.candidate,
                environment_id=args.environment_id,
                gate_ids=csv_values(args.gates),
                expected_cache_key=args.cache_key,
            )
            if args.github_output:
                with args.github_output.open("a", encoding="utf-8") as handle:
                    handle.write(f"bundle_uri={result['bundle_uri']}\n")
                    handle.write(f"bundle_sha256={result['bundle_sha256']}\n")
            print(json.dumps(result, sort_keys=True))
        else:  # pragma: no cover
            raise AssertionError(args.command)
    except (PrebuiltError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"prebuilt performance error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
