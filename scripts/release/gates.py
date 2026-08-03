#!/usr/bin/env python3
"""Plan, execute, reuse, and reconcile version-neutral release gates."""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
from typing import Any, Iterable


CATALOG_SCHEMA = "rvoip-release-gate-catalog-v1"
PLAN_SCHEMA = "rvoip-release-gate-plan-v1"
RECEIPT_SCHEMA = "rvoip-release-gate-receipt-v1"
AGGREGATE_SCHEMA = "rvoip-release-qualification-v1"
DEFAULT_CATALOG = Path("scripts/release/gates.json")
MAX_DIAGNOSTIC_GATES = 20
COMMIT_SHA = re.compile(r"^[0-9a-f]{40}$")


class GateError(RuntimeError):
    """A release-gate invariant failed closed."""


def root_dir() -> Path:
    return Path(__file__).resolve().parents[2]


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def file_sha256(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def run(
    argv: list[str],
    *,
    root: Path,
    check: bool = True,
    capture: bool = True,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        argv,
        cwd=root,
        text=True,
        capture_output=capture,
        check=False,
    )
    if check and completed.returncode:
        detail = (completed.stdout or "") + (completed.stderr or "")
        raise GateError(f"command failed ({completed.returncode}): {' '.join(argv)}\n{detail.strip()}")
    return completed


def git(root: Path, *args: str) -> str:
    return run(["git", *args], root=root).stdout.strip()


def load_json(path: Path, description: str) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise GateError(f"cannot read {description} {path}: {error}") from error
    if not isinstance(payload, dict):
        raise GateError(f"{description} must be a JSON object: {path}")
    return payload


def load_catalog(root: Path, path: Path) -> dict[str, Any]:
    resolved = path if path.is_absolute() else root / path
    catalog = load_json(resolved, "gate catalog")
    validate_catalog(root, catalog)
    return catalog


def gate_map(catalog: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {gate["id"]: gate for gate in catalog["gates"]}


def validate_catalog(root: Path, catalog: dict[str, Any]) -> None:
    if catalog.get("schema") != CATALOG_SCHEMA:
        raise GateError("unsupported release gate catalog schema")
    gates = catalog.get("gates")
    if not isinstance(gates, list) or not gates:
        raise GateError("gate catalog must contain gates")
    ids = [gate.get("id") for gate in gates]
    if any(not isinstance(gate_id, str) or not gate_id for gate_id in ids):
        raise GateError("every gate requires a non-empty string id")
    if len(set(ids)) != len(ids):
        duplicates = sorted(gate_id for gate_id, count in collections.Counter(ids).items() if count > 1)
        raise GateError(f"duplicate gate ids: {duplicates}")
    by_id = gate_map(catalog)
    for gate in gates:
        if gate.get("executor") not in {"argv", "builtin", "legacy-group", "aggregate"}:
            raise GateError(f"gate {gate['id']} has unsupported executor")
        if gate["executor"] == "argv" and not isinstance(gate.get("command"), list):
            raise GateError(f"gate {gate['id']} lacks argv command")
        missing = sorted(set(gate.get("dependencies", [])) - set(by_id))
        if missing:
            raise GateError(f"gate {gate['id']} has unknown dependencies: {missing}")
        if not isinstance(gate.get("affected_paths"), list):
            raise GateError(f"gate {gate['id']} lacks affected_paths")
    for profile, selected in catalog.get("profiles", {}).items():
        if not isinstance(selected, list) or not selected:
            raise GateError(f"profile {profile} must select gates")
        missing = sorted(set(selected) - set(by_id))
        if missing:
            raise GateError(f"profile {profile} references unknown gates: {missing}")
        selected_ids = set(selected)
        dependency_gaps = {
            gate_id: sorted(set(by_id[gate_id].get("dependencies", [])) - selected_ids)
            for gate_id in selected
            if set(by_id[gate_id].get("dependencies", [])) - selected_ids
        }
        if dependency_gaps:
            raise GateError(
                f"profile {profile} omits gate dependencies: {dependency_gaps}"
            )
    coverage = catalog.get("remote_release_legacy_coverage")
    if not isinstance(coverage, dict):
        raise GateError("catalog must declare remote-release legacy coverage")
    unautomated = coverage.get("unautomated_legacy_ids")
    if not isinstance(unautomated, list) or any(
        not isinstance(gate_id, str) for gate_id in unautomated
    ):
        raise GateError("remote-release legacy coverage must list gate IDs")

    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(gate_id: str) -> None:
        if gate_id in visiting:
            raise GateError(f"gate dependency cycle includes {gate_id}")
        if gate_id in visited:
            return
        visiting.add(gate_id)
        for dependency in by_id[gate_id].get("dependencies", []):
            visit(dependency)
        visiting.remove(gate_id)
        visited.add(gate_id)

    for gate_id in ids:
        visit(gate_id)

    legacy = [gate for gate in gates if gate.get("legacy")]
    source_path = root / catalog["legacy_source"]["path"]
    source = load_json(source_path, "canonical legacy gate source")
    source_ids = [record["id"] for record in source.get("records", [])]
    legacy_ids = [
        gate["id"]
        for gate in sorted(legacy, key=lambda item: item["legacy"]["sequence"])
    ]
    if len(source_ids) != 108 or legacy_ids != source_ids:
        raise GateError("catalog must map every canonical 108-gate record exactly once")
    remote_legacy_ids = [
        gate_id for gate_id in catalog["profiles"].get("remote-release", []) if gate_id in legacy_ids
    ]
    coverage_ids = catalog["remote_release_legacy_coverage"].get("unautomated_legacy_ids", [])
    if (
        catalog["remote_release_legacy_coverage"].get("required_legacy_count") != len(source_ids)
        or len(remote_legacy_ids) != catalog["remote_release_legacy_coverage"].get("profile_legacy_count")
        or set(remote_legacy_ids) & set(coverage_ids)
        or set(remote_legacy_ids) | set(coverage_ids) != set(source_ids)
    ):
        raise GateError("remote-release legacy coverage ledger is inconsistent")
    core = [gate for gate in gates if gate["id"].startswith("core.")]
    if len(core) != catalog.get("workspace_package_count") or len(core) != 44:
        raise GateError("catalog must contain one core gate for each of 44 workspace crates")


def tracked_files(root: Path) -> list[str]:
    payload = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, capture_output=True, check=True
    ).stdout
    return sorted(
        decoded
        for path in payload.split(b"\0")
        if path
        for decoded in [path.decode()]
        if (root / decoded).is_file()
    )


def workspace_graph(root: Path) -> tuple[dict[str, str], dict[str, set[str]]]:
    metadata = json.loads(
        run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            root=root,
        ).stdout
    )
    members = set(metadata["workspace_members"])
    packages = [package for package in metadata["packages"] if package["id"] in members]
    names = {package["name"] for package in packages}
    roots = {
        package["name"]: Path(package["manifest_path"])
        .resolve()
        .parent.relative_to(root.resolve())
        .as_posix()
        for package in packages
    }
    dependencies = {
        package["name"]: {
            dependency.get("package", dependency["name"])
            for dependency in package.get("dependencies", [])
            if dependency.get("package", dependency["name"]) in names
        }
        for package in packages
    }
    return roots, dependencies


def dependency_closure(selected: Iterable[str], dependencies: dict[str, set[str]]) -> set[str]:
    result = set(selected)
    pending = list(result)
    while pending:
        package = pending.pop()
        for dependency in dependencies.get(package, set()):
            if dependency not in result:
                result.add(dependency)
                pending.append(dependency)
    return result


def definition_digest(gate: dict[str, Any]) -> str:
    return sha256_bytes(canonical_bytes(gate))


def environment_digest(environment_id: str) -> str:
    return sha256_bytes(environment_id.encode())


def gate_environment_id(environment_id: str, gate: dict[str, Any]) -> str:
    return f"{environment_id}|{gate['resource_class']}"


def matches(path: str, patterns: Iterable[str]) -> bool:
    return any(pattern == "**" or fnmatch.fnmatchcase(path, pattern) for pattern in patterns)


def input_record(
    *,
    root: Path,
    gate: dict[str, Any],
    environment_id: str,
    files: list[str],
    package_roots: dict[str, str],
    package_dependencies: dict[str, set[str]],
    gate_definitions: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    patterns = set(gate.get("affected_paths", []))
    for package in dependency_closure(gate.get("affected_crates", []), package_dependencies):
        if package in package_roots:
            patterns.add(f"{package_roots[package]}/**")
    patterns.update(
        {
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "scripts/release/gates.py",
            "scripts/ci/**",
            ".config/**",
            ".github/workflows/**",
            "deny.toml",
        }
    )
    if gate["resource_class"].startswith("gcp-"):
        patterns.add("infra/release-runners/gcp-release-startup.sh")
    selected = [path for path in files if matches(path, patterns)]
    file_hashes = {path: file_sha256(root / path) for path in selected}
    payload = {
        "gate_definition_sha256": definition_digest(gate),
        "environment_sha256": environment_digest(gate_environment_id(environment_id, gate)),
        "files": file_hashes,
        "dependency_definition_sha256": {
            dependency: definition_digest(gate_definitions[dependency])
            for dependency in sorted(gate.get("dependencies", []))
        },
    }
    return {**payload, "input_sha256": sha256_bytes(canonical_bytes(payload))}


def all_input_records(
    root: Path,
    catalog: dict[str, Any],
    selected: list[str],
    environment_id: str,
) -> dict[str, dict[str, Any]]:
    files = tracked_files(root)
    roots, dependencies = workspace_graph(root)
    by_id = gate_map(catalog)
    return {
        gate_id: input_record(
            root=root,
            gate=by_id[gate_id],
            environment_id=environment_id,
            files=files,
            package_roots=roots,
            package_dependencies=dependencies,
            gate_definitions=by_id,
        )
        for gate_id in selected
    }


def load_prior_receipts(path: Path | None) -> dict[str, list[dict[str, Any]]]:
    result: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    if not path or not path.exists():
        return result
    for receipt_path in sorted(path.rglob("receipt.json")):
        try:
            receipt = json.loads(receipt_path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if receipt.get("schema") != RECEIPT_SCHEMA or not isinstance(receipt.get("gate_id"), str):
            continue
        receipt["_path"] = receipt_path.as_posix()
        receipt["_sha256"] = file_sha256(receipt_path)
        result[receipt["gate_id"]].append(receipt)
    return result


def exact_reusable(
    receipts: list[dict[str, Any]],
    gate: dict[str, Any],
    inputs: dict[str, Any],
    environment_id: str,
) -> dict[str, Any] | None:
    if gate.get("always_fresh"):
        return None
    for receipt in reversed(receipts):
        if (
            receipt.get("status") == "PASS"
            and receipt.get("gate_definition_sha256") == inputs["gate_definition_sha256"]
            and receipt.get("input_sha256") == inputs["input_sha256"]
            and receipt.get("environment_sha256") == inputs["environment_sha256"]
        ):
            return receipt
    return None


def changed_files(root: Path, base: str, candidate: str) -> list[str]:
    output = git(root, "diff", "--name-only", "--find-renames", f"{base}...{candidate}")
    return sorted(set(filter(None, output.splitlines())))


def unknown_change(
    paths: list[str], selected_gates: list[dict[str, Any]]
) -> list[str]:
    meaningful = [
        gate
        for gate in selected_gates
        if not gate.get("always_fresh") and gate.get("kind") not in {"source", "reporting"}
    ]
    return [
        path
        for path in paths
        if not any(matches(path, gate.get("affected_paths", [])) for gate in meaningful)
    ]


def reverse_gate_dependencies(selected: list[str], by_id: dict[str, dict[str, Any]]) -> dict[str, set[str]]:
    reverse = {gate_id: set() for gate_id in selected}
    selected_set = set(selected)
    for gate_id in selected:
        for dependency in by_id[gate_id].get("dependencies", []):
            if dependency in selected_set:
                reverse[dependency].add(gate_id)
    return reverse


def dependent_closure(failed: set[str], reverse: dict[str, set[str]]) -> set[str]:
    result = set(failed)
    pending = list(failed)
    while pending:
        gate_id = pending.pop()
        for dependent in reverse.get(gate_id, set()):
            if dependent not in result:
                result.add(dependent)
                pending.append(dependent)
    return result


def balance_gates(gates: list[dict[str, Any]], count: int) -> list[list[dict[str, Any]]]:
    count = min(max(1, count), len(gates))
    shards: list[tuple[int, list[dict[str, Any]]]] = [(0, []) for _ in range(count)]
    for gate in sorted(gates, key=lambda item: (-int(item.get("estimated_seconds", 1)), item["id"])):
        index = min(range(count), key=lambda item: (shards[item][0], item))
        weight, items = shards[index]
        items.append(gate)
        shards[index] = (weight + int(gate.get("estimated_seconds", 1)), items)
    return [items for _, items in shards]


def matrix_for(plan_gates: list[dict[str, Any]], by_id: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    runnable = [
        by_id[item["id"]]
        for item in plan_gates
        if item["decision"] == "RUN" and by_id[item["id"]]["executor"] != "aggregate"
    ]
    groups: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    for gate in runnable:
        groups[gate["resource_class"]].append(gate)
    matrix = []
    limits = {
        # Keep the complete hosted release fanout below GitHub's 20-job
        # repository limit while avoiding the two 50+ minute serial shards
        # observed during the v0.3.6 qualification. Twelve standard shards,
        # five nightly shards, one evidence shard, and the single GCP
        # controller peak at nineteen concurrent jobs.
        "github-standard": 12,
        "github-nightly": 5,
        "github-evidence": 1,
        "gcp-performance": 6,
        "gcp-performance-soak": 7,
        "gcp-performance-soak-long": 2,
        # The twelve proxy rows have independent ephemeral peer labs. Two
        # workers fit the complete release fanout inside the 100-vCPU regional
        # quota, while a failed row can still be retried alone.
        "gcp-proxy-interop": 2,
        # Interoperability gates share one stateful peer lab. Keep their
        # lifecycle dependency chain in one shard so start/matrix/stop/restore
        # operations cannot race across ephemeral jobs.
        "gcp-interop": 1,
    }
    for resource in sorted(groups):
        gates = groups[resource]
        shards = balance_gates(gates, min(limits.get(resource, 1), len(gates)))
        for index, shard in enumerate(shards, start=1):
            gate_ids = sorted(gate["id"] for gate in shard)
            if resource.startswith("gcp-"):
                runs_on: str | list[str] = ["self-hosted", "rvoip-release", resource]
            else:
                runs_on = "ubuntu-latest"
            matrix.append(
                {
                    "id": re.sub(r"[^A-Za-z0-9_-]", "-", f"{resource}-{index}"),
                    "resource_class": resource,
                    "runs_on": runs_on,
                    "hosted": not resource.startswith("gcp-"),
                    "machine_type": (
                        "n2-standard-2"
                        if resource == "gcp-proxy-interop"
                        else "n2-standard-4"
                        if resource in {"gcp-interop", "gcp-performance-soak"}
                        else "n2-standard-8"
                    ),
                    "disk_type": "pd-standard",
                    "disk_size_gb": 100 if resource == "gcp-proxy-interop" else 200,
                    "gates": gate_ids,
                    "gates_csv": ",".join(gate_ids),
                    "needs_nightly": any(
                        gate["id"].startswith(("security.remote-fuzz", "security.fuzz-"))
                        for gate in shard
                    ),
                    "needs_cargo_deny": any(
                        gate["id"] in {"security.remote-advisories", "security.advisory-audit"}
                        for gate in shard
                    ),
                    "needs_chromium": any(
                        gate["id"] == "interop.browser-dtmf" for gate in shard
                    ),
                    "estimated_seconds": sum(int(gate.get("estimated_seconds", 1)) for gate in shard),
                }
            )
    return matrix


def profile_selection(
    catalog: dict[str, Any], profile: str, only_gates: list[str] | None = None
) -> list[str]:
    profiles = catalog["profiles"]
    if profile != "remote-diagnostic":
        if profile not in profiles:
            raise GateError(f"unknown release profile {profile!r}")
        if only_gates:
            raise GateError("--only-gates is allowed only with remote-diagnostic")
        return profiles[profile]

    requested = sorted(set(only_gates or []))
    if not requested:
        raise GateError("remote-diagnostic requires at least one --only-gates ID")
    if len(requested) > MAX_DIAGNOSTIC_GATES:
        raise GateError(
            f"remote-diagnostic accepts at most {MAX_DIAGNOSTIC_GATES} requested gates"
        )

    diagnostic_ids = set(profiles["remote-release"]) | set(profiles["remote-preflight"])
    by_id = gate_map(catalog)
    unknown = sorted(set(requested) - diagnostic_ids)
    if unknown:
        raise GateError(
            "remote-diagnostic gate IDs must belong to remote-release or remote-preflight: "
            + ", ".join(unknown)
        )
    invalid = sorted(
        gate_id
        for gate_id in requested
        if gate_id != "interop.remote-proxies"
        and (
            not by_id[gate_id]["resource_class"].startswith("gcp-")
            or by_id[gate_id]["executor"] != "argv"
        )
    )
    if invalid:
        raise GateError(
            "remote-diagnostic accepts executable GCP gates or the proxy-matrix aggregate only: "
            + ", ".join(invalid)
        )

    selected = set(requested)
    pending = list(requested)
    while pending:
        gate_id = pending.pop()
        for dependency in by_id[gate_id].get("dependencies", []):
            if dependency not in diagnostic_ids:
                raise GateError(
                    f"remote-diagnostic dependency {dependency!r} is outside the release and preflight profiles"
                )
            if dependency not in selected:
                selected.add(dependency)
                pending.append(dependency)
    return sorted(selected)


def create_plan(
    *,
    root: Path,
    catalog: dict[str, Any],
    profile: str,
    candidate: str,
    environment_id: str,
    prior_path: Path | None,
    first_candidate: bool,
    changed_since: str | None,
    only_gates: list[str] | None = None,
) -> dict[str, Any]:
    selected = profile_selection(catalog, profile, only_gates)
    if profile == "remote-release":
        missing = catalog.get("remote_release_legacy_coverage", {}).get(
            "unautomated_legacy_ids", []
        )
        if missing:
            raise GateError(
                "remote-release is fail-closed until every legacy gate has a "
                "structured replacement: " + ", ".join(missing)
            )
    head = git(root, "rev-parse", "HEAD")
    resolved_candidate = git(root, "rev-parse", f"{candidate}^{{commit}}")
    if head != resolved_candidate:
        raise GateError(f"checkout HEAD {head} does not match candidate {resolved_candidate}")
    by_id = gate_map(catalog)
    inputs = all_input_records(root, catalog, selected, environment_id)
    prior = load_prior_receipts(prior_path)
    changed = changed_files(root, changed_since, resolved_candidate) if changed_since else []
    unknown = unknown_change(changed, [by_id[gate_id] for gate_id in selected]) if changed else []
    previous_failures = {
        gate_id
        for gate_id, receipts in prior.items()
        if gate_id in set(selected) and receipts and receipts[-1].get("status") != "PASS"
    }
    reverse_dependencies = reverse_gate_dependencies(selected, by_id)
    invalidated_by_failure = dependent_closure(previous_failures, reverse_dependencies)
    changed_gate_ids = {
        gate_id
        for gate_id in selected
        if any(matches(path, by_id[gate_id].get("affected_paths", [])) for path in changed)
    }
    invalidated_by_change = dependent_closure(changed_gate_ids, reverse_dependencies)
    reusable_receipts = {
        gate_id: exact_reusable(
            prior.get(gate_id, []), by_id[gate_id], inputs[gate_id], environment_id
        )
        for gate_id in selected
    }
    # A definition, environment, or selected-input mismatch must also rerun
    # every downstream gate that consumes the changed gate. Always-fresh
    # source/report gates are intentionally excluded: regenerating their
    # receipts does not make otherwise exact test evidence stale.
    input_misses = {
        gate_id
        for gate_id in selected
        if not by_id[gate_id].get("always_fresh")
        and reusable_receipts[gate_id] is None
    }
    invalidated_by_input = dependent_closure(input_misses, reverse_dependencies)
    decisions = []
    force_all = first_candidate or bool(unknown)
    for gate_id in selected:
        gate = by_id[gate_id]
        reuse = reusable_receipts[gate_id]
        if (
            force_all
            or gate.get("always_fresh")
            or gate_id in invalidated_by_failure
            or gate_id in invalidated_by_change
            or gate_id in invalidated_by_input
            or not reuse
        ):
            reason = "first candidate" if first_candidate else "input, failure, change, or freshness policy requires execution"
            if unknown:
                reason = "unmapped change fails closed to full execution"
            decision = {
                "id": gate_id,
                "decision": "RUN",
                "reason": reason,
                "executor": gate["executor"],
                "dependencies": gate.get("dependencies", []),
                **{key: inputs[gate_id][key] for key in ("gate_definition_sha256", "environment_sha256", "input_sha256")},
            }
        else:
            decision = {
                "id": gate_id,
                "decision": "REUSE",
                "reason": "exact definition, input, and environment digest match",
                "executor": gate["executor"],
                "dependencies": gate.get("dependencies", []),
                **{key: inputs[gate_id][key] for key in ("gate_definition_sha256", "environment_sha256", "input_sha256")},
                "reuse_receipt": {key: value for key, value in reuse.items() if not key.startswith("_")},
                "reuse_receipt_sha256": reuse["_sha256"],
            }
        decisions.append(decision)
    matrix = matrix_for(decisions, by_id)
    return {
        "schema": PLAN_SCHEMA,
        "created_at": utc_now(),
        "candidate_sha": resolved_candidate,
        "profile": profile,
        # This is the base environment identifier.  Each receipt additionally
        # carries a resource-class-specific environment digest.
        "environment_id": environment_id,
        "environment_sha256": environment_digest(environment_id),
        "catalog_sha256": sha256_bytes(canonical_bytes(catalog)),
        "changed_since": changed_since,
        "changed_files": changed,
        "unmapped_changed_files": unknown,
        "first_candidate": first_candidate,
        "gates": decisions,
        "matrix": matrix,
    }


def write_github_output(path: Path, plan: dict[str, Any]) -> None:
    hosted = [item for item in plan["matrix"] if item["hosted"]]
    gcp = [item for item in plan["matrix"] if not item["hosted"]]
    values = {
        "matrix": json.dumps({"include": plan["matrix"]}, separators=(",", ":")),
        "hosted_matrix": json.dumps({"include": hosted}, separators=(",", ":")),
        "gcp_matrix": json.dumps({"include": gcp}, separators=(",", ":")),
        "shard_count": str(len(plan["matrix"])),
        "hosted_shard_count": str(len(hosted)),
        "gcp_shard_count": str(len(gcp)),
        "run_count": str(sum(item["decision"] == "RUN" for item in plan["gates"])),
        "reuse_count": str(sum(item["decision"] == "REUSE" for item in plan["gates"])),
        "candidate_sha": plan["candidate_sha"],
        "environment_id": plan["environment_id"],
    }
    with path.open("a") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def expand_command(
    command: list[str], root: Path, artifact: Path, candidate: str
) -> list[str]:
    return [
        value.replace("{workspace}", str(root))
        .replace("{artifact_dir}", str(artifact))
        .replace("{candidate}", candidate)
        for value in command
    ]


def prebuilt_performance_command(
    *,
    gate: dict[str, Any],
    root: Path,
    artifact: Path,
    candidate: str,
    environment_id: str,
) -> list[str] | None:
    """Route direct Cargo performance gates through an attested binary bundle."""

    manifest = os.environ.get("RVOIP_PERF_PREBUILT_MANIFEST")
    if not manifest or not str(gate.get("resource_class", "")).startswith(
        "gcp-performance"
    ):
        return None
    command = gate.get("command") or []
    try:
        cargo_index = command.index("cargo")
    except ValueError:
        # Shell orchestrators resolve their individual binaries themselves.
        return None
    if cargo_index + 1 >= len(command) or command[cargo_index + 1] != "test":
        raise GateError(
            f"prebuilt performance gate {gate['id']} has an unsupported Cargo command"
        )
    return [
        "python3",
        str(root / "scripts/release/prebuilt_performance.py"),
        "run-gate",
        "--manifest",
        manifest,
        "--catalog",
        str(root / DEFAULT_CATALOG),
        "--gate-id",
        gate["id"],
        "--workspace",
        str(root),
        "--artifact-dir",
        str(artifact),
        "--candidate",
        candidate,
        "--environment-id",
        environment_id,
    ]


def dependency_order(gate_ids: list[str], by_id: dict[str, dict[str, Any]]) -> list[str]:
    """Return a stable topological order for the gates in one shard."""
    selected = set(gate_ids)
    indegree = {gate_id: 0 for gate_id in selected}
    dependents: dict[str, set[str]] = {gate_id: set() for gate_id in selected}
    for gate_id in selected:
        for dependency in by_id[gate_id].get("dependencies", []):
            if dependency in selected:
                indegree[gate_id] += 1
                dependents[dependency].add(gate_id)
    ready = sorted(gate_id for gate_id, degree in indegree.items() if degree == 0)
    ordered: list[str] = []
    while ready:
        gate_id = ready.pop(0)
        ordered.append(gate_id)
        for dependent in sorted(dependents[gate_id]):
            indegree[dependent] -= 1
            if indegree[dependent] == 0:
                ready.append(dependent)
        ready.sort()
    if len(ordered) != len(selected):
        raise GateError("gate shard contains a dependency cycle")
    return ordered


def clean_source(root: Path) -> tuple[int, str]:
    status = run(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"],
        root=root,
        check=False,
    )
    if status.returncode or status.stdout.strip():
        return 1, "source tree is not clean"
    return 0, json.dumps(
        {
            "commit": git(root, "rev-parse", "HEAD"),
            "tree": git(root, "rev-parse", "HEAD^{tree}"),
        },
        sort_keys=True,
    )


def run_to_log(
    argv: list[str],
    *,
    cwd: Path,
    log_path: Path,
    timeout_seconds: int,
) -> int:
    """Run one gate command with its catalogued timeout and capture output."""
    try:
        with log_path.open("a") as handle:
            completed = subprocess.run(
                argv,
                cwd=cwd,
                stdout=handle,
                stderr=subprocess.STDOUT,
                timeout=timeout_seconds,
                check=False,
            )
        return completed.returncode
    except subprocess.TimeoutExpired:
        with log_path.open("a") as handle:
            handle.write(
                f"\ncommand exceeded catalogued timeout of {timeout_seconds} seconds\n"
            )
        return 124
    except OSError as error:
        with log_path.open("a") as handle:
            handle.write(f"\ncommand could not start: {error}\n")
        return 127


def execute_gate(
    *,
    root: Path,
    gate: dict[str, Any],
    candidate: str,
    environment_id: str,
    inputs: dict[str, Any],
    artifact_root: Path,
    allow_legacy_groups: bool,
) -> dict[str, Any]:
    artifact = artifact_root / re.sub(r"[^A-Za-z0-9_.-]", "-", gate["id"])
    artifact.mkdir(parents=True, exist_ok=True)
    log_path = artifact / "command.log"
    started = utc_now()
    start = time.monotonic()
    attempts = []
    max_attempts = 1 + int(gate.get("max_infrastructure_retries", 0))
    timeout_seconds = max(1, int(gate.get("timeout_minutes", 60))) * 60
    final_code = 1
    for attempt in range(1, max_attempts + 1):
        attempt_start = time.monotonic()
        if gate["executor"] == "aggregate":
            raise GateError(
                f"aggregate gate {gate['id']} must run during evidence collection"
            )
        if gate["executor"] == "builtin":
            final_code, output = clean_source(root)
            with log_path.open("a") as handle:
                handle.write(output + "\n")
            argv = ["builtin", gate["id"]]
        elif gate["executor"] == "legacy-group":
            argv = ["legacy-group", gate["legacy"]["mode"]]
            if allow_legacy_groups:
                argv = [
                    "bash",
                    "crates/sip/rvoip-sip/scripts/beta_gate.sh",
                    f"--{gate['legacy']['mode']}",
                    "--require-external",
                ]
                final_code = run_to_log(
                    argv,
                    cwd=root,
                    log_path=log_path,
                    timeout_seconds=timeout_seconds,
                )
            else:
                with log_path.open("a") as handle:
                    handle.write(
                        "legacy compatibility gate is not part of the remote profile; "
                        "pass --allow-legacy-groups only for a compatibility run\n"
                    )
                final_code = 2
        else:
            argv = prebuilt_performance_command(
                gate=gate,
                root=root,
                artifact=artifact,
                candidate=candidate,
                environment_id=environment_id,
            ) or expand_command(gate["command"], root, artifact, candidate)
            final_code = run_to_log(
                argv,
                cwd=root / gate.get("working_directory", "."),
                log_path=log_path,
                timeout_seconds=timeout_seconds,
            )
        attempts.append(
            {
                "attempt": attempt,
                "argv": argv,
                "exit_code": final_code,
                "duration_seconds": round(time.monotonic() - attempt_start, 3),
            }
        )
        if final_code not in gate.get("retry_on_exit_codes", []) or attempt == max_attempts:
            break

    receipt = {
        "schema": RECEIPT_SCHEMA,
        "gate_id": gate["id"],
        "candidate_sha": candidate,
        "status": "PASS" if final_code == 0 else "FAIL",
        "gate_definition_sha256": inputs["gate_definition_sha256"],
        "input_sha256": inputs["input_sha256"],
        "environment_id": environment_id,
        "environment_sha256": inputs["environment_sha256"],
        "started_at": started,
        "ended_at": utc_now(),
        "duration_seconds": round(time.monotonic() - start, 3),
        "attempts": attempts,
        "log": {
            "path": log_path.relative_to(artifact_root).as_posix(),
            "sha256": file_sha256(log_path),
            "bytes": log_path.stat().st_size,
        },
    }
    prebuilt_manifest_sha = os.environ.get("RVOIP_PERF_PREBUILT_MANIFEST_SHA256")
    prebuilt_bundle_sha = os.environ.get("RVOIP_PERF_PREBUILT_BUNDLE_SHA256")
    if prebuilt_manifest_sha or prebuilt_bundle_sha:
        if not (
            prebuilt_manifest_sha
            and prebuilt_bundle_sha
            and re.fullmatch(r"[0-9a-f]{64}", prebuilt_manifest_sha)
            and re.fullmatch(r"[0-9a-f]{64}", prebuilt_bundle_sha)
        ):
            raise GateError("prebuilt performance digest environment is incomplete")
        receipt["prebuilt_performance"] = {
            "bundle_sha256": prebuilt_bundle_sha,
            "manifest_sha256": prebuilt_manifest_sha,
        }
    (artifact / "receipt.json").write_bytes(canonical_bytes(receipt))
    return receipt


def run_shard(
    *,
    root: Path,
    catalog: dict[str, Any],
    candidate: str,
    environment_id: str,
    gate_ids: list[str],
    output: Path,
    allow_legacy_groups: bool,
) -> int:
    resolved = git(root, "rev-parse", f"{candidate}^{{commit}}")
    if git(root, "rev-parse", "HEAD") != resolved:
        raise GateError("gate shard must run from the exact candidate checkout")
    by_id = gate_map(catalog)
    unknown = sorted(set(gate_ids) - set(by_id))
    if unknown:
        raise GateError(f"unknown gate ids: {unknown}")
    inputs = all_input_records(root, catalog, gate_ids, environment_id)
    output.mkdir(parents=True, exist_ok=True)
    failed = False
    for gate_id in dependency_order(gate_ids, by_id):
        print(f"running release gate {gate_id}", flush=True)
        receipt = execute_gate(
            root=root,
            gate=by_id[gate_id],
            candidate=resolved,
            environment_id=environment_id,
            inputs=inputs[gate_id],
            artifact_root=output,
            allow_legacy_groups=allow_legacy_groups,
        )
        failed |= receipt["status"] != "PASS"
    return 1 if failed else 0


def receipt_index(path: Path) -> dict[str, list[dict[str, Any]]]:
    return load_prior_receipts(path)


def validate_log(evidence_root: Path, receipt: dict[str, Any]) -> bool:
    receipt_path = Path(receipt.get("_path", ""))
    log = receipt.get("log", {})
    if not receipt_path or not isinstance(log, dict) or not log.get("path"):
        return False
    # A gate receipt sits in <artifact-root>/<gate>/receipt.json while its log
    # path is relative to <artifact-root>.
    candidate_roots = [receipt_path.parent.parent, evidence_root]
    for candidate_root in candidate_roots:
        log_path = candidate_root / log["path"]
        if log_path.is_file() and file_sha256(log_path) == log.get("sha256"):
            return True
    return False


def reconcile_performance_regression(root: Path, evidence_root: Path, artifact: Path) -> dict[str, Any]:
    manifests = sorted(evidence_root.rglob("perf-regression-baseline/manifest.json"))
    if len(manifests) != 1:
        raise GateError(
            "performance regression reconciliation requires exactly one packaged baseline manifest"
        )
    manifest = manifests[0]
    baseline = manifest.parent / "perf-results"
    payload = load_json(manifest, "packaged performance baseline")
    comparison_paths = payload.get("comparison_paths")
    if not isinstance(comparison_paths, list) or not comparison_paths:
        raise GateError("packaged performance baseline has no comparison paths")
    current = artifact / "current-performance"
    current.mkdir(parents=True, exist_ok=True)
    selected = {}
    for value in comparison_paths:
        if not isinstance(value, str) or Path(value).is_absolute() or ".." in Path(value).parts:
            raise GateError(f"unsafe performance comparison path: {value!r}")
        matches = sorted(
            path
            for path in (evidence_root / "_perf-results").rglob(Path(value).name)
            if path.as_posix().endswith("/" + value)
        )
        if len(matches) != 1:
            raise GateError(
                f"performance comparison path {value} has {len(matches)} exact-candidate results"
            )
        destination = current / value
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(matches[0], destination)
        selected[value] = file_sha256(destination)
    report = artifact / "perf-audit.md"
    completed = run(
        [
            "python3",
            "crates/sip/rvoip-sip/scripts/perf_audit.py",
            "--baseline",
            str(baseline),
            "--baseline-manifest",
            str(manifest),
            "--current",
            str(current),
            "--out",
            str(report),
            "--tolerance-pct",
            "15",
            "--latency-tolerance-pct",
            "25",
            "--fail-on-regression",
        ],
        root=root,
        check=False,
    )
    if completed.returncode:
        raise GateError(
            "performance regression audit failed:\n"
            + ((completed.stdout or "") + (completed.stderr or "")).strip()
        )
    return {
        "baseline_manifest_sha256": file_sha256(manifest),
        "selected_results": selected,
        "report_sha256": file_sha256(report),
    }


def collect(
    *,
    plan: dict[str, Any],
    evidence_root: Path,
    output: Path,
    root: Path | None = None,
) -> int:
    if plan.get("schema") != PLAN_SCHEMA:
        raise GateError("unsupported or missing release gate plan")
    receipts = receipt_index(evidence_root)
    failures = []
    accepted = []
    accepted_ids: set[str] = set()
    for item in plan["gates"]:
        gate_id = item["id"]
        if item.get("executor") == "aggregate" and item["decision"] == "RUN":
            continue
        if item["decision"] == "REUSE":
            receipt = item.get("reuse_receipt", {})
            if sha256_bytes(canonical_bytes(receipt)) != item.get("reuse_receipt_sha256"):
                failures.append(f"{gate_id}: reused receipt hash mismatch")
                continue
            if receipt.get("status") != "PASS" or not COMMIT_SHA.fullmatch(
                str(receipt.get("candidate_sha", ""))
            ):
                failures.append(f"{gate_id}: reused receipt is not a valid PASS record")
                continue
            source = "reused"
        else:
            candidates = [
                receipt
                for receipt in receipts.get(gate_id, [])
                if receipt.get("candidate_sha") == plan["candidate_sha"]
            ]
            if not candidates:
                failures.append(f"{gate_id}: missing exact-candidate receipt")
                continue
            receipt = candidates[-1]
            if not validate_log(evidence_root, receipt):
                failures.append(f"{gate_id}: log hash mismatch or missing log")
                continue
            source = "fresh"
        expected = {
            "status": "PASS",
            "gate_definition_sha256": item["gate_definition_sha256"],
            "input_sha256": item["input_sha256"],
            "environment_sha256": item["environment_sha256"],
        }
        mismatches = [key for key, value in expected.items() if receipt.get(key) != value]
        if mismatches:
            failures.append(f"{gate_id}: receipt mismatch in {', '.join(mismatches)}")
            continue
        accepted.append(
            {
                "gate_id": gate_id,
                "source": source,
                "receipt_candidate_sha": receipt.get("candidate_sha"),
                "receipt_sha256": sha256_bytes(canonical_bytes({key: value for key, value in receipt.items() if not key.startswith("_")})),
                "input_sha256": item["input_sha256"],
            }
        )
        accepted_ids.add(gate_id)

    plan_by_id = {item["id"]: item for item in plan["gates"]}
    aggregate_ids = [
        item["id"]
        for item in plan["gates"]
        if item.get("executor") == "aggregate" and item["decision"] == "RUN"
    ]
    aggregate_definitions = {
        gate_id: {"dependencies": plan_by_id[gate_id].get("dependencies", [])}
        for gate_id in aggregate_ids
    }
    for gate_id in dependency_order(aggregate_ids, aggregate_definitions):
        item = plan_by_id[gate_id]
        missing = sorted(
            dependency
            for dependency in item.get("dependencies", [])
            if dependency not in accepted_ids
        )
        if missing:
            failures.append(
                f"{gate_id}: cannot aggregate until dependencies pass: {', '.join(missing)}"
            )
            continue
        if root is not None and git(root, "rev-parse", "HEAD") != plan["candidate_sha"]:
            failures.append(f"{gate_id}: collector checkout does not match candidate")
            continue
        artifact = evidence_root / f"collect-{re.sub(r'[^A-Za-z0-9_.-]', '-', gate_id)}"
        artifact.mkdir(parents=True, exist_ok=True)
        log = artifact / "command.log"
        specialized = None
        if gate_id == "report.regression-audit":
            if root is None:
                failures.append(f"{gate_id}: collector workspace is unavailable")
                continue
            try:
                specialized = reconcile_performance_regression(root, evidence_root, artifact)
            except GateError as error:
                failures.append(f"{gate_id}: {error}")
                continue
        reconciliation = {
            "gate_id": gate_id,
            "candidate_sha": plan["candidate_sha"],
            "dependencies": item.get("dependencies", []),
            "accepted_dependency_receipts": sorted(
                row["receipt_sha256"]
                for row in accepted
                if row["gate_id"] in set(item.get("dependencies", []))
            ),
            "source_tree": git(root, "rev-parse", "HEAD^{tree}") if root is not None else None,
            "specialized_evidence": specialized,
        }
        log.write_bytes(canonical_bytes(reconciliation))
        receipt = {
            "schema": RECEIPT_SCHEMA,
            "gate_id": gate_id,
            "candidate_sha": plan["candidate_sha"],
            "status": "PASS",
            "gate_definition_sha256": item["gate_definition_sha256"],
            "input_sha256": item["input_sha256"],
            "environment_id": plan["environment_id"],
            "environment_sha256": item["environment_sha256"],
            "started_at": utc_now(),
            "ended_at": utc_now(),
            "duration_seconds": 0,
            "attempts": [{"attempt": 1, "argv": ["aggregate", gate_id], "exit_code": 0, "duration_seconds": 0}],
            "log": {
                "path": log.relative_to(evidence_root).as_posix(),
                "sha256": file_sha256(log),
                "bytes": log.stat().st_size,
            },
        }
        receipt_path = artifact / "receipt.json"
        receipt_path.write_bytes(canonical_bytes(receipt))
        accepted.append(
            {
                "gate_id": gate_id,
                "source": "fresh",
                "receipt_candidate_sha": plan["candidate_sha"],
                "receipt_sha256": file_sha256(receipt_path),
                "input_sha256": item["input_sha256"],
            }
        )
        accepted_ids.add(gate_id)
    aggregate = {
        "schema": AGGREGATE_SCHEMA,
        "generated_at": utc_now(),
        "candidate_sha": plan["candidate_sha"],
        "profile": plan["profile"],
        "catalog_sha256": plan["catalog_sha256"],
        "environment_id": plan["environment_id"],
        "publishing_attempted": False,
        "gate_count": len(plan["gates"]),
        "fresh_count": sum(item["source"] == "fresh" for item in accepted),
        "reused_count": sum(item["source"] == "reused" for item in accepted),
        "accepted_gates": accepted,
        "status": "FAIL" if failures else "PASS",
        "failures": failures,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(canonical_bytes(aggregate))
    if failures:
        print("release qualification failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print(
        f"release qualification passed with {aggregate['fresh_count']} fresh and "
        f"{aggregate['reused_count']} reused gates"
    )
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--catalog", type=Path, default=DEFAULT_CATALOG)
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("validate")
    plan = commands.add_parser("plan")
    plan.add_argument(
        "--profile",
        choices=(
            "remote-preflight",
            "remote-diagnostic",
            "remote-core",
            "remote-release",
            "legacy-full",
        ),
        required=True,
    )
    plan.add_argument(
        "--only-gates",
        help="comma-separated exact GCP gate IDs for remote-diagnostic",
    )
    plan.add_argument("--candidate", required=True)
    plan.add_argument("--environment-id", required=True)
    plan.add_argument("--prior-evidence", type=Path)
    plan.add_argument("--first-candidate", action="store_true")
    plan.add_argument("--changed-since")
    plan.add_argument("--output", type=Path, required=True)
    plan.add_argument("--github-output", type=Path)
    shard = commands.add_parser("run-shard")
    shard.add_argument("--candidate", required=True)
    shard.add_argument("--environment-id", required=True)
    shard.add_argument("--gates", required=True)
    shard.add_argument("--output", type=Path, required=True)
    shard.add_argument("--allow-legacy-groups", action="store_true")
    collector = commands.add_parser("collect")
    collector.add_argument("--plan", type=Path, required=True)
    collector.add_argument("--evidence", type=Path, required=True)
    collector.add_argument("--output", type=Path, required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    root = root_dir()
    try:
        catalog = load_catalog(root, args.catalog)
        if args.command == "validate":
            print(
                f"catalog valid: {len(catalog['gates'])} gates, "
                "108 legacy mappings, 44 crate gates"
            )
            return 0
        if args.command == "plan":
            prior = args.prior_evidence
            if prior and not prior.is_absolute():
                prior = root / prior
            plan = create_plan(
                root=root,
                catalog=catalog,
                profile=args.profile,
                candidate=args.candidate,
                environment_id=args.environment_id,
                prior_path=prior,
                first_candidate=args.first_candidate,
                changed_since=args.changed_since,
                only_gates=sorted(
                    {
                        value.strip()
                        for value in (args.only_gates or "").split(",")
                        if value.strip()
                    }
                ),
            )
            output = args.output if args.output.is_absolute() else root / args.output
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(canonical_bytes(plan))
            if args.github_output:
                write_github_output(args.github_output, plan)
            print(
                f"planned {sum(item['decision'] == 'RUN' for item in plan['gates'])} "
                f"fresh and {sum(item['decision'] == 'REUSE' for item in plan['gates'])} "
                f"reused gates in {len(plan['matrix'])} shard(s)"
            )
            return 0
        if args.command == "run-shard":
            output = args.output if args.output.is_absolute() else root / args.output
            return run_shard(
                root=root,
                catalog=catalog,
                candidate=args.candidate,
                environment_id=args.environment_id,
                gate_ids=sorted(set(filter(None, args.gates.split(",")))),
                output=output,
                allow_legacy_groups=args.allow_legacy_groups,
            )
        plan = load_json(args.plan if args.plan.is_absolute() else root / args.plan, "gate plan")
        evidence = args.evidence if args.evidence.is_absolute() else root / args.evidence
        output = args.output if args.output.is_absolute() else root / args.output
        return collect(plan=plan, evidence_root=evidence, output=output, root=root)
    except (GateError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"release gates: FAIL: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
