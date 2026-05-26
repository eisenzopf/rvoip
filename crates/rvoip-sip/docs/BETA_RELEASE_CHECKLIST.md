# rvoip-sip Beta Release Checklist

Date: 2026-05-26

This checklist is evidence-backed. Checked rows are covered by the current
reference report or by current repository files. Unchecked rows remain release
blockers until a final clean report archives the evidence.

Current reference report:

- `crates/rvoip-sip/beta-report/20260526T032035Z/summary.md`
- Result: `0` failures, `0` skips
- Git revision: `d6e8beaa`
- Git status at run time: `dirty`

## Documentation

- [x] README claims match beta support levels.
- [x] Use-case docs no longer reference stale crate names as release claims.
- [x] Compatibility matrix is complete for beta claims and post-beta non-claims.
- [x] RFC compliance matrix is evidence-linked for claimed behavior.
- [x] Topology profiles are complete for beta scope and non-claims.
- [x] Interop CI plan has current commands and peer versions.
- [x] Release notes include only verified beta-scope claims and explicit non-claims.
- [x] Security posture document is current and identifies remaining release blockers.
- [x] Performance report is populated with current reference values.
- [x] `scripts/beta_gate.sh --local` passed as part of the reference full gate.
- [ ] Final `scripts/beta_gate.sh --full --require-external` report comes from a clean commit.

## Code and API

- [x] Public API examples compile in the reference full gate.
- [x] Config validation rejects unsupported beta media/security combinations or the docs mark them as post-beta non-claims.
- [x] Unsupported post-beta features fail clearly or are absent from public beta claims.
- [x] Feature flags are documented in `README.md`.
- [x] MSRV and semver policy are documented in `README.md`.
- [x] `rvoip-sip` declares the workspace Rust version in crate metadata.

## Test Gates

- [x] Format check passes in the reference full gate.
- [x] Unit tests pass in the reference full gate.
- [x] Integration tests pass in the reference full gate.
- [x] Doctests pass in the reference full gate.
- [x] Public examples compile in the reference full gate.
- [x] Rustdoc builds in the reference full gate.
- [x] RFC 4475 torture tests pass in the reference full gate.
- [x] Generated SIP message validation passes in the reference full gate.
- [x] SIP dialog generated validation passes in the reference full gate.
- [x] SIPp standalone matrix passes in the reference full gate.
- [x] Asterisk matrix passes in the reference full gate.
- [x] FreeSWITCH matrix passes in the reference full gate.
- [x] baresip strict-UA matrix passes in the reference full gate.
- [x] Kamailio/OpenSIPS proxy audit records this topology as a non-claim.
- [ ] Dependency advisory audit is archived in the final report.
- [ ] Parser fuzz smoke logs for SIP message, URI, header, and SDP parsing are archived in the final report.

## Security Gates

- [x] Trace redaction covers authorization, proxy authorization, authentication challenges, cookies, identity headers, token-like headers, and SDP keying attributes.
- [x] `scripts/beta_gate.sh --security` runs dependency audit and parser fuzz smoke targets.
- [x] Fuzz targets exist for SIP message parsing, URI parsing, header parsing, and SDP parsing.
- [ ] Final dependency audit report has no unaccepted advisories.
- [ ] Final fuzz smoke report has no parser crashes.

## Performance Gates

- [x] Full-media 30 CPS reference point passes.
- [x] Full-media 100 CPS reference point passes.
- [x] Full-media 300 CPS reference point passes.
- [x] Full-media 1,000 CPS reference point passes.
- [x] Full-media 2,000 CPS reference point passes with at least 99.9% success in reference evidence.
- [x] 30-minute soak passes with retained objects `0`, active Bob audio receivers `0`, and post-drain RSS gate pass.
- [x] Overload/recovery scenario passes in the reference full gate.
- [x] Any result above 2,000 CPS is labeled as tuned or experimental.
- [ ] 24-hour release-candidate soak passes, or this requirement is explicitly waived for beta.

## Commands

Primary local command:

```sh
crates/rvoip-sip/scripts/beta_gate.sh --local
```

Security command:

```sh
crates/rvoip-sip/scripts/beta_gate.sh --security
```

Final external release-gate command:

```sh
BETA_RUN_LOCAL_PBX=1 crates/rvoip-sip/scripts/beta_gate.sh --full --require-external
```

Optional external SIPp target command:

```sh
BETA_RUN_SIPP=1 \
BETA_SIPP_TARGET_HOST=<host> BETA_SIPP_TARGET_PORT=<port> \
crates/rvoip-sip/scripts/beta_gate.sh --interop
```

Required release evidence from each interop/perf/security run:

- `summary.md` at the beta-gate artifact root.
- `environment/environment.md` and Docker snapshots under
  `environment/docker-<phase>/`.
- `pbx/summary.md` and `pbx/matrix.tsv` for PBX runs.
- SIPp `run_summary.md`, `runs.tsv`, `analysis.md`, stat CSVs, screen logs,
  and error logs for load runs.
- `security/cargo-audit.txt` and `security/cargo-audit.json`.
- `security/fuzz/sip_message.log`, `security/fuzz/uri.log`,
  `security/fuzz/header.log`, and `security/fuzz/sdp.log`.
- Raw failure logs or packet captures for any beta-blocking failure.

## Final Release Blockers

- Clean full beta gate with `0` failures, `0` skips, and `git_status: clean`.
- Archived dependency audit with no unaccepted advisories. Current Rust 1.88
  short audit passes with zero vulnerabilities; allowed warnings remain
  documented separately.
- Archived parser fuzz smoke logs for all four parser targets.
- 24-hour soak artifact, or a deliberate checklist change accepting the
  30-minute soak as sufficient for beta.
- Final release notes updated to cite the clean report directory.
