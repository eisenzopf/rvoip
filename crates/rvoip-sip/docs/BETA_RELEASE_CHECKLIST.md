# rvoip-sip Beta Release Checklist

Date: 2026-05-25

## Documentation

- [ ] README claims match beta support levels.
- [ ] Use-case docs no longer reference stale crate names as release claims.
- [ ] Compatibility matrix is complete.
- [ ] RFC compliance matrix is complete.
- [ ] Topology profiles are complete.
- [ ] Interop CI plan has current commands and peer versions.
- [ ] Release notes include only verified claims.
- [ ] Security posture document is current.
- [ ] Performance report is populated.
- [ ] `scripts/beta_gate.sh --local` passes and its summary artifact is linked.
- [ ] `scripts/beta_gate.sh --full` has either passed or records external
      skips with explicit non-claim exclusions.

## Code and API

- [ ] Public API examples compile.
- [ ] Config validation rejects unsupported beta media/security combinations.
- [ ] Unsupported post-beta features fail clearly or are absent from public API.
- [ ] Feature flags are documented.
- [ ] MSRV and semver policy are documented.

## Test Gates

- [ ] Unit tests pass.
- [ ] Integration tests pass.
- [ ] Doctests pass.
- [ ] RFC 4475 torture tests pass or list documented exclusions.
- [ ] Generated message validation passes.
- [ ] SIPp scenarios pass.
- [ ] Asterisk matrix passes or lists non-claim exclusions.
- [ ] FreeSWITCH matrix passes or lists non-claim exclusions.
- [ ] PJSIP or baresip matrix passes.
- [ ] Fuzz smoke tests run.

Primary local command:

```sh
crates/rvoip-sip/scripts/beta_gate.sh --local
```

External release-gate command:

```sh
BETA_RUN_PBX=1 BETA_RUN_SIPP=1 \
BETA_SIPP_TARGET_HOST=<host> BETA_SIPP_TARGET_PORT=<port> \
crates/rvoip-sip/scripts/beta_gate.sh --full
```

Local PBX lifecycle-managed interop command:

```sh
BETA_RUN_LOCAL_PBX=1 crates/rvoip-sip/scripts/beta_gate.sh --interop
```

Required release evidence from each interop/perf run:

- `summary.md` at the beta-gate artifact root.
- `environment/environment.md` and Docker snapshots under
  `environment/docker-<phase>/`.
- `pbx/summary.md` and `pbx/matrix.tsv` for PBX runs.
- SIPp `run_summary.md`, `runs.tsv`, `analysis.md`, stat CSVs, screen logs,
  and error logs for load runs.
- Raw failure logs or packet captures for any beta-blocking failure.

## Performance Gates

- [ ] Full-media 30 CPS passes.
- [ ] Full-media 100 CPS passes.
- [ ] Full-media 300 CPS passes.
- [ ] Full-media 1,000 CPS passes.
- [ ] Full-media 2,000 CPS passes with at least 99.9% success.
- [ ] 24-hour soak passes.
- [ ] Overload/recovery scenario passes.
- [ ] Any result above 2,000 CPS is labeled as tuned or experimental.
