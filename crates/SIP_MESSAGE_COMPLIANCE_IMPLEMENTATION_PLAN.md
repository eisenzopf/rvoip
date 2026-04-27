# SIP Message Compliance Implementation Plan

## Purpose

Fix the malformed outbound SIP messages discovered while running
`crates/session-core/examples/asterisk`, then prevent regressions across the
common SIP request and response types.

The immediate failure is an outbound `REGISTER` that reaches Asterisk without a
`Call-ID`. The same construction path also omits `Max-Forwards` and
`Content-Length: 0`. This should not be possible for a wire-bound SIP request.

This plan keeps the crate ownership model intact:

- `session-core` owns session intent and authoritative values such as AoR,
  registration contact, credentials, expiry, authentication challenge state, and
  retry policy.
- `dialog-core` owns SIP request/response construction, transaction routing,
  dialog semantics, and wire-readiness checks before transport send.
- `sip-core` owns SIP message data types, serialization/parsing, and reusable
  protocol validation helpers.
- `sip-transport` sends already-valid SIP messages and should not be expected to
  repair malformed protocol data.

## Findings

### Primary REGISTER bug

Outbound registration enters `dialog-core` through
`UnifiedApi::send_register_impl` in `dialog-core/src/api/unified.rs`.

That function currently builds a `REGISTER` with `SimpleRequestBuilder` and
adds:

- `From`
- `To`
- `Contact`
- `Expires`
- `CSeq`
- optional `Authorization`

It does not add:

- `Call-ID`
- `Max-Forwards`
- `Content-Length: 0`

The transaction manager adds or replaces `Via`, but it does not add the other
mandatory request headers. `Message::to_bytes()` serializes exactly the headers
present on the request. Therefore the malformed message is created before
transport.

### Existing better builder is bypassed

`dialog-core/src/transaction/client/builders.rs` already has a
`RegisterBuilder` that generates:

- `Call-ID`
- `From` tag
- `Via`
- `Max-Forwards`
- `Contact`
- `Content-Length: 0`

However, `UnifiedApi::send_register_impl` bypasses this builder.

### REGISTER Contact semantics

For `REGISTER`, `Contact` is the local UA binding being registered with the
registrar. It is not the registrar or remote server contact.

`session-core` is correct to pass the authoritative contact value to
`dialog-core`. The Asterisk example should be reviewed because it currently
sets `config.local_uri` using the remote SIP server address. When
`Registration::new(...)` defaults `contact_uri` to `Config.local_uri`, that can
make the `Contact` point at Asterisk instead of this client.

The desired model:

- Request-URI: registrar URI, for example `sip:pbx.example.com:5060`
- To: AoR being registered, for example `sip:1001@pbx.example.com`
- From: same AoR, with a local tag
- Contact: reachable UA contact, for example `sip:1001@192.168.1.50:5070`
- Authorization digest URI: registrar URI used for the REGISTER request

### Other at-risk paths

Most in-dialog requests use dialog-aware builders or helpers that add the
common SIP headers. The main risk is out-of-dialog request construction.

Known paths to audit and fix:

- Outbound `REGISTER` in `dialog-core::UnifiedApi::send_register_impl`
- Out-of-dialog `SUBSCRIBE` in `session-core::DialogAdapter::send_subscribe`
- Out-of-dialog `MESSAGE` in `session-core::DialogAdapter::send_message`
- Any public `send_non_dialog_request` callers that pass a partially built
  request
- Utility builders in `dialog-core/src/transaction/client/builders.rs`
- Request helpers in `dialog-core/src/transaction/method/*`
- Response builders in `dialog-core/src/transaction/utils/response_builders.rs`

## Implementation Goals

1. Every outbound SIP request sent by `dialog-core` has RFC 3261 core headers
   before it reaches transport.
2. Empty-body messages carry `Content-Length: 0`.
3. Body-bearing messages carry a `Content-Length` equal to the byte length of
   the body.
4. `REGISTER` uses correct AoR and Contact semantics.
5. Registration `Call-ID` and `CSeq` behavior is explicit and stable across
   auth retries, refreshes, unregister, and 423 retry.
6. Missing mandatory headers are caught by automated tests and optionally by a
   runtime validator before send.

## Phase 1: Fix Outbound REGISTER Construction

### 1.1 Move REGISTER construction onto a compliant dialog-core builder

Update `UnifiedApi::send_register_impl` to stop hand-building a partial
`REGISTER` with `SimpleRequestBuilder::register`.

Preferred approach:

- Extend `dialog-core::transaction::client::builders::RegisterBuilder` so it can
  represent REGISTER correctly for UAC registration:
  - `registrar(registrar_uri)` sets Request-URI.
  - `aor(from_uri)` or `to_uri(aor_uri)` sets To.
  - `user_info(from_uri, display_name)` sets From.
  - `contact(contact_uri)` sets Contact.
  - `expires(seconds)` sets Expires.
  - `authorization(header)` adds Authorization.
  - `outbound_contact_params(params)` or direct `Contact` override preserves the
    existing RFC 5626 path.
  - `local_address(addr)` supplies Via default host/port.
  - `call_id(call_id)` allows stable registration Call-ID.
  - `cseq(cseq)` allows session-core to supply the next registration CSeq.

Alternative if keeping the patch small:

- In `send_register_impl`, add the missing headers explicitly:
  - generate `Call-ID`
  - add `.max_forwards(70)`
  - add `TypedHeader::ContentLength(ContentLength::new(0))`
  - rely on the transaction manager for Via

The preferred builder approach is safer because it centralizes REGISTER shape
in `dialog-core`.

### 1.2 Correct REGISTER To header behavior

Current `RegisterBuilder` uses registrar URI as `To`. That is not ideal for
client registration. Adjust it so:

- Request-URI remains registrar URI.
- `To` is the AoR, normally equal to `From` without a tag.
- `From` is the AoR with a local tag.

Keep backwards-compatible defaults where possible:

- If no explicit AoR/To URI is provided, use `from_uri`.
- If neither exists, return a builder error.

### 1.3 Preserve Contact as local binding

Do not derive `Contact` from registrar URI.

Use the `contact_uri` supplied by `session-core`, after any NAT/public-address
rewrite already performed by `DialogAdapter::send_register`.

For RFC 5626 outbound registration, preserve existing behavior:

- add `;ob` to the Contact URI
- add `+sip.instance`
- add `reg-id`

### 1.4 Add REGISTER wire-shape tests

Add focused tests in `dialog-core` for `send_register_impl` or the updated
`RegisterBuilder`:

- initial REGISTER has `Via`, `From`, `To`, `Call-ID`, `CSeq`, `Max-Forwards`,
  `Contact`, `Expires`, and `Content-Length: 0`
- authenticated REGISTER preserves all required headers and adds
  `Authorization`
- 423 retry REGISTER preserves all required headers and uses the server
  `Min-Expires`
- unregister sends `Expires: 0` and required headers
- outbound Contact mode includes `;ob`, `+sip.instance`, and `reg-id`

## Phase 2: Registration Call-ID and CSeq State

### 2.0 Dialog-core CSeq ownership boundary

This phase applies only to outbound non-dialog `REGISTER` requests.

It must not move general SIP CSeq ownership into `session-core`.
`dialog-core` already owns dialog sequencing through `Dialog.local_cseq` and
`Dialog.remote_cseq`. In-dialog requests such as `INVITE`, re-INVITE, `BYE`,
`UPDATE`, `REFER`, `NOTIFY`, `MESSAGE`, `INFO`, and `PRACK` must continue to
use dialog-core's dialog request template path, which increments
`Dialog.local_cseq` for each new in-dialog request.

`REGISTER` is different because it is a non-dialog request. It does not create
or mutate a `Dialog`, and the non-INVITE transaction layer matches responses by
Via branch and CSeq method. Therefore a registration-scoped Call-ID/CSeq stored
by `session-core` will not conflict with dialog-core's dialog state machine.

The boundary:

- `session-core` may store and advance `registration_call_id` and
  `registration_cseq` for registration lifecycle transactions only.
- `dialog-core` must still generate/manage transaction Via branches and reuse
  the same request for retransmissions.
- `session-core` must increment registration CSeq only when starting a new
  logical REGISTER transaction, not for transaction-layer retransmits.
- All dialog-associated request CSeq values remain owned by `dialog-core`.

### 2.1 Add registration transaction identity to session state

Add fields to the session registration state, likely in
`session-core/src/session_store/state.rs`:

- `registration_call_id: Option<String>`
- `registration_cseq: u32`

Expected behavior:

- First REGISTER creates and stores a registration Call-ID.
- Auth retry reuses the same Call-ID and increments CSeq.
- 423 retry reuses the same Call-ID and increments CSeq.
- Refresh REGISTER reuses the same Call-ID and increments CSeq.
- Unregister reuses the same Call-ID and increments CSeq.

This avoids the current `cseq = authorization.is_some() ? 2 : 1` behavior in
`dialog-core`, which is not sufficient for refreshes and multiple retries.

### 2.2 Keep ownership clean

`session-core` should not build SIP messages. It should pass registration
metadata to `dialog-core` for non-dialog REGISTER construction:

- registrar URI
- AoR/from URI
- contact URI
- expires
- optional authorization header
- registration Call-ID
- next CSeq
- optional outbound Contact params

`dialog-core` should build and validate the `Request`.

This is a narrow exception to dialog-core's normal CSeq ownership: the CSeq
value is supplied by `session-core` only because registration lifecycle state
lives there and REGISTER is not attached to a dialog. It must not be reused as a
pattern for in-dialog methods.

### 2.3 API adjustment

Adjust `dialog-core::UnifiedApi::send_register` and
`send_register_with_outbound_contact` to accept optional Call-ID/CSeq inputs, or
introduce a small options struct:

```rust
pub struct RegisterRequestOptions {
    pub registrar_uri: String,
    pub aor_uri: String,
    pub contact_uri: String,
    pub expires: u32,
    pub authorization: Option<String>,
    pub call_id: Option<String>,
    pub cseq: Option<u32>,
    pub outbound_contact: Option<OutboundContactParams>,
}
```

This avoids widening method signatures repeatedly as registration behavior
grows.

## Phase 3: Add Wire-Ready SIP Validation

### 3.1 Add reusable validator in sip-core

Add a validator module in `sip-core`, for example:

- `sip-core/src/validation/wire.rs`

Candidate APIs:

```rust
pub fn validate_wire_request(request: &Request) -> Result<()>;
pub fn validate_wire_response(response: &Response) -> Result<()>;
pub fn validate_content_length(headers: &[TypedHeader], body_len: usize) -> Result<()>;
```

Validation should check:

- exactly one or at least one top `Via` for requests and responses
- `From` present
- `To` present
- `Call-ID` present
- `CSeq` present
- `Max-Forwards` present on requests except `ACK` generated for non-2xx may be
  handled according to existing helper behavior
- `Content-Length` present
- `Content-Length` equals `body.len()`
- `Content-Type` present when body length is greater than zero
- request-specific checks where they are unambiguous:
  - `REGISTER` has `Contact` unless querying bindings intentionally
  - `INVITE` dialog-creating requests have `Contact`
  - `SUBSCRIBE` has `Event` and `Contact` when creating a subscription dialog
  - `NOTIFY` has `Event` and `Subscription-State`
  - `REFER` has `Refer-To`

Keep strictness configurable. A general wire validator should enforce RFC 3261
core framing, while method validators can enforce method-specific rules.

### 3.2 Use validator in dialog-core before transaction creation

Before `dialog-core` creates client transactions, validate outbound requests:

- `send_non_dialog_request`
- in-dialog send path
- direct ACK send helpers
- CANCEL creation path

For the first implementation, fail fast in debug/test or return a protocol
error in all builds. The safer long-term behavior is to return an error before
transport rather than emitting malformed SIP.

### 3.3 Avoid transport-side repair

Do not make `sip-transport` add missing headers. Transport should serialize and
send only. If needed, transport can log validation failures, but construction
must be fixed above it.

## Phase 4: Fix Other Out-of-Dialog Message Paths

### 4.1 SUBSCRIBE

Current `session-core::DialogAdapter::send_subscribe` builds a request directly
with `SimpleRequestBuilder::subscribe(...)` and only adds `From` and `To`.

Move construction into `dialog-core`, for example:

- add `UnifiedApi::send_subscribe(...)`
- add a `SubscribeBuilder` in `dialog-core`
- have `session-core` pass intent values only

Required outbound SUBSCRIBE shape:

- Request-URI target
- `Via`
- `From`
- `To`
- `Call-ID`
- `CSeq`
- `Max-Forwards`
- `Contact` for dialog creation
- `Event`
- `Expires`
- `Accept` where appropriate
- `Content-Length: 0` unless body is present

### 4.2 Out-of-dialog MESSAGE

Current standalone MESSAGE in `session-core::DialogAdapter::send_message` builds
a request directly and omits several core headers.

Move construction into `dialog-core`, for example:

- add `UnifiedApi::send_message_out_of_dialog(...)`
- use a dialog-core request builder that adds core headers

Required outbound MESSAGE shape:

- Request-URI target
- `Via`
- `From`
- `To`
- `Call-ID`
- `CSeq`
- `Max-Forwards`
- `Content-Type` when body exists
- `Content-Length` matching body bytes

### 4.3 Public send_non_dialog_request

`send_non_dialog_request` currently accepts an arbitrary `Request`. Keep it for
advanced use, but validate before transaction creation. This provides a guard
for external callers that build their own requests.

## Phase 5: Response Compliance

### 5.1 Audit response builders

Review `dialog-core/src/transaction/utils/response_builders.rs` and response
paths for:

- copied `Via`
- copied `From`
- copied `To`
- copied `Call-ID`
- copied `CSeq`
- `Content-Length: 0` on empty body
- correct `Contact` on dialog-establishing 2xx responses
- auth challenge headers on 401/407
- `Min-Expires` on 423

### 5.2 Validate outbound responses

Apply `validate_wire_response` before server transactions send responses.

Initial response validator should check:

- `Via`
- `From`
- `To`
- `Call-ID`
- `CSeq`
- `Content-Length`
- body length agreement

Method/status-specific response validation can be phased in after the core
framing checks.

## Phase 6: Test Matrix

### 6.1 Unit tests

Add unit tests around builders and validators:

- REGISTER initial/auth/retry/unregister
- INVITE with SDP
- INVITE without SDP where supported
- ACK
- BYE
- CANCEL
- UPDATE with and without SDP
- REFER
- NOTIFY
- SUBSCRIBE
- MESSAGE
- OPTIONS
- basic 100/180/200/401/407/423/487 responses

Each test should assert:

- required headers exist
- `Content-Length` exists
- `Content-Length == body.len()`
- serialized bytes parse back through `sip-core`

### 6.2 Session-core integration tests

Add a UDP mock registrar test for outbound REGISTER:

- capture raw bytes from `StreamPeer::register_with`
- parse with `sip-core`
- assert REGISTER method and required headers
- assert `Contact` equals local/contact URI, not registrar URI
- respond 401 and assert retry:
  - same Call-ID
  - incremented CSeq
  - Authorization present
- respond 423 and assert retry:
  - same Call-ID
  - incremented CSeq
  - Expires uses `Min-Expires`
- respond 200 and assert session registered

Extend the existing `register_423_retry` test or add a new
`sip_message_compliance.rs` test to keep assertions focused.

### 6.3 Dialog-core tests

Add tests for `UnifiedApi::send_register_impl` or lower-level builder output
without requiring a real network server when possible.

Add tests for `send_non_dialog_request` rejecting malformed requests:

- missing `Call-ID`
- missing `Max-Forwards`
- missing `Content-Length`
- body with mismatched `Content-Length`

## Phase 7: Asterisk Example Fix

Review `crates/session-core/examples/asterisk/softphone.rs`.

The example currently sets:

```rust
config.local_uri = format!(
    "sip:{}@{}:{}{}",
    username, sip_server, sip_port, transport_suffix
);
```

That makes the default Contact point at the remote server. Change it so:

- AoR/from URI can remain `sip:{username}@{sip_server}` or
  `sip:{username}@{sip_server}:{sip_port}` depending on Asterisk expectations.
- Contact URI is local UA reachability, for example
  `sip:{username}@{local_ip}:{local_port}`.

Because `Registration::new(...)` defaults both From and Contact to
`Config.local_uri`, the example should explicitly set one or both:

```rust
let reg = Registration::new(&registrar_uri, &auth_user, &password)
    .from_uri(format!("sip:{}@{}", username, sip_server))
    .contact_uri(format!("sip:{}@{}:{}", username, advertised_ip, local_port));
```

If `LOCAL_IP=0.0.0.0`, do not use that value as Contact. Require an
`ADVERTISED_IP` or derive a local interface address. NAT discovery can improve
subsequent REGISTERs, but the first REGISTER should still avoid an invalid
`Contact: sip:user@0.0.0.0:port`.

## Proposed Work Order

1. Add sip-core wire validation helpers for core request/response framing.
2. Fix dialog-core REGISTER builder and route `send_register_impl` through it.
3. Add REGISTER builder and integration tests.
4. Add session-core registration Call-ID/CSeq state.
5. Fix Asterisk example Contact/AoR handling.
6. Move out-of-dialog SUBSCRIBE and MESSAGE construction into dialog-core.
7. Add send-time validation to dialog-core transaction entry points.
8. Expand method-specific validation and tests.

## Open Decisions

1. Should malformed outbound requests be hard errors in all builds, or warnings
   outside tests initially?

   Recommendation: hard errors. Sending malformed SIP creates confusing remote
   failures and makes diagnostics harder.

2. Should registration Call-ID/CSeq live only in session-core, or should
   dialog-core maintain registration client state?

   Recommendation: session-core owns it because registration is session intent
   and retry policy. This is safe because REGISTER is non-dialog and does not
   touch `Dialog.local_cseq` / `Dialog.remote_cseq`. dialog-core should accept
   explicit registration Call-ID/CSeq and build a valid request.

3. Should `send_non_dialog_request` remain public?

   Recommendation: yes, but it should validate wire readiness before creating a
   transaction. Higher-level APIs should be preferred for common methods.

4. How strict should REGISTER Contact validation be?

   Recommendation: start with structural validation only, plus tests asserting
   our own examples pass local UA Contact. Do not reject all registrar-host
   Contacts generically because some deployments may use edge cases, aliases, or
   outbound proxy topologies.

## Acceptance Criteria

- Asterisk receives a REGISTER with `Call-ID`, `Max-Forwards`, and
  `Content-Length: 0`.
- REGISTER Contact is the local UA binding supplied by session-core, not the
  registrar URI.
- REGISTER auth retry uses the same Call-ID and a higher CSeq.
- REGISTER refresh and unregister use the same registration Call-ID and higher
  CSeq.
- Empty-body outbound SIP messages include `Content-Length: 0`.
- Body-bearing outbound SIP messages have exact Content-Length.
- Common outbound SIP requests pass the validator before transport.
- Tests fail if a common request path regresses to missing core SIP headers.
