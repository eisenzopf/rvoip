# `bench-lane` gate interface

Status: proposal from the benchmark work (Phase 3), for agreement with the
release-process work that adds the `bench-lane` gate kind.

The benchmark design is in
[`crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md`](../crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md).
This file is only the contract between the gate executor in
`scripts/release/` and the lane orchestrator `scripts/bench/run_lane.py`.

## Gates

| Gate id | Profile | When |
|---|---|---|
| `perf.rfc7502-release` | `release` | every patch release candidate |
| `perf.rfc7502-minor` | `minor` | every minor release candidate (replaces `-release`) |

Both are release-only (never in the nightly profile). The version-bump commit
reuses the candidate's receipt through version-normalized hashing.

## Invocation

```
python3 scripts/bench/run_lane.py \
  --profile release|minor|full|baseline \
  --candidate <40-hex sha> \
  --receipt <path/receipt.json> \
  --dut-bin <rvoip-bench-uas built at the candidate>   # not used by baseline
  [--caller-bin <rvoip-loadgen>] [--ea-uas-bin <ea-uas>] # default: the env-id-pinned builds
  [--env-id <id>]              # default: crates/sip/rvoip-sip/perf-baselines/rfc7502/ENVIRONMENT
  [--bucket <name>]            # default: RVOIP_AWS_EVIDENCE_BUCKET
  [--run-id <id>]              # default: bench-<profile>-<sha12>-<utc stamp>
  [--resume]                   # see below
  [--max-wall <minutes>]       # from process start; see below
  [--dry-run]
```

- **Credentials**: the standard AWS CLI chain (EC2 launch/terminate from the
  bench launch templates, SSM SendCommand, S3). Needs `aws` v2, `openssl`
  and Python 3.10+. The permissions are the `rvoip-bench-orchestrator` managed
  policy from `infra/aws/bench`.
- **Binaries**: the executor builds only the DUT (`rvoip-bench-uas` at the
  candidate). The tester builds are pinned by the env-id:
  `perf-baselines/rfc7502/<env-id>/env.json` names an immutable S3 copy and
  sha256 of `rvoip-loadgen` and `ea-uas` (written by
  `scripts/bench/pin_env.py`); run_lane fetches and verifies them when
  `--caller-bin` / `--ea-uas-bin` are omitted. It refuses a lane whose AMI
  differs from the env-id's (exit 2: re-pin and re-baseline).
- **`--resume`**: state lives in S3 under the run-id, so a leg on any machine
  continues where the last one stopped (finished rows are kept, a search
  resumes its saved state, a trial with a summary is not rerun).
  - A run-id whose verdict is already final rewrites the receipt and exits 0
    or 1 at once, launching nothing.
  - An unknown run-id starts a new run, so every leg can pass the same flags.
- **`--max-wall`**: minutes from process start. No lane launch and no trial
  starts unless it would finish before the limit (launch ~20 min, a trial is
  its stages + hold + 10 min); the leg then tears the lane down and exits 4.
  `release` is about 3.5 h, `minor` about 11 h.
- **Concurrency**: one lane at a time per account. run_lane takes an S3 lock
  (`bench/lock.json`, conditional put) and exits 3 if another run holds it.
- **Inputs that should invalidate a receipt**: the DUT's crates,
  `scripts/bench/**`, `infra/aws/bench/**` and
  `crates/sip/rvoip-sip/perf-baselines/rfc7502/**` (the recorded R values and
  the pinned environment decide the verdict).

## Exit codes

| Code | Meaning | Executor action |
|---|---|---|
| 0 | `PASS` or `WARN` (receipt says which) | gate passes |
| 1 | `FAIL`: the DUT failed a confirmation trial or a latency budget | gate fails |
| 2 | usage or input error (bad sha, missing baseline, missing binary) | gate fails, not retried |
| 3 | `INFRA`: quota, capacity, lock held, SSM/S3 error, tester invalid three times | retry later (`retry_on_exit_codes: [3]`) |
| 4 | stopped at `--max-wall` (lane torn down); call again with `--resume` | re-invoke |

## Receipt

Written to `--receipt` (and to S3 beside the evidence) in the same canonical
JSON style as `gates.py` (sorted keys, no whitespace).

```json
{
  "schema": "rvoip-bench-lane-receipt-v1",
  "profile": "release",
  "candidate_sha": "<40 hex>",
  "run_id": "bench-release-0123456789ab-20261120T060000Z",
  "status": "PASS | WARN | FAIL | INFRA",
  "reasons": ["R2 1.05xR trial passed: likely improvement, record a new R after review"],
  "started_at": "...Z", "ended_at": "...Z", "duration_seconds": 12345.6,
  "environment": {
    "env_id": "aws-usw2-c8i4xl-2xc8in24xl-<ami8>-<caller8>",
    "region": "us-west-2", "availability_zone": "us-west-2b",
    "dut_instance_type": "c8i.4xlarge", "caller_instance_type": "c8in.24xlarge",
    "caller_count": 2, "sources_per_caller": 2, "connections_per_source": 8,
    "ami_id": "ami-...", "kernel": "6.8.0-...",
    "dut_sha256": "...", "caller_sha256": "...", "ea_uas_sha256": null
  },
  "baseline": {"path": "crates/sip/rvoip-sip/perf-baselines/rfc7502/<env-id>/rates.json", "sha256": "..."},
  "rows": [
    {
      "id": "R1", "transport": "tls", "media": "none", "hold_ms": 0,
      "recorded_r": 5000, "tester_cap": 6300,
      "trials": [
        {"factor": 0.95, "rate": 4748, "attempted": 904000, "established": 904000,
         "attempt_failures": 0, "disconnect_failures": 0, "status_counts": {"200": 904000},
         "outcome": "pass | fail | invalid", "invalid_reason": null,
         "evidence": "s3://.../R1/t0-r4748/"}
      ],
      "latency": {
        "rate": 4500,
        "srd_ms": {"p50": 18, "p95": 54, "p99": 66, "p99_9": 87, "max": 300},
        "sdd_ms": {"p50": 9, "p99": 62},
        "baseline_srd_ms": {"p50": 17, "p99": 60},
        "tolerance": {"relative": 0.25, "floor_ms": 10},
        "verdict": "pass | fail"
      },
      "verdict": "pass | warn | fail | infra"
    }
  ],
  "recommendation": {"R2": {"new_r": 1220, "basis": "1.05xR passed"}},
  "evidence": {
    "s3_prefix": "s3://<bucket>/bench/<run-id>/",
    "manifest_sha256": "<sha256 of manifest.json listing every object + sha256>",
    "report": {"md": "s3://.../report/rfc7502-report.md", "html": "..."}
  },
  "cost": {"instance_hours": {"c8i.4xlarge": 3.6, "c8in.24xlarge": 7.2}, "estimated_usd": 49.7},
  "tool": {"run_lane_sha256": "...", "repo_sha": "<sha the orchestrator ran from>"}
}
```

### Verdict rules (`release` profile)

- `FAIL` if any valid confirmation trial at 0.95 x R fails (any Session
  Attempt Failure), or a latency budget is exceeded: SRD/SDD p50..p99.9 at
  0.9 x R regress by more than max(25 %, 10 ms) against the recorded
  baseline (the `perf_audit.py` rule).
- `WARN` if every confirmation passes and the 1.05 x R trial also passes
  (likely improvement; record a new R after review). The receipt carries the
  proposed value.
- `INFRA` (exit 3) if a row cannot produce three valid confirmation trials
  (three invalid trials in a row, capacity, quota).
- `PASS` otherwise.

`minor` adds the bisect searches (a row fails if its R falls below
0.95 x the recorded R), R4 overload/recovery (fails on any non-503 failure or
a recovery stage below 100 %), and the rendered IETF report, which the release
attaches to the GitHub release.

## Recorded rates

`crates/sip/rvoip-sip/perf-baselines/rfc7502/<env-id>/rates.json`:

```json
{"schema": "rvoip-rfc7502-rates-v1", "env_id": "...", "recorded_at": "...",
 "source_run": "s3://...", "candidate_sha": "...",
 "rows": {"R1": {"r": 5000, "tester_cap": 6300, "srd_ms": {...}, "sdd_ms": {...}}}}
```

`perf-baselines/rfc7502/ENVIRONMENT` names the current env-id. A new env-id
(new AMI, caller build or instance type) needs a `baseline` run and a `minor`
run before `release` can gate on it.
