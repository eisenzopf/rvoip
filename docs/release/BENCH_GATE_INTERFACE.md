# Release benchmark gate interface (RFC 7501/7502)

Status: agreed with the benchmark work, 2026-11-16. The release tooling side
is implemented; the lane itself (`scripts/bench/`) is built separately and
owns the receipt's full schema.

Ownership: `scripts/bench/` belongs to the benchmark work. `scripts/release/`,
the gate catalog and the workflows belong to the release tooling.

## Gates and profiles

| Gate | Lane profile | Used by |
|---|---|---|
| `perf.rfc7502-release` | `release` (bench-release: R1, R5, R2 confirmation trials) | every patch release |
| `perf.rfc7502-minor` | `minor` (bench-minor: searches, R6, R4 overload, IETF report) | every minor release |

Both are in resource class `bench-lane`, depend only on
`source.remote-clean`, and are **never** in the nightly profile
`remote-release`, nor among the dependencies of `report.remote-aggregate`.
They appear in four catalog profiles:

| Profile | Contents | When |
|---|---|---|
| `bench-release` | `source.remote-clean`, `perf.rfc7502-release` | on the release candidate, at the start of release day |
| `bench-minor` | `source.remote-clean`, `perf.rfc7502-minor` | on the minor candidate, the evening before |
| `release-patch` | `remote-release` + `perf.rfc7502-release` | requalification of the version-bump commit |
| `release-minor` | `remote-release` + `perf.rfc7502-minor` | requalification of the version-bump commit |

From 0.5.0, `release.py verify` accepts only `release-minor` for an `X.Y.0`
version and `release-patch` or `release-minor` for a patch
(`BENCH_REQUIRED_FROM` in `scripts/release.py`). Earlier versions may still
publish from `remote-release`.

## How it runs

The gate command is the release tooling's wrapper:

```sh
python3 scripts/release/bench_gate.py --profile release|minor \
  --candidate <40-hex sha> --receipt <artifact_dir>/bench-receipt.json
```

It builds the device under test from the exact candidate
(`cargo build --release --locked -p rvoip-bench-uas`) and runs the lane
orchestrator the benchmark work owns:

```sh
python3 scripts/bench/run_lane.py --profile <p> --candidate <sha> \
  --receipt <path> --dut-bin <built binary> \
  --run-id bench-<p>-<sha12>-<github run id> --max-wall <minutes> --resume
```

`--caller-bin` and `--ea-uas-bin` are omitted: they are pinned by the env-id,
not by the candidate, and run_lane fetches and hash-checks them itself
(requested of the benchmark work).

`release-qualify.yml` runs the gate in the `gate-bench` job (`ubuntu-latest`,
350 minutes, the provisioner role through OIDC, a 330-minute `--max-wall`).
`bench-release` (about 3.5 h) fits in that one job. `bench-minor` (about
11 h) does not, so two earlier jobs, `bench-leg-1` and `bench-leg-2`, call
the wrapper with `--advance` to continue the same lane: same run id,
`--resume`, a fresh role session and a 330-minute budget each. The final
`gate-bench` call finds the lane complete or nearly so and, through
`gates.py run-shard`, records the gate receipt. A lane that finishes early
makes the later calls cheap: run_lane, resuming a complete run id, only
rewrites its receipt (requested of the benchmark work).

Environment available to `run_lane.py`:

| Variable | Meaning |
|---|---|
| AWS credentials | the provisioner role session (6 h) |
| `RVOIP_AWS_REGION` | region |
| `RVOIP_AWS_EVIDENCE_BUCKET` | evidence bucket |
| `RVOIP_AWS_SUBNET_ID`, `RVOIP_AWS_SECURITY_GROUP_ID`, `RVOIP_AWS_RUNNER_INSTANCE_PROFILE` | the release worker network and profile |
| `RVOIP_RELEASE_RUN_ID` | `<github run id>-<attempt>`, for tags |

Exit codes (run_lane's, passed through):

| Code | Meaning | Release tooling |
|---|---|---|
| 0 | PASS or WARN (the receipt says which) | gate passes |
| 1 | FAIL | gate fails |
| 2 | usage or input error | gate fails, not retried |
| 3 | INFRA: quota, capacity, lane lock, tester invalid three times | retried once (`retry_on_exit_codes: [3, 75]`) |
| 4 | stopped at `--max-wall` | an earlier leg: progress; the final gate call: fails, rerun the job |

Only one lane runs per account at a time (an S3 lock in run_lane).

## Receipt schema (`rvoip-bench-lane-receipt-v1`)

The benchmark work owns the schema. The collector (`gates.py collect`)
rejects the gate unless the receipt has the schema, the gate's profile
(`release` or `minor`), `candidate_sha` equal to the gate receipt's
candidate, and `status` `PASS` or `WARN`. WARN, a likely improvement with a
proposed new R in `recommendation`, passes the gate; the receipt's `reasons`
say why. The rest of the receipt is for the reports, for example:

```json
{
  "schema": "rvoip-bench-lane-receipt-v1",
  "profile": "release",
  "candidate_sha": "<40-hex commit measured>",
  "status": "PASS | WARN | FAIL",
  "reasons": ["R1: 1.05 x R trial passed; record a new R after review"],
  "started_at": "<ISO 8601 UTC>",
  "ended_at": "<ISO 8601 UTC>",
  "environment": {
    "env_id": "<instance types + AMI + caller build, the baseline key>",
    "dut_instance_type": "c8i.4xlarge",
    "caller_instance_type": "c8in.24xlarge",
    "caller_count": 2,
    "ami_id": "ami-...",
    "caller_build_sha256": "<64-hex>",
    "dut_build_sha256": "<64-hex>"
  },
  "baseline": {
    "path": "crates/sip/rvoip-sip/perf-baselines/rfc7502/<env-id>/<file>.json",
    "sha256": "<64-hex>"
  },
  "rows": [
    {
      "id": "R1",
      "transport": "tls",
      "media": "none",
      "hold_seconds": 0,
      "recorded_rate_cps": 5384,
      "result": "PASS | WARN | FAIL",
      "trials": [
        {
          "rate_factor": 0.95,
          "rate_cps": 5114,
          "duration_seconds": 180,
          "valid": true,
          "attempts": 920520,
          "failures": 0,
          "result": "PASS",
          "evidence_uri": "s3://<bucket>/release/<run>/bench/R1/t1.tar.gz",
          "evidence_sha256": "<64-hex>"
        }
      ],
      "latency": {
        "rate_factor": 0.9,
        "srd_ms": {"p50": 0.0, "p99": 0.0, "p99_9": 0.0},
        "sdd_ms": {"p50": 0.0, "p99": 0.0, "p99_9": 0.0},
        "tolerance_pct": 25,
        "floor_ms": 10,
        "result": "PASS"
      }
    }
  ],
  "deviations": [
    "10 s establishment threshold",
    "warm-up at r/2, unscored",
    "fresh DUT process per trial",
    "rate quantum 4",
    "503 with Retry-After counted as DUT failure"
  ],
  "report": {"path": "rfc7502-report.md", "sha256": "<64-hex>"}
}
```

`report` is required for the `minor` profile and is attached to the GitHub
release.

## Reuse

The bench gates hash `Cargo.toml`, `Cargo.lock`, `crates/foundation/**`,
`crates/media/**`, `crates/sip/**`, `crates/tools/**`, `scripts/bench/**`
`infra/aws/bench/**` and `scripts/release/bench_gate.py`, plus what every gate hashes: `rust-toolchain.toml`,
`scripts/release/gates.py`, `.config/**`, `.github/workflows/**` and
`deny.toml`. Hashing is version-normalized, so the bump commit's
`release-patch` run reuses the candidate's `bench-release` receipt (and its
`bench-receipt.json`, staged by the collector). The benchmark therefore runs
once per release.

Anything else that lands on main between the bench run and the bump PR and
touches one of those paths (a workflow change counts) invalidates the bench
receipt, and `release-patch` reruns the lane. Hold such merges during
release day.

## Release-day sequence

1. Pick a main commit `C` whose nightly `remote-release` run passed.
2. Dispatch **Release qualification** with `profile=bench-release`,
   `candidate=C`, and at the same time **Prepare release PR** for `C`.
3. When both are green, merge the bump PR (its head `B` is `C` plus the
   version bump).
4. Dispatch **Release qualification** with `profile=release-patch`,
   `candidate=B`, `first_candidate=false`,
   `prior_run_id=<nightly run>,<bench run>`. Only the bookkeeping gates run.
5. Publish `B`.
