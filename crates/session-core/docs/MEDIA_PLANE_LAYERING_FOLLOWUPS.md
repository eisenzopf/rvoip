# Sprint 2.5 — Architectural Hygiene (Plan B)

A full-cleanup playbook for the six architectural follow-ups surfaced
during Sprint 2's DTMF/TLS/SRTP example work. Executed in sequence,
this takes roughly **~1.5 weeks of focused work** and lands us on a
clean foundation for Sprint 3 (media polish) and Sprint 4 (WebRTC).

**Insertion point in the roadmap** (see
`GENERAL_PURPOSE_SIP_CLIENT_PLAN.md`): this slots between Sprint 2
(closed 2026-04-23) and Sprint 3 as a net-new **Sprint 2.5** row:

| Sprint | Items | Outcome |
|--------|-------|---------|
| 2 ✅ | A4/A5/B3/B4/B5 | Production-NAT + registration robustness |
| **2.5** | **P1 + P6 + P5 + P2 + P4 + P3 (this doc)** | **Clean architectural base for Sprints 3/4** |
| 3 | A6 + A7 + C1 + C2 | Media polish |
| 4 | D1-D5 | WebRTC (opt-in) |

Sprint 2.5 produces no user-visible features. It exists so Sprint 3/4
can land on consistent internal boundaries — the Priority 2 (unified
SDP) work in particular is load-bearing for Sprint 3 C2 and every
Sprint 4 item.

---

## Context — why we're doing this now

Sprint 2's three new examples (`streampeer/dtmf` round-trip,
`streampeer/tls`, `streampeer/srtp`) exposed pre-existing layering
gaps that had been silently accommodated. Six distinct items were
identified, all documented below. Everything currently shipped works,
but:

1. **Sprint 3 C2 (SDP offer/answer matching helper) cannot cleanly
   land** without first resolving the dual-path SDP generation (P2) —
   otherwise C2 either builds against the wrong path or adds a third.
2. **Sprint 4 D2/D3/D4 (DTLS-SRTP, ICE, TURN) compound the problem** —
   every new SDP attribute family would have to be added to both paths.
3. **DTLS-SRTP (D2) becomes cleaner with `tls_insecure_skip_verify`
   behind a feature flag (P6)** — the production threat model is
   simpler.
4. **`MediaSessionId`/`DialogId` duplication (P5) already caused one
   regression** (the `Action::SendDTMF` fresh-UUID bug). Future
   additions will keep tripping on it.
5. **DTMF send path is not RFC-conformant on lossy networks (P3)** —
   single-packet shape works on localhost only.

Plan B's bet: **pay down the debt once, in order, before Sprint 3/4
compound it**. The alternative (Plan A — minimum viable: P1+P2+P6
only) leaves P3/P4/P5 for later and accepts the technical debt; Plan
C (defer everything) gets Sprint 3 started immediately but guarantees
we revisit this ground later under pressure.

---

## Execution order & rationale

```
Phase 1 ─ P1 — Formalize media-plane / state-machine boundary     (~1 day)
Phase 2 ─ P6 — tls_insecure_skip_verify as Cargo feature           (~3 hrs)
Phase 3 ─ P5 — Collapse MediaSessionId and DialogId                (~1-2 days)
Phase 4 ─ P2 — Unify SDP-offer generation paths                   (~1-2 days)
Phase 5 ─ P4 — Move RFC 4733 retransmit dedup into rtp-core        (~4 hrs)
Phase 6 ─ P3 — RFC 4733 timestamp + retransmit conformance         (~2-3 days)
```

**Why this order:**

- **P1 first** — zero-risk cleanup (deletion + docs), builds
  confidence, removes confusing dead code before we refactor.
- **P6 early** — small isolated security fix that future phases don't
  need to worry about.
- **P5 mid-early** — biggest mechanical rename; doing it on a stable
  base (just after dead-code deletion) means P2/P3 write clean code
  from the start instead of writing-then-renaming.
- **P2 before P3/P4** — the SDP unification touches everything DTMF
  advertises on the offer; P3's new DTMF transmitter wants one
  code path to emit into.
- **P4 before P3** — relocating the dedup first means P3's new sender
  doesn't need to worry about receive-side dedup semantics.
- **P3 last** — biggest feature addition, lands on clean foundation
  built by the other five.

---

## Phase 1 — P1: Formalize the media-plane / state-machine boundary

**Goal.** Make it an explicit, documented architectural rule that
media-plane side effects (DTMF send, mute, audio push, etc.) bypass
the state machine and call `MediaAdapter` directly. Delete the dead
Rust enum variants that falsely suggest otherwise.

**Prerequisites.** None. Pure deletion + documentation.

### Step 1.1 — Audit dead variants

Run a coverage check manually. The YAML loader already logs
`Unknown action 'X', treating as custom` at load time — capture that
list to confirm what needs deletion. Expected offenders:

- `Action::SendDTMF(char)`
- `Action::SendDTMFTone`
- `EventType::SendDTMF { digits }`
- `Action` variants: `MuteLocalAudio`, `UnmuteLocalAudio`,
  `DestroyMediaBridge`, `UnlinkSessions`, `SendPUBLISH` (per load-time
  log output observed during Sprint 2 testing)

Command:
```bash
RUST_LOG=debug cargo test -p rvoip-session-core --lib 2>&1 \
    | grep "Unknown action" | sort -u
```

### Step 1.2 — Delete dead enum variants

**File:** `crates/session-core/src/state_table/types.rs`

Remove:
- `Action::SendDTMF(char)` and `Action::SendDTMFTone`
- `EventType::SendDTMF { digits: String }`
- Any accompanying `EventType::canonical_form` map entry for `SendDTMF`

**File:** `crates/session-core/src/state_machine/actions.rs` (lines
313–328)

Remove the `Action::SendDTMF(digit) => { … }` and
`Action::SendDTMFTone => { … }` match arms now that the enum variants
are gone. The compiler will surface any other references.

Also audit and delete any dead references in:
- `crates/session-core/src/state_table/yaml_loader.rs` — check
  `parse_action_by_name` for any SendDTMF mapping.

### Step 1.3 — Document the rule

**File:** `crates/session-core/docs/ARCHITECTURE_OVERVIEW.md`

Add a new section `## Media-plane side effects` near the state-machine
overview. Content template:

> Not every user-facing API is a state transition. Actions that are
> pure media-plane side effects — sending DTMF, muting a stream,
> pushing an audio frame, starting/stopping recording at the packet
> level — are invoked **directly** on the relevant adapter from
> `UnifiedCoordinator` rather than dispatched as an `EventType`
> through the state machine.
>
> Rule of thumb: if the action doesn't change any observable session
> state (the peer's SIP state machine wouldn't care that it happened),
> it bypasses the state machine. Examples:
>
> - ✅ state-machine: INVITE, BYE, REFER, re-INVITE (hold/resume),
>   REGISTER, 200 OK accept/reject — all change dialog state.
> - ✅ direct-call: DTMF send (RFC 4733), mute, audio push, recording
>   control — media-plane only.
>
> **Contract.** Any new `EventType` variant added to
> `state_table/types.rs` MUST have at least one matching YAML
> transition or be listed in `KNOWN_DIRECT_EVENTS`. Violating this
> contract is caught by the validation pass described below (Step
> 1.4).

### Step 1.4 — Add load-time validation

**File:** `crates/session-core/src/state_table/yaml_loader.rs`

After the YAML is loaded and parsed, iterate `Action::variants()` (use
strum or a hand-maintained list) and assert every variant is either:

- Referenced by at least one YAML transition's `actions:` list, or
- Present in an explicit `KNOWN_DIRECT_CALL_ACTIONS` allow-list (for
  actions that intentionally exist as Rust-only direct-call methods).

Fail loudly with a descriptive error at load time if a variant is
neither referenced nor on the allow-list. This surfaces drift
immediately rather than at the first call site that breaks.

~30 LOC. New test:
`crates/session-core/src/state_table/tests.rs` — add
`test_all_actions_covered_or_on_allowlist`.

### Verification gate 1

```bash
cargo build -p rvoip-session-core
cargo test -p rvoip-session-core --lib
cargo test -p rvoip-session-core --test '*'
./crates/session-core/examples/run_all.sh
```

Expected: all green. Zero `Unknown action` warnings at state-table
load time.

**LOE estimate:** ~1 day including the validation pass.

**Rollback:** `git revert` the phase commit. Deletions are pure
subtraction; the validation pass is additive.

---

## Phase 2 — P6: `tls_insecure_skip_verify` as a Cargo feature

**Goal.** Remove the production footgun where
`Config::tls_insecure_skip_verify = true` silently disables TLS
certificate validation. Gate it behind an opt-in Cargo feature so
production builds physically can't set it.

**Prerequisites.** Phase 1 complete.

### Step 2.1 — Add the feature to session-core

**File:** `crates/session-core/Cargo.toml`

```toml
[features]
default = []
event-history = []
persistence = ["sqlx"]
dev-insecure-tls = []   # NEW — gates tls_insecure_skip_verify
```

### Step 2.2 — Gate the Config field

**File:** `crates/session-core/src/api/unified.rs`

```rust
pub struct Config {
    // … existing fields …

    #[cfg(feature = "dev-insecure-tls")]
    pub tls_insecure_skip_verify: bool,
}
```

Gate all reads/writes of `tls_insecure_skip_verify` under the same
`#[cfg(feature = "dev-insecure-tls")]` attribute. The
`Config::local(...)` / `Config::on(...)` constructors emit the field
only under the feature. Where the field is consumed downstream (the
multi-line transport builder in `UnifiedCoordinator::new`), gate the
consumption the same way.

### Step 2.3 — Thread through sip-transport

**File:** `crates/sip-transport/Cargo.toml`

Add a mirrored feature `dev-insecure-tls` (off by default). Forward it
from session-core: `rvoip-sip-transport = { workspace = true, features
= ["dev-insecure-tls"] }` conditionally when session-core's feature is
enabled.

**File:** `crates/sip-transport/src/transport/tls/mod.rs`

The `bind_with_client_config` path that accepts a
`CertificateVerifier` that accepts-all: gate behind
`#[cfg(feature = "dev-insecure-tls")]`. Under the default build, the
insecure verifier can't even be constructed.

### Step 2.4 — Update example harness

**File:** `crates/session-core/examples/streampeer/tls/run.sh`

Every `cargo run` and `cargo build` invocation for the TLS example
binaries needs `--features dev-insecure-tls`:

```bash
cargo build -p rvoip-session-core \
    --example streampeer_tls_server \
    --example streampeer_tls_client \
    --features dev-insecure-tls

cargo run -p rvoip-session-core \
    --example streampeer_tls_server \
    --features dev-insecure-tls \
    --quiet
```

**File:** `crates/session-core/examples/streampeer/tls/server.rs` and
`client.rs`

Gate the `config.tls_insecure_skip_verify = true;` line with
`#[cfg(feature = "dev-insecure-tls")]` at compile time — use a
`compile_error!` fallback under the default build so someone building
the example without the flag gets a clear message.

### Step 2.5 — Update the TLS integration test

**File:** `crates/session-core/tests/tls_call_integration.rs`

Gate the entire test body with `#[cfg(feature = "dev-insecure-tls")]`
or mark it `#[cfg_attr(not(feature = "dev-insecure-tls"), ignore)]`.
Update `Cargo.toml` to pass the feature when running tests:

```bash
cargo test -p rvoip-session-core \
    --features dev-insecure-tls \
    --test tls_call_integration
```

### Verification gate 2

```bash
# Default build: Config should not expose the field at all.
cargo build -p rvoip-session-core
cargo test -p rvoip-session-core --lib

# Feature build: TLS test passes.
cargo test -p rvoip-session-core --features dev-insecure-tls --test tls_call_integration

# Example still works.
./crates/session-core/examples/streampeer/tls/run.sh

# Grep check: no reference to tls_insecure_skip_verify exists in any
# non-test, non-cfg-gated code path.
grep -rn "tls_insecure_skip_verify" crates/session-core/src \
    | grep -v "cfg.*dev-insecure-tls"
# Expected: zero matches.
```

**LOE estimate:** ~3 hours.

**Rollback:** Single-commit revert. No cross-crate side effects.

---

## Phase 3 — P5: Collapse `MediaSessionId` and `DialogId`

**Goal.** One identifier across media-core and session-core for a
media session / dialog. Eliminates the
`DialogId::new(media_id.0.clone())` reconstruction pattern and the
class of bugs that comes with two `String` wrappers carrying the
same value.

**Prerequisites.** Phases 1–2 complete (stable base for the
wide-surface rename).

### Step 3.1 — Decide on the canonical type

**Recommendation:** keep `rvoip_media_core::DialogId` as the single
type. Rationale:
- media-core already exports it and is the lower layer in the stack.
- session-core currently has TWO ID wrappers in this space
  (`MediaSessionId`, `state_table::DialogId`). Both boil down to the
  same string value.
- Replacing `MediaSessionId` + `state_table::DialogId` with an alias
  to `rvoip_media_core::DialogId` collapses three types into one with
  no churn on media-core.

**Alternative considered:** move the type to a new `rvoip-core-ids`
crate. Rejected — adds a crate for a single type; the one-way
dependency (media-core → core-ids ← session-core) trades one problem
for another.

### Step 3.2 — Introduce the type alias in session-core

**File:** `crates/session-core/src/state_table/types.rs`

```rust
// Was: pub struct MediaSessionId(pub String);
// Was: pub struct DialogId(pub uuid::Uuid);

/// Session-core's identifier for a media session is exactly media-core's
/// DialogId. Aliased here for ergonomic reasons (we can still `use
/// crate::types::MediaSessionId`) but the underlying type is unified.
pub type MediaSessionId = rvoip_media_core::DialogId;

/// Session-core's dialog identifier is a newtype over the same string
/// shape media-core uses, so conversions are trivial.
pub type DialogId = rvoip_media_core::DialogId;
```

**But wait** — session-core's `DialogId` is currently `uuid::Uuid`.
Changing it to a `String` wrapper is a wider semantic change. So the
full plan needs two sub-steps:

**Step 3.2a** — Migrate callers of `state_table::DialogId` that rely
on `uuid::Uuid` API (e.g. `.to_string()`, `Hash` impls) to the new
string-wrapper shape. Most call sites just need `.to_string()` where
it already exists; a few will need `DialogId::from(Uuid::new_v4())`
helpers.

**Step 3.2b** — Replace every `MediaSessionId::new()` call site with
the correct `DialogId`-derived value. This is the fix for the
`Action::SendDTMF` UUID bug generalized.

### Step 3.3 — Surface-wide rename

Use `cargo check` as a driver:

```bash
cargo check --workspace 2>&1 | grep "^error" > /tmp/rename_errors.log
```

Fix each error. The expected failure modes:
- Mismatched `uuid::Uuid` arguments → replace with `DialogId::new_v4()`
  (a convenience constructor we should add on the aliased type).
- `MediaSessionId(string).0` access → replace with
  `.as_str()` or accessor method consistent with media-core's shape.
- `from_dialog(&dialog_id)` converters — these should be no-ops now
  (they're converting between two names for the same thing). Delete
  and inline.

**Files expected to change (~50-70 total):**
- `crates/session-core/src/` — every module that uses either type
  (~30 files based on `grep -r`).
- `crates/session-core/tests/` — any test constructing
  `MediaSessionId` directly.
- `crates/session-core/examples/` — should just compile once the
  source tree does; no example directly constructs `MediaSessionId`.

### Step 3.4 — Delete convenience converters

**File:** `crates/session-core/src/state_table/types.rs`

Delete `impl From<&state_table::DialogId> for rvoip_media_core::DialogId`
and similar converter impls — they're no-ops after the alias.

**File:** `crates/session-core/src/adapters/media_adapter.rs`

Delete any `MediaSessionId::from_dialog(&dialog_id)` call — the
function is now the identity. Tag those functions `#[deprecated]` for
one release if cross-crate dependencies need the staged removal.

### Step 3.5 — Add a `new_v4()` constructor to media-core

**File:** `crates/media-core/src/types/mod.rs`

```rust
impl DialogId {
    /// Generate a fresh dialog id with a UUIDv4 suffix. Matches the
    /// format session-core historically used for
    /// `MediaSessionId::new()`.
    pub fn new_v4() -> Self {
        Self(format!("media-{}", uuid::Uuid::new_v4()))
    }
}
```

### Verification gate 3

```bash
cargo build --workspace
cargo test -p rvoip-media-core --lib
cargo test -p rvoip-dialog-core --lib
cargo test -p rvoip-session-core --lib
cargo test -p rvoip-session-core --test '*' --features dev-insecure-tls
./crates/session-core/examples/run_all.sh
```

Expected: all green. Critically, the `streampeer/dtmf` round-trip
must still report "5/5 DTMF digits round-tripped" — the ID-rename
regression surface is exactly this round-trip path.

**Grep check:**
```bash
# No remaining ::from_dialog conversions in session-core production code.
grep -rn "::from_dialog\|from_dialog(&" crates/session-core/src
```

**LOE estimate:** 1-2 days. The mechanical rename is fast; the
semantic change from `Uuid`-based `DialogId` to `String`-based has a
handful of non-trivial callers (Hash impls, Display impls).

**Rollback:** Single large commit. If the rename breaks something
subtle and the fix isn't obvious within an hour, revert and redo
with a different strategy (e.g. keep the Uuid form of DialogId and
introduce a separate `MediaDialogId` type for the media layer).

---

## Phase 4 — P2: Unify SDP-offer generation paths

**Goal.** Collapse `MediaAdapter::generate_local_sdp` and
`generate_sdp_offer` into a single method that uses `SdpBuilder` for
all cases and attaches crypto / ICE / DTLS attributes conditionally
on config policy.

**Prerequisites.** Phases 1-3 complete. Clean type surface and no
dead state-machine actions interfering with the caller audit.

### Step 4.1 — Capability audit

Read the current `generate_sdp_offer` (media_adapter.rs:247-...) and
enumerate everything it does that `generate_local_sdp` doesn't:

- [ ] RTP/SAVP profile when `offer_srtp`
- [ ] `a=crypto:` per suite via `SrtpNegotiator::new_offerer`
- [ ] `pending_srtp_offerers` state stash
- [ ] Uses `SdpBuilder` for composable construction

And what `generate_local_sdp` does that `generate_sdp_offer` doesn't:

- [ ] `media_sessions.insert()` side effect (caching the info)
- [ ] Hardcoded RTPMAP for PCMU + PCMA both (generate_sdp_offer only
      emits PCMU — verify)
- [ ] `a=rtpmap:101 telephone-event/8000` + `a=fmtp:101 0-15` DTMF
      attributes

### Step 4.2 — Write the unified method

**File:** `crates/session-core/src/adapters/media_adapter.rs`

Rewrite `generate_local_sdp` to be the sole implementation. Outline:

```rust
pub async fn generate_local_sdp(&self, session_id: &SessionId) -> Result<String> {
    let dialog_id = self.resolve_dialog(session_id)?;
    let info = self.cache_session_info(&dialog_id).await?;
    let local_port = info.rtp_port.unwrap_or(info.config.local_addr.port());

    // Base offer: PCMU + PCMA + telephone-event (RFC 4733).
    let mut builder = SdpBuilder::new()
        .origin(&session_id.0, /* … */)
        .connection(/* … */)
        .time("0", "0");

    // Pick the profile + optional crypto attrs based on config.
    let (profile, crypto_attrs) = self.select_profile_and_crypto(session_id)?;

    let mut media = builder
        .media_audio(local_port, profile)
        .formats(&["0", "8", "101"])
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .rtpmap("101", "telephone-event/8000")
        .fmtp("101", "0-15");

    for attr in crypto_attrs {
        media = media.crypto_attribute(attr);
    }

    // Reserved extension points for future Sprint 3/4 work:
    // - Comfort Noise (C1): `.rtpmap("13", "CN/8000")` when enabled.
    // - ICE (D3): candidate + ufrag/pwd attrs.
    // - DTLS-SRTP (D2): fingerprint + setup attrs.
    // - Video (D5): separate m=video section.

    let session = media
        .attribute("sendrecv", None::<String>)
        .done()
        .build()
        .map_err(|e| SessionError::SDPNegotiationFailed(
            format!("SdpBuilder failed: {e}")))?;

    Ok(session.to_string())
}
```

Delete `generate_sdp_offer` entirely. Every internal caller of the
old public `generate_sdp_offer` (check the SRTP negotiator wiring —
there may be one) is redirected to `generate_local_sdp` which now
covers the SRTP case.

### Step 4.3 — Byte-compatibility regression

The repo contains an existing test
`sdp_offer_matches_legacy_format` that asserts the builder output
matches the legacy format-string output byte-for-byte (minus whitespace
normalization). Run it:

```bash
cargo test -p rvoip-session-core --lib sdp_offer_matches_legacy_format
```

If it was previously only checking the non-SRTP case, extend it with
an SRTP variant that asserts the builder path matches what the old
`generate_sdp_offer` produced for SRTP. This locks the refactor
against subtle diff.

### Step 4.4 — Audit answer-side symmetry

**File:** `crates/session-core/src/adapters/media_adapter.rs`
(search for `generate_sdp_answer` / `negotiate_sdp_as_uas` near line
432 based on prior grep).

The answer path has the same dual structure in miniature. Unify it
the same way — one method that branches on `offer_srtp` /
`srtp_required` internally.

### Verification gate 4

```bash
cargo build -p rvoip-session-core
cargo test -p rvoip-session-core --lib
cargo test -p rvoip-session-core --test srtp_call_integration --features dev-insecure-tls
cargo test -p rvoip-session-core --test audio_roundtrip_integration
./crates/session-core/examples/streampeer/srtp/run.sh
./crates/session-core/examples/streampeer/audio/run.sh
./crates/session-core/examples/run_all.sh
```

Expected: all green. The SRTP round-trip still negotiates; the audio
round-trip still hears the peer's tone.

**LOE estimate:** 1-2 days. The write is straightforward; the
byte-compat regression and answer-side audit add the buffer.

**Rollback:** Keep the old `generate_sdp_offer` as a private helper
for one release after the merge, and revert the `generate_local_sdp`
body if anything regresses in the wild. After one clean release,
delete for real.

---

## Phase 5 — P4: Move RFC 4733 retransmit dedup into rtp-core

**Goal.** Move the three-packet DTMF retransmit dedup out of
media-core's `spawn_rtp_event_handler` and into rtp-core's UDP
receive loop where the PT 101 decode already lives. Media-core
receives "one logical digit per tone" instead of three frames.

**Prerequisites.** Phases 1-4 complete.

### Step 5.1 — Add the dedup state to rtp-core UDP transport

**File:** `crates/rtp-core/src/transport/udp.rs`

Near the `UdpRtpTransport` struct definition, add:

```rust
/// RFC 4733 §2.5.1.3 — the sender emits three identical end-of-event
/// frames for loss resilience. Each shares `(peer, ssrc, rtp_timestamp)`
/// with the first. Dedupe here so downstream consumers receive one
/// `RtpEvent::DtmfEvent` per tone. Entries expire after
/// `DTMF_DEDUP_TTL` — long enough that the third retransmit arrives
/// before the entry expires, short enough that the map doesn't grow
/// unboundedly.
dtmf_seen: Arc<DashMap<(SocketAddr, u32, u32), Instant>>,
```

Const near the top:

```rust
const DTMF_DEDUP_TTL: Duration = Duration::from_millis(500);
```

### Step 5.2 — Dedupe at the PT 101 branch

**File:** `crates/rtp-core/src/transport/udp.rs` (the PT 101 branch
added in Sprint 2)

```rust
if packet.header.payload_type == 101 && packet.payload.len() >= 4 {
    let p = &packet.payload[..4];
    let event = p[0];
    let byte1 = p[1];
    let end_of_event = (byte1 & 0b1000_0000) != 0;
    let volume = byte1 & 0b0011_1111;
    let duration = u16::from_be_bytes([p[2], p[3]]);
    let timestamp = packet.header.timestamp;
    let ssrc = packet.header.ssrc;

    // RFC 4733 §2.5.1.3 retransmit dedup. Fire only on the first
    // observed (peer, ssrc, timestamp) triple; the two end-of-event
    // retransmits share the tuple and are suppressed.
    if end_of_event {
        let key = (addr, ssrc, timestamp);
        let now = Instant::now();
        // Prune stale entries inline on each call (cheap: < N ms
        // walks at 25 ms inter-packet arrivals).
        dtmf_seen.retain(|_, seen_at| now.duration_since(*seen_at) < DTMF_DEDUP_TTL);
        if dtmf_seen.insert(key, now).is_some() {
            continue;   // already fired for this tone
        }
    }

    let dtmf = RtpEvent::DtmfEvent { /* … */ };
    event_tx.send(dtmf).ok();
    continue;
}
```

### Step 5.3 — Remove the dedup from media-core

**File:** `crates/media-core/src/relay/controller/mod.rs`
(lines 825-960 based on Sprint 2 scaffolding)

Delete:
- The `dtmf_last_delivered: Arc<RwLock<Option<(u32, u32)>>>` field
  on the per-dialog handler.
- The `if *last == Some((dtmf_ssrc, dtmf_timestamp)) { continue; }`
  check.
- The `*last = Some((dtmf_ssrc, dtmf_timestamp));` write.

The handler becomes a straight forwarder: every
`RtpSessionEvent::DtmfReceived` with `end_of_event=true` triggers one
`DtmfNotification`. The rtp-core layer now guarantees that's
per-tone.

### Step 5.4 — Add unit test for rtp-core dedup

**File:** `crates/rtp-core/src/transport/udp.rs` (test module)

Add:

```rust
#[tokio::test]
async fn test_pt101_retransmits_dedup_to_single_event() {
    // Sender transmits three identical E=1 packets (same ssrc,
    // same timestamp, same duration). Receiver must emit one
    // DtmfEvent, not three.
}
```

### Verification gate 5

```bash
cargo test -p rvoip-rtp-core --lib
cargo test -p rvoip-media-core --lib
cargo test -p rvoip-session-core --lib
./crates/session-core/examples/streampeer/dtmf/run.sh
# Must still assert "5/5 DTMF digits round-tripped".
```

**LOE estimate:** ~4 hours.

**Rollback:** Trivial revert. The rtp-core change is isolated; the
media-core deletion can be reapplied.

---

## Phase 6 — P3: RFC 4733 timestamp coherence + §2.5.1.3 retransmits

**Goal.** Make the outbound DTMF path RFC-conformant on any transport
(not just zero-loss localhost UDP). Two sub-fixes:

- **3a** — DTMF packet timestamps anchor on the audio RTP session's
  current timestamp cursor, not wall clock.
- **3b** — One tone emits the full §2.5.1.3 packet sequence:
  `E=0` start → continuation packets every 20 ms incrementing
  `duration` → three `E=1` final packets.

**Prerequisites.** Phases 1-5 complete. Critically P4 first — the
receive-side dedup must already be in rtp-core so our multi-packet
sender's three retransmits don't cause the receiver to fire three
times.

### Step 6.1 — Expose the RTP timestamp cursor

**File:** `crates/rtp-core/src/session/mod.rs`

Add a public accessor on `RtpSession`:

```rust
impl RtpSession {
    /// Current RTP timestamp cursor — the timestamp the next audio
    /// packet will carry. Used by the RFC 4733 telephone-event
    /// sender so DTMF packet timestamps are coherent with the audio
    /// stream that shares our SSRC (RFC 4733 §2.1).
    pub fn current_timestamp(&self) -> RtpTimestamp {
        self.timestamp_cursor.load(Ordering::Acquire)
    }
}
```

If `RtpSession` doesn't currently track a cursor, the accessor becomes
"last sent audio packet's timestamp" — read from the scheduler's
state, or (if still unavailable) synthesize as
`first_audio_ts + elapsed_audio_samples`. Implementation is
rtp-core-internal and doesn't affect the DTMF module's interface.

### Step 6.2 — Design `DtmfTransmitter`

**New file:** `crates/media-core/src/relay/controller/dtmf_transmitter.rs`

```rust
use std::sync::Arc;
use std::time::Duration;

use crate::codec::audio::dtmf::{DtmfEvent, TelephoneEvent};
use crate::error::Result;
use rvoip_rtp_core::RtpSession;
use tokio::sync::Mutex;

/// RFC 4733 §2.5.1.3 compliant DTMF sender. Schedules the full packet
/// sequence for one digit:
///
/// 1. `E=0` start packet at tone start, timestamp = audio stream's
///    current cursor.
/// 2. Continuation packets every 20 ms, `duration` incrementing by 160
///    samples (20 ms × 8 kHz), timestamp FIXED at the start timestamp.
/// 3. Three `E=1` retransmits back-to-back at the end, duration =
///    final tone duration.
///
/// Owns no state across calls; each `send_digit` spawns an
/// independent task. Caller uses the returned handle for
/// fire-and-forget (drop) or coordination (await).
pub struct DtmfTransmitter {
    rtp_session: Arc<Mutex<RtpSession>>,
    /// Configured volume attenuation in -dBm0, saturates at 63.
    volume: u8,
}

impl DtmfTransmitter {
    pub fn new(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        Self { rtp_session, volume: 10 }
    }

    pub fn send_digit(
        &self,
        digit: char,
        duration_ms: u32,
    ) -> Result<tokio::task::JoinHandle<Result<()>>> {
        let rtp_session = self.rtp_session.clone();
        let volume = self.volume;
        Ok(tokio::spawn(async move {
            run_schedule(rtp_session, digit, duration_ms, volume).await
        }))
    }
}

async fn run_schedule(
    rtp_session: Arc<Mutex<RtpSession>>,
    digit: char,
    duration_ms: u32,
    volume: u8,
) -> Result<()> {
    let event_code = DtmfEvent::from_digit(digit)
        .map(|d| d.0)
        .unwrap_or(0);

    // Anchor timestamp on the audio stream (RFC 4733 §2.1).
    let start_timestamp = {
        let session = rtp_session.lock().await;
        session.current_timestamp()
    };

    // 20 ms per packet at 8 kHz = 160 samples per step.
    const TICK: Duration = Duration::from_millis(20);
    const SAMPLES_PER_TICK: u16 = 160;

    let total_ticks = (duration_ms / 20).max(1) as u16;
    let mut duration_samples: u16 = 0;

    // Start packet: E=0, marker=1 (first packet of a new event).
    duration_samples = SAMPLES_PER_TICK;
    send_packet(&rtp_session, event_code, /*end*/false, volume,
                duration_samples, start_timestamp, /*marker*/true).await?;

    for _ in 1..(total_ticks.saturating_sub(1)) {
        tokio::time::sleep(TICK).await;
        duration_samples = duration_samples.saturating_add(SAMPLES_PER_TICK);
        send_packet(&rtp_session, event_code, /*end*/false, volume,
                    duration_samples, start_timestamp, /*marker*/false).await?;
    }

    // Three end-of-event retransmits (RFC 4733 §2.5.1.3). All share
    // the start timestamp; receiver dedupes on (ssrc, timestamp) —
    // see Phase 5.
    tokio::time::sleep(TICK).await;
    duration_samples = duration_samples.saturating_add(SAMPLES_PER_TICK);
    for _ in 0..3 {
        send_packet(&rtp_session, event_code, /*end*/true, volume,
                    duration_samples, start_timestamp, /*marker*/false).await?;
    }

    Ok(())
}

async fn send_packet(
    rtp_session: &Arc<Mutex<RtpSession>>,
    event: u8,
    end_of_event: bool,
    volume: u8,
    duration: u16,
    timestamp: u32,
    marker: bool,
) -> Result<()> {
    let tele = TelephoneEvent { event, end_of_event, volume, duration };
    let wire = tele.encode();
    let mut session = rtp_session.lock().await;
    session.send_packet_with_pt(
        timestamp,
        bytes::Bytes::from(wire.to_vec()),
        marker,
        /*PT*/101,
    ).await.map_err(|e| /* map to media-core Error */)?;
    Ok(())
}
```

### Step 6.3 — Unit test the transmitter

Add `#[cfg(test)] mod tests` to the new module:

- `start_packet_carries_e0_marker1_and_initial_duration` — set up a
  fake RTP session, spawn the transmitter, capture the first packet,
  assert shape.
- `continuation_packets_share_timestamp_increment_duration` — capture
  the second and third packets.
- `three_end_of_event_packets_at_tone_end` — capture the last three,
  assert `E=1` on all, identical `(ssrc, timestamp)`.
- `single_retransmit_survives_two_packet_loss` — drop two of the three
  at the receiver, assert the digit still arrives (combined with Phase
  5's receive-side dedup).

### Step 6.4 — Thin out `send_dtmf_packet`

**File:** `crates/media-core/src/relay/controller/rtp_management.rs`

The existing `send_dtmf_packet` becomes a thin wrapper:

```rust
pub async fn send_dtmf_packet(
    &self,
    dialog_id: &DialogId,
    digit: char,
    duration_ms: u32,
) -> Result<()> {
    let rtp_session = self.get_rtp_session(dialog_id).await
        .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
    let transmitter = DtmfTransmitter::new(rtp_session);
    let handle = transmitter.send_digit(digit, duration_ms)?;
    // Fire-and-forget: drop the handle so the task runs to completion
    // in the background. Caller gets back as soon as the schedule is
    // armed — critical for softphone UX where a key-down shouldn't
    // block on the full tone.
    drop(handle);
    Ok(())
}
```

### Step 6.5 — Update the DTMF example expectations

**File:** `crates/session-core/examples/streampeer/dtmf/run.sh`

The client currently fires 5 DTMF digits at 500 ms apart. With the
new §2.5.1.3 pattern, each digit takes 100 ms (the configured
duration) to transmit its full schedule. The 500 ms inter-digit gap
is still generous. The run.sh assertion of "5/5 digits round-tripped"
should still hold — the receive-side dedup in Phase 5 collapses the
three E=1 retransmits into one `Event::DtmfReceived`.

Client-side should add a small sleep after the last digit to let the
schedule flush before hangup:

```rust
for digit in ['1', '2', '3', '4', '#'] {
    handle.send_dtmf(digit).await?;
    sleep(Duration::from_millis(500)).await;
}
// Let the last digit's E=1 retransmits complete before hangup.
sleep(Duration::from_millis(200)).await;
handle.hangup().await?;
```

### Verification gate 6

```bash
cargo test -p rvoip-rtp-core --lib
cargo test -p rvoip-media-core --lib
cargo test -p rvoip-session-core --lib
./crates/session-core/examples/streampeer/dtmf/run.sh
# Must still assert "5/5 DTMF digits round-tripped" with the new
# multi-packet sender + Phase 5 dedup working together.
```

**Packet-capture spot-check** (manual, optional): run the dtmf
example under `tcpdump -i lo0 -w /tmp/dtmf.pcap udp port 16600-16700`
then load `/tmp/dtmf.pcap` in Wireshark. Verify each digit shows as
`n` packets (1 start + ~4 continuation + 3 end) all sharing
`RTP Timestamp`, incrementing `duration`.

**LOE estimate:** 2-3 days. Transmitter module ~200 LOC; the
`current_timestamp` accessor plumbing is small but requires
understanding where rtp-core's scheduler tracks that state.

**Rollback:** Keep the old `send_dtmf_packet` single-packet body as a
`send_dtmf_packet_simple` for one release. If the new transmitter
regresses in the wild, flip a runtime flag to use the old path while
investigating.

---

## Cross-phase verification suite

After **each phase**, the full regression suite runs. The bar for
moving to the next phase is all-green here:

```bash
# Library tests — per crate.
cargo test -p rvoip-dialog-core --lib
cargo test -p rvoip-media-core --lib
cargo test -p rvoip-rtp-core --lib
cargo test -p rvoip-session-core --lib
cargo test -p rvoip-sip-core --lib
cargo test -p rvoip-sip-transport --lib

# Integration tests — session-core (the integration hub).
cargo test -p rvoip-session-core \
    --features dev-insecure-tls \
    --test audio_roundtrip_integration \
    --test tls_call_integration \
    --test srtp_call_integration \
    --test register_flow_test \
    --test cancel_integration \
    --test prack_integration \
    --test session_timer_integration \
    --test session_timer_failure_integration \
    --test blind_transfer_integration \
    --test bridge_roundtrip_integration \
    --test notify_send_integration \
    --test glare_retry_integration \
    --test early_media_tests \
    --test redirect_follow \
    --test register_423_retry \
    --test session_422_retry \
    --test invite_auth_tests

# End-to-end examples — the strongest signal.
./crates/session-core/examples/run_all.sh
```

**Non-negotiable gates** (must be green before merging any phase):

- All library test suites pass.
- `audio_roundtrip_integration` passes — lock-in that media round-trip
  still works.
- `streampeer/dtmf/run.sh` still reports "5/5 DTMF digits round-tripped".
- `streampeer/tls/run.sh` still observes a TLS-transported INVITE.
- `streampeer/srtp/run.sh` still negotiates SDES.

---

## Risk register

| # | Risk | Likelihood | Mitigation |
|---|------|------------|------------|
| 1 | P5 rename breaks media-core consumers outside the workspace | Low | media-core is workspace-internal; no downstream crates |
| 2 | P2 byte-compat regression on the SDP offer | Medium | `sdp_offer_matches_legacy_format` test extended to cover SRTP. Revert if diff is non-trivial. |
| 3 | P3 timestamp accessor exposes rtp-core internal state | Low | Accessor returns a `Copy` primitive; no lock held. |
| 4 | P3 multi-packet sender + P4 dedup interact badly | Medium-high | Phase 5 lands first; P3's three-retransmit output is designed to match the dedup key exactly. Phase 6 verification gate has explicit "5/5 round-tripped" check covering this interaction. |
| 5 | P6 feature flag breaks downstream users of `tls_insecure_skip_verify` | Low | Field is dev-only today; no production user is expected. If one materializes, the feature flag's clear name + compile error makes the escape hatch obvious. |
| 6 | Phase-ordering bug discovered mid-sprint | Medium | Each phase has its own rollback boundary (single commit where possible). Can reorder P4/P5 if needed without upstream impact. |

---

## Success criteria

Sprint 2.5 is complete when:

1. ✅ All six priorities (P1-P6) merged to `main`.
2. ✅ All verification gates green.
3. ✅ `ARCHITECTURE_OVERVIEW.md` updated with the Media-plane side
   effects rule (Phase 1) and the single SDP-generation entry point
   (Phase 4).
4. ✅ `GENERAL_PURPOSE_SIP_CLIENT_PLAN.md` progress log has a new
   `2026-04-XX | Sprint 2.5 ✅` row summarizing the cleanup.
5. ✅ Zero `Unknown action 'X', treating as custom` warnings at
   state-table load time. (The silent fallthrough has been replaced
   by a hard `SessionError::InternalError` and a regression test —
   so the criterion strengthens to "the YAML loader hard-errors on
   any unknown action name".)
6. ✅ All three new examples (`streampeer/dtmf`, `streampeer/tls`,
   `streampeer/srtp`) run green under `run_all.sh`.
7. ✅ Sprint 3 (A6 STUN + A7 digest + C1 Comfort Noise + C2 SDP
   offer/answer matching helper) ready to start on the clean base —
   in particular C2 lands on the unified `generate_local_sdp` path
   from P2 without needing a "which SDP generator do I target?"
   question.

### Scope deviations from the plan as written

**P5 reduced to `MediaSessionId` only** (Decision A in the executed
plan — see `/Users/jonathan/.claude/plans/we-are-working-on-optimized-babbage.md`
for the rationale). `state_table::DialogId` was *not* unified — it
stays the `Copy` Uuid newtype with bidirectional `From` impls to
`rvoip_dialog_core::DialogId`. The doc above (Step 3.2) suggests
aliasing both; that would have broken `Copy` semantics, removed
`Serialize`/`Deserialize`, and required a parallel rename in
dialog-core. The reduced scope addresses the actual pain point (the
`MediaSessionId::from_dialog` reconstruction footgun + the "fresh
UUID" bug class) without the dialog-core ripple. Net: ~6 call sites
touched instead of the doc's estimated ~50-70.

**`Action::SendPUBLISH` retained** (Decision B). Step 1.1 of the
plan above lists it as a candidate-dead variant; the YAML actually
wires `StartPublish → Publishing` through it, and per RFC 3903 the
PUBLISH method is genuinely state-machine-shaped. The action variant
maps to `Custom("SendPUBLISH")` until presence publishing lands —
not deleted.

**P1 validation pass uses `strum`** (Decision C). Step 1.4 of the
plan above said "use strum or a hand-maintained list"; the executed
plan added `strum = { version = "0.26", features = ["derive"] }`
and a `#[derive(strum::VariantNames)]` on `Action`. Hand-maintained
list considered and rejected — a derive guarantees coverage at
compile time.

**Pre-existing non-SRTP DTMF bug fixed under P2** with regression
test. The doc above frames P2 as pure unification; the executed
plan also fixes a real bug: the non-SRTP path of the old
`generate_local_sdp` emitted `m=audio N RTP/AVP 0 8` with no PT 101
/ rtpmap / fmtp, silently breaking DTMF negotiation for plaintext
calls since the SRTP work landed in Sprint 1. The unified path
always advertises `0 8 101` regardless of profile.

---

## Cross-reference to `GENERAL_PURPOSE_SIP_CLIENT_PLAN.md`

After Sprint 2.5 ships, update the roadmap doc as follows:

### Sprint plan table (insert row between Sprint 2 and Sprint 3)

The text actually merged into `GENERAL_PURPOSE_SIP_CLIENT_PLAN.md`:

```
| **2.5 — architectural hygiene** ✅ | P1 (boundary rule) ✅ + P6 (TLS feature gate) ✅ + P5 (MediaSessionId alias only — `state_table::DialogId` preserved as Uuid newtype) ✅ + P2 (unified SDP + non-SRTP DTMF bug fix) ✅ + P4 (dedup relocation) ✅ + P3 (RFC 4733 §2.5.1.3 multi-packet send) ✅ | **Shipped 2026-04-25.** Dead state-machine variants deleted + strum-derived YAML drift detection; `tls_insecure_skip_verify` behind `dev-insecure-tls` Cargo feature; `MediaSessionId` aliased to `rvoip_media_core::DialogId` (eliminates `from_dialog` reconstruction); single `generate_local_sdp` entry point that always advertises PCMU + PCMA + telephone-event (fixes pre-existing plaintext DTMF gap); RFC 4733 retransmit dedup moved to rtp-core's UDP layer; outbound DTMF emits the full RFC 4733 §2.5.1.3 packet schedule. No user-visible feature changes. |
```

### Progress log entry

A detailed multi-paragraph entry was added to
`GENERAL_PURPOSE_SIP_CLIENT_PLAN.md` (dated `2026-04-25`) covering
each priority's specific deliverables, the P5 scope reduction
rationale, the pre-existing non-SRTP DTMF bug fix, and the aggregate
test counts. Refer to that doc for the canonical record.

---

## Verification (final)

Before declaring Sprint 2.5 done:

```bash
# Full clean build to catch any feature-flag drift.
cargo clean -p rvoip-session-core -p rvoip-media-core -p rvoip-rtp-core -p rvoip-dialog-core

# Default build (no features).
cargo build --workspace
cargo test --workspace --lib

# Feature build (for the TLS example + test).
cargo build -p rvoip-session-core --features dev-insecure-tls
cargo test -p rvoip-session-core --features dev-insecure-tls --test tls_call_integration

# End-to-end example suite.
./crates/session-core/examples/run_all.sh
```

All green → merge Sprint 2.5. Then proceed to Sprint 3 (A6 STUN,
A7 digest, C1 Comfort Noise, C2 SDP offer/answer matching helper) on
the clean foundation.
