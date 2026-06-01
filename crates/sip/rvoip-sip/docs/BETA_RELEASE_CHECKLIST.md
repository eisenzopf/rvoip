# rvoip-sip Beta Release Checklist

Date: 2026-05-26

This checklist is evidence-backed. Checked rows are covered by the final clean
beta report or by current repository files.

Current reference report:

- `crates/sip/rvoip-sip/beta-report/20260526T221457Z/summary.md`
- Result: `0` failures, `0` skips
- Git revision: `865430d4`
- Git status at run time: `clean`
- Rust/Cargo: `1.88.0`

## Documentation

- [x] README claims match beta support levels.
- [x] Use-case docs no longer reference stale crate names as release claims.
- [x] Compatibility matrix is complete for beta claims and post-beta non-claims.
- [x] RFC compliance matrix is evidence-linked for claimed behavior.
- [x] Topology profiles are complete for beta scope and non-claims.
- [x] Interop CI plan has current commands and peer versions.
- [x] Release notes include only verified beta-scope claims and explicit non-claims.
- [x] Security posture document is current and records completed security gates.
- [x] Performance report is populated with current reference values.
- [x] `scripts/beta_gate.sh --local` passed as part of the reference full gate.
- [x] Final `scripts/beta_gate.sh --full --require-external` report comes from a clean commit.

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
- [x] Dependency advisory audit is archived in the final report.
- [x] Parser fuzz smoke logs for SIP message, URI, header, and SDP parsing are archived in the final report.

## Security Gates

- [x] Trace redaction covers authorization, proxy authorization, authentication challenges, cookies, identity headers, token-like headers, and SDP keying attributes.
- [x] `scripts/beta_gate.sh --security` runs dependency audit and parser fuzz smoke targets.
- [x] Fuzz targets exist for SIP message parsing, URI parsing, header parsing, and SDP parsing.
- [x] Final dependency audit report has no unaccepted advisories.
- [x] Final fuzz smoke report has no parser crashes.

## Performance Gates

- [x] Full-media 30 CPS reference point passes.
- [x] Full-media 100 CPS reference point passes.
- [x] Full-media 300 CPS reference point passes.
- [x] Full-media 1,000 CPS reference point passes.
- [x] Full-media 2,000 CPS reference point passes with at least 99.9% success in reference evidence.
- [x] 30-minute soak passes with retained objects `0`, active Bob audio receivers `0`, and post-drain RSS gate pass.
- [x] Overload/recovery scenario passes in the reference full gate.
- [x] Any result above 2,000 CPS is labeled as tuned or experimental.
- [x] 24-hour release-candidate soak is explicitly waived for beta; the
  30-minute soak is accepted as the beta release bar.

## Commands

Primary local command:

```sh
crates/sip/rvoip-sip/scripts/beta_gate.sh --local
```

Security command:

```sh
crates/sip/rvoip-sip/scripts/beta_gate.sh --security
```

Final external release-gate command:

```sh
BETA_RUN_LOCAL_PBX=1 RUSTUP_TOOLCHAIN=1.88 crates/sip/rvoip-sip/scripts/beta_gate.sh --full --require-external
```

Optional external SIPp target command:

```sh
BETA_RUN_SIPP=1 \
BETA_SIPP_TARGET_HOST=<host> BETA_SIPP_TARGET_PORT=<port> \
crates/sip/rvoip-sip/scripts/beta_gate.sh --interop
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

## Completed Release Evidence

- Clean full beta gate passed with `0` failures, `0` skips, and
  `git_status: clean`.
- Dependency audit is archived with no unaccepted advisories. Allowed warnings
  remain documented separately.
- Parser fuzz smoke logs are archived for all four parser targets with no
  crashes.
- Release notes cite the clean report directory.

## Beta Soak Waiver

The 24-hour release-candidate soak is waived for beta on 2026-05-26. The beta
bar is the archived 30-minute soak from
`crates/sip/rvoip-sip/beta-report/20260526T221457Z/perf-results/perf_soak_30min.json`:
`35,109 / 35,109` calls succeeded, ASR was `1.0`, retained objects after drain
were `0`, active Bob audio receivers after drain were `0`, and the post-drain
RSS slope was `1.5 MB/hr` against the `10 MB/hr` threshold. A 24-hour soak
remains recommended before a broader production-readiness claim.
