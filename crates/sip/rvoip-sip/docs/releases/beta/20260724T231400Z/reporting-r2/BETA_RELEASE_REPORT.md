# Beta Release Candidate Report

> Reporting derivation from verified package `20260724T231400Z`. No gate was rerun by report generation. The candidate identity remains the tested source, not a later reporting commit.

## Qualification

| Field | Value |
|---|---|
| Status | **RELEASE-CANDIDATE** |
| Package | `rvoip-sip 0.2.5` |
| Run | `20260724T231400Z` |
| Tested commit | `8d44fb3574e40f62526aa68f19833e95274cd06b` |
| Tested tree | `fa2e554396831daa54f41ccace9b0fc2a26cd9b9` |
| Source clean and unchanged | `True` |
| Gates | **PASS: 108/108 passed, 0 failed, 0 skipped** |
| Run window | `2026-07-24T23:14:00Z` to `2026-07-25T06:57:00Z` |

The complete gate-by-gate proof is in the [Beta Gate Report](BETA_GATE_REPORT.md). Performance observations are in the [Beta Performance Report](BETA_PERFORMANCE_REPORT.md).

## Category totals

| Category | Required | Passed |
|---|---:|---:|
| Build, API, and documentation | 44 | 44 |
| PBX and interoperability | 16 | 16 |
| Performance and resiliency | 29 | 29 |
| Reporting and regression | 4 | 4 |
| Security | 11 | 11 |
| Source integrity | 4 | 4 |

## Effective configuration

Values are typed. `environment-override` means the value was present in the redacted run environment; `policy-default` means it was supplied by the catalog; other v1 values were recovered from the source attestation.

| Key | Type | Value | Provenance |
|---|---|---|---|
| `beta_aggregate_skip_gate` | `boolean` | `false` | `policy-default` |
| `beta_attestation_features` | `string-list` | `["generated-validation","dev-insecure-tls","perf-tests","g729"]` | `derived-from-v1-attestation` |
| `beta_attestation_target` | `string` | `rustc-host` | `derived-from-v1-attestation` |
| `beta_burst_matrix` | `string` | `all` | `environment-override` |
| `beta_burst_scenario_file` | `path` | `bundled config/perf-burst-scenarios.yaml` | `derived-from-v1-attestation` |
| `beta_canonical_2k_run_dirs` | `path-list` | `["<workspace>/target/perf-results/profiles/20260724T221956Z_clean_KRwxGm","<workspace>/target/perf-results/profiles/20260724T224300Z_clean_GbSiae","<workspace>/target/perf-results/profiles/20260724T225757Z_clean_p5XNXK"]` | `environment-override` |
| `beta_deny_warnings` | `boolean` | `true` | `derived-from-v1-attestation` |
| `beta_gate_artifact_dir` | `path` | `<source-report>` | `environment-override` |
| `beta_gate_mode` | `enum` | `full` | `derived-from-v1-attestation` |
| `beta_gate_require_external` | `boolean` | `true` | `derived-from-v1-attestation` |
| `beta_pbx_api` | `enum` | `all` | `environment-override` |
| `beta_pbx_g729_profiles` | `string-list` | `["g729a","g729ab"]` | `environment-override` |
| `beta_pbx_provider` | `enum` | `both` | `environment-override` |
| `beta_pbx_scenario` | `string` | `all` | `environment-override` |
| `beta_perf_features` | `string-list` | `["perf-tests"]` | `derived-from-v1-attestation` |
| `beta_perf_high_density_burst_cps` | `integer` | `160` | `derived-from-v1-attestation` |
| `beta_perf_high_density_min_asr` | `number` | `0.995` | `derived-from-v1-attestation` |
| `beta_perf_high_density_rss_limit_mb_per_hr` | `number` | `15.0` | `derived-from-v1-attestation` |
| `beta_perf_infra_memory_diagnostics` | `boolean` | `false` | `derived-from-v1-attestation` |
| `beta_perf_latency_tolerance_pct` | `number` | `25.0` | `policy-default` |
| `beta_perf_media_churn_active_calls` | `integer` | `30` | `environment-override` |
| `beta_perf_media_churn_duration_secs` | `integer` | `120` | `environment-override` |
| `beta_perf_media_diagnostics` | `boolean` | `false` | `derived-from-v1-attestation` |
| `beta_perf_media_memory_diagnostics` | `boolean` | `false` | `derived-from-v1-attestation` |
| `beta_perf_monolithic_soak_active_calls` | `integer` | `30` | `environment-override` |
| `beta_perf_monolithic_soak_duration_secs` | `integer` | `3600` | `environment-override` |
| `beta_perf_regression_baseline_id` | `string` | `20260706T181609Z` | `derived-from-v1-attestation` |
| `beta_perf_regression_baseline_manifest` | `path` | `crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json` | `environment-override` |
| `beta_perf_regression_baseline_manifest_sha256` | `sha256` | `739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9` | `derived-from-v1-attestation` |
| `beta_perf_regression_baseline_root` | `path` | `crates/sip/rvoip-sip/perf-baselines/20260706T181609Z` | `environment-override` |
| `beta_perf_regression_fail` | `boolean` | `true` | `environment-override` |
| `beta_perf_rtp_memory_diagnostics` | `boolean` | `false` | `derived-from-v1-attestation` |
| `beta_performance_recipe_file` | `path` | `bundled config/performance-recipes.yaml` | `derived-from-v1-attestation` |
| `beta_profile_matrix` | `profile-matrix` | `endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000` | `derived-from-v1-attestation` |
| `beta_report_dir` | `path` | `<source-report>` | `environment-override` |
| `beta_report_package` | `boolean` | `true` | `environment-override` |
| `beta_require_canonical_2k_evidence` | `boolean` | `true` | `environment-override` |
| `beta_require_clean_source` | `boolean` | `true` | `environment-override` |
| `beta_require_configured_state_table_evidence` | `boolean` | `false` | `derived-from-v1-attestation` |
| `beta_restore_asterisk_up` | `boolean` | `false` | `policy-default` |
| `beta_restore_freeswitch_up` | `boolean` | `false` | `policy-default` |
| `beta_restore_local_pbx` | `boolean` | `true` | `environment-override` |
| `beta_run_burst_matrix` | `boolean` | `true` | `environment-override` |
| `beta_run_burst_smoke` | `boolean` | `true` | `environment-override` |
| `beta_run_fuzz_smoke` | `boolean` | `true` | `environment-override` |
| `beta_run_local_pbx` | `boolean` | `true` | `environment-override` |
| `beta_run_long_soak` | `boolean` | `true` | `environment-override` |
| `beta_run_pbx` | `boolean` | `false` | `derived-from-v1-attestation` |
| `beta_run_perf_all` | `boolean` | `true` | `environment-override` |
| `beta_run_sipp` | `boolean` | `true` | `environment-override` |
| `beta_run_strict_ua` | `boolean` | `true` | `environment-override` |
| `beta_sipp_cps` | `integer-list` | `[30,100,300,1000,2000]` | `environment-override` |
| `beta_sipp_diagnostics` | `boolean` | `false` | `environment-override` |
| `beta_state_table_fallback_reason` | `enum` | `none` | `derived-from-v1-attestation` |
| `beta_state_table_sha256` | `sha256` | `a649a00aba17f6fe1390d24ada78a8936dce8b577c5e4e777714f0e70c89e3ed` | `derived-from-v1-attestation` |
| `beta_state_table_source` | `enum` | `embedded-default` | `derived-from-v1-attestation` |
| `beta_test_log_filter` | `string` | `off` | `derived-from-v1-attestation` |
| `cargo` | `string` | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` | `derived-from-v1-attestation` |
| `cargo_metadata` | `path` | `environment/cargo-metadata.json` | `derived-from-v1-attestation` |
| `colima` | `string` | `time="2026-07-24T16:14:01-07:00" level=info msg="colima is running using macOS Virtualization.Framework"` | `derived-from-v1-attestation` |
| `docker` | `string` | `Client: Docker Engine - Community` | `derived-from-v1-attestation` |
| `git_revision` | `string` | `8d44fb35` | `derived-from-v1-attestation` |
| `git_status` | `string` | `clean` | `derived-from-v1-attestation` |
| `host` | `string` | `Darwin Mac.lan 25.2.0 Darwin Kernel Version 25.2.0: Tue Nov 18 21:09:41 PST 2025; root:xnu-12377.61.12~1/RELEASE_ARM64_T6031 arm64` | `derived-from-v1-attestation` |
| `rustc` | `string` | `rustc 1.95.0 (59807616e 2026-04-14)` | `derived-from-v1-attestation` |
| `rvoip_perf_allocator_diagnostics` | `boolean` | `false` | `derived-from-v1-attestation` |
| `rvoip_perf_app_event_channel_capacity` | `string` | `Config default` | `derived-from-v1-attestation` |
| `rvoip_perf_dhat` | `boolean` | `false` | `derived-from-v1-attestation` |
| `rvoip_perf_heap_snapshot_secs` | `string` | `auto` | `derived-from-v1-attestation` |
| `rvoip_perf_heap_snapshots` | `boolean` | `false` | `derived-from-v1-attestation` |
| `rvoip_perf_leaks_snapshots` | `boolean` | `false` | `derived-from-v1-attestation` |
| `rvoip_perf_malloc_stack_logging` | `boolean` | `false` | `derived-from-v1-attestation` |
| `rvoip_perf_mass_teardown_calls` | `integer` | `500` | `derived-from-v1-attestation` |
| `rvoip_perf_mass_teardown_setup_cps` | `integer` | `30` | `derived-from-v1-attestation` |
| `rvoip_perf_max_rss_growth_mb_per_hr` | `number` | `15.0` | `environment-override` |
| `rvoip_perf_memory_diag_interval_secs` | `integer` | `5` | `derived-from-v1-attestation` |
| `rvoip_perf_memory_diagnostics` | `boolean` | `false` | `derived-from-v1-attestation` |
| `rvoip_perf_mimalloc_collect_at` | `string` | `off` | `derived-from-v1-attestation` |
| `rvoip_perf_retention_drain_wait_secs` | `integer` | `160` | `environment-override` |
| `rvoip_perf_rss_tail_window_secs` | `integer` | `60` | `derived-from-v1-attestation` |
| `rvoip_perf_skip_audio_frame_delivery` | `boolean` | `false` | `environment-override` |
| `rvoip_perf_soak_active_calls` | `integer` | `500` | `environment-override` |
| `rvoip_perf_soak_cps` | `integer` | `0` | `environment-override` |
| `rvoip_perf_soak_drain_cps` | `integer` | `10` | `environment-override` |
| `rvoip_perf_soak_duration_secs` | `integer` | `3600` | `environment-override` |
| `rvoip_perf_soak_error_sample_limit` | `integer` | `32` | `environment-override` |
| `rvoip_perf_soak_max_hold_secs` | `integer` | `360` | `environment-override` |
| `rvoip_perf_soak_min_hold_secs` | `integer` | `10` | `environment-override` |
| `rvoip_perf_system_allocator` | `boolean` | `false` | `derived-from-v1-attestation` |
| `rvoip_require_api_tools` | `boolean` | `true` | `environment-override` |
| `rvoip_strict_ua_host_ip` | `string` | `192.168.1.3` | `environment-override` |
| `source_at_beta_end` | `path` | `environment/source-at-beta-end.json` | `derived-from-v1-attestation` |
| `source_at_beta_start` | `path` | `environment/source-at-beta-start.json` | `derived-from-v1-attestation` |

## Environment and peer coverage

- Runtime: `rustc 1.95.0 (59807616e 2026-04-14)`
- Cargo: `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`
- Host: `Darwin Mac.lan 25.2.0 Darwin Kernel Version 25.2.0: Tue Nov 18 21:09:41 PST 2025; root:xnu-12377.61.12~1/RELEASE_ARM64_T6031 arm64`
- State table: `embedded-default` with SHA-256 `a649a00aba17f6fe1390d24ada78a8936dce8b577c5e4e777714f0e70c89e3ed`

| Peer | Version | Image/config evidence |
|---|---|---|
| asterisk | `asterisk-asterisk` | `sha256:ad00907c0075338febd3b253701f607e81e44f432d18f93f51dd5aeda16715a9` |
| baresip | `baresip v4.8.0 Copyright (C) 2010 - 2025 Alfred E. Heggestad et al.` | `eb69f17138328c4ff79add2d65e58eb3b8264bc4670e2ce778411fea9d994541` |
| freeswitch | `rvoip-freeswitch:local` | `sha256:a9985743e107bd17764f8cebce8f5935d8a8a320c476963bf5a3ede8aafdc377` |
| sipp | `SIPp v3.7.7-TLS-PCAP-SHA256.` | `c82bce5183cfd527797b41a42231f309622419d2223544f0817c28027b2dad74` |

## Evidence package

- Attested artifacts: **1184** (788213172 bytes).
- Attested JSON evidence records: **121**.
- Performance JSON files accounted for in the performance inventory: **59**.
- Artifact kinds: `{"artifact":316,"executable":29,"input":15,"json":129,"log":444,"report":251}`.
- Original v1 attestation SHA-256: `ace54bb7867c533c03edabcf148bf7f0bc4a5118ba050ad32b5e8db2cb93519c`.
- Correction record SHA-256: `012008d3e40676756fff3c0f450ac9a6a1983d6649262e20b18d11f5bc482c7f`.
- Policy catalog SHA-256: `8cddb4b097971714f62d3c705e8ed3de20f6fec995e8e7251f926b584d7d3180`.
- Report generator SHA-256: `59e6d9c6240b7d44b3adee1223e6d18c8044bd7b8fcd0ab85f30235a83defdca`.

## Interoperability result counts

| Evidence | Recorded results | Bound evidence |
|---|---|---|
| PBX matrix | `{"PASS":288,"rows":288}` | `c328238f7c70e5cbaca8cb2412c7014d1b4941fcf1341121373ec4d305f02365` |
| SIPp matrix | `{"PASS":5,"rows":5}` | `3175a1491c5791ede36a0239003c363300b1bd648ff6e698057090a89f4e0a34` |
| strict-UA matrix | `{"PASS":7,"rows":7}` | `952c55a0a20cb104421522917cd47ee1914fadd93cc0e4b62896c7a46480b11d` |

## Limitations and non-claims

- This is a deterministic post-run reporting derivation; no gate was rerun.
- Later reporting-only commits were not exercised by the candidate run.
- The SIPp start and stop entries are backed by the v1 summary and a shared listener log; future runs capture those lifecycle results directly.
- SHA-256 supplies integrity and reproducibility evidence, not third-party authenticity or signing.
- A PASS applies only to the recorded hardware, configuration, peer versions, test scopes, thresholds, and workloads.

## Verification

From the repository root:

```sh
python3 crates/sip/rvoip-sip/scripts/beta_release_report.py verify \
  --docs-root crates/sip/rvoip-sip/docs \
  --report-root /path/to/reports/20260724T231400Z
```
