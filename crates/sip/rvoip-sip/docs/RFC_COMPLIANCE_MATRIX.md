# rvoip-sip Beta RFC Compliance Matrix

Date: 2026-05-26

This matrix records beta claim levels and links each claimed row to test,
interop, generated-validation, report, or explicit non-claim evidence. The
latest complete reference report is
`crates/sip/rvoip-sip/beta-report/20260526T221457Z`, generated from clean git
revision `865430d4`.

| RFC | Area | Beta status | Evidence | Limits |
|-----|------|-------------|----------|--------|
| RFC 3261 | SIP core | Partial | `sip-core RFC 4475 torture tests`, `sip-core generated message validation`, `sip dialog generated validation`, `rvoip-sip integration tests` in `summary.md`; SIPp and PBX matrices | Full section-by-section production audit is not claimed. |
| RFC 3262 | PRACK/100rel | Partial | `crates/sip/rvoip-sip/tests/prack_integration.rs`, `crates/sip/rvoip-sip-dialog/tests/prack_test.rs`, generated validation | Broader PBX reliable-provisional interop remains post-beta. |
| RFC 3263 | Server location | Supported | `crates/sip/rvoip-sip-dialog/tests/rfc3263_resolution.rs`, `crates/sip/rvoip-sip-dialog/tests/rfc3263_failover.rs`, `crates/sip/rvoip-sip-transport/tests/resolver_hickory_e2e.rs` | Published DNS interop profile is still desirable but not a broader DNS service claim. |
| RFC 3264 | SDP offer/answer | Supported | `crates/sip/rvoip-sip/tests/sdp_matcher_integration.rs`, hold/resume rows in PBX `matrix.tsv`, glare tests | Advanced media renegotiation and WebRTC attributes are not beta behavior claims. |
| RFC 3325 | Asserted identity | Partial | `crates/sip/rvoip-sip/tests/pai_integration.rs`, `crates/sip/rvoip-sip/tests/third_party_register_integration.rs`, B2BUA carry-through tests | Trusted-network and carrier certification are not claimed. |
| RFC 3515 | REFER | Supported | `crates/sip/rvoip-sip/tests/refer_auth_retry.rs`, `crates/sip/rvoip-sip/tests/transfer_notify_wiring_tests.rs`, blind-transfer PBX rows | Attended transfer orchestration remains primitives only. |
| RFC 3581 | rport | Supported | `crates/sip/rvoip-sip-dialog/tests/rport_restamp_response.rs`, PBX UDP/TLS rows | NAT traversal matrix is not complete; ICE/TURN are non-claims. |
| RFC 4028 | Session timers | Partial | `crates/sip/rvoip-sip/tests/session_timer_integration.rs`, `crates/sip/rvoip-sip/tests/session_timer_failure_integration.rs` | Full edge-case production audit remains open. |
| RFC 4475 | SIP torture tests | Supported with exclusions | `crates/sip/rvoip-sip/beta-report/20260526T221457Z/sip-core_rfc_4475_torture_tests.log` | Exclusions must remain documented in the torture-test fixture. |
| RFC 5626 | SIP outbound | Partial | TLS registered-flow APIs in `Config`, PBX registration/TLS rows, contact-mode validation tests | Multi-flow outbound behavior is not a beta claim. |
| RFC 6086 | INFO | Supported | `crates/sip/rvoip-sip/tests/info_auth_retry.rs`, DTMF/INFO tests, SIPp/PBX DTMF evidence | INFO package registry completeness is not claimed. |
| RFC 7118 | SIP over WebSocket | Partial | `crates/sip/rvoip-sip-transport/tests/ws_client_round_trip.rs` | Browser/WebRTC and WSS outbound are post-beta/non-claims. |
| RFC 8866 | SDP | Supported | `crates/sip/rvoip-sip-core` SDP parser/builder tests, `sip-core generated message validation`, SDP fuzz target | WebRTC-specific attributes are parser/carry-through only unless wired higher. |
| RFC 3550 | RTP/RTCP | Supported | `crates/sip/rvoip-sip/tests/audio_roundtrip_integration.rs`, `crates/sip/rvoip-sip/tests/bridge_roundtrip_integration.rs`, `perf_rtp_steady_state.json` | Full RTCP feedback behavior is not a beta claim. |
| RFC 3711 / RFC 4568 | SRTP / SDES | Partial | `crates/sip/rvoip-sip/tests/srtp_call_integration.rs`, SRTP negotiator tests, PBX SRTP evidence where present | DTLS-SRTP is excluded. |
| RFC 5764 | DTLS-SRTP | Post-beta | Explicit non-claim in `SECURITY_POSTURE.md` and `COMPATIBILITY_MATRIX.md` | Do not claim. |
| RFC 8445 | ICE | Post-beta | Explicit non-claim in release docs | Do not claim. |
| RFC 8489 | STUN | Partial | `Config::stun_server` docs and startup address-discovery implementation | Not ICE connectivity checks. |
| RFC 8656 | TURN | Post-beta | Explicit non-claim in release docs | Do not claim. |

Release notes must not claim an RFC row beyond the beta status and limits
shown here.
