#!/usr/bin/env python3
"""Prepare and verify one-controller, many-worker GCP release fanout."""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import tarfile
import tempfile
from typing import Any


MANIFEST_SCHEMA = "rvoip-gcp-release-fanout-v1"
RESULT_SCHEMA = "rvoip-gcp-release-shard-v1"
COMMIT_SHA = re.compile(r"^[0-9a-f]{40}$")
RUN_NUMBER = re.compile(r"^[1-9][0-9]*$")
SHARD_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,47}$")
GATE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
CONTROLLER_EVIDENCE_DIR = "_gcp-controller"
WORKER_EVIDENCE_DIR = "_gcp-workers"
WORKER_SIDECAR_NAMES = frozenset(
    {
        "_external-process-memory.tsv",
        "_host-memory-policy.txt",
        "_sccache-stats.txt",
    }
)
RESOURCE_MACHINES = {
    "gcp-interop": "n2-standard-4",
    "gcp-performance": "n2-standard-8",
    "gcp-performance-soak": "n2-standard-4",
    "gcp-performance-soak-long": "n2-standard-8",
    "gcp-proxy-interop": "n2-standard-2",
}
RESOURCE_DISK_GB = {
    "gcp-proxy-interop": 100,
}
DEFERRED_RESOURCE_CLASSES = {
    "gcp-interop",
    "gcp-performance-soak-long",
}
MACHINE_VCPUS = {
    "n2-standard-2": 2,
    "n2-standard-4": 4,
    "n2-standard-8": 8,
}


class FanoutError(RuntimeError):
    """A release fanout invariant failed closed."""


def load_json(path: Path, description: str) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise FanoutError(f"cannot read {description} {path}: {error}") from error
    if not isinstance(payload, dict):
        raise FanoutError(f"{description} must be a JSON object: {path}")
    return payload


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def require_string(record: dict[str, Any], key: str, description: str) -> str:
    value = record.get(key)
    if not isinstance(value, str) or not value:
        raise FanoutError(f"{description} requires a non-empty {key}")
    return value


def prepare_manifest(
    *,
    matrix: dict[str, Any],
    candidate: str,
    environment_id: str,
    run_id: str,
    run_attempt: str,
) -> dict[str, Any]:
    if not COMMIT_SHA.fullmatch(candidate):
        raise FanoutError("candidate must be a full lowercase commit SHA")
    if not environment_id:
        raise FanoutError("environment id must not be empty")
    if not RUN_NUMBER.fullmatch(run_id) or not RUN_NUMBER.fullmatch(run_attempt):
        raise FanoutError("GitHub run id and attempt must be positive integers")
    include = matrix.get("include")
    if not isinstance(include, list) or not include:
        raise FanoutError("GCP matrix must contain at least one worker")

    workers = []
    seen_shards: set[str] = set()
    for raw in include:
        if not isinstance(raw, dict):
            raise FanoutError("every GCP matrix entry must be an object")
        shard_id = require_string(raw, "id", "GCP matrix entry")
        if not SHARD_ID.fullmatch(shard_id):
            raise FanoutError(f"unsafe GCP shard id: {shard_id!r}")
        if shard_id in seen_shards:
            raise FanoutError(f"duplicate GCP shard id: {shard_id}")
        seen_shards.add(shard_id)

        resource_class = require_string(raw, "resource_class", shard_id)
        expected_machine = RESOURCE_MACHINES.get(resource_class)
        if expected_machine is None:
            raise FanoutError(f"{shard_id} has unsupported resource class {resource_class!r}")
        machine_type = require_string(raw, "machine_type", shard_id)
        if machine_type != expected_machine:
            raise FanoutError(
                f"{shard_id} must use {expected_machine}, not {machine_type}"
            )
        disk_type = require_string(raw, "disk_type", shard_id)
        if disk_type != "pd-standard":
            raise FanoutError(f"{shard_id} must use pd-standard storage")
        disk_size_gb = raw.get("disk_size_gb")
        expected_disk_gb = RESOURCE_DISK_GB.get(resource_class, 200)
        if disk_size_gb != expected_disk_gb:
            raise FanoutError(
                f"{shard_id} must use a {expected_disk_gb} GB boot disk"
            )

        gates_csv = require_string(raw, "gates_csv", shard_id)
        gates = gates_csv.split(",")
        if any(not GATE_ID.fullmatch(gate) for gate in gates):
            raise FanoutError(f"{shard_id} contains an unsafe or empty gate id")
        if len(gates) != len(set(gates)):
            raise FanoutError(f"{shard_id} contains duplicate gate ids")

        worker_name = f"rvoip-rel-{run_id}-{run_attempt}-{shard_id}"
        if len(worker_name) > 63:
            raise FanoutError(f"worker name exceeds the GCE 63-character limit: {worker_name}")
        prefix = f"release/{run_id}-{run_attempt}/{shard_id}"
        workers.append(
            {
                "id": shard_id,
                "name": worker_name,
                "prefix": prefix,
                "resource_class": resource_class,
                "machine_type": machine_type,
                "disk_type": disk_type,
                "disk_size_gb": disk_size_gb,
                "gates": gates,
                "gates_csv": gates_csv,
                "gates_b64": base64.b64encode(gates_csv.encode()).decode(),
                "environment_b64": base64.b64encode(environment_id.encode()).decode(),
            }
        )

    workers.sort(key=lambda item: item["id"])
    return {
        "schema": MANIFEST_SCHEMA,
        "candidate_sha": candidate,
        "environment_id": environment_id,
        "github_run_id": run_id,
        "github_run_attempt": run_attempt,
        "worker_count": len(workers),
        "required_vcpus": sum(MACHINE_VCPUS[item["machine_type"]] for item in workers),
        "workers": workers,
    }


def validate_manifest(manifest: dict[str, Any]) -> None:
    if manifest.get("schema") != MANIFEST_SCHEMA:
        raise FanoutError("unsupported GCP fanout manifest schema")
    regenerated = prepare_manifest(
        matrix={
            "include": [
                {
                    "id": worker.get("id"),
                    "resource_class": worker.get("resource_class"),
                    "machine_type": worker.get("machine_type"),
                    "disk_type": worker.get("disk_type"),
                    "disk_size_gb": worker.get("disk_size_gb"),
                    "gates_csv": worker.get("gates_csv"),
                }
                for worker in manifest.get("workers", [])
                if isinstance(worker, dict)
            ]
        },
        candidate=require_string(manifest, "candidate_sha", "fanout manifest"),
        environment_id=require_string(manifest, "environment_id", "fanout manifest"),
        run_id=require_string(manifest, "github_run_id", "fanout manifest"),
        run_attempt=require_string(manifest, "github_run_attempt", "fanout manifest"),
    )
    if regenerated != manifest:
        raise FanoutError("GCP fanout manifest is inconsistent or has been altered")


def safe_archive_members(archive: Path) -> list[tarfile.TarInfo]:
    try:
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
    except (OSError, tarfile.TarError) as error:
        raise FanoutError(f"cannot read evidence archive {archive}: {error}") from error
    if not members:
        raise FanoutError(f"evidence archive is empty: {archive}")
    for member in members:
        path = PurePosixPath(member.name)
        if (
            path.is_absolute()
            or ".." in path.parts
            or not path.parts
            or path.parts[0] != "release-shard"
        ):
            raise FanoutError(f"unsafe evidence archive path: {member.name!r}")
        relative = path.relative_to("release-shard")
        if relative.parts and relative.parts[0] in {
            CONTROLLER_EVIDENCE_DIR,
            WORKER_EVIDENCE_DIR,
        }:
            raise FanoutError(
                f"evidence archive uses a reserved controller path: {member.name!r}"
            )
        if not (member.isdir() or member.isfile()):
            raise FanoutError(f"unsupported evidence archive member: {member.name!r}")
    return members


def merge_archive(archive: Path, destination: Path, shard_id: str) -> None:
    if not SHARD_ID.fullmatch(shard_id):
        raise FanoutError(f"unsafe GCP shard id: {shard_id!r}")
    members = safe_archive_members(archive)
    with tarfile.open(archive, "r:gz") as bundle:
        for member in members:
            relative = PurePosixPath(member.name).relative_to("release-shard")
            if member.isfile() and str(relative) in WORKER_SIDECAR_NAMES:
                # These files describe an individual worker rather than a release
                # gate. Preserve each copy without weakening duplicate detection
                # for the shared gate-evidence namespace.
                target = destination / WORKER_EVIDENCE_DIR / shard_id / relative.name
            else:
                target = destination.joinpath(*relative.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            if target.exists():
                raise FanoutError(f"duplicate evidence path across GCP shards: {relative}")
            target.parent.mkdir(parents=True, exist_ok=True)
            source = bundle.extractfile(member)
            if source is None:
                raise FanoutError(f"cannot extract evidence member: {member.name}")
            with source, target.open("wb") as output:
                shutil.copyfileobj(source, output)


def validate_result(
    *, worker: dict[str, Any], result: dict[str, Any], manifest: dict[str, Any]
) -> bool:
    shard = worker["id"]
    expected_run = f"{manifest['github_run_id']}-{manifest['github_run_attempt']}"
    checks = {
        "schema": (result.get("schema"), RESULT_SCHEMA),
        "candidate_sha": (result.get("candidate_sha"), manifest["candidate_sha"]),
        "github_run_id": (result.get("github_run_id"), expected_run),
        "shard_id": (result.get("shard_id"), shard),
        "gates": (result.get("gates"), sorted(worker["gates"])),
        "publishing_attempted": (result.get("publishing_attempted"), False),
    }
    failures = [
        f"{key}: expected {expected!r}, got {actual!r}"
        for key, (actual, expected) in checks.items()
        if actual != expected
    ]
    archive_sha = result.get("evidence_archive_sha256")
    if not isinstance(archive_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", archive_sha):
        failures.append("evidence_archive_sha256 is missing or invalid")
    if failures:
        raise FanoutError(f"{shard} result failed verification:\n- " + "\n- ".join(failures))
    status = result.get("status")
    exit_code = result.get("exit_code")
    if status == "PASS" and exit_code == 0:
        return True
    if status == "FAIL" and isinstance(exit_code, int) and exit_code != 0:
        return False
    raise FanoutError(
        f"{shard} result has inconsistent status {status!r} and exit code {exit_code!r}"
    )


def load_instance_states(path: Path) -> dict[str, str]:
    """Load the controller's exact GCE instance-name/status snapshot."""
    try:
        with path.open(newline="", encoding="utf-8") as handle:
            rows = list(csv.reader(handle))
    except OSError as error:
        raise FanoutError(f"cannot read GCP instance states {path}: {error}") from error
    states: dict[str, str] = {}
    for row in rows:
        if len(row) != 2 or not row[0] or not row[1]:
            raise FanoutError(f"invalid GCP instance-state row: {row!r}")
        if row[0] in states:
            raise FanoutError(f"duplicate GCP instance-state row: {row[0]}")
        states[row[0]] = row[1]
    return states


def early_failure_decision(
    *, manifest: dict[str, Any], downloads: Path, states: dict[str, str]
) -> dict[str, Any]:
    """Decide whether failed bounded work makes deferred work unnecessary."""
    validate_manifest(manifest)
    early_expected = 0
    early_settled = 0
    failed_shards: list[str] = []
    invalid_results: dict[str, str] = {}
    deferred_running: list[str] = []

    for worker in manifest["workers"]:
        shard = worker["id"]
        is_deferred = worker["resource_class"] in DEFERRED_RESOURCE_CLASSES
        if not is_deferred:
            early_expected += 1
        result_path = downloads / shard / "result.json"
        if result_path.is_file():
            try:
                result = load_json(result_path, f"{shard} result")
                passed = validate_result(
                    worker=worker, result=result, manifest=manifest
                )
            except FanoutError as error:
                passed = False
                invalid_results[shard] = str(error)
            if not passed:
                failed_shards.append(shard)
            if not is_deferred:
                early_settled += 1
            continue

        if states.get(worker["name"]) == "TERMINATED":
            failed_shards.append(shard)
            if not is_deferred:
                early_settled += 1
        elif is_deferred:
            deferred_running.append(worker["name"])

    failed_shards.sort()
    deferred_running.sort()
    return {
        "schema": "rvoip-gcp-early-failure-decision-v1",
        "early_expected": early_expected,
        "early_settled": early_settled,
        "failed_shards": failed_shards,
        "invalid_results": invalid_results,
        "deferred_running": deferred_running,
        "should_stop": bool(
            early_expected > 0
            and early_settled == early_expected
            and failed_shards
            and deferred_running
        ),
    }


def verify_fanout(
    *, manifest: dict[str, Any], downloads: Path, output: Path
) -> dict[str, Any]:
    validate_manifest(manifest)
    if output.exists():
        raise FanoutError(f"refusing to overwrite existing evidence directory: {output}")
    if not downloads.is_dir():
        raise FanoutError(f"GCP download directory does not exist: {downloads}")

    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}-", dir=output.parent))
    try:
        controller = staging / CONTROLLER_EVIDENCE_DIR
        controller.mkdir()
        errors: list[str] = []
        failed_shards: list[str] = []
        trusted_shards: list[str] = []
        for worker in manifest["workers"]:
            shard = worker["id"]
            shard_dir = downloads / shard
            result_path = shard_dir / "result.json"
            archive_path = shard_dir / "release-shard.tar.gz"
            try:
                result = load_json(result_path, f"{shard} result")
                passed = validate_result(
                    worker=worker, result=result, manifest=manifest
                )
            except FanoutError as error:
                errors.append(str(error))
                failed_shards.append(shard)
                continue

            shard_controller = controller / worker["id"]
            shard_controller.mkdir()
            shutil.copy2(result_path, shard_controller / "result.json")
            log = result_path.with_name("qualification.log")
            if log.is_file():
                shutil.copy2(log, shard_controller / "qualification.log")
            if not archive_path.is_file():
                errors.append(f"{shard} evidence archive is missing")
                failed_shards.append(shard)
                continue
            actual_sha = sha256(archive_path)
            if actual_sha != result["evidence_archive_sha256"]:
                errors.append(
                    f"{shard} evidence archive digest mismatch: "
                    f"expected {result['evidence_archive_sha256']}, got {actual_sha}"
                )
                failed_shards.append(shard)
                continue
            try:
                merge_archive(archive_path, staging, shard)
            except FanoutError as error:
                errors.append(str(error))
                failed_shards.append(shard)
                continue
            trusted_shards.append(shard)
            if not passed:
                errors.append(f"{shard} reported a failed gate shard")
                failed_shards.append(shard)

        failed_shards = sorted(set(failed_shards))
        receipt = {
            "schema": "rvoip-gcp-release-fanout-receipt-v1",
            "candidate_sha": manifest["candidate_sha"],
            "github_run_id": manifest["github_run_id"],
            "github_run_attempt": manifest["github_run_attempt"],
            "worker_count": manifest["worker_count"],
            "required_vcpus": manifest["required_vcpus"],
            "trusted_shards": trusted_shards,
            "failed_shards": failed_shards,
            "errors": errors,
            "status": "FAIL" if errors else "PASS",
            "publishing_attempted": False,
        }
        write_json(controller / "fanout-receipt.json", receipt)
        staging.rename(output)
        return receipt
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    prepare = commands.add_parser("prepare")
    prepare.add_argument("--matrix", type=Path, required=True)
    prepare.add_argument("--candidate", required=True)
    prepare.add_argument("--environment-id", required=True)
    prepare.add_argument("--run-id", required=True)
    prepare.add_argument("--run-attempt", required=True)
    prepare.add_argument("--output", type=Path, required=True)

    verify = commands.add_parser("verify")
    verify.add_argument("--manifest", type=Path, required=True)
    verify.add_argument("--downloads", type=Path, required=True)
    verify.add_argument("--output", type=Path, required=True)

    cutoff = commands.add_parser("early-failure-decision")
    cutoff.add_argument("--manifest", type=Path, required=True)
    cutoff.add_argument("--downloads", type=Path, required=True)
    cutoff.add_argument("--states", type=Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "prepare":
            manifest = prepare_manifest(
                matrix=load_json(args.matrix, "GCP matrix"),
                candidate=args.candidate,
                environment_id=args.environment_id,
                run_id=args.run_id,
                run_attempt=args.run_attempt,
            )
            write_json(args.output, manifest)
        elif args.command == "verify":
            receipt = verify_fanout(
                manifest=load_json(args.manifest, "GCP fanout manifest"),
                downloads=args.downloads,
                output=args.output,
            )
            print(json.dumps(receipt, sort_keys=True))
            if receipt["status"] != "PASS":
                return 1
        else:
            decision = early_failure_decision(
                manifest=load_json(args.manifest, "GCP fanout manifest"),
                downloads=args.downloads,
                states=load_instance_states(args.states),
            )
            print(json.dumps(decision, sort_keys=True))
    except FanoutError as error:
        print(f"GCP release fanout error: {error}", flush=True)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
