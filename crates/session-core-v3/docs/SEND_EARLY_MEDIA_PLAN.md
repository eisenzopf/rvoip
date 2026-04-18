# `send_early_media` API — Implementation Plan

Follow-on to Phase C. Unblocks the positive reliable-183 integration test
currently deferred in `PHASE_C_PLAN.md`, and gives applications a real way
to stream early media (ringback tones, announcements, progress audio) before
answering a call.

---

## Context

A UAS today has two states it can expose to the caller:

- **Ringing** (180) — provisional, no media path.
- **Answering / Active** (200 OK + ACK) — call fully up, media both ways.

There is no public API to emit a **183 Session Progress with SDP**, which is
the signal carriers and PBXs use for things like:

- Early announcements ("your call is being routed…").
- Custom ringback from the network instead of local ringing.
- Pre-answer IVR prompts.
- Failure messages that must play before the call is formally declined.

The wire-level machinery already exists in dialog-core (Phase C.1.3): reliable
18x emission with `Require: 100rel` + `RSeq`, T1-backoff retransmit, PRACK
handling, and the `CallState::EarlyMedia` state is already declared in
`crates/session-core-v3/src/types.rs:61`. What's missing is the public
session-core-v3 entry point and the state-table wiring to drive it.

## Goal

Expose an API on the session-core-v3 public surface:

```rust
impl StreamPeer {
    /// Send a reliable 183 Session Progress with an SDP body. If `sdp` is
    /// `None`, generate the answer from the remote offer via media-core
    /// (same path `accept()` uses). Transitions the call to
    /// `CallState::EarlyMedia`.
    pub async fn send_early_media(
        &self,
        call_id: &CallId,
        sdp: Option<String>,
    ) -> Result<()>;
}
```

Reusing it from the state-table-driven flow, so the existing `accept()` /
`reject()` / `hangup()` ergonomics are preserved.

## Non-goals

- **Local RTP playback** of early media. Transmitting packets requires an
  `AudioSource` wired into the media session, which is an existing feature
  surface of media-core — out of scope here. This plan just gets the SIP
  signaling right and the session state right.
- **Auto-fallback to unreliable 183** when the peer didn't advertise 100rel.
  For now we fail fast with a clear error; a future enhancement can downgrade
  to an unreliable 183.
- **Caller-side "accept early media" semantics.** The UAC already processes
  the 183 per Phase C.1.2 (auto-PRACK, dialog state updated). No changes
  needed on the UAC path for this work.

---

## Design

### Reused primitives (no new code needed)

| Primitive | Location | Role |
|-----------|----------|------|
| `CallState::EarlyMedia` | `src/types.rs:61` | Target state after 183 sent |
| `MediaAdapter::negotiate_sdp_as_uas()` | `src/adapters/media_adapter.rs:213` | Generate SDP answer from stored offer |
| `inject_reliable_provisional_headers(resp, rseq)` | `dialog-core/src/manager/transaction_integration.rs:647` | Attach `Require: 100rel` + `RSeq` |
| `spawn_reliable_provisional_retransmit(...)` | `dialog-core/src/transaction/server/reliable_invite.rs:37` | T1 backoff FSM — auto-triggered when response carries `Require: 100rel` |
| `UnifiedDialogApi::send_response_for_session(session_id, 183, body)` | `dialog-core/src/api/unified.rs:685` | Session-addressed response dispatch |
| PRACK handler | `dialog-core/src/protocol/prack_handler.rs` | Already matches incoming RAck and aborts retransmit |
| `Dialog.peer_supports_100rel` + `next_local_rseq()` | `dialog-core/src/dialog/dialog_impl.rs` | Gating + monotonic RSeq |

**Implication**: the heavy lifting is already done. We add a thin plumbing
layer and a state-table transition.

### New additions

1. **Event type** — `EventType::SendEarlyMedia { sdp: Option<String> }`
   in `src/state_table/types.rs`. Adding a variant with a payload means
   checking whether the event enum is currently payload-free; if so, either:
   - Stash the SDP in the `SessionStore` under the call_id before firing a
     unit `SendEarlyMedia` event (cleaner — matches how the offer is already
     stored), **or**
   - Promote the enum to carry data (invasive — touches every match site).

   **Recommendation: stash in session store.** Follows the existing pattern
   — the remote offer is already sidecarred there.

2. **State-table transition** — add to the embedded YAML
   (`src/state_table/embedded.rs` or wherever the default table lives):

   ```yaml
   - state: Ringing
     event: SendEarlyMedia
     actions:
       - SendReliable183WithSdp
     next: EarlyMedia

   - state: Initiating      # UAS state right after IncomingCall dispatched
     event: SendEarlyMedia
     actions:
       - SendReliable183WithSdp
     next: EarlyMedia

   - state: EarlyMedia      # allow re-firing with updated SDP
     event: SendEarlyMedia
     actions:
       - SendReliable183WithSdp
     next: EarlyMedia

   - state: EarlyMedia
     event: AcceptCall
     actions:
       - Send200OK
     next: Answering
   ```

   Final-answer rules don't change: `AcceptCall` from `EarlyMedia` still goes
   to `Answering`, then ACK → `Active`.

3. **Action handler** — `SendReliable183WithSdp` in
   `src/state_machine/actions.rs` (or wherever actions dispatch). Pseudocode:

   ```rust
   Action::SendReliable183WithSdp => {
       let sdp = session_store.take_early_media_sdp(session_id)
           .or_else(|| Some(media_adapter.negotiate_sdp_as_uas(session_id, offer)?));
       dialog_adapter.send_reliable_progress(session_id, 183, sdp).await?;
   }
   ```

4. **DialogAdapter method** — `send_reliable_progress(session_id, status, body)`
   in `src/adapters/dialog_adapter.rs`:

   ```rust
   pub async fn send_reliable_progress(
       &self,
       session_id: &SessionId,
       status: u16,      // 180/183 — caller responsibility; here 183
       body: Option<String>,
   ) -> Result<()> {
       // Look up dialog, check peer_supports_100rel, fail with a typed error
       // if not. Build the response via dialog_api (via a new
       // `send_reliable_provisional_for_session` helper — see below).
       self.dialog_api
           .send_reliable_provisional_for_session(session_id, status, body)
           .await
   }
   ```

5. **dialog-core helper** — `send_reliable_provisional_for_session` in
   `crates/dialog-core/src/api/unified.rs`. Mirrors
   `send_response_for_session` but:
   - Rejects status codes outside 101..=199 (reject 100; 100 is hop-by-hop).
   - Rejects if `dialog.peer_supports_100rel == false` with a distinct error
     (caller can decide to fall back to an unreliable `send_response_for_session`).
   - Pulls `rseq = dialog.next_local_rseq()`.
   - Builds the response, calls `inject_reliable_provisional_headers(&mut r, rseq)`.
   - Sends via `send_response`. The existing hook in `send_transaction_response`
     already detects `Require: 100rel` on the outgoing response and spawns the
     retransmit task — no extra call needed.

6. **Public API** — `StreamPeer::send_early_media` at
   `src/api/stream_peer.rs` (next to `accept` around line 191):

   ```rust
   pub async fn send_early_media(
       &self,
       call_id: &CallId,
       sdp: Option<String>,
   ) -> Result<()> {
       let session_id = self.registry.lookup(call_id)?;
       if let Some(s) = sdp {
           self.session_store.set_early_media_sdp(&session_id, s);
       }
       self.helpers.process_event(&session_id, EventType::SendEarlyMedia).await
   }
   ```

   Also expose the same method on `UnifiedSession` /
   `SimpleSession` for parity.

7. **Incoming call stub** — replace the `_sdp: String` stub at
   `src/api/incoming.rs:87` (`accept_with_sdp`) with a real implementation
   by chaining `send_early_media(Some(sdp))` → `accept()`. Optional polish,
   not required for the core feature.

---

## File-by-file touch list

| File | Change |
|------|--------|
| `crates/session-core-v3/src/state_table/types.rs` | Add `EventType::SendEarlyMedia` variant |
| `crates/session-core-v3/src/state_table/yaml_loader.rs` | String → enum for new event + action |
| `crates/session-core-v3/src/state_table/embedded.rs` (or the YAML file if externalized) | Add 4 transitions listed above |
| `crates/session-core-v3/src/state_machine/actions.rs` | Handler for `SendReliable183WithSdp` |
| `crates/session-core-v3/src/state_machine/helpers.rs` | `send_early_media` helper mirroring `accept_call` |
| `crates/session-core-v3/src/session_store/...` | Optional `early_media_sdp` field on `SessionInfo` + setter/taker |
| `crates/session-core-v3/src/adapters/dialog_adapter.rs` | `send_reliable_progress` method |
| `crates/session-core-v3/src/api/stream_peer.rs` | Public `send_early_media` |
| `crates/session-core-v3/src/api/unified.rs` | Mirror on `UnifiedSession` |
| `crates/session-core-v3/src/api/simple.rs` | Mirror on `SimpleSession` for the simple API |
| `crates/session-core-v3/src/session_store/inspection.rs` | Add `SendEarlyMedia` to event enum listing for diagnostics |
| `crates/dialog-core/src/api/unified.rs` | `send_reliable_provisional_for_session` + error variant for "peer doesn't support 100rel" |
| `crates/dialog-core/src/errors.rs` (or equivalent) | New `ApiError::PeerDoesNotSupport100Rel` |

---

## Test plan

### Unit

- **State transitions**: `Ringing + SendEarlyMedia → EarlyMedia` and
  `EarlyMedia + AcceptCall → Answering`. One test per new transition in
  `src/state_machine/tests.rs` (or wherever state-machine unit tests live).
- **Dialog-adapter error path**: `send_reliable_progress` on a dialog where
  `peer_supports_100rel == false` returns the expected typed error. Can be a
  fake-dialog fixture test; no network required.
- **SDP auto-gen fallback**: `send_early_media(None)` goes through the
  media-adapter path. Mock the adapter, assert `negotiate_sdp_as_uas` is
  called exactly once and its return value flows into the 183 body.

### Integration (multi-binary)

Extend the existing PRACK test at
`crates/session-core-v3/tests/prack_integration.rs`:

- **New test**: `prack_positive_reliable_183_flow()`.
- **Alice (UAC)** at `examples/streampeer/prack/alice.rs`:
  - `use_100rel: Supported`.
  - Keep the existing 420 handler but add a second mode selected by env var
    (`PRACK_MODE=positive` vs the current default that expects 420) so the
    single binary can serve both tests without duplicating setup.
  - In positive mode: expects to receive a 183 with SDP, auto-PRACKs it (the
    UAC auto-PRACK path from Phase C.1.2), then receives 200 OK.
  - Asserts `CallState::EarlyMedia` is observable via session events before
    final `CallAnswered`.
- **Bob (UAS)** at `examples/streampeer/prack/bob.rs`:
  - In positive mode, `on_incoming_call` handler calls
    `session.send_early_media(None).await?` before a short delay, then
    `session.accept().await`.

Port plumbing and subprocess driver copied verbatim from the existing 420 test.

### Regression

- `cargo test -p rvoip-dialog-core --tests --lib` — 17 binaries, 326 pass.
- `cargo test -p rvoip-session-core-v3 --tests --lib` — all green.
- Existing `prack_integration` (420 negative path) continues to pass
  unchanged — env-var defaults to negative mode.

---

## Open questions

1. **Does `EventType` already carry payloads elsewhere?** If no, the plan
   commits to the session-store stash approach for SDP, which is cleaner
   and matches the offer-storage pattern. If yes, we could promote
   `SendEarlyMedia` to carry the SDP directly. Answer by grepping variants
   of `EventType` with fields.
2. **Is the state-table YAML embedded or loaded from disk by default?**
   `Config.state_table_path` hints at both; the "embedded default" needs
   updating so users don't have to opt in.
3. **Should `send_early_media` be idempotent on status-code reuse (same SDP
   twice)?** RFC 3262 allows multiple reliable provisionals per call — the
   RSeq increments make each distinct. Tentative answer: yes, re-emission is
   allowed and each one retransmits until PRACKed. Covered by the
   `EarlyMedia + SendEarlyMedia → EarlyMedia` self-loop in the YAML.
4. **Error ergonomics when peer lacks 100rel.** Surface as a typed error
   (`SessionError::UnreliableProvisionalsNotSupported`) so callers can
   choose to fall back to a plain unreliable 183 via a future
   `send_progress(sdp)` method, or skip early media entirely.

---

## Effort estimate

~1.5 days:

- State-table + event plumbing: 3–4 hrs.
- DialogAdapter + dialog-core helper: 2–3 hrs.
- Public API surface (three API tiers): 1–2 hrs.
- Integration test + example rework: 3–4 hrs.
- Debug + docs (update `RFC_COMPLIANCE_STATUS.md` and `PHASE_C_PLAN.md`
  test-coverage rows): 1–2 hrs.

Deferred ("Not done" in `PHASE_C_PLAN.md`) items that become unblocked once
this lands:

- **Positive reliable-183 integration test** — becomes the test above.
- **Early-media SDP wired to media adapter** (`RFC_COMPLIANCE_STATUS.md`
  partial row) — only partially, since we're not spinning up an RTP sender
  in this scope.
