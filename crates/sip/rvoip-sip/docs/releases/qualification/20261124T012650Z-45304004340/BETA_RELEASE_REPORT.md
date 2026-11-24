# RVoIP 0.5.0 Release Qualification Report

> Generated from the protected `release-minor` run [45304004340](https://github.com/eisenzopf/rvoip/actions/runs/45304004340). No gate was rerun and no measurement was edited during report generation.

## Qualification

| Field | Value |
|---|---|
| Status | **PASS — RELEASE-CANDIDATE** |
| Workspace release | `0.5.0` |
| Tested commit | `2ae9fa49c5903a3e947a53609ee3f7d64c71a013` |
| Profile | `release-minor` |
| Gates | **221/221 passed** |
| Fresh / reused | `13` / `208` |
| Legacy release requirements | **108/108 covered** |
| Run window | `2026-11-24T01:25:46+00:00` to `2026-11-24T01:26:50+00:00` |
| Environment | `rvoip-release-v6-rust-1.95-nextest-0.9.140-prebuilt-perf-v2-lld-ec2-m5-perf16-soaklong16-interop16` |
| Evidence artifact | `14880422047` / `sha256:f2bb79f540d4030cbce6851582b3037bacd59d2630644407d8ea170a978bab6b` |

The [complete gate record](BETA_GATE_REPORT.md), [performance observations](BETA_PERFORMANCE_REPORT.md), and [machine summary](QUALIFICATION_SUMMARY.json) are derived from the same accepted receipts.

## Category totals

| Category | Passed |
|---|---:|
| Build, API, and documentation | 44 |
| PBX and interoperability | 16 |
| Parallel workspace core | 49 |
| Performance and resiliency | 31 |
| Remote release framework | 62 |
| Reporting and regression | 4 |
| Security | 11 |
| Source integrity | 4 |

## Evidence integrity

- Gate catalog: `f86eda843adf8a23b5d31f564b1dc305f9fb559334ba4ac4a8706da0ec21704f`
- Qualification plan: `072e09edd5601e4dc0678cc4d4959a9c9642857e3f60b452d7364a5b9549e873`
- Qualification aggregate: `2abd0bcf542c6b0ac118c10625461c47a9500c6427f64ab78d9364554c4a19d4`
- GitHub artifact archive: `sha256:f2bb79f540d4030cbce6851582b3037bacd59d2630644407d8ea170a978bab6b`
- Every fresh gate receipt and command log was rehashed before rendering; any reused receipt remains explicitly identified and was input-bound by the qualification collector.
- Every published performance row is bound to the tested commit, a clean tree, and rvoip-sip at the release version.

## Claim boundary

PASS applies only to the exact source commit, gate catalog, commands, feature bundles, peer images, environments, limits, and measurements recorded by this run. It is not a general carrier certification or a performance SLA. Production remote-endpoint NAT/TLS/SDES qualification remains separately tracked until live two-UA evidence is recorded.
