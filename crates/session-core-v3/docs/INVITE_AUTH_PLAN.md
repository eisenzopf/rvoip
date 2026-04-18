# Consolidated Digest Auth (INVITE + REGISTER) — Implementation Plan

## Context

Today session-core-v3 supports RFC 3261 digest auth on **REGISTER only**,
and via a shortcut: `DialogAdapter::handle_response_for_session`
(`dialog_adapter.rs:539–593`) intercepts 401/407 on REGISTER directly and
retries inline, *bypassing the state machine*. There's a half-wired
state-table path (`Registration401` → `Authenticating` with
`StoreAuthChallenge` + `SendREGISTERWithAuth`) but the adapter shortcut is
what actually fires in practice.

Meanwhile INVITE challenges (401 / 407) fall through to generic
`CallFailed { status_code: 401|407 }` and the call dies. This blocks
interop with almost every production SIP carrier: Twilio, Vonage,
Bandwidth, most enterprise PBXs (Asterisk, FreeSWITCH with auth), and
upstream proxies all challenge INVITE per-call.

Tracked at `RFC_COMPLIANCE_STATUS.md:53, 124` as a known gap.

This plan does two things:

1. **Add INVITE auth retry.** The carrier-blocker fix.
2. **Consolidate REGISTER auth onto the same path.** Eliminate the
   in-adapter shortcut so both request types share one mechanism — one
   cross-crate event, one state-machine-driven flow, one place to add
   future auth features (nc tracking, auth-int, SHA-256-sess). Without
   this step we'd ship two divergent auth paths that'll drift.

The consolidation is roughly +4 hrs on top of the INVITE work and
eliminates a pre-existing smell, so it's worth doing now rather than
filed as cleanup.

---

## Answers to the design questions the code raises

1. **Handle the retry in dialog-core (like REGISTER) or via the state
   table?** — **State table.** REGISTER gets away with an in-adapter shortcut
   (`DialogAdapter::handle_response_for_session` at
   `dialog_adapter.rs:539–593`) because it's a simple request/response with
   no media or dialog establishment. INVITE is different: a 401 arrives
   while the call is in `Initiating`, and the retry is effectively a new
   transaction that still must not advance the call to `Active`/`Failed`.
   The state-table path gives us visibility, a retry counter, and composes
   cleanly with the existing 3xx redirect / 491 glare machinery. Cost:
   richer event plumbing (next answer).

2. **How do we get the `WWW-Authenticate` header across the dialog-core →
   session-core-v3 boundary?** — Today, `DialogToSessionEvent::CallFailed`
   (`infra-common/src/events/cross_crate.rs:328–332`) carries only
   `status_code` and `reason_phrase`; the full header is stripped in
   `event_hub.rs:354–365`. **We add a distinct event variant**
   `DialogToSessionEvent::AuthRequired { session_id, status_code,
   challenge: String, realm: Option<String> }` emitted from
   `handle_transaction_failure_response` whenever a 401/407 arrives with a
   challenge header — **regardless of method**. Both INVITE and REGISTER
   route through this variant. Keeps `CallFailed` semantics clean (it still
   means "terminal" to consumers) and avoids bolting optional header data
   onto an event that everyone pattern-matches exhaustively.

3. **New dialog-core retry API, or reuse `make_call_for_session`?** —
   **New API.** RFC 3261 §22.2 requires the retry to reuse the same
   Call-ID with a bumped CSeq on a new transaction (new Via branch). Going
   through `make_call_for_session` again would create a brand-new dialog
   with a fresh Call-ID — wrong. Add
   `UnifiedDialogApi::resend_invite_with_auth(dialog_id, auth_header,
   header_name)` that builds a new INVITE client transaction with
   `next_local_cseq()`, same Call-ID, same From/To tags (none from peer
   yet), same Request-URI, and the caller-supplied `Authorization` /
   `Proxy-Authorization` header appended.

4. **Credentials API — per-peer config, per-call override, or both?** —
   **Both, with per-peer as the primary path.** REGISTER currently
   requires credentials to be passed to the `register()` call; for INVITE
   the natural place is `StreamPeer::with_credentials(user, pass)` on the
   builder (set once, apply to every outgoing call). A
   `PeerControl::call_with_auth(target, creds)` escape hatch covers
   multi-tenant clients. Credentials land in `SessionState.credentials`
   (already exists at `state.rs:123`) so the existing action handlers
   don't need new wiring.

5. **Nonce counter (`nc`) — one-shot or tracking?** — **One-shot for MVP.**
   `auth-core::DigestClient::compute_response` hard-codes
   `nc=00000001` at `sip_digest.rs:354`. REGISTER accepts this today; so
   will INVITE. Real-world UASs rarely challenge with the same nonce
   twice in a row (they issue a fresh `nextnonce`). Add a TODO for
   multi-challenge nc tracking — it's a 10-line follow-up once we have
   live carrier data showing it matters.

6. **Should 407 be a separate flow?** — **No.** 401 and 407 are
   identical in mechanics; only the headers differ (`WWW-Authenticate` +
   `Authorization` for 401, `Proxy-Authenticate` + `Proxy-Authorization`
   for 407). The event variant carries the status code; the action picks
   the header name from it.

7. **How do INVITE and REGISTER share one event in the state table?** —
   The state-table key is `(Role, CallState, EventType)`. INVITE auth
   fires while we're in `Initiating`; REGISTER auth fires while we're in
   `Registering`. The state disambiguates which action runs, so **one
   event variant (`EventType::AuthRequired { status_code, challenge }`)
   drives two transitions** without payload-based matching. This is why
   the event doesn't need to carry the method — the state implies it.

8. **What about auth on re-INVITE, UPDATE, or BYE (mid-dialog)?** —
   **Out of scope for this pass.** RFC 3261 §22.1 allows servers to
   challenge any request; in practice mid-dialog challenges are rare
   and stash against an established dialog where our retry machinery
   doesn't apply cleanly. Initial-INVITE auth is where carriers
   actually push back. Document as a follow-up.

---

## Key findings from the code (what we reuse)

| Finding | Location | Why it matters |
|---------|----------|----------------|
| `DigestAuthenticator::parse_challenge(&str)` produces a `DigestChallenge` | `auth-core/src/sip_digest.rs:144–195` | Feed it the raw header value from the 401; no sip-core↔auth-core bridge needed |
| `DigestClient::compute_response` + `format_authorization` | `auth-core/src/sip_digest.rs:336–416` | Produces the `Authorization`/`Proxy-Authorization` value as a `String` ready to attach |
| `SessionState.auth_challenge: Option<DigestChallenge>` | `session_store/state.rs:125` | Already present for REGISTER; reuse verbatim (single call can't be challenged for both REGISTER and INVITE simultaneously) |
| `SessionState.credentials: Option<Credentials>` | `session_store/state.rs:123` | Already present; needs population from a new `StreamPeer::with_credentials` |
| `Credentials { username, password, realm }` | `types.rs:413–437` | Direct reuse |
| 3xx redirect retry (`Action::RetryWithContact`) | `state_machine/actions.rs:138–180` | Template for "transition stays in `Initiating`, re-fire INVITE with modified args" |
| 491 glare retry (`Action::ScheduleReinviteRetry`) | `state_machine/actions.rs:87–137` | Template for "capped retry counter with typed error on exceed" |
| REGISTER 401 retry cap (1 attempt) | `dialog_adapter.rs:548` | Same cap (2 attempts total — initial + one retry) matches RFC-practical behavior |
| `Dialog.local_cseq_counter` / `next_local_cseq()` | `dialog-core/src/dialog/dialog_impl.rs` | Used in `resend_invite_with_auth` to bump CSeq while preserving Call-ID |

---

## Design

### 1. Event plumbing — surface the challenge across crates

**`crates/infra-common/src/events/cross_crate.rs`** — add variant:

```rust
DialogToSessionEvent::AuthRequired {
    session_id: String,
    status_code: u16,          // 401 or 407
    challenge: String,         // raw "Digest realm=..." header value
    realm: Option<String>,     // pre-parsed for logging; auth-core re-parses
}
```

Update `session_id()` match arm; update all exhaustive matches in the
converter and handler files flagged by `cargo build`.

**`crates/dialog-core/src/manager/transaction_integration.rs`
(`handle_transaction_failure_response`, lines 954–1023)** — before falling
through to the existing `CallFailed` emission, check for 401/407 with an
`WWW-Authenticate` or `Proxy-Authenticate` header. If present, emit
`AuthRequired` for **any method** (INVITE, REGISTER, and room for future
request types that need auth). If the status is 401/407 but no challenge
header is present, keep the existing `CallFailed` path (malformed server).

**`crates/dialog-core/src/events/event_hub.rs`** — add conversion arm.

**`crates/session-core-v3/src/adapters/session_event_handler.rs`** — add
`handle_auth_required` that publishes
`EventType::AuthRequired { status_code, challenge }` into the state
machine. The session's current `CallState` (which the executor already
tracks) determines whether this resends INVITE or REGISTER.

**`crates/session-core-v3/src/adapters/dialog_adapter.rs`** — **remove**
the inline REGISTER-auth shortcut at lines 539–593
(`handle_response_for_session` 401/407 branch calling
`handle_401_challenge`). Once dialog-core emits `AuthRequired` for
REGISTER too, the state-machine path supersedes this. Leave a brief
comment noting the removal and pointing to the state-table transition.

### 2. State machine — one event, two transitions

**`crates/session-core-v3/src/state_table/types.rs`**:

```rust
EventType::AuthRequired { status_code: u16, challenge: String },
Action::StoreAuthChallenge,         // already exists — reuse verbatim
Action::SendINVITEWithAuth,         // new
// Action::SendREGISTERWithAuth    -- already exists — reuse verbatim
```

Normalize arm for the event: both fields default-valued (follow the
existing `RejectCall` pattern).

**Retire** the existing `EventType::Registration401` variant — it becomes
an alias for `AuthRequired` at the YAML level, and the adapter-shortcut
code path that fired it is gone. Leave the `Registration401` YAML
transition in place as a deprecated alias (map it to `AuthRequired` in
`yaml_loader.rs`) so any externally-authored state tables keep working;
drop the Rust variant after one release.

**`crates/session-core-v3/state_tables/default.yaml`** — two transitions,
both self-loops so the UAS/UAC stays in its current state between the
challenge and the authenticated retry:

```yaml
# INVITE auth retry (RFC 3261 §22.2)
- role: "UAC"
  state: "Initiating"
  event:
    type: "AuthRequired"
  next_state: "Initiating"
  actions:
    - type: "StoreAuthChallenge"
    - type: "SendINVITEWithAuth"
  description: "Auth retry for 401/407 on INVITE"

# REGISTER auth retry (existing behavior, now state-machine driven)
- role: "UAC"
  state: "Registering"
  event:
    type: "AuthRequired"
  next_state: "Registering"
  actions:
    - type: "StoreAuthChallenge"
    - type: "SendREGISTERWithAuth"
  description: "Auth retry for 401/407 on REGISTER"
```

Delete the old `Registration401` transition block once the alias mapping
is in place.

Optional: add a guard `HasCredentials` so a challenge with no credentials
on file falls through to `CallFailed` (or `RegistrationFailed`) instead of
looping; implement as a `Custom("HasCredentials")` guard evaluated in
`state_machine/guards.rs` (follow the pattern used for
`OtherSessionActive`).

### 3. SessionState — one new field

```rust
// session_store/state.rs
pub invite_auth_retry_count: u8,  // default 0; capped at 1 per RFC-practical
```

Reuse `auth_challenge`, `credentials`, and `registration_retry_count` —
they're already there. `registration_retry_count` stays with its existing
cap (1) and is incremented by `SendREGISTERWithAuth` rather than by the
removed adapter-shortcut path.

### 4. Action handlers

**`Action::StoreAuthChallenge`** (already exists; extend if needed):
parse the challenge string via
`DigestAuthenticator::parse_challenge(&session.pending_auth_challenge)?`,
store in `session.auth_challenge`. Payload extraction from event mirrors
`RejectCall` (executor at `executor.rs:175` writes `AuthRequired { status,
challenge }` fields into a sidecar
`SessionState.pending_auth: Option<(u16, String)>`). This action is now
shared by both INVITE and REGISTER flows.

**`Action::SendREGISTERWithAuth`** (already exists at
`dialog_adapter.rs:461–667`): keep the existing digest-computation logic
but move its entry point from the inline 401 intercept to the
state-machine action dispatch. This is mostly a call-site change: the
body of `send_register()` stays, `handle_401_challenge` goes away, and
the state-machine action invokes `send_register` with the already-stored
`session.auth_challenge`.

**`Action::SendINVITEWithAuth`** (new):

```rust
Action::SendINVITEWithAuth => {
    const CAP: u8 = 1;
    if session.invite_auth_retry_count >= CAP {
        return Err("INVITE auth retry cap exceeded".into());
    }
    session.invite_auth_retry_count += 1;

    let challenge = session.auth_challenge.as_ref()
        .ok_or("no stored auth challenge")?;
    let creds = session.credentials.as_ref()
        .ok_or("no credentials on session — set via StreamPeer::with_credentials")?;
    let request_uri = session.remote_uri.as_deref()
        .ok_or("no request URI on session")?;

    let (response, cnonce) = DigestClient::compute_response(
        &creds.username, &creds.password, challenge,
        "INVITE", request_uri,
    )?;
    let header_value = DigestClient::format_authorization(
        &creds.username, challenge, request_uri, &response, cnonce.as_deref(),
    );

    let (status, _) = session.pending_auth.take()
        .ok_or("lost pending_auth context")?;
    let header_name = if status == 407 {
        "Proxy-Authorization"
    } else {
        "Authorization"
    };

    dialog_adapter
        .resend_invite_with_auth(&session.session_id, header_name, &header_value)
        .await?;
}
```

### 5. Dialog-core retry API

**`crates/dialog-core/src/api/unified.rs`** — new public method:

```rust
pub async fn resend_invite_with_auth(
    &self,
    dialog_id: &DialogId,
    header_name: &str,      // "Authorization" or "Proxy-Authorization"
    header_value: &str,
) -> ApiResult<TransactionKey>;
```

Under the hood:
- Look up the dialog; pull `call_id`, `from_uri`, `from_tag`, `to_uri`,
  `remote_addr`, and the original INVITE's `local_sdp` (capture on
  initial send).
- `let cseq = dialog.next_local_cseq();`
- Build an INVITE request via the existing `InviteBuilder::new()` with the
  bumped CSeq, same Call-ID, same From tag (no To tag yet — dialog not
  established).
- Append the caller-supplied auth header as the last header line.
- Dispatch via `create_invite_client_transaction` (same path the initial
  INVITE uses).
- Re-register the session↔transaction mapping so the response routes back.

Smaller cousin: `DialogAdapter::resend_invite_with_auth(session_id,
header_name, header_value)` in `adapters/dialog_adapter.rs`, thin
passthrough.

### 6. Credentials plumbing

**`crates/session-core-v3/src/api/stream_peer.rs`** — `StreamPeer`
builder:

```rust
impl StreamPeer {
    pub fn with_credentials(mut self, username: &str, password: &str) -> Self {
        self.default_credentials = Some(Credentials::new(username, password));
        self
    }
}
```

Stored on `UnifiedCoordinator` (or `PeerControl`) as
`Option<Credentials>`. At `UnifiedCoordinator::make_call`, before
transitioning to `Initiating`, copy into `SessionState.credentials` if
the per-peer default is set.

Optional `call_with_auth` on `PeerControl` for per-call override:

```rust
pub async fn call_with_auth(
    &self,
    target: &str,
    creds: Credentials,
) -> Result<SessionHandle>;
```

### 7. Event surface for the app

Add one app-level event:

```rust
// api/events.rs
Event::CallAuthRetrying {
    call_id: CallId,
    status_code: u16,
    realm: String,
},
```

Fires when `SendINVITEWithAuth` starts. Lets apps log / refresh a token
mid-call. Existing `CallFailed { 401|407 }` continues to fire when:
- Retry cap exceeded.
- No credentials on file.
- Challenge header is malformed.

### 8. Error handling

Add to `crates/session-core-v3/src/errors.rs`:

```rust
#[error("server challenged INVITE but no credentials are on file (see StreamPeer::with_credentials)")]
MissingCredentialsForInviteAuth,

#[error("INVITE auth retry limit exceeded")]
InviteAuthRetryExhausted,
```

Both surface through `CallFailed { reason: error.to_string() }` with the
original 401/407 status code preserved.

---

## File-by-file touch list

### New INVITE auth path

| File | Change |
|------|--------|
| `crates/infra-common/src/events/cross_crate.rs` | Add `DialogToSessionEvent::AuthRequired` variant + `session_id()` arm |
| `crates/dialog-core/src/manager/transaction_integration.rs` | In `handle_transaction_failure_response`, detect 401/407 + challenge header and emit `AuthRequired` instead of `CallFailed` — method-agnostic |
| `crates/dialog-core/src/events/event_hub.rs` | Converter arm for the new variant |
| `crates/dialog-core/src/api/unified.rs` | `resend_invite_with_auth(dialog_id, header_name, header_value)` |
| `crates/dialog-core/src/manager/unified.rs` | `UnifiedDialogManager::resend_invite_with_auth` inner impl |
| `crates/dialog-core/src/transaction/client/builders.rs` | Small helper: `InviteBuilder::with_extra_header(name, value)` if not already present |
| `crates/session-core-v3/src/state_table/types.rs` | `EventType::AuthRequired`, `Action::SendINVITEWithAuth` |
| `crates/session-core-v3/src/state_table/yaml_loader.rs` | Map the new event + action; alias `Registration401` → `AuthRequired` |
| `crates/session-core-v3/state_tables/default.yaml` | Two transitions (INVITE / Initiating and REGISTER / Registering); delete old `Registration401` block |
| `crates/session-core-v3/src/session_store/state.rs` | `invite_auth_retry_count`, `pending_auth` fields + defaults |
| `crates/session-core-v3/src/state_machine/executor.rs` | Copy `AuthRequired` event payload into `pending_auth` |
| `crates/session-core-v3/src/state_machine/actions.rs` | `SendINVITEWithAuth` handler; refactor `SendREGISTERWithAuth` to read from the shared state-machine path |
| `crates/session-core-v3/src/adapters/event_router.rs` | Exhaustive match arms |
| `crates/session-core-v3/src/adapters/dialog_adapter.rs` | `resend_invite_with_auth` passthrough |
| `crates/session-core-v3/src/adapters/session_event_handler.rs` | `handle_auth_required` publisher (replaces any `Registration401`-specific handler) |
| `crates/session-core-v3/src/api/stream_peer.rs` | `with_credentials` builder; `PeerControl::call_with_auth` |
| `crates/session-core-v3/src/api/unified.rs` | Thread per-peer credentials into new sessions on `make_call` |
| `crates/session-core-v3/src/api/events.rs` | `Event::CallAuthRetrying` + `call_id()` arm |
| `crates/session-core-v3/src/errors.rs` | Two new `SessionError` variants |
| `crates/session-core-v3/docs/RFC_COMPLIANCE_STATUS.md` | Flip 401 row to ✅ for INVITE; add 407 row; flip gap #3 |

### REGISTER consolidation (remove the shortcut)

| File | Change |
|------|--------|
| `crates/session-core-v3/src/adapters/dialog_adapter.rs` | **Delete** the 401/407 intercept branch at lines 539–593; **delete** `handle_401_challenge`. The state machine now owns this flow |
| `crates/session-core-v3/src/adapters/dialog_adapter.rs` | `send_register` stays but is invoked by the `SendREGISTERWithAuth` action rather than inline on 401 receipt |
| `crates/session-core-v3/src/state_table/types.rs` | Deprecate `EventType::Registration401` — keep the variant for one release so external state tables compile, but stop producing it |
| `crates/session-core-v3/state_tables/default.yaml` | Remove the old `Registration401` transition (now covered by the shared `AuthRequired` transition on `Registering`) |
| `crates/session-core-v3/tests/registration_test.rs` (if present) | Update assertions from "inline retry" to "state-machine retry" — behavior is identical end-to-end |

---

## Verification plan

### Unit tests

`tests/invite_auth_tests.rs` (new file, follows `early_media_tests.rs` pattern):

1. **State-table wiring**: `Initiating + InviteAuthRequired → Initiating`
   transition exists with the right actions. One test per code (401, 407).
2. **Payload normalization**: `InviteAuthRequired { status_code, challenge }`
   with and without payloads resolves to the same transition.
3. **Retry cap**: `invite_auth_retry_count` starts at 0, capped at 1 — a
   state-level test on `SendINVITEWithAuth` rejects when the counter is
   already at the cap.

Plus an `auth-core` integration test: build a synthetic `Digest realm=...,
nonce=..., algorithm=MD5, qop="auth"` header, feed through
`parse_challenge` → `compute_response` → `format_authorization`, and
assert the output matches a known-good reference (hand-computed) for a
fixed username/password/URI.

### Integration test (multi-binary)

`tests/invite_auth_integration.rs` — new. Pattern: Bob (UAS) is a minimal
challenge responder that requires digest auth on INVITE:

- `examples/streampeer/invite_auth/bob.rs`: a small handler that on
  `IncomingCall` *without* `Authorization` header rejects with a
  stock 401 + `WWW-Authenticate: Digest realm="rvoip-test",
  nonce="abcd1234", algorithm=MD5, qop="auth"`. On the retry it
  validates the digest response and accepts. Requires hooking into
  dialog-core below session-core-v3 to forge the 401 — or reuse the
  `registrar-core` test registrar if it exposes per-call challenges
  (verify during implementation; if not, hand-build a tiny sipp-like
  UAS or add a knob to `Config.use_invite_auth`).
- `examples/streampeer/invite_auth/alice.rs`: UAC with
  `StreamPeer::with_credentials("alice", "alicesecret")`. Calls Bob,
  asserts receives `CallAuthRetrying` once, then `CallAnswered`. Exits 0.

Negative case test: Alice *without* credentials calls Bob. Asserts
receives `CallFailed { status_code: 401, reason contains "no credentials" }`.

Pattern mirrors `prack_integration.rs` with env-var mode switching.

### REGISTER consolidation — no behavior change, different path

The existing REGISTER auth integration test stays green *but traverses
new code*. To catch drift, add an instrumentation assertion:

- Inject a counter into `SendREGISTERWithAuth` (or a tracing span);
  assert the state-machine action fired exactly once per 401. This
  proves the shortcut is truly gone and the state-machine path is what
  executed.

If no REGISTER auth test exists today (the compliance doc notes
"REGISTER + 423 retry ❌ no test — would need a 423-returning registrar
mock" at `RFC_COMPLIANCE_STATUS.md:151`), add a minimal one using the
same challenging-UAS infrastructure we build for INVITE auth — free
coverage gain from the consolidation.

### Regression

- `cargo test -p rvoip-dialog-core --tests --lib` — no regressions.
- `cargo test -p rvoip-session-core-v3 --tests` — no regressions.
- Any existing REGISTER auth test must stay green after the shortcut
  removal — the behavior is identical, only the code path changed.

---

## Effort estimate

~2.5 days (was 2 for INVITE-only; +4 hrs for REGISTER consolidation):

- Cross-crate event wiring (now shared by both methods): 2–3 hrs.
- Dialog-core `resend_invite_with_auth` + INVITE builder helper: 3–4 hrs.
- State-machine event/action/transition + executor payload extract: 2 hrs.
- **REGISTER shortcut removal + state-machine re-routing: 3–4 hrs.**
  Touches `dialog_adapter.rs` (delete inline intercept), `actions.rs`
  (wire `SendREGISTERWithAuth` to the state-machine path), YAML
  cleanup, and alias plumbing for backward-compat.
- Credentials builder API + per-call override: 1–2 hrs.
- Integration test + example UAS that challenges: 4–5 hrs (biggest
  unknown is whether a minimal challenging UAS can be built on top of
  existing test infrastructure or needs a small SIPp script). The
  same UAS infrastructure serves INVITE and REGISTER tests.
- Docs + regression runs: 1–2 hrs.

---

## Recommended implementation order

Do INVITE first, REGISTER consolidation second — even though they share
most infrastructure, this order minimizes risk:

1. **Phase 1 — add INVITE auth alongside the existing REGISTER shortcut.**
   Land the new cross-crate `AuthRequired` event, state-machine
   transitions for `Initiating`, `resend_invite_with_auth`, credentials
   API, and integration test. REGISTER continues to use its inline
   shortcut; both paths coexist briefly. Ship this — carriers unblock.

2. **Phase 2 — retire the REGISTER shortcut.** Add the `Registering +
   AuthRequired` transition, remove `handle_401_challenge` from
   `dialog_adapter.rs`, delete the old `Registration401` YAML block,
   verify the REGISTER regression test still passes. Ship this.

Each phase is independently revertable. Phase 1 delivers user-visible
value; Phase 2 is internal cleanup.

## Out of scope (follow-ups)

1. **Mid-dialog auth challenges** (re-INVITE, UPDATE, BYE, INFO, REFER,
   NOTIFY getting 401/407). The same `resend_with_auth` infrastructure
   extends to these but each needs its own state-machine plumbing and a
   test. Not common with mainstream carriers.
2. **Nextnonce tracking + monotonic `nc` counter.** Current auth-core
   hard-codes `nc=00000001`. Adequate for virtually all servers; revisit
   if a real-world server rejects on duplicate nc.
3. **`auth-int` qop.** `auth-core` only supports `auth`. Almost nobody
   ships `auth-int` in the wild.
4. **SHA-256 / SHA-256-sess.** `auth-core` supports `DigestAlgorithm::SHA256`
   but not the sess variants. Add when a carrier demands it.
5. **PRACK / CANCEL auth.** CANCEL is a separate transaction from the
   INVITE it cancels and can technically be challenged — unheard of in
   practice. Ignore.
6. **TLS client certificate auth.** Orthogonal; belongs in `sip-transport`.
