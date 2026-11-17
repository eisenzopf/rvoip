#!/usr/bin/env python3
"""Run one release-only RFC 7502 bench lane as a release gate.

The catalog's `perf.rfc7502-release` and `perf.rfc7502-minor` gates call this
wrapper (docs/release/BENCH_GATE_INTERFACE.md). It builds the device under
test (`rvoip-bench-uas`) from the exact candidate, then runs the lane
orchestrator, `scripts/bench/run_lane.py`, which the benchmark work owns.

A lane can outlive one hosted job (bench-minor takes about 11 hours), so every
call uses the same deterministic run id with `--resume` and a `--max-wall`
budget: run_lane continues the lane from its S3 state and exits 4 when the
budget ends first. `--advance` is for the workflow's earlier legs: it treats
"stopped at the budget" as progress, not failure. The final leg runs this
through `gates.py run-shard`, so the gate receipt is always the gate
executor's, and run_lane, finding the lane complete, only rewrites its
receipt.

The minor lane also records: `--record --record-out <artifact dir>/rates.json`
makes run_lane write the env-id's rates.json (the in-tree tester caps plus
this run's measured R per row, each row's source_run and candidate_sha naming
this run) into the gate's evidence instead of the checkout. The release ships
the measured commit unchanged, and that file goes to main in a rates-only PR
after the tag, for the next patch release's `release` lane to gate on.

Before it builds anything the wrapper checks that the lane can run: the
orchestrator, the device-under-test package, and a pinned bench environment
(`perf-baselines/rfc7502/ENVIRONMENT`, its `<env-id>/env.json`, and the
env-id's `rates.json` with the testbed's tester caps). Until the bench
environment PR lands none of these exist, and the gate fails at once with
exit 2, a usage error the executor never retries.

Exit codes follow run_lane: 0 PASS or WARN, 1 FAIL, 2 usage, 3 INFRA
(retried), 4 stopped at the budget.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
RUN_LANE = ROOT / "scripts/bench/run_lane.py"
BASELINES = Path("crates/sip/rvoip-sip/perf-baselines/rfc7502")
DUT_PACKAGE = "rvoip-bench-uas"
DEFAULT_MAX_WALL_MINUTES = 330
RECORDED_RATES = "rates.json"
EXIT_USAGE = 2
EXIT_BUDGET = 4


def preflight(root: Path) -> str | None:
    """Why the lane cannot run from this checkout, or None when it can."""

    if not (root / "scripts/bench/run_lane.py").is_file():
        return "the lane orchestrator scripts/bench/run_lane.py is not installed"
    metadata = subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if metadata.returncode:
        return f"cargo metadata failed: {metadata.stderr.strip()[-500:]}"
    if DUT_PACKAGE not in {package["name"] for package in json.loads(metadata.stdout)["packages"]}:
        return f"the device under test, package {DUT_PACKAGE}, is not in the workspace"
    pin = root / BASELINES / "ENVIRONMENT"
    env_id = pin.read_text().strip() if pin.is_file() else ""
    if not env_id:
        return f"bench environment not pinned: {BASELINES}/ENVIRONMENT is missing or empty"
    for name in ("env.json", "rates.json"):
        if not (root / BASELINES / env_id / name).is_file():
            return (
                f"bench environment not pinned: {BASELINES}/{env_id}/{name} is missing "
                "(the bench environment PR adds env.json and the §6.1 baseline's rates.json)"
            )
    return None


def lane_run_id(profile: str, candidate: str, run: str) -> str:
    """One id for every leg of one qualification run's lane."""

    return f"bench-{profile}-{candidate[:12]}-{run}"


def dut_binary(root: Path) -> Path:
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    if not target.is_absolute():
        target = root / target
    return target / "release" / DUT_PACKAGE


def lane_command(args: argparse.Namespace, dut: Path, run: str) -> list[str]:
    record = (
        ["--record", "--record-out", str(args.receipt.parent / RECORDED_RATES)]
        if args.profile == "minor"
        else []
    )
    return [
        sys.executable,
        str(RUN_LANE),
        "--profile",
        args.profile,
        "--candidate",
        args.candidate,
        "--receipt",
        str(args.receipt),
        "--dut-bin",
        str(dut),
        "--run-id",
        lane_run_id(args.profile, args.candidate, run),
        "--max-wall",
        str(args.max_wall),
        "--resume",
        *record,
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--profile", choices=("release", "minor"), required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument(
        "--max-wall",
        type=int,
        default=int(os.environ.get("RVOIP_BENCH_MAX_WALL", DEFAULT_MAX_WALL_MINUTES)),
        help="minutes this call may run before run_lane stops cleanly",
    )
    parser.add_argument(
        "--advance",
        action="store_true",
        help="an earlier workflow leg: stopping at the budget is progress",
    )
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[0-9a-f]{40}", args.candidate):
        print("bench gate: --candidate must be a full commit SHA", file=sys.stderr)
        return EXIT_USAGE
    problem = preflight(ROOT)
    if problem:
        print(f"bench gate: {problem}; not retried", file=sys.stderr)
        return EXIT_USAGE
    run = os.environ.get("RVOIP_BENCH_RUN_ID", "local")
    build = subprocess.run(
        ["cargo", "build", "--release", "--locked", "-p", DUT_PACKAGE], cwd=ROOT, check=False
    )
    if build.returncode:
        return 1
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    code = subprocess.run(lane_command(args, dut_binary(ROOT), run), cwd=ROOT, check=False).returncode
    if code == EXIT_USAGE:
        print("bench gate: run_lane rejected its inputs (exit 2, before any launch); not retried", file=sys.stderr)
    if args.advance and code in (0, 1, EXIT_BUDGET):
        # The final leg reports the verdict through the gate executor.
        return 0
    return code


if __name__ == "__main__":
    raise SystemExit(main())
