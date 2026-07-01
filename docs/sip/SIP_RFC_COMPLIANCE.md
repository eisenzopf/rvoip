# SIP RFC Compliance Matrix

> Comprehensive catalogue of SIP and SIP-adjacent RFCs with rvoip's
> implementation status and the **test evidence** that attests to each claim.

- **Maintained for:** the `rvoip` SIP stack — `rvoip-sip`, `rvoip-sip-dialog`,
  `rvoip-sip-core`, `rvoip-sip-transport`, `rvoip-sip-proxy`,
  `rvoip-sip-registrar`.
- **Last reviewed:** 2026-06-18
- **Attestation basis:** beta-gate report
  [`20260616T014649Z`](../../crates/sip/rvoip-sip/beta-report/20260616T014649Z/summary.md)
  — clean revision `2bd8c570`, **all 44 gates PASS, 0 failures, 0 skips.**

This document is the **superset** reference. The crate-local
[`RFC_COMPLIANCE_MATRIX.md`](../../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md)
remains the authoritative **beta-claim** record; where the two differ, the crate
matrix governs what may be claimed in release notes.

---

## How to read the Compliance column

Every "✅ Verified" / "🟡 Partial" row is backed by a named, runnable test or by
an interop matrix that was green in the latest beta gate. The **Verified by**
column cites that evidence so the claim is reproducible, not aspirational.

| Badge | Meaning |
|-------|---------|
| ✅ **Verified** | Implemented and covered by an automated test and/or live interop matrix that is **green in the latest beta gate**. Documented limits may still apply. |
| 🟡 **Partial** | Common path implemented and exercised, but coverage, features, or edge cases are incomplete and the broader behaviour is **not claimed**. |
| 🔵 **Types only** | Header / SDP parsing and serialization present (carry-through); no higher-layer protocol behaviour wired into the state machine. |
| 🟠 **Planned / Post-beta** | Recognized and on the roadmap; **explicitly not claimed today** (often an intentional non-claim in the security/compatibility docs). |
| ⚪ **Not implemented** | No support in the SIP crates. Where a sibling crate (`rvoip-webrtc`, `rvoip-uctp`, `users-core`) owns it, that is noted. |
| 📕 **Historical** | Obsoleted by a newer RFC that we track instead; listed for completeness. |

### How the attestation is produced (reproduce it)

```sh
# Full beta gate (PBX interop, SIPp, baresip, perf, fuzz, torture) — the basis for this matrix
crates/sip/rvoip-sip/scripts/beta_gate.sh

# Generator-side RFC 3261 message validity (rvoip-sip public API)
cargo test -p rvoip-sip --features generated-validation --test generated_sip_compliance

# Dialog/transaction builders emit RFC-valid messages
cargo test -p rvoip-sip-dialog --features generated-validation --test generated_sip_compliance
cargo test -p rvoip-sip-dialog --test sip_compliance

# Parser independently accepts/rejects per RFC 4475 torture corpus + generated messages
cargo test -p rvoip-sip-core --features generated-validation --test generated_message_compliance
cargo test -p rvoip-sip-core --test rfc_compliance

# Per-RFC fault-injection / recovery behaviour
cargo test -p rvoip-sip --test '*' resilience
```

> **Note (validation hygiene):** feature-gated targets such as
> `generated_sip_compliance` are skipped by a bare `cargo test`. Always validate
> with the feature flags above (or `--all-features`) or the suite reports a
> false green.

---

## 1. Core SIP & transactions

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3261** | SIP: Session Initiation Protocol | The base protocol: requests/responses, transactions, dialogs, the offer/answer hook, REGISTER/INVITE/ACK/BYE/CANCEL/OPTIONS. | ✅ **Verified** (core; full section-by-section production audit not separately claimed) | `rvoip-sip` + `sip-dialog` `generated_sip_compliance.rs`, `sip-dialog/tests/sip_compliance.rs`, RFC 4475 torture, `resilience/rfc3261_transaction_recovery.rs`, Asterisk/FreeSWITCH/SIPp/baresip matrices |
| **2543** | SIP (original) | First SIP specification. | 📕 Historical — obsoleted by 3261 | n/a (tracked via 3261) |
| **6026** | Correct Transaction Handling for 2xx Responses to INVITE | Fixes INVITE server-transaction state for retransmitted 2xx / late ACK. | 🟡 **Partial** — INVITE transaction state machine implemented; not separately attested as a 6026 conformance run | `sip-dialog` transaction state machine, `resilience/rfc3261_transaction_recovery.rs` |
| **6141** | Re-INVITE and Target-Refresh Request Handling | Clarifies re-INVITE/UPDATE target refresh and glare. | 🟡 **Partial** — re-INVITE + glare handled | `glare_retry_integration.rs`, `sdp_matcher_integration.rs`, `adapter_renegotiate.rs` |
| **5057** | Multiple Dialog Usages in SIP | Guidance on multiple usages sharing a dialog. | 🟠 **Planned** | — |
| **5658** | Addressing Record-Route Issues in SIP | Double Record-Route for transport switches. | 🟡 **Partial** — Record-Route/route-set handled for common topologies | `sbc_topology_hiding_via_strip.rs`, proxy tests |
| **3263** | SIP: Locating SIP Servers | NAPTR/SRV/A resolution + transport selection and failover. | ✅ **Verified** | `sip-dialog/tests/rfc3263_resolution.rs`, `rfc3263_failover.rs`, `sip-transport/tests/resolver_hickory_e2e.rs`, `resilience/rfc3263_dns_failover_recovery.rs` |
| **3264** | An Offer/Answer Model with SDP | Negotiation of media via SDP offer/answer (hold/resume, glare). | ✅ **Verified** | `sdp_matcher_integration.rs`, `glare_retry_integration.rs`, PBX hold/resume rows in `matrix.tsv` |
| **4320** | Actions Addressing Non-INVITE Transaction Issues | Non-INVITE timer/response fixes. | 🟡 **Partial** — non-INVITE transaction timers implemented | `sip-dialog` transaction timer layer |

---

## 2. SIP method extensions

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3262** | Reliability of Provisional Responses (PRACK / 100rel) | Reliable 1xx with RSeq/RAck and PRACK. | ✅ **Verified** (broader PBX reliable-provisional interop is post-beta) | `prack_integration.rs`, `reliable_provisional_bridge.rs`, `early_media_tests.rs`, `sip-dialog/tests/prack_test.rs`, `resilience/rfc3262_reliable_provisional_recovery.rs` |
| **3311** | The SIP UPDATE Method | Update session parameters before final response. | ✅ **Verified** | `resilience/rfc3311_update_reinvite_recovery.rs`, `update_notify_auth_retry.rs`, `update_for_dialog` in `sip-dialog/tests/generated_sip_compliance.rs` |
| **3428** | SIP Extension for Instant Messaging (MESSAGE) | Pager-mode instant messages. | ✅ **Verified** (in-dialog + out-of-dialog builders emit valid MESSAGE) | `message_for_dialog` / `message_out_of_dialog` in `sip-dialog/tests/generated_sip_compliance.rs` |
| **3515** | The SIP REFER Method | Call transfer / reference to another resource. | ✅ **Verified** (attended-transfer orchestration is primitives-only) | `blind_transfer_integration.rs`, `transfer_notify_wiring_tests.rs`, `refer_auth_retry.rs`, `refer_for_dialog` generated-valid |
| **4488** | Suppression of REFER Implicit Subscription | `Refer-Sub: false` to suppress the implicit subscription. | 🔵 **Types only** | REFER header handling in `sip-core` |
| **6086** | SIP INFO Method and Package Framework | Mid-dialog application info (e.g. DTMF, signaling). | ✅ **Verified** (INFO package registry completeness not claimed) | `info_auth_retry.rs`, `info_for_dialog` generated-valid, DTMF/INFO PBX+SIPp evidence |
| **2976** | The SIP INFO Method | Original INFO method. | 📕 Historical — obsoleted by 6086 | n/a |
| **3903** | SIP Extension for Event State Publication (PUBLISH) | Publish event state (SIP-ETag / SIP-If-Match). | 🟡 **Partial** — ETag/If-Match types + presence body builders present | `sip-core` `sip_etag.rs`, `sip_if_match.rs`, `presence_builder_test.rs` |

---

## 3. Event framework & packages (SUBSCRIBE / NOTIFY)

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **6665** | SIP-Specific Event Notification | The SUBSCRIBE/NOTIFY framework, subscription state, multi-package dialogs. | ✅ **Verified** (package-registry completeness not claimed) | `notify_send_integration.rs`, `sip-dialog/tests/subscription_dialogs.rs`, `subscription_multi_id.rs`, `subscribe_out_of_dialog`/`notify_for_dialog` generated-valid |
| **3265** | SIP-Specific Event Notification | Original event framework. | 📕 Historical — obsoleted by 6665 (types remain in `sip-core`) | `sip-core/src/types/event.rs` |
| **4235** | An INVITE-Initiated Dialog Event Package | `dialog`/`dialog-info+xml` state for transfer & BLF. | 🟡 **Partial** — dialog-info+xml NOTIFY bodies generated; package wiring present | `api/dialog_package.rs`, dialog-info NOTIFY in `sip-dialog/tests/generated_sip_compliance.rs` |
| **3856** | A Presence Event Package for SIP | `presence` package (PIDF). | 🟡 **Partial** — presence body builders present | `presence_builder_test.rs` |
| **3857** | A Watcher Information Event Template-Package | `…​.winfo` watcher info. | ⚪ Not implemented | — |
| **3858** | XML Based Format for Watcher Information | `watcherinfo+xml`. | ⚪ Not implemented | — |
| **3680** | A SIP Event Package for Registrations | `reg` event package. | 🟠 **Planned** | — |
| **5263** | SIP Extension for Partial Notification of Presence | Partial PIDF deltas. | ⚪ Not implemented | — |

---

## 4. Registration, routing & connectivity

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3327** | Path Header (registering non-adjacent contacts) | `Path` insertion/echo for edge proxies. | 🟡 **Partial** — Path parsed, stored, and echoed | `server/contact_resolver.rs`, `api/send/register.rs`, `api/respond/register_response.rs` |
| **3608** | Service-Route Discovery During Registration | `Service-Route` returned in 2xx REGISTER and applied to subsequent requests. | 🟡 **Partial** | `sip-core/src/types/service_route.rs`, `api/respond/register_response.rs` |
| **5626** | Managing Client-Initiated Connections (Outbound) | `;ob`, `+sip.instance`, `reg-id`, flow keep-alive. | 🟡 **Partial** — single registered flow verified; **multi-flow not claimed** | outbound contact (`;ob`/`+sip.instance`/`reg-id`) generated-valid in `sip-dialog/tests/generated_sip_compliance.rs`, `resilience/rfc5626_outbound_flow_recovery.rs` |
| **5627** | Obtaining and Using GRUUs | Globally Routable UA URIs (temp/pub). | 🟡 **Partial** — instance-id/GRUU params handled in contacts | outbound contact params, registrar contact handling |
| **5628** | Registration Event Package for GRUU | `reg` package GRUU extension. | ⚪ Not implemented | — |
| **6140** | Registration for Multiple Phone Numbers (SIP trunking) | Bulk/wildcard registration for trunks. | 🟠 **Planned** | — |
| **3680** | SIP Event Package for Registrations | (see §3) | 🟠 **Planned** | — |
| **6223** | Indication of Support for Keep-Alive | STUN/CRLF keep-alive negotiation for outbound flows. | 🟡 **Partial** — CRLF keep-alive on registered flows | transport keep-alive, `resilience/rfc5626_outbound_flow_recovery.rs` |

---

## 5. NAT traversal & transport

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3581** | Symmetric Response Routing (`rport`) | Respond to the source IP/port observed on the request. | ✅ **Verified** | `sip-dialog/tests/rport_restamp_response.rs`, `resilience/rfc3581_rport_nat_recovery.rs`, PBX UDP/TLS rows |
| **7118** | WebSocket as a Transport for SIP | `ws`/`wss` SIP transport. | 🟡 **Partial** — `ws` client round-trip; browser/WebRTC + `wss` outbound post-beta | `sip-transport/tests/ws_client_round_trip.rs` |
| **5923** | Connection Reuse in SIP | Reuse a TLS/TCP connection in both directions (`alias`). | 🟡 **Partial** — connection reuse for TCP/TLS | `sip-transport` connection management |
| **5630** | The Use of the SIPS URI Scheme in SIP | SIPS routing & TLS hop semantics. | 🟡 **Partial** — SIPS/TLS hop handling | `tls_call_integration.rs` |
| **8489** | Session Traversal Utilities for NAT (STUN) | Server-reflexive address discovery at startup. | 🟡 **Partial** — `Config::stun_server` address discovery only; **not** ICE connectivity checks | `Config::stun_server` startup address-discovery |
| **8445** | Interactive Connectivity Establishment (ICE) | Full candidate gathering + connectivity checks. | 🟠 **Post-beta** — explicit non-claim | `SECURITY_POSTURE.md` / release docs non-claim |
| **8656** | Traversal Using Relays around NAT (TURN) | Media relay allocation. | 🟠 **Post-beta** — explicit non-claim | release docs non-claim |
| **8838** | Trickle ICE | Incremental candidate exchange. | 🔵 **Types only** (SDP candidate parsing) — owned by `rvoip-webrtc` | `rvoip-webrtc` WHIP/trickle tests |
| **8840** | SIP Usage for Trickle ICE | `Content-Disposition: ice` + half-trickle in SIP. | 🟠 **Post-beta** | — |
| **8839** | SDP Offer/Answer Procedures for ICE | `a=candidate`, `ice-ufrag`, `ice-pwd`, `ice-options`. | 🔵 **Types only** — typed candidate/ufrag/pwd parsing | `sip-core` SDP ICE attribute types |
| **5389** | STUN (original) | Original STUN. | 📕 Historical — obsoleted by 8489 | n/a |
| **5766** | TURN (original) | Original TURN. | 📕 Historical — obsoleted by 8656 | n/a |
| **5245** | ICE (original) | Original ICE. | 📕 Historical — obsoleted by 8445 | n/a |

---

## 6. Authentication & security

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **7616** | HTTP Digest Access Authentication | Digest with MD5 / SHA-256, `qop=auth`, nonce-count, stale recovery. | ✅ **Verified** (REGISTER & INVITE/PROXY challenge flows) | `generated_sip_compliance.rs` (401/407/stale/`nc` reset), `invite_auth_tests.rs`, `invite_repeated_challenge_auth.rs`, `api/respond/challenge.rs` |
| **2617** | HTTP Authentication: Basic and Digest | Original digest scheme. | 📕 Historical — superseded by 7616 (digest math shared) | tracked via 7616 |
| **8760** | SIP Digest Access Authentication (added algorithms) | SHA-512/256 and algorithm agility for SIP digest. | 🟡 **Partial** — MD5/SHA-256 path verified; SHA-512/256 not claimed | digest algorithm handling in `sip-core` auth types |
| **3329** | Security Mechanism Agreement for SIP | `Security-Client`/`Server`/`Verify` negotiation. | 🟠 **Planned** — requires path-wide proxy support; not claimed | — |
| **8898** | Third-Party Token-Based Authentication (OAuth) for SIP | Bearer/OAuth tokens in SIP auth. | 🟡 **Partial** — token-based auth integration via identity backends | `users-core` / `auth-core` token validators |
| **3323** | A Privacy Mechanism for SIP | `Privacy` header (id, header, user). | 🔵 **Types only** | `sip-core` Privacy header type |
| **3325** | P-Asserted-Identity / P-Preferred-Identity within Trusted Networks | `P-Asserted-Identity` and `P-Preferred-Identity` carry-through. | 🟡 **Partial** — PAI/PPI carry-through; trusted-network / carrier certification not claimed | `pai_integration.rs`, `third_party_register_integration.rs`, B2BUA carry-through |
| **3455** | P-Header Extensions for 3GPP | `P-Access-Network-Info`, `P-Visited-Network-ID`, etc. | 🔵 **Types only** | `sip-core` P-header types |

> TLS / SIPS transport security itself is exercised by `tls_call_integration.rs`
> and the PBX TLS matrix rows; see also RFC 5630 in §5.

---

## 7. Caller identity (STIR / SHAKEN)

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **8224** | Authenticated Identity Management in SIP | `Identity` header carrying a signed PASSporT; sign on egress, verify on ingress. | 🟡 **Partial** — sign/verify wired; carrier trust-anchor certification not claimed | `sip-dialog/tests/identity_sign_outbound.rs`, `identity_verify_inbound.rs`, `manager/identity_verify.rs` |
| **8225** | PASSporT: Personal Assertion Token | The signed JWT (header/payload/signature) conveyed by RFC 8224. | 🟡 **Partial** — PASSporT construction/parse | `sip-core/src/types/identity.rs` |
| **8226** | Secure Telephone Identity Credentials: Certificates | X.509 certs / `x5u` for STIR. | 🟡 **Partial** — cert reference handling | identity cert handling in `sip-dialog` |
| **8588** | PASSporT Extension for SHAKEN | `ppt=shaken`, attestation level, origid. | 🟡 **Partial** — SHAKEN claim shape supported | `sip-core` identity types |
| **4474** | Enhancements for Authenticated Identity Management | Original SIP Identity (`Identity`/`Identity-Info`). | 📕 Historical — obsoleted by 8224 | tracked via 8224 |
| **8946** | PASSporT `div` extension (Diversion) | Diversion claims in PASSporT. | ⚪ Not implemented | — |

---

## 8. SDP & offer/answer details

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **8866** | SDP: Session Description Protocol | The SDP grammar (parse + build). | ✅ **Verified** (WebRTC-specific attrs are carry-through unless wired higher) | `sip-core` SDP parser/builder tests, `generated_message_compliance.rs`, SDP fuzz target |
| **4566** | SDP (previous) | Prior SDP edition. | 📕 Historical — obsoleted by 8866 | tracked via 8866 |
| **3264** | Offer/Answer Model | (see §1) | ✅ **Verified** | `sdp_matcher_integration.rs`, glare tests |
| **4568** | SDP Security Descriptions (SDES) for SRTP | `a=crypto` SRTP keying in SDP. | 🟡 **Partial** — SDES negotiation; DTLS-SRTP excluded | `srtp_call_integration.rs`, `adapters/srtp_negotiator.rs` |
| **5763** | Framework for SRTP context via DTLS | DTLS-SRTP framework. | 🟠 **Post-beta** — explicit non-claim | `SECURITY_POSTURE.md` non-claim |
| **5764** | DTLS Extension to Establish Keys for SRTP | DTLS-SRTP (`a=fingerprint`, `setup`). | 🟠 **Post-beta** — explicit non-claim | `SECURITY_POSTURE.md` / `COMPATIBILITY_MATRIX.md` non-claim |
| **8842** | SDP Offer/Answer for DTLS-SRTP | DTLS role / fingerprint negotiation. | 🟠 **Post-beta** | — |
| **5888** | The SDP Grouping Framework | `a=group` (basis for BUNDLE). | 🔵 **Types only** | `sip-core` SDP group attribute |
| **8843** | Negotiating Media Multiplexing (BUNDLE) | `a=group:BUNDLE`. | ⚪ Not implemented (owned by `rvoip-webrtc`) | `rvoip-webrtc` |
| **5761** | Multiplexing RTP and RTCP on One Port | `a=rtcp-mux`. | 🔵 **Types only** | `sip-core` SDP attribute |
| **5576** | Source-Specific Media Attributes in SDP | `a=ssrc`. | 🔵 **Types only** | `sip-core` SDP attribute |
| **3556** | SDP Bandwidth Modifiers for RTCP | `b=RR:`/`b=RS:`. | 🔵 **Types only** | `sip-core` SDP bandwidth parsing |
| **3605** | RTCP Attribute in SDP | `a=rtcp:`. | 🔵 **Types only** | `sip-core` SDP attribute |
| **4145** | TCP-Based Media Transport in SDP | `a=setup`/`a=connection` (COMEDIA). | 🔵 **Types only** | `sip-core` SDP attribute |
| **4572** | Connection-Oriented Media over TLS in SDP | `a=fingerprint` for TLS media. | 🔵 **Types only** | `sip-core` SDP fingerprint parsing |

---

## 9. RTP / RTCP & media transport

> Media transport is implemented in the `rvoip` media crates and exercised end
> to end through `rvoip-sip` calls; cited tests run real RTP.

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3550** | RTP: A Transport Protocol for Real-Time Applications | Core RTP/RTCP. | ✅ **Verified** (full RTCP feedback behaviour not claimed) | `audio_roundtrip_integration.rs`, `bridge_roundtrip_integration.rs`, `perf_rtp_steady_state` |
| **3551** | RTP Profile for Audio and Video Conferences (AVP) | Static payload types, PCMU/PCMA. | ✅ **Verified** | audio round-trip + PBX G.711/G.729 matrix rows |
| **3711** | The Secure Real-time Transport Protocol (SRTP) | SRTP/SRTCP encryption + auth. | 🟡 **Partial** — SDES-keyed SRTP; DTLS-SRTP excluded | `srtp_call_integration.rs`, SRTP negotiator tests, PBX SRTP rows |
| **4733** | RTP Payload for DTMF / Telephony Tones (telephone-event) | RFC 2833-style out-of-band DTMF. | ✅ **Verified** | DTMF/INFO tests, SIPp + Asterisk/FreeSWITCH DTMF matrix evidence, `media_stream.rs`/`state_machine/actions.rs` |
| **2833** | RTP Payload for DTMF (original) | Predecessor of 4733. | 📕 Historical — obsoleted by 4733 | tracked via 4733 |
| **3389** | RTP Payload for Comfort Noise | CN payload for silence suppression. | 🔵 **Types only** — CN payload recognized | media payload handling |
| **4585** | Extended RTP Profile for RTCP Feedback (AVPF) | NACK/PLI/FIR feedback. | ⚪ Not implemented (WebRTC path in `rvoip-webrtc`) | — |
| **5104** | Codec Control Messages in AVPF | FIR/TMMBR codec control. | ⚪ Not implemented | — |
| **8285** | A General Mechanism for RTP Header Extensions | One/two-byte header extensions. | 🔵 **Types only** | media RTP header-extension parsing |
| **6464** | Client-to-Mixer Audio Level Indication | `a=extmap` audio-level (ssrc-audio-level). | 🔵 **Types only** | SDP `extmap` parsing |
| **6465** | Mixer-to-Client Audio Level Indication | Mixer-side level extension. | ⚪ Not implemented | — |
| **3611** | RTCP Extended Reports (RTCP XR) | Quality metrics reporting blocks. | ⚪ Not implemented | — |
| **5506** | Support for Reduced-Size RTCP | Compound-RTCP relaxation. | ⚪ Not implemented | — |
| **5761** | RTP/RTCP Multiplexing | (see §8) | 🔵 **Types only** | `sip-core` SDP |

---

## 10. Session policy & call-control headers

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **4028** | Session Timers in SIP | `Session-Expires`/`Min-SE`, refresher, 422 recovery. | ✅ **Verified** (full edge-case production audit remains open) | `session_timer_integration.rs`, `session_timer_failure_integration.rs`, `session_422_retry.rs`, `resilience/rfc4028_session_timer_recovery.rs`, 422 retry generated-valid |
| **3326** | The Reason Header Field for SIP | `Reason:` on BYE/CANCEL and responses. | 🟡 **Partial** — Reason emitted on teardown paths | teardown/reason handling, `teardown_rfc_state_table_tests.rs` |
| **3891** | The SIP "Replaces" Header | Replace an existing dialog (attended transfer / pickup). | 🟡 **Partial** — Replaces consumed on transfer | `server/transfer.rs`, `adapters/dialog_adapter.rs` |
| **3892** | The SIP Referred-By Mechanism | `Referred-By` on REFER-initiated requests. | 🟡 **Partial** — Referred-By emitted/propagated on REFER | `api/send/refer.rs`, `adapters/dialog_adapter.rs` |
| **4538** | Target-Dialog (`Target-Dialog` header) | Authorize a request by referencing a known dialog. | 🔵 **Types only** | `sip-core` header type |
| **4916** | Connected Identity in SIP | `P-…`/connected-line update mid-dialog. | ⚪ Not implemented | — |
| **4244** | Request History Information (History-Info) | `History-Info` retargeting trail. | ⚪ Not implemented | — |
| **7044** | An Extension to SIP for Request History Information | Updated History-Info. | ⚪ Not implemented | — |
| **5806** | Diversion Indication in SIP | `Diversion` header (legacy). | 🔵 **Types only** | `sip-core` header type |
| **3840** | Indicating UA Capabilities in SIP | `+sip.*` feature tags on Contact. | 🔵 **Types only** — feature-tag params on contacts | contact param handling |
| **3841** | Caller Preferences for SIP | `Accept-Contact`/`Reject-Contact`/`Request-Disposition`. | 🔵 **Types only** | `sip-core` header types |
| **3327 / 3608** | Path / Service-Route | (see §4) | 🟡 **Partial** | §4 |

---

## 11. URIs, message bodies & encoding

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3261 URIs** | SIP / SIPS URI scheme | `sip:`/`sips:` URI parse + build with params/headers. | ✅ **Verified** | `sip-core` URI parser tests + URI fuzz target |
| **3986** | URI: Generic Syntax | Generic RFC 3986 URI conformance. | ✅ **Verified** | `sip-core` URI parsing + fuzz |
| **3966** | The `tel` URI for Telephone Numbers | `tel:` URIs and `phone-context`. | 🟡 **Partial** — tel URI parse/build | `sip-core` URI types |
| **2045** | MIME Part 1: Format of Message Bodies | MIME headers / encodings for SIP bodies. | 🔵 **Types only** | `sip-core` content-type / MIME parsing |
| **2046** | MIME Part 2: Media Types | `multipart/*`, media-type registry. | 🟡 **Partial** — multipart bodies parsed (torture `mpart01`) | `sip-core` multipart parsing, `rfc_compliance/wellformed/3.1.1.11_mpart01.sip` |
| **5621** | Message Body Handling in SIP | Multipart body handling rules for SIP. | 🟡 **Partial** — multipart carry-through | `sip-core` body handling |
| **5646** | Tags for Identifying Languages (BCP 47) | `Content-Language`/`Accept-Language` tag validation. | 🟡 **Partial** — language-tag parse/validate (incl. grandfathered tags) | `sip-core` language-tag parser tests |
| **4475** | SIP Torture Test Messages | Pathological-message corpus a parser must accept/reject correctly. | ✅ **Verified** (documented exclusions retained in fixture) | `sip-core/tests/rfc_compliance/torture_test.rs` + `malformed/`+`wellformed/` corpus |
| **5118** | SIP Torture Test Messages for IPv6 | IPv6-specific torture cases. | 🟡 **Partial** — IPv6 cases in corpus | `rfc_compliance` corpus (`4.2_ipv6-bad.sip`, …) |

---

## 12. Codecs (payload formats)

> Codec payloads live in the media crates / `rvoip-webrtc`; listed here because
> they are negotiated through SIP/SDP.

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **G.711** (ITU) | PCMU / PCMA | μ-law / A-law audio (static PT 0/8). | ✅ **Verified** | audio round-trip + PBX matrix |
| **G.729** (ITU) | G.729 / G.729A/AB | Low-bitrate audio. | ✅ **Verified** | PBX `g729a g729ab` matrix profiles |
| **6716** | Definition of the Opus Audio Codec | Opus codec. | ⚪ Out of SIP scope — media/webrtc crates | `rvoip-webrtc` |
| **7587** | RTP Payload Format for Opus | `a=rtpmap:… opus`. | 🔵 **Types only** (SDP) — media in webrtc crate | SDP rtpmap parsing |
| **6184** | RTP Payload Format for H.264 | Video payload. | ⚪ Out of SIP scope — media/webrtc crates | `rvoip-webrtc` |

---

## 13. Adjacent specs (handled by sibling crates, not the SIP signaling stack)

These appear in the workspace but are **not** SIP signaling RFCs. They are
listed so the SIP picture is complete and nobody double-counts them against the
SIP crates.

| RFC | Title | Owner | Status |
|-----|-------|-------|--------|
| **8831 / 8832 / 8864** | WebRTC Data Channels (SCTP / DCEP / SDP) | `rvoip-webrtc` | Tracked in webrtc crate |
| **8830** | WebRTC MediaStream Identification (MSID) in SDP | `rvoip-webrtc` | Tracked in webrtc crate |
| **8843 / 8851 / 8852 / 8853** | BUNDLE / payload restrictions / RID-SDES / Simulcast | `rvoip-webrtc` | Tracked in webrtc crate |
| **9725** | WHIP — WebRTC-HTTP Ingestion Protocol | `rvoip-webrtc` | `whip_compliance.rs` (webrtc crate) |
| **9421** | HTTP Message Signatures | `rvoip-uctp` / `auth-core` | UCTP inline-envelope signing |
| **9449** | OAuth 2.0 Demonstrating Proof of Possession (DPoP) | `users-core` / `auth-core` | Identity backend |
| **7638** | JSON Web Key (JWK) Thumbprint | `users-core` / `auth-core` | Identity backend |
| **8785** | JSON Canonicalization Scheme (JCS) | `rvoip-uctp` / `auth-core` | Envelope canonicalization |

---

## Roadmap rollup

Distilled from the statuses above — the natural candidates for the next
milestones, grouped by how far they are from "done".

**🟡 Partial → finish & promote to Verified**
- RFC 5626 Outbound: multi-flow registration (single-flow is verified today).
- RFC 8224/8225/8226/8588 STIR/SHAKEN: carrier trust-anchor + attestation
  certification (sign/verify already wired).
- RFC 3327 / 3608 Path / Service-Route: dedicated conformance tests.
- RFC 3325 PAI/PPI: trusted-network policy enforcement.
- RFC 8760: SHA-512/256 digest algorithm agility.
- RFC 3711 / 4568 SRTP/SDES: broaden interop coverage.

**🔵 Types only → wire behaviour**
- RFC 3903 PUBLISH state machine (ETag/If-Match types exist).
- RFC 4235 / 3856 event-package publication & subscription completeness.
- RFC 3323 Privacy enforcement; RFC 3891 Replaces / RFC 3892 Referred-By
  end-to-end attended transfer.

**🟠 Planned / Post-beta (explicit non-claims today)**
- RFC 8445 ICE, RFC 8656 TURN, RFC 5763/5764/8842 DTLS-SRTP (security non-claims).
- RFC 3329 Security Mechanism Agreement.
- RFC 6140 bulk/trunk registration; RFC 3680 reg-event package.

**⚪ Not implemented (no current support)**
- RFC 4585/5104 RTCP feedback (AVPF), RFC 3611 RTCP XR, RFC 5506 reduced RTCP.
- RFC 4244/7044 History-Info, RFC 4916 Connected Identity.
- RFC 3857/3858 watcher info.

---

## Maintenance

- **When to update:** whenever a compliance test is added/renamed, a new RFC is
  implemented, or a new beta-gate report supersedes the attestation basis above.
- **Keep claims honest:** a row may only be marked ✅ **Verified** if it is backed
  by a named test or interop matrix that is green in the *current* beta gate.
  Promote from 🟡/🔵 only when that evidence exists.
- **Source of beta claims:** the crate-local
  [`RFC_COMPLIANCE_MATRIX.md`](../../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md)
  governs what release notes may claim; this file is the broader engineering &
  roadmap view.
- **Regenerate the attestation:** run `crates/sip/rvoip-sip/scripts/beta_gate.sh`
  (see commands at the top), then update the *Last reviewed* date and the report
  timestamp/revision in the header.
