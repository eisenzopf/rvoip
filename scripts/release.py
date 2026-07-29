#!/usr/bin/env python3
"""Prepare, verify, audit, and publish one version of the rvoip workspace."""

from __future__ import annotations

import argparse
import collections
import concurrent.futures
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
from typing import Any
import urllib.error
import urllib.request


EXPECTED_PACKAGE_COUNT = 44
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
USER_AGENT = "rvoip-unified-release/1.0"
DEFAULT_POLL_SECONDS = 15
DEFAULT_TIMEOUT_SECONDS = 900


class ReleaseError(RuntimeError):
    """A fail-closed release validation error."""


def repository_root() -> Path:
    return Path(__file__).resolve().parent.parent


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def run(
    argv: list[str],
    *,
    cwd: Path,
    check: bool = True,
    capture: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        argv,
        cwd=cwd,
        check=False,
        text=True,
        capture_output=capture,
        env=env,
    )
    if check and completed.returncode:
        detail = (completed.stdout or "") + (completed.stderr or "")
        raise ReleaseError(
            f"command failed ({completed.returncode}): {' '.join(argv)}\n{detail.strip()}"
        )
    return completed


def git_output(root: Path, *args: str) -> str:
    return run(["git", *args], cwd=root).stdout.strip()


def ensure_clean(root: Path) -> None:
    status = git_output(root, "status", "--porcelain")
    if status:
        raise ReleaseError(f"working tree must be clean:\n{status}")


def ensure_release_state(root: Path, version: str, *, require_no_tag: bool) -> str:
    ensure_clean(root)
    branch = git_output(root, "branch", "--show-current")
    if branch != "main":
        raise ReleaseError(f"release commands require branch main, found {branch!r}")
    head = git_output(root, "rev-parse", "HEAD")
    remote = run(
        ["git", "ls-remote", "origin", "refs/heads/main"], cwd=root
    ).stdout.split()
    if not remote or remote[0] != head:
        raise ReleaseError("HEAD must exactly match the current origin/main")
    tag = f"v{version}"
    if require_no_tag:
        local_tag = run(
            ["git", "show-ref", "--verify", "--quiet", f"refs/tags/{tag}"],
            cwd=root,
            check=False,
        )
        remote_tag = run(
            ["git", "ls-remote", "--tags", "origin", f"refs/tags/{tag}"],
            cwd=root,
        ).stdout.strip()
        if local_tag.returncode == 0 or remote_tag:
            raise ReleaseError(f"release tag {tag} already exists")
    return head


def cargo_metadata(root: Path, *, locked: bool) -> dict[str, Any]:
    argv = ["cargo", "metadata", "--no-deps", "--format-version", "1"]
    if locked:
        argv.append("--locked")
    return json.loads(run(argv, cwd=root).stdout)


def publishable_packages(metadata: dict[str, Any]) -> dict[str, dict[str, Any]]:
    members = set(metadata["workspace_members"])
    packages: dict[str, dict[str, Any]] = {}
    for package in metadata["packages"]:
        if package["id"] not in members or package.get("publish") == []:
            continue
        name = package["name"]
        if name in packages:
            raise ReleaseError(f"duplicate workspace package name: {name}")
        packages[name] = package
    return packages


def internal_dependencies(
    package: dict[str, Any], package_names: set[str]
) -> set[str]:
    return {
        dependency["name"]
        for dependency in package["dependencies"]
        if dependency["name"] in package_names and dependency.get("kind") != "dev"
    }


def topological_order(packages: dict[str, dict[str, Any]]) -> list[str]:
    names = set(packages)
    indegree = {name: 0 for name in names}
    dependents: dict[str, list[str]] = {name: [] for name in names}
    for name, package in packages.items():
        for dependency in internal_dependencies(package, names):
            dependents[dependency].append(name)
            indegree[name] += 1
    ready = collections.deque(sorted(name for name, degree in indegree.items() if degree == 0))
    ordered: list[str] = []
    while ready:
        name = ready.popleft()
        ordered.append(name)
        for dependent in sorted(dependents[name]):
            indegree[dependent] -= 1
            if indegree[dependent] == 0:
                ready.append(dependent)
    if len(ordered) != len(packages):
        cycle = sorted(name for name, degree in indegree.items() if degree)
        raise ReleaseError(f"normal/build workspace dependency cycle: {cycle}")
    return ordered


def require_version(value: str) -> str:
    if not SEMVER.fullmatch(value):
        raise ReleaseError(f"version must be stable X.Y.Z SemVer, got {value!r}")
    return value


def version_tuple(value: str) -> tuple[int, int, int]:
    require_version(value)
    return tuple(int(part) for part in value.split("."))  # type: ignore[return-value]


def replace_section_version(text: str, section: str, replacement: str) -> str:
    lines = text.splitlines(keepends=True)
    active = False
    found = 0
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped == f"[{section}]":
            active = True
            continue
        if active and stripped.startswith("["):
            active = False
        if active and re.match(r"^version(?:\.workspace)?\s*=", stripped):
            newline = "\n" if line.endswith("\n") else ""
            indent = line[: len(line) - len(line.lstrip())]
            lines[index] = f"{indent}{replacement}{newline}"
            found += 1
    if found != 1:
        raise ReleaseError(f"expected one version key in [{section}], found {found}")
    return "".join(lines)


def update_workspace_dependency_versions(
    text: str, package_names: set[str], version: str
) -> str:
    lines = text.splitlines(keepends=True)
    active = False
    seen: set[str] = set()
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[workspace.dependencies]":
            active = True
            continue
        if active and stripped.startswith("["):
            active = False
        if not active or "=" not in line:
            continue
        name = line.split("=", 1)[0].strip()
        if name not in package_names:
            continue
        pattern = re.compile(r'(version\s*=\s*")[^"]+(")')
        if not pattern.search(line):
            raise ReleaseError(
                f"internal workspace dependency {name} lacks a registry version"
            )
        lines[index] = pattern.sub(rf"\g<1>{version}\g<2>", line, count=1)
        seen.add(name)
    if not seen:
        raise ReleaseError("no internal workspace dependency versions were updated")
    return "".join(lines)


def planned_version_edits(
    root: Path, packages: dict[str, dict[str, Any]], version: str
) -> dict[Path, bytes]:
    root_manifest = root / "Cargo.toml"
    root_text = root_manifest.read_text()
    root_text = replace_section_version(
        root_text, "workspace.package", f'version = "{version}"'
    )
    root_text = update_workspace_dependency_versions(
        root_text, set(packages), version
    )
    changes: dict[Path, bytes] = {root_manifest: root_text.encode()}
    for package in packages.values():
        manifest = Path(package["manifest_path"])
        text = manifest.read_text()
        updated = replace_section_version(
            text, "package", "version.workspace = true"
        )
        changes[manifest] = updated.encode()
    return changes


def write_atomic(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temporary:
        temporary.write(payload)
        temporary.flush()
        os.fsync(temporary.fileno())
        temp_path = Path(temporary.name)
    os.replace(temp_path, path)


def validate_workspace(
    root: Path,
    version: str,
    *,
    locked: bool,
) -> tuple[dict[str, dict[str, Any]], list[str]]:
    metadata = cargo_metadata(root, locked=locked)
    packages = publishable_packages(metadata)
    if len(packages) != EXPECTED_PACKAGE_COUNT:
        raise ReleaseError(
            f"expected {EXPECTED_PACKAGE_COUNT} publishable workspace packages, "
            f"found {len(packages)}"
        )
    wrong = sorted(
        f"{name}@{package['version']}"
        for name, package in packages.items()
        if package["version"] != version
    )
    if wrong:
        raise ReleaseError(f"workspace packages are not unified at {version}: {wrong}")
    root_manifest = (root / "Cargo.toml").read_text()
    active = False
    dependency_versions: dict[str, str] = {}
    for line in root_manifest.splitlines():
        stripped = line.strip()
        if stripped == "[workspace.dependencies]":
            active = True
            continue
        if active and stripped.startswith("["):
            active = False
        if not active or "=" not in line:
            continue
        name = line.split("=", 1)[0].strip()
        if name not in packages:
            continue
        match = re.search(r'version\s*=\s*"([^"]+)"', line)
        if not match:
            raise ReleaseError(f"internal dependency {name} lacks a version")
        dependency_versions[name] = match.group(1)
    wrong_dependencies = sorted(
        f"{name}@{value}"
        for name, value in dependency_versions.items()
        if value != version
    )
    if wrong_dependencies:
        raise ReleaseError(
            f"internal workspace requirements are not {version}: {wrong_dependencies}"
        )
    for package in packages.values():
        manifest = Path(package["manifest_path"]).read_text()
        package_section = manifest.split("[package]", 1)[1].split("\n[", 1)[0]
        if not re.search(r"(?m)^version\.workspace\s*=\s*true(?:\s*#.*)?$", package_section):
            raise ReleaseError(
                f"{package['name']} does not inherit version.workspace"
            )
    ordered = topological_order(packages)
    return packages, ordered


def crates_io_version(name: str, version: str) -> dict[str, Any] | None:
    url = f"https://crates.io/api/v1/crates/{name}/{version}"
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise ReleaseError(f"crates.io returned HTTP {error.code} for {name}@{version}")
    except Exception as error:
        raise ReleaseError(f"crates.io lookup failed for {name}@{version}: {error}")
    value = payload.get("version")
    if not isinstance(value, dict):
        raise ReleaseError(f"crates.io response lacks version data for {name}@{version}")
    return value


def crates_io_latest(name: str) -> str:
    url = f"https://crates.io/api/v1/crates/{name}"
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.load(response)
        crate = payload["crate"]
        return crate.get("newest_version") or crate.get("max_version") or "UNKNOWN"
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return "UNPUBLISHED"
        return f"HTTP-{error.code}"
    except Exception as error:
        return f"ERROR-{type(error).__name__}"


def audit(root: Path) -> None:
    packages = publishable_packages(cargo_metadata(root, locked=False))
    ordered = topological_order(packages)
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        futures = {
            name: executor.submit(crates_io_latest, name) for name in packages
        }
        latest = {name: future.result() for name, future in futures.items()}
    print(f"{'SEQ':>3}  {'CRATE':<28} {'LOCAL':<10} {'CRATES.IO':<14}")
    print("-" * 62)
    for sequence, name in enumerate(ordered, 1):
        print(
            f"{sequence:>3}  {name:<28} "
            f"{packages[name]['version']:<10} {latest[name]:<14}"
        )
    print(f"\n{len(ordered)} publishable workspace packages; one unified release graph.")


def prepare(root: Path, version: str) -> None:
    ensure_clean(root)
    metadata = cargo_metadata(root, locked=True)
    packages = publishable_packages(metadata)
    if len(packages) != EXPECTED_PACKAGE_COUNT:
        raise ReleaseError(
            f"expected {EXPECTED_PACKAGE_COUNT} publishable packages, found {len(packages)}"
        )
    local_max = max(version_tuple(package["version"]) for package in packages.values())
    if version_tuple(version) < local_max:
        raise ReleaseError(
            f"target {version} would move a package backward from "
            f"{'.'.join(str(part) for part in local_max)}"
        )
    existing = sorted(
        name for name in packages if crates_io_version(name, version) is not None
    )
    if existing:
        raise ReleaseError(
            f"cannot prepare an already-published version for: {existing}"
        )
    edits = planned_version_edits(root, packages, version)
    lock_path = root / "Cargo.lock"
    originals = {
        path: path.read_bytes() if path.exists() else None
        for path in [*edits, lock_path]
    }
    try:
        for path, payload in edits.items():
            write_atomic(path, payload)
        run(["cargo", "metadata", "--format-version", "1"], cwd=root)
        validate_workspace(root, version, locked=True)
        run(
            ["cargo", "check", "--workspace", "--all-targets", "--locked"],
            cwd=root,
            capture=False,
        )
    except Exception:
        for path, payload in originals.items():
            if payload is None:
                path.unlink(missing_ok=True)
            else:
                write_atomic(path, payload)
        raise
    print(f"prepared all {len(packages)} workspace packages at {version}")
    print("review and commit the manifest and Cargo.lock changes before verification")


class ReleaseLog:
    def __init__(self, root: Path, version: str, operation: str):
        timestamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        self.directory = (
            root / "target/release-logs" / version / f"{timestamp}-{operation}"
        )
        self.directory.mkdir(parents=True, exist_ok=False)
        self.text_path = self.directory / "commands.log"
        self.events_path = self.directory / "events.jsonl"

    def message(self, value: str) -> None:
        print(value, flush=True)
        with self.text_path.open("a") as stream:
            stream.write(value + "\n")

    def event(self, kind: str, **values: Any) -> None:
        payload = {"at": utc_now(), "kind": kind, **values}
        with self.events_path.open("a") as stream:
            stream.write(json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n")

    def command(self, argv: list[str], cwd: Path) -> None:
        self.message(f"$ {' '.join(argv)}")
        self.event("command", argv=argv)
        with self.text_path.open("a") as stream:
            process = subprocess.Popen(
                argv,
                cwd=cwd,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
            )
            assert process.stdout is not None
            for line in process.stdout:
                print(line, end="", flush=True)
                stream.write(line)
            code = process.wait()
        self.event("command-result", argv=argv, exit_status=code)
        if code:
            raise ReleaseError(f"command failed ({code}): {' '.join(argv)}")

    def command_capture(self, argv: list[str], cwd: Path) -> str:
        self.message(f"$ {' '.join(argv)}")
        self.event("command", argv=argv)
        completed = subprocess.run(
            argv,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
        output = completed.stdout or ""
        print(output, end="", flush=True)
        with self.text_path.open("a") as stream:
            stream.write(output)
        self.event(
            "command-result",
            argv=argv,
            exit_status=completed.returncode,
        )
        if completed.returncode:
            raise ReleaseError(
                f"command failed ({completed.returncode}): {' '.join(argv)}"
            )
        return output


def verify_beta_reporting(
    root: Path,
    version: str,
    beta_report_root: str | None,
    beta_exception_attestation: str | None,
    log: ReleaseLog,
) -> dict[str, Any]:
    if beta_report_root and beta_exception_attestation:
        raise ReleaseError(
            "--beta-report-root and --beta-exception-attestation are mutually exclusive"
        )
    if beta_exception_attestation:
        attestation = Path(beta_exception_attestation)
        if not attestation.is_absolute():
            attestation = root / attestation
        verifier = root / "scripts/release_exception_attestation.py"
        command = [
            sys.executable,
            str(verifier),
            "verify",
            "--attestation",
            str(attestation),
            "--version",
            version,
        ]
        log.command(command, root)
        payload = json.loads(attestation.read_text())
        relative = (
            attestation.relative_to(root).as_posix()
            if attestation.is_relative_to(root)
            else str(attestation)
        )
        return {
            "mode": "owner-approved-exception",
            "disposition": payload["release"]["disposition"],
            "strict_automated_status": payload["release"][
                "strict_automated_status"
            ],
            "attestation_path": relative,
            "attestation_sha256": hashlib.sha256(attestation.read_bytes()).hexdigest(),
        }

    crate = root / "crates/sip/rvoip-sip"
    reporter = crate / "scripts/beta_release_report.py"
    docs = crate / "docs"
    command = [
        sys.executable,
        str(reporter),
        "verify",
        "--docs-root",
        str(docs),
    ]
    if beta_report_root:
        command.extend(["--report-root", beta_report_root])
    log.command(command, root)
    return {
        "mode": "strict",
        "disposition": "RELEASE-CANDIDATE",
        "strict_automated_status": "PASS",
        "report_root": beta_report_root or "docs-current",
    }


def package_artifact(
    root: Path, name: str, version: str, log: ReleaseLog
) -> tuple[Path, str]:
    artifact = root / "target/package" / f"{name}-{version}.crate"
    artifact.unlink(missing_ok=True)
    log.command(
        ["cargo", "package", "-p", name, "--locked", "--no-verify"],
        root,
    )
    if not artifact.is_file():
        raise ReleaseError(f"cargo package did not create {artifact}")
    digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
    log.event(
        "package",
        crate=name,
        version=version,
        path=artifact.relative_to(root).as_posix(),
        sha256=digest,
        bytes=artifact.stat().st_size,
    )
    return artifact, digest


def package_file_manifest(
    root: Path, name: str, version: str, log: ReleaseLog
) -> tuple[list[str], str]:
    output = log.command_capture(
        ["cargo", "package", "-p", name, "--list", "--locked"],
        root,
    )
    paths = [line.strip() for line in output.splitlines() if line.strip()]
    if not paths or len(paths) != len(set(paths)):
        raise ReleaseError(f"invalid or duplicate package file list for {name}")
    for path in paths:
        candidate = Path(path)
        if candidate.is_absolute() or ".." in candidate.parts:
            raise ReleaseError(f"unsafe package path for {name}: {path!r}")
    if "Cargo.toml" not in paths or "Cargo.toml.orig" not in paths:
        raise ReleaseError(f"package file list for {name} lacks Cargo manifests")
    digest = hashlib.sha256(("\n".join(paths) + "\n").encode()).hexdigest()
    log.event(
        "package-file-manifest",
        crate=name,
        version=version,
        file_count=len(paths),
        sha256=digest,
    )
    return paths, digest


def write_verification_receipt(
    root: Path,
    version: str,
    head: str,
    ordered: list[str],
    log: ReleaseLog,
    package_hashes: dict[str, str],
    package_file_hashes: dict[str, str],
    beta_qualification: dict[str, Any],
) -> None:
    receipt = {
        "schema": "rvoip-unified-release-verification-v3",
        "verified_at": utc_now(),
        "version": version,
        "git_commit": head,
        "package_count": len(ordered),
        "ordered_packages": ordered,
        "package_sha256": package_hashes,
        "package_file_manifest_sha256": package_file_hashes,
        "beta_qualification": beta_qualification,
        "package_hash_scope": (
            "Pre-publication .crate hashes exist only where all target-version "
            "registry dependencies were already resolvable. Publication records "
            "every final .crate hash after leaf-first dependency visibility."
        ),
        "log_directory": log.directory.relative_to(root).as_posix(),
    }
    destination = root / "target/release-logs" / version / "verification.json"
    write_atomic(
        destination,
        (json.dumps(receipt, indent=2, sort_keys=True) + "\n").encode(),
    )
    log.event("verification-receipt", path=destination.relative_to(root).as_posix())


def verify(
    root: Path,
    version: str,
    beta_report_root: str | None,
    beta_exception_attestation: str | None,
) -> None:
    head = ensure_release_state(root, version, require_no_tag=True)
    packages, ordered = validate_workspace(root, version, locked=True)
    log = ReleaseLog(root, version, "verify")
    log.event("start", operation="verify", version=version, git_commit=head)
    beta_qualification = verify_beta_reporting(
        root,
        version,
        beta_report_root,
        beta_exception_attestation,
        log,
    )
    commands = [
        ["cargo", "check", "--workspace", "--all-targets", "--locked"],
        ["cargo", "test", "--workspace", "--lib", "--locked"],
        [
            "cargo",
            "test",
            "--workspace",
            "--bins",
            "--examples",
            "--tests",
            "--locked",
        ],
        ["cargo", "test", "--workspace", "--doc", "--locked"],
    ]
    for command in commands:
        log.command(command, root)
    package_hashes: dict[str, str] = {}
    package_file_hashes: dict[str, str] = {}
    visibility_cache: dict[str, bool] = {}

    def registry_visible(name: str) -> bool:
        if name not in visibility_cache:
            visibility_cache[name] = crates_io_version(name, version) is not None
        return visibility_cache[name]

    for index, name in enumerate(ordered, 1):
        log.message(f"== package {index}/{len(ordered)}: {name}")
        _, package_file_hashes[name] = package_file_manifest(
            root, name, version, log
        )
        dependencies = internal_dependencies(packages[name], set(packages))
        unresolved = sorted(
            dependency
            for dependency in dependencies
            if not registry_visible(dependency)
        )
        if unresolved:
            log.message(
                "archive deferred until target-version dependencies are visible: "
                + ", ".join(unresolved)
            )
            log.event(
                "package-archive-deferred",
                crate=name,
                dependencies=unresolved,
            )
            continue
        _, package_hashes[name] = package_artifact(root, name, version, log)
    write_verification_receipt(
        root,
        version,
        head,
        ordered,
        log,
        package_hashes,
        package_file_hashes,
        beta_qualification,
    )
    log.event("complete", operation="verify", package_count=len(packages))
    log.message(f"verified {len(packages)} packages at {version}")


def read_verification_receipt(
    root: Path, version: str, head: str, ordered: list[str]
) -> dict[str, Any]:
    path = root / "target/release-logs" / version / "verification.json"
    if not path.is_file():
        raise ReleaseError(f"missing verification receipt: {path}")
    receipt = json.loads(path.read_text())
    file_hashes = receipt.get("package_file_manifest_sha256")
    package_hashes = receipt.get("package_sha256")
    expected = (
        receipt.get("schema") == "rvoip-unified-release-verification-v3"
        and receipt.get("version") == version
        and receipt.get("git_commit") == head
        and receipt.get("ordered_packages") == ordered
        and receipt.get("package_count") == len(ordered)
        and isinstance(file_hashes, dict)
        and set(file_hashes) == set(ordered)
        and isinstance(package_hashes, dict)
        and set(package_hashes) <= set(ordered)
        and isinstance(receipt.get("beta_qualification"), dict)
        and receipt["beta_qualification"].get("mode")
        in {"strict", "owner-approved-exception"}
    )
    if not expected:
        raise ReleaseError("verification receipt does not match this release commit")
    return receipt


def assert_existing_checksum(
    name: str, version: str, local_sha256: str, remote: dict[str, Any]
) -> None:
    remote_checksum = remote.get("checksum")
    if not isinstance(remote_checksum, str) or remote_checksum != local_sha256:
        raise ReleaseError(
            f"{name}@{version} already exists but checksum differs "
            f"(local {local_sha256}, crates.io {remote_checksum})"
        )


def wait_for_version(
    name: str,
    version: str,
    *,
    local_sha256: str,
    poll_seconds: int,
    timeout_seconds: int,
    log: ReleaseLog,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    while True:
        remote = crates_io_version(name, version)
        if remote is not None:
            assert_existing_checksum(name, version, local_sha256, remote)
            log.event(
                "visible",
                crate=name,
                version=version,
                sha256=remote.get("checksum"),
            )
            return
        if time.monotonic() >= deadline:
            raise ReleaseError(
                f"{name}@{version} was not visible within {timeout_seconds} seconds"
            )
        time.sleep(poll_seconds)


def credential_preflight(root: Path, first_crate: str, log: ReleaseLog) -> None:
    log.command(
        ["cargo", "owner", "--list", first_crate, "--registry", "crates-io"],
        root,
    )


def publish(
    root: Path,
    version: str,
    *,
    execute: bool,
) -> None:
    head = ensure_release_state(root, version, require_no_tag=True)
    packages, ordered = validate_workspace(root, version, locked=True)
    receipt = read_verification_receipt(root, version, head, ordered)
    verified_hashes = receipt.get("package_sha256")
    verified_file_hashes = receipt.get("package_file_manifest_sha256")
    if (
        not isinstance(verified_hashes, dict)
        or not isinstance(verified_file_hashes, dict)
        or set(verified_hashes) > set(ordered)
        or set(verified_file_hashes) != set(ordered)
    ):
        raise ReleaseError("verification receipt lacks valid package evidence")
    log = ReleaseLog(root, version, "publish" if execute else "dry-run")
    log.event(
        "start",
        operation="publish",
        mode="execute" if execute else "dry-run",
        version=version,
        git_commit=head,
    )
    if execute:
        credential_preflight(root, ordered[0], log)
    visible: set[str] = set()
    for index, name in enumerate(ordered, 1):
        log.message(f"== {index}/{len(ordered)} {name}@{version}")
        _, file_manifest_sha256 = package_file_manifest(
            root, name, version, log
        )
        if file_manifest_sha256 != verified_file_hashes[name]:
            raise ReleaseError(
                f"package file manifest for {name} differs from verification "
                f"({verified_file_hashes[name]} != {file_manifest_sha256})"
            )
        dependencies = internal_dependencies(packages[name], set(packages))
        missing_dependencies = sorted(dependencies - visible)
        if missing_dependencies and not execute:
            log.message(
                f"pending dry-run until these {version} dependencies are live: "
                + ", ".join(missing_dependencies)
            )
            log.event(
                "dry-run-pending",
                crate=name,
                dependencies=missing_dependencies,
            )
            continue
        if missing_dependencies:
            raise ReleaseError(
                f"topological publish invariant failed for {name}: "
                f"{missing_dependencies}"
            )
        _, local_sha256 = package_artifact(root, name, version, log)
        verified_sha256 = verified_hashes.get(name)
        if verified_sha256 is not None and local_sha256 != verified_sha256:
            raise ReleaseError(
                f"package {name} differs from its pre-publication verified artifact "
                f"({verified_sha256} != {local_sha256})"
            )
        remote = crates_io_version(name, version)
        if remote is not None:
            assert_existing_checksum(name, version, local_sha256, remote)
            visible.add(name)
            log.message(f"resume: verified existing {name}@{version}")
            log.event("resume", crate=name, version=version, sha256=local_sha256)
            continue
        log.command(
            [
                "cargo",
                "publish",
                "-p",
                name,
                "--locked",
                "--registry",
                "crates-io",
                "--dry-run",
            ],
            root,
        )
        if execute:
            log.command(
                [
                    "cargo",
                    "publish",
                    "-p",
                    name,
                    "--locked",
                    "--registry",
                    "crates-io",
                ],
                root,
            )
            wait_for_version(
                name,
                version,
                local_sha256=local_sha256,
                poll_seconds=int(
                    os.environ.get(
                        "RVOIP_RELEASE_POLL_SECONDS", DEFAULT_POLL_SECONDS
                    )
                ),
                timeout_seconds=int(
                    os.environ.get(
                        "RVOIP_RELEASE_TIMEOUT_SECONDS", DEFAULT_TIMEOUT_SECONDS
                    )
                ),
                log=log,
            )
            visible.add(name)
        else:
            log.event("dry-run-pass", crate=name, version=version)
    if execute and len(visible) != len(ordered):
        raise ReleaseError(
            f"only {len(visible)}/{len(ordered)} packages became visible"
        )
    log.event(
        "complete",
        operation="publish",
        mode="execute" if execute else "dry-run",
        package_count=len(ordered),
        visible_count=len(visible),
    )
    if execute:
        log.message(f"published and verified all {len(ordered)} packages at {version}")
    else:
        log.message(
            f"dry-run/package preflight complete for all {len(ordered)} packages"
        )


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        description="One release authority for every publishable rvoip crate."
    )
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("audit", help="read-only local/crates.io version audit")
    for name in ("prepare", "verify", "publish"):
        command = commands.add_parser(name)
        command.add_argument("--version", required=True)
        if name == "verify":
            report_group = command.add_mutually_exclusive_group()
            report_group.add_argument(
                "--beta-report-root",
                default=os.environ.get("RVOIP_BETA_REPORT_ROOT"),
            )
            report_group.add_argument(
                "--beta-exception-attestation",
                default=os.environ.get("RVOIP_BETA_EXCEPTION_ATTESTATION"),
            )
        if name == "publish":
            command.add_argument(
                "--execute",
                action="store_true",
                help="publish irreversibly; default is dry-run",
            )
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    root = repository_root()
    try:
        if args.command == "audit":
            audit(root)
        else:
            version = require_version(args.version)
            if args.command == "prepare":
                prepare(root, version)
            elif args.command == "verify":
                verify(
                    root,
                    version,
                    args.beta_report_root,
                    args.beta_exception_attestation,
                )
            elif args.command == "publish":
                publish(root, version, execute=args.execute)
    except ReleaseError as error:
        print(f"release: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
