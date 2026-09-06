# Beta Release Policy and Checklist

This document defines the promotion procedure. It does not duplicate a
candidate's results. The versioned reporting and selection authority is
`config/beta-release-policy.yaml`; current outcomes are generated from evidence:

Current workspace runtime crate version: `0.3.9`.
Current qualified runtime crate version: `0.3.9`.
Next planned candidate runtime crate version: `0.3.10`.
Its protected remote-release run passed 208/208 gates and covered all 108
requirements in the strict legacy ledger. The historical `0.3.2` exception
path did not qualify it.

- [Beta Release Candidate Report](BETA_RELEASE_REPORT.md)
- [Complete Beta Gate Report](BETA_GATE_REPORT.md)
- [Beta Performance Report](BETA_PERFORMANCE_REPORT.md)
- [0.3.2 owner-approved release exception](BETA_RELEASE_EXCEPTION.md)
- [Immutable release history](releases/beta/README.md)
- [Protected qualification history](releases/qualification/README.md)

Verify the currently promoted protected reports with:

```sh
python3 scripts/release/render_qualification_reports.py verify \
  --directory crates/sip/rvoip-sip/docs
```

## Promotion rule

A beta release candidate requires a `full` run that:

- starts from a clean source tree and ends with the identical source
  fingerprint;
- has every gate selected by the effective typed configuration recorded
  exactly once as `PASS`;
- has zero failed and zero skipped gates;
- includes the required unit, integration, doctest, example, downstream,
  security, PBX, SIPp, strict-UA, performance, resiliency, soak, regression,
  evidence-integrity, and final source-fence scopes;
- records peer versions/configuration hashes, build/runtime configuration,
  commands, timestamps, validators, evidence paths, and SHA-256 hashes;
- passes the packaged strict attestation verifier with clean-source,
  unchanged-source, no-skip, pass, and mode-eligibility requirements;
- contains three fresh canonical 2,000-CPS passes produced from the exact
  candidate, plus the current performance evaluation JSON, Markdown report,
  and SHA-256 artifact index; and
- generates and verifies the evidence-complete release reports before a
  successful full-run pointer can update.

The focused security-only diagnostic remains:

```sh
crates/sip/rvoip-sip/scripts/beta_gate.sh --security
```

Conditional gates are required whenever their enabling configuration schedules
them. They are never classified as optional or “additional” after execution.
Unknown, duplicate, ambiguous, uncatalogued, missing, or unvalidated gates fail
closed.

## Owner-approved exception path

The strict promotion rule above is unchanged. A project owner may separately
accept a bounded deviation only through a tracked exception attestation that:

- retains the source run's original `FAIL` / `NON-RC` status and every gate
  result;
- identifies the exact accepted deviation and approval basis;
- binds the decision, full gate inventory, and selected source evidence with
  SHA-256;
- passes the dedicated exception verifier; and
- is supplied explicitly to unified release verification.

For 0.3.2, the owner accepted the high-density full-media burst ASR result of
0.9928 against the 0.995 threshold. The adjacent reporting failure is a derived
roll-up of that same miss. All 106 other gates passed and no gate was skipped.
The immutable report is
[`20260729T010954Z/exception-r1`](releases/beta/20260729T010954Z/exception-r1/BETA_RELEASE_REPORT.md).

Verify the exception by itself:

```sh
python3 scripts/release_exception_attestation.py verify \
  --attestation crates/sip/rvoip-sip/docs/releases/beta/20260729T010954Z/exception-r1/exception-attestation.json \
  --version 0.3.2
```

Use it during the normal unified verification phase:

```sh
python3 scripts/release.py verify \
  --version 0.3.2 \
  --beta-exception-attestation crates/sip/rvoip-sip/docs/releases/beta/20260729T010954Z/exception-r1/exception-attestation.json
```

`--beta-exception-attestation` and the strict `--beta-report-root` input are
mutually exclusive. The verification receipt records which qualification mode
was used and the exception attestation's SHA-256.

## Required release configuration

The policy catalog contains the complete typed defaults and selection
conditions. The beta profile additionally fixes these release-critical values:

| Requirement | Release value |
|---|---:|
| Application audio-frame delivery | full (`RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0`) |
| Canonical 2K evidence | three clean, source-identical passes |
| High-density full-media phase | 160 CPS |
| High-density minimum ASR | 0.995 |
| RSS slope limit | 15 MB/hour |
| Monolithic soak | 3,600 seconds, 30 active full-media calls |
| Split soak | 3,600 seconds, 500 active full-media calls |
| Split-soak hold range | 10–360 seconds |
| Controlled drain | 10 CPS |
| Mass teardown | 500 calls at 30 setup CPS |

Changing a threshold or workload requires a reviewed policy change before the
run. Reporting may not reinterpret or silently relax recorded policy.

The July 2026 performance artifacts are regression baselines and historical
release evidence only. They cannot qualify a later candidate. Every promotion
must generate the three canonical passes and the complete performance matrix
from the exact source candidate being released.

## One-command full local invocation

Commit all intended release changes so the rvoip tree is clean, then run from
the rvoip workspace root:

```sh
crates/sip/rvoip-sip/scripts/full_beta_release.sh
```

The wrapper uses the exact Homebrew Docker/Compose paths, starts or repairs the
default Colima profile with the required resources and reachable network
address, validates every external dependency and both local PBX lab directories,
detects the host address for baresip, produces and validates three canonical
2K passes, and runs the complete fail-closed gate below. It does not fall back
to Docker Desktop, permit external skips, promote reports, or publish crates.
The wrapper may restart and persistently resize/reconfigure the default Colima
profile; it restores the previously selected Docker context when it exits.

Use `full_beta_release.sh --preflight-only` for a non-test environment check.

## Core gate settings reference

The wrapper is the sole executable authority for the full local run. The block
below preserves the core gate settings for policy review, but it is not a
complete environment-isolated equivalent and must not be launched by hand.
`full_beta_release.sh` additionally fixes tool paths, workload inputs,
tolerances, warning policy, reporting capture, and a clean inherited
environment.

```sh
: "${RVOIP_STRICT_UA_HOST_IP:?export a reachable strict-UA host IP}"
: "${BETA_CANONICAL_2K_RUN_DIRS:?export three canonical run directories}"

RVOIP_STRICT_UA_HOST_IP="$RVOIP_STRICT_UA_HOST_IP" \
RVOIP_REQUIRE_API_TOOLS=1 \
BETA_REPORT_PACKAGE=1 \
BETA_REQUIRE_CLEAN_SOURCE=1 \
BETA_REQUIRE_CANONICAL_2K_EVIDENCE=1 \
BETA_CANONICAL_2K_RUN_DIRS="$BETA_CANONICAL_2K_RUN_DIRS" \
BETA_RUN_LOCAL_PBX=1 \
BETA_RESTORE_LOCAL_PBX=1 \
BETA_PBX_PROVIDER=both \
BETA_PBX_API=all \
BETA_PBX_SCENARIO=all \
BETA_PBX_G729_PROFILES="g729a g729ab" \
BETA_RUN_SIPP=1 \
BETA_SIPP_CPS="30 100 300 1000 2000" \
BETA_SIPP_DIAGNOSTICS=0 \
BETA_RUN_STRICT_UA=1 \
BETA_RUN_FUZZ_SMOKE=1 \
BETA_RUN_PERF_ALL=1 \
BETA_PERF_REGRESSION_FAIL=1 \
BETA_PERF_REGRESSION_BASELINE_ROOT=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z \
BETA_PERF_REGRESSION_BASELINE_MANIFEST=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json \
BETA_RUN_BURST_SMOKE=1 \
BETA_RUN_BURST_MATRIX=1 \
BETA_BURST_MATRIX=all \
BETA_RUN_LONG_SOAK=1 \
BETA_PERF_MEDIA_CHURN_DURATION_SECS=120 \
BETA_PERF_MEDIA_CHURN_ACTIVE_CALLS=30 \
BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS=3600 \
BETA_PERF_MONOLITHIC_SOAK_ACTIVE_CALLS=30 \
RVOIP_PERF_SOAK_DURATION_SECS=3600 \
RVOIP_PERF_SOAK_ACTIVE_CALLS=500 \
RVOIP_PERF_SOAK_MIN_HOLD_SECS=10 \
RVOIP_PERF_SOAK_MAX_HOLD_SECS=360 \
RVOIP_PERF_SOAK_CPS=0 \
RVOIP_PERF_SOAK_DRAIN_CPS=10 \
RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=160 \
RVOIP_PERF_MASS_TEARDOWN_CALLS=500 \
RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS=30 \
RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0 \
RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15 \
crates/sip/rvoip-sip/scripts/beta_gate.sh --full --require-external
```

## Performance pass meaning

The selected performance gates must include the complete profile matrix,
canonical 2K evidence, high-density full-delivery burst, monolithic and split
soaks, mass teardown, media/session churn, resiliency workloads, and regression
audit. Relevant gates require:

- the configured ASR and workload thresholds;
- no disallowed signaling, media-setup, overload, teardown, or lifecycle
  errors;
- positive application audio-frame delivery where media is exercised;
- zero retained sessions, dialogs, media resources, receivers, timers, and
  transaction runners after drain where those metrics apply;
- RSS slope no greater than 15 MB/hour in the gate's defined measurement
  window;
- result/configuration/source/executable reconciliation.

The current-candidate reconciliation must emit
`current-performance-evaluation.json`,
`current-performance-evaluation.md`, and
`current-performance-artifact-index.json`. Missing, conflicting, inherited, or
unindexed performance artifacts fail the release.

PASS is bounded by the tested source, executable, host, topology, workload,
duration, peer versions, transports, codecs, and configuration. It is not a
claim about untested deployments.

## Evidence and reporting

Future runs write `effective-gate-config.json` and `gate-results.json` natively.
Markdown is display-only. A qualifying report package also contains the source
attestation, checksums, policy/generator inputs, commands and logs, environment
evidence, peer evidence, PBX/SIPp/strict-UA matrices, all performance JSON,
regression evidence, and generated release reports.

The reporting CLI is:

```sh
python3 crates/sip/rvoip-sip/scripts/beta_release_report.py generate \
  --report-root /path/to/report \
  --output-dir /path/to/generated

python3 crates/sip/rvoip-sip/scripts/beta_release_report.py verify \
  --generated-dir /path/to/generated

python3 crates/sip/rvoip-sip/scripts/beta_release_report.py promote-docs \
  --report-root /path/to/report
```

`promote-docs` first runs strict source verification, generates in a temporary
directory, verifies every generated binding, preserves an existing immutable
snapshot only when byte-identical, and then updates the stable current reports.

## Attestation scope

SHA-256 attestation supplies integrity and reproducibility evidence. It is not
third-party cryptographic signing. A report promotion is a post-run derivation
and does not change the tested candidate's commit identity.
