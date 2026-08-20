# ICE for rvoip — implementation plan

Status: **plan, for owner review** · 2026-08-19 · branch `thelve/rvoip-22-ingress`
Companions: `docs/PRODUCTION_HARDENING_ROADMAP.md` (DTLS-SRTP/ICE/WebRTC track),
Thelve's `docs/architecture/telephony-managed-and-byo-sip-plan.md` (ICE §, which
this plan supersedes as "scoped as a real protocol build").

---

## 1. Why, and why all roles

Owner decision of record: rvoip is a **library**, and ICE should be available
to whatever a developer builds with it — UAC, UAS, B2BUA, or SBC — because
offering the standards-track traversal mechanism makes connecting UAs to each
other work in topologies where a static advertised address cannot, even when
the far side is a server that never needed it.

The IETF position supports this direction and runs opposite to common SBC
practice: **latching** (send media back to wherever media came from — what
SBCs actually do on trunks, and what rvoip's trusted-trunk path effectively
relies on today) is documented in RFC 7362 with an explicit recommendation
**against** using it over the Internet, naming ICE as the replacement. ICE
itself (RFC 8445, obsoleting 5245) is live Standards Track, with current SDP
offer/answer procedures in RFC 8839 and the SIP option-tag in RFC 5768. So
this plan implements the recommended mechanism, keeps restricted latching as
the non-ICE fallback, and treats the two as complementary rather than rivals.

What each role gets:

| Role | Today | With this plan |
|---|---|---|
| **UAS / gateway with public IP** (Thelve) | static `media_public_addr` or STUN-discovered address; NAT'd callers depend on their own SBC | **ice-lite**: NAT'd full-ICE callers traverse to us with no SBC in between |
| **B2BUA** (Thelve gateway shape) | per-leg latching | per-leg independent ICE sessions (RFC 7584 media-terminating B2BUA posture) |
| **UAC behind NAT** | must be given an advertised address; symmetric NAT fails | **full ICE**: gathers host+srflx, checks, nominates |
| **UA ↔ UA** (both NAT'd) | not reachable without a relay in the middle | full ICE both ends; TURN relay later for symmetric↔symmetric |
| **SBC edge** | restricted latching (RFC 7362) | ice-lite on the public side; latching retained as fallback with `a=ice-mismatch` detection |

## 2. Standards set

| RFC | What | Where in this plan |
|---|---|---|
| 8445 | ICE agent (obsoletes 5245; removes aggressive nomination) | Phases I2 (lite), I4 (full) |
| 8839 | SDP offer/answer for ICE (candidate lines, ufrag/pwd, ice-mismatch, default candidate) | Phase I3 |
| 5768 | SIP option-tag `ice` (Supported header) | Phase I3 |
| 8489 | STUN (codec + short-term credentials) | Phase I1 — partially present |
| 5769 | STUN test vectors (MESSAGE-INTEGRITY / FINGERPRINT) | Phase I1 acceptance |
| 7584 | STUN handling in SIP B2BUAs | Phase I5 (posture + docs) |
| 7675 | Consent freshness | Phase I4 (keepalive integration, "should") |
| 8656 | TURN (relay candidates) | Phase I7, deferred |
| 8838/8840 | Trickle ICE | **Non-goal** for SIP v1 (see §10) |
| 7362 | Latching / HNT | retained as the non-ICE fallback |

## 3. Verified inventory (what the tree already has)

Surveyed 2026-08-19 at `6aeb1205`. File paths are load-bearing — this section
exists so the next survey does not re-conclude any of it is missing.

1. **STUN client, RFC 8489 Binding only** —
   `crates/media/rtp-core/src/network/stun/{mod,message}.rs`. Retries with
   fresh transaction ids, XOR-MAPPED-ADDRESS, bounded budget. Header comment
   explicitly states: no MESSAGE-INTEGRITY, no FINGERPRINT, no USERNAME.
   Those are exactly what ICE connectivity checks require → Phase I1.
2. **STUN/RTP demux on media sockets already exists** —
   `classify_rtp_mux_packet` in `crates/media/rtp-core/src/transport/udp.rs`.
   Connectivity checks arrive on the media port; the demux point is built.
3. **Full RFC 8839 candidate-line parser already in sip-core** —
   `crates/sip/sip-core/src/sdp/attributes/candidate.rs` (foundation,
   component-id, transport, priority, typ host/srflx/prflx/relay,
   raddr/rport), plus `ice.rs` (ice-ufrag, ice-pwd, ice-options,
   end-of-candidates) and builder methods for `ice_ufrag`/`ice_pwd` in
   `sdp/builder.rs`. SDP *parsing* is largely done; generation and O/A
   procedure are the gap.
4. **rtcp-mux is a real knob** — `RtpTransportConfig.rtcp_mux`
   (`rtp-core/src/transport/security_transport.rs` tests both modes).
   Muxed ⇒ one ICE component; non-muxed ⇒ two. §9 gates v1 full ICE on mux.
5. **No homegrown ICE agent exists.** `crates/webrtc/*` wrap the external
   webrtc-rs stack (`webrtc` workspace dep; `peer/ice.rs` is 53 lines of
   config plumbing). Nothing to hoist — the agent is a genuine build.
6. **NAT simulation is already a workspace citizen** — `webrtc-util` with the
   `vnet` feature (virtual network with NAT mapping policies) is a dependency
   of rvoip-webrtc; usable as a dev-dependency for ICE tests.
7. **The insertion point is explicit** — `Config::media_public_addr`
   (`rvoip-sip/src/api/unified.rs:2249`); its own doc at :2343 says
   "Internet edges should use a static media_public_addr today", i.e. the
   gap this plan closes. `SipConfig::discover_advertised_addr` (STUN, commit
   `6aeb1205`) doubles as the srflx gathering primitive.
8. **UCTP-over-WebSocket already runs full ICE** via the negotiated WebRTC
   PeerConnection (`docs/CONVERSATION_PROTOCOL.md` §10.2, no trickle in v0).
   Browser-ish substrates are covered; the SIP/RTP path is the gap.

## 4. Target architecture: a sans-io agent crate

New crate **`crates/media/ice-core`** (`rvoip-ice-core`), sitting *below*
rtp-core:

```
sip-core (SDP parse/build)          ice-core (NEW, sans-io)
        \                            - STUN codec (absorbs rtp-core/network/stun)
         \                           - candidate types + priority (8445 §5.1.2)
          rvoip-sip coordinator      - check lists, roles, nomination
          - O/A wiring (I3)          - keepalive scheduling
          - agent lifecycle          - API: handle_packet / handle_timeout /
          - re-INVITE post-nominate         poll_transmit / poll_event
                |
          rtp-core (sockets)
          - demux → agent.handle_packet
          - agent.poll_transmit → socket
          - nominated pair → RTP send target
```

- **Sans-io**: the agent is a pure state machine — caller supplies packets
  and the clock, and polls for transmissions and events. No tokio, no
  sockets, no `Instant::now()` inside. This is the house style for a reason
  this repo already proved twice (`PlayoutBuffer` takes `arrived_at`;
  the conference mixer's `mix_once` is clock-free): every ICE pathology —
  lost checks, role conflicts, nomination races, restarts — becomes a
  deterministic scripted test instead of a flaky timing test.
- **STUN codec moves into ice-core** (rtp-core re-exports for
  compatibility). The codec grows the ICE attribute set in place.
- **Considered and rejected for production: depending on webrtc-rs's `ice`
  crate.** Pros: exists, interop-proven. Cons: tokio-coupled runtime objects
  in the SIP core path, non-deterministic tests, a large dependency tree in
  every SIP deployment, and this repo has deliberately owned its STUN and
  SDP rather than adopt equivalents. **Kept instead as the in-tree test
  opponent** (§8): our agent must interop with webrtc-rs's under vnet NATs
  before it meets a real carrier. It also remains the de-risk fallback if I4
  stalls badly (§9).

## 5. Role × mode matrix

| Deployment | ICE mode | Controlling? | Gathers |
|---|---|---|---|
| UAS / B2BUA / SBC edge on a public address | **lite** (`a=ice-lite`) | never (full peer controls); lite↔lite degenerates to default candidates, i.e. today's behavior | single host candidate (the advertised/discovered address) |
| UAC behind NAT | full | when offering (8445 §6.1.1) | host (+v6) + srflx via existing StunClient |
| UA↔UA | full both ends | offerer | host + srflx |
| B2BUA legs | independent per leg (RFC 7584: we terminate media, so no cross-leg candidate relaying) | per leg | per leg |

ice-lite is only correct on a genuinely public address — the builder must
refuse `IcePolicy::Lite` without one (`advertised_addr` or STUN-discovered).

## 6. Phases

Sizes: S < 1 day-ish, M = days, L = week-plus, XL = the largest single item
on this branch (bigger than the conference mixer).

### I1 — STUN codec completion + server behavior [M]
- Attributes: USERNAME, MESSAGE-INTEGRITY (HMAC-SHA1, short-term
  credentials), FINGERPRINT (CRC-32 xor), PRIORITY, USE-CANDIDATE,
  ICE-CONTROLLING / ICE-CONTROLLED (64-bit tie-breaker), ERROR-CODE
  (400/401/487), UNKNOWN-ATTRIBUTES; Binding **Indication** (keepalives);
  Binding **success/error response generation** (we only parse responses
  today, and only send requests).
- Validation order per 8489 §9.2.4 (fingerprint → integrity → ufrag).
- rtp-core: answer Binding requests arriving on media sockets (demux exists);
  refuse unauthenticated checks once a session has credentials.
- **Acceptance**: RFC 5769 test vectors pass byte-exact; a webrtc-rs agent's
  checks against our socket get valid, integrity-protected responses.

### I2 — ice-lite responder [M]
- `a=ice-lite` in our SDP; single host candidate; per-session ufrag/pwd
  generation (≥4 / ≥22 chars, 8445 §5.3).
- Answer checks with short-term credentials; adopt the peer-nominated pair
  (lite side is always controlled vs a full peer); retarget the RTP session's
  send destination to the nominated pair (supersedes latching for that leg).
- `IcePolicy::Lite` refused without a public address.
- **Acceptance**: a full-ICE client behind a vnet NAT (webrtc-rs opponent)
  reaches an rvoip UAS; two-way media; teardown clean. lite↔lite reduces to
  default-candidate behavior (explicit test).

### I3 — SDP offer/answer + SIP signaling [M–L]
- Candidate **encoding** (parser exists); default candidate into `c=`/`m=`
  per 8839; session-level ufrag/pwd; `ice-options: ice2` (8445 §10);
  `a=ice-mismatch` detection → fall back to latching path (the RFC 7362
  interplay: an SDP-rewriting SBC in the path is *detected*, not fought).
- RFC 5768: advertise `Supported: ice`; never `Require`.
- Offer and answer directions in the UnifiedCoordinator; ufrag/pwd rollover
  and ICE-restart plumbing (used by I4).
- **Acceptance**: O/A against baresip/linphone captures parses and encodes
  round-trip; mismatch path proven by a rewriting test proxy.

### I4 — full ICE agent [XL — the honest center of gravity]
- Gathering: host (incl. IPv6) + srflx (existing StunClient per socket);
  priority formula 8445 §5.1.2.1 (host 126 / prflx 110 / srflx 100 /
  relay 0); foundations; component 1 (see §9 rtcp-mux gate).
- Check lists: frozen→waiting→in-progress→succeeded/failed; pair priority
  (8445 §6.1.2.3); triggered checks; Ta pacing ≥ 50 ms per agent (§14).
- Roles: determination per §6.1.1, tie-breaker, 487 role-conflict repair.
- **Regular nomination only** (aggressive was removed by 8445 — nothing to
  build); controlling side nominates once a pair validates.
- Keepalives: Binding Indication ~15 s (§11); consent freshness (RFC 7675)
  as integrity-protected checks on the nominated pair — "should", same timer
  wheel.
- Post-nomination: controlling side sends re-INVITE updating `c=`/`m=` to
  the nominated pair when it differs from the default (8839 §4.4).
- ICE restart on re-INVITE (new ufrag/pwd) wired to the existing
  connection-replacement machinery.
- prflx candidates learned from inbound checks.
- **Acceptance**: two rvoip UAs behind two *different* vnet NAT policies
  (port-restricted ↔ address-restricted, etc.) establish two-way media with
  no static advertise config; scripted sans-io suites for loss, reorder,
  both-controlling conflict, nomination race, mid-call restart.

### I5 — B2BUA / SBC posture [S–M]
- Per-leg agents in the coordinator; RFC 7584 documented position
  (media-terminating B2BUA ⇒ legs independent).
- SBC guidance: lite on the public interface; restricted latching retained
  as the non-ICE fallback; `ice-mismatch` → latching, logged.

### I6 — facade + Thelve exposure [S–M]
- `SipConfig::ice(IcePolicy::{Disabled, Lite, Full})`, default **Disabled**
  (additive; zero behavior change unconfigured — branch discipline).
- Synergy: `Lite` + `discover_advertised_addr` = zero-config public server.
- Thelve: `RVOIP_SIP_ICE=lite|full`, chart `realtimeIngress.media.ice`,
  runbook rewrite of the "ICE is not implemented" section.

### I7 — TURN client (RFC 8656) [L, deferred]
- Relay candidates; only needed when both peers are behind
  address/port-dependent (symmetric) NAT — not the server-anchored
  topologies rvoip ships today. Allocate/refresh/permissions/channels is its
  own state machine. Schedule when a concrete deployment needs it.

## 7. Cross-crate impact

| Crate | Change | Size |
|---|---|---|
| `media/ice-core` (new) | STUN codec (moved+grown), candidates, agent | the bulk: I1 + I4 |
| `media/rtp-core` | stun module moves out (re-export shim), demux → agent feed, send-target retarget on nomination | M |
| `sip/sip-core` | candidate *encoding*, ice-lite/mismatch attrs, builder completeness | S–M |
| `sip/rvoip-sip` | coordinator O/A wiring, agent lifecycle per media session, option-tag, re-INVITE post-nomination, restart | L |
| `rvoip` (facade) | `IcePolicy`, builder validation (Lite⇒public addr) | S |
| `webrtc/*` | none (webrtc-rs keeps its own ICE); dev-dep opponent only | 0 |
| Thelve | env + chart + runbook | S |

## 8. Test strategy

1. **RFC 5769 vectors** — byte-exact MESSAGE-INTEGRITY/FINGERPRINT.
2. **Sans-io scripted suites** — the reason the agent is sans-io: injected
   clock + packet timelines for loss, duplication, reordering, role
   conflict, nomination race, restart, pacing (assert no burst > Ta).
3. **vnet NAT matrix** — `webrtc-util/vnet` as dev-dep: full-cone,
   address-restricted, port-restricted, symmetric; rvoip↔rvoip and
   rvoip↔webrtc-rs (the external stack as interop opponent *in tests*,
   which is where its interop pedigree pays without becoming a production
   dependency).
4. **Manual interop checklist** — baresip, linphone, FreeSWITCH/Asterisk
   (mod_sofia ICE), documented in the runbook before the pilot-grade label
   moves.

## 9. Risks & mitigations

| Risk | Mitigation |
|---|---|
| I4 is genuinely large; stall risk | I1–I3+I6 ship standalone value (lite server); webrtc-rs `ice` crate held as fallback spike if the agent bogs down |
| SDP-rewriting SBCs break ICE mid-path | 8839 `ice-mismatch` detection → automatic fallback to the latching path; logged, surfaced in quality events |
| Non-muxed RTCP doubles the check matrix | v1 full ICE **requires `rtcp_mux`** (builder-enforced); component 2 stays in the data model for later; lite is unaffected (single candidate either way) |
| Timer load at scale | one timer wheel per coordinator, agents registered; Ta pacing bounds per-agent send rate |
| SRTP interplay | ours is SDES (`a=crypto`) — composes with ICE (keys in SDP, media on nominated pair); DTLS-SRTP-over-ICE remains the WebRTC path's concern |
| Branch/PR keeps growing | each phase is its own commit with its own tests; reconvergence PR reviews phase-by-phase |

## 10. Non-goals (v1, explicit)

- **Trickle over SIP** (8838/8840): needs INFO/UPDATE plumbing; SIP practice
  is full-gather-then-offer; even our UCTP protocol chose no-trickle in v0.
- **TCP candidates** (RFC 6544), **mDNS candidates**: WebRTC-world concerns.
- **Aggressive nomination**: removed by 8445 itself.
- **TURN** until a symmetric↔symmetric deployment exists (I7).

## 11. Sizing summary & recommended order

| Milestone | Phases | Outcome | Rough size |
|---|---|---|---|
| **M1: NAT'd callers reach rvoip servers** | I1→I2→I3→I6 | lite everywhere a public address exists; zero config with STUN discovery | ≈ the recording+conference work combined |
| **M2: rvoip works from behind NAT** | I4→I5 | full agent; UAC + UA↔UA; B2BUA per-leg | ≈ M1 again, mostly I4 |
| **M3: relay** | I7 | symmetric↔symmetric via TURN | L, on demand |

## 12. Decisions of record (owner, 2026-08-19)

1. **Full M2 out of the gate.** No lite-only checkpoint: I1→I6 are one
   continuous build ending with the full agent. The phase order stands as a
   dependency order, not a shipping order.
2. **`ice-core` crate confirmed** — chosen explicitly to keep the blast
   radius away from the other crates.
3. **Interop targets**: local `~/Developer/freeswitch` (dockerized) and
   `~/Developer/asterisk` checkouts, plus webrtc-rs-under-vnet in tests.
4. **rtcp-mux gate for v1 full ICE stands** (explained and accepted): a call
   is two UDP flows (RTP + RTCP) unless RFC 5761 muxes them onto one port;
   ICE traverses per flow ("component"), so non-mux doubles the check
   machinery. When ICE is enabled the offer carries `a=rtcp-mux`; a peer
   that wants ICE-without-mux falls back to non-ICE behavior for that call.
   Everything that speaks ICE in practice speaks mux (WebRTC mandates it;
   FreeSWITCH/Asterisk/baresip support it); component ids stay in the data
   model so two-component support is an extension, not a rewrite. rvoip's
   own SDP builder already ships `.rtcp_mux()`.
