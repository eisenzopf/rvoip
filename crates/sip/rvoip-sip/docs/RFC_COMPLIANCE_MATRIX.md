# rvoip-sip Beta Standards Evidence Matrix

Date: 2026-07-25

This document maps each retained beta-profile claim to exact, non-ignored
executable test sources. It is a claim boundary, not a declaration that the
entire RFC is implemented and not a statement that every listed test passed on
the current source tree.

The earlier July 20 evidence remains bound by the immutable
[baseline manifest](BETA_BASELINE_EVIDENCE_20260720T055257Z.json) and remains
diagnostic-only. Current execution evidence is the clean, source-matched
candidate `20260724T231400Z`: see the
[Beta Release Candidate Report](BETA_RELEASE_REPORT.md) and exact
[108-gate report](BETA_GATE_REPORT.md).

## Status and evidence rules

- **Supported (bounded)** means the behavior stated in that row has executable
  construction or wire evidence. It does not mean unqualified RFC compliance.
- **Partial** means only the stated subset is claimed; the limit column is part
  of the claim.
- **Unsupported** means the beta profile makes no behavior claim. Builder or
  data-model tests may be listed to explain why they are insufficient.
- Evidence strength is **unit** (in-process behavior), **construction** (message
  or header creation/validation), **wire** (local socket or multi-endpoint
  exchange), or **interop** (an independently implemented peer).
- `T-*` entries identify current non-ignored executable tests. They are source
  inventory, not a fresh run result. `J20-*` entries are historical run evidence
  and inherit the dirty-source limitation in the baseline manifest.
- Ignored skeletons, comments, configuration fields, and implementation
  presence are not compliance evidence.

## Retained beta-profile claims

| Claim ID | Standard | Bounded beta claim | Status | Evidence IDs | Explicit limit |
|---|---|---|---|---|---|
| SIP-3261-CORE | RFC 3261 | Core request construction plus INVITE-dialog CANCEL and BYE completion/cleanup behavior. | Partial | `T-3261-C1`, `T-3261-W1`, `T-3261-W2`, `J20-I1` | No section-by-section transaction, proxy, registrar, transport, or error-path certification is claimed. |
| SIP-3262-100REL | RFC 3262 | PRACK construction, reliable `183`/PRACK exchange, and unsupported-policy rejection with `420`. | Partial | `T-3262-C1`, `T-3262-W1`, `T-3262-W2` | Forking, loss/retransmission matrices, and independent-PBX reliable-provisional evidence are not established here. |
| SIP-3263-LOCATION | RFC 3263 | Client NAPTR/SRV/A resolution and recoverable candidate failover for outbound requests. | Supported (bounded) | `T-3263-U1`, `T-3263-U2`, `T-3263-W1` | This is not a general DNS service claim and does not establish every RFC 3263 transport or failure permutation. |
| SIP-3264-OA | RFC 3264 | Audio offer/answer codec intersection, direction propagation, and an established-dialog re-INVITE carrying SDP on the wire. | Partial | `T-3264-U1`, `T-3264-U2`, `T-3264-W1` | Complex multi-stream renegotiation, all glare permutations, and WebRTC negotiation are outside this claim. |
| SIP-3311-UPDATE | RFC 3311 | In-dialog UPDATE transmission plus `401` and `407` digest retry on the same method. | Partial | `T-3311-W1`, `T-3311-W2`, `T-3311-W3` | No complete UPDATE offer/answer, glare, retry-after, or independent-peer matrix is claimed. Ignored resilience skeletons are not evidence. |
| SIP-3325-PAI | RFC 3325 | Configured and per-call `P-Asserted-Identity` reach the receiving endpoint, with per-call override behavior. | Partial | `T-3325-W1`, `T-3325-W2` | Trusted-domain policy, privacy interactions, and carrier certification are not claimed. |
| SIP-3515-REFER | RFC 3515 | Blind REFER request construction, end-to-end blind transfer, and typed NOTIFY progress/final status on the wire. | Supported (bounded) | `T-3515-W1`, `T-3515-W2`, `T-3515-W3` | Attended transfer and RFC 3891 call replacement are excluded. |
| SIP-3581-RPORT | RFC 3581 | Top Via `received`/`rport` response restamping when the inbound request carries the `rport` flag. | Partial | `T-3581-U1`, `T-3581-U2` | No live NAT, multi-hop interoperability, keepalive, ICE, or TURN claim follows from these tests. |
| SIP-3891-REPLACES | RFC 3891 | Call replacement using `Replaces`. | Unsupported | `T-3891-C1`, `T-3891-U1` | The listed tests only construct/carry a Replaces parameter; they do not execute replacement semantics and therefore do not elevate the status. |
| SIP-4028-TIMER | RFC 4028 | Successful session refresh event delivery and refresh-failure event delivery. | Partial | `T-4028-W1`, `T-4028-W2` | Negotiation roles, `422`/Min-SE, proxy behavior, and the complete expiration/race matrix are not claimed. |
| SIP-4475-TORTURE | RFC 4475 | Fixture-driven acceptance of included well-formed messages and rejection of included malformed messages. | Supported with exclusions | `T-4475-U1`, `T-4475-U2` | Well-formed fixtures `3.1.1.2_intmeth.sip`, `4.10_ipv6-bug-abnf-3-colons.sip`, and `3.1.1.1_wsinv.sip` are excluded. The malformed exclusion list is empty. |
| SIP-5626-OUTBOUND | RFC 5626 | Outbound Contact construction with `ob`, `+sip.instance`, and `reg-id`, plus registered-flow configuration validation. | Partial | `T-5626-C1`, `T-5626-U1`, `T-5626-U2` | Flow-token processing, multiple simultaneous flows, keepalive/recovery, failover, and registrar-side behavior are not claimed. Ignored flow-recovery tests are not evidence. |
| SIP-6086-INFO | RFC 6086 | Generic in-dialog INFO transmission and preservation across `401`/`407` authentication retry. | Partial | `T-6086-W1`, `T-6086-W2`, `T-6086-W3` | No Info-Package registry, `Recv-Info` negotiation, or package-specific standards profile is claimed. |
| SIP-6665-SUBSCRIBE | RFC 6665 | Subscription dialog creation/termination primitives, successful NOTIFY handling, subscription-id routing, and authenticated SUBSCRIBE retry. | Partial | `T-6665-U1`, `T-6665-U2`, `T-6665-U3`, `T-6665-W1`, `T-6665-W2` | Full notifier/subscriber state machines, refresh/expiry recovery, forked subscriptions, and independent-peer interoperability are not established. |
| SIP-7118-WS | RFC 7118 | Plain SIP-over-WebSocket client/server round trip delivering REGISTER. | Partial | `T-7118-W1` | WSS release evidence, browser/WebRTC behavior, reconnect, proxy traversal, and complete framing/error matrices are not claimed. |
| SIP-AUTH-DIGEST | RFC 3261 §22, RFC 7616, RFC 8760 | SHA-256 digest generation/validation, `auth-int`, nonce-count progression, stale-nonce retry, and endpoint INVITE digest retry. | Partial | `T-AUTH-U1`, `T-AUTH-U2`, `T-AUTH-U3`, `T-AUTH-W1`, `T-AUTH-W2`, `T-AUTH-W3` | The full algorithm matrix, every challenge-selection rule, independent-server certification, and every SIP method are not claimed. Basic and bearer extensions are not RFC 7616 compliance claims. |
| SDP-8866 | RFC 8866 | SDP audio offer parsing/matching, payload filtering, media direction propagation, and generated INVITE SDP validation. | Partial | `T-SDP-U1`, `T-SDP-U2`, `T-SDP-C1` | Full grammar coverage, every media type, bundle, trickle ICE, and WebRTC negotiation are not claimed. Attribute carry-through alone is not semantic support. |
| RTP-3550 | RFC 3550 | RTP packet serialization/parsing, RTCP receiver-report serialization/parsing, and bidirectional audio/bridge media delivery. | Partial | `T-RTP-U1`, `T-RTP-U2`, `T-RTP-W1`, `T-RTP-W2` | Full RTCP scheduling, feedback profiles, congestion behavior, multicast, and independent RTP-stack certification are not claimed. |
| SRTP-3711-SDES | RFC 3711 and RFC 4568 | SDES-negotiated SRTP call establishment, encrypted media exchange, malformed suite rejection, and executable overhead measurement. | Partial | `T-SRTP-W1`, `T-SRTP-U1`, `T-SRTP-W2` | DTLS-SRTP, MIKEY, complete replay/rollover suites, and independent-peer certification are not claimed. |

## Executable evidence catalog

Every `T-*` row below names the source file and exact executable test. None of
these named tests has an adjacent `#[ignore]` attribute as of this matrix.

| Evidence ID | Strength | Exact executable test | What it demonstrates |
|---|---|---|---|
| `T-3261-C1` | construction | `crates/sip/sip-core/tests/generated_message_compliance.rs::generated_message_compliance_request_method_matrix_roundtrips` | Generated SIP request-method matrix parses and round-trips. |
| `T-3261-W1` | wire | `crates/sip/rvoip-sip/tests/cancel_integration.rs::cancel_emits_exactly_one_callcancelled_event` | CANCEL traverses a live call flow with one cancellation outcome. |
| `T-3261-W2` | wire | `crates/sip/rvoip-sip/tests/fast_bye_cleanup.rs::fast_bye_200_keeps_hangup_successful_and_cleans_media_once` | Fast BYE/200 completion preserves success and one cleanup. |
| `T-3262-C1` | construction | `crates/sip/sip-dialog/tests/prack_test.rs::prack_for_dialog_builds_valid_request` | Dialog PRACK request construction. |
| `T-3262-W1` | wire | `crates/sip/rvoip-sip/tests/prack_integration.rs::prack_positive_reliable_183_flow` | Reliable `183` followed by PRACK on a live exchange. |
| `T-3262-W2` | wire | `crates/sip/rvoip-sip/tests/prack_integration.rs::prack_policy_mismatch_returns_420` | Unsupported reliable-provisional policy returns `420`. |
| `T-3263-U1` | unit | `crates/sip/sip-dialog/tests/rfc3263_resolution.rs::manager_uses_configured_resolver_for_invite_destination` | Dialog manager consumes the configured resolver for INVITE. |
| `T-3263-U2` | unit | `crates/sip/sip-dialog/tests/rfc3263_failover.rs::first_candidate_recoverable_failure_falls_over_to_second` | Recoverable first-candidate failure selects the second candidate. |
| `T-3263-W1` | wire | `crates/sip/sip-transport/tests/resolver_hickory_e2e.rs::hickory_client_resolves_naptr_then_srv_then_a` | Local DNS exchange performs NAPTR, SRV, then A resolution. |
| `T-3264-U1` | unit | `crates/sip/rvoip-sip/tests/sdp_matcher_integration.rs::intersection_in_offerer_order` | Codec intersection follows offerer order. |
| `T-3264-U2` | unit | `crates/sip/rvoip-sip/tests/sdp_matcher_integration.rs::direction_carried_through_per_line` | Media direction is carried per media line. |
| `T-3264-W1` | wire | `crates/sip/rvoip-sip/tests/sip_api_design_2_section_10_skeletons.rs::in_dialog_reinvite_smoke` | Established-dialog re-INVITE carries SDP and staged headers. |
| `T-3311-W1` | wire | `crates/sip/rvoip-sip/tests/sip_api_design_2_section_10_skeletons.rs::in_dialog_update_smoke` | Established-dialog UPDATE reaches the peer. |
| `T-3311-W2` | wire | `crates/sip/rvoip-sip/tests/update_notify_auth_retry.rs::update_401_retry_uses_authorization` | UPDATE retries a `401` with Authorization. |
| `T-3311-W3` | wire | `crates/sip/rvoip-sip/tests/update_notify_auth_retry.rs::update_407_retry_uses_proxy_authorization` | UPDATE retries a `407` with Proxy-Authorization. |
| `T-3325-W1` | wire | `crates/sip/rvoip-sip/tests/pai_integration.rs::config_pai_uri_surfaces_on_inbound_call` | Configured PAI reaches the inbound call. |
| `T-3325-W2` | wire | `crates/sip/rvoip-sip/tests/pai_integration.rs::per_call_pai_overrides_config` | Per-call PAI overrides configured PAI. |
| `T-3515-W1` | wire | `crates/sip/rvoip-sip/tests/outbound_request_builders_integration.rs::refer_builder_extras_reach_the_wire` | REFER builder fields reach the peer. |
| `T-3515-W2` | wire | `crates/sip/rvoip-sip/tests/blind_transfer_integration.rs::blind_transfer_end_to_end` | End-to-end blind transfer. |
| `T-3515-W3` | wire | `crates/sip/rvoip-sip/tests/adapter_refer_status_network.rs::refer_notify_progress_and_final_outcomes_are_typed_on_the_real_wire` | REFER NOTIFY progress/final outcomes traverse the wire. |
| `T-3581-U1` | unit | `crates/sip/sip-dialog/tests/rport_restamp_response.rs::response_via_gets_received_and_rport_when_inbound_via_had_rport_flag` | Top Via receives `received` and `rport`. |
| `T-3581-U2` | unit | `crates/sip/sip-dialog/tests/rport_restamp_response.rs::second_via_in_chain_is_not_modified` | Only the top Via is restamped. |
| `T-3891-C1` | construction | `crates/sip/sip-dialog/tests/refer_handling_test.rs::test_refer_with_replaces_header` | A REFER target can contain a Replaces parameter; no replacement executes. |
| `T-3891-U1` | unit | `crates/sip/sip-dialog/tests/refer_transfer_tests.rs::test_transfer_request_with_replaces` | A transfer request carries Replaces data; no replacement executes. |
| `T-4028-W1` | wire | `crates/sip/rvoip-sip/tests/session_timer_integration.rs::session_timer_refresh_emits_event` | A session refresh produces the success event. |
| `T-4028-W2` | wire | `crates/sip/rvoip-sip/tests/session_timer_failure_integration.rs::session_timer_refresh_failure_emits_event` | Refresh failure produces the failure event. |
| `T-4475-U1` | unit | `crates/sip/sip-core/tests/rfc_compliance/torture_test.rs::test_wellformed_messages` | Included well-formed fixtures parse. |
| `T-4475-U2` | unit | `crates/sip/sip-core/tests/rfc_compliance/torture_test.rs::test_malformed_messages` | Included malformed fixtures are rejected. |
| `T-5626-C1` | construction | `crates/sip/sip-dialog/tests/generated_sip_compliance.rs::generated_sip_compliance_dialog_client_builders_generate_valid_requests` | Outbound REGISTER Contact contains `ob`, `+sip.instance`, and `reg-id`. |
| `T-5626-U1` | unit | `crates/sip/rvoip-sip/tests/unified_api_tests.rs::rfc5626_registered_flow_requires_outbound_and_instance` | Registered-flow mode requires outbound and instance settings. |
| `T-5626-U2` | unit | `crates/sip/rvoip-sip/tests/unified_api_tests.rs::rfc5626_registered_flow_helper_sets_outbound_params` | Registered-flow helper sets and validates outbound parameters. |
| `T-6086-W1` | wire | `crates/sip/rvoip-sip/tests/outbound_request_builders_integration.rs::info_builder_extras_reach_the_wire` | INFO request fields reach the peer. |
| `T-6086-W2` | wire | `crates/sip/rvoip-sip/tests/info_auth_retry.rs::info_extras_survive_401_driven_auth_retry` | INFO fields survive `401` authentication retry. |
| `T-6086-W3` | wire | `crates/sip/rvoip-sip/tests/info_auth_retry.rs::info_407_retry_uses_proxy_authorization` | INFO retries `407` with Proxy-Authorization. |
| `T-6665-U1` | unit | `crates/sip/sip-dialog/tests/subscription_dialogs.rs::test_subscribe_creates_dialog` | SUBSCRIBE creates subscription/dialog state. |
| `T-6665-U2` | unit | `crates/sip/sip-dialog/tests/subscription_dialogs.rs::test_subscribe_with_zero_expires_terminates` | Zero-expiry SUBSCRIBE terminates subscription state. |
| `T-6665-U3` | unit | `crates/sip/sip-dialog/tests/subscription_dialogs.rs::test_notify_always_returns_200` | NOTIFY handling returns a success response in the tested dialog cases. |
| `T-6665-W1` | wire | `crates/sip/rvoip-sip/tests/sip_api_design_2_section_10_skeletons.rs::notify_subscription_id_routing` | NOTIFY Event `id` reaches the wire for subscription routing. |
| `T-6665-W2` | wire | `crates/sip/rvoip-sip/tests/oob_auth_retry.rs::subscribe_with_credentials_retries_with_full_digest` | SUBSCRIBE retries a digest challenge. |
| `T-7118-W1` | wire | `crates/sip/sip-transport/tests/ws_client_round_trip.rs::plain_ws_round_trip_delivers_register_to_server_event_bus` | Plain WebSocket transports REGISTER between client and server. |
| `T-AUTH-U1` | unit | `crates/identity/auth-core/src/sip_digest.rs::sha256_round_trip_with_authenticator` | SHA-256 digest round trip validates. |
| `T-AUTH-U2` | unit | `crates/identity/auth-core/src/sip_digest.rs::auth_int_includes_body_in_ha2` | `auth-int` includes the body in HA2. |
| `T-AUTH-U3` | unit | `crates/identity/auth-core/src/sip_digest.rs::nc_increments_across_calls_with_same_nonce` | Nonce count increments for a reused nonce. |
| `T-AUTH-W1` | wire | `crates/sip/rvoip-sip/tests/endpoint_unified_auth.rs::endpoint_uac_retries_digest_invite_against_unified_uas` | Endpoint INVITE retries a digest challenge. |
| `T-AUTH-W2` | wire | `crates/sip/rvoip-sip/tests/oob_auth_retry.rs::message_with_credentials_recovers_once_from_stale_nonce` | MESSAGE performs one stale-nonce recovery. |
| `T-AUTH-W3` | wire | `crates/sip/rvoip-sip/tests/oob_auth_retry.rs::message_with_credentials_uses_auth_int_when_offered_with_body` | MESSAGE selects and computes `auth-int` with a body. |
| `T-SDP-U1` | unit | `crates/sip/rvoip-sip/tests/sdp_matcher_integration.rs::rtpmap_carryover_filters_to_kept_formats` | Answer filtering keeps mappings only for negotiated payloads. |
| `T-SDP-U2` | unit | `crates/sip/rvoip-sip/tests/sdp_matcher_integration.rs::multi_m_line_offer_independently_matched` | Multiple media lines are matched independently. |
| `T-SDP-C1` | construction | `crates/sip/sip-dialog/tests/generated_sip_compliance.rs::generated_sip_compliance_dialog_client_builders_generate_valid_requests` | Generated INVITE validates with `application/sdp`. |
| `T-RTP-U1` | unit | `crates/media/rtp-core/src/packet/rtp.rs::test_serialize_parse_roundtrip` | RTP packet serialization/parser round trip. |
| `T-RTP-U2` | unit | `crates/media/rtp-core/src/packet/rtcp/receiver_report.rs::test_serialize_parse` | RTCP receiver-report serialization/parser round trip. |
| `T-RTP-W1` | wire | `crates/sip/rvoip-sip/tests/audio_roundtrip_integration.rs::audio_roundtrip_delivers_peer_tone` | Peer audio tone is delivered over a live call. |
| `T-RTP-W2` | wire | `crates/sip/rvoip-sip/tests/bridge_roundtrip_integration.rs::bridge_roundtrip_relays_tones_between_legs` | Bridge relays media tones between call legs. |
| `T-SRTP-W1` | wire | `crates/sip/rvoip-sip/tests/srtp_call_integration.rs::srtp_call_negotiates_and_establishes_end_to_end` | SDES-SRTP negotiation and end-to-end protected media. |
| `T-SRTP-U1` | unit | `crates/media/rtp-core/tests/malformed_input.rs::srtp_suite_with_oversized_tag_length_is_rejected` | Invalid SRTP authentication-tag size is rejected. |
| `T-SRTP-W2` | wire | `crates/sip/rvoip-sip/tests/perf/perf_srtp_overhead.rs::perf_srtp_overhead` | Executable SRTP/RTP overhead comparison on the media path. |

## July 20 historical run evidence

| Evidence ID | Strength | Bound artifact | Recorded result | Release use |
|---|---|---|---|---|
| `J20-I1` | interop | Manifest entry `J20-INTEROP-SUMMARY` | Asterisk, FreeSWITCH, SIPp, and baresip strict-UA gates recorded PASS with zero failures/skips. | Supplemental diagnostic evidence only; source was dirty. |
| `J20-S1` | unit | Manifest entry `J20-SECURITY-SUMMARY` | Dependency-audit and fuzz-smoke gates recorded PASS with zero failures/skips. | Supplemental diagnostic evidence only; source was dirty. |
| `J20-P1` | wire | Manifest entries `J20-PERF-SUMMARY` and `J20-MONOLITHIC-SOAK-LOG` | Pre-soak performance gates recorded PASS; monolithic soak recorded failure. | Negative gate evidence; cannot qualify a release. |

The mutable `target/perf-results/perf_soak_30min.json` snapshot is bound as
`J20-MUTABLE-PERF-JSON`, but it is **untrusted for the archived monolithic
soak** because no archived attestation binds that run to the same SHA-256. The
missing supplied local summary is recorded as `J20-LOCAL-SUMMARY` with
`available: false`.

## Explicit non-claims

| Standard | Beta status | Reason |
|---|---|---|
| RFC 5764 DTLS-SRTP | Unsupported | The retained beta SRTP profile is SDES; DTLS-SRTP is not established by executable release evidence. |
| RFC 8445 ICE | Unsupported | Attribute parsing/carry-through does not implement ICE connectivity checks. |
| RFC 8489 STUN | Unsupported as a compliance claim | A configured address-discovery helper is not a complete STUN or ICE behavior profile. |
| RFC 8656 TURN | Unsupported | No TURN allocation/relay behavior is claimed. |

Release notes and attestations must not broaden any claim beyond the exact
behavior and limit in its row. A future status upgrade requires a non-ignored
executable test at the appropriate strength and source-matched clean release
evidence.
