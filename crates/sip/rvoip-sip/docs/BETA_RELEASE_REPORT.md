# RVoIP 0.4.0 Release Qualification Report

> Generated from the protected `remote-release` run [38195041080](https://github.com/eisenzopf/rvoip/actions/runs/38195041080). No gate was rerun and no measurement was edited during report generation.

## Qualification

| Field | Value |
|---|---|
| Status | **PASS — RELEASE-CANDIDATE** |
| Workspace release | `0.4.0` |
| Tested commit | `f4d528eaabbac935bce9a6404d3ce4fdec35ed0f` |
| Profile | `remote-release` |
| Gates | **214/214 passed** |
| Fresh / reused | `122` / `92` |
| Legacy release requirements | **108/108 covered** |
| Run window | `2026-10-12T02:46:08+00:00` to `2026-10-12T04:20:41+00:00` |
| Environment | `rvoip-release-v6-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-ec2-m5-perf16-soaklong16-interop16` |
| Evidence artifact | `11726014831` / `sha256:2a97885fa36296e6d8ede353aad9c72ad4ed97801f8fdffaf770a6b8077e73fd` |

The [complete gate record](BETA_GATE_REPORT.md), [performance observations](BETA_PERFORMANCE_REPORT.md), and [machine summary](QUALIFICATION_SUMMARY.json) are derived from the same accepted receipts.

## Category totals

| Category | Passed |
|---|---:|
| Build, API, and documentation | 44 |
| PBX and interoperability | 16 |
| Parallel 46-crate core | 46 |
| Performance and resiliency | 29 |
| Remote release framework | 60 |
| Reporting and regression | 4 |
| Security | 11 |
| Source integrity | 4 |

## Evidence integrity

- Gate catalog: `00f259f0e757df92767164c561e6af5fcb3c95e1b7e417b06a0b6d0ed73e1204`
- Qualification plan: `deff2c19c99805248d04321802e56de10ac4cc32e46822ec39794b8c6385ccaf`
- Qualification aggregate: `82cabd724b7a34357385d28818544af01422394faddf2d0308ed78df4215c7a1`
- GitHub artifact archive: `sha256:2a97885fa36296e6d8ede353aad9c72ad4ed97801f8fdffaf770a6b8077e73fd`
- Every fresh gate receipt and command log was rehashed before rendering; any reused receipt remains explicitly identified and was input-bound by the qualification collector.
- Every published performance row is bound to the tested commit, a clean tree, and rvoip-sip at the release version.

## Claim boundary

PASS applies only to the exact source commit, gate catalog, commands, feature bundles, peer images, environments, limits, and measurements recorded by this run. It is not a general carrier certification or a performance SLA. Production remote-endpoint NAT/TLS/SDES qualification remains separately tracked until live two-UA evidence is recorded.
