# rvoip-sip Beta Performance Report

Date: 2026-06-16

This report summarizes the current beta-candidate performance evidence and the
claim policy for the next release. The current reference report is the latest
full beta gate selected by `crates/sip/rvoip-sip/beta-report/latest.txt`.
It passed all gates, but it ran from a dirty tree and is not a clean-tree
publish attestation.

Current reference report:

- Command: `RVOIP_STRICT_UA_HOST_IP=<local-host-ip> BETA_REPORT_PACKAGE=1 BETA_RUN_LOCAL_PBX=1 BETA_PBX_PROVIDER=both BETA_PBX_API=all BETA_PBX_SCENARIO=all BETA_PBX_G729_PROFILES="g729a g729ab" crates/sip/rvoip-sip/scripts/beta_gate.sh --full --require-external`
- Summary: `crates/sip/rvoip-sip/beta-report/20260616T014649Z/summary.md`
- Result: `0` failures, `0` skips
- Git revision: `2bd8c570`
- Git status at run time: `dirty`
- Environment: `crates/sip/rvoip-sip/beta-report/20260616T014649Z/environment/environment.md`
- Raw performance artifacts: `crates/sip/rvoip-sip/beta-report/20260616T014649Z/perf-results/`
- Current release train and runtime crate version: `0.2.2`

## Claim Policy

`rvoip-sip` beta has two performance profiles:

| Profile | Claim policy |
|---------|--------------|
| General full-media profile | The user-facing beta target is up to 2,000 CPS with media enabled under the documented setup. |
| Tuned high-scale profile | Results above 2,000 CPS require explicit tuning parameters, hardware notes, topology notes, and caveats. They are not the default general-user promise. |

Near-10,000 CPS results must be described as tuned or signaling-only unless
they pass the full-media beta profile with the same evidence bar.

## Reference Environment

| Field | Value |
|-------|-------|
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Cargo | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` |
| Host | Apple M3 Max, 128 GB RAM, macOS 26.2 / Darwin 25.2.0 |
| SIPp | SIPp standalone matrix artifacts under `sipp/` |
| Feature/config source | Bundled `config/performance-recipes.yaml` |
| PBX provider coverage | Local Asterisk and local FreeSWITCH |
| PBX G.729 coverage | G.729A and G.729AB analyzer rows passed for local Asterisk and local FreeSWITCH |
| Media mode | Full-media profile for beta claim rows unless marked otherwise |
| Security media mode | Tested SDES-SRTP interop in PBX matrix; DTLS-SRTP remains post-beta |

## General Full-Media Gate

The beta release gate requires the following sweep points:

- 30 CPS
- 100 CPS
- 300 CPS
- 1,000 CPS
- 2,000 CPS

Required result at the declared beta target:

- at least 99.9% successful call completion
- no unbounded memory growth
- no stuck sessions after drain
- p50/p95/p99 setup latency recorded
- teardown latency recorded
- CPU and memory reported
- exact configuration recorded

Reference sweep artifact:
`crates/sip/rvoip-sip/beta-report/20260616T014649Z/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.md`

| CPS target | Achieved CPS | Success rate | p50 setup | p95 setup | p99 setup | Full-cycle p99 | RSS delta | CPU | Errors |
|------------|--------------|--------------|-----------|-----------|-----------|----------------|-----------|-----|--------|
| 30 | 27.9 | 1.0000 | 11.9 ms | 13.5 ms | 15.4 ms | 128.8 ms | 222.6 MB | 13% | 0 |
| 100 | 92.8 | 1.0000 | 11.4 ms | 13.0 ms | 14.1 ms | 127.3 ms | 218.1 MB | 28% | 0 |
| 300 | 278.6 | 1.0000 | 11.3 ms | 12.1 ms | 12.5 ms | 125.8 ms | 249.4 MB | 62% | 0 |
| 1,000 | 928.6 | 1.0000 | 11.1 ms | 12.1 ms | 12.3 ms | 125.3 ms | 655.6 MB | 145% | 0 |
| 2,000 | 1,857.1 | 1.0000 | 11.1 ms | 12.1 ms | 12.3 ms | 125.8 ms | 1,158.9 MB | 208% | 0 |

The SIPp standalone matrix also reached the 2,000 CPS target with 30,000
successful calls and no failed calls:

- Artifact: `crates/sip/rvoip-sip/beta-report/20260616T014649Z/sipp/analysis.md`
- Target: 2,000 CPS
- Achieved: 2,000.0 CPS
- Success: 30,000 / 30,000 calls
- P95/P99 INVITE-to-200 OK latency: `<10 ms` / `<10 ms`
- Retransmissions: 0

## Soak Evidence

Reference soak artifact:
`crates/sip/rvoip-sip/beta-report/20260616T014649Z/perf-results/perf_soak_caller.json`
and
`crates/sip/rvoip-sip/beta-report/20260616T014649Z/perf-results/perf_soak_receiver.json`

| Duration | Offered | Succeeded | ASR | Held media calls | Peak RSS | Post-drain RSS gate | Retained objects after drain | Active Bob audio receivers after drain | Transaction runners after drain |
|----------|---------|-----------|-----|------------------|----------|---------------------|------------------------------|----------------------------------------|---------------------------------|
| 3,600 s | 9,898 | 9,898 | 1.0 | 500 | caller 157.3 MB / receiver 211.5 MB | caller 0.42 MB/hr / receiver 0.21 MB/hr against 10 MB/hr default | 0 | 0 | 0 |

The receiver observed `89,629,787` audio frames and completed `9,898` audio
receivers before the post-drain checks.

## PBX Interop and G.729 Audio Evidence

Reference PBX artifact:
`crates/sip/rvoip-sip/beta-report/20260616T014649Z/pbx/matrix.tsv`

The local PBX matrix passed `234 / 234` rows across local Asterisk and local
FreeSWITCH. It includes `12` G.729 analyzer rows covering G.729A and G.729AB
for the endpoint, stream-peer, and callback APIs. The G.729 call scenario ran
over UDP in this matrix; TLS coverage is present for the non-G.729 PBX
registration, hold/resume, ring-cancel, DTMF, reject, and blind-transfer rows.

The 1-hour split soak passed the current gate. The release checklist explicitly
waives the 24-hour release-candidate soak for beta and accepts this split-soak
artifact as the beta release bar. A 24-hour soak remains recommended before a
broader production-readiness claim.

## Other Reference Performance Gates

The reference report passed these additional gates:

| Gate | Evidence |
|------|----------|
| Endpoint call setup CPS | `summary.md`, `perf_call_setup_cps_endpoint.log` |
| PBX media server call setup CPS | `summary.md`, `perf_call_setup_cps_pbx-media-server.log` |
| Signaling-only high-performance setup CPS | `summary.md`, `perf_call_setup_cps_signaling-only-server-high-performance.log` |
| Registration throughput | `perf-results/perf_registration_throughput.json` |
| Concurrent active calls | `perf-results/perf_concurrent_active_calls.json` |
| RTP steady state | `perf-results/perf_rtp_steady_state.json` |
| Backpressure step | `perf-results/perf_backpressure_step.json` |
| Transport recovery | `perf-results/perf_transport_recovery.json` |
| Session churn leak | `perf-results/perf_session_churn_leak.json` |
| SIPp standalone matrix | `sipp/analysis.md`, `sipp/run_summary.md` |

## Completed Release Evidence

These numbers are usable as current beta evidence because the full gate passed
with dependency audit and parser fuzz smoke logs archived under
`crates/sip/rvoip-sip/beta-report/20260616T014649Z`. The run recorded
`git_status: dirty`; a clean-tree rerun is still required if publish
attestation must prove an unmodified workspace.
