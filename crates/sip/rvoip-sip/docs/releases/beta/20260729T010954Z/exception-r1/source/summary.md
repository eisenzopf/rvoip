# rvoip-sip Beta Gate Summary

- timestamp: 20260729T010954Z
- mode: full
- workspace: <workspace>
- artifact_dir: <workspace>/target/beta-gate/20260729T010954Z
- environment: `environment/environment.md`

## Environment Snapshot

- git_revision: `fbc96ed4`
- git_status: `clean`
- rustc: `rustc 1.95.0 (59807616e 2026-04-14)`
- cargo: `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`
- cargo_metadata: `environment/cargo-metadata.json`
- beta_deny_warnings: `1`
- beta_test_log_filter: `off`
- rvoip_require_api_tools: `1`
- source_at_beta_start: `environment/source-at-beta-start.json`
- source_at_beta_end: `environment/source-at-beta-end.json`
- beta_require_clean_source: `1`
- beta_gate_require_external: `1`
- beta_attestation_features: `generated-validation,dev-insecure-tls,perf-tests,g729`
- beta_attestation_target: `rustc-host`
- beta_require_canonical_2k_evidence: `1`
- beta_canonical_2k_run_dirs: `<workspace>/target/perf-results/profiles/20260729T001656Z_clean_3N6dhy:<workspace>/target/perf-results/profiles/20260729T004007Z_clean_ez7qJb:<workspace>/target/perf-results/profiles/20260729T005501Z_clean_KWysQw`
- beta_state_table_source: `embedded-default`
- beta_state_table_fallback_reason: `none`
- beta_state_table_sha256: `a649a00aba17f6fe1390d24ada78a8936dce8b577c5e4e777714f0e70c89e3ed`
- beta_require_configured_state_table_evidence: `0`
- host: `Darwin Jonathans-MacBook-Pro.local 25.2.0 Darwin Kernel Version 25.2.0: Tue Nov 18 21:09:41 PST 2025; root:xnu-12377.61.12~1/RELEASE_ARM64_T6031 arm64`
- colima: `time="2026-07-28T18:09:56-07:00" level=info msg="colima is running using macOS Virtualization.Framework"`
- docker: `Client: Docker Engine - Community`
- beta_profile_matrix: `endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`
- beta_run_perf_all: `1`
- beta_perf_regression_fail: `1`
- beta_perf_regression_baseline_id: `20260706T181609Z`
- beta_perf_regression_baseline_manifest_sha256: `739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`
- beta_perf_high_density_burst_cps: `160`
- beta_perf_high_density_min_asr: `0.995`
- beta_perf_high_density_rss_limit_mb_per_hr: `15`
- beta_perf_media_churn_duration_secs: `120`
- beta_perf_media_churn_active_calls: `30`
- beta_perf_monolithic_soak_duration_secs: `3600`
- beta_perf_monolithic_soak_active_calls: `30`
- beta_performance_recipe_file: `bundled config/performance-recipes.yaml`
- beta_perf_features: `perf-tests`
- beta_perf_infra_memory_diagnostics: `0`
- beta_perf_media_diagnostics: `0`
- beta_perf_media_memory_diagnostics: `0`
- beta_perf_rtp_memory_diagnostics: `0`
- beta_run_burst_smoke: `1`
- beta_run_burst_matrix: `1`
- beta_burst_scenario_file: `bundled config/perf-burst-scenarios.yaml`
- beta_burst_matrix: `all`
- beta_pbx_provider: `both`
- beta_pbx_api: `all`
- beta_pbx_scenario: `all`
- beta_pbx_g729_profiles: `g729a g729ab`
- beta_run_local_pbx: `1`
- beta_run_pbx: `0`
- beta_run_sipp: `1`
- beta_sipp_diagnostics: `0`
- beta_run_strict_ua: `1`
- beta_run_long_soak: `1`
- rvoip_perf_soak_duration_secs: `3600`
- rvoip_perf_soak_active_calls: `500`
- rvoip_perf_soak_min_hold_secs: `10`
- rvoip_perf_soak_max_hold_secs: `360`
- rvoip_perf_soak_cps: `0`
- rvoip_perf_soak_drain_cps: `10`
- rvoip_perf_soak_error_sample_limit: `32`
- rvoip_perf_retention_drain_wait_secs: `160`
- rvoip_perf_mass_teardown_calls: `500`
- rvoip_perf_mass_teardown_setup_cps: `30`
- rvoip_perf_memory_diagnostics: `0`
- rvoip_perf_allocator_diagnostics: `0`
- rvoip_perf_memory_diag_interval_secs: `5`
- rvoip_perf_mimalloc_collect_at: `off`
- rvoip_perf_system_allocator: `0`
- rvoip_perf_dhat: `0`
- rvoip_perf_heap_snapshots: `0`
- rvoip_perf_heap_snapshot_secs: `auto`
- rvoip_perf_malloc_stack_logging: `0`
- rvoip_perf_leaks_snapshots: `0`
- rvoip_perf_skip_audio_frame_delivery: `0`
- rvoip_perf_max_rss_growth_mb_per_hr: `15`
- rvoip_perf_app_event_channel_capacity: `Config default`
- rvoip_perf_rss_tail_window_secs: `60`

Full environment evidence, Docker state, redacted runtime variables, and local
PBX config snapshots are in `environment/environment.md`.

## Gates

| Status | Gate | Duration | Log |
|--------|------|----------|-----|
| PASS | clean beta source fingerprint | 0s | `clean_beta_source_fingerprint.log` |
| PASS | canonical 2k three-pass evidence | 0s | `canonical_2k_three-pass_evidence.log` |
| PASS | format check | 4s | `format_check.log` |
| PASS | beta evidence helper tests | 8s | `beta_evidence_helper_tests.log` |
| PASS | public API compatibility | 328s | `public_api_compatibility.log` |
| PASS | rvoip-sip all-target check | 300s | `rvoip-sip_all-target_check.log` |
| PASS | claimed lower-crate check | 22s | `claimed_lower-crate_check.log` |
| PASS | supporting SIP crate tests | 35s | `supporting_sip_crate_tests.log` |
| PASS | rtp-core tests | 37s | `rtp-core_tests.log` |
| PASS | workspace unit tests | 306s | `workspace_unit_tests.log` |
| PASS | workspace target and integration tests | 703s | `workspace_target_and_integration_tests.log` |
| PASS | workspace doctests | 615s | `workspace_doctests.log` |
| PASS | rvoip-sip unit tests | 220s | `rvoip-sip_unit_tests.log` |
| PASS | rvoip-sip integration tests | 1048s | `rvoip-sip_integration_tests.log` |
| PASS | rvoip-sip doctests | 52s | `rvoip-sip_doctests.log` |
| PASS | rvoip-sip examples compile | 274s | `rvoip-sip_examples_compile.log` |
| PASS | downstream rvoip default check | 143s | `downstream_rvoip_default_check.log` |
| PASS | downstream rvoip app check | 151s | `downstream_rvoip_app_check.log` |
| PASS | downstream rvoip-client default check | 14s | `downstream_rvoip-client_default_check.log` |
| PASS | downstream rvoip-client full check | 141s | `downstream_rvoip-client_full_check.log` |
| PASS | downstream rvoip-core check | 3s | `downstream_rvoip-core_check.log` |
| PASS | downstream rvoip-amazon-connect server check | 148s | `downstream_rvoip-amazon-connect_server_check.log` |
| PASS | downstream rvoip-uctp check | 143s | `downstream_rvoip-uctp_check.log` |
| PASS | downstream rvoip-quic check | 13s | `downstream_rvoip-quic_check.log` |
| PASS | downstream rvoip-webtransport check | 4s | `downstream_rvoip-webtransport_check.log` |
| PASS | downstream rvoip-websocket media and TLS check | 14s | `downstream_rvoip-websocket_media_and_tls_check.log` |
| PASS | downstream rvoip-webrtc interop check | 155s | `downstream_rvoip-webrtc_interop_check.log` |
| PASS | downstream rvoip-audio-device check | 1s | `downstream_rvoip-audio-device_check.log` |
| PASS | standalone example 01-quickstart-p2p tests | 1s | `standalone_example_01-quickstart-p2p_tests.log` |
| PASS | standalone example 02-softphone-audio tests | 0s | `standalone_example_02-softphone-audio_tests.log` |
| PASS | standalone example 03-register-to-pbx tests | 1s | `standalone_example_03-register-to-pbx_tests.log` |
| PASS | standalone example 04-call-control tests | 0s | `standalone_example_04-call-control_tests.log` |
| PASS | standalone example 05-blind-transfer tests | 1s | `standalone_example_05-blind-transfer_tests.log` |
| PASS | standalone example 06-attended-transfer tests | 1s | `standalone_example_06-attended-transfer_tests.log` |
| PASS | standalone example 07-secure-call-srtp tests | 0s | `standalone_example_07-secure-call-srtp_tests.log` |
| PASS | standalone example 08-tls-transport tests | 1s | `standalone_example_08-tls-transport_tests.log` |
| PASS | standalone example 09-ivr-server tests | 0s | `standalone_example_09-ivr-server_tests.log` |
| PASS | standalone example 10-call-center-b2bua tests | 0s | `standalone_example_10-call-center-b2bua_tests.log` |
| PASS | standalone example 11-ai-harness-demo tests | 0s | `standalone_example_11-ai-harness-demo_tests.log` |
| PASS | standalone example 12-customer-escalation-sip-webrtc tests | 0s | `standalone_example_12-customer-escalation-sip-webrtc_tests.log` |
| PASS | standalone example 13-sip-to-amazon-connect tests | 1s | `standalone_example_13-sip-to-amazon-connect_tests.log` |
| PASS | PBX analyzer unit tests | 7s | `pbx_analyzer_unit_tests.log` |
| PASS | rvoip-sip rustdoc | 18s | `rvoip-sip_rustdoc.log` |
| PASS | sip-core RFC 4475 torture tests | 11s | `sip-core_rfc_4475_torture_tests.log` |
| PASS | sip-core generated message validation | 6s | `sip-core_generated_message_validation.log` |
| PASS | sip dialog generated validation | 25s | `sip_dialog_generated_validation.log` |
| PASS | dependency advisory audit | 2s | `dependency_advisory_audit.log` |
| PASS | parser fuzz smoke (sip_message) | 38s | `parser_fuzz_smoke_sip_message.log` |
| PASS | parser fuzz smoke (uri) | 1s | `parser_fuzz_smoke_uri.log` |
| PASS | parser fuzz smoke (header) | 2s | `parser_fuzz_smoke_header.log` |
| PASS | parser fuzz smoke (sdp) | 2s | `parser_fuzz_smoke_sdp.log` |
| PASS | parser fuzz smoke (rtp_packet) | 55s | `parser_fuzz_smoke_rtp_packet.log` |
| PASS | parser fuzz smoke (rtcp_packet) | 2s | `parser_fuzz_smoke_rtcp_packet.log` |
| PASS | parser fuzz smoke (srtp_unprotect) | 1s | `parser_fuzz_smoke_srtp_unprotect.log` |
| PASS | parser fuzz smoke (dtls_record) | 2s | `parser_fuzz_smoke_dtls_record.log` |
| PASS | parser fuzz smoke (stun_response) | 2s | `parser_fuzz_smoke_stun_response.log` |
| PASS | parser fuzz smoke (g711_unpack) | 2s | `parser_fuzz_smoke_g711_unpack.log` |
| PASS | local FreeSWITCH down before Asterisk | 0s | `local_freeswitch_down_before_asterisk.log` |
| PASS | local Asterisk up | 2s | `local_asterisk_up.log` |
| PASS | local Asterisk PBX matrix | 759s | `local_asterisk_pbx_matrix.log` |
| PASS | local Asterisk down after matrix | 3s | `local_asterisk_down_after_matrix.log` |
| PASS | local Asterisk down before FreeSWITCH | 0s | `local_asterisk_down_before_freeswitch.log` |
| PASS | local FreeSWITCH up | 7s | `local_freeswitch_up.log` |
| PASS | local FreeSWITCH PBX matrix | 417s | `local_freeswitch_pbx_matrix.log` |
| PASS | local FreeSWITCH down after matrix | 9s | `local_freeswitch_down_after_matrix.log` |
| PASS | restore local Asterisk down | 0s | `restore_local_asterisk_down.log` |
| PASS | restore local FreeSWITCH down | 0s | `restore_local_freeswitch_down.log` |
| PASS | SIPp standalone target build | 427s | `sipp_standalone_target_build.log` |
| PASS | SIPp standalone target start | 1s | `sipp_standalone_target_start.log` |
| PASS | SIPp standalone matrix | 87s | `sipp_standalone_matrix.log` |
| PASS | SIPp standalone target stop | 0s | `sipp_standalone_target_stop.log` |
| PASS | baresip strict-UA matrix | 9s | `baresip_strict-ua_matrix.log` |
| PASS | Kamailio/OpenSIPS proxy de-scope audit | 0s | `kamailio_opensips_proxy_de-scope_audit.log` |
| PASS | perf results capture boundary | 0s | `perf_results_capture_boundary.log` |
| PASS | literal-all perf configuration | 0s | `literal-all_perf_configuration.log` |
| PASS | perf call setup CPS (endpoint) | 514s | `perf_call_setup_cps_endpoint.log` |
| PASS | perf call setup CPS (pbx-media-server) | 203s | `perf_call_setup_cps_pbx-media-server.log` |
| PASS | perf call setup CPS (signaling-only-server-high-performance) | 202s | `perf_call_setup_cps_signaling-only-server-high-performance.log` |
| PASS | perf registration throughput | 254s | `perf_registration_throughput.log` |
| PASS | perf concurrent active calls | 250s | `perf_concurrent_active_calls.log` |
| PASS | perf RTP steady state | 252s | `perf_rtp_steady_state.log` |
| PASS | perf backpressure step | 332s | `perf_backpressure_step.log` |
| PASS | perf transport recovery | 273s | `perf_transport_recovery.log` |
| PASS | all registered resiliency tests | 520s | `all_registered_resiliency_tests.log` |
| PASS | perf mid-call signaling under media | 253s | `perf_mid-call_signaling_under_media.log` |
| PASS | perf TLS overhead | 281s | `perf_tls_overhead.log` |
| PASS | perf SRTP overhead | 252s | `perf_srtp_overhead.log` |
| PASS | perf PDD with 180 first | 279s | `perf_pdd_with_180_first.log` |
| PASS | perf sustained long-duration calls | 338s | `perf_sustained_long-duration_calls.log` |
| PASS | perf registrar binding scale | 249s | `perf_registrar_binding_scale.log` |
| PASS | perf mixed workload | 262s | `perf_mixed_workload.log` |
| PASS | perf B2BUA forwarding | 284s | `perf_b2bua_forwarding.log` |
| PASS | perf AI-agent load | 251s | `perf_ai-agent_load.log` |
| PASS | perf contact-center transfers | 257s | `perf_contact-center_transfers.log` |
| PASS | perf SIPp parity | 246s | `perf_sipp_parity.log` |
| PASS | perf soak target invariant tests | 265s | `perf_soak_target_invariant_tests.log` |
| PASS | perf media churn | 334s | `perf_media_churn.log` |
| PASS | perf monolithic soak | 3763s | `perf_monolithic_soak.log` |
| PASS | perf mass teardown stress | 179s | `perf_mass_teardown_stress.log` |
| PASS | perf session churn leak | 413s | `perf_session_churn_leak.log` |
| FAIL | perf media burst matrix | 5591s | `perf_media_burst_matrix.log` |
| PASS | perf soak candidate | 4412s | `perf_soak_candidate.log` |
| PASS | perf regression baseline evidence | 0s | `perf_regression_baseline_evidence.log` |
| PASS | perf regression audit | 0s | `perf_regression_audit.log` |
| PASS | perf results evidence capture | 1s | `perf_results_evidence_capture.log` |
| FAIL | performance gate metrics report | 0s | `performance_gate_metrics_report.log` |
| PASS | beta final source fingerprint capture | 0s | `beta_final_source_fingerprint_capture.log` |
| PASS | canonical 2k beta source unchanged | 0s | `canonical_2k_beta_source_unchanged.log` |

## Performance Gate Metrics

This table is generated from the packaged JSON artifacts. `PASS` means
the recorded policy and every related tracking metric agree.

### High-density media burst

- result: `FAIL`
- evidence: `perf_burst_matrix/burst_20260728_230142_42456/high-density-media-burst/perf_burst_caller_high-density-media-burst.json`, `perf_burst_matrix/burst_20260728_230142_42456/high-density-media-burst/perf_burst_receiver_high-density-media-burst.json`

| Metric | Requirement | Observed | Result |
|--------|-------------|----------|--------|
| media_burst_cps | exactly 160 | `160.0` | PASS |
| minimum_asr | exactly 0.995 | `0.995` | PASS |
| rss_limit_mb_per_hr | exactly 15 | `15.0` | PASS |
| full_audio_frame_delivery | enabled for caller and receiver | `{"caller_skip": false, "receiver_skip": false}` | PASS |
| asr | >= 0.995 | `0.9928` | FAIL |
| timeout_failures | <= 0.5% and exactly reconciled | `{"count": 129, "percent": 0.7166666666666667}` | FAIL |
| non_timeout_errors | 0 | `0` | PASS |
| caller_retained_after_drain | 0 | `0` | PASS |
| receiver_retained_after_drain | 0 | `0` | PASS |
| receiver_active_audio_receivers_after_drain | 0 | `0` | PASS |
| caller_transaction_manager_after_drain | 0 | `0` | PASS |
| receiver_transaction_manager_after_drain | 0 | `0` | PASS |
| delivered_audio_frames | > 0 | `9922061` | PASS |
| caller_rss_gate_mb_per_hr | <= 15 | `0.0` | PASS |
| receiver_rss_gate_mb_per_hr | <= 15 | `0.0` | PASS |

### Monolithic soak

- result: `PASS`
- evidence: `perf_soak_30min.json`

| Metric | Requirement | Observed | Result |
|--------|-------------|----------|--------|
| duration_secs | exactly 3600 | `3600` | PASS |
| active_calls_target | exactly 30 | `30` | PASS |
| rss_limit_mb_per_hr | exactly 15 | `15.0` | PASS |
| errors | 0 | `0` | PASS |
| retained_after_drain | 0 | `0` | PASS |
| active_audio_receivers_after_drain | 0 | `0` | PASS |
| transaction_manager_after_drain | 0 | `0` | PASS |
| transaction_runner_after_drain | 0 | `0` | PASS |
| controlled_drain_failed | 0 | `0` | PASS |
| call_completion | all offered calls succeed | `{"offered": 587, "succeeded": 587}` | PASS |
| delivered_audio_frames | > 0 | `5379966` | PASS |
| rss_gate_growth_mb_per_hr | <= 15 | `12.7` | PASS |

## Accepted Dependency Advisories

- `RUSTSEC-2023-0071` (`rsa`): accepted beta risk because RustSec reports no fixed upgrade.
- `RUSTSEC-2026-0185` (`quinn-proto`), `RUSTSEC-2026-0104`/`-0098`/`-0099` (`rustls-webpki`): accepted; transitive via the `quinn`/`rustls` stacks.
- Affected paths: `users-core` RS256/JWK support and `webauthn-rs` transitive crypto.
- Evidence: `security/accepted-advisories.md`.

## Report Package

- enabled: `1`
- report_dir: `<source-report>`
- raw_attestation: `attestation.json`
- generic_latest_pointer_informational_only: `<workspace>/crates/sip/rvoip-sip/beta-report/latest.txt`
- successful_mode_pointer: `<workspace>/crates/sip/rvoip-sip/beta-report/latest-full-clean.txt`
- pointer_policy: mode-specific pointers update only after an independently
  verified PASS with zero skips and the mode's required evidence. Interop
  requires an identified peer; performance requires an executable and result
  JSON; full additionally requires unchanged clean source, an identified peer,
  performance result JSON, and three canonical 2K runs.

## Result

- failures: 2
- skips: 0
