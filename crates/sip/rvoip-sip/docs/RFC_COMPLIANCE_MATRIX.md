# rvoip-sip Beta Standards Evidence Matrix

Date: 2026-10-12

This document maps each retained beta-profile claim to exact, non-ignored
executable test sources. It is a claim boundary, not a declaration that the
entire RFC is implemented and not a statement that every listed test passed on
the current source tree.

The earlier July 20 evidence remains preserved in the
[legacy archive](releases/beta/20260720T055257Z/baseline-evidence.json) and is
diagnostic-only. Current execution evidence is the clean, source-matched
`0.4.0` release at `f4d528eaabbac935bce9a6404d3ce4fdec35ed0f` (tag `v0.4.0`),
qualified by protected `remote-release` run
[`38195041080`](https://github.com/eisenzopf/rvoip/actions/runs/38195041080):
see the [protected qualification report](BETA_RELEASE_REPORT.md), exact
[214-gate ledger](BETA_GATE_REPORT.md), and
[machine summary](QUALIFICATION_SUMMARY.json). The ledger marks each gate
`fresh` or `reused`; 92 gates were reused from candidate
`d89c1bc34aadb7184e38792db5da1e91f5a1c8c1` because their definition, inputs,
and environment were unchanged.

Release gates run whole test targets, not individual `T-*` tests. Every
rvoip-sip `T-*` test below runs in the fresh `core.rvoip-sip` gate, which
builds with the `test-hooks` feature that the second-scale session-timer
targets need. In the `0.4.0` qualification `test.sip-integration` built
without it and compiled those targets out; after `0.4.0` the rvoip-sip unit,
integration, doctest, all-target, and example gates build with `test-hooks`
too, so from the next release the session-timer targets are also covered by
`test.sip-integration`. `scripts/test_release_gates.py` fails if a gate
selects a `test-hooks`-gated target without the feature. Tests in other
crates run in that crate's `core.<crate>` gate.

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
| SIP-3261-CORE | RFC 3261 | Core request construction, INVITE-dialog CANCEL and BYE completion/cleanup behavior, and (since `0.4.0`) the remote target as the Request-URI of every in-dialog request (§12.2.1.1), with the peer's AOR kept in `To`. | Partial | `T-3261-C1`, `T-3261-W1`, `T-3261-W2`, `T-3261-W3`, `T-3261-U1`, `J20-I1` | No section-by-section transaction, proxy, registrar, transport, or error-path certification is claimed. The in-dialog Request-URI rule has no independent-peer gate. |
| SIP-3262-100REL | RFC 3262 | PRACK construction, reliable `183`/PRACK exchange, and unsupported-policy rejection with `420`. | Partial | `T-3262-C1`, `T-3262-W1`, `T-3262-W2` | Forking, loss/retransmission matrices, and independent-PBX reliable-provisional evidence are not established here. |
| SIP-3263-LOCATION | RFC 3263 | Client NAPTR/SRV/A resolution and recoverable candidate failover for outbound requests. | Supported (bounded) | `T-3263-U1`, `T-3263-U2`, `T-3263-W1` | This is not a general DNS service claim and does not establish every RFC 3263 transport or failure permutation. |
| SIP-3264-OA | RFC 3264 | Audio offer/answer codec intersection, direction propagation, and an established-dialog re-INVITE carrying SDP on the wire. | Partial | `T-3264-U1`, `T-3264-U2`, `T-3264-W1` | Complex multi-stream renegotiation, all glare permutations, and WebRTC negotiation are outside this claim. |
| SIP-3311-UPDATE | RFC 3311 | In-dialog UPDATE transmission plus `401` and `407` digest retry on the same method. | Partial | `T-3311-W1`, `T-3311-W2`, `T-3311-W3` | No complete UPDATE offer/answer, glare, retry-after, or independent-peer matrix is claimed. Ignored resilience skeletons are not evidence. |
| SIP-3325-PAI | RFC 3325 | Configured and per-call `P-Asserted-Identity` reach the receiving endpoint, with per-call override behavior. | Partial | `T-3325-W1`, `T-3325-W2` | Trusted-domain policy, privacy interactions, and carrier certification are not claimed. |
| SIP-3515-REFER | RFC 3515 | Blind REFER request construction, end-to-end blind transfer, and ordered typed NOTIFY progress/final status on the wire. | Supported (bounded) | `T-3515-W1`, `T-3515-W2`, `T-3515-W3`, `J-3515-I1` | `Refer-To` is required. Attended transfer and RFC 3891 call replacement are excluded. `J-3515-I1` is protected `0.4.0` evidence (`interop.jambonz-matrix`, fresh PASS). |
| SIP-3581-RPORT | RFC 3581 | Top Via `received`/`rport` response restamping when the inbound request carries the `rport` flag. | Partial | `T-3581-U1`, `T-3581-U2` | No live NAT, multi-hop interoperability, keepalive, ICE, or TURN claim follows from these tests. |
| SIP-3891-REPLACES | RFC 3891 | Call replacement using `Replaces`. | Unsupported | `T-3891-C1`, `T-3891-U1` | The listed tests only construct/carry a Replaces parameter; they do not execute replacement semantics and therefore do not elevate the status. |
| SIP-3892-REFERRED-BY | RFC 3892 | Parse and expose a typed `Referred-By` header and, when supplied during a blind transfer, copy it unchanged into the referenced INVITE. | Partial | `T-3892-W1`, `J-3892-I1` | RFC 3892 makes `Referred-By` optional; RVoIP does not require it generally. The Jambonz propagation result is protected `0.4.0` evidence (`interop.jambonz-matrix`, fresh PASS). Identity signing, privacy behavior, and attended transfer are not claimed. |
| SIP-4028-TIMER | RFC 4028 | Successful session refresh and refresh-failure event delivery; since `0.4.0`, the 90-second Min-SE floor, `422` with `Min-SE` for a timer-capable caller below the floor, no `Require: timer` toward a caller without timer support (rvoip refreshes), the `refresher=uac` role on refreshes, a `422` retry with the peer's Min-SE, re-INVITE refresh when the peer rules out UPDATE or rejects it, a rejected refresh that keeps the call, and the non-refresher's 408 BYE before expiry. | Partial | `T-4028-W1`–`T-4028-W9`, `T-4028-U1`–`T-4028-U3` | Two rvoip endpoints only: no PBX, SBC, or proxy exercises session timers in a release gate (the `0.4.0` Asterisk/FreeSWITCH session-timer runs were a local diagnostic lab). Glare permutations beyond the tested `491` backoff and every proxy-inserted interval are not claimed. |
| SIP-4475-TORTURE | RFC 4475 | Fixture-driven acceptance of included well-formed messages and rejection of included malformed messages. | Supported with exclusions | `T-4475-U1`, `T-4475-U2` | Well-formed fixtures `3.1.1.2_intmeth.sip`, `4.10_ipv6-bug-abnf-3-colons.sip`, and `3.1.1.1_wsinv.sip` are excluded. The malformed exclusion list is empty. |
| SIP-5626-OUTBOUND | RFC 5626 | Outbound Contact construction plus registrar-side opaque exact-flow ownership, ordered multi-flow selection, replacement rollback, close degradation, and recoverable exact-flow failover. | Partial | `T-5626-C1`, `T-5626-U1`–`T-5626-U7` | Protected evidence from two independent UAs behind real NAT, including rebinding and restart recovery, is still required before this becomes a supported release claim. See `docs/sip/REMOTE_ENDPOINT_PROFILE.md`. |
| SIP-6086-INFO | RFC 6086 | Generic in-dialog INFO transmission and preservation across `401`/`407` authentication retry. | Partial | `T-6086-W1`, `T-6086-W2`, `T-6086-W3` | No Info-Package registry, `Recv-Info` negotiation, or package-specific standards profile is claimed. |
| SIP-6665-SUBSCRIBE | RFC 6665 | Subscription dialog creation/termination primitives, successful NOTIFY handling, subscription-id routing, and authenticated SUBSCRIBE retry. | Partial | `T-6665-U1`, `T-6665-U2`, `T-6665-U3`, `T-6665-W1`, `T-6665-W2` | Full notifier/subscriber state machines, refresh/expiry recovery, forked subscriptions, and independent-peer interoperability are not established. |
| SIP-7118-WS | RFC 7118 | Plain SIP-over-WebSocket client/server round trip delivering REGISTER. | Partial | `T-7118-W1` | WSS release evidence, browser/WebRTC behavior, reconnect, proxy traversal, and complete framing/error matrices are not claimed. |
| SIP-AUTH-DIGEST | RFC 3261 §22, RFC 7616, RFC 8760 | SHA-256 digest generation/validation, `auth-int`, nonce-count progression, stale-nonce retry, and endpoint INVITE digest retry. | Partial | `T-AUTH-U1`, `T-AUTH-U2`, `T-AUTH-U3`, `T-AUTH-W1`, `T-AUTH-W2`, `T-AUTH-W3` | The full algorithm matrix, every challenge-selection rule, independent-server certification, and every SIP method are not claimed. Basic and bearer extensions are not RFC 7616 compliance claims. |
| SDP-8866 | RFC 8866 | SDP audio offer parsing/matching, payload filtering, media direction propagation, and generated INVITE SDP validation. | Partial | `T-SDP-U1`, `T-SDP-U2`, `T-SDP-C1` | Full grammar coverage, every media type, bundle, trickle ICE, and WebRTC negotiation are not claimed. Attribute carry-through alone is not semantic support. |
| RTP-3550 | RFC 3550 | RTP packet serialization/parsing, RTCP receiver-report serialization/parsing, and bidirectional audio/bridge media delivery. | Partial | `T-RTP-U1`, `T-RTP-U2`, `T-RTP-W1`, `T-RTP-W2` | Full RTCP scheduling, feedback profiles, congestion behavior, multicast, and independent RTP-stack certification are not claimed. |
| RTP-5761-MUX | RFC 5761 | Since `0.4.0`: every SDP offer carries `a=rtcp-mux`; periodic RTCP and BYE reach a peer that answers with mux on the RTP port; a call whose answer declines mux sends no RTCP; the opt-out restores the plain offer; strict mode adds `a=rtcp-mux-only` and refuses separate-port peers; peer-reported quality flows over a muxed call. | Supported (bounded) | `T-5761-U1`, `T-5761-W1`–`T-5761-W4` | Two rvoip endpoints or a scripted socket peer only; no release gate checks RTCP against an independent stack (`0.4.0` Asterisk/FreeSWITCH RTCP runs were a local diagnostic lab). `a=rtcp-mux-only` interop is not claimed. |
| RTP-3605-NONMUX | RFC 3550 §11, RFC 3605 | Since `0.4.0`, with `Config::rtcp_non_mux`: RTCP on a separate local port toward a peer that declines mux, at the peer's RTP port + 1 or its `a=rtcp` address, SRTCP on that port under SDES-SRTP, and release of the RTCP port when the answer accepts mux. | Partial | `T-3605-W1`–`T-3605-W3`, `T-3605-U1` | Off by default. Not available with DTLS-SRTP, ICE, or strict mux. No independent-peer release gate. |
| SRTP-3711-SDES | RFC 3711 and RFC 4568 | SDES-negotiated SRTP call establishment, encrypted media exchange, malformed suite rejection, and executable overhead measurement. | Partial | `T-SRTP-W1`, `T-SRTP-U1`, `T-SRTP-W2` | DTLS-SRTP, MIKEY, complete replay/rollover suites, and independent-peer certification are not claimed. |

## Executable evidence catalog

Every `T-*` row below names the source file and exact executable test. None of
these named tests has an adjacent `#[ignore]` attribute as of this matrix.

| Evidence ID | Strength | Exact executable test | What it demonstrates |
|---|---|---|---|
| `T-3261-C1` | construction | `crates/sip/sip-core/tests/generated_message_compliance.rs::generated_message_compliance_request_method_matrix_roundtrips` | Generated SIP request-method matrix parses and round-trips. |
| `T-3261-W1` | wire | `crates/sip/rvoip-sip/tests/cancel_integration.rs::cancel_emits_exactly_one_callcancelled_event` | CANCEL traverses a live call flow with one cancellation outcome. |
| `T-3261-W2` | wire | `crates/sip/rvoip-sip/tests/fast_bye_cleanup.rs::fast_bye_200_keeps_hangup_successful_and_cleans_media_once` | Fast BYE/200 completion preserves success and one cleanup. |
| `T-3261-W3` | wire | `crates/sip/rvoip-sip/tests/in_dialog_request_target.rs::transfer_info_and_notify_target_the_peer_contact_not_its_from_address` | In-dialog REFER, INFO, and NOTIFY use the peer's Contact as Request-URI, not its From/To address. |
| `T-3261-U1` | unit | `crates/sip/sip-dialog/src/manager/core.rs::every_in_dialog_request_targets_the_remote_target_not_the_aor` | Every in-dialog request, and PRACK in an early dialog, targets the remote target; the AOR stays in `To`. |
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
| `T-3892-W1` | wire | `crates/sip/rvoip-sip/tests/header_inspection_integration.rs::inbound_invite_wire_carries_application_routing_headers` | An application-supplied `Referred-By` reaches the peer on the wire and remains available through inbound header inspection. |
| `T-4028-W1` | wire | `crates/sip/rvoip-sip/tests/session_timer_integration.rs::session_timer_refresh_emits_event` | A session refresh produces the success event. |
| `T-4028-W2` | wire | `crates/sip/rvoip-sip/tests/session_timer_failure_integration.rs::session_timer_refresh_failure_emits_event` | Refresh failure produces the failure event. |
| `T-4028-W3` | wire | `crates/sip/rvoip-sip/tests/session_timer_rfc4028_uas.rs::supporting_caller_below_the_floor_gets_422_with_min_se_90` | A timer-capable caller proposing less than the local Min-SE gets `422` with `Min-SE: 90`. |
| `T-4028-W4` | wire | `crates/sip/rvoip-sip/tests/session_timer_rfc4028_uas.rs::caller_without_timer_support_gets_no_require_timer_and_rvoip_refreshes` | A caller without timer support gets no `Require: timer`, and rvoip runs the refresh as `refresher=uas`. |
| `T-4028-W5` | wire | `crates/sip/rvoip-sip/tests/session_timer_rfc4028_uas.rs::non_refresher_sends_408_bye_before_the_session_expires` | The non-refresher sends its `408` BYE before the interval ends. |
| `T-4028-W6` | wire | `crates/sip/rvoip-sip/tests/session_timer_refresh_outcomes.rs::refresh_answered_422_is_retried_with_the_peers_min_se` | A refresh answered `422` is retried with the peer's `Min-SE`. |
| `T-4028-W7` | wire | `crates/sip/rvoip-sip/tests/session_timer_refresh_outcomes.rs::rejected_refresh_is_retried_once_and_a_2xx_rearms_the_timer` | A rejected refresh keeps the call, is retried once, and a 2xx re-arms the timer. |
| `T-4028-W8` | wire | `crates/sip/rvoip-sip/tests/session_timer_refresh_outcomes.rs::peer_without_update_in_allow_is_refreshed_with_reinvite_only` | A peer whose `Allow` omits UPDATE is refreshed with re-INVITE. |
| `T-4028-W9` | wire | `crates/sip/rvoip-sip/tests/session_timer_rfc4028_uac.rs::refresher_falls_back_to_reinvite_with_offer_when_peer_rejects_update` | A rejected refresh UPDATE falls back to a re-INVITE that re-offers the current SDP. |
| `T-4028-U1` | unit | `crates/sip/rvoip-sip/src/state_machine/actions.rs::refresh_names_the_sender_as_refresher_for_either_dialog_role` | Refreshes carry `refresher=uac` whichever side placed the call. |
| `T-4028-U2` | unit | `crates/sip/rvoip-sip/src/state_machine/actions.rs::non_refresher_expires_before_the_full_interval` | The non-refresher deadline is `interval - min(32, interval / 3)`. |
| `T-4028-U3` | unit | `crates/sip/rvoip-sip/tests/config_tests.rs::session_timer_values_below_the_rfc4028_floor_are_rejected` | Configuration below the 90-second floor is rejected. |
| `T-4475-U1` | unit | `crates/sip/sip-core/tests/rfc_compliance/torture_test.rs::test_wellformed_messages` | Included well-formed fixtures parse. |
| `T-4475-U2` | unit | `crates/sip/sip-core/tests/rfc_compliance/torture_test.rs::test_malformed_messages` | Included malformed fixtures are rejected. |
| `T-5626-C1` | construction | `crates/sip/sip-dialog/tests/generated_sip_compliance.rs::generated_sip_compliance_dialog_client_builders_generate_valid_requests` | Outbound REGISTER Contact contains `ob`, `+sip.instance`, and `reg-id`. |
| `T-5626-U1` | unit | `crates/sip/rvoip-sip/tests/unified_api_tests.rs::rfc5626_registered_flow_requires_outbound_and_instance` | Registered-flow mode requires outbound and instance settings. |
| `T-5626-U2` | unit | `crates/sip/rvoip-sip/tests/unified_api_tests.rs::rfc5626_registered_flow_helper_sets_outbound_params` | Registered-flow helper sets and validates outbound parameters. |
| `T-5626-U3` | unit | `crates/sip/sip-registrar/src/api/mod.rs::registered_flow_is_opaque_owned_live_and_fail_closed` | Opaque flow ownership rejects wrong AOR, unreachable, and removed bindings without exposing route identity. |
| `T-5626-U4` | unit | `crates/sip/sip-registrar/src/api/mod.rs::staged_replacement_preserves_old_flow_until_contact_commit` | Zero-wire replacement rollback retains the old route and a staged connection close cannot become reachable at commit. |
| `T-5626-U5` | unit | `crates/sip/rvoip-sip/src/server/contact_resolver.rs::registrar_resolution_returns_ordered_exact_flows_and_skips_degraded_primary` | Registrar lookup returns ordered exact routes, skips a degraded primary, and omits the token from serialization. |
| `T-5626-U6` | unit | `crates/sip/sip-dialog/tests/rfc3263_failover.rs::failed_primary_registered_flow_uses_secondary_exact_flow` | Recoverable primary failure advances to the secondary exact TLS flow without losing flow identity. |
| `T-5626-U7` | unit | `crates/rvoip/src/app.rs::remote_endpoint_profile_fails_closed_until_tls_srtp_identity_and_media_are_complete` | The facade refuses an incomplete remote-endpoint deployment posture. |
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
| `T-5761-U1` | unit | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs::every_offer_path_carries_rtcp_mux_by_default_without_a_fallback_rtcp_port` | The local offer in every media direction and the session-less hold/active fallback offers carry exactly one `a=rtcp-mux`, no `a=rtcp-mux-only`, and no `a=rtcp` fallback by default; the opt-out removes it. |
| `T-5761-W1` | wire | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs::default_offer_carries_rtcp_mux_and_periodic_rtcp_reaches_a_muxing_peer` | Periodic RTCP and BYE reach a muxing peer's RTP port; nothing goes to port + 1. |
| `T-5761-W2` | wire | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs::default_offer_sends_no_rtcp_anywhere_when_the_answer_declines_mux` | A call whose answer declines mux sends no RTCP and no BYE to either port. |
| `T-5761-W3` | wire | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs::required_rtcp_mux_negotiates_and_refuses_separate_port_peers` | Strict mux negotiates with a muxing peer and refuses a separate-port peer. |
| `T-5761-W4` | wire | `crates/sip/rvoip-sip/tests/media_quality_integration.rs::rtcp_mux_call_reports_local_and_peer_quality_and_measures_induced_loss` | A muxed call reports local and peer RTCP quality, including induced loss. |
| `T-3605-W1` | wire | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs::rtcp_non_mux_reaches_a_non_mux_peers_rtp_port_plus_one_both_ways` | With `rtcp_non_mux`, RTCP flows both ways between separate RTCP ports at RTP port + 1. |
| `T-3605-W2` | wire | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs::rtcp_non_mux_follows_the_answers_a_rtcp_port` | RTCP follows the answer's `a=rtcp` port. |
| `T-3605-W3` | wire | `crates/sip/rvoip-sip/src/adapters/media_adapter.rs::rtcp_non_mux_carries_srtcp_with_sdes_both_ways` | Separate-port RTCP is SRTCP-protected both ways under SDES-SRTP. |
| `T-3605-U1` | unit | `crates/media/rtp-core/src/session/mod.rs::non_mux_rtcp_uses_the_separate_socket_and_the_peers_rtp_port_plus_one` | rtp-core sends non-mux RTCP from the separate socket to the peer's RTP port + 1. |
| `T-SRTP-W1` | wire | `crates/sip/rvoip-sip/tests/srtp_call_integration.rs::srtp_call_negotiates_and_establishes_end_to_end` | SDES-SRTP negotiation and end-to-end protected media. |
| `T-SRTP-U1` | unit | `crates/media/rtp-core/tests/malformed_input.rs::srtp_suite_with_oversized_tag_length_is_rejected` | Invalid SRTP authentication-tag size is rejected. |
| `T-SRTP-W2` | wire | `crates/sip/rvoip-sip/tests/perf/perf_srtp_overhead.rs::perf_srtp_overhead` | Executable SRTP/RTP overhead comparison on the media path. |

## Protected external interoperability evidence

These gates execute against independently implemented peers. A local run is
diagnostic; only a protected exact-candidate artifact promotes the result into
published-release evidence. Both rows below are backed by the fresh
`interop.jambonz-matrix` PASS (63/63 cells, Jambonz OSS 0.9.14 confirmed as the
latest line by `interop.jambonz-latest`) in protected `0.4.0` run
[`38195041080`](https://github.com/eisenzopf/rvoip/actions/runs/38195041080).

| Evidence ID | Strength | Executable gate | What it demonstrates |
|---|---|---|---|
| `J-3515-I1` | interop | `examples/pbx/run.sh --pbx jambonz --api all --scenario blind_transfer --transport UDP` | Across all three public SIP APIs, Jambonz forwards REFER, RVoIP accepts it, the implicit subscription emits ordered `100`/progress/terminal NOTIFY requests, a referenced call is established, and both dialogs clean up. |
| `J-3892-I1` | interop | `examples/pbx/run.sh --pbx jambonz --api all --scenario blind_transfer --transport UDP` | A supplied typed `Referred-By` survives Jambonz forwarding and is copied unchanged into the referenced INVITE. |

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
| RFC 5763/5764 DTLS-SRTP | Supported, feature-gated (bounded) | `dtls_srtp_call_integration` proves two rvoip SIP endpoints install DTLS-derived contexts; `srtp_interop_webrtc_srtp` and protected `interop.remote-libsrtp` evidence prove bounded RTP/SRTCP compatibility with an independent implementation. |
| RFC 8445 ICE | Unsupported | Attribute parsing/carry-through does not implement ICE connectivity checks. |
| RFC 8489 STUN | Unsupported as a compliance claim | A configured address-discovery helper is not a complete STUN or ICE behavior profile. |
| RFC 8656 TURN | Unsupported | No TURN allocation/relay behavior is claimed. |

Release notes and attestations must not broaden any claim beyond the exact
behavior and limit in its row. A future status upgrade requires a non-ignored
executable test at the appropriate strength and source-matched clean release
evidence.
