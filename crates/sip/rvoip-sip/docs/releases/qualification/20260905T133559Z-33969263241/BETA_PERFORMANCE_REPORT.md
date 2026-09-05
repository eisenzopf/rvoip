# RVoIP Performance Qualification Report

> Exact-candidate observations archived by protected run [33969263241](https://github.com/eisenzopf/rvoip/actions/runs/33969263241). A blank cell means the scenario does not emit that metric; it does not mean zero. Measurements belonging only to an accepted prior-run receipt are not copied into this table.

Release: `0.3.9` · commit: `8cab44b10f872d21b304c02111d5d203ee8226da` · qualification: **PASS**.

## Archived measurements

| Scenario | Kind | Target CPS | Achieved CPS | ASR | Calls | Setup p99 ms | Peak RSS MB | Duration s | Evidence SHA-256 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| `perf_mass_teardown_stress` | `point` | 30 | — | — | — | — | 223.9 | 0 | `c9fdb240919eb52803d80b4b0e8ff2c5902d3e1a3c941680950899b21b126bee` |
| `perf_mid_call_signal_under_media` | `point` | 30 | — | 1 | —/30 | 16.72 | 70.41 | 0 | `ca52b06217d3787f4181a9e88fc8e2b212a09a533c8c5d6a4cd4d6cda1f2ffb7` |
| `perf_mixed_workload` | `point` | 50 | 50 | 1 | — | 2.09 | 113.41 | 0 | `82f0a0e3c1d3825967e22fa5f3ed711fdc0515d5036db7bead4678750b1b0ef2` |
| `perf_call_setup_cps_endpoint` | `point` | 30 | 27.86 | 1 | 975/975 | 2.175 | 116.1 | 0 | `6812ba2c4cf62eac76752c3f8e341d36c58ddc0653ae2c463adfccb92c19c126` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.852 | 314.48 | 0 | `74c0144c4ea6d1530c9591d85b53691739a0ccf5eb51dfdc7f75ca03b319f415` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 1000 | 928.55 | 1 | 32500/32500 | 1.861 | 1066.05 | 0 | `38f507f85c549b73de9603c80316a71ec962d7a26edd2225b6b1c2dcfa9d48ff` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 2000 | 1857.12 | 1 | 65000/65000 | 3.992 | 2004.29 | 0 | `02922c5314c12d399a4a4191577fe31c9a198f94dc7ce58c6df7fa68398aaaac` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 30 | 27.86 | 1 | 975/975 | 1.974 | 218.85 | 0 | `c520ecaa60a49fd6fdb985b2249046159460bec27852dff7de1ef7d98aa290c7` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.663 | 507.87 | 0 | `85ff1ce2491de69cda6244bc9a6f66b164c19b9da6d69a6a387a272faf05f548` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `sweep` | 2000 | 1857.12 | 1 | — | — | — | — | `d9fab0e7adc4103fa6cb063511f7c9638e5e0455dfe7b531bca656a211083542` |
| `perf_concurrent_active_calls` | `point` | 500 | — | 1 | —/500 | 201.72 | 209.41 | 0 | `ab4ee69204229c5bacd8287c3fc50d5fc93c16081a398f0aa7a2e15413abe4c8` |
| `perf_ai_agent_load` | `point` | 30 | — | 1 | — | 17.793 | 62.73 | 0 | `547823b5633f6c72d673d82f88f7fecd162deb00c5fcebd67223f328318e6f43` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.816 | 329.43 | 0 | `2195e2c6b8aa38f203e8d11dd37cf9ae466038de71cdb04936af035c3c8113ae` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.56 | 1 | 32500/32500 | 1.989 | 1082.8 | 0 | `87a93829531bdd3dab5cb04d20cd0b1239825875da0b625844e990c35d6f53d4` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 6.009 | 2020.53 | 0 | `484669a46c51387727551835658fad38febcae3caca1f6d7e58df3af7dba88b6` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 2.084 | 226.61 | 0 | `4d266d745c236548319e107a2c16c140f670f1584bebf6b2a85c56b6bfe0a116` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.595 | 521.37 | 0 | `f94b888a9dcfdf1e164f155196f35103d5a991e91000a8006f74964071515e13` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.1 | 1 | — | — | — | — | `a65c4caf1b387eab317f0d3b798f0abdd809b7dc22f124214cbaf494298c9e97` |
| `perf_session_churn_leak` | `point` | 0 | — | — | 250/250 | 1.77 | 82.3 | 0 | `8a1d5374c34bb7810015b21a93449abf6a3d9eeb97c678e06accaacb3110bf4a` |
| `perf_transport_recovery` | `point` | 5 | — | — | — | — | 82.54 | 0 | `6eabb5fb25eda508a2954c5fb9ab99dabbec3da22e840aafa8ea4716d6ec8091` |
| `perf_pdd_with_180_first` | `point` | 50 | 46.43 | 1 | 1625/1625 | 2.361 | 150.93 | 0 | `8993dc814c527c4027e4614a806c908025644fbfa916be41fc1205c3d1c11d8a` |
| `perf_registrar_binding_scale` | `point` | 100 | — | — | — | — | 79.79 | 0 | `0289f19129561045a0f9a3a8fc206bc61362def947adbd0883534288bcaee41a` |
| `perf_registration_throughput` | `point` | 100 | — | — | — | — | 81.54 | 0 | `0673765245328f5eed726e90d0ec1ba5cae02f82cc8337eb8c658915291e8a2a` |
| `perf_sustained_long_duration_calls` | `point` | 30 | 28.85 | 1 | 1875/1875 | 2.466 | 268.04 | 0 | `0ae27ecb368c0eceb2f92b0ad2e75bd46459b91c7fa49e6fd8c4f468577f3e3f` |
| `perf_backpressure_step` | `point` | 200 | — | — | 13831/13831 | — | 372.16 | 0 | `47610ebebe0fc6d331c7c50d0fa58928b461e8da2cf2b4bbdebd5a74f8ccaa63` |
| `perf_contact_center_transfers` | `point` | 20 | — | 1 | —/20 | 10.052 | 60.86 | 0 | `5a2aad5ea517a66dcb932b43e4367686cc8d1733e090544e161542bc9a682234` |
| `perf_srtp_overhead` | `point` | 50 | — | 1 | —/50 | 27.967 | 71.41 | 0 | `d39c9d56b48cc0fcb6eaa5f319f12961286bd7559ca89e00294fb4894fae6353` |
| `perf_tls_overhead` | `point` | 100 | 92.83 | 0.9997 | 3249/3250 | 40.272 | 179.55 | 0 | `87dcb5db952e9aa2e54465eb674a17299cf522c136f47257488dec6763e68eef` |
| `perf_b2bua_forwarding` | `point` | 30 | 27.86 | 1 | 975/975 | 2.347 | 318.77 | 0 | `7f17da4683890f0e94d9a9feb9455d744956ad3a5119b1d36adc38a52e517424` |
| `perf_media_churn` | `point` | 0 | — | — | — | — | — | 0 | `0b430862e1e1a2cde1f81f540c355624e31b4b230cbdc78f083af0eca50241ac` |
| `perf_rtp_steady_state` | `point` | 50 | — | 1 | —/50 | 28.164 | 70.43 | 0 | `7320698f6531bf0affa1498945b647f08fa3fd7f7552f6a686635dc3e5cb9e81` |
| `perf_sipp_parity` | `point` | 20 | — | — | — | 0 | 45.6 | 0 | `3d240f504a276838a43cd3c5ae1743556fe08e7b487c7afb0da3a696b9335736` |
| `perf_soak_caller` | `point` | 0 | — | 1 | 9904/9904 | 173.539 | 138.64 | 0 | `cef90fb0da41263f953bfb8f993b5bf0dee25ccfb7588ca1caee39f8a67b0236` |
| `perf_soak_receiver` | `point` | 0 | — | — | — | — | 153.8 | 0 | `b8840dc01f906a5c0cee604626b16491e2004c8790d714c0517524e5c9b0e28d` |
| `perf_soak_30min` | `point` | 0 | — | 1 | 587/587 | 22.659 | 101.29 | 0 | `e37be962fe89b36c07b82a7fca4e26eec9cfe54c1fd84c2865f111fa26ba63a1` |

## Interpretation

- The rows are observations, not individually invented PASS verdicts. Their governing performance, soak, regression, cleanup, and evidence-integrity gates are PASS in the complete gate report.
- Call-setup sweeps use loopback networking on the recorded GCP qualification host. They establish repeatable release regression evidence, not public-network latency or carrier capacity.
- Full JSON, resource windows, diagnostics, and scenario-specific counters remain in the GitHub evidence artifact; this report intentionally avoids flattening non-equivalent metrics into one score.

## Evidence paths

- `_perf-results/gcp-performance-1/perf_mass_teardown_stress.json` — `c9fdb240919eb52803d80b4b0e8ff2c5902d3e1a3c941680950899b21b126bee`
- `_perf-results/gcp-performance-1/perf_mid_call_signal_under_media.json` — `ca52b06217d3787f4181a9e88fc8e2b212a09a533c8c5d6a4cd4d6cda1f2ffb7`
- `_perf-results/gcp-performance-1/perf_mixed_workload.json` — `82f0a0e3c1d3825967e22fa5f3ed711fdc0515d5036db7bead4678750b1b0ef2`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_endpoint.json` — `6812ba2c4cf62eac76752c3f8e341d36c58ddc0653ae2c463adfccb92c19c126`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/100.json` — `74c0144c4ea6d1530c9591d85b53691739a0ccf5eb51dfdc7f75ca03b319f415`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/1000.json` — `38f507f85c549b73de9603c80316a71ec962d7a26edd2225b6b1c2dcfa9d48ff`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/2000.json` — `02922c5314c12d399a4a4191577fe31c9a198f94dc7ce58c6df7fa68398aaaac`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/30.json` — `c520ecaa60a49fd6fdb985b2249046159460bec27852dff7de1ef7d98aa290c7`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/300.json` — `85ff1ce2491de69cda6244bc9a6f66b164c19b9da6d69a6a387a272faf05f548`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/_sweep.json` — `d9fab0e7adc4103fa6cb063511f7c9638e5e0455dfe7b531bca656a211083542`
- `_perf-results/gcp-performance-2/perf_concurrent_active_calls.json` — `ab4ee69204229c5bacd8287c3fc50d5fc93c16081a398f0aa7a2e15413abe4c8`
- `_perf-results/gcp-performance-3/perf_ai_agent_load.json` — `547823b5633f6c72d673d82f88f7fecd162deb00c5fcebd67223f328318e6f43`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/100.json` — `2195e2c6b8aa38f203e8d11dd37cf9ae466038de71cdb04936af035c3c8113ae`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/1000.json` — `87a93829531bdd3dab5cb04d20cd0b1239825875da0b625844e990c35d6f53d4`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/2000.json` — `484669a46c51387727551835658fad38febcae3caca1f6d7e58df3af7dba88b6`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/30.json` — `4d266d745c236548319e107a2c16c140f670f1584bebf6b2a85c56b6bfe0a116`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/300.json` — `f94b888a9dcfdf1e164f155196f35103d5a991e91000a8006f74964071515e13`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/_sweep.json` — `a65c4caf1b387eab317f0d3b798f0abdd809b7dc22f124214cbaf494298c9e97`
- `_perf-results/gcp-performance-3/perf_session_churn_leak.json` — `8a1d5374c34bb7810015b21a93449abf6a3d9eeb97c678e06accaacb3110bf4a`
- `_perf-results/gcp-performance-3/perf_transport_recovery.json` — `6eabb5fb25eda508a2954c5fb9ab99dabbec3da22e840aafa8ea4716d6ec8091`
- `_perf-results/gcp-performance-4/perf_pdd_with_180_first.json` — `8993dc814c527c4027e4614a806c908025644fbfa916be41fc1205c3d1c11d8a`
- `_perf-results/gcp-performance-4/perf_registrar_binding_scale.json` — `0289f19129561045a0f9a3a8fc206bc61362def947adbd0883534288bcaee41a`
- `_perf-results/gcp-performance-4/perf_registration_throughput.json` — `0673765245328f5eed726e90d0ec1ba5cae02f82cc8337eb8c658915291e8a2a`
- `_perf-results/gcp-performance-4/perf_sustained_long_duration_calls.json` — `0ae27ecb368c0eceb2f92b0ad2e75bd46459b91c7fa49e6fd8c4f468577f3e3f`
- `_perf-results/gcp-performance-5/perf_backpressure_step.json` — `47610ebebe0fc6d331c7c50d0fa58928b461e8da2cf2b4bbdebd5a74f8ccaa63`
- `_perf-results/gcp-performance-5/perf_contact_center_transfers.json` — `5a2aad5ea517a66dcb932b43e4367686cc8d1733e090544e161542bc9a682234`
- `_perf-results/gcp-performance-5/perf_srtp_overhead.json` — `d39c9d56b48cc0fcb6eaa5f319f12961286bd7559ca89e00294fb4894fae6353`
- `_perf-results/gcp-performance-5/perf_tls_overhead.json` — `87dcb5db952e9aa2e54465eb674a17299cf522c136f47257488dec6763e68eef`
- `_perf-results/gcp-performance-6/perf_b2bua_forwarding.json` — `7f17da4683890f0e94d9a9feb9455d744956ad3a5119b1d36adc38a52e517424`
- `_perf-results/gcp-performance-6/perf_media_churn.json` — `0b430862e1e1a2cde1f81f540c355624e31b4b230cbdc78f083af0eca50241ac`
- `_perf-results/gcp-performance-6/perf_rtp_steady_state.json` — `7320698f6531bf0affa1498945b647f08fa3fd7f7552f6a686635dc3e5cb9e81`
- `_perf-results/gcp-performance-6/perf_sipp_parity.json` — `3d240f504a276838a43cd3c5ae1743556fe08e7b487c7afb0da3a696b9335736`
- `_perf-results/gcp-performance-soak-long-1/perf_soak_caller.json` — `cef90fb0da41263f953bfb8f993b5bf0dee25ccfb7588ca1caee39f8a67b0236`
- `_perf-results/gcp-performance-soak-long-1/perf_soak_receiver.json` — `b8840dc01f906a5c0cee604626b16491e2004c8790d714c0517524e5c9b0e28d`
- `_perf-results/gcp-performance-soak-long-2/perf_soak_30min.json` — `e37be962fe89b36c07b82a7fca4e26eec9cfe54c1fd84c2865f111fa26ba63a1`
