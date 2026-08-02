#!/usr/bin/env python3
"""Collect PR evidence and enforce the stable PR Gate contract."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import sys
from typing import Any


SCHEMA = "rvoip-pr-test-receipt-v1"
COMMAND_SCHEMA = "rvoip-ci-command-receipt-v1"


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def load_receipts(root: Path) -> list[dict[str, Any]]:
    receipts = []
    for path in sorted(root.rglob("*.json")):
        try:
            payload = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if payload.get("schema") == COMMAND_SCHEMA:
            payload["artifact_path"] = path.relative_to(root).as_posix()
            receipts.append(payload)
    return receipts


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--job", action="append", default=[])
    parser.add_argument("--required-job", action="append", default=[])
    parser.add_argument("--required-receipt", action="append", default=[])
    parser.add_argument(
        "--shard-layout",
        choices=("jobs", "shards"),
        default="jobs",
        help="validate receipts from shard_jobs or one receipt per shard",
    )
    args = parser.parse_args(argv)

    failures: list[str] = []
    try:
        plan = json.loads(args.plan.read_text())
    except (OSError, json.JSONDecodeError) as error:
        plan = {}
        failures.append(f"missing or invalid impact plan: {error}")

    jobs: dict[str, str] = {}
    for item in args.job:
        if "=" not in item:
            failures.append(f"invalid job result {item!r}")
            continue
        name, result = item.split("=", 1)
        jobs[name] = result

    for required in ("plan", "policy"):
        if jobs.get(required) != "success":
            failures.append(f"required job {required} was {jobs.get(required, 'missing')}")
    for required in args.required_job:
        if jobs.get(required) != "success":
            failures.append(f"required job {required} was {jobs.get(required, 'missing')}")

    shard_count = len(plan.get("shards", []))
    sip_count = len(plan.get("sip_jobs", []))
    specialty_count = len(plan.get("specialty_gates", []))
    if shard_count and jobs.get("crate-tests") != "success":
        failures.append(f"crate-tests was {jobs.get('crate-tests', 'missing')}")
    if not shard_count and jobs.get("crate-tests") not in {"skipped", "success"}:
        failures.append(f"unexpected crate-tests result {jobs.get('crate-tests', 'missing')}")
    if sip_count and jobs.get("sip-tests") != "success":
        failures.append(f"sip-tests was {jobs.get('sip-tests', 'missing')}")
    if not sip_count and jobs.get("sip-tests") not in {None, "skipped", "success"}:
        failures.append(f"unexpected sip-tests result {jobs.get('sip-tests', 'missing')}")
    if specialty_count and jobs.get("specialty") != "success":
        failures.append(f"specialty was {jobs.get('specialty', 'missing')}")
    if not specialty_count and jobs.get("specialty") not in {"skipped", "success"}:
        failures.append(f"unexpected specialty result {jobs.get('specialty', 'missing')}")

    receipts = load_receipts(args.evidence)
    by_name = {receipt.get("name"): receipt for receipt in receipts}
    if len(by_name) != len(receipts):
        failures.append("duplicate command receipt names were found")
    candidate_sha = plan.get("candidate_sha")
    if not isinstance(candidate_sha, str) or len(candidate_sha) != 40:
        failures.append("impact plan is missing a 40-character candidate SHA")
    expected = {"policy"}
    shard_jobs = plan.get("shard_jobs")
    if args.shard_layout == "shards" or shard_jobs is None:
        expected.update(f"shard-{shard['id']}" for shard in plan.get("shards", []))
    else:
        expected.update(f"shard-{job['shard_id']}-{job['check']}" for job in shard_jobs)
    expected.update(f"sip-{job['id']}" for job in plan.get("sip_jobs", []))
    expected.update(f"specialty-{gate}" for gate in plan.get("specialty_gates", []))
    expected.update(args.required_receipt)
    missing = sorted(expected - set(by_name))
    if missing:
        failures.append("missing command receipts: " + ", ".join(missing))
    for name in sorted(expected & set(by_name)):
        if by_name[name].get("status") != "PASS":
            failures.append(f"command receipt {name} did not pass")
        if candidate_sha and by_name[name].get("git_commit") != candidate_sha:
            failures.append(
                f"command receipt {name} is bound to "
                f"{by_name[name].get('git_commit', 'missing')} instead of {candidate_sha}"
            )

    receipt = {
        "schema": SCHEMA,
        "generated_at": utc_now(),
        "workflow_commit": os.getenv("GITHUB_SHA"),
        "repository": os.getenv("GITHUB_REPOSITORY"),
        "run_id": os.getenv("GITHUB_RUN_ID"),
        "run_attempt": os.getenv("GITHUB_RUN_ATTEMPT"),
        "artifact_links": artifact_links(receipts),
        "job_results": jobs,
        "plan": plan,
        "command_receipts": receipts,
        "status": "FAIL" if failures else "PASS",
        "failures": failures,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    if failures:
        print("PR Gate failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print(
        f"PR Gate passed: {shard_count} crate shard(s), {sip_count} SIP lane(s), "
        f"{specialty_count} specialty gate(s)."
    )
    return 0


def artifact_links(receipts: list[dict[str, Any]]) -> list[dict[str, str]]:
    """Bind receipt names to the immutable Actions run that stores them."""
    repository = os.getenv("GITHUB_REPOSITORY")
    run_id = os.getenv("GITHUB_RUN_ID")
    server = os.getenv("GITHUB_SERVER_URL", "https://github.com").rstrip("/")
    if not repository or not run_id:
        return []
    run_url = f"{server}/{repository}/actions/runs/{run_id}"
    return [
        {"name": str(receipt.get("name")), "url": run_url}
        for receipt in sorted(receipts, key=lambda item: str(item.get("name")))
    ]


if __name__ == "__main__":
    raise SystemExit(main())
