# RVoIP Performance Qualification Report

> Exact-candidate observations archived by protected run [34074372543](https://github.com/eisenzopf/rvoip/actions/runs/34074372543). A blank cell means the scenario does not emit that metric; it does not mean zero. Measurements belonging only to an accepted prior-run receipt are not copied into this table.

Release: `0.3.10` · commit: `77a99cd38a07641294cf7dc547146b115b135dc7` · qualification: **PASS**.

## Archived measurements

| Scenario | Kind | Target CPS | Achieved CPS | ASR | Calls | Setup p99 ms | Peak RSS MB | Duration s | Evidence SHA-256 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| `perf_mass_teardown_stress` | `point` | 30 | — | — | — | — | 225.59 | 0 | `043df657335f74b730466e94a49470b96df722f620df369b3b8ecf948243641d` |
| `perf_mid_call_signal_under_media` | `point` | 30 | — | 1 | —/30 | 16.794 | 70.94 | 0 | `a5dca1b8db5c2a4ae73c42b86b9809172ebe866f9097bfa665cfc99bb5dc0002` |
| `perf_mixed_workload` | `point` | 50 | 50 | 1 | — | 2.195 | 112.66 | 0 | `9991f7791eca124266a959f3671dface10b06c3b5b4f70525413d8762a5ae107` |
| `perf_call_setup_cps_endpoint` | `point` | 30 | 27.86 | 1 | 975/975 | 2.198 | 116.32 | 0 | `1234c1419f4316784422dea158ca45bf565178a82fb6ca18deb8f45c04ac4419` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.652 | 314.66 | 0 | `d4a2bb70947ec36929f646baea73b30f56cb99a7cf8cffdcde49419002b4717a` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 1000 | 928.57 | 1 | 32500/32500 | 1.785 | 1071.29 | 0 | `a7c1f1c0e20033d2f35eff26ce185d91c52bf8d842bedd2169cf7b33b7f775d1` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 3.24 | 2006.62 | 0 | `070658f674eb5835ab1720b92518985b082d068a776a53743cf57434d2474391` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 30 | 27.86 | 1 | 975/975 | 1.905 | 218.22 | 0 | `6dd1d4cfafc2873b5c4c4c8a442b1c442b9c20f00dd7e0c7fdd5bcdf94c60a9b` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `point` | 300 | 278.56 | 1 | 9750/9750 | 1.452 | 507.48 | 0 | `1a8571ad1a6342f9b421bcd919c4345fcfa48f68cea1d28aa98ee8298685bbd3` |
| `perf_call_setup_cps_signaling-only-server-high-performance` | `sweep` | 2000 | 1857.1 | 1 | — | — | — | — | `b888d97029fdd7ce44d0fff345bd08e878ab977d4c64135840e6dd464e6b0863` |
| `perf_concurrent_active_calls` | `point` | 500 | — | 1 | —/500 | 190.579 | 211.71 | 0 | `39d0c4c7f6a4ae1c57dcd5b974cdb09477d0312a65f713087dee5620377dd5e6` |
| `perf_ai_agent_load` | `point` | 30 | — | 1 | — | 16.925 | 63.21 | 0 | `cbae00e10e2ff04cf5170d9dad35bea3c6332f7bddcfa7214590c641ebde7ef3` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.91 | 327.85 | 0 | `72027eb52929361e35082be4e237db93975edc5c42a587a103062cc9658d9f1d` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.54 | 1 | 32500/32500 | 2.087 | 1081.57 | 0 | `05abc41fa268cc1645e81851e53bdab07fd7b9e0eed23f5eccb4a4df2a0c8453` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.1 | 1 | 65000/65000 | 4.174 | 2005.08 | 0 | `672a619859e41863f504eba1126a0fb80ee9cfb5fcf2ea663c597ebe200f97d7` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 2.21 | 227.45 | 0 | `8baf61d41a01fd16810e990a42aa7d344bd7645e401c90a7ae3eb16f93d0f356` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.56 | 1 | 9750/9750 | 1.737 | 523.66 | 0 | `21af093de8ce796e4430c8d8ef43b12acf6d53f269a2dd06f85b888c4ab8843e` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.1 | 1 | — | — | — | — | `d5a1ab960e8bcbd7ea0503972ccf67e58fdf0abf0d672cf79da1a7f13fd66958` |
| `perf_session_churn_leak` | `point` | 0 | — | — | 250/250 | 1.838 | 82.12 | 0 | `e52c0b99e50fc27aba38581c4da3920deeb28be8f5f7ef1e2d3e62a7063583bc` |
| `perf_transport_recovery` | `point` | 5 | — | — | — | — | 83.04 | 0 | `9fdde02ca403a4713488e30ccd7edcdcd7befe7bb77cbcc6c21ed634659acb53` |
| `perf_pdd_with_180_first` | `point` | 50 | 46.43 | 1 | 1625/1625 | 2.212 | 151.27 | 0 | `bd9e819baae57e36fa56be91015f714c492ccefa2102d10c206c9017809dc097` |
| `perf_registrar_binding_scale` | `point` | 100 | — | — | — | — | 80.66 | 0 | `ce1554bbd9475edfa97a6c4fc79b8e4677fa6e76d292f1ac629d3da62cc4d2be` |
| `perf_registration_throughput` | `point` | 100 | — | — | — | — | 81.54 | 0 | `8be69d0e462112a6775e2a57fa76341d0c15c22ecd2e5f119427a12726234ec6` |
| `perf_sustained_long_duration_calls` | `point` | 30 | 28.85 | 1 | 1875/1875 | 2.318 | 266.95 | 0 | `02c85cb080ab778618d1eee479bae2858c57f8db239e51d409674468b49b7b31` |
| `perf_backpressure_step` | `point` | 200 | — | — | 13851/13851 | — | 376.99 | 0 | `bd1578cab0511de968bac9b227fa5b8a027357a5d68ea3ee7d7387364437de3c` |
| `perf_contact_center_transfers` | `point` | 20 | — | 1 | —/20 | 9.814 | 61.02 | 0 | `7f1b3bf9f32dbaff83eb1484e749d62ff7f093350714cf208ac362ce54cb2942` |
| `perf_srtp_overhead` | `point` | 50 | — | 1 | —/50 | 26.034 | 71.18 | 0 | `a571cf59574a0a0940e406170c7e8f03868d515885213da8b2c5536a5d26857d` |
| `perf_tls_overhead` | `point` | 100 | 92.85 | 1 | 3250/3250 | 20.578 | 178.73 | 0 | `d5ac49da78853d016f113aea612fc5992a30ee44a3080bcec1b0e46a6b153d98` |
| `perf_b2bua_forwarding` | `point` | 30 | 27.86 | 1 | 975/975 | 2.439 | 319.04 | 0 | `d9abfc06a5cf6ae7551c1e258854c3548ae67db704932e2365e222c83494d256` |
| `perf_media_churn` | `point` | 0 | — | — | — | — | — | 0 | `03b517b0e473345249124c9283880c3fab737076175d2784bcfb1eb576b839a2` |
| `perf_rtp_steady_state` | `point` | 50 | — | 1 | —/50 | 26.149 | 71.29 | 0 | `1b358d5586b496a3bb59982bcbc41571b3c9d3352473555d88767f2d1d743b60` |
| `perf_sipp_parity` | `point` | 20 | — | — | — | 0 | 46.17 | 0 | `5ff260681fd212c8cf5d16156110fd5488061b51024b2c6067bc3d0a47b93703` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.836 | 329.16 | 0 | `200c1f655e98df2e08aa2ccf7da8b97e64ec4d6f88370f780b438eeee85d2f00` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.56 | 1 | 32500/32500 | 1.947 | 1079.04 | 0 | `235e8ac07106eecb9b610442d3dead1788e3e1826dba32b2137747863b7bf2cd` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 4.01 | 2011.39 | 0 | `31f68b1996bd11b12e372ce143d39f07a57896df99291958c40bf1a19f21df1a` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 2.107 | 226.62 | 0 | `dfddee00f5f80fd1f8bebe9f2923319391795c1dc6d3c89c9a8e0aaafe326c1a` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.56 | 1 | 9750/9750 | 1.65 | 523.67 | 0 | `eedca2b2dd43273101f0727c8533ab1c8c2dde14299c63706add50232f8b591d` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.13 | 1 | — | — | — | — | `c72291df377c223b572df58129f6722d05ec4137613fefde1de8d3ef0b39f90b` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 4.01 | 2011.39 | 0 | `31f68b1996bd11b12e372ce143d39f07a57896df99291958c40bf1a19f21df1a` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 4.01 | 2011.39 | 0 | `31f68b1996bd11b12e372ce143d39f07a57896df99291958c40bf1a19f21df1a` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.86 | 1 | 3250/3250 | 1.731 | 327.9 | 0 | `add9342978312750a58668e90c5986235021af66ea1cb531a096970817d5f51b` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.56 | 1 | 32500/32500 | 1.945 | 1078.34 | 0 | `89afdc5f1abb8451462e1a78a6ad354609e1fa14e927c698ce98bfae07d95f27` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.12 | 1 | 65000/65000 | 3.463 | 2015.45 | 0 | `6150dbf016ae49422a148f6ceee4778dfe18b9e257813ad543977e3dd14945f7` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.921 | 226.43 | 0 | `46971b1c9d7c041487fceb97635eff677007feb17bc46acfccb930bddf2b4175` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.53 | 521.95 | 0 | `b5a1d3b9b038b27549d57589e6fb47a0eb24d33ab9daf7d98a7907ae79a6d5a5` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.12 | 1 | — | — | — | — | `b58e8e9ec88085a205f88a0b28367dd6b0e71a5e48d4d2d26bd8fcdf07958f05` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.12 | 1 | 65000/65000 | 3.463 | 2015.45 | 0 | `6150dbf016ae49422a148f6ceee4778dfe18b9e257813ad543977e3dd14945f7` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.12 | 1 | 65000/65000 | 3.463 | 2015.45 | 0 | `6150dbf016ae49422a148f6ceee4778dfe18b9e257813ad543977e3dd14945f7` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 100 | 92.85 | 1 | 3250/3250 | 1.858 | 328.21 | 0 | `8940b1e908730752acab062d55bf2b5eb316c37da2f71ed2a5dc36bc7c39c69b` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 1000 | 928.57 | 1 | 32500/32500 | 1.973 | 1084.91 | 0 | `8b61d56e87795e38475d230af318814afc9de921a865301df59e2343ea38eb4b` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 3.86 | 2025.25 | 0 | `680ceedc6cc87b50d8076526071ce380db930af0c8296a383eb4459903732ab3` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 30 | 27.86 | 1 | 975/975 | 1.966 | 226.53 | 0 | `718f66980efa4385d506aed3d875afeef9cc2efbde09672c87ba2daed9ee294e` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 300 | 278.57 | 1 | 9750/9750 | 1.71 | 520.41 | 0 | `6e27d1e3a8dd4a43039b429128e6db9c00b253cc0d906c5b4680923e8d14d099` |
| `perf_call_setup_cps_pbx-media-server` | `sweep` | 2000 | 1857.13 | 1 | — | — | — | — | `d5371222bf3bd7be29ed9d5a62289d5696d6a2dbfad1a359547a1fc8925d744a` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 3.86 | 2025.25 | 0 | `680ceedc6cc87b50d8076526071ce380db930af0c8296a383eb4459903732ab3` |
| `perf_call_setup_cps_pbx-media-server` | `point` | 2000 | 1857.13 | 1 | 65000/65000 | 3.86 | 2025.25 | 0 | `680ceedc6cc87b50d8076526071ce380db930af0c8296a383eb4459903732ab3` |
| `perf_soak_30min` | `point` | 0 | — | 1 | 587/587 | 15.737 | 101.63 | 0 | `e7e8781c5fef58e31f1bb609dd34d4fcb2be37586afd915d2ec03e4de213adce` |
| `perf_soak_caller` | `point` | 0 | — | 1 | 9904/9904 | 175.636 | 139.65 | 0 | `5cf5e043f6c5994eb4b28cd6c67056237c4770d4aba32ef0cea993579be919b2` |
| `perf_soak_receiver` | `point` | 0 | — | — | — | — | 154.05 | 0 | `8e16951ba332c2478d895cc74596781d465bf203a59456094052efa48817a2a6` |

## Interpretation

- The rows are observations, not individually invented PASS verdicts. Their governing performance, soak, regression, cleanup, and evidence-integrity gates are PASS in the complete gate report.
- The supported general full-media beta claim remains up to 2,000 CPS with media enabled. Results above 2,000 CPS remain tuned or experimental and require their own topology, hardware, and qualification evidence.
- Call-setup sweeps use loopback networking on the recorded GCP qualification host. They establish repeatable release regression evidence, not public-network latency or carrier capacity.
- Full JSON, resource windows, diagnostics, and scenario-specific counters remain in the GitHub evidence artifact; this report intentionally avoids flattening non-equivalent metrics into one score.

## Evidence paths

- `_perf-results/gcp-performance-1/perf_mass_teardown_stress.json` — `043df657335f74b730466e94a49470b96df722f620df369b3b8ecf948243641d`
- `_perf-results/gcp-performance-1/perf_mid_call_signal_under_media.json` — `a5dca1b8db5c2a4ae73c42b86b9809172ebe866f9097bfa665cfc99bb5dc0002`
- `_perf-results/gcp-performance-1/perf_mixed_workload.json` — `9991f7791eca124266a959f3671dface10b06c3b5b4f70525413d8762a5ae107`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_endpoint.json` — `1234c1419f4316784422dea158ca45bf565178a82fb6ca18deb8f45c04ac4419`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/100.json` — `d4a2bb70947ec36929f646baea73b30f56cb99a7cf8cffdcde49419002b4717a`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/1000.json` — `a7c1f1c0e20033d2f35eff26ce185d91c52bf8d842bedd2169cf7b33b7f775d1`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/2000.json` — `070658f674eb5835ab1720b92518985b082d068a776a53743cf57434d2474391`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/30.json` — `6dd1d4cfafc2873b5c4c4c8a442b1c442b9c20f00dd7e0c7fdd5bcdf94c60a9b`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/300.json` — `1a8571ad1a6342f9b421bcd919c4345fcfa48f68cea1d28aa98ee8298685bbd3`
- `_perf-results/gcp-performance-2/perf_call_setup_cps_signaling-only-server-high-performance/_sweep.json` — `b888d97029fdd7ce44d0fff345bd08e878ab977d4c64135840e6dd464e6b0863`
- `_perf-results/gcp-performance-2/perf_concurrent_active_calls.json` — `39d0c4c7f6a4ae1c57dcd5b974cdb09477d0312a65f713087dee5620377dd5e6`
- `_perf-results/gcp-performance-3/perf_ai_agent_load.json` — `cbae00e10e2ff04cf5170d9dad35bea3c6332f7bddcfa7214590c641ebde7ef3`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/100.json` — `72027eb52929361e35082be4e237db93975edc5c42a587a103062cc9658d9f1d`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/1000.json` — `05abc41fa268cc1645e81851e53bdab07fd7b9e0eed23f5eccb4a4df2a0c8453`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/2000.json` — `672a619859e41863f504eba1126a0fb80ee9cfb5fcf2ea663c597ebe200f97d7`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/30.json` — `8baf61d41a01fd16810e990a42aa7d344bd7645e401c90a7ae3eb16f93d0f356`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/300.json` — `21af093de8ce796e4430c8d8ef43b12acf6d53f269a2dd06f85b888c4ab8843e`
- `_perf-results/gcp-performance-3/perf_call_setup_cps_pbx-media-server/_sweep.json` — `d5a1ab960e8bcbd7ea0503972ccf67e58fdf0abf0d672cf79da1a7f13fd66958`
- `_perf-results/gcp-performance-3/perf_session_churn_leak.json` — `e52c0b99e50fc27aba38581c4da3920deeb28be8f5f7ef1e2d3e62a7063583bc`
- `_perf-results/gcp-performance-3/perf_transport_recovery.json` — `9fdde02ca403a4713488e30ccd7edcdcd7befe7bb77cbcc6c21ed634659acb53`
- `_perf-results/gcp-performance-4/perf_pdd_with_180_first.json` — `bd9e819baae57e36fa56be91015f714c492ccefa2102d10c206c9017809dc097`
- `_perf-results/gcp-performance-4/perf_registrar_binding_scale.json` — `ce1554bbd9475edfa97a6c4fc79b8e4677fa6e76d292f1ac629d3da62cc4d2be`
- `_perf-results/gcp-performance-4/perf_registration_throughput.json` — `8be69d0e462112a6775e2a57fa76341d0c15c22ecd2e5f119427a12726234ec6`
- `_perf-results/gcp-performance-4/perf_sustained_long_duration_calls.json` — `02c85cb080ab778618d1eee479bae2858c57f8db239e51d409674468b49b7b31`
- `_perf-results/gcp-performance-5/perf_backpressure_step.json` — `bd1578cab0511de968bac9b227fa5b8a027357a5d68ea3ee7d7387364437de3c`
- `_perf-results/gcp-performance-5/perf_contact_center_transfers.json` — `7f1b3bf9f32dbaff83eb1484e749d62ff7f093350714cf208ac362ce54cb2942`
- `_perf-results/gcp-performance-5/perf_srtp_overhead.json` — `a571cf59574a0a0940e406170c7e8f03868d515885213da8b2c5536a5d26857d`
- `_perf-results/gcp-performance-5/perf_tls_overhead.json` — `d5ac49da78853d016f113aea612fc5992a30ee44a3080bcec1b0e46a6b153d98`
- `_perf-results/gcp-performance-6/perf_b2bua_forwarding.json` — `d9abfc06a5cf6ae7551c1e258854c3548ae67db704932e2365e222c83494d256`
- `_perf-results/gcp-performance-6/perf_media_churn.json` — `03b517b0e473345249124c9283880c3fab737076175d2784bcfb1eb576b839a2`
- `_perf-results/gcp-performance-6/perf_rtp_steady_state.json` — `1b358d5586b496a3bb59982bcbc41571b3c9d3352473555d88767f2d1d743b60`
- `_perf-results/gcp-performance-6/perf_sipp_parity.json` — `5ff260681fd212c8cf5d16156110fd5488061b51024b2c6067bc3d0a47b93703`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `200c1f655e98df2e08aa2ccf7da8b97e64ec4d6f88370f780b438eeee85d2f00`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `235e8ac07106eecb9b610442d3dead1788e3e1826dba32b2137747863b7bf2cd`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `31f68b1996bd11b12e372ce143d39f07a57896df99291958c40bf1a19f21df1a`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `dfddee00f5f80fd1f8bebe9f2923319391795c1dc6d3c89c9a8e0aaafe326c1a`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `eedca2b2dd43273101f0727c8533ab1c8c2dde14299c63706add50232f8b591d`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `c72291df377c223b572df58129f6722d05ec4137613fefde1de8d3ef0b39f90b`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `31f68b1996bd11b12e372ce143d39f07a57896df99291958c40bf1a19f21df1a`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T021744Z_clean_Rvz6yI/report.json` — `31f68b1996bd11b12e372ce143d39f07a57896df99291958c40bf1a19f21df1a`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `add9342978312750a58668e90c5986235021af66ea1cb531a096970817d5f51b`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `89afdc5f1abb8451462e1a78a6ad354609e1fa14e927c698ce98bfae07d95f27`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `6150dbf016ae49422a148f6ceee4778dfe18b9e257813ad543977e3dd14945f7`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `46971b1c9d7c041487fceb97635eff677007feb17bc46acfccb930bddf2b4175`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `b5a1d3b9b038b27549d57589e6fb47a0eb24d33ab9daf7d98a7907ae79a6d5a5`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `b58e8e9ec88085a205f88a0b28367dd6b0e71a5e48d4d2d26bd8fcdf07958f05`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `6150dbf016ae49422a148f6ceee4778dfe18b9e257813ad543977e3dd14945f7`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T023255Z_clean_hTjCfU/report.json` — `6150dbf016ae49422a148f6ceee4778dfe18b9e257813ad543977e3dd14945f7`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/output-target/perf-results/perf_call_setup_cps_pbx-media-server/100.json` — `8940b1e908730752acab062d55bf2b5eb316c37da2f71ed2a5dc36bc7c39c69b`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/output-target/perf-results/perf_call_setup_cps_pbx-media-server/1000.json` — `8b61d56e87795e38475d230af318814afc9de921a865301df59e2343ea38eb4b`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/output-target/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `680ceedc6cc87b50d8076526071ce380db930af0c8296a383eb4459903732ab3`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/output-target/perf-results/perf_call_setup_cps_pbx-media-server/30.json` — `718f66980efa4385d506aed3d875afeef9cc2efbde09672c87ba2daed9ee294e`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/output-target/perf-results/perf_call_setup_cps_pbx-media-server/300.json` — `6e27d1e3a8dd4a43039b429128e6db9c00b253cc0d906c5b4680923e8d14d099`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/output-target/perf-results/perf_call_setup_cps_pbx-media-server/_sweep.json` — `d5371222bf3bd7be29ed9d5a62289d5696d6a2dbfad1a359547a1fc8925d744a`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/perf-results/perf_call_setup_cps_pbx-media-server/2000.json` — `680ceedc6cc87b50d8076526071ce380db930af0c8296a383eb4459903732ab3`
- `_perf-results/gcp-performance-soak-long-1/profiles/20260907T024805Z_clean_Gl0kwb/report.json` — `680ceedc6cc87b50d8076526071ce380db930af0c8296a383eb4459903732ab3`
- `_perf-results/gcp-performance-soak-long-2/perf_soak_30min.json` — `e7e8781c5fef58e31f1bb609dd34d4fcb2be37586afd915d2ec03e4de213adce`
- `_perf-results/gcp-performance-soak-long-2/perf_soak_caller.json` — `5cf5e043f6c5994eb4b28cd6c67056237c4770d4aba32ef0cea993579be919b2`
- `_perf-results/gcp-performance-soak-long-2/perf_soak_receiver.json` — `8e16951ba332c2478d895cc74596781d465bf203a59456094052efa48817a2a6`
