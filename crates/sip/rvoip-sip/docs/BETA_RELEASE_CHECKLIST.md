# rvoip-sip Beta Release Checklist

Date: 2026-06-16

This checklist is evidence-backed. Checked rows are covered by the latest beta
report selected by `crates/sip/rvoip-sip/beta-report/latest.txt` or by current
repository files.

Current reference report:

- `crates/sip/rvoip-sip/beta-report/20260616T014649Z/summary.md`
- Result: `0` failures, `0` skips
- Git revision: `2bd8c570`
- Git status at run time: `dirty`
- Rust/Cargo: `1.95.0`
- Current release train and runtime crate version: `0.2.2`

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
- [x] Latest `scripts/beta_gate.sh --full --require-external` report passed
  with `0` failures and `0` skips.
- [ ] Clean-tree publish attestation requires a fresh full gate from a clean
  workspace if that evidence is needed for release signoff.

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
- [x] PBX matrix covers G.729A and G.729AB audio analyzer rows for Asterisk
  and FreeSWITCH.
- [x] baresip strict-UA matrix passes in the reference full gate.
- [x] Kamailio/OpenSIPS proxy audit records this topology as a non-claim.
- [x] Dependency advisory audit is archived in the reference report.
- [x] Parser fuzz smoke logs for SIP message, URI, header, and SDP parsing are archived in the reference report.

## Security Gates

- [x] Trace redaction covers authorization, proxy authorization, authentication challenges, cookies, identity headers, token-like headers, and SDP keying attributes.
- [x] `scripts/beta_gate.sh --security` runs dependency audit and parser fuzz smoke targets.
- [x] Fuzz targets exist for SIP message parsing, URI parsing, header parsing, and SDP parsing.
- [x] Reference dependency audit report has no unaccepted advisories.
- [x] Reference fuzz smoke report has no parser crashes.

## Performance Gates

- [x] Full-media 30 CPS reference point passes.
- [x] Full-media 100 CPS reference point passes.
- [x] Full-media 300 CPS reference point passes.
- [x] Full-media 1,000 CPS reference point passes.
- [x] Full-media 2,000 CPS reference point passes with at least 99.9% success in reference evidence.
- [x] 1-hour split soak passes with retained objects `0`, active Bob audio
  receivers `0`, transaction runners `0`, and post-drain RSS gate pass.
- [x] Overload/recovery scenario passes in the reference full gate.
- [x] Any result above 2,000 CPS is labeled as tuned or experimental.
- [x] 24-hour release-candidate soak is explicitly waived for beta; the
  1-hour split soak is accepted as the beta release bar.
- [ ] Three consecutive `perf_call_setup_2k_profile.sh clean` manifests pass
  the current absolute acceptance and relative-audit gates from one source
  fingerprint.
- [ ] The final beta report imports those exact three run directories under
  `canonical-2k/` and its end-of-gate source fence remains unchanged.

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
# First run this three times without changing the source tree. Record each
# printed target/perf-results/profiles/<run> artifact directory.
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh clean

RVOIP_STRICT_UA_HOST_IP=<local-host-ip> \
BETA_REPORT_PACKAGE=1 \
BETA_REQUIRE_CANONICAL_2K_EVIDENCE=1 \
BETA_CANONICAL_2K_RUN_DIRS="<oldest-run>:<middle-run>:<newest-run>" \
BETA_RUN_LOCAL_PBX=1 \
BETA_PBX_PROVIDER=both \
BETA_PBX_API=all \
BETA_PBX_SCENARIO=all \
BETA_PBX_G729_PROFILES="g729a g729ab" \
crates/sip/rvoip-sip/scripts/beta_gate.sh --full --require-external
```

For a literal-all performance qualification rather than the standard beta
performance subset, add the following switches. This runs every registered
performance/resiliency target, all configured burst scenarios, and both the
split and monolithic long soaks. The isolated media-churn diagnostic defaults
to 120 seconds, the legacy monolithic soak to 1800 seconds, and the split soak
to `RVOIP_PERF_SOAK_DURATION_SECS` (3600 seconds in the beta gate). Their
duration controls are deliberately independent:

```sh
BETA_RUN_PERF_ALL=1 \
BETA_RUN_BURST_MATRIX=1 \
BETA_BURST_MATRIX=all \
BETA_RUN_LONG_SOAK=1 \
BETA_PERF_MEDIA_CHURN_DURATION_SECS=120 \
BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS=1800 \
RVOIP_PERF_SOAK_DURATION_SECS=3600 \
crates/sip/rvoip-sip/scripts/beta_gate.sh --full --require-external
```

Optional external SIPp target command:

```sh
BETA_RUN_SIPP=1 \
BETA_SIPP_TARGET_HOST=<host> BETA_SIPP_TARGET_PORT=<port> \
crates/sip/rvoip-sip/scripts/beta_gate.sh --interop
```

Required release evidence from each interop/perf/security run:

- `summary.md` at the beta-gate artifact root.
- `canonical-2k/index.json` and its three read-only `run-N/` copies.
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

- Latest full beta gate passed with `0` failures and `0` skips.
- The latest gate recorded `git_status: dirty`; a clean-tree rerun is still
  required for publish attestation if release policy requires that evidence.
- Dependency audit is archived with no unaccepted advisories. Allowed warnings
  remain documented separately.
- Parser fuzz smoke logs are archived for all four parser targets with no
  crashes.
- PBX interop matrix passed `234 / 234` rows across local Asterisk and local
  FreeSWITCH, including `12` G.729A/G.729AB analyzer rows.
- Release notes should cite the reference report directory and its dirty-tree
  caveat.

## Beta Soak Waiver

The 24-hour release-candidate soak is waived for beta on 2026-06-16. The beta
bar is the archived 1-hour split soak from
`crates/sip/rvoip-sip/beta-report/20260616T014649Z/perf-results/perf_soak_caller.json`
and
`crates/sip/rvoip-sip/beta-report/20260616T014649Z/perf-results/perf_soak_receiver.json`:
`9,898 / 9,898` caller calls succeeded, ASR was `1.0`, `500` active media
calls were held, retained objects after drain were `0`, active Bob audio
receivers after drain were `0`, transaction runners after drain were `0`, and
the post-drain RSS growth was `0.42 MB/hr` on the caller and `0.21 MB/hr` on
the receiver against the `10 MB/hr` threshold. A 24-hour soak remains
recommended before a broader production-readiness claim.
