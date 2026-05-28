# rvoip-sip Beta Performance Report

Date: 2026-05-26

This report summarizes the current beta-candidate performance evidence and the
claim policy for the next release. The current complete report is the final
clean beta release-candidate attestation for beta-scope performance claims.

Current reference report:

- Command: `BETA_RUN_LOCAL_PBX=1 RUSTUP_TOOLCHAIN=1.88 crates/rvoip-sip/scripts/beta_gate.sh --full --require-external`
- Summary: `crates/rvoip-sip/beta-report/20260526T221457Z/summary.md`
- Result: `0` failures, `0` skips
- Git revision: `865430d4`
- Git status at run time: `clean`
- Environment: `crates/rvoip-sip/beta-report/20260526T221457Z/environment/environment.md`
- Raw performance artifacts: `crates/rvoip-sip/beta-report/20260526T221457Z/perf-results/`

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
| Rust | `rustc 1.88.0 (6b00bc388 2025-06-23)` |
| Cargo | `cargo 1.88.0 (873a06493 2025-05-10)` |
| Host | Apple M3 Max, 128 GB RAM, macOS Darwin 25.2.0 |
| SIPp | SIPp standalone matrix artifacts under `sipp/` |
| Feature/config source | Bundled `config/performance-recipes.yaml` |
| PBX provider coverage | Local Asterisk and local FreeSWITCH |
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
`crates/rvoip-sip/beta-report/20260526T221457Z/perf-results/perf_call_setup_cps/_sweep.md`

| CPS target | Achieved CPS | Success rate | p50 setup | p95 setup | p99 setup | Full-cycle p99 | RSS delta | CPU | Errors |
|------------|--------------|--------------|-----------|-----------|-----------|----------------|-----------|-----|--------|
| 30 | 27.9 | 1.0000 | 11.8 ms | 12.8 ms | 13.2 ms | 126.8 ms | 201.2 MB | 16% | 0 |
| 100 | 92.8 | 1.0000 | 11.4 ms | 12.5 ms | 12.9 ms | 126.2 ms | 246.8 MB | 40% | 0 |
| 300 | 278.6 | 1.0000 | 11.2 ms | 12.2 ms | 12.6 ms | 125.7 ms | 358.8 MB | 80% | 0 |
| 1,000 | 928.6 | 1.0000 | 11.3 ms | 12.1 ms | 12.5 ms | 125.8 ms | 1,066.2 MB | 158% | 0 |
| 2,000 | 1,857.1 | 1.0000 | 11.1 ms | 12.2 ms | 13.0 ms | 126.2 ms | 1,541.6 MB | 201% | 0 |

The SIPp standalone matrix also reached the 2,000 CPS target with 30,000
successful calls and no failed calls:

- Artifact: `crates/rvoip-sip/beta-report/20260526T221457Z/sipp/analysis.md`
- Target: 2,000 CPS
- Achieved: 2,000.0 CPS
- Success: 30,000 / 30,000 calls
- P95/P99 INVITE-to-200 OK latency: `<10 ms` / `<10 ms`
- Retransmissions: 0

## Soak Evidence

Reference soak artifact:
`crates/rvoip-sip/beta-report/20260526T221457Z/perf-results/perf_soak_30min.json`

| Duration | Offered | Succeeded | ASR | Held media calls | Peak RSS | Post-drain RSS gate | Retained objects after drain | Active Bob audio receivers after drain | Transaction runners after drain |
|----------|---------|-----------|-----|------------------|----------|---------------------|------------------------------|----------------------------------------|---------------------------------|
| 1,800 s | 35,109 | 35,109 | 1.0 | 30 | 292.1 MB | 1.5 MB/hr against 10 MB/hr default | 0 | 0 | 0 |

The 30-minute soak passed the current gate. The release checklist explicitly
waives the 24-hour release-candidate soak for beta and accepts this 30-minute
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

These numbers are usable as final beta release evidence because the full gate
ran from a clean commit, included dependency audit and parser fuzz smoke logs,
and archived the report package under `crates/rvoip-sip/beta-report/20260526T221457Z`.
