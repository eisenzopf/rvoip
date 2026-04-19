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
| 183 Session Progress (early media) | ✅ | UAS emits reliable 183 with SDP via `PeerControl::send_early_media`/`IncomingCall::send_early_media` (RFC 3262). UAC auto-PRACKs; state transitions through `EarlyMedia` with negotiated SDP preserved into the 200 OK. Local RTP playback of early media is a separate media-adapter capability. |
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

**UAS-side 3xx (sending)**: `UnifiedCoordinator::redirect_call(session_id, status, contacts)` / `CallHandlerDecision::Redirect` builds a 3xx response with one or more `Contact:` URIs per RFC 3261 §21.3 and terminates the session cleanly (state table: `UAS/Ringing` + `UAS/EarlyMedia` → `Terminated` via `SendRedirectResponse`). Contact URIs are parsed via `sip-core` and rejected at the API boundary if malformed.

### 4xx Client Error

| Code | Status | Behavior |
|------|--------|----------|
| 400 Bad Request | ✅ | Emits `Event::CallFailed { status_code: 400, … }`; session → Failed |
| 401 Unauthorized | ✅ | RFC 3261 §22.2. INVITE and REGISTER both auto-retry once with `Authorization:` computed from `StreamPeerBuilder::with_credentials` / `PeerControl::call_with_auth`. Unified state-machine path (`EventType::AuthRequired` → `StoreAuthChallenge` + `SendINVITEWithAuth` / `SendREGISTERWithAuth`). |
| 403 Forbidden, 404 Not Found, 486 Busy, etc. | ✅ | Emits `Event::CallFailed { status_code, reason_phrase, … }` |
| 407 Proxy Authentication Required | ✅ | Same path as 401, but uses `Proxy-Authorization` header. Applies to INVITE and REGISTER. |
| 422 Session Interval Too Small | ✅ | RFC 4028 §6. UAS emits with `Min-SE:` when peer's `Min-SE` exceeds our `Session-Expires`. UAC retry-with-bumped-Min-SE is pending (carriers rarely 422 on fresh INVITE). |
| 423 Interval Too Brief | ✅ | Parses `Min-Expires`, re-issues REGISTER with server's required value. 2-retry cap. Covered by `tests/register_423_retry.rs`. |
| 487 Request Terminated | ✅ | UAC `SessionHandle::hangup()` dispatches by state per RFC 3261 §9.1: Initiating → local teardown (no wire CANCEL), Ringing / EarlyMedia → CANCEL, Active / OnHold → BYE. UAS receives CANCEL, replies 200 OK to CANCEL + 487 Request Terminated to the pending INVITE, terminates the dialog, and emits `CallCancelled` up the stack. UAC sees the 487 and surfaces `Event::CallCancelled` (distinct from `CallFailed` for "missed call" UI semantics). `tests/cancel_integration.rs::cancel_emits_callcancelled_event` covers the multi-binary end-to-end flow. |
| 491 Request Pending (mid-dialog) | ✅ | RFC 3261 §14.1 glare retry with random backoff (2.1-4.0 s), re-issues pending hold/resume/SDP-update. 3-retry cap. UAS-side 491 emission is automatic: the `HasPendingReinvite`-guarded YAML transition in `state_tables/default.yaml` fires `SendSIPResponse(491, "Request Pending")` whenever a peer re-INVITE arrives while our own is in flight. Wire-level coverage in `tests/glare_retry_integration.rs` (two peers call `hold()` simultaneously; each side 491s the other, retries, converges on OnHold). |

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
| INVITE | ✅ | ✅ | Initial + re-INVITE for hold/resume. UAS-side re-INVITE is driven by the state machine: dialog-core dispatches in-dialog INVITE to `handle_reinvite` and emits `DialogToSession::ReinviteReceived { method }`, which maps to `EventType::ReinviteReceived`. The `Active + ReinviteReceived` YAML transition answers 200 OK via `NegotiateSDPAsUAS` + `SendSIPResponse`; `HasPendingReinvite`-guarded transitions emit 491 Request Pending for RFC 3261 §14.1 glare. Covered by `tests/glare_retry_integration.rs`. |
| ACK | ✅ | ✅ | Handled automatically by dialog-core |
| BYE | ✅ | ✅ | |
| CANCEL | ✅ | ✅ | |
| REGISTER | ✅ | ✅ | With digest auth + 423 auto-retry |
| REFER | ✅ | ✅ | Blind transfer (`SessionHandle::transfer_blind`) + REFER-with-Replaces primitive (`SessionHandle::transfer_attended`, RFC 3891). Attended-transfer *orchestration* (original + consultation session linkage) is a higher-layer concern outside this crate. |
| NOTIFY | ✅ | ✅ | For REFER progress (RFC 3515) |
| OPTIONS | ⚠️ | ✅ | Incoming responds 200 OK; no outbound helper in public API |
| UPDATE | ✅ (dialog-core) | ✅ | RFC 3311 UPDATE inbound is now state-machine-driven. dialog-core's `process_update_in_dialog` emits the same cross-crate `ReinviteReceived` event with `method: "UPDATE"`; session-core-v3 dispatches to `EventType::UpdateReceived` and the `Active + UpdateReceived` / `OnHold + UpdateReceived` transitions answer 200 OK. UPDATE for RFC 4028 session-timer refresh carries no SDP (no `NegotiateSDPAsUAS` on those transitions). Outbound UPDATE for session modification from session-core-v3's public API is still unused (hold/resume goes through re-INVITE — see `docs/UPDATE_STATUS.md`). |
| MESSAGE | ✅ | ✅ | SIP IM (RFC 3428) |
| INFO | ⚠️ | ⚠️ | dialog-core has the plumbing; no session-core-v3 helper yet |
| PRACK | ✅ | ✅ | RFC 3262 100rel. UAC auto-PRACK on reliable 18x; UAS retransmits 18x with body using T1 backoff until PRACK arrives; 420 Bad Extension on 100rel policy mismatch. |
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
| `SessionRefreshed { expires_secs }` | RFC 4028 session-timer refresh sent + acknowledged |
| `SessionRefreshFailed { reason }` | RFC 4028 refresh timeout / error — dialog torn down with BYE (§10) |
| `CallOnHold` / `CallResumed` | Local or remote hold/resume via re-INVITE |
| `CallMuted` / `CallUnmuted` | Local mute |
| `DtmfReceived` | In-band or SIP INFO DTMF |
| `MediaQualityChanged` | Periodic media quality samples |
| `ReferReceived`, `TransferAccepted`, `TransferCompleted`, `TransferFailed`, `TransferProgress` | REFER / transfer flow |
| `RegistrationSuccess`, `RegistrationFailed`, `UnregistrationSuccess`, `UnregistrationFailed` | REGISTER lifecycle |
| `NetworkError` | Transport-layer failure |
| `AuthenticationRequired` | 401/407 received and requires credentials |
| `CallAuthRetrying { status_code, realm }` | INVITE challenged with 401/407 — about to retry with digest (RFC 3261 §22.2) |

---

## Known gaps (future work)

### Partial / aesthetic

1. **305 / 380** — Treated as generic 3xx; no proxy semantics.
2. **Early-media RTP playback** — the 183 Session Progress signalling path is complete (SDP negotiation, reliable 18x, auto-PRACK, handoff into 200 OK). What's *not* yet in scope: wiring an `AudioSource` onto the media session so UAS-side audio actually streams during the `EarlyMedia` window. Applications can send a 183 + SDP today to keep NAT pinholes alive and satisfy carriers that demand a reliable progress indication, but local playback of a ringback tone or announcement is a separate media-adapter feature.
3. ~~**INVITE proxy/downstream auth (401/407 on INVITE)**~~ — ✅ Shipped. INVITE 401/407 now drives the same state-machine-based retry as REGISTER (`EventType::AuthRequired`). Nonce counter (`nc`) is still hard-coded to `00000001`; multi-challenge tracking is a future enhancement when a real-world server rejects on duplicate nc.
4. **INFO method** — dialog-core supports it; no public session-core-v3 helper API.
5. **Attended transfer orchestration (original + consultation session linkage)** — Intentionally **not** in session-core-v3. The crate models one session per `SessionHandle`; linking two calls and threading a `Replaces` header between them is a higher-layer concern (application code or a dedicated multi-session coordinator). session-core-v3 exposes the primitives: `SessionHandle::transfer_attended(target, replaces)` to send REFER-with-Replaces (RFC 3891) and `SessionHandle::dialog_identity()` to read the Call-ID + tags needed to construct the Replaces value. See `src/api/handle.rs` + `DialogIdentity::to_replaces_value()`.
6. **422 UAC-side retry** — UAS emits 422 Session Interval Too Small + Min-SE when peer's Min-SE exceeds our Session-Expires, but the UAC doesn't auto-retry with a bumped Session-Expires (rare in practice).
7. ~~**Session-timer BYE Reason header**~~ — ✅ Shipped. `DialogManager::send_bye_with_reason` threads a typed `Reason` header (RFC 3326) through `request_builder_from_dialog_template`'s new `extra_headers` parameter; `session_timer.rs` on refresh failure sends BYE with `Reason: SIP ;cause=408 ;text="Session expired"` per RFC 4028 §10 while still surfacing the cause on the `SessionRefreshFailed` event for apps.

### Carrier / real-world interop (not RFC gaps, but block production use)

The RFC-level plumbing above is largely complete for basic SIP, but a
handful of transport / networking capabilities determine whether a
session-core-v3 UAC can actually talk to production carriers (Twilio,
Vonage, Bandwidth, enterprise PBXs behind NAT). Audit performed; specific
file references below.

| Capability | Status | Details |
|------------|--------|---------|
| **DNS — INVITE target** | ✅ | `dialog-core/src/dialog/dialog_utils.rs:125-174` `resolve_uri_to_socketaddr` uses `ToSocketAddrs` / `tokio::net::lookup_host`. A UAC can `call("sip:bob@pbx.example.com")` and it will resolve via system DNS. Silent failure on DNS error (logged, not surfaced as `CallFailed` with a clear reason). |
| **DNS — REGISTER target** | ✅ | `dialog-core/src/api/unified.rs::send_register` now routes through the same `resolve_uri_to_socketaddr` helper as INVITE. `register("sip:alice@sip.twilio.com", …)` resolves the registrar hostname via the system resolver. DNS failure surfaces as `ApiError::protocol("Failed to resolve registrar URI: …")`. SRV/NAPTR (RFC 3263) still not implemented — A/AAAA only. |
| **RFC 3263 SRV + NAPTR** | ❌ | No `trust-dns` / `hickory` / `_sip._udp` / `NAPTR` anywhere in the workspace. Only system A/AAAA fallback. Impact: can't reach carriers that publish SRV priority/weight for geo-failover; won't auto-select TCP/TLS vs UDP per NAPTR. |
| **TLS transport (`sips:` / 5061)** | ❌ | Two concerns: (a) `sip-transport` has two TLS implementations — `crates/sip-transport/src/tls.rs` is a placeholder that returns `Error::NotImplemented`, `crates/sip-transport/src/transport/tls/mod.rs` is more complete (rustls-based server handshake) but not consolidated. (b) Even if that's sorted, session-core-v3 **hardcodes `enable_tls: false`** at `src/api/unified.rs:585` inside `create_dialog_api`. `Config` has no `tls_cert_path` / `tls_key_path` fields. To reach Twilio/Vonage this needs: finish sip-transport's TLS client-side connector, add config fields, flip the hardcoded flag. |
| **TCP transport** | ❌ | Same pattern as TLS: `TransportManagerConfig` supports TCP (`crates/dialog-core/src/transaction/transport/mod.rs`), but session-core-v3's `create_dialog_api` hardcodes `enable_tcp: false`. Some PBXs fall back to TCP for large SDP / video. |
| **Outgoing `rport` (RFC 3581)** | ✅ | `create_via_header` at `crates/dialog-core/src/transaction/manager/handlers.rs` now emits `Via: … ;branch=… ;rport` unconditionally. Responses with `received=`/`rport=` echoed back are already honored (handlers.rs:564-587), so NAT response routing works end-to-end. |
| **Incoming `received` / `rport` honored** | ✅ | Response handler reads both at `handlers.rs:564-587` and uses them for ACK routing. This is the subset of NAT that does work today. |
| **Contact header rewrite from discovered NAT address** | ❌ | `Contact:` is built once from `local_ip:sip_port` (see `InviteBuilder` in `crates/dialog-core/src/transaction/client/builders.rs:690-728`) and never updated. A UAC behind NAT that discovers its public IP via a `received=` param has no way to propagate that into subsequent registrations or re-INVITEs. |
| **STUN / ICE** | ❌ | No implementation anywhere. A `stun_server` field exists in the older session-core v1 builder but is dead code — never read. |
| **SIP Outbound (RFC 5626) keepalive + flow-id** | ❌ | Not implemented. Required by some carriers for registration behind NAT. CRLF keepalive on TCP/TLS is the common ask. |
| **`public_address` / `external_ip` config knob** | ❌ | `Config` only exposes `local_ip` (the bind address). No way to tell the stack "I'm behind NAT, my public address is X" short of STUN. |
| **Digest `nc` counter tracking** | ⚠️ | Hard-coded `00000001` at `auth-core/src/sip_digest.rs:354`. Some strict servers reject duplicate `nc` across multi-challenge sequences. |
| **Digest `auth-int` qop** | ❌ | Only `qop=auth`. Rare requirement. |
| **Digest algorithms** | ⚠️ | MD5 and SHA-256 present; `-sess` variants (RFC 8760) not implemented. |
| **Multiple Contact / failover** | ❓ | Not investigated yet. Matters for active/standby carrier pools. |

**Concrete carrier readiness by target:**

- **Asterisk / FreeSWITCH on LAN, IP-based endpoints**: ✅ should work today. REGISTER with an IP registrar + INVITE by hostname are both fine; digest auth is complete.
- **Carrier with hostname-only REGISTER (Twilio, Vonage, Bandwidth)**: ✅ unblocked — REGISTER now resolves hostnames via system DNS. A/AAAA only (no SRV/NAPTR yet, so no geo-failover via `_sip._udp`).
- **Carrier requiring TLS 5061**: ❌ blocked on TLS transport (sip-transport consolidation + session-core-v3 config wiring). Multi-day fix.
- **UAC behind NAT reaching a public carrier**: ⚠️ partial — outgoing Via now requests `rport` and response `received=`/`rport=` is honored, so short-lived calls and fresh REGISTER round-trips work through NAT. Contact rewrite from discovered NAT address is still pending, so long-duration registrations and inbound in-dialog requests after the pinhole expires remain brittle.

**Recommendation**: the "ship path" is (1) ~~DNS in REGISTER~~ ✅, (2) ~~outgoing rport~~ ✅ / Contact rewrite from received/rport, (3) TLS. Items 1 and the outgoing-rport half of item 2 are now done (v0.2 hardening pass). Contact rewrite, TLS, RFC 3263 SRV/NAPTR, STUN, and SIP Outbound are follow-on work for the edges that need them.

### Audit findings (resolved — see docs)

- Hold/resume previously used UPDATE instead of re-INVITE (Timer F timeouts in logs). **Fixed** — hold/resume now uses re-INVITE per RFC 3261. See `UPDATE_STATUS.md`.
- IncomingCall Drop previously auto-rejected unconditionally, racing with dispatch's decision. **Fixed** — Drop now only fires on panic, sends 500 Server Internal Error per RFC 3261 §21.5.1.
- 4xx/5xx/6xx responses were dropped by dialog-core's event_hub. **Fixed** — all final failure responses now propagate to session-core-v3 as `CallFailed` with correct status code.
- UAS mid-dialog response routing picked the wrong server transaction when both INVITE-server (retained for retransmission) and a later UPDATE-server / reINVITE-server were live for the same dialog, so in-dialog 200 OKs were built with the INVITE's Via/branch and the UAC saw "phantom" INVITE retransmissions instead of a real UPDATE answer. **Fixed** — `dialog-core/src/api/unified.rs::send_response_for_session` now filters candidate transactions on `is_server()` + open state (Initial/Trying/Proceeding) and prefers non-INVITE when both are live; `dialog-core/src/protocol/update_handler.rs::process_update_in_dialog` now inserts the new UPDATE server-tx into `transaction_to_dialog` the same way the re-INVITE path does. Exposed by the session-timer `await_tx_outcome` work; invisible under the previous optimistic `SessionRefreshed`-on-send behaviour.
- UAS-side re-INVITE / UPDATE were silently dropped. **Fixed** (see `REINVITE_WIRING_PLAN.md`). Three bugs compounded: (1) dialog-core's `handle_unassociated_transaction_event` routed every inbound INVITE — including re-INVITEs in an established dialog — straight to `handle_initial_invite`; a `find_dialog_for_request` check was added, mirroring the REFER arm. (2) `event_hub.rs::convert_coordination_to_cross_crate` had no arm for `SessionCoordinationEvent::ReInvite`, so the cross-crate event never reached session-core-v3; new arm filters on `Method::Invite | Method::Update` and emits `DialogToSessionEvent::ReinviteReceived { method }`. (3) session-core-v3's YAML `DialogACK` event name wasn't registered in `parse_event_by_name`, so the `UAS/Answering + DialogACK → Active` transition was stored under `MediaEvent("DialogACK")` and never matched — the UAS was permanently stuck in `Answering`. Now a UAC's ACK correctly promotes the UAS to `Active`, which is the precondition for `Active + ReinviteReceived → 200 OK` to fire.

---

## Test coverage

| Scenario | Covered |
|----------|---------|
| Successful call setup + teardown | ✅ (hello, auto_answer, etc.) |
| Call rejection (403, 404, 486, etc.) | ✅ (routing example) |
| Call cancel before answer (487) | ✅ (`tests/cancel_integration.rs::cancel_emits_callcancelled_event` — multi-binary; exercises the full UAC-hangup → CANCEL → 200 OK → 487 → `Event::CallCancelled` round trip) |
| Panic in handler → 500 response | ✅ (panic_safety_test) |
| REGISTER + digest auth | ✅ (registration example) |
| PRACK 420 policy mismatch | ✅ (`tests/prack_integration.rs` — multi-binary) |
| INVITE + REGISTER 401/407 digest auth wiring | ✅ (`tests/invite_auth_tests.rs` — unit-level state-table coverage; multi-binary end-to-end blocked on a challenging-UAS fixture) |
| PRACK positive reliable-183 flow | ✅ (`tests/prack_integration.rs::prack_positive_reliable_183_flow` — multi-binary; uses `send_early_media`) |
| Session timer refresh (UPDATE, UAC refresher) | ✅ (`tests/session_timer_integration.rs`) |
| Session timer refresh-failure BYE | ✅ (`tests/session_timer_failure_integration.rs` — multi-binary; Bob accepts the call and exits before first refresh → Alice's UPDATE + re-INVITE both time out → BYE carries `Reason: SIP ;cause=408 ;text="Session expired"` per RFC 4028 §10 → `SessionRefreshFailed` fires. `dialog-core/src/manager/session_timer.rs` now subscribes to each refresh transaction's outcome via `TransactionManager::subscribe_to_transaction` plus a `last_response` peek for the race case; `SessionRefreshed` fires only on 2xx.) |
| REGISTER + 423 retry | ✅ (`tests/register_423_retry.rs` — in-process raw-UDP mock registrar returns 423 + Min-Expires, asserts retry carries the bumped Expires and `is_registered` flips on 200 OK) |
| Blind transfer | ✅ (blind_transfer example) |
| Hold/resume | ✅ (hold_resume example — UAS side now runs through the state machine: `Active + ReinviteReceived → NegotiateSDPAsUAS + SendSIPResponse(200)`) |
| UAS-side re-INVITE/UPDATE dispatch | ✅ (dialog-core's `handle_unassociated_transaction_event` now re-checks `find_dialog_for_request` for INVITE before treating it as initial, so re-INVITEs route through `handle_reinvite` + the new `ReInvite → ReinviteReceived` cross-crate conversion arm; session-core-v3 state-table entries for `ReinviteReceived` / `UpdateReceived` respond 200 OK via `SendSIPResponse`) |
| 3xx redirect follow | ✅ (`tests/redirect_follow.rs` — in-process raw-UDP mock UAS returns 302 + Contact; asserts the UAC re-issues INVITE to the redirect target) |
| 491 glare retry | ✅ (`tests/glare_retry_integration.rs` — multi-binary; Alice and Bob simultaneously `hold()` → each side's `HasPendingReinvite`-guarded YAML transition answers 491 → `ReinviteGlare` schedules retry → both converge on stable OnHold) |
| 3xx redirect send (UAS) | ✅ (wired through `UnifiedCoordinator::redirect_call` + `CallHandlerDecision::Redirect`; dialog-core builds the response with `Contact:` via `UnifiedDialogApi::send_redirect_response_for_session`) |
| `accept_call_with_sdp` (b2bua bridge-through) | ✅ (public API on `UnifiedCoordinator`; the `GenerateLocalSDP`/`NegotiateSDPAsUAS` actions no-op when caller has already populated `local_sdp + sdp_negotiated`) |
| Auto-cleanup of terminal sessions | ✅ (terminal events — `CallEnded`, `CallFailed`, `CallCancelled` — now release the session from the store and registry after publish via `publish_and_release_session`; prevents `SessionStore` leaks in long-running peers and b2bua) |
| Clean `CallbackPeer::run` shutdown | ✅ (in-flight handler spawns are tracked in a `JoinSet` and drained before `run()` returns; `run()` = `Ok(())` now guarantees all user-handler tasks have completed) |
| `StreamPeer::shutdown_handle()` | ✅ (symmetric with `CallbackPeer::shutdown_handle`; clonable and safe to pass to a supervisor) |

Tests to add would fill the "no test" gaps above. 423 REGISTER retry is covered by an in-process raw-UDP mock registrar in `tests/register_423_retry.rs`. 491 glare was previously covered by a state-table unit test; it's now a real multi-binary integration test (`tests/glare_retry_integration.rs`) using the `cancel_integration.rs` two-subprocess pattern.
