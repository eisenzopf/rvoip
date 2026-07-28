# Beta Performance Report

> Current canonical performance evidence for candidate `20260724T231400Z` (tested commit `8d44fb35`). This replaces historical current values. No performance or soak workload was rerun to generate this report.

Current release train and runtime crate version: `0.2.5`.

## Release performance policy

- Full application audio-frame delivery was enabled (`RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0`).
- High-density media burst: exactly 160 CPS, ASR at least 0.995, RSS slope at most 15 MB/hour.
- Canonical 2K evidence: three clean passes from the tested source and common executable.
- General beta performance claim: up to 2,000 CPS with media enabled under the recorded profile.
- Monolithic and split soaks: full delivery, zero post-drain retention, and applicable RSS slope at most 15 MB/hour.

## Canonical 2K three-pass evidence

| Run | Target CPS | Achieved CPS | ASR | Offered | Succeeded | Retained after drain | Evidence SHA-256 |
|---:|---:|---:|---:|---:|---:|---:|---|
| 1 | 2000.0 | 1857.12 | 1.0 | 65000 | 65000 | 0 | `2511317c19c17fcbb65b6da970c701827ea7835ce41c83df52a1b0984c141393` |
| 2 | 2000.0 | 1857.13 | 1.0 | 65000 | 65000 | 0 | `dc7b048f25adbf1bf6e010c0e6f80e8d5bca46201372785967e662e1b1f228fd` |
| 3 | 2000.0 | 1857.02 | 1.0 | 65000 | 65000 | 0 | `f17c616334f4cee4663b7216bb520beedd2d6438fdde368cf2438b4b374c41c4` |

These are three distinct canonical executions, all at 2,000 target CPS with media enabled and full delivery. Their common executable and source identity are bound by `canonical-2k/index.json` in the source package.

## Complete call-setup profile matrix

| Profile | Target CPS | Achieved CPS | ASR | Offered | Succeeded |
|---|---:|---:|---:|---:|---:|
| `endpoint` | 30.0 | 27.86 | 1.0 | 975 | 975 |
| `pbx-media-server` | 30.0 | 27.86 | 1.0 | 975 | 975 |
| `pbx-media-server` | 100.0 | 92.85 | 1.0 | 3250 | 3250 |
| `pbx-media-server` | 300.0 | 278.56 | 1.0 | 9750 | 9750 |
| `pbx-media-server` | 1000.0 | 928.53 | 1.0 | 32500 | 32500 |
| `pbx-media-server` | 2000.0 | 1857.12 | 1.0 | 65000 | 65000 |
| `signaling-only-server-high-performance` | 30.0 | 27.86 | 1.0 | 975 | 975 |
| `signaling-only-server-high-performance` | 100.0 | 92.85 | 1.0 | 3250 | 3250 |
| `signaling-only-server-high-performance` | 300.0 | 278.56 | 1.0 | 9750 | 9750 |
| `signaling-only-server-high-performance` | 1000.0 | 928.56 | 1.0 | 32500 | 32500 |
| `signaling-only-server-high-performance` | 2000.0 | 1857.07 | 1.0 | 65000 | 65000 |

## High-density full-delivery media burst

**PASS** — 17965/18000 calls, ASR 0.9981, 10222582 application audio frames delivered, peak 9873 active calls.

| Metric | Requirement | Observed | Result |
|---|---|---|---|
| `media_burst_cps` | exactly 160 | `160.0` | PASS |
| `minimum_asr` | exactly 0.995 | `0.995` | PASS |
| `rss_limit_mb_per_hr` | exactly 15 | `15.0` | PASS |
| `full_audio_frame_delivery` | enabled for caller and receiver | `{"caller_skip":false,"receiver_skip":false}` | PASS |
| `asr` | >= 0.995 | `0.9981` | PASS |
| `timeout_failures` | <= 0.5% and exactly reconciled | `{"count":35,"percent":0.19444444444444445}` | PASS |
| `non_timeout_errors` | 0 | `0` | PASS |
| `caller_retained_after_drain` | 0 | `0` | PASS |
| `receiver_retained_after_drain` | 0 | `0` | PASS |
| `receiver_active_audio_receivers_after_drain` | 0 | `0` | PASS |
| `caller_transaction_manager_after_drain` | 0 | `0` | PASS |
| `receiver_transaction_manager_after_drain` | 0 | `0` | PASS |
| `delivered_audio_frames` | > 0 | `10222582` | PASS |
| `caller_rss_gate_mb_per_hr` | <= 15 | `0.0` | PASS |
| `receiver_rss_gate_mb_per_hr` | <= 15 | `-0.0` | PASS |

## Monolithic soak

**PASS** — 587/587 calls, 5379777 application audio frames delivered, RSS gate slope 11.97 MB/hour.

| Metric | Requirement | Observed | Result |
|---|---|---|---|
| `duration_secs` | exactly 3600 | `3600` | PASS |
| `active_calls_target` | exactly 30 | `30` | PASS |
| `rss_limit_mb_per_hr` | exactly 15 | `15.0` | PASS |
| `errors` | 0 | `0` | PASS |
| `retained_after_drain` | 0 | `0` | PASS |
| `active_audio_receivers_after_drain` | 0 | `0` | PASS |
| `transaction_manager_after_drain` | 0 | `0` | PASS |
| `transaction_runner_after_drain` | 0 | `0` | PASS |
| `controlled_drain_failed` | 0 | `0` | PASS |
| `call_completion` | all offered calls succeed | `{"offered":587,"succeeded":587}` | PASS |
| `delivered_audio_frames` | > 0 | `5379777` | PASS |
| `rss_gate_growth_mb_per_hr` | <= 15 | `11.97` | PASS |

## Split soak

| Role | Configured duration seconds | Offered | Completed | Delivered frames | RSS gate MB/hour | Retained after drain | Full delivery | Evidence SHA-256 |
|---|---:|---:|---:|---:|---:|---:|---|---|
| caller | 3600 | 9904 | 9904 | not set | 0.52 | 0 | yes | `e710d06b98123a9b704242a7f81924ec95fd43b09c48c1c046e3b9bb6072b47d` |
| receiver | 3600 | not set | 9904 | 89692369 | 6.25 | 0 | yes | `a18cef5be1a5dfd4b814852d035b851ad139c473ab2ef85c95f4974b7290bb8a` |

## Regression evidence

- Result: **PASS**.
- Reviewed baseline: `{"absent_reason":null,"baseline_id":"20260706T181609Z","comparison_paths":["perf_call_setup_cps_pbx-media-server/2000.json"],"files":[{"baseline_path":"perf_call_setup_cps_pbx-media-server/30.json","bytes":233,"path":"perf-regression-baseline/perf-results/perf_call_setup_cps_pbx-media-server/30.json","sha256":"506ad2fb7150b0d065f1e327c26214a92f0bc699e81ddea2ed18ff2861dd7760"},{"baseline_path":"perf_call_setup_cps_pbx-media-server/100.json","bytes":236,"path":"perf-regression-baseline/perf-results/perf_call_setup_cps_pbx-media-server/100.json","sha256":"8f3cee86869e83fe3ddbf1e2d0b0d665b84e9e9ae61ed96eb5586ca855db6672"},{"baseline_path":"perf_call_setup_cps_pbx-media-server/300.json","bytes":236,"path":"perf-regression-baseline/perf-results/perf_call_setup_cps_pbx-media-server/300.json","sha256":"d0efecc4f207233abf2fd0e313663fbc5e634867fc819ed72cfd1723a2ca6afc"},{"baseline_path":"perf_call_setup_cps_pbx-media-server/1000.json","bytes":239,"path":"perf-regression-baseline/perf-results/perf_call_setup_cps_pbx-media-server/1000.json","sha256":"97a5021c8aa1fde8f49256aeb059ea94ad308b166c49ec6950545bdafeda290f"},{"baseline_path":"perf_call_setup_cps_pbx-media-server/2000.json","bytes":14982,"path":"perf-regression-baseline/perf-results/perf_call_setup_cps_pbx-media-server/2000.json","sha256":"6d55df11e169ff22dd955c466c02cb86a506830559e3f9d2aa03fc2b86f417c3"},{"baseline_path":"perf_call_setup_cps_pbx-media-server/_sweep.json","bytes":80,"path":"perf-regression-baseline/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json","sha256":"cbe5567367454969e2d88f1f1865fd8b3e691c1832013ec15488060ed1366ddf"}],"input_path":"inputs/performance-regression-baseline.json","input_sha256":"739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9","manifest_path":"perf-regression-baseline/manifest.json","manifest_sha256":"739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9","qualification":{"permitted_use":"reviewed immutable performance-regression threshold input only; this dirty historical run is not release evidence","release_evidence":false},"source":{"git_revision":"a3c84d8f","git_status":"dirty"},"status":"captured"}`.
- Audit evidence: `perf-audit.md` (SHA-256 `13d02a1878228921c5efd796d19e5ad86df8861737efe8ca2f02f62652d34032`).

## Performance JSON artifact inventory

All **59** JSON files under the packaged `perf-results/` tree are listed. Primary results and supporting build/source provenance are distinguished by role; none is silently omitted.

| # | Role | Evidence | SHA-256 | Bytes |
|---:|---|---|---|---:|
| 1 | `workload` | `perf-results/perf_ai_agent_load.json` | `2f243c7d39b414b38dbc429767cddaf0bd4a02f4bd37fcde5f9b47190d2bc8ca` | 2503 |
| 2 | `workload` | `perf-results/perf_b2bua_forwarding.json` | `d08684ee832789be1375b0487116051d82e861438a562183f0cf0a0e111f2e5e` | 2440 |
| 3 | `workload` | `perf-results/perf_backpressure_step.json` | `555056dea7f78552446ea16a33f85c50a76cdfb01d7dd24a84b393393dbb6df2` | 2865 |
| 4 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/access-edge-microburst/perf_burst_caller_access-edge-microburst.json` | `04804fce29ff3b7f9bbb6e1e71ec98cb9487559434726f19e23c8c7041fd4589` | 436577 |
| 5 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/access-edge-microburst/perf_burst_receiver_access-edge-microburst.json` | `0963f35318265d8249bc374629e941391206c54fcee86b32f77780e098523985` | 101164 |
| 6 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/buffer-ab-legacy/perf_burst_caller_buffer-ab-legacy.json` | `e1400ecc612c6ae4328df6069bd52a7dbe84c158c97dde63619b6e2ee9dbbb2d` | 190077 |
| 7 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/buffer-ab-legacy/perf_burst_receiver_buffer-ab-legacy.json` | `94908331c2375edc98e918ced57cd348eab4339ce243537065cd8fc2b1a6aa7e` | 98720 |
| 8 | `build-provenance` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/build/perf_burst_caller-artifact.json` | `a9baa61eb9c401f7d0a58fae779ce66e25bcd3fe5188882e71d970d7d60e3e14` | 1874 |
| 9 | `build-provenance` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/build/perf_burst_receiver-artifact.json` | `7ae61e3b1a18037d0ba64e37bb61bca9fa71ebaf508dbfc69d259fd0dcc9b4bb` | 1884 |
| 10 | `build-provenance` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/build/source-after-build.json` | `0f9007412bb9ca8bd30da9425cd5a483b0f9d83f0f440407036751912513ad30` | 209 |
| 11 | `build-provenance` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/build/source-at-build.json` | `0f9007412bb9ca8bd30da9425cd5a483b0f9d83f0f440407036751912513ad30` | 209 |
| 12 | `build-provenance` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/build/source-at-finalize.json` | `0f9007412bb9ca8bd30da9425cd5a483b0f9d83f0f440407036751912513ad30` | 209 |
| 13 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/carrier-smoke/perf_burst_caller_carrier-smoke.json` | `ba552c36eb7551848c951e1896067d6780d14a5ed6d30e3717584e1f86b7d06f` | 128696 |
| 14 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/carrier-smoke/perf_burst_receiver_carrier-smoke.json` | `0f5d307565e7c9982f7c5c80287d711124d9e7935b5d61eb0a32f5874825cb60` | 98344 |
| 15 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/contact-center-flash/perf_burst_caller_contact-center-flash.json` | `babe9358f11fe3007b2bbaa60b0fdda80806ec03e3e88a0e2006523762e33242` | 273188 |
| 16 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/contact-center-flash/perf_burst_receiver_contact-center-flash.json` | `18aa1117bb1e8ff2cc00ecd10471afde58ee946f5a320586ae6b18b432ee0205` | 100962 |
| 17 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/high-density-media-burst/perf_burst_caller_high-density-media-burst.json` | `e9a6f50cc70f32e44439449eca78fd3545bf708e470f63a197ce8e8cc1f04f32` | 435940 |
| 18 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/high-density-media-burst/perf_burst_receiver_high-density-media-burst.json` | `3a8a50b1adae308675b45c10f2621081914b02f04f73221147642f9183caf224` | 101366 |
| 19 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/overload-recovery/perf_burst_caller_overload-recovery.json` | `c82c281d593f5700f94297b20085c22a4f6ae95519009a6a28bfd09053680683` | 149957 |
| 20 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/overload-recovery/perf_burst_receiver_overload-recovery.json` | `0e263127739111a919b27db05cd4d48de0fe7e8bca896cea83be4b594f3b7217` | 99025 |
| 21 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/shift-change-long-hold/perf_burst_caller_shift-change-long-hold.json` | `d6658b8b7281e26929bda14ddad8de9db0285c342efcd3e46fc303d97a69320f` | 274783 |
| 22 | `burst` | `perf-results/perf_burst_matrix/burst_20260724_211035_72834/shift-change-long-hold/perf_burst_receiver_shift-change-long-hold.json` | `08772f00d5e0aa0bc9bc39277f01d2e694a942ae6f4c8237cf3918858fcf3718` | 102325 |
| 23 | `matrix-point` | `perf-results/perf_call_setup_cps_endpoint.json` | `139a61eb442c169ec451f8b2f70cf1a2b7080622644e5b43f0e7ae8efd98b4d4` | 16147 |
| 24 | `matrix-point` | `perf-results/perf_call_setup_cps_pbx-media-server/100.json` | `88b8701d89fd431560f4ab23e1edad1708c5630afeb72e49144bc768c5d3dfea` | 23462 |
| 25 | `matrix-point` | `perf-results/perf_call_setup_cps_pbx-media-server/1000.json` | `bb478052b732a019bbd6a3d542b239bbb5caed4cbc098fb6cafee2bd32fb08cc` | 23713 |
| 26 | `matrix-point` | `perf-results/perf_call_setup_cps_pbx-media-server/2000.json` | `3557ce4c3e872913ffb1bf77a02bcd2a00f25b64704c6fcb6acd7feda5021988` | 23861 |
| 27 | `matrix-point` | `perf-results/perf_call_setup_cps_pbx-media-server/30.json` | `33efac3f7d7c625332999b7bea11e1dc0d533ce7439cb2afd1c4cf6622f6702e` | 23325 |
| 28 | `matrix-point` | `perf-results/perf_call_setup_cps_pbx-media-server/300.json` | `0bbdea5e573bf66e6f9a02abc0d4b848bf61f610b50e121eac1667f59d04050d` | 23591 |
| 29 | `matrix-summary` | `perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` | `7beba642eb1ad2f05c11829971c9d1578125cbb5f91733e4a481d630e0fac9ef` | 131822 |
| 30 | `matrix-point` | `perf-results/perf_call_setup_cps_signaling-only-server-high-performance/100.json` | `468fb1e516de635b5c372e575c9fb87d2a2a975ddb00d09155b0439eb9c5f90b` | 23608 |
| 31 | `matrix-point` | `perf-results/perf_call_setup_cps_signaling-only-server-high-performance/1000.json` | `53d8c96f83f77e1ecd733c79b082ca2749674d2a97870774c0a229943eb15ce9` | 23876 |
| 32 | `matrix-point` | `perf-results/perf_call_setup_cps_signaling-only-server-high-performance/2000.json` | `0d05cb3a79e2c22c495643daa576eca392f36967b5b9f7c097d3f9ac3138044a` | 24025 |
| 33 | `matrix-point` | `perf-results/perf_call_setup_cps_signaling-only-server-high-performance/30.json` | `927a6746c5ca8fda7e3b820d8c1e3921893f2434160afcc2175a6af5115dc543` | 23454 |
| 34 | `matrix-point` | `perf-results/perf_call_setup_cps_signaling-only-server-high-performance/300.json` | `b170e207eea3d24bbcaa7d0fae1904fe872c5814f5d555d3964131a37ce3a26f` | 23762 |
| 35 | `matrix-summary` | `perf-results/perf_call_setup_cps_signaling-only-server-high-performance/_sweep.json` | `432a0b775e61971547e34dcd85249189ff90d72f092056e50f79491fcdf47fda` | 132637 |
| 36 | `workload` | `perf-results/perf_concurrent_active_calls.json` | `4a09ab652882009cb5493ed94b444e192862b12709e62cd5218ba1da80572134` | 2567 |
| 37 | `workload` | `perf-results/perf_contact_center_transfers.json` | `baf9803192d903a729485609f517d2a95de439ed769729b473dd1b53f2729388` | 2553 |
| 38 | `workload` | `perf-results/perf_mass_teardown_stress.json` | `d07fbbe6c3356f9e2dc893baa554fc82f4a0ab26a0bb3f8fbba2c3b08ebeb50c` | 46599 |
| 39 | `workload` | `perf-results/perf_media_churn.json` | `9e87693b780a303222374539740d787a6fcdf6060ce696442857e326707baa13` | 2681 |
| 40 | `workload` | `perf-results/perf_mid_call_signal_under_media.json` | `ead490ff150e97dafab587070baab2b94b8c386004fa8992187fb7b6a429dd31` | 2670 |
| 41 | `workload` | `perf-results/perf_mixed_workload.json` | `8ae20d1cbeae25c9e57c776e3a5508b06791cef1cc77a2c45bae8e5b92ee2e3b` | 2577 |
| 42 | `workload` | `perf-results/perf_pdd_with_180_first.json` | `9cab6da34c7da3a70fbbc9990ba5cc3078f1ef2eded3c38c1f5babd12b7f37d5` | 2707 |
| 43 | `workload` | `perf-results/perf_registrar_binding_scale.json` | `a2809c126f1d10a690d7ece23ee86b79edfd787e12490a0229e5c4d0c4f01836` | 2561 |
| 44 | `workload` | `perf-results/perf_registration_throughput.json` | `6a89cf92045ae23ba8ad5cfd9e3f292b358257899a483d294e86fea38462a5a0` | 2268 |
| 45 | `workload` | `perf-results/perf_rtp_steady_state.json` | `405cd18e6c0d8eb41dfeac2f49494be936c88ea687444de26b36fa7c961652e1` | 2586 |
| 46 | `workload` | `perf-results/perf_session_churn_leak.json` | `b940d62966e162d233eb15505adbdcb8aef1797aec0e978e7bb9e127ba0bc3b6` | 82775 |
| 47 | `workload` | `perf-results/perf_sipp_parity.json` | `f3e915f51e6dfbcf0bc3caca8f94392e76e739dfdf987e0e329d568bb532ea0e` | 2250 |
| 48 | `soak` | `perf-results/perf_soak_30min.json` | `9161e389212a9ba1444279e29bf78fbaa882baf422fdd8bd37c5d9209331252b` | 173724 |
| 49 | `soak` | `perf-results/perf_soak_caller.json` | `e710d06b98123a9b704242a7f81924ec95fd43b09c48c1c046e3b9bb6072b47d` | 62020 |
| 50 | `soak` | `perf-results/perf_soak_receiver.json` | `a18cef5be1a5dfd4b814852d035b851ad139c473ab2ef85c95f4974b7290bb8a` | 62375 |
| 51 | `build-provenance` | `perf-results/perf_soak_split_20260724_224329_99410/build/perf_soak_caller-artifact.json` | `5ea0646aa44629071273b3610beb0e2114dedb7bd30f4b77486c7a5a1c61b2b1` | 1861 |
| 52 | `build-provenance` | `perf-results/perf_soak_split_20260724_224329_99410/build/perf_soak_receiver-artifact.json` | `0dbfbed99cb682b4c785b9576d1c30fe6fffd08c02e7332e4fa7e81e0c1eb9d6` | 1871 |
| 53 | `build-provenance` | `perf-results/perf_soak_split_20260724_224329_99410/build/source-after-build.json` | `0f9007412bb9ca8bd30da9425cd5a483b0f9d83f0f440407036751912513ad30` | 209 |
| 54 | `build-provenance` | `perf-results/perf_soak_split_20260724_224329_99410/build/source-at-build.json` | `0f9007412bb9ca8bd30da9425cd5a483b0f9d83f0f440407036751912513ad30` | 209 |
| 55 | `build-provenance` | `perf-results/perf_soak_split_20260724_224329_99410/build/source-at-finalize.json` | `0f9007412bb9ca8bd30da9425cd5a483b0f9d83f0f440407036751912513ad30` | 209 |
| 56 | `workload` | `perf-results/perf_srtp_overhead.json` | `9df549a7b7fa2dcb1319d40d01a3635d4602b7adf3f8b82a13b84e5615108bf0` | 3011 |
| 57 | `workload` | `perf-results/perf_sustained_long_duration_calls.json` | `18798d37f641ac05b52c88755ebc36f1f173a98cedf4c534ad1fd5851996d578` | 2384 |
| 58 | `workload` | `perf-results/perf_tls_overhead.json` | `454a8dc1cc57a611c38625f398efb7ee6f286035c7b9fc54d80185b79f4ffabe` | 2666 |
| 59 | `workload` | `perf-results/perf_transport_recovery.json` | `aa166b1d22704e7ed355c256ceaad8d049ca00aa545d905f15780ed1e44f7b5c` | 2932 |

## Interpretation

PASS establishes the recorded thresholds only for this source, executable, host, loopback topology, configurations, durations, and workloads. It does not claim untested Internet conditions, hardware, concurrency, codecs, peers, or sustained durations.
