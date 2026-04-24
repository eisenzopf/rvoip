# Architectural follow-ups: media-plane / signaling layering

Working notes captured 2026-04-24 after Sprint 2 closed (A5 Phase 2c + B4
DTMF wiring) and three new examples landed (`streampeer/dtmf` round-trip
assertion, `streampeer/tls`, `streampeer/srtp`).

Building those examples surfaced several pre-existing layering gaps that
were silently accommodated rather than fixed. Everything that shipped
works, but the cleaner designs deserve recording so the next contributor
(or the next "why is this like this?" moment) has a written answer.

**Nothing here is a regression or a must-fix.** Each item notes the
cost/benefit so the tradeoff stays explicit; any of them can be deferred
until real-world pain justifies the work.

Cross-references:
- `GENERAL_PURPOSE_SIP_CLIENT_PLAN.md` — Sprint 1/2 work log and remaining roadmap.
- `ARCHITECTURE_OVERVIEW.md` — current state-machine + adapter topology.
- `RFC_COMPLIANCE_STATUS.md` — method/header matrix.

---

## Priority 1 — Formalize the media-plane / state-machine boundary

**Problem.** The session-core state machine is YAML-driven and models
dialog/session state transitions. Media-plane side effects (DTMF send,
mute, audio frame push) don't fit that model — they're in-call actions
that don't change observable session state. Today the system is
inconsistent about this:

- Rust `Action::SendDTMF(char)`, `Action::SendDTMFTone`, and
  `EventType::SendDTMF { digits }` all exist in the state-table types,
  but the YAML file has **zero** matching transitions or action entries.
- `UnifiedCoordinator::send_dtmf` used to dispatch
  `EventType::SendDTMF` through the state machine, where it was silently
  dropped (no transition fired). This was fixed 2026-04-24 by bypassing
  the state machine and calling `MediaAdapter::send_dtmf_rfc4733`
  directly.
- Several other actions show up in the state-table load-time log as
  `Unknown action 'X', treating as custom`: `MuteLocalAudio`,
  `UnmuteLocalAudio`, `DestroyMediaBridge`, `UnlinkSessions`,
  `SendPUBLISH`. Each is dead plumbing waiting to confuse a future
  contributor.

**The decision to make.** One of these two, explicitly:

1. **Media-plane bypasses the state machine** — DTMF send, mute, audio
   push, etc. invoke `MediaAdapter` directly from `UnifiedCoordinator`.
   Document the rule: *"state-machine events are for things that change
   dialog/session state; direct adapter calls are for in-call side
   effects."* Delete the dead `Action::SendDTMF` / `EventType::SendDTMF`
   variants.
2. **Every user-visible control routes through the state machine** —
   add the SendDTMF YAML transition (self-loop on Active) + wire the
   action parser to `Action::SendDTMF(char)` with parameter extraction.
   More machinery, but uniform.

**Recommendation: option 1.** It matches the existing hold/resume
pattern, keeps the state table focused on what it models well, and DTMF
semantically has no state to track. The rule should be documented in
`ARCHITECTURE_OVERVIEW.md` so future contributors don't re-add
`EventType::SendMute` and wonder why nothing fires.

**Files:**
- `crates/session-core/src/state_table/types.rs` — remove
  `Action::SendDTMF`, `Action::SendDTMFTone`, `EventType::SendDTMF`.
- `crates/session-core/src/state_machine/actions.rs:313-328` — remove
  the `SendDTMF` / `SendDTMFTone` match arms (dead after enum removal).
- `crates/session-core/docs/ARCHITECTURE_OVERVIEW.md` — add a short
  "Media-plane side effects" section codifying the rule.

**Validation nice-to-have.** At state-table load time, assert that
every Rust `Action` variant is either (a) referenced by at least one
YAML transition or (b) present on an explicit allow-list of
"direct-call actions." Surfaces drift immediately instead of at
first-call-that-breaks. Roughly 30 LOC in the YAML loader.

---

## Priority 2 — Unify the two SDP-offer generation paths

**Problem.** `MediaAdapter` today has both:

- `generate_local_sdp` — hardcoded `format!()` with `m=audio … RTP/AVP …`.
- `generate_sdp_offer` — typed `SdpBuilder` with SRTP awareness.

The state-machine `Action::GenerateLocalSDP` calls the first. The SRTP
integration test implicitly took the lenient path that accepts plain
offers (it didn't set `srtp_required`), which is why the divergence
wasn't caught sooner. The 2026-04-24 fix patched `generate_local_sdp`
to delegate to `generate_sdp_offer` when `offer_srtp = true`, which
works but leaves two methods in the public surface with overlapping
responsibilities.

**The fix.** One `generate_local_sdp` that always goes through
`SdpBuilder`. Crypto attributes attached conditionally on the SRTP
policy. The legacy format-string path disappears.

**Why it matters.** Every future SDP feature — ICE candidates
(RFC 8445), DTLS fingerprints (RFC 5763), video m-lines (RFC 6184),
comfort noise (RFC 3389) — will need to be added to exactly one place.
Today you'd have to remember to touch both paths or they drift again.

**Files:**
- `crates/session-core/src/adapters/media_adapter.rs:800-856` —
  collapse `generate_local_sdp` into a pure dispatch to the builder,
  delete the format string.
- `crates/session-core/src/adapters/media_adapter.rs:247`
  `generate_sdp_offer` — becomes the sole implementation.

---

## Priority 3 — RFC 4733 conformance gaps on the DTMF send path

Two related items, both stemming from the single-packet shortcut taken
when the send path was first implemented.

### 3a. Timestamp coherence

RFC 4733 §2.1 specifies that telephone-event timestamps "mark the time
the tone begins, as observed in the audio stream," and §2.3 requires
retransmits to share that timestamp. The current sender derives the
outbound RTP timestamp from wall-clock milliseconds × 8, completely
independent of the audio RTP session's timestamp cursor. Since DTMF
rides on the same SSRC as audio (we reuse the existing `rtp_session`),
compliant peers will see a non-monotonic timestamp jump when a DTMF
packet interleaves with the audio stream.

### 3b. §2.5.1.3 retransmits

The current sender emits one packet per digit with `E=1`. The spec
wants:

1. One packet with `E=0` at tone start.
2. Continuation packets every 20 ms during the tone (incrementing
   `duration`, keeping the timestamp fixed).
3. Three final packets with `E=1` for loss resilience.

The single-packet shape works on localhost UDP because loopback drops
zero packets, but on a real network the only copy can be lost and the
digit disappears.

### Fix

- Expose a "current RTP timestamp" accessor on `RtpSession` so
  `send_dtmf_packet` can anchor on the audio transmitter's position
  instead of wall clock.
- Introduce a `DtmfTransmitter` helper (new module in
  `media-core/src/relay/`) that owns the `tokio::spawn`'d schedule:
  first packet → N continuation packets → 3 end-of-event packets, with
  correct timestamp/duration math.
- Caller-facing API stays the same:
  `MediaAdapter::send_dtmf_rfc4733(session_id, digit, duration_ms)`
  becomes non-blocking, returns a handle, and the transmitter does the
  real work asynchronously.

**Cost:** ~150 LOC new. **Benefit:** RFC-compliant behavior on any
transport, not just zero-loss UDP loopback.

**Files:**
- `crates/rtp-core/src/session/mod.rs` — add
  `fn current_timestamp(&self) -> RtpTimestamp` accessor.
- `crates/media-core/src/relay/controller/dtmf_transmitter.rs` — new
  module implementing the §2.5.1.3 schedule.
- `crates/media-core/src/relay/controller/rtp_management.rs:send_dtmf_packet`
  — becomes a thin wrapper that delegates to the transmitter.

---

## Priority 4 — Move RFC 4733 retransmit dedup into rtp-core

**Problem.** The three-packet retransmit dedup currently lives in
`media-core/src/relay/controller/mod.rs` inside
`spawn_rtp_event_handler`. That's one layer too high — RFC 4733
retransmit semantics are a protocol detail that belongs next to the PT
101 decode. Media-core shouldn't have to know about §2.5.1.3; it should
just receive "one logical digit per tone."

**The fix.** Two options:

1. Dedup inline in `rtp-core/src/transport/udp.rs` at the PT 101 branch;
   emit `RtpEvent::DtmfEvent` only on the first-seen `(ssrc, timestamp)`
   pair.
2. Split the event: `RtpEvent::DtmfFrame` (every packet, no dedup) +
   `RtpEvent::DtmfDigit` (deduped, keyed on timestamp). Consumers pick
   by subscription. Useful if some future consumer needs the raw frames
   (e.g. low-level debugging or a protocol-compliance analyzer).

Recommendation: option 1 unless option 2's consumer materializes.

**Files:**
- `crates/rtp-core/src/transport/udp.rs` — add a
  `DashMap<(SocketAddr, u32, u32), Instant>` for
  `(peer, ssrc, timestamp)` seen-set; evict old entries via TTL (e.g.
  500 ms — longer than any plausible retransmit window).
- `crates/media-core/src/relay/controller/mod.rs:825-960` — remove the
  `dtmf_last_delivered` tracking; handler becomes a straight forwarder.

---

## Priority 5 — Collapse `MediaSessionId` and `DialogId`

**Problem.** `DialogId` (in media-core) and `MediaSessionId` (in
session-core) are both `String` wrappers that currently carry the same
value, formatted the same way (`media-{session_id}`). Code reconstructs
one from the other via `DialogId::new(media_id.0.clone())`.

It's an abstraction that adds no safety (both are `String`), confuses
readers, and invites bugs — the `Action::SendDTMF` UUID regression
found 2026-04-24 was exactly this: the action code called
`MediaSessionId::new()` which generated a fresh UUID that didn't map to
any real dialog.

**The fix.** One identifier. Either:

- Have media-core depend on a shared identifier crate (session-core's
  `SessionId`, or a small `rvoip-core-ids` crate if crate-dependency
  direction is the sticking point).
- Or accept that media-core is the lower layer and have session-core
  alias `MediaSessionId = rvoip_media_core::DialogId`.

**Cost:** surface-wide rename. Not trivial but mechanical.

---

## Priority 6 — `tls_insecure_skip_verify` as a Cargo feature

**Problem.** The TLS example sets `tls_insecure_skip_verify = true`
because a dev-generated self-signed cert isn't in the system trust
store. That knob is also on `Config` for production code. Nothing
prevents a production deployment from flipping it by accident — a
`let mut config = Config::default(); config.tls_insecure_skip_verify =
true;` one-liner silently disables certificate validation across all
TLS traffic.

**The fix.** Move it behind a Cargo feature `dev-insecure-tls` (off by
default). Production builds physically can't set it. Examples opt in
via `--features dev-insecure-tls` in their cargo invocation. The field
becomes `#[cfg(feature = "dev-insecure-tls")]` on `Config` (and setters
conditional on the same feature).

**Cost:** ~20 LOC + a couple of `#[cfg(...)]` gates. **Benefit:**
defense-in-depth against a real security footgun. Worth it even
though it's an ergonomic regression in the example path.

---

## Out of scope for this doc

- The `UnifiedCoordinator::send_dtmf` bypass itself is a defensible
  tradeoff once Priority 1 is explicit about the rule. It does not need
  to change.
- The three new examples (dtmf / tls / srtp) are fine as shipped; no
  revisit needed.
- Sprint 3 items (A6 STUN, A7 digest, C1 Comfort Noise, C2 SDP
  offer/answer matching helper) remain the next normal roadmap work,
  ahead of this refactor list.

---

## Recommended sequencing

If/when any of this is picked up, the cleanest order is:

1. **Priority 1** first — documenting the architectural rule + removing
   dead code. Small, unblocks clean reasoning about everything else.
   Roughly 50 LOC plus the ARCHITECTURE_OVERVIEW.md addendum.
2. **Priority 2** next — unify SDP generation paths before Sprint 3
   adds ICE/DTLS/video features that would otherwise double the surface
   area.
3. **Priority 3** when a real carrier deployment is imminent — the
   retransmit shape matters most on lossy networks.
4. Priorities 4, 5, 6 are good-hygiene items; pull them in when
   touching adjacent code.

Or, since none of this is load-bearing today, defer the whole list
until a deployment actually surfaces pain. That is also a reasonable
answer.

---

## Verification (if executed)

Per-item verification lives with the item's implementation, but the
system-level regression check is:

```bash
cargo test -p rvoip-dialog-core --lib
cargo test -p rvoip-media-core --lib
cargo test -p rvoip-rtp-core --lib
cargo test -p rvoip-session-core --lib

cargo test -p rvoip-session-core \
    --test audio_roundtrip_integration \
    --test tls_call_integration \
    --test srtp_call_integration \
    --test register_flow_test

./crates/session-core/examples/run_all.sh
```

All must stay green. The example harness is the strongest end-to-end
signal — if `streampeer/dtmf` still asserts "5/5 digits round-tripped",
`streampeer/tls` still observes a TLS-transported INVITE, and
`streampeer/srtp` still negotiates SDES, the refactor hasn't broken the
happy paths.
