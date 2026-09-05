# RVoIP 0.3.9 Release Qualification Report

> Generated from the protected `remote-release` run [33969263241](https://github.com/eisenzopf/rvoip/actions/runs/33969263241). No gate was rerun and no measurement was edited during report generation.

## Qualification

| Field | Value |
|---|---|
| Status | **PASS — RELEASE-CANDIDATE** |
| Workspace release | `0.3.9` |
| Tested commit | `8cab44b10f872d21b304c02111d5d203ee8226da` |
| Profile | `remote-release` |
| Gates | **208/208 passed** |
| Fresh / reused | `208` / `0` |
| Legacy release requirements | **108/108 covered** |
| Run window | `2026-09-05T13:35:31+00:00` to `2026-09-05T15:27:32+00:00` |
| Environment | `rvoip-release-v5-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-n2-cascade-lake` |
| Evidence artifact | `9972005366` / `sha256:0b5dd80b42be87b0823bba9224a983db4be855712e2f463d6849fb2d4f21b051` |

The [complete gate record](BETA_GATE_REPORT.md), [performance observations](BETA_PERFORMANCE_REPORT.md), and [machine summary](QUALIFICATION_SUMMARY.json) are derived from the same accepted receipts.

## Category totals

| Category | Passed |
|---|---:|
| Build, API, and documentation | 44 |
| PBX and interoperability | 16 |
| Parallel 45-crate core | 45 |
| Performance and resiliency | 29 |
| Remote release framework | 55 |
| Reporting and regression | 4 |
| Security | 11 |
| Source integrity | 4 |

## Evidence integrity

- Gate catalog: `c71e60d8d900e45ba52dd7ab4d85a08d0ac9cbccd7977c8e13e55978659d80bb`
- Qualification plan: `0263177edeb7b0c95e81a1bbcdf85b3cceeb4896971aa7588d047da5aed95671`
- Qualification aggregate: `ca0c1331d6b06064a14452c0bf3f5a56bee931fc6ac53e80d89e32995684873c`
- GitHub artifact archive: `sha256:0b5dd80b42be87b0823bba9224a983db4be855712e2f463d6849fb2d4f21b051`
- Every fresh gate receipt and command log was rehashed before rendering; any reused receipt remains explicitly identified and was input-bound by the qualification collector.
- Every published performance row is bound to the tested commit, a clean tree, and rvoip-sip at the release version.

## Claim boundary

PASS applies only to the exact source commit, gate catalog, commands, feature bundles, peer images, environments, limits, and measurements recorded by this run. It is not a general carrier certification or a performance SLA. Production remote-endpoint NAT/TLS/SDES qualification remains separately tracked until live two-UA evidence is recorded.
