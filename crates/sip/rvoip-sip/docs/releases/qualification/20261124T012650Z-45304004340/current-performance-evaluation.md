## Performance Gate Metrics

This table is generated from the packaged JSON artifacts. `PASS` means
the recorded policy and every related tracking metric agree.

### Canonical 2,000-CPS evaluation

- result: `PASS`
- evidence: `canonical-2k/index.json`, `canonical-2k/run-1/report.json`, `canonical-2k/run-2/report.json`, `canonical-2k/run-3/report.json`

| Metric | Requirement | Observed | Result |
|--------|-------------|----------|--------|
| schema | rvoip-canonical-2k-evidence-v2 | `"rvoip-canonical-2k-evidence-v2"` | PASS |
| status | PASS | `"PASS"` | PASS |
| scenario | perf_call_setup_cps_pbx-media-server | `"perf_call_setup_cps_pbx-media-server"` | PASS |
| run_count | 3 | `3` | PASS |
| indexed_runs | 3 | `3` | PASS |
| candidate_commit | 2ae9fa49c5903a3e947a53609ee3f7d64c71a013 | `"2ae9fa49c5903a3e947a53609ee3f7d64c71a013"` | PASS |
| source_fingerprint | one 64-character SHA-256 shared by source and every run | `"53052e0ab62c6e666715a4ac49b4d8489a1c3c74f89dee994e5c81203800b27b"` | PASS |
| executable_identity | one 64-character SHA-256 shared by every run | `"5f924d8cc57d4b3db14b8fc7ee92ca00a7926964800f1a53e2df003aaf071b0d"` | PASS |
| canonical_run_1 | clean accepted 2,000-CPS / 65,000-call PASS | `{"achieved_cps": 1857.11, "asr": 1.0, "calls_offered": 65000, "calls_succeeded": 65000, "sequence": 1, "setup_latency_p50_ns": 1324031, "setup_latency_p95_ns": 1825791, "setup_latency_p99_ns": 2271231, "target_cps": 2000.0}` | PASS |
| canonical_run_2 | clean accepted 2,000-CPS / 65,000-call PASS | `{"achieved_cps": 1857.1, "asr": 1.0, "calls_offered": 65000, "calls_succeeded": 65000, "sequence": 2, "setup_latency_p50_ns": 1326079, "setup_latency_p95_ns": 1850367, "setup_latency_p99_ns": 2289663, "target_cps": 2000.0}` | PASS |
| canonical_run_3 | clean accepted 2,000-CPS / 65,000-call PASS | `{"achieved_cps": 1857.14, "asr": 1.0, "calls_offered": 65000, "calls_succeeded": 65000, "sequence": 3, "setup_latency_p50_ns": 1376255, "setup_latency_p95_ns": 1879039, "setup_latency_p99_ns": 2275327, "target_cps": 2000.0}` | PASS |

### High-density media burst

- result: `PASS`
- evidence: `perf_burst_matrix/burst_20261123_225121_2673/high-density-media-burst/perf_burst_caller_high-density-media-burst.json`, `perf_burst_matrix/burst_20261123_225121_2673/high-density-media-burst/perf_burst_receiver_high-density-media-burst.json`

| Metric | Requirement | Observed | Result |
|--------|-------------|----------|--------|
| media_burst_cps | exactly 160 | `160.0` | PASS |
| minimum_asr | exactly 0.995 | `0.995` | PASS |
| rss_limit_mb_per_hr | exactly 15 | `15.0` | PASS |
| full_audio_frame_delivery | enabled for caller and receiver | `{"caller_skip": false, "receiver_skip": false}` | PASS |
| asr | >= 0.995 | `1.0` | PASS |
| timeout_failures | <= 0.5% and exactly reconciled | `{"count": 0, "percent": 0.0}` | PASS |
| non_timeout_errors | 0 | `0` | PASS |
| caller_retained_after_drain | 0 | `0` | PASS |
| receiver_retained_after_drain | 0 | `0` | PASS |
| receiver_active_audio_receivers_after_drain | 0 | `0` | PASS |
| caller_transaction_manager_after_drain | 0 | `0` | PASS |
| receiver_transaction_manager_after_drain | 0 | `0` | PASS |
| delivered_audio_frames | > 0 | `11610101` | PASS |
| caller_rss_gate_mb_per_hr | <= 15 | `-1.41` | PASS |
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
| rss_gate_window | active_tail_1200s | `"active_tail_1200s"` | PASS |
| rss_active_tail_window_complete | true | `true` | PASS |
| rss_active_tail_estimator | theil_sen_pairwise_slopes | `"theil_sen_pairwise_slopes"` | PASS |
| rss_active_tail_window_secs | >= 1190 | `1195.04` | PASS |
| call_completion | all offered calls succeed | `{"offered": 587, "succeeded": 587}` | PASS |
| delivered_audio_frames | > 0 | `5380613` | PASS |
| rss_gate_growth_mb_per_hr | <= 15 | `11.71` | PASS |

### Split soak

- result: `PASS`
- evidence: `perf_soak_caller.json`, `perf_soak_receiver.json`

| Metric | Requirement | Observed | Result |
|--------|-------------|----------|--------|
| duration_secs | exactly 7200 for caller and receiver | `{"caller": 7200, "receiver": 7200}` | PASS |
| active_calls_target | exactly 500 for caller and receiver | `{"caller": 500, "receiver": 500}` | PASS |
| rss_limit_mb_per_hr | exactly 15 for caller and receiver | `{"caller": 15.0, "receiver": 15.0}` | PASS |
| full_audio_frame_delivery | enabled for caller and receiver | `{"caller_skip": false, "receiver_skip": false}` | PASS |
| call_completion | every offered call succeeds and completes at the receiver | `{"offered": 19655, "receiver_completed": 19655, "succeeded": 19655}` | PASS |
| errors | 0 | `{"call_failed": 0, "media_setup_failed": 0, "teardown_failed": 0}` | PASS |
| retained_after_drain | 0 for caller and receiver | `{"caller": 0, "receiver": 0}` | PASS |
| receiver_active_audio_receivers_after_drain | 0 | `0` | PASS |
| transaction_managers_after_drain | 0 for caller and receiver | `{"caller": 0, "receiver": 0}` | PASS |
| transaction_runners_after_drain | 0 for caller and receiver | `{"caller": 0, "receiver": 0}` | PASS |
| receiver_stop_seen | true | `true` | PASS |
| rss_gate_window | active_tail_1200s for caller and receiver | `{"caller": "active_tail_1200s", "receiver": "active_tail_1200s"}` | PASS |
| rss_active_tail_window_complete | true for caller and receiver | `{"caller": true, "receiver": true}` | PASS |
| delivered_audio_frames | > 0 | `179742779` | PASS |
| caller_rss_active_tail_window_secs | >= 1190 | `1195.03` | PASS |
| caller_rss_gate_growth_mb_per_hr | <= 15 | `2.81` | PASS |
| receiver_rss_active_tail_window_secs | >= 1190 | `1195.03` | PASS |
| receiver_rss_gate_growth_mb_per_hr | <= 15 | `-1.21` | PASS |
