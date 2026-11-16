# RVoIP Performance Qualification Report

> Exact-candidate observations archived by protected run [38195041080](https://github.com/eisenzopf/rvoip/actions/runs/38195041080). A blank cell means the scenario does not emit that metric; it does not mean zero. Measurements belonging only to an accepted prior-run receipt are not copied into this table.

Release: `0.4.0` · commit: `f4d528eaabbac935bce9a6404d3ce4fdec35ed0f` · qualification: **PASS**.

## Archived measurements

| Scenario | Kind | Target CPS | Achieved CPS | ASR | Calls | Setup p99 ms | Peak RSS MB | Duration s | Evidence SHA-256 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| `perf_mass_teardown_stress` | `point` | 30 | — | — | — | — | 271.93 | 0 | `6fe96f1277fcc8e3bcbbd7e189608aacc00754d5a7c4fc3eea23036038f2624c` |
| `perf_mid_call_signal_under_media` | `point` | 30 | — | 1 | —/30 | 16.572 | 91.86 | 0 | `96ed67993a5aeea8530bf2f98c377b567e74647193e049ae8cba25e9523f2fe5` |
| `perf_mixed_workload` | `point` | 50 | 50 | 1 | — | 2.061 | 130.21 | 0 | `e1556c9d2a9cd44d103fbae477a0181bacbebb060f9abd07eb8c128a47d3fd59` |
| `perf_call_setup_cps_endpoint` | `point` | 30 | 27.86 | 1 | 975/975 | 2.101 | 133.17 | 0 | `98de7668328090dd3a0e9da80c175e8046f21330f97fa2db3f82be6f7ceb86ee` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.587 | 360.93 | 0 | `2b330d38b0f20a7c53c55fc549acc01e5edf5efcfa0785d5a79074c3df51e5d2` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 1000 | 928.56 | 1 | 32500/32500 | 1.863 | 1148.4 | 0 | `5981661e4ed80328b38d9bf733421525929486a7f877db89216bf8d76c92c86e` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 2000 | 1857.12 | 1 | 65000/65000 | 2.654 | 2070.03 | 0 | `6211c8525e269df01fac4c38637c8dd320e7adb4d8ef00bbb05de5121e951277` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 30 | 27.86 | 1 | 975/975 | 1.751 | 259 | 0 | `4f54bbee81007da110a1f7aefcbd177d176ebceb64656baf21951ad36fa00d83` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.431 | 559.23 | 0 | `1ce2a28c4d49dd91dd42f7232288cec34f8154630d5339c0ac217c8cf9cfe059` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `sweep` | 2000 | 1857.12 | 1 | — | — | — | — | `1b3956f1d5bcdf7d101660a32c6c14baaa0934152fdd057621830b34f089fbc0` |
| `perf_concurrent_active_calls` | `point` | 500 | — | 1 | —/500 | 213.778 | 253.96 | 0 | `8c26f523349bf90f6d897bd796b2e5bb26da2d2d050ba045c2910630d96d606e` |
| `perf_ai_agent_load` | `point` | 30 | — | 1 | — | 16.728 | 84.95 | 0 | `1463069144737cb9a80ce9d5a30bd2638a80694e3ec59e100cabb9fa18c5fce8` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.806 | 376.33 | 0 | `ae7ea2a940ccc67dae508c39f316eb78636cebe5ff25337b2e66ef907eccfbf8` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.55 | 1 | 32500/32500 | 2.114 | 1167.38 | 0 | `95ef8ee3115d6c32a516620d82ac6f6cbd20828db4739cdbfc0e4be505bfeb6d` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.8 | 2083.2 | 0 | `ab3b196ddd5b2f6393a315dad35c7cb27ee3484908e5a8028552a331be3525f5` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.923 | 267.36 | 0 | `035e98fd4513c7500c99d4c9ff5d1e348b6c9fd39e37e823b7ab4d13e4f34771` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.627 | 575.07 | 0 | `951b9018f4406fbb40d8a8562ba55fbd2b824a4220da5efd703f6c4c37034d0b` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.13 | 1 | — | — | — | — | `ec564cf145309718f6308614c330fcec772cec1eef3d8a7d735eaaa6ef88f72d` |
| `perf_session_churn_leak` | `point` | 0 | — | — | 250/250 | 1.792 | 98.5 | 0 | `87d2462a4d385b2b31c30dfa8b7614f8b8a655d517200583a7846a3776070ffb` |
| `perf_transport_recovery` | `point` | 5 | — | — | — | — | 113.29 | 0 | `43ecf9b0466f6b01fecb2cc5b54e2bc1897095d7548a4fb262ce69c26ccdd021` |
| `perf_pdd_with_180_first` | `point` | 50 | 46.43 | 1 | 1625/1625 | 1.759 | 171.5 | 0 | `4fd2a3a7ee4967d3df8615b5c2214347c96b215d76ad438112fdfc79cdc32b73` |
| `perf_registrar_binding_scale` | `point` | 100 | — | — | — | — | 89.08 | 0 | `f91283630d9bd3ff09494d30cff47351a9a4f26a92f99edd6f0aea7c3b5c76e7` |
| `perf_registration_throughput` | `point` | 100 | — | — | — | — | 90.61 | 0 | `691ef17652c378b7968ea44dc07380f7995070e75c3daea67f7a9b5ffffb6aff` |
| `perf_sustained_long_duration_calls` | `point` | 30 | 28.85 | 1 | 1875/1875 | 2.087 | 336.54 | 0 | `6e8f7b4f16bc5664b2e0915e5c058a77b9c3be1e6cad64a678e33b2a9d43dd76` |
| `perf_backpressure_step` | `point` | 200 | — | — | 13912/13912 | — | 396.28 | 0 | `a887fd3aa89d268dd38955fc6265fe87dc41a101a4595e880b51d8dc6084ca25` |
| `perf_contact_center_transfers` | `point` | 20 | — | 1 | —/20 | 9.273 | 82.5 | 0 | `4551854f33d4091eccc3abadf206922aa434e3a7f6733c6e332d33410f033b84` |
| `perf_srtp_overhead` | `point` | 50 | — | 1 | —/50 | 24.789 | 93.75 | 0 | `a5eb4ebede2f4381b756687953a842dc0dc95d68c7b3ae6c00a455ac09fd280f` |
| `perf_tls_overhead` | `point` | 100 | 92.85 | 1 | 3250/3250 | 20.726 | 197.97 | 0 | `a8212547160d0a9bec865d7df17a62fa81c136509d6d205958e7e5d5b0833adb` |
| `perf_b2bua_forwarding` | `point` | 30 | 27.86 | 1 | 975/975 | 2.079 | 409.11 | 0 | `57b401989da543f45084c339d9378df6c8c4a00bc852e70fdaaa16aded902bf2` |
| `perf_media_churn` | `point` | 0 | — | — | — | — | — | 0 | `58708efad33cb5f22bd5751ab62650313e8dd84789cd40d9aac1a4e9289a1b74` |
| `perf_rtp_steady_state` | `point` | 50 | — | 1 | —/50 | 26.116 | 93.61 | 0 | `d5deffdc081e801a52ad5ec707d639823a382ed9c1a65c218a88d6940984c924` |
| `perf_sipp_parity` | `point` | 20 | — | — | — | 0 | 54.96 | 0 | `4880410a9e9060f22c0990a29af0486fc5d2c1aee597a189381116c80886cb64` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.576 | 375.98 | 0 | `0c09ccb96224d92c47cbe81c61203b94c9950f32cc334a4b8f9158d9d33caae8` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.55 | 1 | 32500/32500 | 1.855 | 1161.65 | 0 | `094c78a8b2f1f74823675eb946808355a14fb34a7b5c821458d504957cd131b5` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 2.568 | 2082.1 | 0 | `d80d565854c617ef7689d7b77b8838ea027b2f007d6cc9558867b33815379439` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.906 | 266.59 | 0 | `5498932e83657d7a985c27d0fe1d86a5f34d56231f89e59e219af101cd8e9858` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.56 | 1 | 9750/9750 | 1.461 | 573.4 | 0 | `aa80f03ecb936c6001d047976b42b600d2dd52e2de5bf650fff1b5a2ea2c97c6` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.1 | 1 | — | — | — | — | `8f1533b981c5208ba5c722a83cfa3a4f37dbf65ce044c3e792798b5e024ffd9c` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 2.568 | 2082.1 | 0 | `d80d565854c617ef7689d7b77b8838ea027b2f007d6cc9558867b33815379439` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 2.568 | 2082.1 | 0 | `d80d565854c617ef7689d7b77b8838ea027b2f007d6cc9558867b33815379439` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.522 | 375.22 | 0 | `e2604ecd472a6092ec37e97841a8c35d66c9b498bcff074f16f5129d9d86064d` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.56 | 1 | 32500/32500 | 1.9 | 1172.65 | 0 | `cae244bba1db9ebbd8ffd55947cb06b8b02ed9517d5243fddbbdc7c654dad579` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.605 | 2083.6 | 0 | `df40d056592ae01ac44ff0385bdc34dbdac3d0ac63980b8c3120a731aa240605` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.742 | 267.58 | 0 | `9e72c6ddb033ccaed203b55167593bcfe4ef7b0b653425a1fd2aeac685441047` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.448 | 575.27 | 0 | `a4b49128bd6e6824861e4a81050aa2045f522a555a9d3813107a3120c7c8314e` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.13 | 1 | — | — | — | — | `65772e8d7574116e684c761c79887c7ce97a76c009186712cb0b1b252654bb32` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.605 | 2083.6 | 0 | `df40d056592ae01ac44ff0385bdc34dbdac3d0ac63980b8c3120a731aa240605` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.605 | 2083.6 | 0 | `df40d056592ae01ac44ff0385bdc34dbdac3d0ac63980b8c3120a731aa240605` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.551 | 375.83 | 0 | `d7fcd2a901cd265f87cbe7a762dc62d4e9aa92d8a3513b8f1ab21f2c69f312d1` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.54 | 1 | 32500/32500 | 2.084 | 1164 | 0 | `bfb3910a2387a609ef7197663e88362ef1026de1d6515dce34e4389f316ac0a9` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.683 | 2082.64 | 0 | `35fe9d75e632aab2ec8980568fe7f345d08d40530b1cb983dd10ad6021a90f9f` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.753 | 267.14 | 0 | `16556a495b103c5da95b930bd2448f1c308f23ef0f64af51190cd95b6cc1b423` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.509 | 574.4 | 0 | `e3eb384141f2375c683cdf19952d27175367257241e515e41ed3c83f2b2b6cf0` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.13 | 1 | — | — | — | — | `9a9e17eeb4feaaa7307f118c30efac8ca9f98cb8577cb615cdca75216947c6d0` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.683 | 2082.64 | 0 | `35fe9d75e632aab2ec8980568fe7f345d08d40530b1cb983dd10ad6021a90f9f` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 2.683 | 2082.64 | 0 | `35fe9d75e632aab2ec8980568fe7f345d08d40530b1cb983dd10ad6021a90f9f` |
| `perf_soak_caller` | `point` | 0 | — | 1 | 9904/9904 | 166.986 | 169.45 | 0 | `33ac89cef9d63494f390f34642d7dd19bf759ea3d8e489e81b1a06830f45549b` |
| `perf_soak_receiver` | `point` | 0 | — | — | — | — | 183.79 | 0 | `a66ef092258a893984e5e70104d2c215ecf04b970a2e45ae3aec77d27aa57a21` |
| `perf_soak_30min` | `point` | 0 | — | 1 | 587/587 | 15.032 | 119.53 | 0 | `e0efe862aa8e22267ec7b54b94b1142854b5f33cbfbf5237f1d9bbf5288877dc` |

## Interpretation

- The rows are observations, not individually invented PASS verdicts. Their governing performance, soak, regression, cleanup, and evidence-integrity gates are PASS in the complete gate report.
- The supported general full-media beta claim remains up to 2,000 CPS with media enabled. Results above 2,000 CPS remain tuned or experimental and require their own topology, hardware, and qualification evidence.
- Call-setup sweeps use loopback networking on the recorded EC2 qualification host. They establish repeatable release regression evidence, not public-network latency or carrier capacity.
- Full JSON, resource windows, diagnostics, and scenario-specific counters remain in the GitHub evidence artifact; this report intentionally avoids flattening non-equivalent metrics into one score.

## Evidence paths

- `_perf-results/ec2-performance-1/perf_mass_teardown_stress.json` — `6fe96f1277fcc8e3bcbbd7e189608aacc00754d5a7c4fc3eea23036038f2624c`
- `_perf-results/ec2-performance-1/perf_mid_call_signal_under_media.json` — `96ed67993a5aeea8530bf2f98c377b567e74647193e049ae8cba25e9523f2fe5`
- `_perf-results/ec2-performance-1/perf_mixed_workload.json` — `e1556c9d2a9cd44d103fbae477a0181bacbebb060f9abd07eb8c128a47d3fd59`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_endpoint.json` — `98de7668328090dd3a0e9da80c175e8046f21330f97fa2db3f82be6f7ceb86ee`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/100.json` — `2b330d38b0f20a7c53c55fc549acc01e5edf5efcfa0785d5a79074c3df51e5d2`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/1000.json` — `5981661e4ed80328b38d9bf733421525929486a7f877db89216bf8d76c92c86e`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/2000.json` — `6211c8525e269df01fac4c38637c8dd320e7adb4d8ef00bbb05de5121e951277`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/30.json` — `4f54bbee81007da110a1f7aefcbd177d176ebceb64656baf21951ad36fa00d83`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/300.json` — `1ce2a28c4d49dd91dd42f7232288cec34f8154630d5339c0ac217c8cf9cfe059`
- `_perf-results/ec2-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/_sweep.json` — `1b3956f1d5bcdf7d101660a32c6c14baaa0934152fdd057621830b34f089fbc0`
- `_perf-results/ec2-performance-2/perf_concurrent_active_calls.json` — `8c26f523349bf90f6d897bd796b2e5bb26da2d2d050ba045c2910630d96d606e`
- `_perf-results/ec2-performance-3/perf_ai_agent_load.json` — `1463069144737cb9a80ce9d5a30bd2638a80694e3ec59e100cabb9fa18c5fce8`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/100.json` — `ae7ea2a940ccc67dae508c39f316eb78636cebe5ff25337b2e66ef907eccfbf8`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/1000.json` — `95ef8ee3115d6c32a516620d82ac6f6cbd20828db4739cdbfc0e4be505bfeb6d`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/2000.json` — `ab3b196ddd5b2f6393a315dad35c7cb27ee3484908e5a8028552a331be3525f5`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/30.json` — `035e98fd4513c7500c99d4c9ff5d1e348b6c9fd39e37e823b7ab4d13e4f34771`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/300.json` — `951b9018f4406fbb40d8a8562ba55fbd2b824a4220da5efd703f6c4c37034d0b`
- `_perf-results/ec2-performance-3/perf_call_setup_cps_pbx-media-server/_sweep.json` — `ec564cf145309718f6308614c330fcec772cec1eef3d8a7d735eaaa6ef88f72d`
- `_perf-results/ec2-performance-3/perf_session_churn_leak.json` — `87d2462a4d385b2b31c30dfa8b7614f8b8a655d517200583a7846a3776070ffb`
- `_perf-results/ec2-performance-3/perf_transport_recovery.json` — `43ecf9b0466f6b01fecb2cc5b54e2bc1897095d7548a4fb262ce69c26ccdd021`
- `_perf-results/ec2-performance-4/perf_pdd_with_180_first.json` — `4fd2a3a7ee4967d3df8615b5c2214347c96b215d76ad438112fdfc79cdc32b73`
- `_perf-results/ec2-performance-4/perf_registrar_binding_scale.json` — `f91283630d9bd3ff09494d30cff47351a9a4f26a92f99edd6f0aea7c3b5c76e7`
- `_perf-results/ec2-performance-4/perf_registration_throughput.json` — `691ef17652c378b7968ea44dc07380f7995070e75c3daea67f7a9b5ffffb6aff`
- `_perf-results/ec2-performance-4/perf_sustained_long_duration_calls.json` — `6e8f7b4f16bc5664b2e0915e5c058a77b9c3be1e6cad64a678e33b2a9d43dd76`
- `_perf-results/ec2-performance-5/perf_backpressure_step.json` — `a887fd3aa89d268dd38955fc6265fe87dc41a101a4595e880b51d8dc6084ca25`
- `_perf-results/ec2-performance-5/perf_contact_center_transfers.json` — `4551854f33d4091eccc3abadf206922aa434e3a7f6733c6e332d33410f033b84`
- `_perf-results/ec2-performance-5/perf_srtp_overhead.json` — `a5eb4ebede2f4381b756687953a842dc0dc95d68c7b3ae6c00a455ac09fd280f`
- `_perf-results/ec2-performance-5/perf_tls_overhead.json` — `a8212547160d0a9bec865d7df17a62fa81c136509d6d205958e7e5d5b0833adb`
- `_perf-results/ec2-performance-6/perf_b2bua_forwarding.json` — `57b401989da543f45084c339d9378df6c8c4a00bc852e70fdaaa16aded902bf2`
- `_perf-results/ec2-performance-6/perf_media_churn.json` — `58708efad33cb5f22bd5751ab62650313e8dd84789cd40d9aac1a4e9289a1b74`
- `_perf-results/ec2-performance-6/perf_rtp_steady_state.json` — `d5deffdc081e801a52ad5ec707d639823a382ed9c1a65c218a88d6940984c924`
- `_perf-results/ec2-performance-6/perf_sipp_parity.json` — `4880410a9e9060f22c0990a29af0486fc5d2c1aee597a189381116c80886cb64`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `0c09ccb96224d92c47cbe81c61203b94c9950f32cc334a4b8f9158d9d33caae8`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `094c78a8b2f1f74823675eb946808355a14fb34a7b5c821458d504957cd131b5`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `d80d565854c617ef7689d7b77b8838ea027b2f007d6cc9558867b33815379439`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `5498932e83657d7a985c27d0fe1d86a5f34d56231f89e59e219af101cd8e9858`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `aa80f03ecb936c6001d047976b42b600d2dd52e2de5bf650fff1b5a2ea2c97c6`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `8f1533b981c5208ba5c722a83cfa3a4f37dbf65ce044c3e792798b5e024ffd9c`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `d80d565854c617ef7689d7b77b8838ea027b2f007d6cc9558867b33815379439`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T031437Z_clean_fDfXCe/report.json` — `d80d565854c617ef7689d7b77b8838ea027b2f007d6cc9558867b33815379439`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `e2604ecd472a6092ec37e97841a8c35d66c9b498bcff074f16f5129d9d86064d`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `cae244bba1db9ebbd8ffd55947cb06b8b02ed9517d5243fddbbdc7c654dad579`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `df40d056592ae01ac44ff0385bdc34dbdac3d0ac63980b8c3120a731aa240605`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `9e72c6ddb033ccaed203b55167593bcfe4ef7b0b653425a1fd2aeac685441047`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `a4b49128bd6e6824861e4a81050aa2045f522a555a9d3813107a3120c7c8314e`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `65772e8d7574116e684c761c79887c7ce97a76c009186712cb0b1b252654bb32`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `df40d056592ae01ac44ff0385bdc34dbdac3d0ac63980b8c3120a731aa240605`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T032949Z_clean_g3cKtc/report.json` — `df40d056592ae01ac44ff0385bdc34dbdac3d0ac63980b8c3120a731aa240605`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `d7fcd2a901cd265f87cbe7a762dc62d4e9aa92d8a3513b8f1ab21f2c69f312d1`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `bfb3910a2387a609ef7197663e88362ef1026de1d6515dce34e4389f316ac0a9`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `35fe9d75e632aab2ec8980568fe7f345d08d40530b1cb983dd10ad6021a90f9f`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `16556a495b103c5da95b930bd2448f1c308f23ef0f64af51190cd95b6cc1b423`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `e3eb384141f2375c683cdf19952d27175367257241e515e41ed3c83f2b2b6cf0`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `9a9e17eeb4feaaa7307f118c30efac8ca9f98cb8577cb615cdca75216947c6d0`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `35fe9d75e632aab2ec8980568fe7f345d08d40530b1cb983dd10ad6021a90f9f`
- `_perf-results/ec2-performance-soak-long-1/profiles/20261012T034459Z_clean_tsjDf7/report.json` — `35fe9d75e632aab2ec8980568fe7f345d08d40530b1cb983dd10ad6021a90f9f`
- `_perf-results/ec2-performance-soak-long-2/perf_soak_caller.json` — `33ac89cef9d63494f390f34642d7dd19bf759ea3d8e489e81b1a06830f45549b`
- `_perf-results/ec2-performance-soak-long-2/perf_soak_receiver.json` — `a66ef092258a893984e5e70104d2c215ecf04b970a2e45ae3aec77d27aa57a21`
- `_perf-results/ec2-performance-soak-long-3/perf_soak_30min.json` — `e0efe862aa8e22267ec7b54b94b1142854b5f33cbfbf5237f1d9bbf5288877dc`
