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
RELEASE_LOCK_MANIFESTS = (Path("Cargo.toml"), Path("examples/Cargo.toml"))
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
COMMIT_SHA = re.compile(r"^[0-9a-f]{40}$")
USER_AGENT = "rvoip-unified-release/1.0"
DEFAULT_POLL_SECONDS = 15
DEFAULT_TIMEOUT_SECONDS = 900
VERIFICATION_RECEIPT_SCHEMA = "rvoip-unified-release-verification-v4"
REMOTE_QUALIFICATION_SCHEMA = "rvoip-release-qualification-v1"
REMOTE_GATE_CATALOG_SCHEMA = "rvoip-release-gate-catalog-v1"
TARGETED_DELTA_ATTESTATION_SCHEMA = "rvoip-targeted-delta-attestation-v1"
TARGETED_POSTGRES_EVIDENCE_SCHEMA = "rvoip-vcon-postgres-live-evidence-v1"
CARRY_FORWARD_ATTESTATION_SCHEMA = "rvoip-release-carry-forward-attestation-v1"
VCON_SCHEMA_COMMIT = "2342aba64bdb71d9e80ab6e274a3921e2b1c769e"
TARGETED_DELTA_VERSION = "0.3.3"
TARGETED_DELTA_BASE_TAG = "v0.3.2"
TARGETED_DELTA_EXACT_PATHS = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "README.md",
        "CHANGELOG.md",
        "docs/RELEASING.md",
        "docs/PRD.md",
        "docs/CONVERSATION_PROTOCOL.md",
        "docs/INTERFACE_DESIGN.md",
        "docs/GAP_PLAN.md",
        ".github/workflows/vcon.yml",
        "scripts/release.py",
        "scripts/release.sh",
        "scripts/test_release.py",
        "crates/sip/rvoip-sip/Cargo.toml",
        "crates/webrtc/rvoip-webrtc-stack/Cargo.toml",
        "crates/foundation/rvoip-core/Cargo.toml",
        "crates/foundation/rvoip-core/README.md",
        "crates/foundation/rvoip-core/src/events.rs",
        "crates/foundation/rvoip-core/src/lib.rs",
        "crates/foundation/rvoip-core/src/orchestrator.rs",
        "crates/foundation/rvoip-core/src/vcon.rs",
        "crates/foundation/rvoip-core/src/store/vcon_store.rs",
        "crates/foundation/rvoip-core/tests/vcon_emission.rs",
        "crates/rvoip/README.md",
        "crates/rvoip/src/lib.rs",
        "examples/README.md",
        "crates/uctp/rvoip-quic/Cargo.toml",
        "crates/uctp/rvoip-quic/tests/e2e_full_stack.rs",
        "crates/uctp/rvoip-uctp/UCTP_IMPLEMENTATION_PLAN.md",
    }
)
TARGETED_DELTA_PATH_PREFIXES = (
    "crates/extensions/rvoip-vcon/",
    "crates/extensions/rvoip-vcon-postgres/",
    "examples/11-ai-harness-demo/",
)

TARGETED_DELTA_COMMANDS: tuple[tuple[str, tuple[str, ...]], ...] = (
    (
        "vcon-all-targets",
        ("cargo", "test", "-p", "rvoip-vcon", "--all-targets", "--locked"),
    ),
    (
        "core-vcon-lib",
        ("cargo", "test", "-p", "rvoip-core", "--lib", "vcon", "--locked"),
    ),
    (
        "core-vcon-emission",
        (
            "cargo",
            "test",
            "-p",
            "rvoip-core",
            "--test",
            "vcon_emission",
            "--locked",
        ),
    ),
    (
        "core-no-default-features",
        (
            "cargo",
            "check",
            "-p",
            "rvoip-core",
            "--no-default-features",
            "--all-targets",
            "--locked",
        ),
    ),
    (
        "core-all-features",
        (
            "cargo",
            "check",
            "-p",
            "rvoip-core",
            "--all-features",
            "--all-targets",
            "--locked",
        ),
    ),
    (
        "quic-e2e-full-stack",
        (
            "cargo",
            "test",
            "-p",
            "rvoip-quic",
            "--test",
            "e2e_full_stack",
            "--locked",
        ),
    ),
    (
        "facade-voip-3",
        ("cargo", "check", "-p", "rvoip", "--features", "voip-3", "--locked"),
    ),
    (
        "ai-harness-example",
        (
            "cargo",
            "check",
            "--manifest-path",
            "examples/11-ai-harness-demo/Cargo.toml",
        ),
    ),
    (
        "release-unit-tests",
        ("python3", "-m", "unittest", "scripts/test_release.py"),
    ),
)
TARGETED_POSTGRES_COMMAND = (
    "cargo",
    "test",
    "-p",
    "rvoip-vcon-postgres",
    "--all-targets",
    "--features",
    "core-store,live-tests",
    "--locked",
)


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


def release_lock_paths(root: Path) -> tuple[Path, ...]:
    return tuple(root / manifest.parent / "Cargo.lock" for manifest in RELEASE_LOCK_MANIFESTS)


def refresh_release_lockfiles(root: Path) -> None:
    for manifest in RELEASE_LOCK_MANIFESTS:
        manifest_path = root / manifest
        if not manifest_path.is_file():
            raise ReleaseError(f"release lock manifest is missing: {manifest}")
        run(
            [
                "cargo",
                "metadata",
                "--manifest-path",
                str(manifest),
                "--format-version",
                "1",
            ],
            cwd=root,
        )


def validate_release_lockfiles(root: Path) -> None:
    for manifest in RELEASE_LOCK_MANIFESTS:
        run(
            [
                "cargo",
                "metadata",
                "--manifest-path",
                str(manifest),
                "--format-version",
                "1",
                "--locked",
            ],
            cwd=root,
        )


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
        package_match = re.search(r'\bpackage\s*=\s*"([^"]+)"', line)
        package_name = package_match.group(1) if package_match else name
        if package_name not in package_names:
            continue
        pattern = re.compile(r'(version\s*=\s*")[^"]+(")')
        if not pattern.search(line):
            raise ReleaseError(
                "internal workspace dependency "
                f"{name}->{package_name} lacks a registry version"
            )
        lines[index] = pattern.sub(rf"\g<1>{version}\g<2>", line, count=1)
        seen.add(name)
    if not seen:
        raise ReleaseError("no internal workspace dependency versions were updated")
    return "".join(lines)


def update_member_dependency_versions(
    text: str, dependency_names: set[str], version: str
) -> str:
    lines = text.splitlines(keepends=True)
    dependency_section = re.compile(
        r"^\[(?:target\..+\.)?"
        r"(?:dependencies|dev-dependencies|build-dependencies)\]$"
    )
    dependency_table = re.compile(
        r"^\[(?:target\..+\.)?"
        r"(?:dependencies|dev-dependencies|build-dependencies)"
        r"\.([A-Za-z0-9_-]+)\]$"
    )
    active_section = False
    active_dependency: str | None = None
    version_field = re.compile(r'(version\s*=\s*")[^"]+(")')
    table_version = re.compile(r'^(\s*version\s*=\s*")[^"]+(")')
    simple_version = re.compile(
        r'^(\s*[A-Za-z0-9_-]+\s*=\s*")[^"]+(")'
    )

    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("["):
            table_match = dependency_table.fullmatch(stripped)
            active_dependency = (
                table_match.group(1)
                if table_match and table_match.group(1) in dependency_names
                else None
            )
            active_section = dependency_section.fullmatch(stripped) is not None
            continue
        if active_dependency is not None:
            if table_version.search(line):
                lines[index] = table_version.sub(
                    rf"\g<1>{version}\g<2>", line, count=1
                )
            continue
        if not active_section or "=" not in line:
            continue
        name = line.split("=", 1)[0].strip()
        if name not in dependency_names:
            continue
        if version_field.search(line):
            lines[index] = version_field.sub(
                rf"\g<1>{version}\g<2>", line, count=1
            )
        elif simple_version.search(line):
            lines[index] = simple_version.sub(
                rf"\g<1>{version}\g<2>", line, count=1
            )
    return "".join(lines)


def validate_member_dependency_versions(
    packages: dict[str, dict[str, Any]], version: str
) -> None:
    expected_requirement = f"^{version}"
    wrong: set[str] = set()
    for package_name, package in packages.items():
        for dependency in package.get("dependencies", []):
            dependency_name = dependency.get("name")
            if dependency_name not in packages:
                continue
            requirement = dependency.get("req")
            if requirement == expected_requirement or (
                dependency.get("kind") == "dev" and requirement == "*"
            ):
                continue
            declared_name = dependency.get("rename") or dependency_name
            target = (
                f"{declared_name} ({dependency_name})"
                if declared_name != dependency_name
                else str(dependency_name)
            )
            wrong.add(f"{package_name} -> {target}@{requirement}")
    if wrong:
        raise ReleaseError(
            f"member internal requirements are not {version}: {sorted(wrong)}"
        )


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
        internal_dependency_names = {
            dependency.get("rename") or dependency.get("name")
            for dependency in package.get("dependencies", [])
            if dependency.get("name") in packages
        }
        updated = update_member_dependency_versions(
            updated, internal_dependency_names, version
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
    validate_member_dependency_versions(packages, version)
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
    lock_paths = release_lock_paths(root)
    originals = {
        path: path.read_bytes() if path.exists() else None
        for path in [*edits, *lock_paths]
    }
    try:
        for path, payload in edits.items():
            write_atomic(path, payload)
        refresh_release_lockfiles(root)
        validate_release_lockfiles(root)
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
    print("review and commit the manifest and lockfile changes before verification")


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


def relative_or_absolute(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix() if path.is_relative_to(root) else str(path)


def load_json_object(path: Path, description: str) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ReleaseError(f"cannot read {description} {path}: {error}") from error
    if not isinstance(payload, dict):
        raise ReleaseError(f"{description} must be a JSON object: {path}")
    return payload


def require_nonempty_string(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ReleaseError(f"targeted delta attestation requires non-empty {field}")
    return value


def require_timestamp(value: Any, field: str) -> str:
    timestamp = require_nonempty_string(value, field)
    try:
        parsed = dt.datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError as error:
        raise ReleaseError(
            f"targeted delta attestation requires ISO-8601 {field}"
        ) from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ReleaseError(f"targeted delta attestation requires timezone-aware {field}")
    return timestamp


def targeted_delta_path_allowed(path: str) -> bool:
    return path in TARGETED_DELTA_EXACT_PATHS or path.startswith(
        TARGETED_DELTA_PATH_PREFIXES
    )


def verify_attested_command(
    record: Any,
    *,
    name: str,
    argv: tuple[str, ...],
    head: str,
) -> None:
    if not isinstance(record, dict):
        raise ReleaseError(f"targeted command {name!r} must be a JSON object")
    if record.get("name") != name:
        raise ReleaseError(f"targeted command name mismatch for {name!r}")
    if record.get("argv") != list(argv):
        raise ReleaseError(f"targeted command argv mismatch for {name!r}")
    if record.get("exit_status") != 0:
        raise ReleaseError(f"targeted command {name!r} did not attest exit status 0")
    if record.get("git_commit") != head:
        raise ReleaseError(f"targeted command {name!r} is not bound to release commit")


def verify_targeted_delta_attestation(
    root: Path,
    version: str,
    head: str,
    attestation_value: str,
    log: ReleaseLog,
) -> tuple[dict[str, Any], list[tuple[str, list[str]]]]:
    attestation = Path(attestation_value)
    if not attestation.is_absolute():
        attestation = root / attestation
    if not attestation.is_file():
        raise ReleaseError(f"missing targeted delta attestation: {attestation}")
    payload = load_json_object(attestation, "targeted delta attestation")
    if payload.get("schema") != TARGETED_DELTA_ATTESTATION_SCHEMA:
        raise ReleaseError("unexpected targeted delta attestation schema")
    if version != TARGETED_DELTA_VERSION:
        raise ReleaseError(
            f"targeted delta mode is limited to release {TARGETED_DELTA_VERSION}"
        )

    release = payload.get("release")
    if not isinstance(release, dict):
        raise ReleaseError("targeted delta attestation lacks release metadata")
    if release.get("version") != version:
        raise ReleaseError("targeted delta attestation version mismatch")
    if release.get("git_commit") != head or not COMMIT_SHA.fullmatch(head):
        raise ReleaseError("targeted delta attestation release commit mismatch")
    if release.get("vcon_schema_commit") != VCON_SCHEMA_COMMIT:
        raise ReleaseError("targeted delta attestation vCon schema commit mismatch")

    base_commit = release.get("base_commit")
    if not isinstance(base_commit, str) or not COMMIT_SHA.fullmatch(base_commit):
        raise ReleaseError("targeted delta base commit must be a full lowercase SHA-1")
    try:
        resolved_base = git_output(
            root, "rev-parse", "--verify", f"{base_commit}^{{commit}}"
        )
    except ReleaseError as error:
        raise ReleaseError(
            f"targeted delta base commit does not exist: {base_commit}"
        ) from error
    if resolved_base != base_commit:
        raise ReleaseError("targeted delta base commit did not resolve exactly")
    try:
        expected_base = git_output(
            root,
            "rev-parse",
            "--verify",
            f"refs/tags/{TARGETED_DELTA_BASE_TAG}^{{commit}}",
        )
    except ReleaseError as error:
        raise ReleaseError(
            f"targeted delta requires immutable base tag {TARGETED_DELTA_BASE_TAG}"
        ) from error
    if base_commit != expected_base:
        raise ReleaseError(
            "targeted delta base commit must be the commit identified by "
            f"{TARGETED_DELTA_BASE_TAG}"
        )
    ancestor = run(
        ["git", "merge-base", "--is-ancestor", base_commit, head],
        cwd=root,
        check=False,
    )
    if ancestor.returncode:
        raise ReleaseError("targeted delta base commit is not an ancestor of release")

    actual_paths = sorted(
        line
        for line in git_output(
            root,
            "diff",
            "--name-only",
            "--diff-filter=ACDMRTUXB",
            base_commit,
            head,
        ).splitlines()
        if line
    )
    disallowed_paths = [
        path for path in actual_paths if not targeted_delta_path_allowed(path)
    ]
    if disallowed_paths:
        raise ReleaseError(
            "targeted delta contains paths outside the hard-coded vCon release "
            f"policy: {disallowed_paths}"
        )
    allowed_paths = payload.get("allowed_changed_paths")
    if (
        not isinstance(allowed_paths, list)
        or not allowed_paths
        or any(not isinstance(path, str) or not path for path in allowed_paths)
        or allowed_paths != sorted(set(allowed_paths))
    ):
        raise ReleaseError(
            "targeted delta allowed_changed_paths must be a non-empty sorted unique list"
        )
    if actual_paths != allowed_paths:
        missing = sorted(set(actual_paths) - set(allowed_paths))
        excess = sorted(set(allowed_paths) - set(actual_paths))
        raise ReleaseError(
            "targeted delta changed paths differ from the exact allowlist "
            f"(not allowed: {missing}; not changed: {excess})"
        )

    command_records = payload.get("commands")
    if not isinstance(command_records, list):
        raise ReleaseError("targeted delta attestation lacks command records")
    by_name: dict[str, dict[str, Any]] = {}
    for record in command_records:
        if not isinstance(record, dict) or not isinstance(record.get("name"), str):
            raise ReleaseError("targeted delta command record is malformed")
        name = record["name"]
        if name in by_name:
            raise ReleaseError(f"duplicate targeted command record: {name}")
        by_name[name] = record
    expected_names = {name for name, _ in TARGETED_DELTA_COMMANDS}
    if set(by_name) != expected_names:
        raise ReleaseError(
            "targeted command names differ from the approved matrix "
            f"(missing: {sorted(expected_names - set(by_name))}; "
            f"unexpected: {sorted(set(by_name) - expected_names)})"
        )
    commands: list[tuple[str, list[str]]] = []
    for name, argv in TARGETED_DELTA_COMMANDS:
        verify_attested_command(by_name[name], name=name, argv=argv, head=head)
        commands.append((name, list(argv)))

    postgresql = payload.get("postgresql")
    if not isinstance(postgresql, dict) or postgresql.get("live_database") is not True:
        raise ReleaseError("targeted delta requires explicit live PostgreSQL evidence")
    if postgresql.get("ephemeral_database") is not True:
        raise ReleaseError("targeted delta PostgreSQL evidence must use an ephemeral database")
    server_version = require_nonempty_string(
        postgresql.get("server_version"), "postgresql.server_version"
    )
    environment = postgresql.get("environment")
    if not isinstance(environment, dict):
        raise ReleaseError("targeted delta lacks PostgreSQL environment metadata")
    postgres_environment = {
        "provider": require_nonempty_string(
            environment.get("provider"), "postgresql.environment.provider"
        ),
        "image": require_nonempty_string(
            environment.get("image"), "postgresql.environment.image"
        ),
        "database": require_nonempty_string(
            environment.get("database"), "postgresql.environment.database"
        ),
        "run_id": require_nonempty_string(
            environment.get("run_id"), "postgresql.environment.run_id"
        ),
    }
    if environment != postgres_environment:
        raise ReleaseError(
            "PostgreSQL environment must contain exactly provider, image, database, and run_id"
        )
    postgres_command = postgresql.get("command")
    verify_attested_command(
        postgres_command,
        name="postgres-core-store-live",
        argv=TARGETED_POSTGRES_COMMAND,
        head=head,
    )
    evidence = postgresql.get("evidence")
    if not isinstance(evidence, dict):
        raise ReleaseError("targeted delta lacks PostgreSQL evidence metadata")
    evidence_value = require_nonempty_string(
        evidence.get("path"), "postgresql.evidence.path"
    )
    evidence_path = Path(evidence_value)
    if not evidence_path.is_absolute():
        evidence_path = root / evidence_path
    if not evidence_path.is_file():
        raise ReleaseError(f"missing PostgreSQL evidence file: {evidence_path}")
    evidence_sha256 = evidence.get("sha256")
    actual_evidence_sha256 = hashlib.sha256(evidence_path.read_bytes()).hexdigest()
    if (
        not isinstance(evidence_sha256, str)
        or not re.fullmatch(r"[0-9a-f]{64}", evidence_sha256)
        or evidence_sha256 != actual_evidence_sha256
    ):
        raise ReleaseError("PostgreSQL evidence SHA-256 mismatch")
    evidence_payload = load_json_object(
        evidence_path, "targeted delta PostgreSQL evidence"
    )
    expected_evidence = {
        "schema": TARGETED_POSTGRES_EVIDENCE_SCHEMA,
        "git_commit": head,
        "argv": list(TARGETED_POSTGRES_COMMAND),
        "exit_status": 0,
        "live_database": True,
        "ephemeral_database": True,
        "server_version": server_version,
        "environment": postgres_environment,
    }
    for field, expected_value in expected_evidence.items():
        if evidence_payload.get(field) != expected_value:
            raise ReleaseError(
                f"PostgreSQL evidence {field} does not match the release"
            )
    postgres_recorded_at = require_timestamp(
        evidence_payload.get("recorded_at"), "postgresql.recorded_at"
    )

    approval = payload.get("approval")
    if not isinstance(approval, dict):
        raise ReleaseError("targeted delta attestation lacks approval metadata")
    approved_by = require_nonempty_string(
        approval.get("approved_by"), "approval.approved_by"
    )
    approved_at = require_timestamp(
        approval.get("approved_at"), "approval.approved_at"
    )
    rationale = require_nonempty_string(
        approval.get("rationale"), "approval.rationale"
    )

    attestation_sha256 = hashlib.sha256(attestation.read_bytes()).hexdigest()
    qualification = {
        "mode": "targeted-delta",
        "disposition": "APPROVED-TARGETED-DELTA",
        "strict_automated_status": "NOT-RERUN",
        "workspace_test_status": "TARGETED-ONLY",
        "base_commit": base_commit,
        "vcon_schema_commit": VCON_SCHEMA_COMMIT,
        "changed_paths": actual_paths,
        "changed_path_count": len(actual_paths),
        "targeted_command_count": len(commands),
        "attestation_path": relative_or_absolute(root, attestation),
        "attestation_sha256": attestation_sha256,
        "approved_by": approved_by,
        "approved_at": approved_at,
        "rationale": rationale,
        "postgresql": {
            "status": "ATTESTED-PASS",
            "live_database": True,
            "ephemeral_database": True,
            "server_version": server_version,
            "environment": postgres_environment,
            "recorded_at": postgres_recorded_at,
            "command": list(TARGETED_POSTGRES_COMMAND),
            "evidence_path": relative_or_absolute(root, evidence_path),
            "evidence_sha256": evidence_sha256,
        },
    }
    log.event(
        "targeted-delta-attestation",
        path=qualification["attestation_path"],
        sha256=attestation_sha256,
        base_commit=base_commit,
        vcon_schema_commit=VCON_SCHEMA_COMMIT,
        changed_path_count=len(actual_paths),
        targeted_command_count=len(commands),
        postgres_evidence_sha256=evidence_sha256,
    )
    return qualification, commands


def verify_beta_reporting(
    root: Path,
    version: str,
    beta_report_root: str | None,
    beta_exception_attestation: str | None,
    beta_carry_forward_attestation: str | None,
    log: ReleaseLog,
) -> dict[str, Any]:
    supplied = [
        value
        for value in (
            beta_report_root,
            beta_exception_attestation,
            beta_carry_forward_attestation,
        )
        if value
    ]
    if len(supplied) > 1:
        raise ReleaseError(
            "strict, exception, and carry-forward beta inputs are mutually exclusive"
        )
    if beta_carry_forward_attestation:
        attestation = Path(beta_carry_forward_attestation)
        if not attestation.is_absolute():
            attestation = root / attestation
        verifier = root / "scripts/release_carry_forward_attestation.py"
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
        payload = load_json_object(attestation, "carry-forward attestation")
        release = payload.get("release")
        inherited = payload.get("inherited_beta_background")
        current = payload.get("current_evidence")
        if not isinstance(release, dict) or not isinstance(inherited, dict):
            raise ReleaseError("carry-forward attestation lacks release metadata")
        if not isinstance(current, dict):
            raise ReleaseError("carry-forward attestation lacks current evidence")
        return {
            "mode": "owner-approved-carry-forward",
            "disposition": release.get("disposition"),
            "strict_automated_status": release.get("beta_suite"),
            "current_workspace_verification": "PASS",
            "inherited_beta_background": inherited,
            "current_canonical_2k": current.get("canonical_2k"),
            "attestation_path": relative_or_absolute(root, attestation),
            "attestation_sha256": hashlib.sha256(attestation.read_bytes()).hexdigest(),
        }
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
        return {
            "mode": "owner-approved-exception",
            "disposition": payload["release"]["disposition"],
            "strict_automated_status": payload["release"][
                "strict_automated_status"
            ],
            "attestation_path": relative_or_absolute(root, attestation),
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
    verification_scope: dict[str, Any],
) -> None:
    receipt = {
        "schema": VERIFICATION_RECEIPT_SCHEMA,
        "verified_at": utc_now(),
        "version": version,
        "git_commit": head,
        "package_count": len(ordered),
        "ordered_packages": ordered,
        "package_sha256": package_hashes,
        "package_file_manifest_sha256": package_file_hashes,
        "beta_qualification": beta_qualification,
        "verification_scope": verification_scope,
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


def canonical_json_sha256(value: Any) -> str:
    payload = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
    return hashlib.sha256(payload).hexdigest()


def verify_remote_qualification(
    root: Path,
    version: str,
    head: str,
    qualification_path: str,
    log: ReleaseLog,
) -> dict[str, Any]:
    path = Path(qualification_path)
    if not path.is_absolute():
        path = root / path
    if not path.is_file():
        raise ReleaseError(f"missing remote qualification aggregate: {path}")
    aggregate = load_json_object(path, "remote qualification aggregate")
    catalog_path = root / "scripts/release/gates.json"
    catalog = load_json_object(catalog_path, "release gate catalog")
    if catalog.get("schema") != REMOTE_GATE_CATALOG_SCHEMA:
        raise ReleaseError("release gate catalog has an unsupported schema")
    expected_gates = catalog.get("profiles", {}).get("remote-release")
    coverage = catalog.get("remote_release_legacy_coverage", {})
    accepted = aggregate.get("accepted_gates")
    accepted_ids = (
        [item.get("gate_id") for item in accepted]
        if isinstance(accepted, list)
        and all(isinstance(item, dict) for item in accepted)
        else []
    )
    expected_catalog_hash = canonical_json_sha256(catalog)
    valid = (
        aggregate.get("schema") == REMOTE_QUALIFICATION_SCHEMA
        and aggregate.get("status") == "PASS"
        and aggregate.get("failures") == []
        and aggregate.get("candidate_sha") == head
        and aggregate.get("profile") == "remote-release"
        and aggregate.get("catalog_sha256") == expected_catalog_hash
        and isinstance(expected_gates, list)
        and len(expected_gates) > 44
        and coverage.get("required_legacy_count") == 108
        and coverage.get("profile_legacy_count") == 108
        and coverage.get("unautomated_legacy_ids") == []
        and aggregate.get("gate_count") == len(expected_gates)
        and len(accepted_ids) == len(expected_gates)
        and len(set(accepted_ids)) == len(accepted_ids)
        and set(accepted_ids) == set(expected_gates)
        and aggregate.get("fresh_count", 0) >= 4
        and aggregate.get("fresh_count", 0) + aggregate.get("reused_count", 0)
        == len(expected_gates)
    )
    if not valid:
        raise ReleaseError(
            "remote qualification does not bind the complete remote-release "
            "profile to this release commit"
        )
    aggregate_sha256 = hashlib.sha256(path.read_bytes()).hexdigest()
    log.event(
        "remote-qualification",
        path=relative_or_absolute(root, path),
        sha256=aggregate_sha256,
        gate_count=len(expected_gates),
        fresh_count=aggregate["fresh_count"],
        reused_count=aggregate["reused_count"],
    )
    return {
        "mode": "remote-release",
        "disposition": "RELEASE-CANDIDATE",
        "strict_automated_status": "PASS",
        "version": version,
        "git_commit": head,
        "aggregate_path": relative_or_absolute(root, path),
        "aggregate_sha256": aggregate_sha256,
        "catalog_sha256": expected_catalog_hash,
        "gate_count": len(expected_gates),
        "fresh_count": aggregate["fresh_count"],
        "reused_count": aggregate["reused_count"],
    }


def verify(
    root: Path,
    version: str,
    beta_report_root: str | None,
    beta_exception_attestation: str | None,
    beta_carry_forward_attestation: str | None,
    targeted_delta_attestation: str | None,
    remote_qualification: str | None,
) -> None:
    head = ensure_release_state(root, version, require_no_tag=True)
    packages, ordered = validate_workspace(root, version, locked=True)
    log = ReleaseLog(root, version, "verify")
    log.event("start", operation="verify", version=version, git_commit=head)
    if targeted_delta_attestation and (
        beta_report_root
        or beta_exception_attestation
        or beta_carry_forward_attestation
    ):
        raise ReleaseError(
            "--targeted-delta-attestation is mutually exclusive with beta "
            "reporting and exception inputs"
        )

    workspace_check = [
        "cargo",
        "check",
        "--workspace",
        "--all-targets",
        "--locked",
    ]
    named_commands: list[tuple[str, list[str]]]
    if remote_qualification:
        beta_qualification = verify_remote_qualification(
            root, version, head, remote_qualification, log
        )
        named_commands = []
        verification_scope = {
            "mode": "remote-release",
            "workspace_manifest": "PASS",
            "workspace_compile": "PASS-REMOTE",
            "workspace_tests": "PASS-REMOTE",
            "workspace_doctests": "PASS-REMOTE",
            "beta_suite": "REPLACED-BY-REMOTE-GATES",
            "targeted_commands": [],
            "postgresql_evidence": None,
            "package_file_manifests": "PASS",
            "package_archives": "VERIFIED-WHEN-REGISTRY-RESOLVABLE",
        }
    elif targeted_delta_attestation:
        beta_qualification, named_commands = verify_targeted_delta_attestation(
            root,
            version,
            head,
            targeted_delta_attestation,
            log,
        )
        verification_scope = {
            "mode": "targeted-delta",
            "workspace_manifest": "PASS",
            "workspace_compile": "PASS",
            "workspace_tests": "NOT-RERUN",
            "workspace_doctests": "NOT-RERUN",
            "beta_suite": "NOT-RERUN",
            "targeted_commands": [
                {"name": name, "argv": argv, "exit_status": 0}
                for name, argv in named_commands
            ],
            "postgresql_evidence": beta_qualification["postgresql"],
            "package_file_manifests": "PASS",
            "package_archives": "VERIFIED-WHEN-REGISTRY-RESOLVABLE",
        }
    else:
        beta_qualification = verify_beta_reporting(
            root,
            version,
            beta_report_root,
            beta_exception_attestation,
            beta_carry_forward_attestation,
            log,
        )
        named_commands = [
            ("workspace-lib-tests", ["cargo", "test", "--workspace", "--lib", "--locked"]),
            (
                "workspace-target-tests",
                [
                    "cargo",
                    "test",
                    "--workspace",
                    "--bins",
                    "--examples",
                    "--tests",
                    "--locked",
                ],
            ),
            (
                "workspace-doctests",
                ["cargo", "test", "--workspace", "--doc", "--locked"],
            ),
        ]
        verification_scope = {
            "mode": "full",
            "workspace_manifest": "PASS",
            "workspace_compile": "PASS",
            "workspace_tests": "PASS",
            "workspace_doctests": "PASS",
            "beta_suite": (
                "PASS"
                if beta_qualification["mode"] == "strict"
                else (
                    "OWNER-APPROVED-CARRY-FORWARD"
                    if beta_qualification["mode"]
                    == "owner-approved-carry-forward"
                    else "OWNER-APPROVED-EXCEPTION"
                )
            ),
            "targeted_commands": [],
            "postgresql_evidence": None,
            "package_file_manifests": "PASS",
            "package_archives": "VERIFIED-WHEN-REGISTRY-RESOLVABLE",
        }

    log.command(workspace_check, root)
    for name, command in named_commands:
        log.message(f"== verification command: {name}")
        log.event("named-verification-command", name=name, argv=command)
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
        verification_scope,
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
    qualification = receipt.get("beta_qualification")
    scope = receipt.get("verification_scope")
    qualification_mode = (
        qualification.get("mode") if isinstance(qualification, dict) else None
    )
    scope_mode = scope.get("mode") if isinstance(scope, dict) else None
    common_scope = (
        isinstance(scope, dict)
        and scope.get("workspace_manifest") == "PASS"
        and scope.get("workspace_compile") in {"PASS", "PASS-REMOTE"}
        and scope.get("package_file_manifests") == "PASS"
        and scope.get("package_archives")
        == "VERIFIED-WHEN-REGISTRY-RESOLVABLE"
    )
    full_scope = (
        scope_mode == "full"
        and qualification_mode
        in {
            "strict",
            "owner-approved-exception",
            "owner-approved-carry-forward",
        }
        and scope.get("workspace_tests") == "PASS"
        and scope.get("workspace_doctests") == "PASS"
        and scope.get("beta_suite")
        in {
            "PASS",
            "OWNER-APPROVED-EXCEPTION",
            "OWNER-APPROVED-CARRY-FORWARD",
        }
        and scope.get("targeted_commands") == []
        and scope.get("postgresql_evidence") is None
    ) if isinstance(scope, dict) else False
    carry_forward_qualification = (
        qualification_mode == "owner-approved-carry-forward"
        and qualification.get("disposition")
        == "OWNER-APPROVED-CARRY-FORWARD"
        and qualification.get("strict_automated_status") == "NOT-RERUN"
        and qualification.get("current_workspace_verification") == "PASS"
        and isinstance(qualification.get("inherited_beta_background"), dict)
        and qualification["inherited_beta_background"].get("version") == "0.3.2"
        and qualification["inherited_beta_background"].get("disposition")
        == "APPROVED-WITH-EXCEPTION"
        and qualification["inherited_beta_background"].get(
            "strict_automated_status"
        )
        == "NON-RC"
        and isinstance(qualification.get("current_canonical_2k"), dict)
        and qualification["current_canonical_2k"].get("status") == "PASS"
        and isinstance(qualification.get("attestation_sha256"), str)
        and re.fullmatch(
            r"[0-9a-f]{64}", qualification["attestation_sha256"]
        )
        is not None
    ) if isinstance(qualification, dict) else False
    carry_forward_attestation_current = True
    if qualification_mode == "owner-approved-carry-forward":
        carry_forward_attestation_current = False
        attestation_value = qualification.get("attestation_path")
        if isinstance(attestation_value, str) and attestation_value:
            attestation = Path(attestation_value)
            if not attestation.is_absolute():
                attestation = root / attestation
            if (
                attestation.is_file()
                and hashlib.sha256(attestation.read_bytes()).hexdigest()
                == qualification.get("attestation_sha256")
            ):
                verifier = root / "scripts/release_carry_forward_attestation.py"
                verified = run(
                    [
                        sys.executable,
                        str(verifier),
                        "verify",
                        "--attestation",
                        str(attestation),
                        "--version",
                        version,
                    ],
                    cwd=root,
                    check=False,
                )
                carry_forward_attestation_current = verified.returncode == 0
    targeted_commands = (
        scope.get("targeted_commands") if isinstance(scope, dict) else None
    )
    targeted_command_shape = (
        isinstance(targeted_commands, list)
        and targeted_commands
        == [
            {"name": name, "argv": list(argv), "exit_status": 0}
            for name, argv in TARGETED_DELTA_COMMANDS
        ]
    )
    postgres_evidence = (
        scope.get("postgresql_evidence") if isinstance(scope, dict) else None
    )
    targeted_scope = (
        scope_mode == "targeted-delta"
        and qualification_mode == "targeted-delta"
        and qualification.get("disposition") == "APPROVED-TARGETED-DELTA"
        and qualification.get("strict_automated_status") == "NOT-RERUN"
        and qualification.get("workspace_test_status") == "TARGETED-ONLY"
        and isinstance(qualification.get("base_commit"), str)
        and COMMIT_SHA.fullmatch(qualification["base_commit"]) is not None
        and qualification.get("vcon_schema_commit") == VCON_SCHEMA_COMMIT
        and isinstance(qualification.get("changed_paths"), list)
        and qualification["changed_paths"]
        == sorted(set(qualification["changed_paths"]))
        and all(
            isinstance(path, str) and targeted_delta_path_allowed(path)
            for path in qualification["changed_paths"]
        )
        and isinstance(qualification.get("changed_path_count"), int)
        and qualification.get("changed_path_count")
        == len(qualification["changed_paths"])
        and qualification.get("changed_path_count", 0) > 0
        and qualification.get("targeted_command_count")
        == len(TARGETED_DELTA_COMMANDS)
        and isinstance(qualification.get("attestation_sha256"), str)
        and re.fullmatch(
            r"[0-9a-f]{64}", qualification["attestation_sha256"]
        )
        is not None
        and scope.get("workspace_tests") == "NOT-RERUN"
        and scope.get("workspace_doctests") == "NOT-RERUN"
        and scope.get("beta_suite") == "NOT-RERUN"
        and targeted_command_shape
        and isinstance(postgres_evidence, dict)
        and postgres_evidence.get("status") == "ATTESTED-PASS"
        and postgres_evidence.get("live_database") is True
        and postgres_evidence.get("ephemeral_database") is True
        and isinstance(postgres_evidence.get("server_version"), str)
        and bool(postgres_evidence["server_version"].strip())
        and isinstance(postgres_evidence.get("environment"), dict)
        and set(postgres_evidence["environment"])
        == {"provider", "image", "database", "run_id"}
        and all(
            isinstance(postgres_evidence["environment"].get(field), str)
            and bool(postgres_evidence["environment"][field].strip())
            for field in ("provider", "image", "database", "run_id")
        )
        and isinstance(postgres_evidence.get("recorded_at"), str)
        and postgres_evidence.get("command") == list(TARGETED_POSTGRES_COMMAND)
        and isinstance(postgres_evidence.get("evidence_sha256"), str)
        and re.fullmatch(
            r"[0-9a-f]{64}", postgres_evidence["evidence_sha256"]
        )
        is not None
        and qualification.get("postgresql") == postgres_evidence
    ) if isinstance(scope, dict) and isinstance(qualification, dict) else False
    remote_aggregate_current = False
    if qualification_mode == "remote-release" and isinstance(qualification, dict):
        aggregate_value = qualification.get("aggregate_path")
        if isinstance(aggregate_value, str) and aggregate_value:
            aggregate_path = Path(aggregate_value)
            if not aggregate_path.is_absolute():
                aggregate_path = root / aggregate_path
            remote_aggregate_current = (
                aggregate_path.is_file()
                and hashlib.sha256(aggregate_path.read_bytes()).hexdigest()
                == qualification.get("aggregate_sha256")
            )
    remote_scope = (
        scope_mode == "remote-release"
        and qualification_mode == "remote-release"
        and qualification.get("disposition") == "RELEASE-CANDIDATE"
        and qualification.get("strict_automated_status") == "PASS"
        and qualification.get("git_commit") == head
        and isinstance(qualification.get("gate_count"), int)
        and qualification.get("gate_count", 0) > 44
        and qualification.get("fresh_count", 0) >= 4
        and qualification.get("fresh_count", 0)
        + qualification.get("reused_count", 0)
        == qualification.get("gate_count")
        and remote_aggregate_current
        and scope.get("workspace_tests") == "PASS-REMOTE"
        and scope.get("workspace_doctests") == "PASS-REMOTE"
        and scope.get("beta_suite") == "REPLACED-BY-REMOTE-GATES"
        and scope.get("targeted_commands") == []
        and scope.get("postgresql_evidence") is None
    ) if isinstance(scope, dict) and isinstance(qualification, dict) else False
    expected = (
        receipt.get("schema") == VERIFICATION_RECEIPT_SCHEMA
        and receipt.get("version") == version
        and receipt.get("git_commit") == head
        and receipt.get("ordered_packages") == ordered
        and receipt.get("package_count") == len(ordered)
        and isinstance(file_hashes, dict)
        and set(file_hashes) == set(ordered)
        and isinstance(package_hashes, dict)
        and set(package_hashes) <= set(ordered)
        and common_scope
        and (full_scope or targeted_scope or remote_scope)
        and (
            qualification_mode != "owner-approved-carry-forward"
            or (
                carry_forward_qualification
                and carry_forward_attestation_current
            )
        )
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
            report_group.add_argument(
                "--beta-carry-forward-attestation",
                default=os.environ.get("RVOIP_BETA_CARRY_FORWARD_ATTESTATION"),
            )
            report_group.add_argument(
                "--targeted-delta-attestation",
                default=os.environ.get("RVOIP_TARGETED_DELTA_ATTESTATION"),
            )
            report_group.add_argument(
                "--remote-qualification",
                default=os.environ.get("RVOIP_REMOTE_QUALIFICATION"),
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
                    args.beta_carry_forward_attestation,
                    args.targeted_delta_attestation,
                    args.remote_qualification,
                )
            elif args.command == "publish":
                publish(root, version, execute=args.execute)
    except ReleaseError as error:
        print(f"release: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
