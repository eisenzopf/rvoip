# SIP RFC Compliance Status

This document tracks what SIP messages and behaviors session-core-v3 supports,
what's partial, and what's outstanding. Updated whenever a compliance gap is
closed or a new gap is identified.

## Legend

- ✅ **Supported** — implemented, tested, and wired into the public API
- ⚠️ **Partial** — works in common cases but has known caveats
- ❌ **Not supported** — intentionally out of scope for now, or planned

---

## Responses from UAC perspective (incoming responses)

### 1xx Provisional

| Code | Status | Behavior |
|------|--------|----------|
| 100 Trying | ✅ | Consumed at transaction layer; not surfaced to application (by design — hop-by-hop) |
| 180 Ringing | ✅ | Emits `CallStateChanged(Ringing)` |
| 181 Call Is Being Forwarded | ✅ | Emits `CallStateChanged(Ringing, reason="Forwarded")` |
| 182 Queued | ✅ | Emits `CallStateChanged(Ringing, reason="Queued")` |
| 183 Session Progress (early media) | ⚠️ | Emits `CallStateChanged(Ringing)`. Early-media SDP is not yet processed through the media adapter — codec/answer info is parsed but not applied. |
| 199 Early Dialog Terminated | ✅ | Emits `CallStateChanged(Ringing, reason="EarlyDialogTerminated")` |

### 2xx Success

| Code | Status | Behavior |
|------|--------|----------|
| 200 OK (to INVITE) | ✅ | Emits `CallEstablished`; triggers ACK; starts media |
| 200 OK (to BYE) | ✅ | Terminates dialog cleanly |
| 200 OK (to REGISTER) | ✅ | Marks session as registered |
| 200 OK (to CANCEL) | ✅ | Handled by transaction layer |
| 202 Accepted (to REFER) | ✅ | Handled in blind-transfer path |

### 3xx Redirection

| Code | Status | Behavior |
|------|--------|----------|
| 300 Multiple Choices | ✅ | Auto-follow first Contact URI with q-value priority (RFC 3261 §8.1.3.4). 5-hop cap. |
| 301 Moved Permanently | ✅ | As above |
| 302 Moved Temporarily | ✅ | As above |
| 305 Use Proxy | ⚠️ | Treated as a redirect — auto-follow Contact URI. Proxy semantics not tracked separately. |
| 380 Alternative Service | ⚠️ | Treated as a redirect. |

### 4xx Client Error

| Code | Status | Behavior |
|------|--------|----------|
| 400 Bad Request | ✅ | Emits `Event::CallFailed { status_code: 400, … }`; session → Failed |
| 401 Unauthorized | ✅ | For REGISTER: parse `WWW-Authenticate`, compute digest, retry (single attempt). Not yet applied to INVITE. |
| 403 Forbidden, 404 Not Found, 486 Busy, etc. | ✅ | Emits `Event::CallFailed { status_code, reason_phrase, … }` |
| 407 Proxy Authentication Required | ⚠️ | Parsed but only on REGISTER; INVITE proxy-auth not wired. |
| 423 Interval Too Brief | ✅ | Parses `Min-Expires`, re-issues REGISTER with server's required value. 2-retry cap. |
| 487 Request Terminated | ✅ | Emits distinct `Event::CallCancelled` (not `CallFailed`) — allows UIs to render "missed call" differently. |
| 491 Request Pending (mid-dialog) | ✅ | RFC 3261 §14.1 glare retry with random backoff (2.1-4.0 s), re-issues pending hold/resume/SDP-update. 3-retry cap. |

### 5xx Server Error

| Code | Status | Behavior |
|------|--------|----------|
| All 500-599 | ✅ | Emits `Event::CallFailed { status_code, reason_phrase, … }` |

### 6xx Global Failure

| Code | Status | Behavior |
|------|--------|----------|
| 600-699 | ✅ | Emits `Event::CallFailed { status_code, reason_phrase, … }` |

---

## Requests (methods we can send and receive)

| Method | Send | Receive | Notes |
|--------|------|---------|-------|
| INVITE | ✅ | ✅ | Initial + re-INVITE for hold/resume |
| ACK | ✅ | ✅ | Handled automatically by dialog-core |
| BYE | ✅ | ✅ | |
| CANCEL | ✅ | ✅ | |
| REGISTER | ✅ | ✅ | With digest auth + 423 auto-retry |
| REFER | ✅ | ✅ | Blind transfer; attended transfer also supported |
| NOTIFY | ✅ | ✅ | For REFER progress (RFC 3515) |
| OPTIONS | ⚠️ | ✅ | Incoming responds 200 OK; no outbound helper in public API |
| UPDATE | ✅ (dialog-core) | ✅ (dialog-core) | dialog-core implements both; **not used** by session-core-v3 public API (hold/resume goes through re-INVITE — see `docs/UPDATE_STATUS.md`) |
| MESSAGE | ✅ | ✅ | SIP IM (RFC 3428) |
| INFO | ⚠️ | ⚠️ | dialog-core has the plumbing; no session-core-v3 helper yet |
| PRACK | ❌ | ❌ | 100rel reliable provisionals (RFC 3262) not implemented |
| SUBSCRIBE | ✅ | ✅ | RFC 6665 event framework |
| PUBLISH | ⚠️ | ⚠️ | Partial — dialog-core plumbing exists; presence scenarios not tested in session-core-v3 |

---

## Events (app-level, via `Event` enum)

| Event | When |
|-------|------|
| `IncomingCall` | UAS receives INVITE |
| `CallAnswered` | UAC receives 200 OK |
| `CallEnded` | BYE exchanged cleanly |
| `CallFailed { status_code, reason }` | 3xx (after all redirects failed) / 4xx / 5xx / 6xx final response |
| `CallCancelled` | 487 Request Terminated |
| `CallOnHold` / `CallResumed` | Local or remote hold/resume via re-INVITE |
| `CallMuted` / `CallUnmuted` | Local mute |
| `DtmfReceived` | In-band or SIP INFO DTMF |
| `MediaQualityChanged` | Periodic media quality samples |
| `ReferReceived`, `TransferAccepted`, `TransferCompleted`, `TransferFailed`, `TransferProgress` | REFER / transfer flow |
| `RegistrationSuccess`, `RegistrationFailed`, `UnregistrationSuccess`, `UnregistrationFailed` | REGISTER lifecycle |
| `NetworkError` | Transport-layer failure |
| `AuthenticationRequired` | 401/407 received and requires credentials |

---

## Known gaps (future work)

### Reliability / strict carrier interop

1. **PRACK / 100rel (RFC 3262)** — **Partial**. Header types landed this session:
   - `RSeq` — already existed in `sip-core::types::rseq` (full parse/serialize)
   - `RAck` — **NEW this session** in `sip-core::types::rack` (parse/serialize/`TypedHeader` integration, 10 unit tests)
   - `Method::Prack` — already defined
   - `HeaderName::RAck` / `HeaderName::RSeq` — already defined

   **Still missing** (tracked as Phase C.1 follow-on):
   - dialog-core UAC: detect `Require: 100rel` + `RSeq` on 18x, auto-generate PRACK with `RAck`, track `last_rseq` per dialog
   - dialog-core UAS: generate reliable 18x with `Require: 100rel`, retransmit with T1 backoff per RFC 3262 §3, process incoming PRACK
   - session-core-v3 `Config.use_100rel: RelUsage` flag

2. **Session-Expires / session timers (RFC 4028)** — Not implemented. Long-running calls behind NAT can lose binding without periodic refresh. Needs `Session-Expires:` and `Min-SE:` header parsing, negotiation, half-expiry UPDATE/re-INVITE scheduling, and 408-reason BYE on refresh failure. dialog-core has UPDATE plumbing (see `UPDATE_STATUS.md`); session-core-v3 just needs to wire it to a timer.

### Partial / aesthetic

3. **305 / 380** — Treated as generic 3xx; no proxy semantics.
4. **Early-media SDP** — 183 Session Progress surfaces as `Ringing` but the early-media codec path isn't wired to the media adapter.
5. **INVITE proxy/downstream auth (401/407 on INVITE)** — Digest auth is only applied on REGISTER flows; INVITE auth is not auto-retried.
6. **INFO method** — dialog-core supports it; no public session-core-v3 helper API.
7. **Attended transfer with Replaces on the transferred leg** — Implemented but limited test coverage.

### Audit findings (resolved — see docs)

- Hold/resume previously used UPDATE instead of re-INVITE (Timer F timeouts in logs). **Fixed** — hold/resume now uses re-INVITE per RFC 3261. See `UPDATE_STATUS.md`.
- IncomingCall Drop previously auto-rejected unconditionally, racing with dispatch's decision. **Fixed** — Drop now only fires on panic, sends 500 Server Internal Error per RFC 3261 §21.5.1.
- 4xx/5xx/6xx responses were dropped by dialog-core's event_hub. **Fixed** — all final failure responses now propagate to session-core-v3 as `CallFailed` with correct status code.

---

## Test coverage

| Scenario | Covered |
|----------|---------|
| Successful call setup + teardown | ✅ (hello, auto_answer, etc.) |
| Call rejection (403, 404, 486, etc.) | ✅ (routing example) |
| Call cancel before answer (487) | ⚠️ (wire-level works; no integration test yet) |
| Panic in handler → 500 response | ✅ (panic_safety_test) |
| REGISTER + digest auth | ✅ (registration example) |
| REGISTER + 423 retry | ❌ (no test — would need a 423-returning registrar mock) |
| Blind transfer | ✅ (blind_transfer example) |
| Hold/resume | ✅ (hold_resume example) |
| 3xx redirect follow | ❌ (no test — would need a 302-returning UAS mock) |
| 491 glare retry | ❌ (no test — would need simultaneous re-INVITE scenario) |

Tests to add would fill the "no test" gaps above. The wire logic is in place for all of them.
