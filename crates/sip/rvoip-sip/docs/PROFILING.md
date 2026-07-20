# Profiling `rvoip-sip`

This guide is for finding a performance problem. Use
[`BENCHMARKING.md`](BENCHMARKING.md) to produce beta-comparable numbers and
[`TUNING.md`](TUNING.md) to select a supported runtime profile.

A profiler changes scheduling, CPU use, allocation behavior, or all three. A
profile is diagnostic evidence, never a release-gate result. Always pair it
with an unprofiled control built from the same source tree and Cargo features.

## Canonical 2,000-CPS reproduction

The exact in-process regression workload has one supported driver:

```bash
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh clean
```

For `clean`, it fixes the experiment at:

- release plus debug symbols, mimalloc, and only the `perf-tests` feature;
- shared-peer conditioning at 30, 100, 300, and 1,000 CPS, followed by the
  measured 2,000-CPS point;
- five-second ramp, 30-second steady state, five-second cooldown at every
  point, then 95 seconds of post-drain resource sampling at 2,000 CPS;
- eight Tokio workers and four Alice endpoint shards;
- Bob's `pbx-media-server` recipe with capacity 8,000;
- four round-robin UDP parse workers, two transaction workers, four dialog
  workers, four session-event workers, and transaction command capacity 128;
- diagnostics, generated-audio pacing, and the shared audio scheduler off.

The conditioning sequence is part of the measurement identity. The reviewed
2,000-CPS result was the fifth point of this exact shared-peer sweep, so a cold
single-point process is useful diagnostic evidence but is not beta-comparable.

The runner resolves `perf_call_setup_cps` from Cargo's
`compiler-artifact.executable` JSON message. It does not scan
`target/release/deps` or select a binary by modification time, because stale
feature variants may coexist there.

Four modes execute the same call shape:

| Mode | Tool | Use |
| --- | --- | --- |
| `clean` | direct executable | Acceptance control; retains the normal ASR gate |
| `cpu` | samply | CPU stacks and scheduler/context-switch analysis |
| `timing` | Instruments Time Profiler | macOS CPU, thread, and lock timing |
| `memory` | Instruments Allocations plus a four-phase retention timeline | macOS live-allocation and retained-state investigation |

Examples:

```bash
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh cpu
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh timing
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh memory
```

Diagnostic modes disable the ASR assertion so profiler overhead cannot end the
capture before the useful tail. The generated report still contains every
error and latency, so do not present it as a passing result.

Artifacts live under
`target/perf-results/profiles/<timestamp>_<mode>_<unique>/`. Each directory
contains:

- Cargo JSON messages and the exact executable path;
- the run log and profiler output;
- a v2 manifest with explicit `BUILD_ONLY`, `PASS`, or `FAIL` status, the test
  exit code, report, acceptance and relative-audit status, executable SHA-256,
  requested features, Cargo release profile, build-time and post-run source
  fingerprints, environment, effective
  configuration, conditioning identity, phase markers, requested versus actual
  RSS coverage, and sample counts;
- an immutable copy of the report produced by that invocation;
- for `clean`, `acceptance.json` plus a baseline-compatible
  `perf-results/<scenario>/2000.json` audit view and `perf-audit.md` comparison
  against the reviewed `20260706T181609Z` beta report.

The test writes its raw report beneath that run directory through
`RVOIP_PERF_OUTPUT_ROOT`; it never selects or overwrites a shared report from a
different profiling invocation. An executed run fails if its report is missing
or malformed, its source fingerprint changes between build, execution, and the
fresh post-run finalization capture, or its required provenance fields are
absent. A clean run also fails if its load,
allocator, Cargo features, Bob/Alice recipe, runtime switches, or exact 65,000
call count differ from the canonical identity. Compiler, release-profile, and
mimalloc environment overrides are rejected before Cargo starts.

Set `RVOIP_PERF_PROFILE_BUILD_ONLY=1` to validate the build and exact artifact
resolution without running 65,000 calls. Its manifest is `BUILD_ONLY`, never a
passing acceptance result.

## Reproducibility fields

Perf JSON now distinguishes dirty builds at the same commit. Its environment
block records:

- short and full Git revisions;
- whether the source tree was dirty;
- a SHA-256 over HEAD, tracked changes, and untracked source bytes;
- active rvoip-sip Cargo features and the runner-requested feature string;
- allocator, release profile, Rust version, OS, CPU, and RAM.

The call-setup diagnostics block also records:

- the bundled or external recipe SHA-256;
- an allowlisted snapshot of relevant runtime switches;
- the complete effective Bob and Alice configurations;
- configured and effective transaction priority/INVITE-2xx defaults;
- planned ramp/steady/cooldown boundaries and actual dispatch, drain, and
  sampler-stop markers;
- in `memory` mode, retention snapshots at ramp end, steady end, cooldown end,
  and a full 64-second fence after both call drain and cooldown capture. Those
  scans and the retention wait are absent from clean controls.

For conditioning-overlap investigations, run
`scripts/perf_call_setup_2k_profile.sh boundary` (equivalent to setting
`RVOIP_PERF_BOUNDARY_SNAPSHOT=1` on the normal shared-peer sweep). It captures
one endpoint/allocator snapshot immediately after each point's calls drain and
then continues without waiting out the anti-reuse horizon. This preserves the
same overlapping 64–90 second retention pattern as the canonical sweep. It is
diagnostic evidence only and is always disabled for clean acceptance runs.

Clean mode instead keeps the lightweight process RSS sampler running for 95
seconds after the measured calls drain. It computes the legacy-compatible
active slope only from `point_start` through `calls_drained`, computes a
separate diagnostic post-drain cleanup slope, and computes a robust retained
RSS delta from median endpoint bands. Requested cleanup coverage is exactly the
configured 95 seconds; actual sample coverage is reported separately. The
short-window RSS gate normalizes the robust delta by the actual separation
between the endpoint bands' median sample timestamps and enforces the
unadjusted 10 MB/hour intent, while the structural snapshot must still
converge exactly to zero. Sampling stops before that one compact
structural-convergence snapshot, and no endpoint retention scan runs inside the
active clean measurement. The 30-minute and one-hour soaks remain authoritative
for the same unadjusted 10 MB/hour slope.

Compare artifacts only when these fields describe the experiment you intended.
A dirty source fingerprint is valid diagnostic evidence, but it must match
between the control and candidate if the comparison is supposed to isolate a
runtime switch.

## CPU profiling with samply

Install `samply` and run `samply setup` once on macOS if attach/stack capture
requires it. The canonical driver records at 1,000 Hz and emits presymbolicated
sidecar data:

```bash
RVOIP_PERF_PROFILE_SAMPLY_RATE=1000 \
  crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh cpu
```

Add context-switch markers only for a focused scheduler investigation; they
increase capture overhead:

```bash
RVOIP_PERF_PROFILE_CSWITCH_MARKERS=1 \
  crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh cpu
```

For a smaller, continuously looping target, use one of the profiling examples:

```bash
cargo build --profile flamegraph -p rvoip-sip \
  --example profiling_call_setup_loop
samply record --save-only --output call-setup.json.gz \
  target/flamegraph/examples/profiling_call_setup_loop
```

Other targets are:

- `profiling_parser_loop` for SIP parse/serialize CPU cost;
- `profiling_registration_storm` for registrar and authentication churn;
- `profiling_dialog_steady_state` for lookup/lock cost at a retained backlog.

`RVOIP_PROFILE_DURATION` controls their run length. The dialog target also
accepts `RVOIP_PROFILE_BACKLOG`, and registration accepts
`RVOIP_PROFILE_FANOUT`.

## macOS timing and memory tools

The `timing` mode launches the exact Cargo artifact under the Instruments Time
Profiler. The `memory` mode uses the Allocations template and captures endpoint
retention at ramp end, steady end, cooldown end, and after the complete
64-second identifier anti-reuse fence. Clean mode never creates this task.

The production allocator is mimalloc. Instruments, `heap`, `leaks`, and
`malloc_history` describe macOS allocator-visible behavior and VM regions; they
do not automatically attribute every mimalloc-internal byte to a Rust call
site. Use them with the JSON RSS series and rvoip retention counters.

For a long split caller/receiver capture, use:

```bash
crates/sip/rvoip-sip/scripts/perf_soak_profile_receiver.sh
```

For `vmmap`, `heap`, `leaks`, and `malloc_history` collection around the split
receiver, use:

```bash
crates/sip/rvoip-sip/scripts/perf_soak_malloc_profile_receiver.sh
```

Allocator substitutions are diagnostic A/Bs. `perf-system-allocator` and
`dhat` change allocation behavior and must not be compared directly with the
mimalloc beta threshold.

## DHAT allocation profiles

DHAT instruments every allocation and intentionally uses its own global
allocator. Run the smallest target that represents the suspected layer:

```bash
cargo run --release --features dhat -p rvoip-sip \
  --example profiling_dhat_parse
cargo run --release --features dhat -p rvoip-sip \
  --example profiling_dhat_udp
cargo run --release --features dhat -p rvoip-sip \
  --example profiling_dhat_dialog
cargo run --release --features dhat -p rvoip-sip \
  --example profiling_dhat_b2bua
```

Each writes `dhat-heap.json` in the working directory. Do not enable `dhat`
alongside the normal mimalloc acceptance configuration.

## Criterion microbenchmarks

Use Criterion after the system profile identifies a specific structure or
operation. It answers whether a candidate implementation improves that hot
path; it does not prove whole-call behavior.

```bash
cargo bench -p rvoip-sip --bench call_setup
cargo bench -p rvoip-sip --bench dialog_steady_state
cargo bench -p rvoip-sip --bench session_lookup_create
```

`session_lookup_create` includes uncontended and contended session lookup,
create/remove, registry, and key-representation comparisons. Preserve its
baseline before changing `SessionStore` or its indexes.

## Tokio task and wait analysis

The registration and steady-dialog examples have an opt-in Tokio Console hook:

```bash
RUSTFLAGS="--cfg tokio_unstable" \
  cargo run --profile flamegraph -p rvoip-sip \
  --features tokio-console --example profiling_dialog_steady_state
```

Use this to distinguish CPU work from task wakeups, channel backpressure, and
mutex wait time. This build is diagnostic and is not feature-equivalent to the
clean beta binary.

## Investigation workflow

1. Run `clean` and retain its manifest and JSON.
   The driver also writes `acceptance.json` and exits non-zero unless the
   canonical conditioning/window identity, actual coverage, structural drain,
   absolute 2,000-CPS beta thresholds, and
   zero-error condition pass. It also runs `perf_audit.py --fail-on-regression`
   against the reviewed `20260706T181609Z` baseline and records that status in
   the manifest.
2. Run one profiler mode from the identical source fingerprint.
3. Identify the largest inclusive stacks, waits, retained structures, or task
   populations. Do not tune from a single leaf frame.
4. Make one isolated library change.
5. Run focused correctness tests and then `clean` again.
6. Inspect the automatically generated comparison. To reproduce it manually:

   ```bash
   RUN_DIR="$(ls -dt target/perf-results/profiles/*_clean_* | head -1)"
   CURRENT_RESULTS="$(cat "${RUN_DIR}/audit-results-dir.txt")"
   python3 crates/sip/rvoip-sip/scripts/perf_audit.py \
     --baseline crates/sip/rvoip-sip/beta-report/20260706T181609Z/perf-results \
     --current "${CURRENT_RESULTS}" \
     --out "${RUN_DIR}/perf-audit.md" \
     --fail-on-regression
   ```

   The auditor treats an empty baseline, no shared scenario path, zero
   comparable metrics, or a conditioning/window identity mismatch as a hard
   refusal rather than an `OK` result. `NON_COMPARABLE` means no scalar claim
   was made. The reviewed older sweep is accepted only through a documented
   inference from its complete ordered sweep and per-point call counts.

7. Profile again only if the clean control still misses a gate.

Keep the established runtime defaults during library recovery: four UDP parse
workers, two transaction workers, four dialog workers, command capacity 128,
ACK+BYE priority burst 64, INVITE-2xx maintenance budget 2,048, and generated
audio pacing/shared scheduling disabled. A switch may be tested as a separate
experiment, but it must not hide a library regression.

## Reading phase and retention evidence

Every resource report exposes requested and actual tail seconds, sample count,
and completeness. Phase-selected windows additionally record their phase names,
first/last sample, actual coverage, and slope. Clean mode's release threshold is
the active-load slope (`point_start` through `calls_drained`), not a silently
truncated 60-second tail. It then samples the measured point for 95 seconds
after drain and takes a compact structural snapshot only after sampling stops.

`memory` mode separately captures endpoint state at
ramp end, steady end, cooldown end, and after the full 64-second retention
fence. These are diagnostic allocations and are never accepted as headline
performance. The report records scheduled and actual capture times.

For time-series retention through an extended cooldown, use the split soak
harness. A short 2,000-CPS run identifies peak state and cleanup backlog; it
does not replace the final 30-minute monolithic or one-hour split soak.

## Troubleshooting

- Run from the workspace containing `Cargo.toml`; the scripts compute that
  path and do not rely on the caller's current directory.
- A profiler may require macOS developer-tool permissions. Verify a trivial
  `samply`/`xctrace` capture before interpreting an empty profile.
- Release builds contain DWARF and are unstripped by workspace policy. Do not
  switch to a stripped deployment profile for source-level investigation.
- Profiles and debug symbols are large. Remove old generated directories under
  `target/perf-results/profiles/` when disk space is tight; do not use a stale
  hashed executable from an earlier feature set.
- Port-bind failures usually mean another perf process is still active. Stop
  it and rerun rather than changing the canonical port or workload shape.
