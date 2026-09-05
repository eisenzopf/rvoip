# rvoip 0.3.9 release plan

Status: release candidate preparation, 2026-09-05.

Implementation is complete for 18 of the 20 issues assigned to the milestone.
PR #213 merged the complete implementation and passed 32/32 PR jobs plus all
five CodeQL analyses. Issue #182 remains open until the protected
`remote-release` run regenerates the evidence documents. Issue #102 remains
open until the production remote-endpoint profile has its required live
two-UA NAT/TLS/SDES evidence; its product code and deterministic tests are in
the candidate. These qualification-bound items must not be closed from unit or
hosted CI evidence alone.

The release exposes six manifest-derived deployment bundles and tests each
with default features disabled: SIP endpoint, carrier SIP, browser gateway,
AI conversation gateway, full pure-Rust, and full native. The original
planning baseline below is retained as the decision record; statements about
what was absent or open describe the 2026-08-16 baseline, not current status.

This is one plan built from two verified sweeps: the full open-issue triage
(all 47 issues) and the complete Thelve backlog audit (all 40 partner issues,
#79–#118). **Every claim below was checked against `main` at `b52c4806`** —
file/line citations are what the tree says today. Nothing is scheduled on
reputation, and nothing was closed on age.

## Release theme: carrier-grade honesty

One finding repeats across both sweeps: **the primitive exists, and nothing
reaches it.** A jitter buffer whose constructor has one occurrence — its own
definition. An RTCP XR VoIP-metrics sender with zero callers. A MOS formula
whose consumers ride a hardcoded 4.5. A typed P-Asserted-Identity parser that
no inbound path ever consults. Config and READMEs assert the opposite of
runtime behaviour in each case.

0.3.9 exists to close that gap on the SIP media path specifically: make rvoip
behave the way a carrier auditing it would already believe it behaves.

---

## Workstream 1 — carrier-grade media: jitter, quality reporting (#168 + new)

What a carrier expects on a SIP trunk, versus what the tree does today:

| Carrier expectation | Primitive in tree | Reachable from SIP path |
|---|---|---|
| Jitter buffer w/ reorder + PLC | 4 implementations | **No** — `enable_jitter_buffer: false` at `media-core/src/relay/controller/mod.rs:858`; config read into `let _` at `rtp-core/src/session/mod.rs:750-752`; `with_jitter_buffer` has 1 occurrence (its own definition, `stream.rs:115`); PLC commented out (`processing/audio/mod.rs:20`) |
| RTCP SR/RR | Full packet layer (`rtp-core/src/packet/rtcp/`) | Yes |
| RTCP XR VoIP metrics (RFC 3611) | `VoipMetricsBlock`, `send_rtcp_xr_voip_metrics` (`api/server/transport/mod.rs:288`) | **No** — zero callers outside rtp-core |
| MOS / R-factor | `calculate_mos(loss, jitter, latency)` (`media-core/src/quality/metrics.rs:159`) | **Partial** — the function exists; `QualityMetrics::default()` hardcodes `mos_score: Some(4.5)` (`types/stats.rs:78`), "default to excellent" |

### 1a. Jitter buffer on the SIP receive path (#168)

Select one of the four implementations, wire it into
`rtp-core/src/session/stream.rs` delivery, flip the controller default, and
delete the three dead `let _` bindings. Acceptance: a loss/reorder/drift matrix
in the release gates — packets delivered in RTP order under reorder, concealment
on single-packet loss for G.711, drift compensated across a long soak.
**This is the release's big rock. Size it first; if it does not fit, 0.3.9
reduces scope elsewhere rather than shipping a half-wired buffer.**

Scope note: 0.3.7's adaptive-jitter work was the Vapi/WebSocket path. It does
not touch this.

### 1b. RTCP XR + honest MOS wired end-to-end (new issue to file)

- Feed measured loss/jitter/RTT into `calculate_mos`; remove the 4.5 default
  (an `Option::None` until measured is honest; 4.5 is not).
- Emit RTCP XR VoIP-metrics blocks per RFC 3611 on the SIP media path at a
  configurable cadence, populated from the same measurements.
- Surface per-call quality (loss, jitter, RTT, MOS) on the stats API that CDR
  emission already reads, so carriers get call-quality records without scraping.
- The RX jitter/gap telemetry currently gated behind `perf-diagnostics`
  (media-core default features are `["pcmu","pcma"]`) must be measurable in a
  stock build — a carrier cannot be told to recompile to see degradation.

### 1c. Sinkless-frame logging (#186) and barge-in flush (#188)

Small, same family: a blackholed media graph must say so
(#186), and `flush_media_graph` must gain its production caller on the Vapi
barge-in path (#188).

---

## Workstream 2 — carrier trunk interop: P-headers and header policy (new)

Carriers deliver caller identity and charging context in P-headers
(RFC 3325 P-Asserted-Identity, P-Charging-Vector, P-Early-Media). Verified
today:

- **Parsing exists**: typed `PAssertedIdentity` / `PPreferredIdentity`
  (`sip-core/src/types/p_asserted_identity.rs`, header names registered).
  No P-Charging-Vector / P-Early-Media types.
- **Inbound**: no production path reads PAI — its only appearances in
  `rvoip-sip/src/adapters/` are inside `mod tests`. `capture_headers`
  (`rvoip/src/app.rs:262`) accepts only `X-*` and rejects PAI by design.
- **Outbound**: `originate.rs:756` strips PAI/PPI from custom headers —
  "owned by typed SIP operations" — but no typed operation supplies them.

The existing refusal is correct as a default: a peer-supplied identity header
is a claim, not a fact. RFC 3325 agrees — PAI is only meaningful inside a
trust domain (Spec(T)). The gap is that rvoip has the trust-domain primitive
(`trusted_trunk`, CIDR + TLS subject, shipped 0.3.7) and does not use it to
gate identity headers.

### 2a. Trust-domain P-header acceptance (closes the #170 refusal properly)

- `SipConfig` gains an explicit identity-header policy bound to trunks:
  PAI accepted **only** from peers matching a `trusted_trunk`; stripped
  otherwise, per RFC 3325 §5. Untrusted default unchanged: reject.
- Accepted identity flows into `InboundConnectionContext` as a distinct,
  provenance-marked field — never mixed into `X-*` metadata, so an application
  can always tell asserted identity from captured headers.
- Configurable accept-list for further P-/proprietary headers on trusted
  trunks (P-Charging-Vector for billing correlation first), same
  trust-gating, same provenance marking.

### 2b. Outbound typed identity

A typed way to set PAI/PPI on originate for trunks that require it (the strip
at `originate.rs:756` stays; the typed operation it promises starts existing).

### 2c. Response Contact transport planning (#184) and NAT-path consistency (#185)

Both are trunk-visible correctness bugs adjacent to this workstream: TCP/WS
dialogs currently receive a UDP-resolving `sip:` fallback Contact (#184), and
the observed-source NAT override misses `send_bye_with_reason` and INFO/NOTIFY
(#185), so NAT behaviour differs across methods within one dialog.

---

## Workstream 3 — security: AAuth delegation semantics (#93)

The only *exploitable* finding of either sweep; three of its five claims
verified on `main`:

- **Scope union** — `auth-core/src/aauth.rs:198-201`: effective scopes are
  subject ∪ actor, so an actor token contributes authority the human subject
  never held. The module doc (`:15`) states the union plainly — the code does
  what it intends; the intent is wrong for a trust gate. Fix: delegated
  authority must narrow (intersection, or subject-bounded), never widen.
- **No upper clock bound** — `sig9421.rs:211-212` rejects only `age > ttl`; a
  far-future timestamp gives negative age and passes.
- **Replay not consume-once** — process-local bounded evicting cache,
  get-then-insert (`sig9421.rs:163`, `:11`); concurrency, replicas, or
  eviction each defeat it.

Unverified from the issue: RFC 8785 canonicalisation coverage; unqualified
`KeyResolver` key IDs. **Severity triage before scheduling: if any deployment
gates AI tool payloads on AAuth today, the scope union is privilege escalation
and ships as a point fix, not as part of 0.3.9.**

---

## Workstream 4 — API honesty (#169) and codec identity (#86, #177)

- **#169** — `frames_in()` returns a guaranteed-dead receiver
  (`rvoip-vapi/src/media.rs:170-171`). Delete it; the fallible variants are
  what everything real already uses. Breaking trait change — belongs in a
  release that says so.
- **#86** — per-session codec override for renegotiation. Sequenced **after**
  Parts B/C of the codec-identity plan (`RtpAudioCodec` in codec-core →
  rvoip-sip rewiring → typed top-layer API); doing it alone adds another
  per-session path over the ~20 scattered codec↔PT copies Part B consolidates.
- **#177** (+ #107) — `RtpMediaPayload` drops marker/CSRC/extensions/padding
  at the UCTP boundary; scope with the packetization helpers as one boundary
  fix.

---

## Workstream 5 — release engineering (#198, #182, + #80 pickoff)

- **#198** — record partial shard results before worker teardown. Cost 0.3.8
  at least one full qualification cycle. Highest-leverage pipeline change.
- **#182** — regenerate the beta evidence docs from the qualification run
  (frozen at 0.2.5; deferred out of 0.3.8 by explicit decision).
- **#80 pickoff** — wire `RVOIP_WT_SMOKE=1` into a gate. The Chromium
  WebTransport browser test exists (`tests/browser-smoke/tests/wt_smoke.spec.mjs`)
  and has never run in CI (`RVOIP_WT_SMOKE` appears nowhere in `.github/` or
  `scripts/`). Zero product code converts "present" into "proven".
- Watch item: the evidence-validation retry (`b52c4806`) has never fired in
  CI. A `retry.log` in 0.3.9 qualification is its first live proof.

---

## Thelve backlog disposition (all 40 verified — none closable)

Full table retained from the sweep. Summary: **not one of the forty is fully
delivered.** 0.3.5–0.3.8 did not advance this backlog.

| Cluster | Issues | State |
|---|---|---|
| Harness AI runtime | 81, 82, 97, 98, 101, 104 | **No Harness crate exists.** Six issues describing a subsystem that has not been started — consolidate behind one product decision. |
| Connection handoff | 79, 92 | Not started. `atomic_inbound_handoff` machinery governs duplicate inbound admission, not device movement; no public handoff API. |
| WebTransport browser | 80 | Partial, unproven — see Workstream 5. |
| UCTP continuity/control | 83, 84, 89, 91, 114 | Not started (no reconnect/grace, attended-transfer, push, SFU, remote adapter). |
| Vapi tool-result | 85, 164 | Not started; `VapiCommand` = say/add-message/control. Same capability, two requirements (typed API vs in-band) — scope together. |
| Codec/RTP boundary | 86, 107, 177 | Workstream 4. |
| vCon | 87, 88, 95 | Not started (no provenance links; "JWE is not implemented", `UCTP_IMPLEMENTATION_PLAN.md:1361`). |
| Security | 93 | **Workstream 3.** |
| Identity extensions | 94 | Crates exist; storage/verification boundaries **not assessed in depth** — needs its own security review. |
| Client/App surface | 96, 100, 105, 108, 110–113 | Not started (`rvoip-client/src/` is one `lib.rs`; no per-call API on `RvoipApp`). |
| SIP registration/NAT | 102 | Partial — RFC 5626 flow-recovery test exists; registration/NAT profile does not. |
| Vapi idempotency | 103 | Not started. |
| RTCP across bridges | 109 | Not started; related to Workstream 1b. |
| Fallback streams | 90 | Stub (`allocate_subscriber_stream` error path). |
| Tooling/infra | 106, 116, 117, 118 | Not started, except #118 partial (`trusted_trunks` shipped 0.3.7; infra-common bearer utilities unverified). |

Which Thelve items ride 0.3.9: **#86, #93, #107/#177, and the #80 pickoff** —
the ones that intersect the carrier/media/security theme. The rest need product
prioritisation with the partner, not silent scheduling.

## PR #35 audit — what still needs implementing

PR #35 (`release` → `main`, +20103/−2705 over 201 files, now CONFLICTING) merged
a 41-commit `develop` line with the 0.3.2 release line. Verified item-by-item
against `main`: **roughly half its content has since landed via 0.3.5's
independent reimplementation; the other half remains genuinely unimplemented.**
The PR itself is unmergeable and should be closed once its remaining value is
captured as issues; the branch remains the reference implementation.

**Superseded — already on `main`, do not re-implement:**
RTT middle-32-bit LSR (`ntp.rs:48`), RTP padding flag/length validation,
RFC 8285 ID-15/padding handling (`extension/mod.rs:369`), loss/jitter
accounting across rollover/reorder (0.3.5), SRTP/SRTCP hardening + replay +
libSRTP interop (0.3.5), SDES directional answer keys (0.3.5, issue #46), real
Opus backend (0.3.5), RFC 7118 WS subprotocol echo, WS/WSS `has_connection_to`,
WS/WSS config on `Config`, per-call TLS/WSS client identity (present in
sip-transport), RFC 4733 DTMF types in rtp-core.

**Still absent from `main` — the live remainder:**

| Item | Evidence | Disposition |
|---|---|---|
| **DTLS-SRTP on the SIP path** | `run_dtls_handshake_and_install`, `Config::srtp_keying`: absent; 0.3.5 made unsupported DTLS *fail closed* rather than work | **#202 — scheduled for 0.3.9** (owner direction, 2026-08-16) |
| **ICE / RFC 8445** (`rvoip-nat-core`) | No crate; no `ice-ufrag` offer/answer wiring; no `enable_ice` | **#203 — scheduled for 0.3.9**; closes the media half of Thelve #102 |
| **Tolerant RTCP compound walker** (`RtcpPacketIter`) | Absent; strict parser still aborts compound on unmodeled PT 205/206 | **#204 — scheduled for 0.3.9**, lands before or with #200: real peers compound SR with PT 205/206 feedback, so the strict-only parser starves the XR/MOS pipeline on exactly the paths carriers care about |
| **`RtpPacketSequencer` / `Shared`** | Absent | New issue, low priority; not scheduled |
| **G.722 real implementation** | No `g722` feature in codec-core; 0.3.5 chose to stop advertising it | Decide: implement (ezk-g722 per the PR) or close as declined |

The three scheduled items share one caveat carried from the PR: they were
written against the pre-0.3.5 SRTP/session architecture. The branch is mined
for design and tests, not cherry-picked — 0.3.5 rebuilt the SRTP state model
underneath it.

## Full disposition — all 54 open issues, nothing closed

Owner direction (2026-08-16): Thelve and rvoip share an owner, so
"blocked on partner prioritisation" was never a real category. Re-read with
that in mind, every issue in the 2026-08-02 batch carries the same boundary
statement — *"application-specific authorization, policy, persistence, and UI
remain outside the requested RVoIP boundary"* — which means they were each
scoped to this crate deliberately. **They all belong here, so none is closed;
every one gets a release assignment instead.**

Fifty-four issues cannot ride one patch release, so the honest structure is two
milestones rather than one aspirational list.

### Milestone `0.3.9` — 20 issues

Carrier-grade honesty, as detailed in the workstreams above.

| Theme | Issues |
|---|---|
| Media path correctness | #168, #186, #188, #204 |
| Carrier quality reporting | #200 |
| Carrier trunk interop | #201, #184, #185 |
| Transport security / NAT | #202 (DTLS-SRTP), #203 (ICE), #102 (media half) |
| Security | #93 |
| API honesty / codec identity | #169, #86, #107, #177 |
| Client surface | #105 |
| Release engineering | #198, #182, #80 |

### Milestone `0.4.0` — 34 issues

The platform buildout. Verified absent, genuinely in scope, too large for a
patch release. Grouped by the subsystem each one actually lands in — several
collapse into far less work than 34 separate efforts suggests:

| Cluster | Issues | Note |
|---|---|---|
| **Harness AI runtime** | #81, #82, #97, #98, #101, #104 | No crate exists. One subsystem, six requirement documents — build it once and all six close together. |
| **Connection continuity / handoff** | #79, #92, #83 | The two P0s plus UCTP grace behaviour; one coherent design problem. |
| **UCTP call control & reach** | #84, #89, #91, #90, #114 | Attended transfer, push wake-up, SFU, fallback streams, remote adapter. |
| **vCon completion** | #87, #88, #95, #99 | Provenance links, JWE, sibling grouping, loss-observable lifecycle. |
| **RvoipApp production runtime** | #100, #108, #110, #111, #112, #113 | Per-call ownership is the spine; the rest hang off it. |
| **Client & packaging** | #96, #106, #116 | `rvoip-client` is one `lib.rs` today; plus TS packages and the testkit. |
| **Vapi** | #85, #164, #103 | Tool-result over WSS is one capability from two angles (#85 typed API, #164 in-band delivery); plus idempotent call creation. |
| **Identity & config** | #94, #117, #118 | #118 is half-shipped (`trusted_trunks`, 0.3.7); #94 needs the same security-review lane as #93. |
| **Media follow-on** | #109 | RTCP translation across transcoding bridges — successor to #200/#204. |

**Consolidation note, not a closure plan.** The Harness cluster and the Vapi
tool-result pair are each one piece of work wearing several issue numbers.
Recommend a tracking issue per cluster that the members close against, so
progress is visible without discarding the requirement detail each one carries.

Result: **54 of 54 open issues assigned to a milestone, zero closed, zero
unassigned.**

## Closed during triage

| Issue | Disposition |
|---|---|
| #192 | Both items shipped in 0.3.8 (`e9d5625a`, `d39ace2c`+`b52c4806`); remainder split to #198. |
| #170 | Addressed by 0.3.7 (`InboundConnectionContext`); the PAI refusal is superseded by #201's trust-domain design rather than reversed. |

These two are the only closures in the whole triage, and both closed because
the work was *done*, not deferred.

## Suggested sequencing

1. **#93 severity triage** (possibly a 0.3.8.x point fix) and **#80 gate wiring** — immediately.
2. **Sizing pass on the three big rocks together** — #168 (jitter buffer),
   #202 (DTLS-SRTP), #203 (ICE). These are the release's engineering mass, and
   #202/#203 share the RFC 7983/STUN shared-socket demux layer, so they are
   cheaper together than apart. If all three do not fit, the cut order is
   #202 → #203 (ICE without DTLS still serves the SIP NAT case; DTLS without
   ICE serves almost nothing carriers need).
3. **#204 tolerant RTCP walker** — small, lands before or with #200.
4. Workstream 2 (P-headers, #201) and 1b (XR/MOS, #200) — parallel; additive API.
5. #184, #185, #186, #188, #169 — small, land throughout.
6. #198, #182 — release-engineering lane, independent of product code.
7. Codec identity Parts B/C, then #86/#177 — if the big rocks leave room; else 0.3.10.
