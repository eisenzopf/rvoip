#!/usr/bin/env python3
"""Plan bounded PR checks from Cargo's workspace dependency graph."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import fnmatch
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
from typing import Any, Iterable


SCHEMA = "rvoip-pr-test-plan-v1"


class PlanError(RuntimeError):
    """A fail-closed impact-planning error."""


@dataclass(frozen=True)
class Package:
    name: str
    root: str
    dependencies: frozenset[str]


def run(argv: list[str], root: Path) -> str:
    completed = subprocess.run(
        argv, cwd=root, text=True, capture_output=True, check=False
    )
    if completed.returncode:
        detail = (completed.stdout or "") + (completed.stderr or "")
        raise PlanError(f"command failed: {' '.join(argv)}\n{detail.strip()}")
    return completed.stdout


def normalize_path(value: str) -> str:
    normalized = str(PurePosixPath(value.replace("\\", "/")))
    while normalized.startswith("./"):
        normalized = normalized[2:]
    if normalized == "." or normalized.startswith("../") or normalized.startswith("/"):
        raise PlanError(f"changed path escapes the repository: {value!r}")
    return normalized


def parse_name_status_z(payload: bytes) -> list[str]:
    """Return old and new paths from `git diff --name-status -z`."""
    fields = payload.decode("utf-8", errors="strict").split("\0")
    if fields and fields[-1] == "":
        fields.pop()
    paths: list[str] = []
    index = 0
    while index < len(fields):
        status = fields[index]
        index += 1
        if not status:
            raise PlanError("empty git diff status")
        path_count = 2 if status[0] in {"R", "C"} else 1
        if index + path_count > len(fields):
            raise PlanError(f"truncated git diff record for {status!r}")
        for value in fields[index : index + path_count]:
            paths.append(normalize_path(value))
        index += path_count
    return sorted(set(paths))


def changed_paths(root: Path, base: str, head: str) -> list[str]:
    completed = subprocess.run(
        [
            "git",
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            f"{base}...{head}",
        ],
        cwd=root,
        capture_output=True,
        check=False,
    )
    if completed.returncode:
        detail = completed.stderr.decode("utf-8", errors="replace")
        raise PlanError(f"cannot calculate changed files: {detail.strip()}")
    return parse_name_status_z(completed.stdout)


def load_metadata(root: Path, metadata_file: Path | None) -> dict[str, Any]:
    if metadata_file:
        return json.loads(metadata_file.read_text())
    return json.loads(
        run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            root,
        )
    )


def workspace_packages(root: Path, metadata: dict[str, Any]) -> dict[str, Package]:
    members = set(metadata.get("workspace_members", []))
    raw_packages = [package for package in metadata.get("packages", []) if package["id"] in members]
    names = {package["name"] for package in raw_packages}
    if not names:
        raise PlanError("cargo metadata returned no workspace packages")
    if len(names) != len(raw_packages):
        raise PlanError("workspace package names must be unique")

    result: dict[str, Package] = {}
    resolved_root = root.resolve()
    for raw in raw_packages:
        manifest = Path(raw["manifest_path"]).resolve()
        try:
            package_root = manifest.parent.relative_to(resolved_root).as_posix()
        except ValueError as error:
            raise PlanError(f"workspace manifest is outside repository: {manifest}") from error
        dependencies = frozenset(
            dependency.get("package", dependency["name"])
            for dependency in raw.get("dependencies", [])
            if dependency.get("package", dependency["name"]) in names
        )
        result[raw["name"]] = Package(raw["name"], package_root, dependencies)
    return result


def matches_any(path: str, patterns: Iterable[str]) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)


def documentation_path(path: str, known_policy_paths: set[str]) -> bool:
    if matches_any(path, known_policy_paths):
        return True
    if path.startswith("docs/") and path.endswith((".md", ".txt")):
        return True
    if path.endswith(".md") and "/public-api/" not in path:
        return True
    return path.startswith(".github/ISSUE_TEMPLATE/")


def owning_package(path: str, packages: dict[str, Package]) -> str | None:
    owners = [
        package
        for package in packages.values()
        if path == package.root or path.startswith(f"{package.root}/")
    ]
    if not owners:
        return None
    return max(owners, key=lambda package: len(package.root)).name


def reverse_closure(direct: set[str], packages: dict[str, Package]) -> set[str]:
    dependents: dict[str, set[str]] = {name: set() for name in packages}
    for package in packages.values():
        for dependency in package.dependencies:
            dependents[dependency].add(package.name)
    selected = set(direct)
    pending = list(sorted(direct))
    while pending:
        dependency = pending.pop()
        for dependent in sorted(dependents[dependency]):
            if dependent not in selected:
                selected.add(dependent)
                pending.append(dependent)
    return selected


def make_shards(
    selected: set[str], policy: dict[str, Any]
) -> list[dict[str, Any]]:
    if not selected:
        return []
    max_shards = int(policy.get("max_shards", 6))
    target_weight = max(1, int(policy.get("target_shard_weight", 12)))
    weights = policy.get("package_weights", {})
    weighted = sorted(
        ((name, max(1, int(weights.get(name, 2)))) for name in selected),
        key=lambda item: (-item[1], item[0]),
    )
    total = sum(weight for _, weight in weighted)
    shard_count = min(max_shards, len(weighted), max(1, (total + target_weight - 1) // target_weight))
    shards: list[dict[str, Any]] = [
        {"id": str(index + 1), "packages": [], "weight": 0}
        for index in range(shard_count)
    ]
    for name, weight in weighted:
        target = min(shards, key=lambda shard: (shard["weight"], shard["id"]))
        target["packages"].append(name)
        target["weight"] += weight
    for shard in shards:
        shard["packages"].sort()
        shard["packages_csv"] = ",".join(shard["packages"])
    return shards


def make_plan(
    *,
    root: Path,
    metadata: dict[str, Any],
    policy: dict[str, Any],
    paths: list[str],
    base: str,
    head: str,
    candidate: str | None = None,
) -> dict[str, Any]:
    normalized = sorted({normalize_path(path) for path in paths})
    packages = workspace_packages(root, metadata)
    known_policy_paths = set(policy.get("known_policy_paths", []))
    specialty_set = {
            rule["gate"]
            for rule in policy.get("specialty_rules", [])
            if any(matches_any(path, rule.get("patterns", [])) for path in normalized)
        }
    if "examples" in specialty_set:
        specialty_set.remove("examples")
        projects = policy.get("example_projects", [])
        directly_changed = {
            project
            for project in projects
            if any(path.startswith(f"examples/{project}/") for path in normalized)
        }
        # Changes contained within examples build only those projects. Public
        # API/facade changes use one representative contract lane on PRs;
        # Main Gate still builds every standalone example.
        if directly_changed and all(path.startswith("examples/") for path in normalized):
            specialty_set.update(f"example--{project}" for project in directly_changed)
        else:
            specialty_set.add("examples-contract")
    specialty = sorted(specialty_set)

    full_reasons = [
        path for path in normalized if matches_any(path, policy.get("full_paths", []))
    ]
    docs_only = bool(normalized) and all(
        documentation_path(path, known_policy_paths) for path in normalized
    )
    direct: set[str] = set()
    unknown: list[str] = []
    if not docs_only and not full_reasons:
        for path in normalized:
            owner = owning_package(path, packages)
            if owner:
                direct.add(owner)
            elif not matches_any(path, known_policy_paths):
                # Specialty-only trees are mapped even though they are not Cargo members.
                if not any(
                    matches_any(path, rule.get("patterns", []))
                    for rule in policy.get("specialty_rules", [])
                ):
                    unknown.append(path)

    if not normalized:
        full_reasons.append("empty change set")
    if unknown:
        full_reasons.extend(unknown)

    if docs_only:
        mode = (
            "docs"
            if all(path.endswith((".md", ".txt")) for path in normalized)
            else "policy"
        )
        selected: set[str] = set()
        reason = "documentation or repository policy only"
    elif full_reasons:
        mode = "full"
        selected = set(packages)
        reason = "full-workspace input: " + ", ".join(sorted(full_reasons))
    else:
        mode = "targeted"
        selected = reverse_closure(direct, packages)
        reason = "changed crates plus transitive reverse dependencies"

    shards = make_shards(selected, policy)
    shard_jobs = [
        {
            "id": f"{shard['id']}-all",
            "shard_id": shard["id"],
            "check": "all",
            "packages": shard["packages"],
            "packages_csv": shard["packages_csv"],
            "weight": shard["weight"],
        }
        for shard in shards
    ]
    candidate_sha = (
        run(["git", "rev-parse", f"{candidate}^{{commit}}"], root).strip()
        if candidate
        else None
    )
    return {
        "schema": SCHEMA,
        "base": base,
        "head": head,
        # In a pull_request workflow `head` is the contributor's source
        # commit, while tests run against GitHub's synthetic merge commit.
        # Keep both identities so receipts can bind to what was actually run.
        "candidate_sha": candidate_sha,
        "mode": mode,
        "reason": reason,
        "changed_files": normalized,
        "direct_crates": sorted(direct),
        "selected_crates": sorted(selected),
        "specialty_gates": specialty,
        "shards": shards,
        "shard_jobs": shard_jobs,
    }


def write_github_outputs(path: Path, plan: dict[str, Any]) -> None:
    shard_jobs = {"include": plan["shard_jobs"]}
    shards = {"include": plan["shards"]}
    specialty = {"include": [{"gate": gate} for gate in plan["specialty_gates"]]}
    values = {
        "mode": plan["mode"],
        "reason": plan["reason"],
        "candidate_sha": plan["candidate_sha"] or "",
        "shard_jobs": json.dumps(shard_jobs, separators=(",", ":")),
        "shards": json.dumps(shards, separators=(",", ":")),
        "shard_job_count": str(len(plan["shard_jobs"])),
        "shard_count": str(len(plan["shards"])),
        "specialty": json.dumps(specialty, separators=(",", ":")),
        "specialty_count": str(len(plan["specialty_gates"])),
    }
    with path.open("a") as handle:
        for key, value in values.items():
            if "\n" in value:
                raise PlanError(f"GitHub output {key} unexpectedly contains a newline")
            handle.write(f"{key}={value}\n")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--base", default="origin/main")
    result.add_argument("--head", default="HEAD")
    result.add_argument(
        "--candidate",
        default="HEAD",
        help="checked-out commit that the CI receipt must bind to",
    )
    result.add_argument("--changed-file", action="append", default=[])
    result.add_argument("--specialty-gate", action="append", default=[])
    result.add_argument("--metadata-file", type=Path)
    result.add_argument(
        "--policy", type=Path, default=Path("scripts/ci/policy.json")
    )
    result.add_argument("--output", type=Path, default=Path("target/ci-plan/plan.json"))
    result.add_argument("--github-output", type=Path)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    root = Path(__file__).resolve().parents[2]
    try:
        policy_path = args.policy if args.policy.is_absolute() else root / args.policy
        metadata_file = args.metadata_file
        if metadata_file and not metadata_file.is_absolute():
            metadata_file = root / metadata_file
        paths = args.changed_file or changed_paths(root, args.base, args.head)
        plan = make_plan(
            root=root,
            metadata=load_metadata(root, metadata_file),
            policy=json.loads(policy_path.read_text()),
            paths=paths,
            base=args.base,
            head=args.head,
            candidate=args.candidate,
        )
        for gate in args.specialty_gate:
            if not gate or any(character not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for character in gate):
                raise PlanError(f"invalid forced specialty gate: {gate!r}")
        plan["specialty_gates"] = sorted(
            set(plan["specialty_gates"]) | set(args.specialty_gate)
        )
        output = args.output if args.output.is_absolute() else root / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
        if args.github_output:
            write_github_outputs(args.github_output, plan)
        print(json.dumps(plan, indent=2, sort_keys=True))
        return 0
    except (OSError, ValueError, PlanError, json.JSONDecodeError) as error:
        print(f"PR impact planning failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
