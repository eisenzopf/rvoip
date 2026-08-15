# SIP RFC Compliance Matrix

> Comprehensive catalogue of SIP and SIP-adjacent RFCs with rvoip's bounded
> implementation status, explicit limits, and the evidence basis for each claim.

- **Maintained for:** the `rvoip` SIP stack — `rvoip-sip`, `rvoip-sip-dialog`,
  `rvoip-sip-core`, `rvoip-sip-transport`, `rvoip-sip-proxy`,
  `rvoip-sip-registrar`.
- **Last reviewed:** 2026-07-21
- **Evidence basis:** the exact, non-ignored source inventory in the crate-local
  [`RFC_COMPLIANCE_MATRIX.md`](../../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md).
  The archived July 20 run is diagnostic because it came from a dirty source
  tree and its monolithic soak failed. A clean, current-source full beta
  attestation is still required before any release-candidate claim.

This document is the **superset** reference. The crate-local
[`RFC_COMPLIANCE_MATRIX.md`](../../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md)
remains the authoritative **beta-claim** record; where the two differ, the crate
matrix governs what may be claimed in release notes.

---

## How to read the Compliance column

Every "✅ Verified" row states a deliberately bounded behavior and cites one or
more `T-*` IDs from the crate-local
[`Executable evidence catalog`](../../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md#executable-evidence-catalog).
Those IDs resolve to exact, non-ignored executable tests. A partial row may
also cite construction evidence or a code location to describe its limit, but
that does not promote the broader RFC behavior to verified status.

| Badge | Meaning |
|-------|---------|
| ✅ **Verified** | The exact bounded behavior stated in the row has direct, non-ignored executable evidence. This is not whole-RFC certification and does not replace a clean, source-matched release attestation. |
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

# Static claim-to-evidence validation (also rejects ignored/stub-only evidence)
cargo test -p rvoip-sip --test beta_release_docs
```

> **Note (validation hygiene):** feature-gated targets such as
> `generated_sip_compliance` are skipped by a bare `cargo test`. Always validate
> with the feature flags above (or `--all-features`) or the suite reports a
> false green. Files under `tests/resilience/` that are marked `#[ignore]` as
> stubs are roadmap notes, not compliance evidence.

---

## 1. Core SIP & transactions

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3261** | SIP: Session Initiation Protocol | Core request construction plus INVITE-dialog CANCEL and BYE completion/cleanup behavior. | 🟡 **Partial** — no section-by-section transaction, proxy, registrar, transport, or error-path certification | `T-3261-C1`, `T-3261-W1`, `T-3261-W2`; ignored resilience stubs are excluded |
| **2543** | SIP (original) | First SIP specification. | 📕 Historical — obsoleted by 3261 | n/a (tracked via 3261) |
| **6026** | Correct Transaction Handling for 2xx Responses to INVITE | Fixes INVITE server-transaction state for retransmitted 2xx / late ACK. | 🟡 **Partial** — implementation is present, but no dedicated non-ignored RFC 6026 conformance test is claimed | `sip-dialog` transaction state machine inspection only; ignored resilience stubs are excluded |
| **6141** | Re-INVITE and Target-Refresh Request Handling | Clarifies re-INVITE/UPDATE target refresh and glare. | 🟡 **Partial** — re-INVITE + glare handled | `glare_retry_integration.rs`, `sdp_matcher_integration.rs`, `adapter_renegotiate.rs` |
| **5057** | Multiple Dialog Usages in SIP | Guidance on multiple usages sharing a dialog. | 🟠 **Planned** | — |
| **5658** | Addressing Record-Route Issues in SIP | Double Record-Route for transport switches. | 🟡 **Partial** — Record-Route/route-set handled for common topologies | `sbc_topology_hiding_via_strip.rs`, proxy tests |
| **3263** | SIP: Locating SIP Servers | Configured NAPTR/SRV/A resolution and recoverable first-candidate failover for outbound requests. | ✅ **Verified** (bounded client-resolution behavior; not every transport/failure permutation) | `T-3263-U1`, `T-3263-U2`, `T-3263-W1` |
| **3264** | An Offer/Answer Model with SDP | Audio codec intersection, media-direction propagation, and an established-dialog re-INVITE carrying SDP. | 🟡 **Partial** — complex multi-stream negotiation, all glare permutations, and WebRTC negotiation are not claimed | `T-3264-U1`, `T-3264-U2`, `T-3264-W1` |
| **4320** | Actions Addressing Non-INVITE Transaction Issues | Non-INVITE timer/response fixes. | 🟡 **Partial** — non-INVITE transaction timers implemented | `sip-dialog` transaction timer layer |

---

## 2. SIP method extensions

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3262** | Reliability of Provisional Responses (PRACK / 100rel) | PRACK construction, a reliable `183`/PRACK exchange, and unsupported-policy rejection with `420`. | 🟡 **Partial** — forking, loss/retransmission matrices, and independent-PBX reliable-provisional evidence are not established | `T-3262-C1`, `T-3262-W1`, `T-3262-W2`; ignored resilience stubs are excluded |
| **3311** | The SIP UPDATE Method | In-dialog UPDATE transmission plus `401` and `407` digest retry on the same method. | 🟡 **Partial** — no complete UPDATE offer/answer, glare, Retry-After, or independent-peer matrix | `T-3311-W1`, `T-3311-W2`, `T-3311-W3`; ignored resilience stubs are excluded |
| **3428** | SIP Extension for Instant Messaging (MESSAGE) | Pager-mode MESSAGE request construction for in-dialog and out-of-dialog use. | 🟡 **Partial** — construction/auth flows do not establish a complete messaging interoperability profile | `sip-dialog/tests/generated_sip_compliance.rs`, `T-AUTH-W2`, `T-AUTH-W3` |
| **3515** | The SIP REFER Method | Blind REFER construction, end-to-end blind transfer, and typed NOTIFY progress/final status on the wire. | ✅ **Verified** (bounded blind-transfer behavior; attended transfer and RFC 3891 replacement are excluded) | `T-3515-W1`, `T-3515-W2`, `T-3515-W3` |
| **4488** | Suppression of REFER Implicit Subscription | `Refer-Sub: false` to suppress the implicit subscription. | 🔵 **Types only** | REFER header handling in `sip-core` |
| **6086** | SIP INFO Method and Package Framework | Generic in-dialog INFO transmission and preservation across `401`/`407` authentication retry. | 🟡 **Partial** — no Info-Package registry, `Recv-Info` negotiation, or package-specific standards profile | `T-6086-W1`, `T-6086-W2`, `T-6086-W3` |
| **2976** | The SIP INFO Method | Original INFO method. | 📕 Historical — obsoleted by 6086 | n/a |
| **3903** | SIP Extension for Event State Publication (PUBLISH) | Publish event state (SIP-ETag / SIP-If-Match). | 🔵 **Types only** — ETag/If-Match and presence-body construction do not implement PUBLISH lifecycle behavior | `sip-core` `sip_etag.rs`, `sip_if_match.rs`, `presence_builder_test.rs` |

---

## 3. Event framework & packages (SUBSCRIBE / NOTIFY)

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **6665** | SIP-Specific Event Notification | Subscription dialog creation/termination primitives, successful NOTIFY handling, subscription-id routing, and authenticated SUBSCRIBE retry. | 🟡 **Partial** — full notifier/subscriber state machines, refresh/expiry recovery, forks, and independent-peer interop are not established | `T-6665-U1`, `T-6665-U2`, `T-6665-U3`, `T-6665-W1`, `T-6665-W2` |
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
| **5626** | Managing Client-Initiated Connections (Outbound) | Outbound Contact construction with `ob`, `+sip.instance`, and `reg-id`, plus registered-flow configuration validation. | 🟡 **Partial** — flow tokens, multi-flow behavior, keepalive/recovery, failover, and registrar-side behavior are not claimed | `T-5626-C1`, `T-5626-U1`, `T-5626-U2`; ignored flow-recovery stubs are excluded |
| **5627** | Obtaining and Using GRUUs | Globally Routable UA URIs (temp/pub). | 🟡 **Partial** — instance-id/GRUU params handled in contacts | outbound contact params, registrar contact handling |
| **5628** | Registration Event Package for GRUU | `reg` package GRUU extension. | ⚪ Not implemented | — |
| **6140** | Registration for Multiple Phone Numbers (SIP trunking) | Bulk/wildcard registration for trunks. | 🟠 **Planned** | — |
| **3680** | SIP Event Package for Registrations | (see §3) | 🟠 **Planned** | — |
| **6223** | Indication of Support for Keep-Alive | CRLF keep-alive framing on supported stream transports. | 🟡 **Partial** — STUN negotiation and end-to-end registered-flow recovery are not claimed | `sip-transport` TCP/TLS keep-alive frame tests; ignored RFC 5626 recovery stubs are excluded |

---

## 5. NAT traversal & transport

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **3581** | Symmetric Response Routing (`rport`) | Restamp the top Via with `received` and `rport` when the inbound request carries the `rport` flag. | 🟡 **Partial** — no live-NAT, multi-hop, keepalive, ICE, or TURN claim follows | `T-3581-U1`, `T-3581-U2`; the ignored NAT-pinhole resilience stub is excluded |
| **7118** | WebSocket as a Transport for SIP | `ws`/`wss` SIP transport. | 🟡 **Partial** — `ws` client round-trip; browser/WebRTC + `wss` outbound post-beta | `sip-transport/tests/ws_client_round_trip.rs` |
| **5923** | Connection Reuse in SIP | Reuse a TLS/TCP connection in both directions (`alias`). | 🟡 **Partial** — connection reuse for TCP/TLS | `sip-transport` connection management |
| **5630** | The Use of the SIPS URI Scheme in SIP | SIPS routing & TLS hop semantics. | 🟡 **Partial** — SIPS/TLS hop handling | `tls_call_integration.rs` |
| **8489** | Session Traversal Utilities for NAT (STUN) | Server-reflexive address discovery. | 🟠 **Post-beta** as a compliance claim — a configured startup helper does not establish the RFC 8489 behavior profile or ICE connectivity checks | `Config::stun_server` is configuration/implementation presence, not conformance evidence |
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
| **7616** | HTTP Digest Access Authentication | SHA-256 digest generation/validation, `auth-int`, nonce-count progression, stale-nonce recovery, and endpoint INVITE digest retry. | 🟡 **Partial** — the full algorithm/method/challenge-selection matrix and independent-server certification are not claimed | `T-AUTH-U1`, `T-AUTH-U2`, `T-AUTH-U3`, `T-AUTH-W1`, `T-AUTH-W2`, `T-AUTH-W3` |
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
| **8866** | SDP: Session Description Protocol | SDP audio-offer parsing/matching, payload filtering, media-direction propagation, and generated INVITE SDP validation. | 🟡 **Partial** — full grammar/media coverage, BUNDLE, trickle ICE, and WebRTC negotiation are not claimed | `T-SDP-U1`, `T-SDP-U2`, `T-SDP-C1` |
| **4566** | SDP (previous) | Prior SDP edition. | 📕 Historical — obsoleted by 8866 | tracked via 8866 |
| **3264** | Offer/Answer Model | (see §1) | 🟡 **Partial** | `T-3264-U1`, `T-3264-U2`, `T-3264-W1` |
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
| **3550** | RTP: A Transport Protocol for Real-Time Applications | RTP packet and RTCP receiver-report round trips plus bidirectional audio/bridge delivery. | 🟡 **Partial** — full RTCP scheduling/feedback, congestion, multicast, and independent-stack certification are not claimed | `T-RTP-U1`, `T-RTP-U2`, `T-RTP-W1`, `T-RTP-W2` |
| **3551** | RTP Profile for Audio and Video Conferences (AVP) | Static PCMU/PCMA payload negotiation and audio delivery. | 🟡 **Partial** — the cited call tests do not certify the complete AVP profile | `T-RTP-W1`, `T-RTP-W2`; PBX codec rows are supplemental interop evidence only |
| **3711** | The Secure Real-time Transport Protocol (SRTP) | SRTP/SRTCP encryption + auth. | 🟡 **Partial** — SDES-keyed SRTP; DTLS-SRTP excluded | `srtp_call_integration.rs`, SRTP negotiator tests, PBX SRTP rows |
| **4733** | RTP Payload for DTMF / Telephony Tones (telephone-event) | Telephone-event send/receive behavior in the supported audio-call profile. | 🟡 **Partial** — implementation locations and historical PBX/SIPp rows do not certify every event/timing/interoperability case | DTMF integration tests; PBX/SIPp rows are supplemental rather than sole conformance evidence |
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
| **4028** | Session Timers in SIP | Successful session-refresh and refresh-failure event delivery. | 🟡 **Partial** — negotiation roles, `422`/Min-SE behavior, proxy handling, and the expiration/race matrix are not claimed | `T-4028-W1`, `T-4028-W2`; ignored resilience stubs are excluded |
| **3326** | The Reason Header Field for SIP | `Reason:` on BYE/CANCEL and responses. | 🟡 **Partial** — Reason emitted on teardown paths | teardown/reason handling, `teardown_rfc_state_table_tests.rs` |
| **3891** | The SIP "Replaces" Header | Replace an existing dialog (attended transfer / pickup). | 🔵 **Types/construction only** — Replaces can be constructed and carried, but executable replacement semantics are not established | `T-3891-C1`, `T-3891-U1` (neither test executes call replacement) |
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
| **3261 URIs** | SIP / SIPS URI scheme | `sip:`/`sips:` URI parsing/building with supported parameters and headers. | 🟡 **Partial** — parser and fuzz coverage do not establish every RFC 3261 URI production or normalization rule | `sip-core` URI parser tests and URI fuzz target |
| **3986** | URI: Generic Syntax | Generic URI parsing used by the supported SIP URI profile. | 🟡 **Partial** — no complete RFC 3986 conformance suite is claimed | `sip-core` URI parser tests and URI fuzz target |
| **3966** | The `tel` URI for Telephone Numbers | `tel:` URIs and `phone-context`. | 🟡 **Partial** — tel URI parse/build | `sip-core` URI types |
| **2045** | MIME Part 1: Format of Message Bodies | MIME headers / encodings for SIP bodies. | 🔵 **Types only** | `sip-core` content-type / MIME parsing |
| **2046** | MIME Part 2: Media Types | `multipart/*`, media-type registry. | 🟡 **Partial** — multipart bodies parsed (torture `mpart01`) | `sip-core` multipart parsing, `rfc_compliance/wellformed/3.1.1.11_mpart01.sip` |
| **5621** | Message Body Handling in SIP | Multipart body handling rules for SIP. | 🟡 **Partial** — multipart carry-through | `sip-core` body handling |
| **5646** | Tags for Identifying Languages (BCP 47) | `Content-Language`/`Accept-Language` tag validation. | 🟡 **Partial** — language-tag parse/validate (incl. grandfathered tags) | `sip-core` language-tag parser tests |
| **4475** | SIP Torture Test Messages | Included well-formed fixtures parse and included malformed fixtures are rejected. | ✅ **Verified** (bounded to the checked-in corpus; the documented well-formed exclusions remain outside the claim) | `T-4475-U1`, `T-4475-U2` |
| **5118** | SIP Torture Test Messages for IPv6 | IPv6-specific torture cases. | 🟡 **Partial** — IPv6 cases in corpus | `rfc_compliance` corpus (`4.2_ipv6-bad.sip`, …) |

---

## 12. Codecs (payload formats)

> Codec payloads live in the media crates / `rvoip-webrtc`; listed here because
> they are negotiated through SIP/SDP.

| RFC | Title | Description | Compliance | Verified by |
|-----|-------|-------------|------------|-------------|
| **G.711** (ITU) | PCMU / PCMA | μ-law / A-law audio using static payload types 0/8. | 🟡 **Partial** — audio round-trip is executable; PBX rows require a source-matched release attestation | `T-RTP-W1`, `T-RTP-W2`; PBX matrix is supplemental interop evidence |
| **G.729** (ITU) | G.729 / G.729A/AB | Low-bitrate audio in the configured PBX profiles. | 🟡 **Partial** — historical `g729a`/`g729ab` PBX rows do not qualify the current source | PBX `g729a g729ab` matrix profiles, subject to clean attestation |
| **4867** | RTP Payload Format for AMR and AMR-WB | `a=rtpmap:… AMR/8000` / `AMR-WB/16000` with `mode-set`, `octet-align` and `mode-change-*` fmtp; octet-aligned and bandwidth-efficient framing, CMR and DTX. | 🟡 **Partial** — codec and payload format are implemented and lab-verified, including against an independent dissector; no release attestation covers them | Framing agreement with Wireshark's AMR dissector; loopback and SDES-SRTP call tests; PBX and proxy interop lab rows |
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
- RFC 5626 Outbound: flow-token processing, keepalive/recovery, and multi-flow
  registration (current evidence covers Contact construction and configuration
  validation only).
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
- **Keep claims honest:** a row may only be marked ✅ **Verified** if its exact,
  bounded behavior cites at least one `T-*` entry whose named test exists, is
  executable, and is neither ignored nor a stub. A release claim additionally
  requires that evidence to be green in a clean, source-matched current beta
  gate. Promote from 🟡/🔵 only when both conditions are met.
- **Source of beta claims:** the crate-local
  [`RFC_COMPLIANCE_MATRIX.md`](../../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md)
  governs what release notes may claim; this file is the broader engineering &
  roadmap view.
- **Regenerate the attestation:** run `crates/sip/rvoip-sip/scripts/beta_gate.sh`
  (see commands at the top), then update the *Last reviewed* date and the report
  timestamp/revision in the header.
