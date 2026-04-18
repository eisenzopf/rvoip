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

### 4xx Client Error

| Code | Status | Behavior |
|------|--------|----------|
| 400 Bad Request | ✅ | Emits `Event::CallFailed { status_code: 400, … }`; session → Failed |
| 401 Unauthorized | ✅ | RFC 3261 §22.2. INVITE and REGISTER both auto-retry once with `Authorization:` computed from `StreamPeerBuilder::with_credentials` / `PeerControl::call_with_auth`. Unified state-machine path (`EventType::AuthRequired` → `StoreAuthChallenge` + `SendINVITEWithAuth` / `SendREGISTERWithAuth`). |
| 403 Forbidden, 404 Not Found, 486 Busy, etc. | ✅ | Emits `Event::CallFailed { status_code, reason_phrase, … }` |
| 407 Proxy Authentication Required | ✅ | Same path as 401, but uses `Proxy-Authorization` header. Applies to INVITE and REGISTER. |
| 422 Session Interval Too Small | ✅ | RFC 4028 §6. UAS emits with `Min-SE:` when peer's `Min-SE` exceeds our `Session-Expires`. UAC retry-with-bumped-Min-SE is pending (carriers rarely 422 on fresh INVITE). |
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
5. **Attended transfer with Replaces on the transferred leg** — Implemented but limited test coverage.
6. **422 UAC-side retry** — UAS emits 422 Session Interval Too Small + Min-SE when peer's Min-SE exceeds our Session-Expires, but the UAC doesn't auto-retry with a bumped Session-Expires (rare in practice).
7. **Session-timer BYE Reason header** — the 408 cause is surfaced via the `SessionRefreshFailed` event string; a proper `Reason: SIP ;cause=408;text="Session expired"` header on the BYE (RFC 4028 §10 nicety) is not yet added.

### Carrier / real-world interop (not RFC gaps, but block production use)

The RFC-level plumbing above is largely complete for basic SIP, but a
handful of transport / networking capabilities determine whether a
session-core-v3 UAC can actually talk to production carriers (Twilio,
Vonage, Bandwidth, enterprise PBXs behind NAT). Audit performed; specific
file references below.

| Capability | Status | Details |
|------------|--------|---------|
| **DNS — INVITE target** | ✅ | `dialog-core/src/dialog/dialog_utils.rs:125-174` `resolve_uri_to_socketaddr` uses `ToSocketAddrs` / `tokio::net::lookup_host`. A UAC can `call("sip:bob@pbx.example.com")` and it will resolve via system DNS. Silent failure on DNS error (logged, not surfaced as `CallFailed` with a clear reason). |
| **DNS — REGISTER target** | ❌ | `dialog-core/src/api/unified.rs:890-906`: `send_register` explicitly rejects hostnames with `ApiError::protocol("Cannot parse domain as IP (DNS not implemented)")`. A user cannot `register` against `sip.twilio.com` today — they must pre-resolve to an IP. **This is the likely first carrier blocker.** Fix is small: mirror `resolve_uri_to_socketaddr` in `send_register`. |
| **RFC 3263 SRV + NAPTR** | ❌ | No `trust-dns` / `hickory` / `_sip._udp` / `NAPTR` anywhere in the workspace. Only system A/AAAA fallback. Impact: can't reach carriers that publish SRV priority/weight for geo-failover; won't auto-select TCP/TLS vs UDP per NAPTR. |
| **TLS transport (`sips:` / 5061)** | ❌ | Two concerns: (a) `sip-transport` has two TLS implementations — `crates/sip-transport/src/tls.rs` is a placeholder that returns `Error::NotImplemented`, `crates/sip-transport/src/transport/tls/mod.rs` is more complete (rustls-based server handshake) but not consolidated. (b) Even if that's sorted, session-core-v3 **hardcodes `enable_tls: false`** at `src/api/unified.rs:585` inside `create_dialog_api`. `Config` has no `tls_cert_path` / `tls_key_path` fields. To reach Twilio/Vonage this needs: finish sip-transport's TLS client-side connector, add config fields, flip the hardcoded flag. |
| **TCP transport** | ❌ | Same pattern as TLS: `TransportManagerConfig` supports TCP (`crates/dialog-core/src/transaction/transport/mod.rs`), but session-core-v3's `create_dialog_api` hardcodes `enable_tcp: false`. Some PBXs fall back to TCP for large SDP / video. |
| **Outgoing `rport` (RFC 3581)** | ❌ | `create_via_header` at `crates/dialog-core/src/transaction/manager/handlers.rs:634-654` adds only `branch`. Comment at 641-642 acknowledges rport isn't added. Carriers often require `rport` for NAT keepalive to work. |
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
- **Carrier with hostname-only REGISTER (Twilio, Vonage, Bandwidth)**: ❌ blocked on DNS-in-REGISTER. ~4-hour fix.
- **Carrier requiring TLS 5061**: ❌ blocked on TLS transport (sip-transport consolidation + session-core-v3 config wiring). Multi-day fix.
- **UAC behind NAT reaching a public carrier**: ⚠️ partial — response rport is honored, but outgoing Via doesn't request rport, and Contact isn't rewritten. Works for short-lived calls where the pinhole stays open; breaks for long registrations or inbound in-dialog requests.

**Recommendation**: the "ship path" is (1) DNS in REGISTER, (2) outgoing rport + Contact rewrite from received/rport, (3) TLS. That unblocks most deployments. RFC 3263, STUN, and SIP Outbound are follow-on work for the edges that need them.

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
| PRACK 420 policy mismatch | ✅ (`tests/prack_integration.rs` — multi-binary) |
| INVITE + REGISTER 401/407 digest auth wiring | ✅ (`tests/invite_auth_tests.rs` — unit-level state-table coverage; multi-binary end-to-end blocked on a challenging-UAS fixture) |
| PRACK positive reliable-183 flow | ✅ (`tests/prack_integration.rs::prack_positive_reliable_183_flow` — multi-binary; uses `send_early_media`) |
| Session timer refresh (UPDATE, UAC refresher) | ✅ (`tests/session_timer_integration.rs`) |
| Session timer refresh-failure BYE | ⚠️ (wire-level implemented; test blocked on session-core-v3 API for dropping UPDATE) |
| REGISTER + 423 retry | ❌ (no test — would need a 423-returning registrar mock) |
| Blind transfer | ✅ (blind_transfer example) |
| Hold/resume | ✅ (hold_resume example) |
| 3xx redirect follow | ❌ (no test — would need a 302-returning UAS mock) |
| 491 glare retry | ❌ (no test — would need simultaneous re-INVITE scenario) |

Tests to add would fill the "no test" gaps above. The wire logic is in place for all of them.
