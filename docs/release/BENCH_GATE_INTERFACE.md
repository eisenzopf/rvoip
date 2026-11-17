# Release benchmark gate interface (RFC 7501/7502)

Status: stub interface, 2026-11-16. The release tooling side is implemented.
The lane itself (`scripts/bench/`) is built separately; anything marked
**open** below is for that work to settle, and this file should be updated
when it does.

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

`release-qualify.yml` runs every `bench-lane` shard in the `gate-bench` job:
`ubuntu-latest`, 360-minute limit, with the provisioner role assumed through
OIDC. The job runs the standard gate executor:

```sh
python3 scripts/release/gates.py run-shard --candidate <sha> \
  --environment-id <RELEASE_ENVIRONMENT_ID> --gates perf.rfc7502-release \
  --output target/release-shard
```

which runs the catalogued command from the repository root:

```sh
python3 scripts/bench/run_lane.py --profile release|minor \
  --candidate <40-hex sha> --receipt <artifact_dir>/bench-receipt.json
```

Environment available to `run_lane.py`:

| Variable | Meaning |
|---|---|
| AWS credentials | the provisioner role session (6 h) |
| `RVOIP_AWS_REGION` | region |
| `RVOIP_AWS_EVIDENCE_BUCKET` | bucket for trial evidence; write under `release/<RVOIP_RELEASE_RUN_ID>/bench/` |
| `RVOIP_AWS_SUBNET_ID`, `RVOIP_AWS_SECURITY_GROUP_ID`, `RVOIP_AWS_RUNNER_INSTANCE_PROFILE` | the release worker network and profile (a lane may need its own; **open**) |
| `RVOIP_RELEASE_RUN_ID` | `<github run id>-<attempt>`, for tags and evidence prefixes |

Contract for `run_lane.py`:

- Exit `0` only when the lane passes (`status` `PASS` or `WARN`). Any other
  exit fails the gate. Exit `75` marks an infrastructure failure, which the
  executor retries once (`retry_on_exit_codes`).
- Always write the receipt, even on failure, before exiting.
- Write anything else (summaries, the rendered IETF report) into the same
  directory as the receipt; it becomes part of the gate's evidence artifact.
- Tag and terminate everything it creates. The janitor's
  `rvoip-expires-at` tag convention applies.
- Measure the candidate it is given and nothing else.

**Open:** `bench-minor` takes 10 to 12 hours and a hosted job stops at 6.
Either `run_lane.py` detaches (an EC2 lane controller that uploads the
receipt to S3, with `--detach` / `--attach <state-uri>` split across two
jobs), or bench-minor runs as its own workflow. Patch releases (about 3.5 h)
fit as is.

## Receipt schema (`rvoip-bench-lane-receipt-v1`)

The collector (`gates.py collect`) rejects the gate unless the receipt has
the schema, the gate's profile, the measured commit equal to the gate
receipt's candidate, status `PASS` or `WARN`, and an empty `failures`. The
rest of the schema is for the reports; field names below are the proposal.

```json
{
  "schema": "rvoip-bench-lane-receipt-v1",
  "profile": "release",
  "candidate_sha": "<40-hex commit measured>",
  "status": "PASS | WARN | FAIL",
  "failures": [],
  "warnings": ["R1: 1.05 x R trial passed; record a new R after review"],
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
and `infra/aws/bench/**`, plus what every gate hashes: `rust-toolchain.toml`,
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
