# rvoip-sip Beta Performance Report

Date: 2026-05-25

This is the beta performance report template and claim policy. It does not
replace raw benchmark output. Raw output should remain under `target/` or the
CI artifact store; release notes should cite the summarized tables here.

The release-gate entry point is:

```sh
crates/rvoip-sip/scripts/beta_gate.sh --perf
```

For the full beta candidate, run:

```sh
BETA_RUN_LONG_SOAK=1 RVOIP_PERF_SOAK_DURATION_SECS=86400 \
crates/rvoip-sip/scripts/beta_gate.sh --perf
```

## Claim Policy

`rvoip-sip` beta has two performance profiles:

| Profile | Claim policy |
|---------|--------------|
| General full-media profile | The user-facing beta target is up to 2,000 CPS with media enabled under the documented setup. |
| Tuned high-scale profile | Results above 2,000 CPS require explicit tuning parameters, hardware notes, topology notes, and caveats. They are not the default general-user promise. |

Near-10,000 CPS results must be described as tuned or signaling-only unless
they pass the full-media beta profile with the same evidence bar.

## General Full-Media Gate

The beta release should pass:

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

## Required Report Fields

| Field | Required |
|-------|----------|
| Git revision | Yes |
| Rust version | Yes |
| OS and kernel | Yes |
| CPU model and core count | Yes |
| RAM | Yes |
| SIPp version | Yes |
| Peer versions | Yes |
| Feature flags | Yes |
| Media mode | Yes |
| Codec set | Yes |
| SRTP/TLS mode | Yes |
| Config tuning knobs | Yes |
| Raw artifact location | Yes |

## Result Table Template

| CPS target | Achieved CPS | Success rate | p50 setup | p95 setup | p99 setup | RSS delta | CPU | Retransmits | Cleanup |
|------------|--------------|--------------|-----------|-----------|-----------|-----------|-----|-------------|---------|
| 30 | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| 100 | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| 300 | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| 1,000 | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| 2,000 | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |

## Current Evidence Notes

- Existing tuning docs show signaling-only and tuned high-CPS investigations
  above the general beta target.
- Existing investigation docs also show normal media becoming the limiting path
  above roughly the low-thousands CPS range.
- The beta release should therefore lead with 2,000 CPS full-media evidence and
  keep higher results in a separate tuned-profile section.
