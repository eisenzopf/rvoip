# RVoIP Performance Qualification Report

> Exact-candidate observations archived by protected run [45304004340](https://github.com/eisenzopf/rvoip/actions/runs/45304004340). A blank cell means the scenario does not emit that metric; it does not mean zero. Measurements belonging only to an accepted prior-run receipt are not copied into this table.

Release: `0.5.0` · commit: `2ae9fa49c5903a3e947a53609ee3f7d64c71a013` · qualification: **PASS**.

## Archived measurements

| Scenario | Kind | Target CPS | Achieved CPS | ASR | Calls | Setup p99 ms | Peak RSS MB | Duration s | Evidence SHA-256 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| `perf_mass_teardown_stress` | `point` | 30 | — | — | — | — | 237.38 | 0 | `29622d7cabf35fa94c3334b0bd018f2f9b7b2c0320bd8e2daf03b591a2b9a3ce` |
| `perf_mid_call_signal_under_media` | `point` | 30 | — | 1 | —/30 | 20.365 | 89.9 | 0 | `9fc5cfdcf36bed0387d2a95d0f531eb73c65585f7d2a7c1a3fdc4c7da6c85404` |
| `perf_mixed_workload` | `point` | 50 | 50 | 1 | — | 2.879 | 129.46 | 0 | `570c606a467302a3e6ca41c2da180da76b57bf31251e2046e687dbf144f8662a` |
| `perf_call_setup_cps_endpoint` | `point` | 30 | 27.86 | 1 | 975/975 | 1.965 | 131.82 | 0 | `abdc213395435b3ba742d3024f8fa26f3acdfe2b98d9795ef604c423ea66f49f` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.684 | 361.08 | 0 | `14da3c242ad73da96e5824716aed2802dfc49379895e06785190f001883ab620` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 1000 | 928.54 | 1 | 32500/32500 | 2.107 | 1175.23 | 0 | `f02fa24d0daa9ebb0e2244106c6f8c7878028bb1131de9c6d0b6090470c26843` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.877 | 2077.32 | 0 | `e3f49b7842ff48521edd0529ec5fde5afd06c47d90e4be2abc64d4f646082990` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 30 | 27.86 | 1 | 975/975 | 1.878 | 260.89 | 0 | `f4c0f113ff5097c1d8456bd41b8826a95880aec8ae476e8a9bde6eacfea25592` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.465 | 567.7 | 0 | `cb89910d67e8ad721a7e31d79cc3cc0003cf091219e29f6c4caf99b20d9ff427` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `sweep` | 2000 | 1857.13 | 1 | — | — | — | — | `c55c661542bfaa9f2775b32ac09e34bce006993141acc96b67960c7883707470` |
| `perf_concurrent_active_calls` | `point` | 500 | — | 1 | —/500 | 202.768 | 235.17 | 0 | `49af1cf75033040172b436feba0fdb88e658485db2a2ae87b4b9a510a81f5c8b` |
| `perf_ai_agent_load` | `point` | 30 | — | 1 | — | 18.301 | 82.63 | 0 | `e0ffda6f373d9e5960352a3931f2a36fe0ee8581ea80bd18dc8ec4ef9e7e36df` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.789 | 377.48 | 0 | `25795069b5c7b1023412b044718ee634c79a3553c0e6dbd9c365b6d2094d96c7` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.57 | 1 | 32500/32500 | 1.967 | 1191.53 | 0 | `174a5ba65840d582771aa07cb732a86a4aa301a0666f4e6c379a77a348a49be5` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.11 | 1 | 65000/65000 | 2.804 | 2095.31 | 0 | `9865082893a07d078aa478395c2a9a0c2f23f837068f73ebb2fed68f677b2bd5` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.954 | 270.62 | 0 | `06317661c76c846e1febb9a467f2fc9fc62186401955fad330f2a941edb4bbce` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.56 | 1 | 9750/9750 | 1.556 | 583.98 | 0 | `b3017a655bd2a387efc976c4b5831d49466fcc30157092e044de8ff74bfa6c99` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.11 | 1 | — | — | — | — | `9c374b5dde7dcba97a6152b5e18a559c7a2a1e0bb675a3b9e917e66fa6e7f7d8` |
| `perf_session_churn_leak` | `point` | 0 | — | — | 250/250 | 1.622 | 98.16 | 0 | `82e575998cf6f4ef69ded24dace663ff805edee9c2dfd8d6c3570800092493c5` |
| `perf_transport_recovery` | `point` | 5 | — | — | — | — | 111.36 | 0 | `d56981fdde9a1e282a3c29618d6537c495e81df96ba0bfa04bd088d9006cbfa4` |
| `perf_pdd_with_180_first` | `point` | 50 | 46.43 | 1 | 1625/1625 | 2.022 | 171.82 | 0 | `60c75fa7a3e65f7930e75353414f99a6f55e384e7146a8499da515bd700649a3` |
| `perf_registrar_binding_scale` | `point` | 100 | — | — | — | — | 91.14 | 0 | `13f571723a3f1ab5e4b00fd04c70ad8b5bf9a4185ed4bf1d3a6c3228768d11df` |
| `perf_registration_throughput` | `point` | 100 | — | — | — | — | 93.09 | 0 | `092d74739f9ff363d164ed65b39ded33574c5f8e9d68faf5f13ff5c32bb46bba` |
| `perf_sustained_long_duration_calls` | `point` | 30 | 28.85 | 1 | 1875/1875 | 2.148 | 292.42 | 0 | `f40a86e01def916a32d7722037d9966fbd69cdad34b816a3e247316058c890e2` |
| `perf_backpressure_step` | `point` | 200 | — | — | 13483/13483 | — | 404.22 | 0 | `c793f53749cfe5d3327ba461221b08bf59eb8f7cc6849240fb127a6301c833ae` |
| `perf_contact_center_transfers` | `point` | 20 | — | 1 | —/20 | 10.592 | 81.56 | 0 | `3463a1c22182022fafb962863c772e27131ea0ba418250d98f1b70d4a74156f4` |
| `perf_srtp_overhead` | `point` | 50 | — | 1 | —/50 | 30.933 | 90.73 | 0 | `aa5c169b8e0a4cbf299714db72d349a589afec3a56f350119b431830080cf7d0` |
| `perf_tls_overhead` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.773 | 206.75 | 0 | `41bbcda2ccff6503b9da8ccd8417e93f7b48616b606feae7c82cd591ef38b462` |
| `perf_b2bua_forwarding` | `point` | 30 | 27.86 | 1 | 975/975 | 1.686 | 357.86 | 0 | `2c91a0c71283dadd934dac74dd5c6aa1352a93489c46742ba9b3b6dd7443dffb` |
| `perf_media_churn` | `point` | 0 | — | — | — | — | — | 0 | `505fa48a013043b5fca3d98525b2f3557ade18be0225ca52037169f6d4e92f56` |
| `perf_rtp_steady_state` | `point` | 50 | — | 1 | —/50 | 24.134 | 91.2 | 0 | `88b1838c7b51b527d96bbf9aef887cb114902a0a5412118cf34215a591c8395d` |
| `perf_sipp_parity` | `point` | 20 | — | — | — | 0 | 54.83 | 0 | `204478703acd9dd30bb3904e696be530ceff03128f11d2fa930f0d346a0c0a94` |
| `perf_soak_caller` | `point` | 0 | — | 1 | 19655/19655 | 117.506 | 175.82 | 0 | `8dbff563613c5eb865622a80f421af86ab36186ae87c8aee4bcaa271be7520f4` |
| `perf_soak_receiver` | `point` | 0 | — | — | — | — | 171.35 | 0 | `2e9e3bc8c387d1b7d6f9e3b7c11da9aa1da39c0e46cab8cdce030f4be5e40aa2` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.808 | 376.56 | 0 | `7afa8a05764cc2273466bc408f3cec705c93a9d9feba5430b7b273ce5b8f4de5` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.56 | 1 | 32500/32500 | 1.897 | 1183.75 | 0 | `ec664eecad80afdbc1dbda1190f177a7e9fe01986296dd3eda616fe4af744ef4` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.11 | 1 | 65000/65000 | 2.271 | 2088.92 | 0 | `68748a46dbac7feb0f5c7a2b563af4d69002067572e07d04d523454235244a60` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.965 | 270.21 | 0 | `8f2f28dda4633a8dc433020bd6e4e0814e48980ae6db0a190c87e26226d2902e` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.56 | 1 | 9750/9750 | 1.611 | 585.33 | 0 | `ea2e57b376005211af5a29d3d78697f5f7f4fe3c3eaabf4d5a5ad8ab42906a47` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.11 | 1 | — | — | — | — | `ebf717e0c593fa7d993cf67af76763ce6fd4c72e27eb4dcd8e0c35c67aec0525` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.11 | 1 | 65000/65000 | 2.271 | 2088.92 | 0 | `68748a46dbac7feb0f5c7a2b563af4d69002067572e07d04d523454235244a60` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.11 | 1 | 65000/65000 | 2.271 | 2088.92 | 0 | `68748a46dbac7feb0f5c7a2b563af4d69002067572e07d04d523454235244a60` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.299 | 376.6 | 0 | `050450d908e94f67bdab6db0314d07193e456d156217869b0f92b9ed2a988f0d` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.56 | 1 | 32500/32500 | 1.855 | 1179.92 | 0 | `ed21dcabca9d3edf84f25c8127eca5e160d329c268d7ca85dfd5977eb9de615e` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 2.29 | 2098.78 | 0 | `774b5daf3345ba7d3fd75b54c3a4c5f48913d0be1b98a3f4a724fde3c3a7213b` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.854 | 269.6 | 0 | `bc35c424fe708c5acf15981bfdb8232fc4c503b947551cdb0c694fb50fff01f5` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.626 | 584.39 | 0 | `41fbe2fc960a2dc040689c037d4f1f6524c0f735bc9030c0ef65e8c9f8e187af` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.1 | 1 | — | — | — | — | `3364835ebe737af2db04b7853f178faa32f2316c0b8e2bbf56b20c0283fd1c5d` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 2.29 | 2098.78 | 0 | `774b5daf3345ba7d3fd75b54c3a4c5f48913d0be1b98a3f4a724fde3c3a7213b` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 2.29 | 2098.78 | 0 | `774b5daf3345ba7d3fd75b54c3a4c5f48913d0be1b98a3f4a724fde3c3a7213b` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.388 | 376.83 | 0 | `8e5b5316012cedb0a8dfb6381da760fbc02ea9bf253bb98cb3c33c2d6802aee2` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.55 | 1 | 32500/32500 | 1.909 | 1183.3 | 0 | `b3a280f075bf49e11021e47e7e83f2de03623f695c65f9487f834edaff4cac01` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.14 | 1 | 65000/65000 | 2.275 | 2096.71 | 0 | `1cfb1cfba6a7df63394bc3364c94094dafedd9ba1d2b1f25fb5f7ca138eda4a6` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.756 | 270.56 | 0 | `516cf618e1024c987c87207cb57dc9722a2006f9fa19519f30416ec1491f835d` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.372 | 582.82 | 0 | `02d9a6b58db78d1236376ee3666fe492300f90e99e6a0a2b57217218fe28ce1e` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.14 | 1 | — | — | — | — | `aa036a996e41e713b219cef5021d014365e17b2cd356aaad6fcf8a32ad533887` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.14 | 1 | 65000/65000 | 2.275 | 2096.71 | 0 | `1cfb1cfba6a7df63394bc3364c94094dafedd9ba1d2b1f25fb5f7ca138eda4a6` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.14 | 1 | 65000/65000 | 2.275 | 2096.71 | 0 | `1cfb1cfba6a7df63394bc3364c94094dafedd9ba1d2b1f25fb5f7ca138eda4a6` |
| `perf_soak_30min` | `point` | 0 | — | 1 | 587/587 | 16.138 | 115.25 | 0 | `36fcee018f4ad537624b49c0136ba3332d89f42468d887e2279786570419f3b1` |

## Interpretation

- The rows are observations, not individually invented PASS verdicts. Their governing performance, soak, regression, cleanup, and evidence-integrity gates are PASS in the complete gate report.
- The supported general full-media beta claim remains up to 2,000 CPS with media enabled. Results above 2,000 CPS remain tuned or experimental and require their own topology, hardware, and qualification evidence.
- Call-setup sweeps use loopback networking on the recorded EC2 qualification host. They establish repeatable release regression evidence, not public-network latency or carrier capacity.
- Full JSON, resource windows, diagnostics, and scenario-specific counters remain in the GitHub evidence artifact; this report intentionally avoids flattening non-equivalent metrics into one score.

## Evidence paths

- `_perf-results/ec2-performance-1/perf_mass_teardown_stress.json` — `29622d7cabf35fa94c3334b0bd018f2f9b7b2c0320bd8e2daf03b591a2b9a3ce`
- `_perf-results/ec2-performance-1/perf_mid_call_signal_under_media.json` — `9fc5cfdcf36bed0387d2a95d0f531eb73c65585f7d2a7c1a3fdc4c7da6c85404`
- `_perf-results/ec2-performance-1/perf_mixed_workload.json` — `570c606a467302a3e6ca41c2da180da76b57bf31251e2046e687dbf144f8662a`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_endpoint.json` — `abdc213395435b3ba742d3024f8fa26f3acdfe2b98d9795ef604c423ea66f49f`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/100.json` — `14da3c242ad73da96e5824716aed2802dfc49379895e06785190f001883ab620`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/1000.json` — `f02fa24d0daa9ebb0e2244106c6f8c7878028bb1131de9c6d0b6090470c26843`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/2000.json` — `e3f49b7842ff48521edd0529ec5fde5afd06c47d90e4be2abc64d4f646082990`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/30.json` — `f4c0f113ff5097c1d8456bd41b8826a95880aec8ae476e8a9bde6eacfea25592`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/300.json` — `cb89910d67e8ad721a7e31d79cc3cc0003cf091219e29f6c4caf99b20d9ff427`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/_sweep.json` — `c55c661542bfaa9f2775b32ac09e34bce006993141acc96b67960c7883707470`
- `_perf-results/ec2-performance-2/perf_concurrent_active_calls.json` — `49af1cf75033040172b436feba0fdb88e658485db2a2ae87b4b9a510a81f5c8b`
- `_perf-results/ec2-performance-3/perf_ai_agent_load.json` — `e0ffda6f373d9e5960352a3931f2a36fe0ee8581ea80bd18dc8ec4ef9e7e36df`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/100.json` — `25795069b5c7b1023412b044718ee634c79a3553c0e6dbd9c365b6d2094d96c7`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/1000.json` — `174a5ba65840d582771aa07cb732a86a4aa301a0666f4e6c379a77a348a49be5`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/2000.json` — `9865082893a07d078aa478395c2a9a0c2f23f837068f73ebb2fed68f677b2bd5`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/30.json` — `06317661c76c846e1febb9a467f2fc9fc62186401955fad330f2a941edb4bbce`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/300.json` — `b3017a655bd2a387efc976c4b5831d49466fcc30157092e044de8ff74bfa6c99`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/_sweep.json` — `9c374b5dde7dcba97a6152b5e18a559c7a2a1e0bb675a3b9e917e66fa6e7f7d8`
- `_perf-results/ec2-performance-3/perf_session_churn_leak.json` — `82e575998cf6f4ef69ded24dace663ff805edee9c2dfd8d6c3570800092493c5`
- `_perf-results/ec2-performance-3/perf_transport_recovery.json` — `d56981fdde9a1e282a3c29618d6537c495e81df96ba0bfa04bd088d9006cbfa4`
- `_perf-results/ec2-performance-4/perf_pdd_with_180_first.json` — `60c75fa7a3e65f7930e75353414f99a6f55e384e7146a8499da515bd700649a3`
- `_perf-results/ec2-performance-4/perf_registrar_binding_scale.json` — `13f571723a3f1ab5e4b00fd04c70ad8b5bf9a4185ed4bf1d3a6c3228768d11df`
- `_perf-results/ec2-performance-4/perf_registration_throughput.json` — `092d74739f9ff363d164ed65b39ded33574c5f8e9d68faf5f13ff5c32bb46bba`
- `_perf-results/ec2-performance-4/perf_sustained_long_duration_calls.json` — `f40a86e01def916a32d7722037d9966fbd69cdad34b816a3e247316058c890e2`
- `_perf-results/ec2-performance-5/perf_backpressure_step.json` — `c793f53749cfe5d3327ba461221b08bf59eb8f7cc6849240fb127a6301c833ae`
- `_perf-results/ec2-performance-5/perf_contact_center_transfers.json` — `3463a1c22182022fafb962863c772e27131ea0ba418250d98f1b70d4a74156f4`
- `_perf-results/ec2-performance-5/perf_srtp_overhead.json` — `aa5c169b8e0a4cbf299714db72d349a589afec3a56f350119b431830080cf7d0`
- `_perf-results/ec2-performance-5/perf_tls_overhead.json` — `41bbcda2ccff6503b9da8ccd8417e93f7b48616b606feae7c82cd591ef38b462`
- `_perf-results/ec2-performance-6/perf_b2bua_forwarding.json` — `2c91a0c71283dadd934dac74dd5c6aa1352a93489c46742ba9b3b6dd7443dffb`
- `_perf-results/ec2-performance-6/perf_media_churn.json` — `505fa48a013043b5fca3d98525b2f3557ade18be0225ca52037169f6d4e92f56`
- `_perf-results/ec2-performance-6/perf_rtp_steady_state.json` — `88b1838c7b51b527d96bbf9aef887cb114902a0a5412118cf34215a591c8395d`
- `_perf-results/ec2-performance-6/perf_sipp_parity.json` — `204478703acd9dd30bb3904e696be530ceff03128f11d2fa930f0d346a0c0a94`
- `_perf-results/ec2-performance-soak-long-1/perf_soak_caller.json` — `8dbff563613c5eb865622a80f421af86ab36186ae87c8aee4bcaa271be7520f4`
- `_perf-results/ec2-performance-soak-long-1/perf_soak_receiver.json` — `2e9e3bc8c387d1b7d6f9e3b7c11da9aa1da39c0e46cab8cdce030f4be5e40aa2`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `7afa8a05764cc2273466bc408f3cec705c93a9d9feba5430b7b273ce5b8f4de5`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `ec664eecad80afdbc1dbda1190f177a7e9fe01986296dd3eda616fe4af744ef4`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `68748a46dbac7feb0f5c7a2b563af4d69002067572e07d04d523454235244a60`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `8f2f28dda4633a8dc433020bd6e4e0814e48980ae6db0a190c87e26226d2902e`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `ea2e57b376005211af5a29d3d78697f5f7f4fe3c3eaabf4d5a5ad8ab42906a47`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `ebf717e0c593fa7d993cf67af76763ce6fd4c72e27eb4dcd8e0c35c67aec0525`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `68748a46dbac7feb0f5c7a2b563af4d69002067572e07d04d523454235244a60`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T225108Z_clean_dbubgu/report.json` — `68748a46dbac7feb0f5c7a2b563af4d69002067572e07d04d523454235244a60`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `050450d908e94f67bdab6db0314d07193e456d156217869b0f92b9ed2a988f0d`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `ed21dcabca9d3edf84f25c8127eca5e160d329c268d7ca85dfd5977eb9de615e`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `774b5daf3345ba7d3fd75b54c3a4c5f48913d0be1b98a3f4a724fde3c3a7213b`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `bc35c424fe708c5acf15981bfdb8232fc4c503b947551cdb0c694fb50fff01f5`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `41fbe2fc960a2dc040689c037d4f1f6524c0f735bc9030c0ef65e8c9f8e187af`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `3364835ebe737af2db04b7853f178faa32f2316c0b8e2bbf56b20c0283fd1c5d`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `774b5daf3345ba7d3fd75b54c3a4c5f48913d0be1b98a3f4a724fde3c3a7213b`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T230622Z_clean_aR6kVr/report.json` — `774b5daf3345ba7d3fd75b54c3a4c5f48913d0be1b98a3f4a724fde3c3a7213b`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `8e5b5316012cedb0a8dfb6381da760fbc02ea9bf253bb98cb3c33c2d6802aee2`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `b3a280f075bf49e11021e47e7e83f2de03623f695c65f9487f834edaff4cac01`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `1cfb1cfba6a7df63394bc3364c94094dafedd9ba1d2b1f25fb5f7ca138eda4a6`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `516cf618e1024c987c87207cb57dc9722a2006f9fa19519f30416ec1491f835d`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `02d9a6b58db78d1236376ee3666fe492300f90e99e6a0a2b57217218fe28ce1e`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `aa036a996e41e713b219cef5021d014365e17b2cd356aaad6fcf8a32ad533887`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `1cfb1cfba6a7df63394bc3364c94094dafedd9ba1d2b1f25fb5f7ca138eda4a6`
- `_perf-results/ec2-performance-soak-long-2/profiles/20261123T232135Z_clean_sjgM9D/report.json` — `1cfb1cfba6a7df63394bc3364c94094dafedd9ba1d2b1f25fb5f7ca138eda4a6`
- `_perf-results/ec2-performance-soak-long-3/perf_soak_30min.json` — `36fcee018f4ad537624b49c0136ba3332d89f42468d887e2279786570419f3b1`
