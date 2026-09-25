# RVoIP 0.3.10 Release Qualification Report

> Generated from the protected `remote-release` run [34074372543](https://github.com/eisenzopf/rvoip/actions/runs/34074372543). No gate was rerun and no measurement was edited during report generation.

## Qualification

| Field | Value |
|---|---|
| Status | **PASS — RELEASE-CANDIDATE** |
| Workspace release | `0.3.10` |
| Tested commit | `77a99cd38a07641294cf7dc547146b115b135dc7` |
| Profile | `remote-release` |
| Gates | **213/213 passed** |
| Fresh / reused | `213` / `0` |
| Legacy release requirements | **108/108 covered** |
| Run window | `2026-09-07T01:53:20+00:00` to `2026-09-07T04:27:26+00:00` |
| Environment | `rvoip-release-v5-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-n2-cascade-lake` |
| Evidence artifact | `pending-upload` / `pending-upload` |

The [complete gate record](BETA_GATE_REPORT.md), [performance observations](BETA_PERFORMANCE_REPORT.md), and [machine summary](QUALIFICATION_SUMMARY.json) are derived from the same accepted receipts.

## Category totals

| Category | Passed |
|---|---:|
| Build, API, and documentation | 44 |
| PBX and interoperability | 16 |
| Parallel 45-crate core | 45 |
| Performance and resiliency | 29 |
| Remote release framework | 60 |
| Reporting and regression | 4 |
| Security | 11 |
| Source integrity | 4 |

## Evidence integrity

- Gate catalog: `bfbcfeee415fea34dd36701d81258b412c624a2c5757305dabad6342bec43531`
- Qualification plan: `ee35c1a066c09d15d55c5ab3f65fece4dd60e852035056c26f38c5f6ee42331d`
- Qualification aggregate: `95f715ec03675a1139da0fbb226b0b77b49721bc97116ba449c6aca8c2271ac6`
- GitHub artifact archive: `pending-upload`
- Every fresh gate receipt and command log was rehashed before rendering; any reused receipt remains explicitly identified and was input-bound by the qualification collector.
- Every published performance row is bound to the tested commit, a clean tree, and rvoip-sip at the release version.

## Claim boundary

PASS applies only to the exact source commit, gate catalog, commands, feature bundles, peer images, environments, limits, and measurements recorded by this run. It is not a general carrier certification or a performance SLA. Production remote-endpoint NAT/TLS/SDES qualification remains separately tracked until live two-UA evidence is recorded.
