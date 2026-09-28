# rvoip for carriers and CPaaS operators: compliance, performance, reliability

Reviewed 2026-09-27 against branch `codex/downstream-feature-review`. The
release evidence cited here is the protected `0.3.10` qualification run
`34074372543` at commit `77a99cd38a07641294cf7dc547146b115b135dc7`
([qualification report](../crates/sip/rvoip-sip/docs/BETA_RELEASE_REPORT.md),
[213-gate ledger](../crates/sip/rvoip-sip/docs/BETA_GATE_REPORT.md),
[performance report](../crates/sip/rvoip-sip/docs/BETA_PERFORMANCE_REPORT.md)).
Code-level statements describe the branch head, which is newer than that
commit; where a behaviour landed after `0.3.10` this document says so.

This is an evidence index, not a certification. Every row points at a gate id
in [`scripts/release/gates.json`](../scripts/release/gates.json), a named test,
a source file, or a document in this repository. Where the repository does not
contain evidence, the row says so. The release report's own claim boundary
applies throughout: PASS covers the exact commit, gate catalog, peer images,
environments, and thresholds recorded by the run, and is "not a general carrier
certification or a performance SLA."

Contents: [1 Standards compliance](#1-standards-compliance) ·
[2 Interoperability attestation](#2-interoperability-attestation) ·
[3 Performance](#3-performance) · [4 Reliability](#4-reliability) ·
[5 Security](#5-security) · [6 Known gaps and roadmap](#6-known-gaps-and-roadmap) ·
[7 How to reproduce](#7-how-to-reproduce)

## 1. Standards compliance

### Status levels

| Status | Meaning in this table |
|---|---|
| **Release-gated** | Exercised by a gate in the `0.3.10` ledger whose id is named in the Evidence column. The gate runs against an independent peer (Asterisk, FreeSWITCH, Jambonz, Kamailio, OpenSIPS, SIPp, baresip, libsrtp, Chromium) or a named performance/fuzz executable. The bounded scope of the underlying claim still applies; see [RFC_COMPLIANCE_MATRIX.md](../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md). |
| **Tested** | A non-ignored unit or integration test exists for the behaviour; one is named. These tests also run under the ledger's `core.<crate>` gates (for example `core.rvoip-sip`, `core.rvoip-sip-dialog`, `core.rvoip-sip-core`, `core.rvoip-sip-transport`, `core.rvoip-rtp-core`), but a crate-wide test gate does not name the RFC, so it is not counted as Release-gated here. |
| **Implemented, untested against peers** | Behaviour exists in source; no test could be named for it in reasonable time. Treat as unverified. |
| **Parsed only** | The header, URI form, SDP attribute, or payload-type entry is parsed and round-tripped; no protocol behaviour is claimed. |
| **Not supported** | An explicit non-claim in the evidence docs, or absent from the code. Obsoleted RFCs are listed with the status of their successor and marked "obsoleted by". |

Two repository documents carry the authoritative claim boundary and should be
read with this table: the crate-local
[RFC_COMPLIANCE_MATRIX.md](../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md)
(20 bounded claims with `T-*` evidence ids, plus explicit non-claims) and the
broader [docs/sip/SIP_RFC_COMPLIANCE.md](sip/SIP_RFC_COMPLIANCE.md). Where the
table below is more conservative than either document, the table reflects what
could be located in code and the `0.3.10` ledger on this branch.

Gate ids referenced below (all PASS in the `0.3.10` ledger):
`interop.asterisk-matrix`, `interop.freeswitch-matrix` (scenarios
`registration basic_call g729_call amr_call amr_transcode_call b2bua_call hold_resume ring_cancel dtmf reject blind_transfer`
over UDP and TLS, APIs `endpoint`/`stream_peer`/`callback`),
`interop.jambonz-matrix` (same runner, UDP only, PCMU/PCMA),
`interop.sipp-matrix`, `interop.strict-ua` (baresip),
`interop.remote-proxies.{kamailio,opensips}.{rvoip-first,peer-first}.{udp,tcp,tls}`,
`interop.proxy-pbx.{kamailio,opensips}.matrix`, `interop.amr-rate-sweep.*`,
`interop.remote-libsrtp`, `interop.browser-dtmf`, `perf.*`, `security.fuzz-*`.

### Core signalling

| RFC | Title | Status | Evidence |
|---|---|---|---|
| 3261 | SIP | Release-gated | UA role: `interop.asterisk-matrix`, `interop.freeswitch-matrix`, `interop.jambonz-matrix`, `interop.sipp-matrix`, `interop.strict-ua`. Proxy role (§16): `interop.remote-proxies.*` (12 rows). Registrar role: `registration` scenario in the PBX matrices. Bounded claim `SIP-3261-CORE` is "Partial"; no section-by-section certification is claimed. |
| 2543 | SIP (1999) | Not supported (obsoleted by 3261) | Referenced only in `crates/sip/sip-core/src/types/via.rs` |
| 3263 | Locating SIP servers (NAPTR/SRV/A) | Release-gated | "Transport failure and RFC 3263 failover" scenario in `interop.remote-proxies.*` (`crates/sip/sip-proxy/tests/interop/scripts/rfc3263_dns.py`); `crates/sip/sip-transport/tests/resolver_hickory_e2e.rs::hickory_client_resolves_naptr_then_srv_then_a`; `crates/sip/sip-dialog/tests/rfc3263_failover.rs::first_candidate_recoverable_failure_falls_over_to_second` |
| 2782 | DNS SRV | Tested | `resolver_hickory_e2e.rs` (above); `crates/sip/sip-transport/src/resolver/srv.rs` |
| 3264 | SDP offer/answer | Release-gated | `hold_resume` and `basic_call` scenarios in `interop.asterisk-matrix`, `interop.freeswitch-matrix`, `interop.jambonz-matrix`; `crates/sip/rvoip-sip/tests/sdp_matcher_integration.rs::intersection_in_offerer_order` |
| 6026 | Correct 2xx handling for INVITE | Tested | `crates/sip/sip-dialog/src/transaction/manager/tests.rs::invite_retransmission_after_2xx_reuses_uas_response_cache` |
| 6141 | Re-INVITE and target refresh | Tested | `crates/sip/rvoip-sip/tests/glare_retry_integration.rs`; `crates/sip/rvoip-sip/tests/adapter_renegotiate.rs` |
| 4320 | Non-INVITE transaction issues | Tested | `crates/sip/sip-proxy/tests/proxy_rfc_edge_cases.rs::all_non_invite_timeouts_and_a_late_response_remain_silent`; `crates/sip/sip-proxy/src/proxy.rs` |
| 5057 | Multiple dialog usages | Not supported | Listed as planned in [SIP_RFC_COMPLIANCE.md](sip/SIP_RFC_COMPLIANCE.md) |
| 5658 | Record-Route issues (double Record-Route) | Not supported | No source reference. Loose/strict Route handling per RFC 3261 §16 is tested: `crates/sip/sip-proxy/tests/proxy_routing_live.rs::loose_route_is_preprocessed_then_resolved_with_ordered_failover` |
| 3262 | Reliable provisional responses (PRACK) | Tested | `crates/sip/rvoip-sip/tests/prack_integration.rs::prack_positive_reliable_183_flow`, `::prack_policy_mismatch_returns_420`. No PBX scenario exercises 100rel. |
| 3311 | UPDATE | Tested | `crates/sip/rvoip-sip/tests/update_notify_auth_retry.rs::update_401_retry_uses_authorization`; `sip_api_design_2_section_10_skeletons.rs::in_dialog_update_smoke` |
| 3428 | MESSAGE | Tested | `crates/sip/rvoip-sip/tests/oob_auth_retry.rs::message_with_credentials_retries_with_full_digest`; `adapter_data_message_network.rs::data_message_wrong_realm_challenge_fails_closed_without_retry_or_secret_leak`; `message_body_*.xml` in the proxy interop scenarios |
| 3515 | REFER | Release-gated | `blind_transfer` scenario in `interop.asterisk-matrix`, `interop.freeswitch-matrix`, `interop.jambonz-matrix`; `crates/sip/rvoip-sip/tests/blind_transfer_integration.rs::blind_transfer_end_to_end`. Blind transfer only; `Refer-To` required. |
| 3892 | Referred-By | Release-gated | `interop.jambonz-matrix` (`J-3892-I1`: typed `Referred-By` copied unchanged into the referenced INVITE); `header_inspection_integration.rs::inbound_invite_wire_carries_application_routing_headers`. Optional-header propagation only; no identity signing. |
| 4488 | Refer-Sub | Parsed only | REFER header handling in `crates/sip/sip-core` |
| 5589 | Call control: transfer (BCP) | Release-gated | Blind-transfer flows only, via the `blind_transfer` gates above; `crates/sip/rvoip-sip/src/server/transfer.rs`. Attended/consultative flows are not claimed (see 3891). |
| 6086 | INFO | Tested | `crates/sip/rvoip-sip/tests/info_auth_retry.rs::info_extras_survive_401_driven_auth_retry`. No Info-Package negotiation. |
| 2976 | INFO (2000) | Tested (obsoleted by 6086) | As 6086 |
| 3903 | PUBLISH | Parsed only | `crates/sip/sip-core/src/types/sip_etag.rs`, `sip_if_match.rs`; COMPATIBILITY_MATRIX lists PUBLISH as parser-only/post-beta |
| 4028 | Session timers | Tested | `crates/sip/rvoip-sip/tests/session_timer_integration.rs::session_timer_refresh_emits_event`; `session_422_retry.rs::invite_422_retry_bumps_session_expires_and_succeeds` |
| 3326 | Reason header | Implemented, untested against peers | Emitted on teardown in `crates/sip/rvoip-sip/src/state_machine/actions.rs`; no test named for the header itself |
| 3891 | Replaces | Parsed only | `crates/sip/sip-dialog/tests/refer_handling_test.rs::test_refer_with_replaces_header` constructs the header; no replacement executes (`SIP-3891-REPLACES` is Unsupported) |
| 4538 | Target-Dialog | Parsed only | `crates/sip/rvoip-sip/src/api/headers/convenience.rs` |
| 4916 | Connected identity | Not supported | Absent |
| 4244, 7044 | History-Info | Parsed only | Carried through the B2BUA unchanged: `crates/sip/rvoip-sip/tests/b2bua_carry_through_integration.rs::b2bua_carry_through_runs_strip_and_rewrite_end_to_end`; no retargeting semantics |
| 5806 | Diversion | Parsed only | Same carry-through test; `crates/sip/rvoip-sip/src/api/headers/convenience.rs` |
| 3840 | UA capabilities (feature tags) | Parsed only | Contact parameter handling in `crates/sip/sip-core/src/types/outbound.rs` |
| 3841 | Caller preferences | Not supported | No `Accept-Contact` type in `crates/sip/sip-core/src/types`; mentioned only in `builder/builder.md` |
| 3312 | Preconditions | Parsed only | Option tag only (`crates/sip/sip-core/src/builder/headers/proxy_require.rs`) |
| 4412 | Resource-Priority | Parsed only | `crates/sip/sip-core/src/builder/headers/priority.rs` |
| 4475 | SIP torture messages | Tested | `crates/sip/sip-core/tests/rfc_compliance/torture_test.rs::test_wellformed_messages`, `::test_malformed_messages`. Three well-formed fixtures excluded (see matrix row `SIP-4475-TORTURE`). |
| 5118 | Torture messages for IPv6 | Tested | IPv6 fixtures in `crates/sip/sip-core/tests/rfc_compliance/wellformed/` (`4.1_ipv6-good.sip`, `4.6_ipv6-in-sdp.sip`, ...). Parser only; COMPATIBILITY_MATRIX: network-stack IPv6 "Not audited". |
| 3986, 2396 | URI generic syntax | Tested | `crates/sip/sip-core/tests/parser/uri_parser_test.rs`; robustness: `security.fuzz-uri` |
| 3966 | tel URI | Tested | `crates/sip/sip-core/tests/parser/uri_parser_test.rs::test_parse_tel_uri` |
| 2045, 2046, 2183, 5621 | MIME bodies, multipart, Content-Disposition | Tested | Multipart fixture `3.1.1.11_mpart01.sip` under `torture_test.rs::test_wellformed_messages`; `crates/sip/sip-core/src/parser/multipart.rs` |
| 3204 | ISUP/QSIG MIME types | Parsed only | `crates/sip/rvoip-sip/src/api/bodies.rs` |
| 5646, 3066, 1766 | Language tags | Parsed only | `crates/sip/sip-core/src/parser/headers/content_language.rs` |
| 2822 | Date header format | Parsed only | `crates/sip/sip-core/src/types/date.rs` |
| 3629, 5198, 1034, 1035, 5890, 4291, 3879, 6874 | UTF-8, hostname, IPv6 literal grammar | Parsed only | `crates/sip/sip-core/src/parser/uri/` |
| 2616, 7230 | HTTP grammar reused by SIP headers | Parsed only | `crates/sip/sip-core/src/types/accept*.rs`, `sdp/attributes/common.rs` |

### Registration, routing, NAT, transport

| RFC | Title | Status | Evidence |
|---|---|---|---|
| 3327 | Path | Implemented, untested against peers | `crates/sip/rvoip-sip/src/api/respond/register_response.rs` (echo on 2xx), `crates/sip/sip-dialog/src/protocol/register_handler.rs`, `crates/sip/rvoip-sip/src/server/contact_resolver.rs` |
| 3608 | Service-Route | Implemented, untested against peers | `register_response.rs`, `register_handler.rs`; `crates/sip/sip-core/src/types/service_route.rs` |
| 3455 | 3GPP P-headers | Parsed only | `P-Associated-URI` stamped on REGISTER 2xx (`register_response.rs`); other P-headers typed in `convenience.rs` |
| 7315 | P-Charging-Vector | Parsed only | `crates/sip/rvoip-sip/src/api/headers/convenience.rs` |
| 3325 | P-Asserted-Identity / P-Preferred-Identity | Tested | `crates/sip/rvoip-sip/tests/pai_integration.rs::config_pai_uri_surfaces_on_inbound_call`, `::per_call_pai_overrides_config`; `third_party_register_integration.rs`. No trusted-domain policy enforcement. |
| 3323 | Privacy | Parsed only | `convenience.rs`; carry-through only |
| 5626 | SIP Outbound | Tested | `crates/sip/sip-registrar/src/api/mod.rs::registered_flow_is_opaque_owned_live_and_fail_closed`, `::staged_replacement_preserves_old_flow_until_contact_commit`; `crates/sip/sip-dialog/tests/rfc3263_failover.rs::failed_primary_registered_flow_uses_secondary_exact_flow`; `crates/rvoip/src/app.rs::remote_endpoint_profile_fails_closed_until_tls_srtp_identity_and_media_are_complete`. Two-UA real-NAT evidence pending ([REMOTE_ENDPOINT_PROFILE.md](sip/REMOTE_ENDPOINT_PROFILE.md)). |
| 6223 | Keep-alive indication (CRLF) | Tested | `crates/sip/sip-transport/src/transport/tcp/connection.rs::keepalive_ping_at_offset_0_is_recognised_as_frame`; `tcp/mod.rs::send_raw_triggers_server_pong` |
| 5627 | GRUU | Parsed only | `pub-gruu`/`temp-gruu` Contact parameters in `crates/sip/sip-core/src/types/outbound.rs` |
| 5628, 3680 | reg event package | Not supported | Absent |
| 6140 | Trunk registration (bulk numbers) | Not supported | Listed as planned in SIP_RFC_COMPLIANCE.md |
| 3581 | Symmetric response routing (rport) | Tested | `crates/sip/sip-dialog/tests/rport_restamp_response.rs::response_via_gets_received_and_rport_when_inbound_via_had_rport_flag`. Ignored NAT stub `tests/resilience/rfc3581_rport_nat_recovery.rs` is not evidence. |
| 5923 | Connection reuse (`alias`) | Not supported | No source reference to the `alias` parameter |
| 5630 | SIPS URI | Tested | `crates/sip/rvoip-sip/tests/tls_call_integration.rs::sips_call_establishes_through_tls_transport`; `crates/sip/sip-proxy/tests/proxy_routing_live.rs::per_transport_advertised_via_and_sips_no_downgrade_are_enforced`; `sips_routing_*.xml` in the proxy interop scenarios |
| 3261 §18 | UDP / TCP / TLS transport | Release-gated | UDP: every PBX and SIPp gate. TCP: `interop.remote-proxies.*.tcp`. TLS: `interop.remote-proxies.*.tls` (hostname-verifying, gate-owned CA), `interop.asterisk-matrix` / `interop.freeswitch-matrix` TLS cells, `interop.amr-rate-sweep.amr{nb,wb}-tls` |
| 8446, 5246 | TLS 1.3 / 1.2 | Release-gated | rustls 0.23 (`crates/sip/sip-transport/Cargo.toml`); exercised by the TLS gates above; `crates/sip/sip-transport/tests/tls_handshake_test.rs::tls_client_default_validation_rejects_self_signed_cert`. Negotiated versions are not asserted separately. |
| 7118 | SIP over WebSocket | Tested | `crates/sip/sip-transport/tests/ws_client_round_trip.rs::plain_ws_round_trip_delivers_register_to_server_event_bus`. WSS outbound: Not supported (COMPATIBILITY_MATRIX). |
| 6455 | WebSocket | Tested | As 7118; `ws_handshake_admission.rs::ws_handshake_deadline_releases_saturated_admission` |
| 8489, 5389 | STUN | Not supported (as a compliance claim) | Explicit non-claim in RFC_COMPLIANCE_MATRIX. A bounded address-discovery client exists: `crates/media/rtp-core/tests/stun_loopback.rs::stun_client_round_trip_against_loopback_server`; STUN on the live RTP socket (`::transport_stun_client_uses_the_live_rtp_socket`) landed after `0.3.10`. |
| 5769 | STUN test vectors | Tested | `crates/media/ice-core/src/stun.rs::rfc5769_request_parses_and_authenticates` |
| 8445, 5245 | ICE | Not supported | Explicit non-claim ([SECURITY_POSTURE.md](../crates/sip/rvoip-sip/docs/SECURITY_POSTURE.md)). `crates/media/ice-core` exists with scripted tests (`tests/scripted.rs`) but is not part of the SIP claim. |
| 6544, 7675 | ICE-TCP, ICE consent | Not supported | Referenced in `crates/media/ice-core/src/`; covered by the ICE non-claim |
| 8838, 8840 | Trickle ICE | Not supported | Types in `rvoip-webrtc` only |
| 8656, 5766 | TURN | Not supported | Explicit non-claim |

### SDP and media

| RFC | Title | Status | Evidence |
|---|---|---|---|
| 8866, 4566 | SDP | Release-gated | Every call scenario in the PBX, Jambonz, SIPp, and proxy gates; `security.fuzz-sdp`; `sdp_matcher_integration.rs::multi_m_line_offer_independently_matched`. Full grammar, BUNDLE, trickle ICE, WebRTC negotiation not claimed. |
| 3550 | RTP / RTCP | Release-gated | `basic_call` tone-verified scenarios in `interop.asterisk-matrix`, `interop.freeswitch-matrix`, `interop.jambonz-matrix`; `perf.rtp-steady-state`; `interop.remote-libsrtp`; `crates/media/rtp-core/src/packet/rtp.rs::test_serialize_parse_roundtrip`. RTCP scheduling/feedback matrix not claimed. |
| 3551 | RTP/AVP profile (PCMU, PCMA) | Release-gated | `interop.jambonz-matrix` (`PBX_G711_PROFILES=pcmu pcma`); `basic_call` in the Asterisk/FreeSWITCH matrices; `security.fuzz-g711` |
| 4733, 2833 | telephone-event DTMF | Release-gated | `dtmf` scenario in `interop.asterisk-matrix`, `interop.freeswitch-matrix`, `interop.jambonz-matrix`; `interop.browser-dtmf` (Chromium, via `rvoip-webrtc`) |
| 3389 | Comfort noise | Tested | `crates/media/media-core/src/relay/controller/cn_transmitter.rs::send_emits_pt13_with_level_byte`; `config_channel_capacity_integration.rs::comfort_noise_payload_requires_comfort_noise_flag`. Off by default. |
| 4867 | AMR / AMR-WB payload | Release-gated (feature-gated `amr-nb`/`amr-wb`) | `interop.amr-rate-sweep` (17 modes over UDP and TLS+SRTP against Asterisk), `interop.proxy-pbx.{kamailio,opensips}.matrix` (`PBX_REQUIRE_AMR=1`), `amr_call` in the PBX matrices; bit-exact decode: `crates/media/codec-core/src/codecs/amr/mod.rs::the_public_api_decodes_bit_exactly_at_every_rate`. Excluded from the full-media performance claim. |
| ITU-T G.729 (via 3551 §4.5.6) | G.729A / G.729AB | Release-gated (feature-gated `g729`) | `g729_call` with `PBX_G729_PROFILES=g729a g729ab` in `interop.asterisk-matrix`, `interop.freeswitch-matrix`; `crates/media/codec-core/src/codecs/g729/mod.rs::g729ba_accepts_sid_and_nodata_payloads`. Excluded from the full-media performance claim. |
| 6716, 7587 | Opus codec and payload | Tested (feature-gated `opus`) | `crates/media/codec-core/src/codecs/opus.rs::test_encoding_decoding_roundtrip`; `crates/media/rtp-core/src/payload/opus.rs::test_opus_payload_format`. No peer gate. |
| ITU-T G.722 | G.722 | Not supported | Construction/negotiation rejection tests (COMPATIBILITY_MATRIX) |
| 2029, 2250, 2435, 4587, 4629, 6184, 7741, 8741 | Video payload formats (H.261, MPEG, JPEG, H.263, H.264, VP8, VP9) | Parsed only | Payload-type registry: `crates/media/rtp-core/src/payload/registry.rs`. No SIP video call claim. |
| 4585, 5124 | AVPF / SAVPF profiles | Parsed only | Profile tokens and `a=rtcp-fb`: `crates/sip/sip-core/src/sdp/media/transport.rs`; feedback packet types in `crates/media/rtp-core/src/feedback/` have no named tests |
| 5104 | Codec control messages | Parsed only | `crates/media/rtp-core/src/feedback/packets.rs` |
| 3611 | RTCP XR | Tested (packet layer only) | `crates/media/rtp-core/src/packet/rtcp/xr.rs::test_voip_metrics_block`. No XR reporting schedule; SIP_RFC_COMPLIANCE lists XR as not implemented at the behaviour level. |
| 5506 | Reduced-size RTCP | Not supported | Not implemented (SIP_RFC_COMPLIANCE.md) |
| 5761 | RTP/RTCP multiplexing | Tested | `crates/media/rtp-core/src/transport/security_transport.rs::wrapper_protects_and_unprotects_srtcp_on_muxed_and_separate_sockets`; `a=rtcp-mux` parsed in `crates/sip/sip-core/src/sdp/attributes/rtcp.rs` |
| 8285, 5285 | RTP header extensions | Tested | `crates/media/rtp-core/src/packet/extension/mod.rs::test_one_byte_extensions`, `::test_two_byte_extensions` |
| 6464 | Client-to-mixer audio level | Parsed only | `a=extmap` and extension ids in `extension/mod.rs`, `sdp/attributes/extmap.rs` |
| 6465 | Mixer-to-client audio level | Not supported | Absent |
| 5450, 7742, 8852 | Transmission offset, video orientation, RID extension | Parsed only | `crates/media/rtp-core/src/packet/extension/mod.rs` |
| 3556, 3605, 3890, 4091, 4145, 5888, 5576, 8830, 8839, 8841, 8843, 8851, 8853, 8864, 4960 | SDP attributes: bandwidth, `a=rtcp`, TIAS, ANAT, `a=setup`/COMEDIA, grouping, `a=ssrc`, msid, ICE attributes, sctpmap, BUNDLE, RID, simulcast, data channel | Parsed only | `crates/sip/sip-core/src/sdp/attributes/`, `sdp/parser/attribute_parser.rs`. Behaviour for BUNDLE/RID/simulcast/data channels is owned by `rvoip-webrtc`, not the SIP stack. |
| 4855 | Media type registration of RTP payloads | Parsed only | `crates/sip/rvoip-sip/src/media_stream.rs` |

### Security (media and signalling)

| RFC | Title | Status | Evidence |
|---|---|---|---|
| 3261 §22, 7616, 2617 | Digest authentication | Release-gated (MD5 against PBX peers) | `registration` scenario in `interop.asterisk-matrix`, `interop.freeswitch-matrix`, `interop.jambonz-matrix`; `crates/identity/auth-core/src/sip_digest.rs::sha256_round_trip_with_authenticator`, `::auth_int_includes_body_in_ha2`; `oob_auth_retry.rs::message_with_credentials_recovers_once_from_stale_nonce`. SHA-256 and `auth-int` are Tested only. |
| 8760 | SIP Digest added algorithms (SHA-512-256) | Implemented, untested against peers | Algorithm identifiers in `crates/identity/auth-core/src/sip_digest.rs` and `crates/sip/rvoip-sip/src/auth/mod.rs`; no test named for SHA-512-256 was located. COMPATIBILITY_MATRIX says Supported; SIP_RFC_COMPLIANCE says not claimed. |
| 7617 | Basic authentication | Tested (explicit cleartext opt-in) | `oob_auth_retry.rs::message_with_basic_auth_requires_explicit_cleartext_opt_in`, `::message_with_basic_auth_retries_when_cleartext_opted_in` |
| 8898, 6750 | Bearer / OAuth tokens in SIP | Tested | `oob_auth_retry.rs::message_with_bearer_token_retries_with_bearer_authorization`; `crates/sip/sip-core/tests/www_authenticate_bearer_test.rs`; validators in `crates/identity/auth-core/src/{bearer,jwt,jwks}.rs` |
| 3310, 4169 | IMS AKA (AKAv1-MD5, AKAv2-MD5) | Tested (provider-backed shape only) | `crates/sip/rvoip-sip/tests/endpoint_unified_auth.rs::endpoint_uac_retries_aka_provider_shape_against_unified_uas`. Vector issuance, USIM, Milenage, and IMS certification are the application's (SECURITY_POSTURE.md). |
| 3329 | Security mechanism agreement | Not supported | Option tag mention only; planned |
| 3711 | SRTP | Release-gated | `interop.remote-libsrtp` (libsrtp known answers: `crates/media/rtp-core/tests/srtp_libsrtp_known_answers.rs::protects_rtp_to_libsrtp_known_answer`; `srtp_interop_webrtc_srtp.rs::our_encrypt_their_decrypt_agree`), `interop.amr-rate-sweep.amr{nb,wb}-tls` (TLS+SRTP against Asterisk); `security.fuzz-srtp` |
| 4568 | SDES (`a=crypto`) | Release-gated | As 3711 (SDES-keyed); `crates/sip/rvoip-sip/tests/srtp_call_integration.rs::srtp_call_negotiates_and_establishes_end_to_end`. Suites: `AES_CM_128_HMAC_SHA1_{80,32}`, `AES_256_CM_HMAC_SHA1_{80,32}` ([CRYPTO_CAPABILITIES.md](../crates/sip/rvoip-sip/docs/CRYPTO_CAPABILITIES.md)). Base64 per RFC 4648, compatible-padding mode by default. |
| 6188 | AES-256 SRTP | Tested | `srtp_libsrtp_known_answers.rs` (`aes256_context`) |
| 7714 | AES-GCM SRTP | Not supported | Fails closed: `crates/media/rtp-core/tests/security_fail_closed.rs::aes_gcm_profiles_retain_identity_but_cannot_be_constructed_or_advertised` |
| 5763, 5764, 8842 | DTLS-SRTP framework, keying, offer/answer | Release-gated (feature-gated `dtls-srtp`) | `interop.remote-libsrtp` ("pinned SRTP and DTLS-SRTP interoperability"); `crates/sip/rvoip-sip/tests/dtls_srtp_call_integration.rs::dtls_srtp_call_negotiates_and_installs_contexts_on_both_endpoints`. No SIP PBX peer performs DTLS-SRTP in the ledger. See the contradiction note in §6. |
| 8122, 4572 | SDP fingerprint | Tested | `dtls_srtp_call_integration.rs`; `crates/sip/rvoip-sip/src/adapters/dtls_negotiator.rs` |
| 7983 | DTLS/RTP/STUN demultiplexing | Tested | `crates/media/rtp-core/tests/dtls_srtp_transport_bridge_test.rs::handshake_completes_through_the_shared_socket_demux_bridge` |
| 6347, 5705 | DTLS 1.2 and key derivation | Tested | `crates/media/rtp-core/tests/dtls_srtp_handshake_test.rs`; `crates/media/rtp-core/src/dtls/crypto/keys.rs`; `security.fuzz-dtls` |
| 3830 | MIKEY | Not supported | Lower-layer code exists under `crates/media/rtp-core/src/security/mikey/` but is never advertised or negotiated (CRYPTO_CAPABILITIES.md) |
| 6189 | ZRTP | Not supported | As MIKEY (`security/zrtp/`) |
| 8224 | Authenticated identity (`Identity` header) | Tested | `crates/sip/sip-dialog/tests/identity_verify_inbound.rs::rfc_8224_status_codes`, `::policy_gate_matches_rfc_8224_intent`; `identity_sign_outbound.rs::signer_receives_e164_tn_from_tel_uri`; byte preservation in `crates/sip/sip-transport/tests/raw_bytes_preservation.rs` |
| 8225 | PASSporT | Tested | `crates/extensions/rvoip-stir-shaken/tests/sign_verify_round_trip.rs::full_sign_then_verify_round_trip_yields_valid`, `::tampered_signature_yields_bad_signature` |
| 8226 | STI certificates | Tested | `crates/extensions/rvoip-stir-shaken/tests/chain_validation.rs` |
| 8588 | SHAKEN PASSporT extension | Implemented, untested against peers | `crates/sip/sip-dialog/src/manager/identity_verify.rs`; attestation-level claims constructed in `chain_validation.rs::matching_claims`. No STI-CA, STI-VS, or carrier trust-anchor interop. |
| 4474 | SIP Identity (2006) | Not supported (obsoleted by 8224) | Tracked via 8224 |
| 8946 | PASSporT `div` | Not supported | Absent |
| 9421, 8785 | HTTP message signatures, JCS | Not a SIP claim | Used by `rvoip-uctp` envelopes; outside the SIP stack |

### Presence and events

| RFC | Title | Status | Evidence |
|---|---|---|---|
| 6665, 3265 | SIP event notification | Tested | `crates/sip/sip-dialog/tests/subscription_dialogs.rs::test_subscribe_creates_dialog`, `::test_subscribe_with_zero_expires_terminates`; `oob_auth_retry.rs::subscribe_with_credentials_retries_with_full_digest`. Full notifier/subscriber state machines and peer interop not established. |
| 4235 | Dialog event package | Implemented, untested against peers | `crates/sip/rvoip-sip/src/api/dialog_package.rs`; dialog-info NOTIFY construction in `crates/sip/sip-dialog/tests/generated_sip_compliance.rs` |
| 3856, 3863 | Presence package, PIDF | Parsed only | `crates/sip/sip-core/tests/presence_builder_test.rs` (builders only); `crates/sip/sip-core/src/types/pidf.rs` |
| 3842 | message-summary (MWI) | Parsed only | `crates/sip/sip-dialog/src/subscription/event_package.rs` |
| 4575 | Conference event package | Parsed only | `crates/sip/sip-core/src/builder/headers/event.rs` |
| 4662 | Resource lists (RLS) | Not supported | Package name only in `event_package.rs` |
| 3857, 3858, 5263 | Watcher info, partial presence | Not supported | Absent |

## 2. Interoperability attestation

### Peers in the `0.3.10` ledger

| Peer | Pinned version / image | Gate ids | Executed scope | Not covered by this peer |
|---|---|---|---|---|
| Asterisk (`res_pjsip`) | 20.20.1 on Debian 12 (`infra/release-runners/pbx/asterisk/Dockerfile`) | `interop.asterisk-matrix` (+ `up`/`down`/`restore`), `interop.amr-rate-sweep.*` | `Endpoint`, `StreamPeer`, `CallbackPeer`; registration, basic_call (PCMU/PCMA, bidirectional tone analysis), g729_call (G.729A, G.729AB), amr_call, amr_transcode_call, b2bua_call, hold_resume, ring_cancel, dtmf, reject, blind_transfer; UDP and TLS; AMR-NB/WB every mode over UDP and TLS+SRTP | 100rel/PRACK, session timers, SUBSCRIBE/NOTIFY packages, DTLS-SRTP, IPv6 |
| FreeSWITCH (Sofia) | v1.10.12, sofia-sip v1.13.17 (`infra/release-runners/pbx/freeswitch/Dockerfile`) | `interop.freeswitch-matrix` (+ lifecycle) | Same runner and scenario list as Asterisk | Same as Asterisk |
| Jambonz OSS | Release line 0.9.9; `sbc-inbound` `b7b707cc…`, `sbc-outbound` `fec25d5d…`, drachtio/rtpengine/registrar/auth/redis/mysql images digest-pinned (`infra/release-runners/pbx/jambonz/versions.env`); latest-line check `interop.jambonz-latest` | `interop.jambonz-latest`, `interop.jambonz-up`, `interop.jambonz-matrix`, `interop.jambonz-down` (residue proof) | All three APIs; UDP only; PCMU/PCMA; registration, basic_call, hold_resume, ring_cancel, dtmf, reject, blind_transfer with ordered NOTIFY and optional Referred-By | G.729, AMR, b2bua_call, TLS/SRTP, WebRTC, PSTN, application verbs, recording, HA, load; commercial Jambonz 10.x and jambonz.cloud |
| Kamailio | 6.1.3 Bookworm `ghcr.io/kamailio/kamailio:6.1.3-bookworm@sha256:26b26c6…` (`crates/sip/sip-proxy/tests/interop/README.md`) | `interop.remote-proxies.kamailio.{rvoip-first,peer-first}.{udp,tcp,tls}`, `interop.proxy-pbx.kamailio.matrix` | Transaction-stateful proxy in both hop orders (`SIPp UAC -> rvoip -> Kamailio -> SIPp UAS` and the reverse) over UDP, TCP, and hostname-verified mTLS through gate-owned boundaries; scenario inventory: INVITE success, CANCEL before/after provisional, matched/unmatched/retransmitted CANCEL, 2xx and non-2xx ACK routing, forks, Timer C, Via push/pop, Route/Record-Route strict and loose, SIPS, 401/407 aggregation, RFC 3263 failover, stray-response discard, retention drain. Registrar-proxy + rtpengine lab: registration, basic_call, amr_call with the `Endpoint` API (`run.sh` `provider_scenario_supported`) | hold_resume, REFER, DTMF, and TLS cells through the proxy labs are gated off in `run.sh` until proven; native outbound TLS hostname enforcement by the peer is explicitly not claimed |
| OpenSIPS | 3.6.7 (`opensips/opensips:3.6@sha256:eba1396…`) | `interop.remote-proxies.opensips.*`, `interop.proxy-pbx.opensips.matrix` | Same 12-row proxy matrix and rtpengine lab scope as Kamailio | Same as Kamailio; the OpenSIPS lab has no TLS listener for the registrar-proxy media cells |
| SIPp | 3.7.7 (proxy interop README) | `interop.sipp-build/start/matrix/stop`, `perf.sipp-parity` | Standalone `uac_perf.xml` INVITE/200/ACK/BYE at 30, 100, 300, 1,000, 2,000 CPS against `perf_listener`, `RVOIP_PERF_MIN_SUCCESS_PCT=99.9`; proxy interop UAC/UAS scenarios | The SIPp matrix in [INTEROP_CI_PLAN.md](../crates/sip/rvoip-sip/docs/INTEROP_CI_PLAN.md) (PRACK, REFER, INFO, malformed) is a plan; only the call-setup comparison and the proxy scenarios are gated |
| baresip | Version recorded at run time (`run_strict_ua.sh` logs `baresip -h`) | `interop.strict-ua` | Strict-UA INVITE, 200 OK, ACK, media start, BYE against the rvoip listener | Anything beyond a single call |
| libsrtp / webrtc-srtp | libsrtp 2.8.0 at commit `24b3bf8f…` (`scripts/test_libsrtp_interop.sh`) | `interop.remote-libsrtp` | SRTP/SRTCP known answers, DTLS-SRTP context agreement at the RTP layer | Not a SIP call peer |
| Chromium | Pinned by `rvoip-webrtc` browser test | `interop.browser-dtmf` | Outbound RFC 4733 through `rvoip-webrtc` | SIP stack not involved |

### How the attestation is bound

- Source: `source.clean-start` fingerprints a clean tree before any gate;
  `source.final-capture` / `source.canonical-2k-unchanged` / `source.remote-clean` /
  `source.remote-final` prove it did not change. Every row of the gate report
  carries the candidate SHA, a receipt SHA-256, and a command-log SHA-256.
- Catalog and plan: gate catalog `bfbcfeee…`, plan `ee35c1a0…`, aggregate
  `95f715ec…` ([BETA_RELEASE_REPORT.md](../crates/sip/rvoip-sip/docs/BETA_RELEASE_REPORT.md)).
- Peers: image digests and source tarball hashes are release inputs; floating
  tags are prohibited; Jambonz must still be the latest stable OSS line at run
  start (`interop.jambonz-latest`); OpenSIPS runtime version is asserted with
  `opensips -V`.
- Refuse-to-report rules (`scripts/release/render_qualification_reports.py`):
  native report generation "requires a clean, unchanged, zero-skip full PASS";
  a "missing, skipped, ambiguous, unpinned, or failing required" peer prevents a
  release-candidate report; the proxy matrix must contain exactly both peers,
  both adjacency orders, and UDP/TCP/verified-TLS rows, with each peer covering
  the complete scenario inventory independently.
- Legacy coverage: all 108 requirements of the earlier strict beta ledger are
  mapped onto the 213-gate profile (`remote_release_legacy_coverage` in
  `gates.json`, `unautomated_legacy_ids: []`).

### Not covered

From the evidence matrix non-claims, COMPATIBILITY_MATRIX, and the run
configuration: 100rel with any PBX; session-timer negotiation with any PBX;
SUBSCRIBE/NOTIFY event packages with any peer; DTLS-SRTP with any SIP peer;
IPv6 on the network path; WSS; ICE/TURN; any carrier SBC or IMS core;
PSTN; any peer version other than the pinned one; Jambonz over TLS/SRTP or with
G.729/AMR; hold/resume, transfer, and DTMF through the Kamailio/OpenSIPS
registrar-proxy labs (gated off in `run.sh`). The
[remote endpoint profile](sip/REMOTE_ENDPOINT_PROFILE.md) (RFC 5626 + TLS +
mandatory SDES) is deterministic-tested but "not release-qualified until
protected evidence also records two independent UAs behind real NAT".

## 3. Performance

### What was measured, where

All `0.3.10` performance rows ran on ephemeral GCP workers over loopback
networking (environment `rvoip-release-v5-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-n2-cascade-lake`).
Machine classes ([AWS_RELEASE_WORKERS.md](AWS_RELEASE_WORKERS.md) "Was" column):
performance and long-soak workers `n2-standard-8` (8 vCPU Cascade Lake, 32 GB),
short-soak `n2-standard-4`, interop `n2-standard-4`, proxy interop
`n2-standard-2`. Performance executables were compiled once on an
`n2-standard-32` builder and verified by SHA-256 on each worker so compilation
never overlaps measurement ([RELEASING.md](RELEASING.md)). The branch has since
moved the fleet to AWS `m5` equivalents; that change is not part of the
`0.3.10` evidence.

The performance report states the reading rule: "Call-setup sweeps use
loopback networking on the recorded GCP qualification host. They establish
repeatable release regression evidence, not public-network latency or carrier
capacity."

### Canonical 2,000 CPS full-media run (three passes)

Gate `perf.canonical-2k-current` runs `canonical_2k_release_eval.sh` three
times on the `pbx-media-server` profile with media enabled (PCMU/PCMA/DTMF), four
Alice shards pinned to 16 dispatcher workers each. Acceptance thresholds are in
`crates/sip/rvoip-sip/scripts/perf_2k_acceptance.py`:

| Metric | Threshold | Pass 1 (`…021744Z`) | Pass 2 (`…023255Z`) | Pass 3 (`…024805Z`) |
|---|---|---|---|---|
| Calls at target 2,000 CPS | 65,000/65,000 | 65,000/65,000 | 65,000/65,000 | 65,000/65,000 |
| ASR | >= 0.999 | 1.0 | 1.0 | 1.0 |
| NER | >= 0.999 | pass | pass | pass |
| Achieved CPS | (reported) | 1,857.13 | 1,857.12 | 1,857.13 |
| Setup p99 | <= 16.69 ms | 4.01 ms | 3.463 ms | 3.86 ms |
| Full-cycle p99 | <= 159.66 ms | pass | pass | pass |
| Peak RSS | <= 3,202.26 MB | 2,011.39 MB | 2,015.45 MB | 2,025.25 MB |
| Post-drain RSS growth | <= 10 MB/h | pass | pass | pass |

Achieved/target is 0.9286 at every step of every sweep (27.86/30, 92.85/100,
278.56/300, 928.57/1,000, 1,857.1/2,000) on both the `pbx-media-server` and
`signaling-only-server-high-performance` profiles. That is the driver's pacing
ratio, not a throughput knee; the report publishes achieved CPS rather than
normalising it.

### Other archived rows (single-shot, same run)

| Scenario (gate) | Target | Result |
|---|---|---|
| `perf_call_setup_cps_signaling-only-server-high-performance` (`perf.call-setup-signaling`) | 2,000 CPS | 65,000/65,000, ASR 1.0, p99 3.24 ms, peak RSS 2,006.62 MB |
| `perf_call_setup_cps_endpoint` (`perf.call-setup-endpoint`) | 30 CPS | 975/975, p99 2.198 ms |
| `perf_concurrent_active_calls` (`perf.concurrent-calls`) | 500 active | ASR 1.0, p99 190.579 ms, peak RSS 211.71 MB |
| `perf_tls_overhead` (`perf.tls-overhead`) | 100 CPS | 3,250/3,250, p99 20.578 ms |
| `perf_srtp_overhead` (`perf.srtp-overhead`) | 50 CPS | ASR 1.0, p99 26.034 ms |
| `perf_b2bua_forwarding` (`perf.b2bua`) | 30 CPS | 975/975, p99 2.439 ms |
| `perf_pdd_with_180_first` (`perf.pdd-180`) | 50 CPS | 1,625/1,625, p99 2.212 ms |
| `perf_registration_throughput`, `perf_registrar_binding_scale` | 100 | PASS; peak RSS 81.54 / 80.66 MB |
| `perf_backpressure_step` (`perf.backpressure`) | 200 CPS step | 13,851/13,851 |
| `perf_sipp_parity` (`perf.sipp-parity`) | 20 CPS | PASS (the archived row shows setup p99 `0`; treat that cell as not emitted) |

### Burst profiles

Gates `perf.media-burst.{carrier-smoke,access-edge-microburst,contact-center-flash,shift-change-long-hold,overload-recovery,high-density-media-burst,buffer-ab-legacy}`
and the aggregate `perf.media-burst-matrix` all PASS in the ledger. Shapes and
acceptance are in
[`perf-burst-scenarios.yaml`](../crates/sip/rvoip-sip/config/perf-burst-scenarios.yaml):

| Scenario | Shape | Acceptance |
|---|---|---|
| access-edge-microburst | 20 CPS baseline, bursts of 120 and 160 CPS for 20 s each, capacity 6,000 | ASR >= 0.999, 0 media-setup/teardown failures, 0 retained after drain, RSS <= 15 MB/h, recovery <= 60 s |
| contact-center-flash | 220 CPS for 45 s over 25 CPS, capacity 8,500, 75/20/5 short/medium/long holds | ASR >= 0.999, recovery <= 90 s |
| shift-change-long-hold | 90 CPS for 90 s, 40 % of holds 241–600 s | ASR >= 0.999, recovery <= 120 s |
| overload-recovery | 300 CPS into a 250-slot admission cap | Overload rejections allowed; recovery ASR >= 0.999 within 5 s; 0 leaks |
| high-density-media-burst | 160 CPS full-RTP for 90 s, capacity 12,500 | ASR >= 0.995 (the one profile that permits 0.5 % setup loss), all cleanup gates exact |

Per-scenario counters live in the run's evidence artifact, not in the
repository report. The `0.3.2` release carried a documented exception on this
profile (17,871/18,000, ASR 0.9928;
[BETA_PERFORMANCE_EXCEPTION.md](../crates/sip/rvoip-sip/docs/BETA_PERFORMANCE_EXCEPTION.md));
`0.3.10` passed it without exception.

### Exclusions from the performance claim

- The supported general full-media claim is "up to 2,000 CPS with media
  enabled" for PCMU/PCMA/telephone-event ([TOPOLOGY_PROFILES.md](../crates/sip/rvoip-sip/docs/TOPOLOGY_PROFILES.md)).
  G.729, AMR-NB/WB, Opus, SRTP-at-scale, TLS-at-scale, and B2BUA-at-scale are
  measured only at the small single-shot targets above.
- Results above 2,000 CPS are "tuned or experimental" (COMPATIBILITY_MATRIX
  performance profiles; [TUNING.md](../crates/sip/rvoip-sip/docs/TUNING.md)).
- No 24-hour claim; no public-network latency claim; no per-call CPU or
  power figures.
- [CARRIER_BURST_TUNING.md](../crates/sip/rvoip-sip/docs/CARRIER_BURST_TUNING.md)
  is the experiment ledger for `access-edge-microburst`. Its stated
  conclusion: "no `access-edge-microburst` Config recipe is ready to promote";
  media-plane pacing candidates pass signalling gates but "audio-quality
  diagnostics still show RTP continuity risk under overload". Operators should
  start from TUNING.md's "Recipe: Carrier Media Burst" and "Stress Tuning
  Decision Guide" rather than from ledger rows.

## 4. Reliability

| Evidence | Gate / source | What it shows |
|---|---|---|
| One-hour split soak, 500 active calls | `perf.soak-candidate` (`perf_soak_split.sh`; `RVOIP_PERF_SOAK_ACTIVE_CALLS=500`, hold 10–360 s, 3,600 s, `RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15`) | `perf_soak_caller`: 9,904/9,904 calls, ASR 1.0, setup p99 175.636 ms, peak RSS 139.65 MB; `perf_soak_receiver`: peak RSS 154.05 MB |
| One-hour monolithic soak, 30 active calls | `perf.monolithic-soak` | `perf_soak_30min`: 587/587, ASR 1.0, p99 15.737 ms, peak RSS 101.63 MB |
| Mass teardown | `perf.mass-teardown` (500 calls torn down together, 160 s retention drain) | Peak RSS 225.59 MB; 0 retained |
| Session churn leak | `perf.session-churn` | 250/250, peak RSS 82.12 MB |
| Transport recovery, resiliency invariants | `perf.transport-recovery`, `perf.resiliency-all` (`crates/sip/rvoip-sip/tests/resiliency_invariants.rs`) | PASS |
| Overload behaviour | `perf.media-burst.overload-recovery` | Admission cap rejects cleanly; recovery ASR >= 0.999 within 5 s |
| Regression audit | `report.regression-baseline`, `report.regression-audit` against `crates/sip/rvoip-sip/perf-baselines/20260706T181609Z` | PASS |

### Fail-closed evidence rules

- Every perf gate binds its JSON to the tested commit, a clean tree, and
  rvoip-sip at the release version ("Evidence integrity" in the release report).
- Blank cells in the performance report mean "not emitted", never zero.
- Ignored resilience stubs under `crates/sip/rvoip-sip/tests/resilience/`
  (RFC 3261/3262/3263/3311/3581/4028/5626 recovery) compile but are not
  evidence; the matrix excludes them explicitly.
- Host drop counters: `scripts/release/linux_performance_host.py` snapshots
  `/proc/net/snmp` UDP `RcvbufErrors`+`SndbufErrors`, `/proc/net/softnet_stat`
  dropped and time-squeeze totals, and loopback `rx_dropped`/`tx_dropped`
  before and after a run. `perf_burst_matrix.sh` takes the snapshot/delta;
  `--require-zero-drops` fails the run on any non-zero delta in
  `udp_dropped_full_socket_buffers`, `softnet_dropped_total`,
  `loopback_rx_dropped`, `loopback_tx_dropped`. The `0.3.2` exception record
  confirms "host UDP full-buffer-drop checks remained within policy" is part
  of the policy.

### Admission and ingress controls

- Ingress request budget and admission observer (landed after `0.3.10`, on
  this branch; [CHANGELOG.md](../CHANGELOG.md) "SIP ingress budget and
  admission observer"): `SipListenerAuthPolicy::with_source_rate_limit` drops
  requests from a source over its token budget before any other admission
  check and sends no response; `with_ingress_observer` reports every
  admitted/rejected/dropped decision as `SipIngressEvent`
  (`crates/sip/rvoip-sip/src/auth/listener.rs`). Exposed through
  `rvoip::app::SipConfig::source_rate_limit` / `ingress_observer`. Not in the
  `0.3.10` ledger.
- Authoritative application ingress (`RvoipAppBuilder::authoritative_ingress`,
  `ingress_health`, `drain(budget)`; `crates/rvoip/src/app.rs`): a lagged
  operational receiver flips `admits_new_work` to false so a readiness probe
  can fail.
- Per-tenant session quota exists but is not yet linearizable across the
  Initiating state (issue #111 in [ISSUE_TRIAGE_0_3_11.md](ISSUE_TRIAGE_0_3_11.md)).
- Inbound OPTIONS is answered 200 before application policy can return 503
  while draining (issue #207, same document).

### RFC 5626 flow recovery and the remote endpoint profile

[REMOTE_ENDPOINT_PROFILE.md](sip/REMOTE_ENDPOINT_PROFILE.md) defines the
bounded profile for phones behind NAT: TLS listener, authenticated REGISTER
with `ob`/`+sip.instance`/`reg-id`, exact process-local flow ownership,
mandatory SDES-SRTP, `439` for incomplete registrations, prepare/commit
replacement with rollback, immediate degradation on connection close, ordered
multi-flow failover. Deterministic tests are listed under RFC 5626 in §1.
Flow tokens are process-local: multi-replica deployments must preserve
AOR/registrar affinity, and a restart invalidates every flow until the UA
re-registers.

### Not yet proven

- Two independent UAs behind real NAT: TLS registration, SDES-SRTP both
  directions, DTMF, hold/resume, expiry and re-registration, NAT rebinding,
  primary-flow loss and failover, restart/affinity recovery (REMOTE_ENDPOINT_PROFILE.md "Release qualification boundary").
- Multi-day soak; failover between rvoip replicas; behaviour under packet
  loss/reordering injected below the SIP layer (resilience stubs).
- Public-network latency or jitter.

## 5. Security

| Area | Status | Evidence |
|---|---|---|
| TLS client and server | Release-gated | `interop.remote-proxies.*.tls`, PBX TLS cells, `interop.amr-rate-sweep.*-tls`; `tls_handshake_test.rs`; rustls 0.23. `dev-insecure-tls` is a test-only feature that "must not appear in production recipes". |
| mTLS | Release-gated in the proxy TLS rows (gate-owned CA, exact DNS-SAN client certificates); "Partial" for general use | `crates/sip/sip-transport/tests/mtls_server_auth.rs`; SECURITY_POSTURE: "Do not market broad mTLS interop until external peer-verification matrices are archived." |
| SDES-SRTP | Release-gated | §1 rows 3711/4568; suites `AES_CM_128_HMAC_SHA1_{80,32}`, `AES_256_CM_HMAC_SHA1_{80,32}`; AEAD-GCM, MIKEY, ZRTP fail closed |
| DTLS-SRTP | Release-gated at the RTP layer (`interop.remote-libsrtp`), feature-gated `dtls-srtp`; SIP-peer DTLS-SRTP not tested | §1 rows 5763/5764/8842; `UDP/TLS/RTP/SAVP`, SHA-256 fingerprint, RFC 8842 `a=setup`, RFC 7983 demux on the RTP socket |
| Digest (MD5, MD5-sess, SHA-256, SHA-256-sess, SHA-512-256, SHA-512-256-sess; `auth`, `auth-int`; stale-nonce recovery once) | Release-gated for MD5 against PBX peers; SHA-256 Tested; SHA-512-256 untested | §1; unsupported algorithms fail rather than downgrade (`oob_auth_retry.rs::message_with_credentials_rejects_unsupported_digest_algorithm`) |
| Bearer / JWT / JWKS | Tested | §1 row 8898 |
| Basic | Tested, cleartext requires explicit opt-in | §1 row 7617 |
| IMS AKA | Provider-backed API only | §1 row 3310 |
| STIR/SHAKEN (8224/8225/8226/8588) | Tested library support; no carrier certification, no STI-CA/STI-VS interop | §1; SECURITY_POSTURE: "Library support and SIP `Identity` preservation only." |
| Trace redaction | Tested | `crates/sip/rvoip-sip/tests/trace_redaction.rs`; redacts auth headers, tokens, identity headers, SDES `a=crypto`, ICE passwords; flow tokens redacted from events |
| Dependency advisories | Release-gated | `security.advisory-audit`, `security.remote-advisories` (`cargo deny check advisories bans sources`). "PASS does not prove the absence of vulnerabilities." |
| Parser fuzz smoke | Release-gated | `security.fuzz-{sip-message,uri,header,sdp,rtp,rtcp,srtp,dtls,stun,g711}`, `security.fuzz-amr-{unpack,decode,encode}`; 1,000 runs / 10 s each per gate |

Explicit non-claims from [SECURITY_POSTURE.md](../crates/sip/rvoip-sip/docs/SECURITY_POSTURE.md):
ICE and TURN; browser/WebRTC security; ZRTP and MIKEY; WSS outbound; Basic
over cleartext as a recommendation; built-in SIM/USIM or Milenage; STIR/SHAKEN
certification. The same document lists DTLS-SRTP both as "Supported,
feature-gated (bounded)" and, under non-claims, as "post-beta"; see §6.

## 6. Known gaps and roadmap

What a carrier or CPaaS would normally expect and rvoip does not yet claim:

| Gap | Tracking |
|---|---|
| ICE (RFC 8445), TURN (RFC 8656), trickle ICE for SIP endpoints behind arbitrary NAT | Non-claims in RFC_COMPLIANCE_MATRIX.md and SECURITY_POSTURE.md; [ICE_IMPLEMENTATION_PLAN.md](ICE_IMPLEMENTATION_PLAN.md), [TURN_IMPLEMENTATION_PLAN.md](TURN_IMPLEMENTATION_PLAN.md) |
| Live two-UA real-NAT qualification of the RFC 5626 remote endpoint profile | [REMOTE_ENDPOINT_PROFILE.md](sip/REMOTE_ENDPOINT_PROFILE.md); issue #102 (closed as implemented, qualification still pending) |
| IPv6 on the network path | COMPATIBILITY_MATRIX "Not audited" |
| WSS (SIP over secure WebSocket) outbound | COMPATIBILITY_MATRIX "Not supported" |
| 100rel/PRACK, session timers, UPDATE offer/answer against real PBXs | RFC_COMPLIANCE_MATRIX limits on `SIP-3262-100REL`, `SIP-4028-TIMER`, `SIP-3311-UPDATE`; no PBX scenario |
| Attended transfer / Replaces (RFC 3891) end to end | `SIP-3891-REPLACES` Unsupported; README lists attended-transfer primitives as developer preview |
| History-Info, Diversion, Privacy semantics (beyond carry-through) | §1 Parsed-only rows |
| PUBLISH, presence agent, MWI, dialog-event notifier state machines | §1 Presence and events |
| Trunk registration (RFC 6140), reg-event (RFC 3680), GRUU behaviour (RFC 5627) | §1 |
| SHAKEN attestation with real STI-CA/STI-VS, `div` PASSporT | §1 rows 8588/8946 |
| DTLS-SRTP with a SIP PBX peer; AEAD-GCM SRTP | §1; CRYPTO_CAPABILITIES.md |
| RTCP XR reporting, AVPF feedback, reduced-size RTCP | §1 SDP and media |
| Translated RTCP across transcoding bridges | issue #109 in [ISSUE_TRIAGE_0_3_11.md](ISSUE_TRIAGE_0_3_11.md) |
| Linearizable per-tenant admission and quota resize | issue #111 |
| Trusted-network authentication method for trunks | issue #118 |
| Inbound OPTIONS 503/Retry-After policy while draining | issue #207 |
| Production storage for SAML/SCIM/LDAP/WebAuthn (all process-local) | issue #94 |
| Redacted secret references and strict config loading | issue #117 |
| Production multi-call `RvoipApp` runtime | issue #108 |
| Carrier SBC certification, IMS certification | TOPOLOGY_PROFILES.md "Post-beta" |
| Anything above 2,000 CPS as a supported claim | TOPOLOGY_PROFILES.md "Advanced" |
| Deferred design items (AAuth hardening, RFC 9421 default-on signing, DTLS fingerprint binding default-on, SIP-over-QUIC) | [GAP_PLAN.md](GAP_PLAN.md) deferred backlog |

Contradictions between evidence documents found during this review (a carrier
reading them side by side will notice):

1. DTLS-SRTP: RFC_COMPLIANCE_MATRIX.md and SECURITY_POSTURE.md's table say
   "Supported, feature-gated (bounded)" with the `interop.remote-libsrtp` gate;
   SECURITY_POSTURE.md's non-claims, COMPATIBILITY_MATRIX.md, TOPOLOGY_PROFILES.md,
   and docs/sip/SIP_RFC_COMPLIANCE.md still say "Post-beta / explicit non-claim".
2. SHA-512-256 Digest: COMPATIBILITY_MATRIX.md "Supported"; SIP_RFC_COMPLIANCE.md
   "SHA-512/256 not claimed"; no named test found.
3. Session timer `422`/Min-SE: RFC_COMPLIANCE_MATRIX.md lists it as not claimed;
   `session_422_retry.rs::invite_422_retry_bumps_session_expires_and_succeeds` exists.
4. Kamailio/OpenSIPS registrar-proxy scope: the top-level README says
   "registration, calls, AMR in all four framings, DTMF, SDES-SRTP"; the runner
   (`run.sh` `provider_scenario_supported`) admits only `registration`,
   `basic_call`, `amr_call` for those providers, the OpenSIPS lab has no TLS
   listener, and INTEROP_CI_PLAN.md describes Jambonz PAI/Diversion policy cells
   that are not in the scenario list.
5. Release pointers: RFC_COMPLIANCE_MATRIX.md, COMPATIBILITY_MATRIX.md,
   SECURITY_POSTURE.md, CRYPTO_CAPABILITIES.md, and TOPOLOGY_PROFILES.md still
   name the `0.3.9` 208-gate run as the current authority while
   BETA_RELEASE_REPORT.md and BETA_GATE_REPORT.md carry the `0.3.10` 213-gate
   run; docs/sip/README.md cites a June attestation basis. COMPATIBILITY_MATRIX
   lists the Jambonz row as "pending" and TCP as not yet in an external matrix,
   although both are PASS in the `0.3.10` ledger.
6. BETA_RELEASE_REPORT.md records the evidence artifact as `pending-upload`.

## 7. How to reproduce

| Purpose | Command | Reference |
|---|---|---|
| Full local beta gate (PBX interop, SIPp, baresip, perf, fuzz, torture) | `crates/sip/rvoip-sip/scripts/beta_gate.sh --full` (modes: `--local`, `--interop`, `--perf`, `--security`; add `--require-external` or `BETA_GATE_REQUIRE_EXTERNAL=1` so missing peers fail instead of skip) | [INTEROP_CI_PLAN.md](../crates/sip/rvoip-sip/docs/INTEROP_CI_PLAN.md), [BETA_RELEASE_CHECKLIST.md](../crates/sip/rvoip-sip/docs/BETA_RELEASE_CHECKLIST.md) |
| One PBX matrix | `crates/sip/rvoip-sip/examples/pbx/run.sh --pbx asterisk\|freeswitch\|jambonz\|kamailio\|opensips --api all --scenario all --transport UDP\|TLS\|all` | [`examples/pbx/README.md`](../crates/sip/rvoip-sip/examples/pbx/README.md) |
| Bring a lab up/down as the release does | `bash infra/release-runners/interop-lifecycle.sh jambonz-up` (also `asterisk-`, `kamailio-`, `opensips-` `up`/`down`) | [`infra/release-runners/README.md`](../infra/release-runners/README.md) |
| Stateful proxy matrix | `bash crates/sip/sip-proxy/tests/interop/scripts/beta_gate.sh <kamailio\|opensips> <rvoip-first\|peer-first> <udp\|tcp\|tls>` | [`sip-proxy/tests/interop/README.md`](../crates/sip/sip-proxy/tests/interop/README.md) |
| AMR per-rate sweep | `crates/sip/rvoip-sip/examples/pbx/rate-sweep.sh --pbx asterisk --profile amrnb\|amrwb --transport UDP\|TLS` | gate `interop.amr-rate-sweep` |
| SIPp comparison | `crates/sip/rvoip-sip/tests/perf/sipp_scenarios/run_comparison.sh 127.0.0.1 35060 rvoip` with `RVOIP_PERF_CPS="30 100 300 1000 2000" RVOIP_PERF_MIN_SUCCESS_PCT=99.9` | gate `interop.sipp-matrix` |
| Canonical 2,000 CPS three-pass evaluation | `RVOIP_CANONICAL_EVAL_OUTPUT=<dir> crates/sip/rvoip-sip/scripts/canonical_2k_release_eval.sh` | [BENCHMARKING.md](../crates/sip/rvoip-sip/docs/BENCHMARKING.md) |
| Burst matrix with host drop deltas | `RVOIP_PERF_BURST_SCENARIOS=all crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh` | [CARRIER_BURST_TUNING.md](../crates/sip/rvoip-sip/docs/CARRIER_BURST_TUNING.md) |
| One-hour split soak | `RVOIP_PERF_SOAK_DURATION_SECS=3600 RVOIP_PERF_SOAK_ACTIVE_CALLS=500 RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15 crates/sip/rvoip-sip/scripts/perf_soak_split.sh` | gate `perf.soak-candidate` |
| Claim-to-evidence static check | `cargo test -p rvoip-sip --test beta_release_docs` | [RFC_COMPLIANCE_MATRIX.md](../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md) |
| Generated RFC validity suites (feature-gated; a bare `cargo test` skips them) | `cargo test -p rvoip-sip --features generated-validation --test generated_sip_compliance`; `cargo test -p rvoip-sip-core --test rfc_compliance` | [docs/sip/SIP_RFC_COMPLIANCE.md](sip/SIP_RFC_COMPLIANCE.md) |
| Hosted qualification | `.github/workflows/release-qualify.yml` with `profile` = `remote-preflight` (capacity probe), `remote-diagnostic` (up to 20 named gates), `remote-core` (hosted-runner dry run, 104 gates), `remote-release` (full 213 gates on real workers) | [RELEASING.md](RELEASING.md), [`gates.json`](../scripts/release/gates.json) `profiles` |
| Verify a published qualification | `scripts/release.sh verify --version X.Y.Z --remote-qualification /path/to/aggregate.json` | [RELEASING.md](RELEASING.md) |

The gate catalog is the authoritative list of what a qualification runs;
`scripts/release/gates.py` and `build_gate_catalog.py` generate and validate it,
and `scripts/test_release_gates.py` pins its shape.
