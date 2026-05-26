# UCTP v0.x — Remaining Work

**Status as of:** end of the production-hardening pass that landed Tracks A / B / C5 / C2(signaling) / C2(audio-passthrough) / C4(JWT+JWKS+DPoP) / D1–D4. See [UCTP_IMPLEMENTATION_PLAN.md §13](UCTP_IMPLEMENTATION_PLAN.md#13-v0x--production-hardening-track) for the full as-built record.

**Test surface at this milestone:** 155 tests / 0 failures across `rvoip-auth-core` (42) + `rvoip-uctp` (67) + `rvoip-core` (30) + `rvoip-vcon` (9) + `rvoip-quic` (5) + `rvoip-webtransport` (2).

---

This document tracks every item from the original plan that is **not yet landed** and explains the blocker. Items fall into three categories:

1. **Externally blocked** — owned by someone outside this crate, can't proceed without their input.
2. **Cross-ecosystem** — would require pulling in non-Rust infrastructure (Node.js / JavaScript build tooling) that's a meaningful scope-creep decision.
3. **Substantial standards-track work** — each item is a multi-day implementation of an IETF standard, properly its own session(s).

The split matters because category 1 and 2 are decisions for a human, not implementation work; category 3 is just queued effort.

> **Note (2026-05-25):** see [`UCTP_GAP_PLAN.md`](UCTP_GAP_PLAN.md) for the live remaining-work picture. The C3 row below has since landed; the rest of the table is being executed in the gap plan's order.

---

## 1. Externally blocked

### 1.1 B3 — Spec catalog: `501 not-implemented` / `505 version-not-supported`

**Owner:** maintainer of `crates/rvoip-core/CONVERSATION_PROTOCOL.md`.

**Why blocked:** §11.2 of CONVERSATION_PROTOCOL.md is the wire-format authority for error codes. Adding `501` and `505` is a one-line spec PR — not code work. The implementer's follow-up that swaps `503 transient` (today's compromise) → `501 not-implemented` in:

- `OrchestratorSubscriptionHandler` (legacy reject path)
- `ConnectionAdapter::{hold,resume,transfer,send_dtmf-replaced,renegotiate_media,verify_request_signature}` `NotImplemented` returns
- Future-protocol-version handshake rejection (currently produces no code; `505` would land here)

...is ~10 LOC scattered across a handful of files but **must not ship before the spec change** or we'd lock in non-canonical codes.

**What unblocks:** spec PR landing `501` + `505` in §11.2's table with the existing `400/401/403/404/408/487/488/500/503` set. Plan §7.1 carries the assignment.

**Scope when unblocked:** ~30 minutes of mechanical sed-style work + one new test asserting the legacy 503 path is gone.

---

### 1.2 C3 — `rvoip-websocket` media plane (webrtc-rs integration)

**Landed 2026-05-25.** `WebRtcMediaBridge` is fully wired under the `media-webrtc` feature; end-to-end WS↔WS bridge proof at [`crates/rvoip-websocket/tests/ws_bridge_flow.rs`](../rvoip-websocket/tests/ws_bridge_flow.rs). See [`UCTP_GAP_PLAN.md`](UCTP_GAP_PLAN.md) for follow-ups.

---

## 2. Cross-ecosystem (scope-creep decision)

### 2.1 D5 — Browser smoke automation (Playwright / headless Chrome)

**Why not done:** would add Node.js + npm + Playwright to a Rust-only workspace. That's a meaningful CI/CD shape change — `cargo` no longer fully describes the build, and contributors need a Node toolchain installed. Worth deciding deliberately rather than tucking into a "polish" PR.

**Current state:** manual browser smoke kit ships at [`examples/uctp_to_sip_bridge/browser/`](examples/uctp_to_sip_bridge/browser/) with `index.html`, `agent.js`, `ws_smoke.html`, `ws_agent.js`, and a README explaining the Chrome flags needed for the self-signed cert. A human can reproduce the smoke in ~2 minutes.

**What unblocks:** decision on whether to add Node infrastructure to the workspace.

**Scope when unblocked:** ~200 LOC of Playwright (`smoke.mjs`) that:
1. Launches headless Chrome with `--ignore-certificate-errors-spki-list=<sha256>` flag.
2. Opens `index.html` against a local orchestrator the test spawns.
3. Polls `localStorage` for the `auth.session` confirmation.
4. Exits 0/1.

Plus `package.json`, `.github/workflows/browser-smoke.yml`, and documentation. Not technically hard; the gate is the dependency-discipline decision.

---

## 3. Substantial standards-track work (separate sessions each)

### 3.1 C4 remaining — AAuth (IETF actor-authentication)

**Why not done:** AAuth is its own IETF standards-track draft (`draft-ietf-aauth-*`) covering actor/subject token attestation — the "I am acting on behalf of user X" pattern. It needs:

1. A new validator (`AAuthValidator`) that parses AAuth's actor+subject claim shape.
2. UCTP envelope flow for the actor-claim assertion (peer sends actor token alongside their access token).
3. Mapping to `IdentityAssurance::UserAuthorized { user_id, identity, scopes }` where `user_id` is the *subject* and `identity` is the *actor*.

The existing `IdentityAssurance::UserAuthorized` variant *already has the right shape* for AAuth (note the distinct `user_id` and `identity` fields). The validator is what's missing.

**Scope:** ~500 LOC + integration tests. Independent of any other in-flight work.

**Reference:** `rvoip_core::identity::IdentityAssurance::UserAuthorized` already carries the distinction. Plan §1.4's "RFC 9421 / DPoP / AAuth backends" row.

---

### 3.2 C4 remaining — RFC 9421 HTTP Message Signatures

**Why not done:** RFC 9421 defines per-request signatures over HTTP message components. Applying it to UCTP envelopes requires CONVERSATION_PROTOCOL.md to first define which envelope fields are part of the signature base — that spec coordination hasn't happened.

Without the canonical-fields spec, any implementation would lock in choices that future deployments couldn't interop with.

**What unblocks:**

1. CONVERSATION_PROTOCOL.md PR defining: which envelope fields participate in the signature base, the canonical serialization, and the signature header format.
2. *Then* implementation in `auth-core::sig9421` (~600-800 LOC including key resolution, canonicalization, signature verification, replay protection).

**Reference:** RFC 9421. Plan §1.4 lumps this with DPoP/AAuth in the "real auth backends" row. DPoP and AAuth landed/are queued; RFC 9421 needs the spec gate first.

---

### 3.3 C2 remaining — Full RFC 4733 / RFC 2833 audio-pipeline integration

**Why not done as full integration:** would require adding `payload_type: Option<u8>` to `rvoip_core::stream::MediaFrame` so the bridge's frame-pump can distinguish telephone-event RTP packets from audio frames per-packet. That touches **70+ MediaFrame construction sites** across `rvoip-core`, `rvoip-uctp`, `rvoip-quic`, `rvoip-webtransport`, `rvoip-websocket`, `rvoip-media-core`, `rvoip-rtp-core`, and `rvoip-webrtc` (the latter off-limits). A mechanical change but invasive and risky to land as a "by the way" item.

**What landed in this session as a partial:** the `frame_pump` now detects 4-byte transcode failures (the RFC 4733 telephone-event size) and passes the frame through instead of dropping. Metric: `rvoip_bridge_dtmf_passthrough_total`. This prevents DTMF events from getting silently dropped at the SIP↔UCTP bridge boundary but doesn't convert them to UCTP `dtmf.send` envelopes — they pass through as opaque audio frames the receiving end has to route by PT itself.

**What unblocks:** dedicated session to:

1. Add `payload_type: Option<u8>` to `MediaFrame`; update all 70+ construction sites.
2. In the SIP-side adapter's RTP reader, set `payload_type` per-packet from the RTP header.
3. In `frame_pump`, route PT=101 (or the negotiated telephone-event PT) to a separate sink that emits `UctpSessionEvent::Dtmf` on the bridged UCTP side.
4. Reverse direction: UCTP `dtmf.send` → synthesize RFC 4733 RTP packets on the SIP side.

**Scope:** ~300-400 LOC including the 70-site touch, plus end-to-end tests for SIP→UCTP and UCTP→SIP DTMF flow.

**Reference:** RFC 4733 + RFC 2833 (predecessor). Plan §1.4's "DTMF, quality reports" row. The signaling side of DTMF + Quality already landed (plan §13.6 / §13.7).

---

## 4. Summary table

| ID | Item | Category | Why | Owner / unblock |
|---|---|---|---|---|
| **B3** | Spec codes 501 / 505 | Externally blocked | Spec PR needed; refusing to land non-canonical codes | CONVERSATION_PROTOCOL.md maintainer |
| **D5** | Playwright browser smoke | Cross-ecosystem | Adds Node.js to a Rust workspace; deliberate decision | Workspace-policy decision |
| **C4 AAuth** | AAuth validator | Substantial standards | ~500 LOC IETF standards-track work | Queued |
| **C4 RFC 9421** | HTTP Message Signatures | Externally blocked (spec) + Substantial | Spec PR needed first to define canonical fields | CONVERSATION_PROTOCOL.md maintainer, then ~600-800 LOC |
| **C2 audio full** | Per-frame RTP PT in MediaFrame | Substantial cross-crate refactor | 70+ construction sites; partial DTMF passthrough already landed | Queued |

Nothing else from the original UCTP_IMPLEMENTATION_PLAN.md (v0 spike + v0.x production-hardening) remains open as of this writing.

---

## 5. Where to look next

- **Completed work:** [UCTP_IMPLEMENTATION_PLAN.md §11](UCTP_IMPLEMENTATION_PLAN.md#11-v0-spike--what-shipped) (v0 spike) and [§13](UCTP_IMPLEMENTATION_PLAN.md#13-v0x--production-hardening-track) (this session's work).
- **Architectural concerns inventory:** [UCTP_IMPLEMENTATION_PLAN.md §7](UCTP_IMPLEMENTATION_PLAN.md#7-known-tensions--gaps-to-revisit-after-v0) — production concerns from the original audit. All actionable items in that list are now closed.
- **Spec source of truth:** `../rvoip-core/CONVERSATION_PROTOCOL.md`.
