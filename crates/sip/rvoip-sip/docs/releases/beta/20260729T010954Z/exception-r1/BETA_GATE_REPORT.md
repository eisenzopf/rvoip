# 0.3.2 Complete Beta Gate Record

> Status and structured results from `source/gate-results.json` for full run
> `20260729T010954Z`. Original statuses and source evidence hashes are
> preserved; personal absolute host paths are deterministically redacted.

## Result

**Strict result: FAIL — 106/108 passed,
2 failed, 0 skipped.**

The owner-approved release exception covers one root policy deviation:
`perf.media-burst-matrix`. `report.performance-metrics` is its derived
reporting roll-up.

## Category totals

| Category | Required | Passed | Failed | Skipped |
|---|---:|---:|---:|---:|
| Build, API, and documentation | 44 | 44 | 0 | 0 |
| PBX and interoperability | 16 | 16 | 0 | 0 |
| Performance and resiliency | 29 | 28 | 1 | 0 |
| Reporting and regression | 4 | 3 | 1 | 0 |
| Security | 11 | 11 | 0 | 0 |
| Source integrity | 4 | 4 | 0 | 0 |

## Complete ordered inventory

| Seq | Gate ID | Status | Seconds | Evidence |
|---:|---|---|---:|---|
| 001 | `source.clean-start` | PASS | 0 | clean_beta_source_fingerprint.log |
| 002 | `source.canonical-2k` | PASS | 0 | canonical_2k_three-pass_evidence.log |
| 003 | `build.format` | PASS | 4 | format_check.log |
| 004 | `build.evidence-helper-tests` | PASS | 8 | beta_evidence_helper_tests.log |
| 005 | `build.public-api` | PASS | 328 | public_api_compatibility.log |
| 006 | `build.sip-all-targets` | PASS | 300 | rvoip-sip_all-target_check.log |
| 007 | `build.claimed-lower-crates` | PASS | 22 | claimed_lower-crate_check.log |
| 008 | `test.supporting-sip-crates` | PASS | 35 | supporting_sip_crate_tests.log |
| 009 | `test.rtp-core` | PASS | 37 | rtp-core_tests.log |
| 010 | `test.workspace-unit` | PASS | 306 | workspace_unit_tests.log |
| 011 | `test.workspace-integration` | PASS | 703 | workspace_target_and_integration_tests.log |
| 012 | `test.workspace-doctest` | PASS | 615 | workspace_doctests.log |
| 013 | `test.sip-unit` | PASS | 220 | rvoip-sip_unit_tests.log |
| 014 | `test.sip-integration` | PASS | 1048 | rvoip-sip_integration_tests.log |
| 015 | `test.sip-doctest` | PASS | 52 | rvoip-sip_doctests.log |
| 016 | `build.sip-examples` | PASS | 274 | rvoip-sip_examples_compile.log |
| 017 | `build.downstream-rvoip` | PASS | 143 | downstream_rvoip_default_check.log |
| 018 | `build.downstream-rvoip-app` | PASS | 151 | downstream_rvoip_app_check.log |
| 019 | `build.downstream-client` | PASS | 14 | downstream_rvoip-client_default_check.log |
| 020 | `build.downstream-client-full` | PASS | 141 | downstream_rvoip-client_full_check.log |
| 021 | `build.downstream-core` | PASS | 3 | downstream_rvoip-core_check.log |
| 022 | `build.downstream-amazon-connect` | PASS | 148 | downstream_rvoip-amazon-connect_server_check.log |
| 023 | `build.downstream-uctp` | PASS | 143 | downstream_rvoip-uctp_check.log |
| 024 | `build.downstream-quic` | PASS | 13 | downstream_rvoip-quic_check.log |
| 025 | `build.downstream-webtransport` | PASS | 4 | downstream_rvoip-webtransport_check.log |
| 026 | `build.downstream-websocket` | PASS | 14 | downstream_rvoip-websocket_media_and_tls_check.log |
| 027 | `build.downstream-webrtc` | PASS | 155 | downstream_rvoip-webrtc_interop_check.log |
| 028 | `build.downstream-audio-device` | PASS | 1 | downstream_rvoip-audio-device_check.log |
| 029 | `test.example-01` | PASS | 1 | standalone_example_01-quickstart-p2p_tests.log |
| 030 | `test.example-02` | PASS | 0 | standalone_example_02-softphone-audio_tests.log |
| 031 | `test.example-03` | PASS | 1 | standalone_example_03-register-to-pbx_tests.log |
| 032 | `test.example-04` | PASS | 0 | standalone_example_04-call-control_tests.log |
| 033 | `test.example-05` | PASS | 1 | standalone_example_05-blind-transfer_tests.log |
| 034 | `test.example-06` | PASS | 1 | standalone_example_06-attended-transfer_tests.log |
| 035 | `test.example-07` | PASS | 0 | standalone_example_07-secure-call-srtp_tests.log |
| 036 | `test.example-08` | PASS | 1 | standalone_example_08-tls-transport_tests.log |
| 037 | `test.example-09` | PASS | 0 | standalone_example_09-ivr-server_tests.log |
| 038 | `test.example-10` | PASS | 0 | standalone_example_10-call-center-b2bua_tests.log |
| 039 | `test.example-11` | PASS | 0 | standalone_example_11-ai-harness-demo_tests.log |
| 040 | `test.example-12` | PASS | 0 | standalone_example_12-customer-escalation-sip-webrtc_tests.log |
| 041 | `test.example-13` | PASS | 1 | standalone_example_13-sip-to-amazon-connect_tests.log |
| 042 | `test.pbx-analyzer` | PASS | 7 | pbx_analyzer_unit_tests.log |
| 043 | `build.rustdoc` | PASS | 18 | rvoip-sip_rustdoc.log |
| 044 | `test.rfc4475` | PASS | 11 | sip-core_rfc_4475_torture_tests.log |
| 045 | `test.generated-message` | PASS | 6 | sip-core_generated_message_validation.log |
| 046 | `test.generated-dialog` | PASS | 25 | sip_dialog_generated_validation.log |
| 047 | `security.advisory-audit` | PASS | 2 | dependency_advisory_audit.log |
| 048 | `security.fuzz-sip-message` | PASS | 38 | parser_fuzz_smoke_sip_message.log |
| 049 | `security.fuzz-uri` | PASS | 1 | parser_fuzz_smoke_uri.log |
| 050 | `security.fuzz-header` | PASS | 2 | parser_fuzz_smoke_header.log |
| 051 | `security.fuzz-sdp` | PASS | 2 | parser_fuzz_smoke_sdp.log |
| 052 | `security.fuzz-rtp` | PASS | 55 | parser_fuzz_smoke_rtp_packet.log |
| 053 | `security.fuzz-rtcp` | PASS | 2 | parser_fuzz_smoke_rtcp_packet.log |
| 054 | `security.fuzz-srtp` | PASS | 1 | parser_fuzz_smoke_srtp_unprotect.log |
| 055 | `security.fuzz-dtls` | PASS | 2 | parser_fuzz_smoke_dtls_record.log |
| 056 | `security.fuzz-stun` | PASS | 2 | parser_fuzz_smoke_stun_response.log |
| 057 | `security.fuzz-g711` | PASS | 2 | parser_fuzz_smoke_g711_unpack.log |
| 058 | `interop.freeswitch-down-before-asterisk` | PASS | 0 | local_freeswitch_down_before_asterisk.log |
| 059 | `interop.asterisk-up` | PASS | 2 | local_asterisk_up.log |
| 060 | `interop.asterisk-matrix` | PASS | 759 | local_asterisk_pbx_matrix.log |
| 061 | `interop.asterisk-down-after` | PASS | 3 | local_asterisk_down_after_matrix.log |
| 062 | `interop.asterisk-down-before-freeswitch` | PASS | 0 | local_asterisk_down_before_freeswitch.log |
| 063 | `interop.freeswitch-up` | PASS | 7 | local_freeswitch_up.log |
| 064 | `interop.freeswitch-matrix` | PASS | 417 | local_freeswitch_pbx_matrix.log |
| 065 | `interop.freeswitch-down-after` | PASS | 9 | local_freeswitch_down_after_matrix.log |
| 066 | `interop.restore-asterisk-down` | PASS | 0 | restore_local_asterisk_down.log |
| 067 | `interop.restore-freeswitch-down` | PASS | 0 | restore_local_freeswitch_down.log |
| 068 | `interop.sipp-build` | PASS | 427 | sipp_standalone_target_build.log |
| 069 | `interop.sipp-start` | PASS | 1 | sipp_standalone_target_start.log |
| 070 | `interop.sipp-matrix` | PASS | 87 | sipp_standalone_matrix.log |
| 071 | `interop.sipp-stop` | PASS | 0 | sipp_standalone_target_stop.log |
| 072 | `interop.strict-ua` | PASS | 9 | baresip_strict-ua_matrix.log |
| 073 | `interop.proxy-descope` | PASS | 0 | kamailio_opensips_proxy_de-scope_audit.log |
| 074 | `perf.capture-boundary` | PASS | 0 | perf_results_capture_boundary.log |
| 075 | `perf.literal-all-config` | PASS | 0 | literal-all_perf_configuration.log |
| 076 | `perf.call-setup-endpoint` | PASS | 514 | perf_call_setup_cps_endpoint.log |
| 077 | `perf.call-setup-pbx` | PASS | 203 | perf_call_setup_cps_pbx-media-server.log |
| 078 | `perf.call-setup-signaling` | PASS | 202 | perf_call_setup_cps_signaling-only-server-high-performance.log |
| 079 | `perf.registration` | PASS | 254 | perf_registration_throughput.log |
| 080 | `perf.concurrent-calls` | PASS | 250 | perf_concurrent_active_calls.log |
| 081 | `perf.rtp-steady-state` | PASS | 252 | perf_rtp_steady_state.log |
| 082 | `perf.backpressure` | PASS | 332 | perf_backpressure_step.log |
| 083 | `perf.transport-recovery` | PASS | 273 | perf_transport_recovery.log |
| 084 | `perf.resiliency-all` | PASS | 520 | all_registered_resiliency_tests.log |
| 085 | `perf.mid-call-signaling` | PASS | 253 | perf_mid-call_signaling_under_media.log |
| 086 | `perf.tls-overhead` | PASS | 281 | perf_tls_overhead.log |
| 087 | `perf.srtp-overhead` | PASS | 252 | perf_srtp_overhead.log |
| 088 | `perf.pdd-180` | PASS | 279 | perf_pdd_with_180_first.log |
| 089 | `perf.long-duration` | PASS | 338 | perf_sustained_long-duration_calls.log |
| 090 | `perf.registrar-scale` | PASS | 249 | perf_registrar_binding_scale.log |
| 091 | `perf.mixed-workload` | PASS | 262 | perf_mixed_workload.log |
| 092 | `perf.b2bua` | PASS | 284 | perf_b2bua_forwarding.log |
| 093 | `perf.ai-agent` | PASS | 251 | perf_ai-agent_load.log |
| 094 | `perf.contact-center` | PASS | 257 | perf_contact-center_transfers.log |
| 095 | `perf.sipp-parity` | PASS | 246 | perf_sipp_parity.log |
| 096 | `perf.soak-invariants` | PASS | 265 | perf_soak_target_invariant_tests.log |
| 097 | `perf.media-churn` | PASS | 334 | perf_media_churn.log |
| 098 | `perf.monolithic-soak` | PASS | 3763 | perf_monolithic_soak.log |
| 099 | `perf.mass-teardown` | PASS | 179 | perf_mass_teardown_stress.log |
| 100 | `perf.session-churn` | PASS | 413 | perf_session_churn_leak.log |
| 101 | `perf.media-burst-matrix` | FAIL | 5591 | perf_media_burst_matrix.log |
| 102 | `perf.soak-candidate` | PASS | 4412 | perf_soak_candidate.log |
| 103 | `report.regression-baseline` | PASS | 0 | perf_regression_baseline_evidence.log |
| 104 | `report.regression-audit` | PASS | 0 | perf_regression_audit.log |
| 105 | `report.perf-evidence-capture` | PASS | 1 | perf_results_evidence_capture.log |
| 106 | `report.performance-metrics` | FAIL | 0 | performance_gate_metrics_report.log |
| 107 | `source.final-capture` | PASS | 0 | beta_final_source_fingerprint_capture.log |
| 108 | `source.canonical-2k-unchanged` | PASS | 0 | canonical_2k_beta_source_unchanged.log |

The full structured checks, commands, timestamps, and SHA-256 evidence bindings
are retained in [`source/gate-results.json`](source/gate-results.json) and
[`source/attestation.json`](source/attestation.json).
