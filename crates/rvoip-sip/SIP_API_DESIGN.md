# Gateway-grade SIP header API for `rvoip-sip` (inbound + outbound)

**Status:** approved. Layer-separation and completeness review complete;
implementation may begin per the phases below.

## Context

`rvoip-sip` advertises itself for "softphones, test clients, IVRs, B2BUA
legs, routing servers, and PBX/SBC interop tools". `server::*` modules,
`examples/sip_b2bua.rs`, and `examples/unified/04_b2bua_bridge/` are
first-class. But the public API is shaped almost entirely around
*endpoint* use cases — a peer that places and receives calls and lets
the library hide SIP wire details. Gateway, B2BUA, SBC, and call-center
applications need the opposite: full inspection on incoming SIP, full
authorship on outgoing SIP, and easy composition so an inbound request
can be transformed and re-sent on the other leg — without breaking
RFC-3261 correctness or layer boundaries.

The crate exposes four public API surfaces: **Endpoint, Peer (StreamPeer),
Callback (CallbackPeer), and Unified (UnifiedCoordinator)**. Every
header-authoring affordance described in this document must be reachable
from all four with consistent shape.

Today's gaps:

- **Inbound:** `IncomingCall.headers: HashMap<String, String>` exists but
  is never populated (`src/api/incoming.rs:80`). `IncomingCallInfo`
  carries only a hand-picked subset (from, to, sdp, p_asserted_identity).
  No public API surfaces the typed inbound `Request` for inspection.
  Mid-dialog received requests (REFER, NOTIFY, INFO) reach the
  application only via pre-decoded `Event::ReferReceived` /
  `Event::NotifyReceived` with fixed fields — original headers are
  dropped.
- **Outbound:** every public SIP request is a fixed-parameter method:
  `make_call_with_auth`, `make_call_with_pai`, `make_call_with_headers`,
  `send_refer`, `send_refer_with_replaces`, `send_notify(event, body,
  state)`, `send_info(content_type, body)`, `register_with(Registration)`.
  Compound combinations (auth + headers, pai + headers, custom NOTIFY
  headers, REFER with Referred-By, …) require either explosive
  `_with_X_and_Y_and_Z` overloads or feature gaps that get documented as
  "out of scope for this release".
- **Response side:** `reject_call(status, reason)`, `redirect_call(status,
  contacts)`, `accept_call_with_sdp(sdp)` — no path to attach custom
  headers to outgoing responses (`Retry-After`, `Warning`, vendor routing
  hints, etc.). No typed 401/407 challenge authoring.
- **Layer pass-through gap:** `rvoip-sip-dialog`'s public API accepts
  `Vec<TypedHeader>` only for INVITE
  (`make_call_with_extra_headers_for_session`). REGISTER, REFER, NOTIFY,
  INFO, BYE, UPDATE, SUBSCRIBE all take fixed parameters with no
  application-header carrier. Any uniform `with_header` affordance in
  `rvoip-sip` therefore requires an additive extension to dialog-core's
  public API — the alternative (authoring requests inside `rvoip-sip`)
  duplicates CSeq, Route-Set, and Contact logic that lives in dialog-core
  and violates the layer boundary.
- **Header-safety gap:** today nothing prevents an application from
  attaching `Call-ID`, `CSeq`, `Via`, `Max-Forwards`, or `From` via the
  raw `extra_headers` channel. Each of these corrupts the dialog or the
  transaction and produces non-RFC wire output.

Goal: ship a uniform, builder-shaped SIP request/response API that makes
"inspect, change, add, delete SIP fields" the same shape across every
request type and every direction, with first-class composition primitives
for B2BUA/SBC carry-through — and with guardrails so applications cannot
accidentally desync dialogs or send invalid SIP.

## Layer architecture (verified)

| Crate | Owns | Builds messages? | Holds dialog state? | Bus role |
|---|---|---|---|---|
| `rvoip-sip-transport` | UDP/TCP/TLS/WS sockets | No | No | publishes `TransportEvent`, `SipTraceEvent` |
| `rvoip-sip-core` | `Request`/`Response` types, `TypedHeader`, `HeaderName`, parser, `SimpleRequestBuilder` / `SimpleResponseBuilder`, RFC validators | Yes — raw, generic | No | none (foundation crate, no internal deps) |
| `rvoip-sip-dialog` | Dialog/transaction state, CSeq, Route-Set, in-dialog request authorship via `transaction/utils/{request,response}_builders.rs` + `transaction/dialog/request_builder_from_dialog_template` | Yes — dialog-bound | Yes | consumes `SessionToDialogEvent`, publishes `DialogToSessionEvent`, `DialogCreated/Terminated`, `SipTraceEvent` |
| `rvoip-sip` | Session lifecycle, state machine, the four public API surfaces, `DialogAdapter` | No — delegates | No (via dialog-core) | consumes `DialogToSessionEvent` → `Event`, publishes `SessionToDialogEvent` |
| `infra-common` | `GlobalEventCoordinator`, `EventBus`, `Publisher`, `EventPool`; also **defines** the cross-crate event enums `DialogToSessionEvent` / `SessionToDialogEvent` (`crates/infra-common/src/events/cross_crate.rs:514` and friends) | No | No | bus host. **Has no `rvoip-sip-core` dependency** — cross-crate event payloads cannot reference `rvoip_sip_core::Request` directly without forming a new dependency arrow that the architecture deliberately avoids. |

**Call-path truth** (traced through
`crates/rvoip-sip/src/adapters/dialog_adapter.rs`):

```
UnifiedCoordinator.make_call_with_headers(...)
  → StateMachineHelpers::make_call_with_headers_and_credentials_and_pai
        (state_machine/helpers.rs:156)   // there are 5 sibling helpers
                                         // at lines 99/108/126/140/156
  → stash extra_headers on SessionState.extra_headers (state.rs:190)
  → emit Action::SendINVITE
  → DialogAdapter::send_invite_with_extra_headers (adapters/dialog_adapter.rs:926)
  → UnifiedDialogApi::make_call_with_extra_headers_for_session
        (rvoip-sip-dialog unified.rs:617)
  → builds Request via sip-core builder, sends via transaction layer
```

For the INVITE path the layering is clean and dialog-core's public method
**already accepts `Vec<TypedHeader>`**. For every other method it does not
— see the dialog-core extension section.

> **Note (audit-corrected):** there is no `make_call_inner` helper in
> `state_machine/helpers.rs` today; the INVITE path is a cluster of five
> sibling methods (`make_call`, `make_call_with_credentials`,
> `make_call_with_pai`, `make_call_with_credentials_and_pai`,
> `make_call_with_headers_and_credentials_and_pai`). Phase C
> **introduces** a single `make_call_inner(opts)` that collapses these
> and authors brand-new `send_refer_inner`, `send_notify_inner`,
> `send_info_inner`, `send_bye_inner`, `send_cancel_inner`,
> `send_update_inner`, `send_subscribe_inner`, `send_message_inner`,
> `send_options_inner`, `send_register_inner` siblings — none exist
> today.

## Goals

1. Every public SIP request emitted by `rvoip-sip` is constructable
   through a builder that exposes typed `with_header` / `with_headers` /
   `strip_header` / `with_headers_from` and method-specific setters. No
   more `_with_X_and_Y`.
2. Every public SIP request received by `rvoip-sip` is inspectable
   through a uniform header-view API (`SipHeaderView`). Typed access for
   headers `rvoip-sip-core` knows about, raw access for anything else.
3. B2BUA carry-through is a one-liner:
   `outbound.with_headers_from(&inbound, &[HeaderName::HistoryInfo,
   HeaderName::Diversion])?` — and stack-managed headers are filtered
   automatically with an auditable report.
4. Every builder rejects attempts to attach dialog/transaction-managed
   headers (`Call-ID`, `CSeq`, `Via`, `Max-Forwards`, `Record-Route`,
   `Content-Length`, dialog `From`/`To`). Method-shaped headers
   (`Refer-To`, `Event`, `Subscription-State`, `Authorization`, etc.)
   are only reachable through their dedicated setter — the error
   message names the setter.
5. The flat methods that exist today (`make_call*`, `register*`, etc.)
   stay as `#[deprecated]` wrappers so nothing breaks on upgrade.
6. All four surfaces (`Endpoint`, `StreamPeer`, `CallbackPeer`,
   `UnifiedCoordinator`) expose the same builders with consistent shape.
   Endpoint and Peer surfaces pre-populate `from`/`contact` from the
   surface's local URI; the Endpoint surface continues to run
   `resolve_target` on bare extensions.
7. Dialog-core's public API is extended **additively** to accept
   application headers for every dialog-bound request method. Existing
   dialog-core methods stay and delegate. Dialog state machine, route-set
   logic, transaction core, and CSeq management are not touched.

## Non-goals

- Changing the dialog state machine, route-set logic, transaction core,
  or CSeq management in `rvoip-sip-dialog`. The dialog-core changes are
  additive options structs only.
- Changes to `rvoip-sip-core`. The crate's builders, types, parser, and
  validators are already adequate.
- Changes to `rvoip-sip-transport`. The wire layer is unaffected.
- Re-architecting media or RTP. SDP/RTP are unaffected.
- Removing the deprecated flat methods. They stay through at least
  0.3.0; a separate breaking-change PR removes them later.
- Migrating in-tree examples and tests onto the new API. Follow-up sweep
  PR once the builders are stable.

---

## API surface

### 1. Inbound: `SipHeaderView` — uniform header inspection

A small trait implemented by every type that wraps a received SIP
message. Lives in `src/api/headers/view.rs`.

```rust
pub trait SipHeaderView {
    /// First header matching `name`, typed when sip-core has a variant for it.
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader>;

    /// Every header matching `name`, in wire order. Returns an empty iterator
    /// when none are present. Object-safe via boxed iterator.
    fn headers_named<'a>(&'a self, name: &HeaderName)
        -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a>;

    /// All headers in wire order.
    fn headers<'a>(&'a self)
        -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a>;

    /// Convenience: header value as a string via the typed header's `Display`
    /// impl. For `TypedHeader::Other(name, value)` (every header sip-core
    /// does not have a typed variant for, e.g. `Diversion`, `History-Info`,
    /// `Privacy`, `Replaces`), `Display` reproduces the original wire value.
    /// Returns `None` when the header is missing.
    fn header_str(&self, name: &HeaderName) -> Option<String> {
        self.header(name).map(|h| h.to_string())
    }

    /// All header names present, deduped, in first-seen order. Use for
    /// snapshot logging without walking the typed enum.
    fn header_names(&self) -> Vec<HeaderName>;

    /// Escape hatch: the underlying parsed `Request` shared as `Arc`.
    /// Returns `None` if the wrapper is synthesized rather than parsed.
    /// Sharing as `Arc` keeps zero-copy distribution across event-bus
    /// subscribers (`DialogToSessionEvent` payloads, trace consumers).
    fn raw_request(&self) -> Option<&Arc<rvoip_sip_core::Request>>;
}
```

> Note: an earlier draft included `header_wire_value(&str) -> Option<&str>`.
> Verification of `rvoip-sip-core` shows `Request.headers: Vec<TypedHeader>`
> — the parser converts every header to typed form at parse time and the
> raw header bytes are not retained. `header_str` returns the canonical
> wire-equivalent value via `TypedHeader::Display`; for `Other(_)` variants
> this is the unchanged inbound value. The separate `header_wire_value`
> method is therefore dropped.

Implementors:

- `IncomingCall` — populated for every inbound INVITE
- `IncomingRequest` (new) — for in-dialog received REFER / NOTIFY / INFO
  / OPTIONS / UPDATE / MESSAGE
- `IncomingResponse` (new) — for non-2xx finals when the caller wants to
  inspect `Retry-After`, `Warning`, etc.
- `IncomingRegister` (new) — for inbound REGISTER on registrar surfaces
  (the `rvoip-sip-registrar` crate today reads the raw Request directly;
  this gives it a typed entry point)

The existing `IncomingCall.headers: HashMap<String, String>` field stays
for backwards compatibility but the empty-default bug is fixed
(populated from the parsed INVITE at session creation time). The map is
lowercased single-valued and lossy; the trait above is the recommended
path. `From`, `To`, `Contact`, `Call-ID`, `Diversion`, `History-Info`,
`Referred-By`, `P-Asserted-Identity`, `Privacy`, `Reason`, `Retry-After`,
`Path`, `Service-Route`, `Replaces`, `Refer-To`, all `X-*`, and every
other RFC 3261 header is reachable through `header()`.

### 2. Outbound: `SipRequestOptions` — shared builder trait

```rust
pub trait SipRequestOptions: Sized + Send + Sync {
    // Send + Sync is mandatory: every builder must cross `.await` points
    // and be safe to spawn via `tokio::spawn` for B2BUA / call-center
    // per-leg concurrency. See second-round audit §"Builder Send+Sync
    // requirement" for the rationale.

    /// The SIP method this builder will emit. Used by HeaderPolicy.
    fn method(&self) -> Method;

    /// Append one header. Returns Err for stack-managed or method-shaped
    /// headers; the Err names the dedicated setter when one exists.
    fn with_header(self, header: TypedHeader)
        -> Result<Self, HeaderPolicyViolation>;

    /// Batch form. Fails fast on the first violation, reporting which
    /// header was rejected.
    fn with_headers(self, headers: Vec<TypedHeader>)
        -> Result<Self, HeaderPolicyViolation>;

    /// Convenience: parse `value` and append a `TypedHeader::Other(name, value)`.
    /// Same policy check as `with_header`.
    fn with_raw_header(self, name: impl Into<HeaderName>, value: impl Into<String>)
        -> Result<Self, HeaderPolicyViolation>;

    /// Drop any header with `name` that was added earlier in the builder
    /// chain (or inherited from a carry-through). Infallible; stack-managed
    /// names are silent no-op (they were never reachable via this API).
    fn strip_header(self, name: &HeaderName) -> Self;

    /// B2BUA carry-through: copy the listed headers from `source` verbatim.
    /// Stack-managed headers in `names` are filtered automatically and
    /// reported in `HeaderCarryThroughReport.skipped`. Missing headers are
    /// silently ignored.
    fn with_headers_from<S: SipHeaderView>(
        self,
        source: &S,
        names: &[HeaderName],
    ) -> Result<(Self, HeaderCarryThroughReport), HeaderPolicyViolation>;

    /// Inspect headers staged so far — useful when carry-through and
    /// custom-author logic interleave.
    fn staged_headers(&self) -> &[TypedHeader];

    /// Strict-vs-flexible policy. Defaults to `Config.default_builder_strictness`
    /// (which defaults to Strict). See "Strict vs flexible outbound mode"
    /// under "Layer-audit refinements" for semantics.
    fn with_strictness(self, mode: BuilderStrictness) -> Self;
}

pub struct HeaderPolicyViolation {
    pub method: Method,
    pub header: HeaderName,
    pub reason: ViolationReason,
}

pub enum ViolationReason {
    /// Owned by the dialog or transaction layer (Call-ID, CSeq, Via, …)
    StackManaged,
    /// Wrong method for this header (Event on INVITE, Refer-To on BYE, …)
    WrongMethod,
    /// Header has a dedicated builder setter that must be used instead.
    UseDedicatedSetter(&'static str),
}

pub struct HeaderCarryThroughReport {
    pub copied: Vec<HeaderName>,
    pub skipped: Vec<(HeaderName, ViolationReason)>,
}
```

Every send-side builder implements this trait. The `Result` return on
`with_*` forces call sites to acknowledge the policy decision — `?`
chains cleanly:

```rust
coord.invite(from, to)
    .with_credentials(creds)
    .with_header(typed_pai)?
    .with_raw_header("X-Customer-ID", customer_id)?
    .send().await?;
```

### 3. `HeaderPolicy` — the layer-boundary enforcement

`src/api/headers/policy.rs`. Three categories:

- **Stack-managed** (forbidden via `with_header`): `Call-ID`, `CSeq`,
  `Via`, `Max-Forwards`, `Record-Route`, `Content-Length`. Dialog
  `From`/`To` URIs and tags are stack-managed once the dialog is
  established; the builder method's `from`/`to` constructor args
  legitimately control them.
- **Method-shaped** (set via dedicated builder setter): `Refer-To`
  (constructor arg on `ReferBuilder`), `Event` / `Subscription-State`
  (`NotifyBuilder` setters), `Authorization` / `Proxy-Authorization`
  (`with_credentials` or `with_precomputed_authorization`), `Contact`
  for REGISTER (`with_contact_uri`), `Expires` for REGISTER
  (`with_expires`).
- **Application-controlled** (free to add): `Diversion`, `History-Info`,
  `Referred-By` (also has a setter), `Replaces`, `P-Asserted-Identity`,
  `P-Preferred-Identity`, `Privacy`, `Reason`, `Retry-After`, `Warning`,
  `Subject`, `Date`, `User-Agent`, `Server`, `Accept`, `Allow`,
  `Supported`, `Require`, `Path`, `Service-Route`, `Reply-To`, any
  `Other(...)` / `X-*`.

The classification is **method-aware**: `Contact` is stack-managed for
mid-dialog requests but application-controlled for REGISTER and 3xx
responses. `Authorization` is method-shaped for builders that have
`with_credentials` but application-controlled when injecting a
precomputed value via `with_precomputed_authorization`.

```rust
pub enum HeaderRole {
    StackManaged,
    MethodShaped { setter: &'static str },
    ApplicationControlled,
}

pub fn classify(method: Method, name: &HeaderName) -> HeaderRole;

/// Whether `name` should be silently filtered when carrying through
/// from an inbound message. Stack-managed names are always filtered;
/// the trace logs `tracing::warn!` listing skipped names.
pub fn forbidden_for_carry_through(name: &HeaderName) -> bool;

/// Method-specific check that all required application-supplied headers
/// are present. Run by every builder's `.send()` before dispatch.
pub fn validate_outbound(method: Method, headers: &[TypedHeader])
    -> Result<(), Vec<MissingRequiredHeader>>;
```

A const lookup table per method covers the ~25 RFC headers that are ever
stack-managed. `HeaderName::Other(_)` is always application-controlled.

### 4. New inbound types

- `IncomingRequest { call_id, method, raw, … }` — handed to
  `CallHandler` hooks that today receive only a pre-decoded target
  string (`on_transfer_request(handle, target: String)` becomes
  `on_transfer_request_full(handle, req: IncomingRequest)` with the
  original REFER still accessible).
- `IncomingResponse { status_code, reason, raw }` — emitted on
  `CallProgress`, `CallFailed`, etc. when the application wants to
  read `Retry-After`, `Warning`, or carrier disconnect codes.
- `IncomingRegister { raw, … }` — surfaced to registrar applications.

All implement `SipHeaderView`.

### 5. Send-side builders

All entered from `UnifiedCoordinator` and proxied through each surface
(`Endpoint`, `StreamPeer`, `CallbackPeer`). All implement
`SipRequestOptions`. Terminal method is `.send().await` returning the
type that matches the existing flat method's return.

| Builder | Coordinator entry | `.send()` returns | Method-specific setters |
|---|---|---|---|
| `OutboundCallBuilder` | `coord.invite(from, to)` | `Result<SessionId>` | `with_credentials`, `with_pai` / `without_pai`, `as_transfer_leg(&SessionId)`, `with_subject`, `with_from_display`, `with_contact_uri` (B2BUA-critical, see second-round audit), `with_outbound_proxy(uri)` / `without_outbound_proxy`, `with_precomputed_authorization`, `with_sdp` |
| `OutboundCallBuilder` (re-INVITE) | `coord.reinvite(&session)` | `Result<()>` | `with_sdp`, `as_session_timer_refresh`, `with_precomputed_authorization` |
| `RegisterBuilder` | `coord.register(registrar, user, pw)` | `Result<RegistrationHandle>` | `with_expires`, `with_from_uri`, `with_contact_uri`, `with_outbound_proxy(uri)` / `without_outbound_proxy`, `with_path(uri)` (RFC 3327), `with_q_value(f32)`, `with_sip_instance(urn)`, `with_reg_id(u32)`, `with_precomputed_authorization` |
| `ReferBuilder` | `coord.refer(&session, refer_to)` | `Result<()>` | `with_replaces(value)`, `with_referred_by(uri)` (RFC 3892), `with_target_dialog(&IncomingRequest)` |
| `ByeBuilder` | `coord.bye(&session)` | `Result<()>` | `with_reason(SipReason)` |
| `CancelBuilder` | `coord.cancel(&session)` | `Result<()>` | `with_reason(SipReason)` |
| `NotifyBuilder` | `coord.notify(&session, event_package)` | `Result<()>` | `with_body(impl Into<Vec<u8>>)`, `with_content_type(s)`, `with_subscription_state(s)`, `with_retry_after(u32)` |
| `SubscribeBuilder` | `coord.subscribe(target, event_package)` | `Result<SubscriptionHandle>` | `with_from_uri`, `with_contact_uri`, `with_expires`, `with_accept(content_type)`, `with_credentials` |
| `InfoBuilder` | `coord.info(&session, content_type)` | `Result<()>` | `with_body(impl Into<Vec<u8>>)` |
| `UpdateBuilder` | `coord.update(&session)` | `Result<()>` | `with_sdp(s)`, `as_session_timer_refresh()` |
| `MessageBuilder` | `coord.message(target)` | `Result<()>` | `with_body`, `with_content_type`, `with_credentials`, `with_from_uri` |
| `OptionsBuilder` | `coord.options(target)` | `Result<IncomingResponse>` | `with_from_uri`, `with_accept` |

**Not exposed** (state machine emits automatically; gateway authors
don't need to author these):

- `ACK` for 2xx — RFC 3261 §13.2.2.4
- `PRACK` — RFC 3262

Each builder is a value-type that consumes `self` per setter, so chaining
compiles to one struct literal after monomorphization. Per-surface
adapters (`Endpoint`, `StreamPeer`, `CallbackPeer`) expose the same
builders but pre-populate `from`/`contact` from the surface's local URI,
and the `Endpoint` adapter still runs `resolve_target` on bare extensions.

### 6. Send-side response builders (B2BUA-critical)

Servers and B2BUAs need authorship on the **response** side too. All
implement `SipRequestOptions`. Dialog-core's
`UnifiedDialogApi::send_response(transaction_id, Response)` at
`crates/rvoip-sip-dialog/src/api/unified.rs:784-790` already accepts a
fully-authored `Response`, so response builders compose the `Response`
in `rvoip-sip` via `SimpleResponseBuilder` from sip-core and hand it
over. **Today's `DialogAdapter::send_response` at
`dialog_adapter.rs:1064-1089` is a thin pass-through** that builds a
basic Response from `(status_code, sdp)` and forwards; Phase D adds
`send_response_with_options(session_id, Response)` as a sibling that
takes a pre-built Response and resolves the session's pending
transaction key.

| Builder | Entry | `.send()` returns | Method-specific setters |
|---|---|---|---|
| `AcceptBuilder` | `incoming.accept_builder()` or `coord.accept(&session)` | `Result<SessionHandle>` | `with_sdp(s)` |
| `RejectBuilder` | `incoming.reject_builder()` or `coord.reject(&session)` | `Result<()>` | `with_status(u16)`, `with_reason(s)`, `with_retry_after(u32)`, `with_warning(code, agent, text)` |
| `RedirectBuilder` | `incoming.redirect_builder()` or `coord.redirect(&session)` | `Result<()>` | `with_status(u16)` (default 302), `with_contact(uri)` (chainable), `with_contacts(Vec)` |
| `ProvisionalBuilder` | `incoming.send_provisional_builder(code)` | `Result<()>` | `with_sdp` (for 183 early media), `with_require_100rel(bool)` |
| `AuthChallengeBuilder` | `incoming.challenge_builder(scheme)` | `Result<()>` | `with_realm`, `with_nonce`, `with_algorithm`, `with_qop`, `with_stale`, `with_opaque`, `as_proxy_challenge(bool)` (toggles 401/`WWW-Authenticate` vs 407/`Proxy-Authenticate`) |

`AuthChallengeBuilder` wraps `SimpleResponseBuilder::www_authenticate_digest`
/ `www_authenticate_bearer` from sip-core for typed challenge authoring —
needed by registrars and B2BUA auth-relay code.

### 7. Surface symmetry — every builder reachable from every surface

| Surface | Entry shape | Pre-fills | `.send()` returns |
|---|---|---|---|
| `UnifiedCoordinator` | `coord.invite(from, to)` etc. | nothing | `Result<SessionId>` |
| `Endpoint` | `endpoint.invite(to)` etc. | `from = endpoint.local_uri`, `resolve_target` on bare extensions | `Result<SessionHandle>` |
| `StreamPeer` | `peer.invite(to)` | `from = peer.local_uri` | `Result<SessionHandle>` |
| `CallbackPeer` | `peer.invite(to)` | `from = peer.local_uri` | `Result<SessionHandle>` |

In-dialog builders (REFER, NOTIFY, INFO, BYE, CANCEL, UPDATE) on all
surfaces take a `&SessionHandle` (or `&SessionId` on coordinator) and
need no `from`/`to` because dialog state provides them.

### 8. B2BUA composition example (target ergonomics)

The litmus test the API has to pass — a gateway forwarding an inbound
INVITE with carried-through diagnostic headers, a rewritten PAI, a
stripped Privacy header, and full audit:

```rust
let incoming = peer.wait_for_incoming().await?;

// Inspect inbound
let original_pai = incoming.header(&HeaderName::PAssertedIdentity);
let history = incoming.headers_named(&HeaderName::HistoryInfo);

// Build outbound leg — every with_* returns Result; ? chains cleanly
let (outbound, report) = coord
    .invite(local_from, upstream_target)
    .with_credentials(carrier_creds)
    .with_headers_from(&incoming, &[
        HeaderName::HistoryInfo,
        HeaderName::Diversion,
        HeaderName::Other("X-Customer-ID".into()),
    ])?;
let outbound = outbound
    .strip_header(&HeaderName::Privacy)
    .with_raw_header("P-Asserted-Identity", rewritten_pai_value)?;

tracing::info!(skipped = ?report.skipped, "carry-through audit");

let session = outbound.send().await?;

// Bridge media
coord.bridge(&incoming.call_id, &session).await?;
```

Same pattern, in reverse, for the REFER-rewriting flow:

```rust
// CallHandler::on_transfer_request_full now receives the original REFER
async fn on_transfer_request_full(
    &self,
    _handle: SessionHandle,
    refer: IncomingRequest,
) -> bool {
    let original_refer_to = refer.header(&HeaderName::ReferTo).unwrap();
    let rewritten = rewrite_for_downstream(original_refer_to);

    let (b, _report) = coord.refer(&other_leg, &rewritten)
        .with_headers_from(&refer, &[HeaderName::ReferredBy])
        .unwrap_or_else(|e| panic!("policy violation: {:?}", e));
    b.send().await.map(|_| true).unwrap_or(false)
}
```

---

## Dialog-core extensions (additive, layer-correct)

The single non-`rvoip-sip` change set. New options structs and
`*_with_options` methods on `UnifiedDialogApi`
(`crates/rvoip-sip-dialog/src/api/unified.rs`):

Every options struct derives `Default` so callers can use
`..Default::default()` for fields they don't care about.

```rust
#[derive(Default)]
pub struct ReferRequestOptions {
    pub refer_to: String,
    pub replaces: Option<String>,
    pub referred_by: Option<String>,
    pub target_dialog: Option<String>,           // RFC 4538
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct NotifyRequestOptions {
    pub event: String,
    pub subscription_state: String,
    pub content_type: Option<String>,
    pub body: Option<Bytes>,
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct InfoRequestOptions {
    pub content_type: String,
    pub body: Bytes,
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct ByeRequestOptions {
    pub reason: Option<String>,                  // RFC 3326
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct CancelRequestOptions {
    pub reason: Option<String>,                  // RFC 3326
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct UpdateRequestOptions {
    pub sdp: Option<String>,
    pub session_timer_refresh: bool,
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct ReInviteRequestOptions {
    pub sdp: Option<String>,
    pub session_timer_refresh: bool,
    pub precomputed_authorization: Option<String>,
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct SubscribeRequestOptions {
    pub event: String,
    pub expires: u32,
    pub accept: Option<String>,
    pub from_uri: Option<String>,
    pub contact_uri: Option<String>,
    pub credentials: Option<Credentials>,
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct MessageRequestOptions {
    pub from_uri: String,
    pub to_uri: String,
    pub content_type: String,
    pub body: Bytes,
    pub credentials: Option<Credentials>,
    pub extra_headers: Vec<TypedHeader>,
}
#[derive(Default)]
pub struct OptionsRequestOptions {
    pub from_uri: String,
    pub to_uri: String,
    pub accept: Option<String>,
    pub extra_headers: Vec<TypedHeader>,
}

// Existing RegisterRequestOptions (`api/unified.rs:228-239`) currently
// derives only `Debug, Clone` and lacks both `extra_headers` and a
// `Default` impl. Phase B adds:
//   - `pub extra_headers: Vec<TypedHeader>`
//   - `#[derive(Default)]` (every options struct in this design derives Default)

impl UnifiedDialogApi {
    pub async fn send_refer_with_options(&self, &DialogId, ReferRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_notify_with_options(&self, &DialogId, NotifyRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_info_with_options(&self, &DialogId, InfoRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_bye_with_options(&self, &DialogId, ByeRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_cancel_with_options(&self, &DialogId, CancelRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_update_with_options(&self, &DialogId, UpdateRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_reinvite_with_options(&self, &DialogId, ReInviteRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_subscribe_with_options(&self, &str, SubscribeRequestOptions)
        -> ApiResult<SubscriptionHandle>;
    pub async fn send_subscribe_refresh_with_options(&self, &DialogId, SubscribeRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_message_with_options(&self, MessageRequestOptions)
        -> ApiResult<TransactionKey>;

    // No `send_options*` method exists in dialog-core today. Phase B
    // **authors** `send_options_out_of_dialog_with_options` (new path
    // through `transaction/utils/request_builders.rs`) — there is no
    // pre-existing implementation to delegate to, unlike the other
    // methods listed here.
    pub async fn send_options_out_of_dialog_with_options(&self, OptionsRequestOptions)
        -> ApiResult<TransactionKey>;
}
```

> The earlier draft listed only 6 new options structs. Verification of
> `rvoip-sip-dialog/src/api/unified.rs` confirmed:
> - `send_reinvite` (line 1485) currently routes through generic
>   `send_request()` and does **not** accept extra headers — re-INVITE
>   needs its own options struct.
> - `send_cancel` (line 1531) and `send_message_out_of_dialog` (line 1408)
>   need options forms to keep parity with the rvoip-sip builder surface.
> - `RegisterRequestOptions` already exists at unified.rs:228-239 — Phase
>   B adds the `extra_headers` field *and* `#[derive(Default)]` (today
>   the struct derives only `Debug, Clone`).
> - **OPTIONS has no existing dialog-core method at all** — neither
>   `send_options` nor `send_options_out_of_dialog` exists. Phase B
>   authors a new dialog-core entry point on top of the transaction
>   layer, layer-parallel to `send_message_out_of_dialog`. This is
>   strictly authorship (new code), not "promote an existing internal
>   method".
>
> MESSAGE and OPTIONS are out-of-dialog. They are authored inside
> dialog-core (not in rvoip-sip) because the transaction-layer plumbing
> already lives there; this keeps the layer boundary clean. (An earlier
> draft asserted they "bypass dialog-core" — that was incorrect.)

Implementation: each delegates to the existing internal request builder
path (`transaction/utils/request_builders.rs` for non-dialog-bound,
`transaction/dialog/request_builder_from_dialog_template` for in-dialog).
That template already accepts an `extra_headers: Option<Vec<TypedHeader>>`
parameter (`transaction/dialog/mod.rs:107-118`) and appends them **after**
CSeq, Route-Set, Contact, From, To, Call-ID, Via, Max-Forwards are
stamped. Bodies and method-specific headers are taken from the options
struct, not from extra headers — the `rvoip-sip` builder layer routes
them correctly via the `HeaderPolicy::MethodShaped` rule before calling
dialog-core.

Existing dialog-core methods (`send_refer`, `send_notify`, etc.) stay,
delegate to the `*_with_options` form with an empty `extra_headers` and
defaults, and are **not deprecated** — other crates depend on them.

Dialog state machine, route-set logic, transaction core, CSeq management
are untouched. The dialog state (`DialogImpl` at
`src/dialog/dialog_impl.rs`), CSeq counter (`local_cseq` line 47,
incremented by `increment_local_cseq()` line 706), Route-Set
(`route_set: Vec<Uri>` line 56), and Contact headers remain authoritative
in dialog-core; only application-controlled headers ride alongside.

---

## Implementation phases

Five phases, each shippable as a separate PR in this order.

### Phase A — Inbound inspection (rvoip-sip + minimal infra-common)

`src/api/headers/view.rs` (new) defines `SipHeaderView`. `IncomingCall`
gains a `request: Arc<rvoip_sip_core::Request>` field populated by the
state machine at session creation. `header()`, `headers_named()`,
`headers()`, `header_str()`, `header_names()`, `raw_request()`
implementations. The empty-`HashMap` bug at `src/api/incoming.rs:80` is
fixed (populated for back-compat, deprecation note pointing readers at
the trait).

**Cross-crate event enrichment (Phase A scope, not deferred to E):**
the existing `DialogToSessionEvent::IncomingCall` variant
(`crates/infra-common/src/events/cross_crate.rs:516`) carries
pre-decoded fields today and does not retain the parsed Request.
Phase A adds `raw_request: Arc<Bytes>` to that variant (additive;
existing consumers unaffected) and dialog-core's publish site
re-serializes the parsed INVITE via `Request::to_bytes()` into the
event. rvoip-sip's state-machine handler re-parses on receipt to
populate `IncomingCall.request`. Same plumbing pattern as Phase E.

`IncomingResponse`, `IncomingRequest`, `IncomingRegister` new types,
each implementing `SipHeaderView`. `Event::CallProgressDetailed
(IncomingResponse)` etc. — additive event variants; existing variants
stay so callers don't churn.

### Phase B — Dialog-core options extension (rvoip-sip-dialog)

Per the section above: 6 new options structs, 6 new `*_with_options`
methods on `UnifiedDialogApi`, `extra_headers` field added to existing
`RegisterRequestOptions`. Existing methods stay and delegate. Internal
implementation appends `extra_headers` to the in-dialog `Request`
constructed by `transaction/utils/request_builders.rs` before
transaction dispatch.

### Phase C — Send-side builders (rvoip-sip, the bulk of the work)

Per-method modules under `src/api/send/`:

- `outbound_call.rs` → `OutboundCallBuilder` (INVITE + re-INVITE)
- `register.rs` → `RegisterBuilder` (existing `Registration` struct
  kept as backwards-compat alias / `From` impl)
- `refer.rs` → `ReferBuilder`
- `bye.rs` → `ByeBuilder`, `CancelBuilder`
- `notify.rs` → `NotifyBuilder`
- `subscribe.rs` → `SubscribeBuilder`
- `info.rs` → `InfoBuilder`
- `update.rs` → `UpdateBuilder` (new)
- `message.rs` → `MessageBuilder` (new)
- `options.rs` → `OptionsBuilder` (new)

Shared infrastructure:

- `src/api/headers/options.rs` — `SipRequestOptions` trait,
  `BuilderHeaderState`, `HeaderPolicyViolation`,
  `HeaderCarryThroughReport`, `ViolationReason`
- `src/api/headers/policy.rs` — `HeaderPolicy::classify`,
  `forbidden_for_carry_through`, `validate_outbound`, the const lookup
  tables

`SipRequestOptions` has default implementations of `with_headers`,
`with_raw_header`, `strip_header`, and `with_headers_from` that operate
on `BuilderHeaderState`, so each builder only declares it owns one.

Author / refactor internal helpers in `state_machine/helpers.rs`. Per
the 2026-05-11 audit, only the INVITE family has helpers today (5
sibling methods at lines 99-156). Phase C collapses those into a single
`make_call_inner(opts)` and **authors brand-new** siblings for the
other methods:

```rust
// Collapse of 5 existing methods (lines 99-156):
pub(crate) async fn make_call_inner(opts);

// NEW (no current analog in helpers.rs):
pub(crate) async fn send_refer_inner(opts);
pub(crate) async fn send_notify_inner(opts);
pub(crate) async fn send_info_inner(opts);
pub(crate) async fn send_bye_inner(opts);                // replaces hangup_with_reason
pub(crate) async fn send_cancel_inner(opts);
pub(crate) async fn send_update_inner(opts);
pub(crate) async fn send_subscribe_inner(opts);
pub(crate) async fn send_message_inner(opts);
pub(crate) async fn send_options_inner(opts);
pub(crate) async fn send_register_inner(opts);
pub(crate) async fn send_reinvite_inner(opts);
```

**Routing matrix (verified 2026-05-11, supersedes earlier wording).**
Today's `UnifiedCoordinator` public-API entry points split as follows:

| Method | UAC public-API path today | Phase C plan |
|---|---|---|
| INVITE | `EventType::MakeCall` → state machine → `Action::SendINVITE` (carries headers via `SessionState.extra_headers` stash) → `DialogAdapter::send_invite_with_extra_headers` | Same path. Stash widens from `Vec<TypedHeader>` to `Arc<OutboundCallOptions>` (or stash both side-by-side for back-compat). |
| re-INVITE | `EventType::ReInvite*` → state machine → `Action::SendReINVITE` (parameterless today, `state_table/types.rs:489`) → `DialogAdapter::send_reinvite_session` | Same path. `Action::SendReINVITE` widens to carry `Arc<ReInviteRequestOptions>` — clean refactor since it has no existing payload. |
| REGISTER | `EventType::Register*` → state machine → `Action::SendREGISTER` / `SendREGISTERWithAuth` → `DialogAdapter::send_register*` | Same path. Action payload (or stash) widens to options. |
| SUBSCRIBE | `EventType::Subscribe*` → state machine → `Action::SendSUBSCRIBE` → `DialogAdapter::send_subscribe` | Same path. |
| NOTIFY | **Two paths today.** UAC public-API (`UnifiedCoordinator::send_notify` at unified.rs:2472) bypasses the state machine and calls `dialog_adapter.send_notify` directly. The `Action::SendNOTIFY` variant (actions.rs:1446) is used only for state-machine-driven emissions (e.g., REFER-implicit NOTIFY, subscription-state changes). | Builder's `.send()` keeps the direct path (no state-machine round-trip). Action payload widens too so state-machine-emitted NOTIFYs also carry options. |
| MESSAGE | `EventType::Message*` → state machine → `Action::SendMESSAGE` → `DialogAdapter::send_message` | Same path. |
| BYE | `EventType::HangupCall` → state machine → `Action::SendBYE` (reads `SessionState.pending_bye_reason` stash) → `DialogAdapter::send_bye_session_with_reason` | Same path. Stash widens (`pending_bye_options: Arc<ByeRequestOptions>`). Required because BYE drives session lifecycle — bypassing the state machine would skip CallEnded events / cleanup. |
| CANCEL | State-machine-driven (`Action::SendCANCEL`); no public-API path | Builder routes through state machine to preserve CANCEL semantics (RFC 3261 §9). Stash + Action payload widen together. |
| REFER | Direct: `UnifiedCoordinator::send_refer` at unified.rs:2260 stashes `transfer_target`/`replaces_header`/`transfer_state` and calls `dialog_adapter.send_refer_session` directly. **No `Action::SendREFER` variant exists.** | Builder's `.send()` keeps the direct path. No new Action variant required; the existing transfer-state stash plus the new options struct ride together as function arguments to `DialogAdapter::send_refer_with_options`. |
| INFO | Direct: `UnifiedCoordinator::send_info` at unified.rs:2437 calls `dialog_adapter.send_info` directly. No state-machine path. | Builder's `.send()` keeps the direct path. |
| UPDATE | No public-API path today. | Builder's `.send()` calls `DialogAdapter::send_update_with_options` directly; no Action variant. |
| OPTIONS | Does not exist anywhere today (no UnifiedCoordinator method, no Action variant, no dialog-core method). | Builder's `.send()` calls `DialogAdapter::send_options_with_options` which calls the new `UnifiedDialogApi::send_options_out_of_dialog_with_options`. New authorship through and through. |

Phase C therefore touches three categories of code:

1. **Action-variant widening** (INVITE, re-INVITE, REGISTER, SUBSCRIBE, MESSAGE, NOTIFY, BYE, CANCEL): the `Action::Send*` variant payload widens to carry `Arc<XxxRequestOptions>`, or the corresponding `SessionState` stash field widens. No new variants. The state machine's transition table (`src/state_table/yaml_loader.rs` + `src/state_table/types.rs`) is unchanged.

2. **Direct-dispatch builders** (REFER, INFO, UPDATE, OPTIONS): builder's `.send()` calls `DialogAdapter::send_*_with_options` directly without touching the state machine. No SessionState stash widening required for these.

3. **Inner helper authorship** (`state_machine/helpers.rs`):

```rust
// Collapse of 5 existing methods (lines 99-156):
pub(crate) async fn make_call_inner(opts);

// NEW (no current analog in helpers.rs):
pub(crate) async fn send_register_inner(opts);
pub(crate) async fn send_subscribe_inner(opts);
pub(crate) async fn send_message_inner(opts);
pub(crate) async fn send_notify_inner(opts);     // for the state-machine-emit path
pub(crate) async fn send_bye_inner(opts);        // replaces hangup_with_reason internals
pub(crate) async fn send_cancel_inner(opts);
pub(crate) async fn send_reinvite_inner(opts);

// Direct-dispatch (no state machine — these never enter helpers.rs):
//   send_refer_inner, send_info_inner, send_update_inner, send_options_inner
//   live as inherent methods on the builder, calling DialogAdapter directly.
```

The state machine is unchanged structurally — payload widening is the entire change. The `SessionState` stash fields gain `pending_<method>_options: Option<Arc<XxxRequestOptions>>` siblings to the existing `pending_bye_reason` / `transfer_target` fields (`session_store/state.rs:43-209`).

`UnifiedCoordinator` gains the 12 `.invite() / .reinvite() / .register()
/ .refer() / .bye() / .cancel() / .notify() / .subscribe() / .info() /
.update() / .message() / .options()` entry points. Each is a 1–2 line
stub that constructs the builder. Surface adapters (`Endpoint`,
`StreamPeer`, `CallbackPeer`) add the same entry points returning
`PeerXBuilder` wrappers that translate the terminal `SessionId` →
`SessionHandle` etc.

`DialogAdapter` (`src/adapters/dialog_adapter.rs`) gains 6 mirror
methods (`send_refer_with_options`, `send_notify_with_options`,
`send_info_with_options`, `send_bye_with_options`,
`send_update_with_options`, `send_subscribe_with_options`). Each
translates `SessionId → DialogId`, prepends outbound-proxy Route if
configured (reuse `prepend_outbound_proxy_route` at `adapter:2087`),
runs `HeaderPolicy::validate_outbound`, and forwards to dialog-core.

### Phase D — Response builders (rvoip-sip)

`src/api/respond/` mirrors the send tree:

- `accept.rs` → `AcceptBuilder`
- `reject.rs` → `RejectBuilder`
- `redirect.rs` → `RedirectBuilder`
- `provisional.rs` → `ProvisionalBuilder`
- `challenge.rs` → `AuthChallengeBuilder` (new)

Each builder composes a `Response` via `SimpleResponseBuilder` from
sip-core, runs `HeaderPolicy::validate_outbound(Method::Invite_or_what,
&response_headers)` (response-side check is method-aware too), and
hands the `Response` to `UnifiedDialogApi::send_response`.

`IncomingCall::accept_builder()`, `reject_builder()`,
`redirect_builder()`, `send_provisional_builder(code)`,
`challenge_builder(scheme)` entries. `UnifiedCoordinator` mirrors with
explicit-session variants.

The existing `IncomingCall::{accept, reject, reject_busy,
reject_decline, redirect_to, redirect_with_contacts}` methods stay and
become one-line wrappers over the builders.

### Phase E — In-dialog request surface (rvoip-sip + dialog-core enrichment)

`IncomingRequest` new type. `Event::ReferReceived` /
`Event::NotifyReceived` / `Event::InfoReceived` gain
`request: IncomingRequest` fields (additive; existing pre-decoded fields
stay so callers don't break). Verification shows there is **no
`Event::InfoReceived` today** — Phase E introduces it alongside the
existing variants.

**Cross-crate event enrichment** — the inbound surface is the one place
the design has to thread inbound bytes back through the
`GlobalEventCoordinator` bus. The current
`DialogToSessionEvent::{TransferRequested, NotifyReceived, IncomingRegister}`
variants (defined in `crates/infra-common/src/events/cross_crate.rs:514+`,
not in dialog-core) carry pre-decoded fields. Phase E adds one
additive field per variant **and** four new variants:

```rust
// in infra-common::events::cross_crate.rs (NOT dialog-core)
// — see "Cross-layer mechanics" §3 above for why the payload is bytes
//   rather than Arc<Request>.

TransferRequested { ..existing.., raw_request: Arc<Bytes> }   // REFER
NotifyReceived    { ..existing.., raw_request: Arc<Bytes> }
IncomingRegister  { ..existing.., raw_request: Arc<Bytes> }

// New variants — no analogs exist today:
InfoReceived      { session_id: String, raw_request: Arc<Bytes> }
MessageReceived   { session_id: String, raw_request: Arc<Bytes> }
OptionsReceived   { session_id: String, raw_request: Arc<Bytes> }
UpdateReceived    { session_id: String, raw_request: Arc<Bytes> }
// — alternatively, enrich existing ReinviteReceived with raw_request and
//   widen its semantic to cover UPDATE; both UPDATE and re-INVITE land
//   on the same handler today.
```

`Arc<Bytes>` keeps the bus cheap-clone under fan-out (Bytes is itself
an Arc internally). Existing subscribers that don't read `raw_request`
are unaffected (additive field). The rvoip-sip side re-parses with
`rvoip_sip_core::parse_message` to produce the typed `Request` that
`IncomingRequest` / `IncomingResponse` / `IncomingRegister` wrap.

`CallHandler` trait methods that take pre-decoded data
(actual names today: `on_refer_received`, `on_notify_received` at
`src/api/callback_peer.rs:814+`) gain `_full` companion methods that
take `IncomingRequest`; the original versions become
default-implemented adapters that strip the request down to today's
shape. New methods (no string-shaped predecessor in today's trait):

- `on_info_received_full(handle, request)` — INFO is not surfaced today
  as a typed callback (it only reaches the app through the generic
  event channel).
- `on_message_received_full(handle, request)` — same.
- `on_options_received_full(handle, request)` — OPTIONS is dropped on
  the floor today (no inbound routing); Phase E plumbs it.
- `on_update_received_full(handle, request)` — UPDATE today shares the
  re-INVITE channel; the `_full` variant lets the application
  distinguish.

Each has a default no-op implementation so existing `CallHandler`
implementors compile unchanged.

```rust
// Naming corrected from earlier drafts: the existing trait methods are
// `on_transfer_request` (callback_peer.rs:894), `on_refer_notify` (904),
// and `on_notify` (980) — NOT `on_refer_received` / `on_notify_received`.
// The `_full` companions take `IncomingRequest` and default-decode to
// the legacy positional arguments so existing implementers compile
// unchanged.

async fn on_transfer_request_full(&self, h: SessionHandle, r: IncomingRequest) -> bool {
    let target = r.header_str(&HeaderName::ReferTo).unwrap_or_default();
    self.on_transfer_request(h, target).await
}
async fn on_refer_notify_full(&self, h: SessionHandle, r: IncomingRequest) {
    // default impl forwards pre-decoded fields to on_refer_notify(...)
}
async fn on_notify_full(&self, h: SessionHandle, r: IncomingRequest) {
    // default impl forwards pre-decoded fields to on_notify(...)
}
async fn on_info_full(&self, h: SessionHandle, r: IncomingRequest) {}
async fn on_message_full(&self, h: SessionHandle, r: IncomingRequest) {}
async fn on_options_full(&self, h: SessionHandle, r: IncomingRequest) {}
async fn on_update_full(&self, h: SessionHandle, r: IncomingRequest) {}
```

Existing implementations keep compiling; B2BUA implementations override
the `_full` variant to inspect / forward headers.

> Verified callback names (audit) — required: `on_incoming_call`;
> optional: `on_event`, `on_call_established`, `on_call_progress`,
> `on_call_ended`, `on_call_failed`, `on_call_cancelled`, `on_dtmf`,
> `on_media_security_negotiated`, `on_call_on_hold`, `on_call_resumed`,
> `on_remote_call_on_hold`, `on_remote_call_resumed`,
> `on_refer_received`, `on_notify_received`.
> The string `on_transfer_request` appeared in earlier drafts and does
> not exist in the trait — corrected to `on_refer_received` throughout.

### Deprecation

Every method below gets `#[deprecated(since = "0.3.0", note = "use
coord.<verb>(...).send().await — see SIP_API_DESIGN.md")]` with a
one-line body that forwards to the builder:

`UnifiedCoordinator::{make_call, make_call_with_auth, make_call_with_pai,
make_call_with_headers, register, register_with, send_refer,
send_refer_with_replaces, send_notify, send_info, hangup_with_reason,
reject_call, redirect_call, subscribe_dialogs}`,
`PeerControl::{call, call_with_auth, call_with_headers}`,
`StreamPeer::{call, call_with_headers}`,
`CallbackPeerControl::{call, call_with_auth, call_with_headers}`,
`EndpointControl::{call, call_with_headers}`,
`Endpoint::{call, call_with_headers}`.

`make_transfer_leg` keeps its name and signature (specialised, called
by the b2bua wrapper crate); its body becomes
`self.invite(from, to).as_transfer_leg(transferor).send().await`.

The workspace already has `deprecated = "allow"` at the lint level
(`Cargo.toml:54-67`), so internal examples and tests don't break.
Migrating their call sites is a follow-up.

---

## Critical files

**New files**

- `crates/rvoip-sip/SIP_API_DESIGN.md` — this document
- `crates/rvoip-sip/src/api/headers/mod.rs`, `view.rs`, `options.rs`,
  `policy.rs`
- `crates/rvoip-sip/src/api/send/mod.rs` + 10 builder modules
  (`outbound_call`, `register`, `refer`, `bye`, `notify`, `subscribe`,
  `info`, `update`, `message`, `options`)
- `crates/rvoip-sip/src/api/respond/mod.rs` + 5 builder modules
  (`accept`, `reject`, `redirect`, `provisional`, `challenge`)
- `crates/rvoip-sip/tests/header_policy_unit.rs`
- `crates/rvoip-sip/tests/header_inspection_integration.rs`
- `crates/rvoip-sip/tests/outbound_request_builders_integration.rs`
- `crates/rvoip-sip/tests/response_builders_integration.rs`
- `crates/rvoip-sip/tests/b2bua_carry_through_integration.rs`
- `crates/rvoip-sip/tests/forbidden_header_guard_integration.rs`
- `crates/rvoip-sip/tests/builder_strictness_integration.rs`
- `crates/rvoip-sip/tests/config_builder_coexistence.rs`

**Modified — `infra-common` (Phase E only, additive)**

- `src/events/cross_crate.rs` — enrich `DialogToSessionEvent::{TransferRequested,
  NotifyReceived, IncomingRegister}` with `raw_request: Arc<bytes::Bytes>`;
  add new variants `InfoReceived`, `MessageReceived`, `OptionsReceived`,
  and either enrich `ReinviteReceived` with `raw_request` (preferred) or
  add a dedicated `UpdateReceived`. `bytes` is already a workspace dep
  on `infra-common`; **no new `rvoip-sip-core` dependency is introduced**.

**Modified — `rvoip-sip-dialog` (additive only)**

- `src/api/unified.rs` — add `ReferRequestOptions`,
  `NotifyRequestOptions`, `InfoRequestOptions`, `ByeRequestOptions`,
  `CancelRequestOptions`, `UpdateRequestOptions`, `ReInviteRequestOptions`,
  `SubscribeRequestOptions`, `MessageRequestOptions`,
  `OptionsRequestOptions` (NEW — no existing send_options path);
  add `extra_headers` field AND `#[derive(Default)]` to the existing
  `RegisterRequestOptions` (lines 228-239); add 11 `*_with_options`
  methods (the OPTIONS one is brand-new authorship on top of
  `transaction/utils/request_builders.rs`)
- `src/manager/dialog_operations.rs` (or sibling) — internal

**Modified — `rvoip-sip`**

- `src/api/mod.rs` — declare new `headers/`, `send/`, `respond/` modules,
  re-export traits and builders
- `src/api/incoming.rs` — `request: Arc<Request>` field, `SipHeaderView`
  impl, `accept_builder` / `reject_builder` / `redirect_builder` /
  `send_provisional_builder` / `challenge_builder` entries; fix the
  empty-`HashMap` bug
- `src/api/unified.rs` — 12 new entry points (`invite`, `reinvite`,
  `register`, `refer`, `bye`, `cancel`, `notify`, `subscribe`, `info`,
  `update`, `message`, `options`); mark ~14 existing methods
  `#[deprecated]`
- `src/api/stream_peer.rs` — peer-surface builder entries; mark 3
  methods `#[deprecated]`
- `src/api/callback_peer.rs` — same; mark 3 methods; add new
  `CallHandler::*_full` methods with default impls
- `src/api/endpoint.rs` — endpoint-surface builder entries; mark 4
  methods `#[deprecated]`
- `src/api/events.rs` — add `IncomingResponse`, `IncomingRequest`,
  `IncomingRegister`; enrich `Event::{ReferReceived, NotifyReceived,
  InfoReceived}` with `request: IncomingRequest` (additive)
- `src/adapter.rs` — thin re-export shim; update its `pub use` list to
  cover the new builders / traits / convenience constructors. This file
  is 14 KB; the heavy lifting is in `src/adapters/dialog_adapter.rs`.
- `src/adapters/dialog_adapter.rs` — 11 new `send_*_with_options` mirror
  methods (one per new dialog-core entry point; OPTIONS is the new
  authored path)
- `src/state_machine/helpers.rs` — promote internal helpers to
  `pub(crate)`, refactor to options-shape; populate
  `IncomingCall.request`
- `src/state_machine/actions.rs` — emit `IncomingCall` with parsed
  `Request` attached; thread options through outbound actions
- `src/lib.rs` — re-export every builder + `SipHeaderView`,
  `SipRequestOptions`, `HeaderPolicy`; add a "Gateway / B2BUA / SBC
  Authoring" section to the crate `//!` block

**Files that stay unchanged**

- `rvoip-sip-core`, `rvoip-sip-transport`, `rvoip-sip-dialog` dialog
  state machine / route-set / transaction core / CSeq logic,
  `dialog-core` (the older path being phased out), `media-core`,
  `rtp-core`, `rvoip-core`
- Existing examples and tests under `examples/` and `tests/` — they
  emit deprecation warnings but compile and pass

## Reused utilities

- `rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder}`
  — for response authoring inside `rvoip-sip`'s response builders,
  before handing the `Response` to `UnifiedDialogApi::send_response`
- `rvoip_sip_core::validation::{validate_notify_request,
  validate_publish_request, validate_subscribe_request,
  validate_wire_request, validate_wire_response, validate_content_length}`
  — invoked by `HeaderPolicy::validate_outbound` where the method matches.
  **Audit note:** `validate_register_request` and `validate_refer_request`
  do **not** exist in sip-core today; for those methods the policy layer
  on the rvoip-sip side does the application-slice check and the wire
  validator catches any structural issue at the transaction layer.
- `rvoip_sip_core::parse_message` — for inbound `IncomingCall.request`
  attachment if the state machine didn't already retain the parsed form
- `prepend_outbound_proxy_route` (`adapters/dialog_adapter.rs:2086`) —
  reused by all 11 new `send_*_with_options` adapter methods
- `rvoip_sip_dialog::transaction::utils::{request_builders,
  response_builders}` — dialog-core's existing in-dialog builders; the
  new `*_with_options` API appends `extra_headers` after they run
- `SipTraceConfig` + `Event::SipTrace` — already the pattern for
  wire-level test observation; new tests reuse the helper from
  `tests/extra_headers_integration.rs`
- `StateMachineHelpers::make_call_inner`, `register_*`, `send_refer_*`,
  etc. — promoted to `pub(crate)` so builders call them directly
  without going through five wrapper methods

## Verification

End-to-end test plan, run in order; each must pass before the next:

1. `cargo build -p rvoip-sip-dialog` — additive options structs compile;
   no existing call sites broken
2. `cargo test -p rvoip-sip-dialog` — full dialog-core suite still passes
3. `cargo build -p rvoip-sip` — builders + policy compile
4. `cargo doc -p rvoip-sip --no-deps` — clean
   (`#![deny(rustdoc::broken_intra_doc_links)]`)
5. `cargo test --doc -p rvoip-sip` — every new `with_*` setter has a
   doc-test; ~60 new doc-tests added
6. `cargo test -p rvoip-sip --test header_policy_unit` — policy table
   covers every `TypedHeader` variant + the per-method `MethodShaped`
   overrides
7. `cargo test -p rvoip-sip --test forbidden_header_guard_integration` —
   `with_header(TypedHeader::CallId(...))` returns
   `Err(HeaderPolicyViolation { reason: StackManaged, .. })`;
   `with_header(TypedHeader::Authorization(...))` on a builder where
   `with_credentials` is the right path returns
   `Err(.. UseDedicatedSetter("with_credentials"))`;
   `with_headers_from` returns `Ok((_, report))` with Via/CSeq/Call-ID/
   Max-Forwards in `report.skipped`
8. `cargo test -p rvoip-sip --test header_inspection_integration` —
   inbound INVITE / mid-dialog REFER / NOTIFY / INFO / failure response
   surfaces have `Diversion`, `History-Info`, `Referred-By`,
   `Retry-After` accessible via `SipHeaderView`
9. `cargo test -p rvoip-sip --test outbound_request_builders_integration`
   — each of the 12 builders (INVITE / re-INVITE / REGISTER / REFER /
   BYE / CANCEL / NOTIFY / SUBSCRIBE / INFO / UPDATE / MESSAGE /
   OPTIONS) sends an asserted-on-wire custom `X-Test` header
10. `cargo test -p rvoip-sip --test response_builders_integration` —
    reject with `Retry-After`, redirect with multiple `Contact` entries +
    q-values, accept with custom header, 401 challenge with
    `WWW-Authenticate`, 407 with `Proxy-Authenticate`
11. `cargo test -p rvoip-sip --test b2bua_carry_through_integration` —
    the §8 example actually executes: inbound INVITE → outbound INVITE
    carrying `History-Info` and `Diversion`, stripping `Privacy`,
    rewriting PAI; wire trace on both legs validates ordering;
    carry-through correctly drops `Via`, `CSeq`, `Call-ID`,
    `Max-Forwards`, `Content-Length` and reports them in
    `HeaderCarryThroughReport.skipped`
12. `cargo test -p rvoip-sip --test builder_strictness_integration` —
    `BuilderStrictness::Strict` rejects a `with_header(Authorization(...))`
    on `RegisterBuilder` with `Err(UseDedicatedSetter("with_credentials"))`;
    `BuilderStrictness::Lenient` silently drops it with a warn log and
    proceeds; both modes reject `with_header(CallId(...))` as hard
    `Err(StackManaged)`. Adds a guard test that the wire-level
    `validate_wire_request` still runs regardless of `BuilderStrictness`
    (force an internally malformed body and assert it fails at the
    transaction layer).
13. `cargo test -p rvoip-sip --test config_builder_coexistence` —
    the three worked examples from "Config + builder coexistence"
    (pure Config, pure builder, mixed) all produce wire output that
    matches an expected fixture under `tests/fixtures/`.
14. `cargo test -p rvoip-sip` — full suite, including the legacy
    `pai_integration.rs` and `extra_headers_integration.rs` (proves
    `#[deprecated]` wrappers still work)
15. `cargo build --examples -p rvoip-sip` — examples still compile
    despite emitting deprecation warnings
16. Manual: open `target/doc/rvoip_sip/index.html`. The crate-level
    `//!` now has a "Gateway / B2BUA / SBC Authoring" section with the
    §8 example and a header-classification reference table (StackManaged
    / MethodShaped / ApplicationControlled). Each new builder type page
    shows its trait impls (`SipRequestOptions`) and links across.

## Verified findings & layer-respect refinements

A layer-by-layer code review across `rvoip-sip`, `rvoip-sip-core`,
`rvoip-sip-dialog`, and `rvoip-sip-transport` confirmed that the layer
boundaries the plan assumes are intact today. This section records
what was verified, what changed in the design as a result, and how
the new builder surface must thread through the existing message-bus
and state-machine plumbing without disturbing it.

### Layer separation — verified

- **`rvoip-sip-transport`** builds **zero** SIP messages outside
  `#[cfg(test)]`. It owns sockets, parses inbound bytes via
  `rvoip_sip_core::parse_message`, serializes outbound via
  `Message::to_bytes`, and emits `TransportEvent`s. Holds no dialog
  or transaction state. The design needs **no changes** here.
- **`rvoip-sip-core`** is a true foundation crate (no internal rvoip
  deps). `SimpleRequestBuilder` / `SimpleResponseBuilder`, `parse_message`,
  every cited validator, every cited `Method` variant, and ordered
  `headers: Vec<TypedHeader>` on both `Request` and `Response` are
  present. Bodies use `bytes::Bytes`. **No changes** here.
- **`rvoip-sip-dialog`** is the authoritative owner of CSeq
  (`DialogImpl::local_cseq` at `src/dialog/dialog_impl.rs:47`,
  incremented by `increment_local_cseq()` line 706), Route-Set
  (`route_set: Vec<Uri>` line 56, applied via
  `request_builder_from_dialog_template`), Contact for in-dialog
  requests, and `From`/`To` tags once a dialog is established. The
  proposed `*_with_options` additions append application headers
  **after** the dialog template runs (`transaction/dialog/mod.rs:107-118`
  is the existing append point), so dialog state stays authoritative.
- **`rvoip-sip`** orchestrates session lifecycle and the state machine.
  Owns no transport sockets and no dialog state. The state machine
  emits `Action::*` variants that the `DialogAdapter`
  (`src/adapters/dialog_adapter.rs`) translates into dialog-core calls.

### Cross-layer mechanics: how options reach the wire

The verified outbound INVITE path today:

```
UnifiedCoordinator.make_call_with_headers(...)        // src/api/unified.rs:1808
  → StateMachineHelpers::make_call_inner             // src/state_machine/helpers.rs:215
    • stashes extra_headers on SessionState.extra_headers
    • emits EventType::MakeCall (in-process channel, not the bus)
  → state_machine/actions.rs handler                  // emits Action::SendINVITE
  → DialogAdapter::send_invite_with_extra_headers     // src/adapters/dialog_adapter.rs:926
    • reads extra_headers off SessionState
    • prepends outbound-proxy Route via prepend_outbound_proxy_route (adapter:2086)
  → UnifiedDialogApi::make_call_with_extra_headers_for_session  // dialog/src/api/unified.rs:617
  → transaction/dialog/* and transaction layer → socket
```

Three layer-respect rules the new builders must honor, made explicit:

1. **Outbound options do NOT travel on `GlobalEventCoordinator`.**
   The `rvoip-infra-common` event bus
   (`rvoip_infra_common::events::coordinator::GlobalEventCoordinator`)
   carries cross-crate **state-change notifications** —
   `SessionToDialogEvent`, `DialogToSessionEvent`, `SipTraceEvent`,
   `DialogCreated`, `DialogTerminated`, etc. — never request payloads
   for outbound authoring. New builder options follow the same
   in-process path the INVITE `extra_headers` use today: stash on
   `SessionState`, emit a state-machine `Action`, `DialogAdapter`
   reads the stash and forwards. **No new `CrossCrateEvent` variants
   are added for the outbound path.** Adding them would force the
   bus to serialize/deserialize options structs and would cross the
   layer boundary the bus is meant to keep loose.
2. **The `Action` enum gains one variant per new in-dialog method.**
   Phase C adds (or reshapes) `Action::SendBYEWithOptions`,
   `Action::SendCANCELWithOptions`, `Action::SendREFERWithOptions`,
   `Action::SendNOTIFYWithOptions`, `Action::SendINFOWithOptions`,
   `Action::SendUPDATEWithOptions`, `Action::SendREINVITEWithOptions`,
   `Action::SendSUBSCRIBEWithOptions`, `Action::SendMESSAGEWithOptions`,
   `Action::SendOPTIONSWithOptions`, `Action::SendREGISTERWithOptions`.
   Each carries an `Arc<XxxRequestOptions>` (cheap to clone within
   the state-machine action loop). Existing `Action` variants without
   `WithOptions` stay valid and dispatch with `Default::default()`
   options. The state machine itself is unchanged structurally — it
   already routes `Action`s to the adapter; only the per-action
   payload widens.
3. **Inbound enrichment IS a bus change — minimal additive, but the
   payload type matters.** Phase E needs the parsed `Request` to ride
   from dialog-core to rvoip-sip so
   `IncomingRequest`/`IncomingResponse`/`IncomingRegister` can wrap it.

   **Layer constraint (critical):** the cross-crate event enums live in
   `infra-common::events::cross_crate`
   (`DialogToSessionEvent` at `cross_crate.rs:514`). `infra-common`'s
   `Cargo.toml` has **no `rvoip-sip-core` dependency** today —
   deliberately, so the bus stays SIP-agnostic. Adding a
   `request: Arc<rvoip_sip_core::Request>` field to those variants
   would force a new dependency arrow `infra-common → rvoip-sip-core`,
   breaking the foundation-crate isolation that lets every layer share
   the bus without leaking SIP types.

   **Resolution:** carry the raw wire bytes as `Arc<bytes::Bytes>` (the
   `bytes` crate is already a workspace dep on both sides and is what
   `rvoip-sip-core` uses for `Request.body`). The rvoip-sip side
   re-parses with `rvoip_sip_core::parse_message` (cheap: the message
   was already validated upstream; parse is purely structural) and
   wraps the result in `IncomingRequest`/`IncomingResponse`/`IncomingRegister`.

   **Plumbing note (verified):** dialog-core today holds the parsed
   `Request` at publish time but **strips it to pre-decoded scalar
   fields** before constructing the bus variant. Verified publish
   sites:
   - REGISTER → `crates/rvoip-sip-dialog/src/protocol/register_handler.rs:92-108`
   - NOTIFY → `crates/rvoip-sip-dialog/src/manager/protocol_handlers.rs:656-728`
   - REFER → `crates/rvoip-sip-dialog/src/events/event_hub.rs:588-596`

   Phase E gives each publish site one extra line:
   ```rust
   raw_request: Arc::new(request.to_bytes()),   // re-serialize from typed
   ```
   `Request::to_bytes` (sip-core) is canonical and round-trips. If
   preserving the *exact* inbound byte sequence is required for some
   B2BUA use case (e.g., signature verification across the bus), an
   optional follow-up holds the original `Bytes` from the parse step
   in the dialog handler and threads it to the publish site — but
   re-serialization is the default because it keeps the parse layer
   stateless and meets every other B2BUA need (the parsed semantics
   match exactly; only insignificant whitespace and header-case may
   differ).

   ```rust
   // infra-common::events::cross_crate.rs — additive fields
   TransferRequested  { ..existing.., raw_request: Arc<Bytes> }   // REFER
   NotifyReceived     { ..existing.., raw_request: Arc<Bytes> }
   IncomingRegister   { ..existing.., raw_request: Arc<Bytes> }
   // NEW variants (no analog today):
   InfoReceived       { session_id: String, raw_request: Arc<Bytes> }
   MessageReceived    { session_id: String, raw_request: Arc<Bytes> }
   OptionsReceived    { session_id: String, raw_request: Arc<Bytes> }
   UpdateReceived     { session_id: String, raw_request: Arc<Bytes> }
   //                  ^ today UPDATE is folded into ReinviteReceived;
   //                    Phase E either enriches ReinviteReceived with
   //                    `raw_request` OR adds the dedicated variant
   ```

   `Arc<Bytes>` is already cheap-clone (Bytes is internally an Arc); the
   `Arc<...>` wrap is belt-and-braces against the field being copied
   through several subscribers. Existing consumers that don't read
   `raw_request` are unaffected (additive field).

   **Variant naming corrected from earlier drafts:** the cross-crate
   variant for inbound REFER is `TransferRequested` (`cross_crate.rs:667`),
   **not** `ReferReceived`. `NotifyReceived` (line 713) and
   `IncomingRegister` (line 734) exist. `InfoReceived`,
   `MessageReceived`, `OptionsReceived`, and `UpdateReceived` (or an
   enriched `ReinviteReceived`) **do not exist today and must be added**
   as part of Phase E.

### Configuration merge semantics (`Config` + builders coexist)

The user requirement is that callers can continue to use the existing
`Config` struct (`src/api/unified.rs:185`) and/or the new builders. The
builders inherit Config defaults at the `DialogAdapter` boundary, and
override them with builder-supplied values when present. Merge rules
the adapter applies just before invoking dialog-core:

| Field | Resolution order (highest priority first) |
|---|---|
| `From` URI / display | builder.with_from_uri / .with_from_display → surface.local_uri (Endpoint/StreamPeer/CallbackPeer) → Config.local_uri |
| `Contact` URI | builder.with_contact_uri → Config-derived from `sip_contact_mode` |
| `P-Asserted-Identity` | builder.without_pai disables; builder.with_pai overrides; else Config.pai_uri |
| `Authorization` (UAC) | builder.with_precomputed_authorization → builder.with_credentials → Config.credentials (consumed on 401/407 retry) |
| `Route` (outbound proxy) | always prepended unless builder.without_outbound_proxy is set; source is Config.outbound_proxy_uri |
| `User-Agent` | builder.with_header(UserAgent) wins; else Config-driven default (if set) |
| `Allow` | stack-managed by dialog/transaction; builders cannot override |
| `Max-Forwards`, `Via`, `Call-ID`, `CSeq`, `Content-Length` | stack-managed; not reachable via builder |

Two additional opt-out setters on every relevant builder, for parity
with Config-driven defaults:

```rust
.without_pai()             // suppress Config.pai_uri for this request
.without_outbound_proxy()  // suppress Config.outbound_proxy_uri for this request
```

`EndpointBuilder` and `EndpointConfig` (`src/api/endpoint.rs`) retain
their existing `.configure(|cfg: &mut Config| ...)` escape hatch for
mutating Config before the endpoint starts — this is unaffected.

### Typed-header coverage gaps in `rvoip-sip-core`

Five headers the design treats as application-controlled have **no
typed `TypedHeader` variant** in sip-core today:

- `Diversion` (RFC 5806)
- `History-Info` (RFC 7044)
- `Privacy` (RFC 3323)
- `Replaces` (RFC 3891) — mentioned in docstrings only
- `Target-Dialog` (RFC 4538) — caught in the 2026-05-11 audit

`TypedHeader::Other(HeaderName::Other(name), value)` works for all five
and is the canonical builder path; `with_raw_header("Diversion", "...")`
goes the same way. To preserve B2BUA ergonomics, `rvoip-sip` ships
typed helper constructors that produce the correctly-cased `Other`
variant — see the expanded list in the **Typed-header convenience
expanded** subsection of "Layer-audit refinements" below.

`classify()` normalizes `HeaderName::Other("Diversion")` identically to
`HeaderName::Diversion` if/when sip-core later promotes them — no
builder API change is needed at that point.

### Header-policy classification table (completed)

The plan's `HeaderPolicy::classify(method, name)` function must encode
the following matrix for the method-aware cells:

| Header | INVITE | re-INVITE | REGISTER | REFER / BYE / CANCEL / NOTIFY / INFO / UPDATE | SUBSCRIBE (init) | SUBSCRIBE (refresh) | MESSAGE / OPTIONS | 3xx response |
|---|---|---|---|---|---|---|---|---|
| `Contact` | stack | stack | **app** | stack | **app** | stack | **app** | **app** |
| `Authorization` | shaped | shaped | shaped | stack | shaped | stack | shaped | n/a |
| `Expires` | n/a | n/a | shaped (`with_expires`) | n/a | shaped (`with_expires`) | shaped | n/a | n/a |
| `Route` | stack | stack | stack | stack | stack | stack | stack | n/a |
| `Refer-To` | n/a | n/a | n/a | shaped (REFER ctor) | n/a | n/a | n/a | n/a |
| `Event`, `Subscription-State` | n/a | n/a | n/a | shaped (NOTIFY setter) | shaped (`event`) | shaped | n/a | n/a |

`Path` (RFC 3327), `Service-Route` (RFC 3608), `P-Charging-Vector`,
`Reason`, `Retry-After`, `Warning`, `Subject`, `Date`,
`P-Asserted-Identity`, `P-Preferred-Identity`, `Reply-To`,
`Target-Dialog`, all `X-*`, and every `Other(...)` remain
unconditionally **application-controlled** regardless of method.

`with_raw_header(name, value)` runs the **same** classify check as
`with_header(typed)` — passing `"Call-ID"` as a raw name still hits the
`StackManaged` violation. The policy module canonicalizes the name
before classification so mixed-case strings (`"call-id"`, `"Call-Id"`)
behave identically.

### Validation runs on application-staged headers only

`HeaderPolicy::validate_outbound(method, headers)` is invoked by each
builder's `.send()` **before** the options struct crosses into
dialog-core. At that point only application-staged headers are visible;
CSeq, Call-ID, Via, From-tag, To-URI, Max-Forwards, Content-Length, and
in-dialog Contact have not yet been stamped. This is by design: the
builder layer's validation enforces the *application-controlled* slice;
the *stack-managed* slice cannot be corrupted because it does not exist
in the staged-headers vector.

`rvoip_sip_core::validation::{validate_wire_request, validate_wire_response}`
already runs inside the transaction layer after the full request is
assembled — that is a separate, pre-existing belt-and-braces check.

### Body types — standardize on `Bytes`

Every builder `with_body(...)` / options-struct `body` field takes /
holds `bytes::Bytes`. sip-core uses `Bytes` natively on
`Request.body` / `Response.body`. Builder setter signature is
`with_body(impl Into<Bytes>)`, which accepts `String`, `&str`,
`Vec<u8>`, and `Bytes` without copies on assignment to the request
struct.

### AuthChallengeBuilder — UAS-mode surfaces only

A digest challenge is emitted by a UAS in response to an inbound
request. The `AuthChallengeBuilder` is reachable from:

- `IncomingCall::challenge_builder(scheme)` — challenges an inbound INVITE
- `IncomingRequest::challenge_builder(scheme)` — challenges any in-dialog request
- `IncomingRegister::challenge_builder(scheme)` — challenges an inbound REGISTER
- `UnifiedCoordinator::challenge(&session, scheme)` — explicit-session form

It is **not** exposed on `Endpoint`, `StreamPeer`, or `CallbackPeer`
as a top-level builder, because those surfaces are UAC-shaped for the
common case. Callback handlers receive `IncomingCall` / `IncomingRequest`
and can call the builder from there.

### Surface-adapter ergonomics

Per-surface builder wrappers (`PeerInviteBuilder`,
`EndpointInviteBuilder`, etc.) implement `SipRequestOptions` by
deferring to the inner core builder. The terminal `.send()` is **not**
on the `SipRequestOptions` trait — each surface's wrapper provides its
own `.send()` with the surface-appropriate return type
(`SessionHandle` for Endpoint/Peer surfaces, `SessionId` for
`UnifiedCoordinator`). This keeps the trait object-safe without forcing
a single return type and matches the existing per-surface return-type
asymmetry.

### Deprecation already accommodated by lint config

The workspace already sets `deprecated = "allow"` at the lint level
(`Cargo.toml:54-67`). The ~14 `#[deprecated]` markings the design adds
will not break existing examples, `rvoip-sip-registrar`, or other
internal consumers that build through this workspace. External consumers
get standard deprecation warnings on `cargo build`.

### Things the verification flagged that the design now corrects

| Item | Earlier draft said | Corrected to |
|---|---|---|
| `SipHeaderView::header_wire_value` | "raw header value as it appeared on wire" | dropped — sip-core stores typed-only; `header_str` via `Display` is the canonical wire-equivalent path; `Other(_)` displays the unchanged inbound value |
| Dialog-core `*_with_options` count | 6 methods | 12 methods (add re-INVITE, CANCEL, MESSAGE, OPTIONS, subscribe-refresh). **`send_options*` is NEW authorship** — no pre-existing dialog-core method to wrap. |
| MESSAGE / OPTIONS authorship | "bypass dialog-core, authored in rvoip-sip" | dialog-core hosts them; `send_message_with_options` and `send_options_with_options` are additive on `UnifiedDialogApi` |
| `Event::InfoReceived` | implied to exist | does not exist today; Phase E adds it |
| `CallHandler::on_info` | implied to exist | does not exist today; Phase E adds `on_info_full`, `on_message_full`, `on_options_full` (default no-op) |
| `adapter:2087` | line ref | actual is `adapters/dialog_adapter.rs:2086` |
| `HeaderCarryThroughReport` source | inbound only | also from inbound responses (`IncomingResponse` implements `SipHeaderView`) — supports B2BUA carrying `Allow` from a 200 OK on one leg to the other |
| Re-INVITE extra headers | implicit (covered by "INVITE") | re-INVITE uses generic `send_request()` today and needs its own `send_reinvite_with_options` in dialog-core |
| Options structs `Default` derive | unspecified | every options struct derives `Default` for `..Default::default()` ergonomics |

---

---

## Layer-audit refinements (2026-05-11)

A second-round code audit across all four crates yielded the following
corrections and additions. The earlier "Verified findings &
layer-respect refinements" section captured the first-round findings;
this section supersedes it where the two conflict.

### Layer-separation verdict: PASS — with named risks

| Crate | Audit verdict | Risk addressed by |
|---|---|---|
| `rvoip-sip-transport` | Pure byte pipe. Zero message construction in production code (test code excepted). No state. No semantic header reads. No changes required. | n/a |
| `rvoip-sip-core` | True foundation crate (no internal rvoip deps, verified in `Cargo.toml`). Has `Vec<TypedHeader>` storage, `bytes::Bytes` bodies, lossless `Display` for `TypedHeader::Other`, and `ParseMode::{Strict, Lenient}` already implemented. Two RFC-method-specific validators missing (`validate_register_request`, `validate_refer_request`). | Builder runs `validate_wire_request` (which exists and runs at the transaction layer regardless); per-method policy lives in `HeaderPolicy::validate_outbound` on the rvoip-sip side. No sip-core changes. |
| `rvoip-sip-dialog` | Owns CSeq, Route-Set, in-dialog Contact, From/To tags. Existing in-dialog request builder accepts `extra_headers` and appends *after* stack-managed headers (`transaction/dialog/mod.rs:102-196`). **`send_options` does not exist at all** — Phase B authors a new path. Internal `DialogEvent` carries only `NotifyReceived` (no `ReferReceived`, no `InfoReceived`). | Phase B adds the 11 `*_with_options` methods including the new `send_options_out_of_dialog_with_options`; `RegisterRequestOptions` (already at `unified.rs:228-239`) gains `extra_headers` + `#[derive(Default)]`. |
| `rvoip-sip` | No transport, no dialog state. Response-side custom header authoring is **impossible today** (`SendSIPResponse` / `SendRejectResponse` / `SendRedirectResponse` synthesize from state with no header-injection hook). State-machine `Action` enum lacks `SendREFER` / `SendINFO` / `SendUPDATE` / `SendOPTIONS` — these methods bypass the state machine today and call `DialogAdapter` directly. | Phase D's response-builder tree adds the missing authorship surface. Phase C keeps the direct `DialogAdapter` path for in-dialog non-INVITE methods; only INVITE / re-INVITE / REGISTER / SUBSCRIBE / MESSAGE / NOTIFY widen their existing `Action` payload to carry `Arc<XxxRequestOptions>`. |
| `infra-common` | Hosts the bus and defines `DialogToSessionEvent` / `SessionToDialogEvent`. **No `rvoip-sip-core` dependency** — cross-crate event payloads cannot carry `Arc<rvoip_sip_core::Request>` without breaking the foundation-crate isolation. | Phase E uses `Arc<bytes::Bytes>` as the cross-crate payload and re-parses on the rvoip-sip side. See "Cross-layer mechanics §3" above. |

### Audit-driven corrections to specific design claims

| Item | Earlier draft | Corrected |
|---|---|---|
| Cross-crate event enum location | "in rvoip-sip-dialog" | Actually `infra-common::events::cross_crate.rs:514`. Phase E's bus changes are in `infra-common`, not `rvoip-sip-dialog`. The audit confirmed `infra-common` has no sip dep, ruling out `Arc<Request>` payloads — Phase E uses `Arc<Bytes>` and re-parses. |
| REFER inbound event variant | `DialogToSessionEvent::ReferReceived` | Actual variant is `TransferRequested` (`cross_crate.rs:667`). The rvoip-sip side decodes it into `Event::ReferReceived` (`api/events.rs:387-401`) — that pre-decoded `Event` already exists and is what's surfaced to applications today. |
| `Event::InfoReceived` | Implied to exist | Does not exist. Phase E adds it. INFO today reaches the application only through `on_event(...)`. |
| Inbound `OPTIONS` / `MESSAGE` routing | Implied to be in place | Verified: OPTIONS is **not handled at all** today; MESSAGE routes through `ProcessMESSAGE`. Phase E plumbs both. |
| `dialog_adapter.rs:2087` | line ref | Actual line is `adapter:2086` (already corrected once; double-confirmed). |
| `helpers.rs:215` `make_call_inner` | "Promote to pub(crate)" | No `make_call_inner` exists. Five sibling helpers (`make_call`, `make_call_with_credentials`, `make_call_with_pai`, `make_call_with_credentials_and_pai`, `make_call_with_headers_and_credentials_and_pai`, lines 99-156) collapse into a new `make_call_inner(opts)`. Phase C **authors** the equivalents for register/refer/notify/info/bye/cancel/update/subscribe/message/options — none exist today. |
| `RegisterRequestOptions` `Default` | unspecified | Today derives `Debug, Clone` only. Phase B adds `Default` AND `extra_headers`. |
| dialog-core `send_options*` | Implied wrap of existing method | No `send_options` method exists in dialog-core today. Phase B authors `send_options_out_of_dialog_with_options` on top of `transaction/utils/request_builders.rs`. |
| `DialogEvent::ReferReceived` (internal) | Implied | Doesn't exist — internal `DialogEvent` (`dialog_events.rs:11`) has only `NotifyReceived`. REFER reaches rvoip-sip via the cross-crate `TransferRequested` variant, not the internal one. |
| `CallHandler::on_transfer_request` | Implied to exist | Actual name is `on_refer_received` (the trait verified at `callback_peer.rs:814+`). `_full` companions throughout the design renamed accordingly. |
| `Endpoint::call_with_auth` | Implied to exist | Today `Endpoint` only exposes `call` + `call_with_headers`. The new builder API closes this gap (every surface gets `invite()` with `with_credentials`). |
| Missing sip-core validators | "Reused: validate_register_request / validate_refer_request" | Neither exists in `crates/rvoip-sip-core/src/validation/`. `HeaderPolicy::validate_outbound` does the per-method check; the wire-level `validate_wire_request` (which does exist) is the belt-and-braces check at the transaction layer. |
| State-machine Action additions | "One variant per new method" | Only INVITE / re-INVITE / REGISTER / SUBSCRIBE / MESSAGE / NOTIFY have today's Action variants. REFER / INFO / UPDATE / BYE / CANCEL / OPTIONS go through `DialogAdapter::send_*_with_options` directly without a state-machine action. Phase C does not add `SendREFER`/`SendINFO`/etc. — it widens the payload of existing variants and adds direct-dispatch helpers for the rest. |
| `src/adapter.rs` vs `src/adapters/dialog_adapter.rs` | Only the latter mentioned | Both exist; `adapter.rs` is a thin re-export shim (14 KB) over `adapters/dialog_adapter.rs` (80 KB). Phase C work targets the latter; `adapter.rs` only updates its re-export list. |

### Strict vs flexible outbound mode (NEW — addresses explicit user ask)

The user requested an explicit knob for strict-vs-flexible validation
of outbound messages. `rvoip-sip-core` already implements
`ParseMode::{Strict, Lenient}` (`crates/rvoip-sip-core/src/parser/message.rs:45-50`),
used today for inbound parsing. The builder layer surfaces a parallel
*outbound* knob:

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum BuilderStrictness {
    /// Default. Any HeaderPolicyViolation is a hard `Err`. Stack-managed
    /// names are always rejected (even in Lenient — they would desync
    /// the dialog).
    #[default]
    Strict,
    /// Method-shaped violations (`UseDedicatedSetter`) downgrade to a
    /// `tracing::warn!` and the offending header is silently dropped.
    /// WrongMethod violations also downgrade. StackManaged violations
    /// remain hard errors.
    Lenient,
}

pub trait SipRequestOptions: Sized {
    // …existing methods…
    fn with_strictness(self, mode: BuilderStrictness) -> Self;
}

impl Config {
    /// Default for every builder derived from this Config. Defaults to Strict.
    pub default_builder_strictness: BuilderStrictness,
}
```

Wire-level validation is unaffected by `BuilderStrictness` —
`rvoip_sip_core::validation::{validate_wire_request, validate_wire_response}`
runs inside the transaction layer regardless and is the final
correctness gate. Strict-vs-Lenient governs only the
*application-staged-headers* policy check that happens in the builder
layer before the message crosses into dialog-core.

### Outbound wire-validity & ordering guarantees (NEW — addresses explicit user ask)

The user explicitly asked: "we want sip headers to be safely built when
sent so we still check to make sure ordering and all outbound sip
messages are still valid". The design honors this through three layered
checkpoints:

1. **Builder layer (`rvoip-sip`).** `HeaderPolicy::validate_outbound`
   runs on `.send()` before the options struct crosses into dialog-core.
   Rejects stack-managed and (under Strict) method-shaped violations.
2. **Dialog/transaction layer (`rvoip-sip-dialog`).** The dialog
   template (`transaction/dialog/mod.rs:102-196`) and out-of-dialog
   request builder (`transaction/utils/request_builders.rs`) stamp
   `Via`, `Max-Forwards`, `Call-ID`, `CSeq`, `From`-tag, `To`-URI,
   `Content-Length`, and (for in-dialog) `Route-Set` and `Contact` in
   RFC-required order, *then* append application headers. Application
   `with_header` calls land in the tail slot and cannot reorder the
   stack-managed prefix.
3. **Wire layer (`rvoip-sip-core`).** `validate_wire_request` /
   `validate_wire_response` run inside the transaction layer after the
   full message is assembled. This is the final correctness gate; it
   runs regardless of `BuilderStrictness`. NOTIFY / SUBSCRIBE / PUBLISH
   additionally pass through `validate_notify_request` /
   `validate_subscribe_request` / `validate_publish_request`. (REGISTER
   and REFER lack dedicated validators in sip-core today; the per-method
   policy check on the rvoip-sip side covers the application slice.)

`Content-Length` is **never** stageable through `with_header` (it's
`StackManaged`) — it is stamped by `SimpleRequestBuilder` from the body
length immediately before serialization. Bodies travel on the options
struct's `body: Bytes` field, never via headers, and `with_body(impl
Into<Bytes>)` accepts `String`, `&str`, `Vec<u8>`, `Bytes` zero-copy.

### Config + builder coexistence (NEW — addresses explicit user ask)

The user explicitly asked that both paths remain first-class: keep
using `Config` and never touch a builder, OR opt into builders without
touching `Config`, OR mix the two. Three end-to-end examples:

**Path 1: Pure Config (today's experience, unchanged).** Defaults flow
from `Config` straight to the wire.

```rust
let cfg = Config::default()
    .with_local_uri("sip:alice@pbx.example")
    .with_pai_uri("sip:+15551234@pbx.example")
    .with_outbound_proxy_uri("sip:proxy.carrier.net");
let coord = UnifiedCoordinator::start(cfg).await?;
let id = coord.make_call("sip:bob@pbx.example").await?;   // works as today
```

**Path 2: Pure builder.** Builders override everything for a single
request without mutating `Config`. The surrounding Config can be a
minimal `Config::default()`.

```rust
let id = coord.invite(local_from, "sip:bob@pbx.example")
    .with_credentials(creds)
    .with_pai("sip:+15551234@pbx.example")
    .with_raw_header("X-Customer-ID", customer_id)?
    .send().await?;
```

**Path 3: Mixed (the B2BUA / SBC case).** `Config` carries the leg's
identity; builder overrides per-request fields and adds per-call
headers, with carry-through from the inbound side.

```rust
// Config provides: local_uri, outbound_proxy, default Credentials
let (outbound, report) = coord
    .invite(/*from=*/None, upstream)     // None → use Config.local_uri
    .with_headers_from(&incoming, &[
        HeaderName::HistoryInfo,
        HeaderName::Diversion,
    ])?
    .with_raw_header("P-Asserted-Identity", rewritten_pai)?
    .without_pai()                        // suppress Config.pai_uri this call
    .with_strictness(BuilderStrictness::Lenient);  // accept best-effort PAI rewrite
let session = outbound.send().await?;
```

The merge precedence the `DialogAdapter` applies is the table in
"Configuration merge semantics" above. Both opt-out setters
(`.without_pai()`, `.without_outbound_proxy()`) and per-request
overrides land in `SessionState` alongside the builder's options and
ride through to dialog-core via the existing `extra_headers` channel
plus the new options structs.

### Response-side carry-through worked example (NEW)

B2BUAs frequently need to carry headers from one leg's response onto
the other leg's response — e.g. propagating `Allow` / `Supported` /
`Server` from the upstream 200 OK to the downstream caller. The
`IncomingResponse` type implementing `SipHeaderView` is what enables
this; the example below was missing from earlier drafts.

```rust
async fn on_upstream_answered(
    &self,
    upstream_resp: IncomingResponse,
    inbound_call: IncomingCall,
) -> Result<()> {
    let (builder, report) = inbound_call.accept_builder()
        .with_sdp(answer_sdp_from_upstream)
        .with_headers_from(&upstream_resp, &[
            HeaderName::Allow,
            HeaderName::Supported,
            HeaderName::Server,
            HeaderName::Other("Session-Expires".into()),
        ])?;
    tracing::info!(
        copied = ?report.copied,
        skipped = ?report.skipped,    // Via/CSeq/Call-ID/Content-Length get filtered
        "downstream 200 OK carry-through audit"
    );
    builder.send().await.map(|_| ())
}
```

The same shape works for `RejectBuilder` carrying `Retry-After` /
`Warning` from an upstream failure, and for `RedirectBuilder` carrying
contact-list provenance from an upstream 3xx.

### Typed-header convenience expanded

`api::headers::convenience` ships typed helper constructors for every
header that lacks a `TypedHeader` variant in sip-core today (each
returns a `TypedHeader::Other(HeaderName::Other(name), …)` with
correctly-cased canonical name):

```rust
pub mod api::headers::convenience {
    pub fn diversion(value: impl Into<String>) -> TypedHeader;
    pub fn history_info(value: impl Into<String>) -> TypedHeader;
    pub fn privacy(value: impl Into<String>) -> TypedHeader;
    pub fn replaces(value: impl Into<String>) -> TypedHeader;
    pub fn target_dialog(value: impl Into<String>) -> TypedHeader;   // RFC 4538
    pub fn session_expires(value: impl Into<String>) -> TypedHeader; // RFC 4028
    pub fn min_se(seconds: u32) -> TypedHeader;                      // RFC 4028
    pub fn p_charging_vector(value: impl Into<String>) -> TypedHeader; // RFC 7315
    pub fn p_called_party_id(value: impl Into<String>) -> TypedHeader; // RFC 3455
}
```

`Target-Dialog` is the audit's only newly identified gap; `Session-Expires`,
`Min-SE`, and the `P-Charging-*` / `P-Called-*` family are added for B2BUA
ergonomics. `HeaderPolicy::classify` canonicalizes the name before
classification so all of these are treated identically whether the
application passes the convenience constructor's output or types out
`Other(HeaderName::Other("Diversion".into()), …)` by hand.

### Registrar surface boundary (clarification)

`rvoip-sip-registrar` is a downstream crate that currently reads inbound
REGISTER directly. The new `IncomingRegister` type + `challenge_builder()`
on `rvoip-sip` is **additive**: registrar continues to compile and run
unchanged. Migrating `rvoip-sip-registrar` onto `IncomingRegister` is
a follow-up PR outside this design's scope.

### Response-side authorship plumbing (Phase D detail)

Audit-confirmed flow for response authoring:

- `UnifiedDialogApi::send_response(transaction_id: &TransactionKey, response: Response)`
  (`crates/rvoip-sip-dialog/src/api/unified.rs:784-790`) exists today and
  takes a **fully-built `Response`**. This is the layer-correct hand-off
  point.
- `DialogAdapter::send_response(session_id, code, sdp)` at
  `dialog_adapter.rs:1064-1089` is the current thin pass-through that
  builds a basic `Response` from `(status_code, sdp)` and forwards.
- For provisional 183 with SDP, `send_early_media` exists at
  `state_machine/helpers.rs:298-310`. No 100 Trying / 180 Ringing
  authoring path exists today; Phase D adds them through
  `ProvisionalBuilder`.

Phase D builders (`AcceptBuilder`, `RejectBuilder`, `RedirectBuilder`,
`ProvisionalBuilder`, `AuthChallengeBuilder`) compose `Response`
objects inside `rvoip-sip` using `SimpleResponseBuilder` from sip-core,
then call `UnifiedDialogApi::send_response` directly. The new
`DialogAdapter::send_response_with_options` is a thin proxy that
forwards the pre-built `Response` and resolves the session's pending
transaction key — it does not re-build the message.

### AuthChallengeBuilder is NEW authorship (not a wrap)

Audit-confirmed: the only 401 sender in `rvoip-sip` today is
`RegistrationAdapter` at `src/adapters/registration_adapter.rs:112-132`,
and it challenges only inbound REGISTER. **No code path exists today
to send 401/407 in response to inbound INVITE or in-dialog requests.**

Phase D's `AuthChallengeBuilder` adds this surface from scratch:

```rust
incoming.challenge_builder(AuthScheme::Digest)
    .with_realm("pbx.example")
    .with_nonce(generate_nonce())
    .with_algorithm("MD5")
    .with_qop("auth")
    .send().await?;   // sends 401 Unauthorized + WWW-Authenticate
```

Builder uses `SimpleResponseBuilder::www_authenticate_digest`
(`crates/rvoip-sip-core/src/builder/response.rs:1480-1487`) for the
typed challenge, composes the 401/407 Response, and hands it to
`UnifiedDialogApi::send_response`. No new dialog-core method required —
the existing `send_response` accepts any pre-built `Response`.

### Surface types are inherent impls, not traits (corrected)

`PeerControl` and `CallbackPeerControl` (`stream_peer.rs:364-465`,
`callback_peer.rs:1025-1069`) are concrete structs with **inherent
impls**, not trait definitions. The design's earlier wording about
"trait methods" to deprecate is corrected: `#[deprecated]` annotations
go on the inherent methods directly. No trait changes are needed; the
new builder entry points are added as additional inherent methods on
the same structs.

### Phase A: incoming-Request attachment to `IncomingCall`

Audit confirmed `IncomingCall.headers: HashMap<String,String>` exists
at `api/incoming.rs:58` and is **never populated** (initialized empty
at line 80, no code fills it). Phase A:

1. Adds `request: Arc<rvoip_sip_core::Request>` to `IncomingCall`.
2. Implements `SipHeaderView` on `IncomingCall`, delegating to the new
   field.
3. Populates the field in `state_machine/actions.rs` at the point
   where `IncomingCall` is constructed for the `Action::AcceptCall`
   handler path. The parsed INVITE Request is already available from
   the inbound `DialogToSessionEvent::IncomingCall` variant — the
   audit confirmed `cross_crate.rs:516` carries pre-decoded fields
   today; Phase A also enriches the IncomingCall variant with
   `raw_request: Arc<Bytes>` and re-parses on the rvoip-sip side
   (same pattern as Phase E).
4. Fixes the empty-HashMap bug by populating from the parsed Request
   for back-compat with code that reads `IncomingCall.headers` today.
   Deprecate the field with a doc-note pointing readers at
   `SipHeaderView::header_str`.

### Open questions for the implementer

These are deliberately left open because the answer depends on
implementation decisions outside the scope of the design:

1. **CANCEL header injection.** RFC 3261 §9.1 requires the CANCEL
   request copy `Call-ID`, `From`, `To` (without tag), `CSeq` (with
   method changed to CANCEL), and the `Route` header from the original
   INVITE. Should `CancelBuilder.with_header(...)` allow application
   headers, or is the message wholly stack-managed? Recommendation:
   allow application headers but tighten `HeaderPolicy::classify(Cancel, ...)`
   so the RFC-required clones are all `StackManaged` — they cannot be
   overridden via the builder API.

2. **`UpdateReceived` vs `ReinviteReceived` consolidation.** Two
   options: (a) enrich the existing `ReinviteReceived` variant with
   `raw_request` and a `method: String` field (already present at
   `cross_crate.rs:663`) so consumers can branch on method;
   (b) add a dedicated `UpdateReceived` variant. Option (a) is one
   bus-payload change; option (b) is cleaner at the API surface.
   Recommend (a) for smaller blast radius.

3. **PUBLISH.** Out of scope per the existing non-goals, but the
   builder trait shape makes adding `PublishBuilder` trivial when
   needed — the bus and dialog-core do not need PUBLISH-specific
   plumbing because PUBLISH is non-dialog-bound.

---

## Second-round layer-audit refinements (2026-05-11, post-review)

A code-level re-audit across all four crates (rvoip-sip, rvoip-sip-core,
rvoip-sip-dialog, rvoip-sip-transport) plus `infra-common` confirmed
that **the plan honors layer separation** and **does not disturb the
state machine, dialog/transaction state, the cross-crate event bus, or
the transport pipe**. This section records gaps the re-audit surfaced
and the targeted refinements that close them. Where this section
conflicts with earlier wording in the document, this section wins.

### Layer-separation verdict (re-confirmed)

| Layer concern | Audit finding | Plan honors? |
|---|---|---|
| `rvoip-sip-transport` zero authorship | All `SimpleRequestBuilder` / `Message::new` use is `#[cfg(test)]`-gated. Production paths only `parse_message` inbound and `Message::to_bytes` outbound. Emits `TransportEvent` only. | ✅ Yes — design proposes no transport changes. |
| `rvoip-sip-core` foundation isolation | No `rvoip-*` deps in `Cargo.toml`. `Request.headers: Vec<TypedHeader>`, `body: Bytes`, `TypedHeader::Other` round-trips losslessly via `Display`. `ParseMode::{Strict, Lenient}` exists at `parser/message.rs:45-50`. | ✅ Yes — design proposes no sip-core changes. |
| Dialog/transaction state in `rvoip-sip-dialog` | `DialogImpl.local_cseq` (line 47), `route_set: Vec<Uri>` (line 56), `increment_local_cseq()` (line 706). In-dialog template at `transaction/dialog/mod.rs:102-196` appends `extra_headers` only **after** From/To/Call-ID/CSeq/Via/Max-Forwards/Contact/Route/Content-Type/Content-Length are stamped. | ✅ Yes — `*_with_options` additions append at the existing tail slot. State machine, route-set, CSeq, transaction core untouched. |
| `infra-common` bus SIP-agnostic | `infra-common/Cargo.toml` has **no `rvoip-sip-core` dep**. Cross-crate event payloads cannot carry `Arc<rvoip_sip_core::Request>` without breaking foundation isolation. Bus is in-process `broadcast::Sender<Arc<dyn CrossCrateEvent>>` (`events/coordinator.rs:111-125`); no wire serialization in current monolithic mode. | ✅ Yes — Phase A/E use `Arc<bytes::Bytes>` and re-parse on the rvoip-sip side. |
| `rvoip-sip` state-machine non-disturbance | `Action::Send*` variants in `state_table/types.rs:469-631` — payload-widening (existing variants gain `Arc<XxxRequestOptions>` payloads) is purely additive. The transition table (`state_table/yaml_loader.rs`) is **not** modified. REFER/INFO/UPDATE/OPTIONS continue to use direct `DialogAdapter` dispatch (no new Action variants). | ✅ Yes — state-machine semantics unchanged. |
| Config-vs-builder coexistence | `Config` at `api/unified.rs:185` with `local_uri` (204), `pai_uri` (236), `credentials` (228), `outbound_proxy_uri` (256), `sip_contact_mode` (331). All builder overrides flow through `DialogAdapter` merge precedence and never mutate `Config`. | ✅ Yes — Path 1/2/3 examples in "Config + builder coexistence" cover the three modes. |

**The plan, as written, does not push any layer into doing work it
shouldn't do.** The refinements below close gaps in specificity, not
violations.

### Audit-surfaced corrections to the body of the document

| Doc claim (earlier wording) | Audited reality | Effect on plan |
|---|---|---|
| `CallHandler::on_refer_received`, `on_notify_received` | The trait at `callback_peer.rs:814+` actually defines **`on_transfer_request(handle, target: String)` (line 894)**, **`on_refer_notify(handle, status, reason, subscription_state, body)` (line 904)**, and **`on_notify(handle, event_package, subscription_state, content_type, body)` (line 980)**. There is no `on_refer_received` / `on_notify_received`. | The `_full` companion methods in Phase E must be named `on_transfer_request_full`, `on_refer_notify_full`, `on_notify_full` (each taking `IncomingRequest`). Default impl pulls the same pre-decoded fields from the typed Request to forward to the legacy method. The "verified callback names" callout earlier in this document is the source of truth; treat any occurrence of `on_refer_received` / `on_notify_received` elsewhere as a doc-bug to be fixed during implementation. |
| Deprecation list for `StreamPeer` is `{call, call_with_headers}` | `StreamPeer::call_with_auth` exists at `stream_peer.rs:416`. | Phase C deprecation list must include `StreamPeer::call_with_auth`. The new `peer.invite(to).with_credentials(creds).send()` shape replaces it. |
| `Endpoint` has `{call, call_with_headers}` only | Confirmed — no `call_with_auth` on Endpoint today. | The new builder unifies these surfaces: Endpoint also gains a chainable `with_credentials` via its `endpoint.invite(to)` entry, closing a real gap (today Endpoint callers cannot authenticate outbound calls without the builder). |
| dialog-core `send_response` at `unified.rs:784-790` accepts a fully-built `Response` | Verified. | Phase D's `DialogAdapter::send_response_with_options(session_id, Response)` is a thin proxy that locates the pending transaction key and forwards. No new dialog-core entry point required for responses. |
| dialog-core `send_subscribe_with_options` / `send_subscribe_refresh_with_options` exist | Neither exists today. `send_register_with_options` is the **only** existing `*_with_options` method (`unified.rs:1250`). | Phase B authors both new methods on top of the existing `send_subscribe_out_of_dialog` and `send_subscribe_refresh` paths. |
| Inbound INFO / MESSAGE / OPTIONS / UPDATE flow on the cross-crate bus | **They do not.** Today these inbound requests publish to the **internal-to-dialog-core** `SessionCoordinationEvent` enum (`events/session_coordination.rs:17`): OPTIONS → `CapabilityQuery` (`protocol_handlers.rs:277, 526`), MESSAGE → internal handler, INFO routes through a co-located path, UPDATE shares the re-INVITE channel. The `event_hub.rs::convert_session_coordination_to_cross_crate` bridge at `event_hub.rs:182+` translates only a subset (IncomingCall, CallTerminating, TransferRequest, AckReceived, etc.) to `DialogToSessionEvent`. **CapabilityQuery is not bridged at all** — OPTIONS inbound never reaches rvoip-sip today. | **Phase E is doing more work than the earlier draft implied.** It must: (a) add the new variants to `DialogToSessionEvent` (`infra-common::events::cross_crate.rs`); AND (b) add new `SessionCoordinationEvent` variants in dialog-core where today's pre-decoded scalar fields are not enough to round-trip the original Request; AND (c) extend the conversion bridge in `event_hub.rs:182+` to map the new internal variants to the new cross-crate variants. See §"Phase E plumbing — three-stage" below for the explicit step list. |
| Phase A enriches `DialogToSessionEvent::IncomingCall` with `raw_request: Arc<Bytes>` | Variant verified at `cross_crate.rs:516`. Plan also needs the dialog-core publish site to re-serialize the parsed INVITE via `Request::to_bytes()` and attach. | Phase A scope is unchanged; the implementation note here is that the publish site for `IncomingCall` lives in dialog-core's call-handling path (paired with where it stamps `from`, `to`, `sdp_offer` today). Same pattern as Phase E for REGISTER/NOTIFY/REFER. |
| `bytes` is a workspace dep on infra-common | `bytes` is in the workspace root but **not** explicitly declared in `infra-common/Cargo.toml`. | Phase A (the first phase to touch `cross_crate.rs` with a `Bytes` field) must add `bytes = { workspace = true }` to `infra-common/Cargo.toml`. No new transitive deps; `bytes` is already pulled by `rvoip-sip-core` and `rvoip-sip-transport`. |
| `Default` on every options struct | `RegisterRequestOptions` (`unified.rs:228-239`) today derives `Debug, Clone` only. | Phase B adds `#[derive(Default)]` to `RegisterRequestOptions` AND every new `*RequestOptions` struct. The plan already calls this out; flagging here for completeness. |

### B2BUA / SBC / call-center gaps — explicit closure

The user requirement is that B2BUA / SBC / call-center applications can
**read and rewrite SIP headers freely** without breaking the dialog,
transaction, or transport layers. The audit confirmed the design's
carry-through model handles the common case; the gaps below close
specific real-world authoring needs.

#### Outbound `Contact` rewrite (B2BUA-critical)

**Gap.** B2BUAs must publish their own `Contact` URI on the outbound
INVITE — otherwise mid-dialog requests bypass the B2BUA and flow
endpoint-to-endpoint, breaking bridging and media anchoring. Today
`OutboundCallBuilder` derives `Contact` from `Config.sip_contact_mode`
with no per-call override.

**Resolution.** Add `with_contact_uri(uri)` to `OutboundCallBuilder`
and `RegisterBuilder`. For INVITE this controls only the **initial**
`Contact` on the outbound INVITE; dialog-core then takes over Contact
authority for the established dialog (per RFC 3261 §12, in-dialog
`Contact` is the locally-registered local target). Classification:
`MethodShaped { setter: "with_contact_uri" }` on INVITE and REGISTER;
remains `StackManaged` for every other in-dialog method. Merge
precedence in the "Configuration merge semantics" table updated:

| `Contact` URI (initial INVITE / REGISTER) | builder.with_contact_uri → surface.contact_uri → Config.sip_contact_mode |

#### Per-leg outbound-proxy override

**Gap.** A B2BUA may need to route different legs through different
outbound proxies (e.g., one carrier per termination region).
`.without_outbound_proxy()` exists in the plan but only **suppresses**
Config's default; there is no way to **redirect** to a different
proxy for one builder.

**Resolution.** Add `with_outbound_proxy(uri)` to every outbound
builder. Forwarded to `DialogAdapter` which already owns the
`prepend_outbound_proxy_route` helper at `dialog_adapter.rs:2086` —
the adapter consults the per-call override before falling back to
`Config.outbound_proxy_uri`. No new dialog-core or transport changes.

#### Inbound provisional-response surface (early-media B2BUA bridging)

**Gap.** B2BUAs bridging early media (183 Session Progress with SDP)
must inspect the upstream 183's `Contact`, `Record-Route`, `Allow`,
`Supported`, and `Server` headers and selectively forward them to the
downstream caller before the 200 OK lands. The plan introduces
`IncomingResponse` for non-2xx finals but is silent on provisional
inbound.

**Resolution.** `IncomingResponse` covers **every** inbound non-2xx
status, including 1xx provisional. The `Event::CallProgressDetailed
(IncomingResponse)` variant (added in Phase A) fires on **each** 1xx
upstream-side, not only finals. The pre-existing
`Event::CallProgress` (pre-decoded) stays in parallel for non-B2BUA
callers. Add a doc-example to Phase A showing 183 inspection.

#### Response-side per-leg authoring (UAS path completeness)

**Gap audit.** Response-side builders (Accept/Reject/Redirect/
Provisional/AuthChallenge) implement `SipRequestOptions` so
`with_header` / `with_headers_from` / `strip_header` work. But the
list of *method-shaped* responses needs explicit clarification:

- `100 Trying` is stack-emitted by the transaction layer per RFC 3261 §17.2.1; **not** authoring-exposed. The `ProvisionalBuilder` is for 18x only.
- `180 Ringing` / `183 Session Progress` go through `ProvisionalBuilder`; `with_sdp` valid only on 183.
- `200 OK` for INVITE goes through `AcceptBuilder`. For non-INVITE in-dialog (BYE/UPDATE/INFO/MESSAGE/OPTIONS), the **transaction layer** synthesizes 200 OK automatically with no application override — *unless* the app rejects the request first.

**Resolution.** Add a `GenericResponseBuilder` that accepts arbitrary
`status: u16` (3xx-6xx range gated for safety) for cases not covered
by the named builders (e.g., 491 Request Pending, 480 Temporarily
Unavailable with a custom `Retry-After`). Reachable from
`IncomingCall::respond_builder(status)` and
`IncomingRequest::respond_builder(status)`. Implements
`SipRequestOptions` exactly like the others. Wraps
`SimpleResponseBuilder` from sip-core; dispatches via
`UnifiedDialogApi::send_response`. **Does not** support 1xx (use
`ProvisionalBuilder`) or 2xx for INVITE (use `AcceptBuilder`).

#### REGISTER 3rd-party / on-behalf-of authoring (PBX/SBC pattern)

**Gap.** A PBX or SBC may need to register **on behalf of** an AOR it
doesn't own — `From`, `To`, and `Contact` URIs differ; the
`P-Asserted-Identity` may name the registering proxy not the AOR.
The plan's `RegisterBuilder.with_from_uri` covers `From`, but the
explicit pattern is missing from the doc.

**Resolution.** Add a "3rd-party REGISTER" worked example to the
"B2BUA composition example" section, demonstrating
`coord.register(registrar, user, pw).with_from_uri(behalf_uri)
.with_contact_uri(my_proxy_contact).with_raw_header("P-Asserted-Identity",
proxy_pai)?.send()`. No new API; documentation only.

### Phase E plumbing — three-stage (corrected)

Phase E is the most layer-sensitive phase. The audit confirmed the
publish-site plumbing requires **three** discrete additions in
dialog-core + infra-common, not one. Earlier wording implied a single
variant-enrichment pass; this section makes the full plumbing explicit.

For each new inbound method surfaced to applications
(INFO, MESSAGE, OPTIONS, UPDATE):

1. **Internal dialog-core event** — `events/session_coordination.rs`
   either gains a new `SessionCoordinationEvent` variant carrying the
   parsed `Request` (rvoip-sip-dialog already depends on rvoip-sip-core,
   so `Arc<Request>` is valid here), OR an existing variant's payload
   widens to include the parsed Request. This step is **internal to
   dialog-core** — no layer boundary crossed.
2. **Cross-crate event bridge** — `events/event_hub.rs:182+`
   (`convert_session_coordination_to_cross_crate`) maps the internal
   variant to a `DialogToSessionEvent` variant carrying
   `raw_request: Arc<bytes::Bytes>` (re-serialized via
   `request.to_bytes()`). This is the layer-boundary hop; the bus
   stays SIP-agnostic.
3. **Cross-crate variant authorship** — `infra-common::events::cross_crate.rs`
   gains the new variants (`InfoReceived`, `MessageReceived`,
   `OptionsReceived`; `UpdateReceived` may enrich the existing
   `ReinviteReceived` per the recommended consolidation).

The rvoip-sip side (state-machine handlers + `IncomingRequest`
construction) reads `raw_request` and re-parses via
`rvoip_sip_core::parse_message` to populate `IncomingRequest`,
exactly as Phase A does for `IncomingCall`.

**OPTIONS today is dropped at the dialog-core layer entirely** (the
internal `SessionCoordinationEvent::CapabilityQuery` is not bridged
to the cross-crate bus). Phase E therefore performs *new* authorship
for OPTIONS — not "promote an existing pipe."

### Builder Send+Sync requirement (call-center / B2BUA concurrency)

Every builder type (`OutboundCallBuilder`, `RegisterBuilder`,
`ReferBuilder`, `AcceptBuilder`, etc.) must be `Send + Sync` so
applications can `tokio::spawn` per-leg authoring tasks and pass
builders across `.await` points. The trait definition pins it:

```rust
pub trait SipRequestOptions: Sized + Send + Sync {
    // ...
}
```

`BuilderHeaderState` carries `Vec<TypedHeader>` which is `Send + Sync`
because `TypedHeader` already is. `Arc<XxxRequestOptions>` (stashed on
`SessionState`) is `Send + Sync`. No additional changes needed; this
constraint is documented to prevent accidental regression.

### Header-name canonicalization & case-insensitive lookup

Per RFC 3261 §7.3.1, SIP header names are case-insensitive. The audit
confirmed `HeaderName` in sip-core already canonicalizes via `as_str()`
(`types/headers/header_name.rs:162-215+`), but the design must specify
that the new API surfaces lower-case at the lookup boundary:

- `SipHeaderView::header(&name)`, `headers_named(&name)`, `header_str(&name)`,
  and `HeaderPolicy::classify(method, &name)` all canonicalize via
  `HeaderName::canonical_form()` before comparing. Mixed-case raw
  strings (`"X-Customer-ID"`, `"x-customer-id"`, `"X-CUSTOMER-ID"`)
  resolve to the same `Other(HeaderName::Other("X-Customer-ID"))`.
- `with_raw_header(name, value)` normalizes `name` to the canonical
  cased form at staging time so wire output is RFC-tidy.

### Auth retry preserves staged application headers

UAC requests that draw a 401 / 407 today are retried by the state
machine with credentials applied. Application-staged headers
(`with_header`, `with_raw_header`, `with_headers_from`) must survive
the retry — losing them would silently corrupt B2BUA carry-through.

**Resolution.** `Arc<XxxRequestOptions>` stashed on `SessionState`
is the source-of-truth for retry. The state machine's auth-retry
handler reads the same options struct on the retry pass and only
swaps in the computed `Authorization` / `Proxy-Authorization` header.
No application-staged header is dropped. Phase C test
`builder_auth_retry_preserves_headers` (added to the verification
matrix) asserts this on the wire.

### Trace / observability integration

`SipTraceEvent` (already published by both `rvoip-sip-transport` and
`rvoip-sip-dialog`) captures every outbound and inbound SIP message.
The new builders' `.send()` does **not** introduce a parallel trace
path — outbound traces continue to fire at the existing transport-layer
emission point, which means all `with_header` / `with_headers_from` /
carry-through results are visible on the trace identically to legacy
`make_call_with_headers` output today. Add an explicit assertion in
`b2bua_carry_through_integration.rs` that `SipTraceEvent` for the
outbound INVITE contains both the carried-through and rewritten
headers.

### CANCEL header-policy decision (resolved open question)

The earlier "Open questions" §1 left CANCEL semantics undecided. The
audit-confirmed RFC 3261 §9.1 requirement is that CANCEL **must**
copy `Call-ID`, `From`, `To` (without tag), `CSeq` (method changed),
and `Route` headers from the INVITE.

**Decision.** `CancelBuilder` allows application headers
(`with_header`, `with_raw_header`, `with_headers_from`). The RFC-required
fields (`Call-ID`, `From`, `To`, `CSeq`, `Route`) are `StackManaged` —
the builder layer rejects any attempt to override them and the dialog
layer (`transaction/utils/request_builders.rs::create_cancel_from_invite`)
clones them from the INVITE deterministically. Application headers
land in the same tail slot as for every other method.

### Stateless-proxy use case — out of scope (explicit)

The `rvoip-sip` crate is built around dialog-bound sessions. Pure
stateless SIP proxying (forwarding requests with `Via` push/pop only,
no dialog or session creation) is **not in scope** for this design.
B2BUA-style stateful proxying (each leg is a session, both legs
bridged) is in scope and is the primary B2BUA use case the design
serves.

A future stateless-proxy surface, if needed, would live as a new
crate or a new top-level module that talks directly to
`rvoip-sip-transport` and `rvoip-sip-core` (no dialog-core dependency).
The builder shapes introduced in this document would be reusable
verbatim, but the dispatch path would differ.

### Summary of additive changes vs. previous draft

| Section in this audit | New API surface added | Files newly touched | Layer crossed? |
|---|---|---|---|
| `with_contact_uri` on `OutboundCallBuilder` / `RegisterBuilder` | 2 setters | `src/api/send/outbound_call.rs`, `register.rs` | No (just adds an authoring option already supported at the wire). |
| `with_outbound_proxy(uri)` on every outbound builder | 1 setter on shared trait helper | `src/api/headers/options.rs`, `adapters/dialog_adapter.rs` (consults new override before `prepend_outbound_proxy_route`) | No. |
| `IncomingResponse` for 1xx provisional | New `Event::CallProgressDetailed` variant | `src/api/events.rs` | No (rides existing inbound pipe). |
| `GenericResponseBuilder` | New builder | `src/api/respond/generic.rs` | No (wraps `SimpleResponseBuilder` like the others). |
| `bytes = { workspace = true }` in `infra-common/Cargo.toml` | none | `infra-common/Cargo.toml` | No (build-system only). |
| `Send + Sync` bound on `SipRequestOptions` | none (assertion) | `src/api/headers/options.rs` | No. |
| Header-name case-insensitive lookup | none (assertion) | `src/api/headers/policy.rs`, `view.rs` | No. |
| Auth-retry preserves staged headers | none (test) | `tests/builder_auth_retry_preserves_headers.rs` | No. |
| Trace assertion in carry-through test | none (test) | `tests/b2bua_carry_through_integration.rs` | No. |
| CANCEL header-policy decision | classification only | `src/api/headers/policy.rs` | No. |
| Phase E three-stage plumbing | clarification | dialog-core `events/session_coordination.rs`, `events/event_hub.rs`, infra-common `events/cross_crate.rs` | Bridge stays at the dialog-core publish site; the bus payload stays `Arc<Bytes>`. |
| Trait-name corrections (`on_transfer_request_full`, `on_refer_notify_full`, `on_notify_full`) | rename in plan only | `src/api/callback_peer.rs` | No. |
| `StreamPeer::call_with_auth` to deprecation list | rename in plan only | `src/api/stream_peer.rs` | No. |

**None of these additions cross a layer boundary that was not already
crossed by the original plan.** Every new setter, builder, or assertion
either rides the existing in-process state-machine action loop, the
existing `DialogAdapter` direct-dispatch path, or the existing
cross-crate event bus with `Arc<Bytes>` payloads.

### Test additions

The following tests are added to the Phase C / D / E verification
matrix (numbered to extend the existing list):

17. `cargo test -p rvoip-sip --test b2bua_contact_rewrite_integration` —
    `OutboundCallBuilder::with_contact_uri` rewrites Contact on the
    outbound INVITE; wire trace asserts the override; dialog-core
    accepts it as the local target for the established dialog.
18. `cargo test -p rvoip-sip --test per_leg_outbound_proxy_integration` —
    `with_outbound_proxy(uri)` on one leg uses a different proxy than
    `Config.outbound_proxy_uri`; second leg uses the Config default.
19. `cargo test -p rvoip-sip --test provisional_carry_through_integration` —
    inbound 183 with SDP triggers `Event::CallProgressDetailed
    (IncomingResponse)`; B2BUA carries `Contact`/`Allow`/`Server` to
    downstream 183 via `ProvisionalBuilder::with_headers_from`.
20. `cargo test -p rvoip-sip --test generic_response_integration` —
    `IncomingCall::respond_builder(491).with_raw_header("Retry-After",
    "5")?.send()` produces a valid 491 Request Pending with the
    custom header.
21. `cargo test -p rvoip-sip --test builder_auth_retry_preserves_headers` —
    `coord.invite(...).with_credentials(creds).with_raw_header("X-Trace",
    id)?.send()` sees the X-Trace header on both the initial 401-drawing
    INVITE and the credentialed retry INVITE.
22. `cargo test -p rvoip-sip --test header_case_insensitive_lookup` —
    `with_raw_header("x-customer-id", ...)` and
    `headers_named(&HeaderName::Other("X-CUSTOMER-ID".into()))`
    resolve to the same staged header.
23. `cargo test -p rvoip-sip --test third_party_register_integration` —
    `coord.register(...).with_from_uri(behalf).with_contact_uri(proxy)
    .with_raw_header("P-Asserted-Identity", proxy_pai)?.send()`
    yields a wire REGISTER with the rewritten From/Contact/PAI.

---

## Third-round refinements — operational and interop corners (2026-05-11)

A third pass over the same four crates focused on the operational
corners B2BUA / SBC / call-center / registrar / softphone authors hit
once the builders are in use: stash lifecycle, multipart bodies,
reliable provisional, registrar response authorship, topology hiding,
trust boundaries, and timeouts. Where this section conflicts with
earlier wording it wins.

### Stash lifecycle (memory safety, retry correctness)

**Verified plumbing.** `SessionState` (`session_store/state.rs`) carries
the existing stash fields:

- `extra_headers: Vec<TypedHeader>` (line 190) — set at
  `state_machine/helpers.rs:256`, consumed by
  `DialogAdapter::send_invite_with_extra_headers` (`dialog_adapter.rs:926`).
- `pending_bye_reason: Option<(String, u16, Option<String>)>` (line 160) —
  set at `api/unified.rs:2001` when `hangup_with_reason` is called,
  consumed by the `Action::SendBYE` handler.
- `pending_reinvite: Option<PendingReinvite>` (line 134) — 491 retry state.

The doc earlier introduced `pending_<method>_options:
Option<Arc<XxxRequestOptions>>` fields alongside these. Three lifecycle
guarantees the implementer must preserve so the builders don't leak
state or replay stale options on a future call:

1. **Set-once, consumed-once.** Each `Action::Send*` handler reads the
   stash, clones the `Arc` into the dialog-adapter call, and then
   sets the field back to `None`. The existing `pending_bye_reason`
   pattern (`Option<...>` cleared by the action handler) is the template.
2. **Auth retry path re-reads, does not re-set.** The 401/407 retry
   loop reads the same `Arc<XxxRequestOptions>` for the retry
   transaction. The options struct stays in the stash until the
   transaction reaches a final response (success, terminal failure,
   or hard timeout). The state machine's existing
   `invite_auth_retry_count` (state.rs:122) drives the retry; the
   options stash piggybacks on the same lifecycle so application
   headers (`X-Trace-ID`, etc.) ride identically across both
   transactions.
3. **Session teardown clears all stashes.** When a session enters
   the `Terminated` state the stash fields are set to `None` /
   `Vec::new()` as part of normal cleanup. The existing
   `extra_headers: Vec::new()` reset (`state.rs:280`, init path) is
   the pattern; `pending_<method>_options` siblings follow the same
   discipline.

**Test addition (verification matrix #24):**
`cargo test -p rvoip-sip --test stash_lifecycle_integration` — assert
that after a successful `coord.invite(...).with_raw_header("X-Trace",
id)?.send().await` the session's `pending_invite_options` is `None`,
and that a subsequent re-INVITE on the same session does not see the
stale `X-Trace` header.

### Multipart bodies (SS7 ↔ SIP gateways, ISUP-tunneling B2BUAs)

**Gap.** SS7-SIP gateways emit INVITE bodies with `multipart/mixed`
containing `application/sdp` and `application/isup` parts. Telephony
B2BUAs need to pass these through (or strip ISUP) without
disturbing the SDP offer-answer.

**Verification.** `rvoip-sip-core`'s builders do **not** synthesize
multipart structure — `Request.body: bytes::Bytes` holds whatever
bytes the application provides, and the application sets
`Content-Type: multipart/mixed; boundary=…` explicitly. The wire is
already faithful.

**API resolution.**

- Every builder's `with_body(impl Into<Bytes>)` accepts pre-formed
  multipart bytes verbatim — no special-casing in the builder layer.
- A small helper module `api::headers::convenience::multipart`
  ships:

  ```rust
  pub fn multipart_mixed(parts: &[(&str, Bytes)]) -> (String, Bytes) {
      // returns (content_type with boundary, body bytes)
  }
  ```

  Returned tuple plugs directly into `with_content_type(s)` and
  `with_body(b)`. RFC 5621 boundary generation handled inside.
- `SipHeaderView` consumers parse inbound multipart via the same
  helper (`api::headers::convenience::multipart::parse(content_type,
  body) -> Vec<(String, Bytes)>`).
- B2BUA carry-through doctest: SS7 inbound INVITE → outbound INVITE
  preserving both SDP and ISUP parts.

**No new dialog-core or sip-core code.** The helper sits in `rvoip-sip`
and produces plain bytes + Content-Type that sip-core already serializes
correctly.

### Reliable provisional (RFC 3262) — `100rel` interaction

**Verified state.** Dialog-core has `UnifiedDialogApi::send_prack(&DialogId,
rseq: u32)` at `unified.rs:1502` — PRACK authorship plumbing exists.
The plan deliberately excludes a `PrackBuilder` from public API
because the state machine emits PRACK automatically on receipt of a
reliable 1xx.

**B2BUA / SBC concern.** When the upstream leg requires `100rel` and
the downstream leg does not (or vice versa), the B2BUA must either
(a) bridge 100rel end-to-end, or (b) terminate it. The builder API
needs to let the application drive this.

**Resolution.**

- `ProvisionalBuilder::with_require_100rel(bool)` (already listed in
  the design): when `true`, stamps `Require: 100rel` and `RSeq` on
  the outbound 1xx, and the state machine arms its PRACK-await
  timer per RFC 3262.
- `OutboundCallBuilder::with_supported_100rel(bool)` (new):
  controls whether the outbound INVITE advertises `Supported: 100rel`.
  Defaults to `false` (matches today's no-100rel default); SBCs
  bridging 100rel set it to `true` per-leg.
- `IncomingResponse::is_reliable_provisional() -> bool` convenience
  on the inbound response view: returns `true` when the upstream 1xx
  carries `Require: 100rel`. Lets B2BUA bridges decide whether to
  forward the 1xx with reliability semantics intact or to consume
  the reliability locally and forward an unreliable 1xx.

PRACK itself stays under state-machine control. The application
authors only the **1xx** (via `ProvisionalBuilder`) and the
**INVITE** (via `OutboundCallBuilder`); PRACK reuse the same `Arc`-shared
options for the dialog target. No new dialog-core method.

### Topology hiding — explicit guarantee for B2BUA

**Verified property.** Because `Via`, `Record-Route`, `Contact`
(in-dialog), `Call-ID`, `CSeq`, and `Max-Forwards` are `StackManaged`,
and because each B2BUA leg is an independent session with its own
dialog, **topology hiding is automatic**: the upstream leg's
`Via`/`Record-Route`/`Call-ID` never leak onto the downstream wire,
because dialog-core generates fresh values for the downstream leg.

**Explicit doc statement** (now in design): a B2BUA achieves
topology hiding by **not carrying through** the stack-managed
headers. The `with_headers_from(&inbound, names)` API rejects
stack-managed names automatically (per
`HeaderPolicy::forbidden_for_carry_through`), so a naïvely-written
B2BUA cannot accidentally leak topology. The `HeaderCarryThroughReport
.skipped` list logs every filtered header for audit.

**Threat-model addition.** The `Via` header on a 200 OK is the
SIP equivalent of an X-Forwarded-For chain. Carrying it across legs
defeats topology hiding. The policy table makes this impossible by
construction; no application-level discipline is required.

### Trust-boundary patterns — P-Asserted-Identity, History-Info, Privacy

**Gap.** B2BUAs/SBCs sit on trust boundaries. RFC 3325 says PAI must
be stripped when crossing from a trusted to an untrusted SIP network.
RFC 7044 says History-Info should not cross trust boundaries unless
the application explicitly permits. The plan provides the *mechanism*
(`.strip_header(name)`, `.without_pai()`) but doesn't document the
**discipline**.

**Resolution (documentation-only).** Add a "Trust boundaries" subsection
to the crate-level `//!` block with three template B2BUA patterns:

```rust
// 1. Trusted → untrusted egress: strip PAI, strip History-Info,
//    keep Diversion only if regulator-required.
let (out, _) = coord.invite(from, untrusted_target)
    .with_headers_from(&inbound, &[HeaderName::Diversion])?;
let out = out
    .strip_header(&HeaderName::Other("History-Info".into()))
    .without_pai();    // disregards Config.pai_uri

// 2. Untrusted → trusted ingress: ASSERT identity from local AAA,
//    do NOT carry through inbound PAI.
let (out, _) = coord.invite(asserted_from, trusted_target)
    .with_pai(local_aaa_resolved_identity)
    .with_headers_from(&inbound, &[/* nothing identity-related */])?;

// 3. Trusted-to-trusted (intra-domain): carry through verbatim.
let (out, _) = coord.invite(from, peer_pbx)
    .with_headers_from(&inbound, &[
        HeaderName::Other("P-Asserted-Identity".into()),
        HeaderName::Other("History-Info".into()),
        HeaderName::Other("Diversion".into()),
    ])?;
```

No new API — these are doctests in the crate root that travel with
`cargo doc` and that turn into compile-tested examples via the
existing doc-test infrastructure.

### Registrar UAS path — Service-Route, Path, Contact response authoring

**Verified state.** `RegistrationAdapter::send_register_response`
(`registration_adapter.rs:112-132`) is the only registrar-response
path today. It publishes a `SessionToDialogEvent::SendRegisterResponse`
with `status_code, reason, www_authenticate, contact, expires`.

**Gap.** Registrars built on top of `rvoip-sip-registrar` need to
attach **`Service-Route`** (RFC 3608), **`Path`** echo-back (RFC 3327),
**`P-Associated-URI`** (RFC 3455), and **`Min-Expires`** (RFC 3261
§10.2.8) headers on the 200 OK / 423 / 401 response. None of this
is reachable today.

**Resolution.** `IncomingRegister::accept_builder()` returns a
`RegisterResponseBuilder` implementing `SipRequestOptions`. Setters:

| Setter | Effect |
|---|---|
| `with_expires(u32)` | Sets per-contact Expires on the 200 OK |
| `with_min_expires(u32)` | Sets Min-Expires on a 423 response (only valid via `IncomingRegister::reject_builder().with_status(423)`) |
| `with_service_route(Vec<Uri>)` | Stamps Service-Route on 200 OK |
| `with_path_echo()` | Echoes inbound Path header on 200 OK |
| `with_associated_uri(Vec<Uri>)` | Stamps P-Associated-URI |
| `with_contact_from_binding(binding)` | Adds a Contact line per registered binding (multiple calls accumulate) |
| `with_raw_header(name, value)` | Same general policy check as outbound builders |

This sits next to `IncomingCall::accept_builder()` /
`IncomingRegister::challenge_builder()` already in the design. The
implementation uses `SimpleResponseBuilder::register_response` from
sip-core and dispatches via `UnifiedDialogApi::send_response`. **No
new dialog-core method required.**

The existing `SessionToDialogEvent::SendRegisterResponse` variant
in `infra-common::events::cross_crate.rs:502` widens additively to
carry an optional pre-built `Response` bytes payload — see Phase D
plumbing pattern.

### OPTIONS request timeout & response capture

**Verified state.** No `OPTIONS` plumbing exists at all today (no
`UnifiedDialogApi::send_options`, no `Action::SendOPTIONS`, no
inbound handler). Phase B / Phase C / Phase E all author from
scratch.

**Spec for the new `OptionsBuilder::send().await -> Result<IncomingResponse>`:**

- Default timeout: 32 seconds (Timer F default per RFC 3261 §7.1.1
  for non-INVITE transactions, derived from `T1 * 64`).
- `OptionsBuilder::with_timeout(Duration)` setter for application
  override. Reachable via `SipRequestOptions`-extension setter (does
  not belong on the trait itself because only OPTIONS expects a
  response back to the caller).
- On timeout: returns `Err(SendError::Timeout)`. The underlying
  non-INVITE transaction is destroyed by the transaction layer per
  RFC 3261 §17.1.2.4. No retransmission state leaks.
- On success: returns the full `IncomingResponse` so the caller can
  read `Allow`, `Supported`, `Accept`, `User-Agent`, `Server` —
  the entire reason OPTIONS is useful for B2BUA capability discovery.

`MessageBuilder::send().await -> Result<()>` does **not** return the
200 OK because MESSAGE is fire-and-forget per RFC 3428; the 200 OK is
absorbed by the transaction layer.

### Auto-emitted message header injection (state-machine-originated outbound)

**Gap.** The state machine emits some outbound requests **without**
application initiation: automatic BYE on session-timer expiry,
automatic CANCEL on `dialog_terminated_during_INVITE`, automatic
NOTIFY on subscription state change driven by upstream REFER. The
plan focuses on application-initiated paths; auto-emitted paths
inherit the stash but offer no fresh authoring hook.

**Resolution.** Three patterns, ordered by ergonomics:

1. **Stash before auto-emission.** Applications that need headers on
   the auto-emitted CANCEL/BYE/NOTIFY stash options proactively:

   ```rust
   coord.prepare_auto_emitted_options::<ByeBuilder>(&session, |b| {
       b.with_reason("SIP;cause=503;text=\"Upstream gone\"")
        .with_raw_header("X-Trace-ID", id).unwrap()
   });
   // Later, on session-timer expiry, the state machine emits BYE with
   // the prepared options.
   ```

2. **Event hook to author last-minute.** New
   `CallHandler::on_auto_emit_outbound(method, builder) -> builder`
   default-impl identity. Application overrides to mutate
   `BuilderHeaderState` immediately before dispatch. Synchronous
   (no `.await`) so it must be cheap. Optional; default no-op preserves
   today's behavior exactly.

3. **Per-Config defaults.** `Config` gains
   `auto_emit_extra_headers: Vec<TypedHeader>` for tenant-wide
   defaults (`User-Agent: MyApp/1.0`, `X-Operator: tenant-42`). Cheap
   default; applies to every outbound the state machine generates
   without per-call options. Stack-managed names still rejected;
   classify runs at Config-construction time so misconfigurations
   fail fast.

Pattern 3 is the recommended default for most B2BUA/SBC tenants —
no per-call code, no event hook, just a Config field. Patterns 1 and
2 cover the per-call-but-not-application-initiated case.

### Re-INVITE glare and 491 retry — staged header preservation

**Verified state.** `pending_reinvite: Option<PendingReinvite>`
(state.rs:134) carries 491 retry state per RFC 3261 §14.1. Today
the retry has no application headers.

**Resolution.** `Arc<ReInviteRequestOptions>` is stashed alongside
`pending_reinvite`; on 491 the retry transaction reads the same
`Arc` and includes application-staged headers identically to the
initial re-INVITE. Same lifecycle discipline as auth-retry:
set-once, consumed-on-final-response, cleared at termination.

### Subscription multiplex on a single dialog

**Concern.** Multiple subscriptions can ride one dialog (event
package multiplex per RFC 6665 §4.5.2). The `NotifyBuilder` needs to
target the right subscription. Plan's `coord.notify(&session,
event_package)` takes the package name, but the audit confirms today's
`UnifiedDialogApi::send_notify` (unified.rs:1449) and `send_refer_notify`
(unified.rs:1462) take the **dialog id**, not a subscription id.

**Resolution.** Add `NotifyBuilder::for_subscription(SubscriptionId)`
setter — when set, the dialog-core call carries the subscription id
through to the existing subscription manager; when omitted, the
single-subscription-on-dialog default applies (today's behavior).
No dialog-core API change required if the subscription manager
already accepts the id; otherwise add an optional field to
`NotifyRequestOptions`.

### Body type coverage — application/dtmf-relay, application/pidf+xml

**No change required.** `Bytes` plus `with_content_type(String)`
covers every body type. The convenience module ships factories for
the common SIP body types:

```rust
pub mod api::bodies {
    pub fn sdp(s: impl Into<String>) -> (String, Bytes);                // "application/sdp"
    pub fn dtmf_relay(signal: char, duration_ms: u32) -> (String, Bytes); // "application/dtmf-relay"
    pub fn pidf_xml(presence: &Presence) -> (String, Bytes);            // "application/pidf+xml" — RFC 3863
    pub fn simple_message_summary(...) -> (String, Bytes);              // "application/simple-message-summary" — RFC 3842
    pub fn isup_l3(bytes: impl Into<Bytes>) -> (String, Bytes);         // "application/isup" — RFC 3204
}
```

Returned `(content_type, body)` tuple plugs into `with_content_type(s)`
+ `with_body(b)`.

### Sensitive header masking on `SipTraceEvent`

**Concern.** SIP traces routinely carry `Authorization`, custom
`X-Auth-Token`, and PII-bearing fields like `P-Asserted-Identity`.
For compliance (GDPR, HIPAA) operators need a redaction hook before
traces hit logs.

**Resolution.** New `Config.trace_redaction: Option<Arc<dyn
TraceRedactor + Send + Sync>>`. The redactor receives the parsed
`Request` / `Response` before `SipTraceEvent` is published and
returns either `Allow`, `Redact(headers: Vec<HeaderName>)`, or
`Drop`. Default `None` preserves today's behavior. **Wire output is
unaffected** — redaction applies only to the trace event payload.

This is a `rvoip-sip` change; no dialog-core or transport change.
The trace emission site
(`adapters/dialog_adapter.rs` `on_sip_trace` publish or the equivalent
hook) consults the redactor before publishing the event.

### Verification matrix — third-round additions

24. `cargo test -p rvoip-sip --test stash_lifecycle_integration` —
    stash fields cleared after successful send; not replayed on
    subsequent send; preserved across 401/407 retry.
25. `cargo test -p rvoip-sip --test multipart_body_integration` —
    `OutboundCallBuilder::with_body(multipart_mixed(...))` produces
    a wire INVITE with correctly-formatted multipart structure and
    SDP + ISUP parts; inbound `IncomingCall::body()` parses both
    parts back out.
26. `cargo test -p rvoip-sip --test reliable_provisional_bridge` —
    upstream 18x with `Require: 100rel` bridged to downstream;
    `with_supported_100rel(true)` advertises on outbound INVITE.
27. `cargo test -p rvoip-sip --test topology_hiding_guarantee` —
    every `Via`, `Record-Route`, `Call-ID`, `CSeq` on the inbound
    leg is reported as skipped in
    `HeaderCarryThroughReport.skipped` when `with_headers_from`
    requests `&[]` (carry nothing); none appears on the outbound
    wire trace.
28. `cargo test -p rvoip-sip --test registrar_response_builder` —
    `IncomingRegister::accept_builder()
    .with_expires(3600)
    .with_service_route(routes)
    .with_path_echo()
    .send()` produces a wire 200 OK with all three headers in the
    right slots.
29. `cargo test -p rvoip-sip --test options_timeout` — OPTIONS
    request times out after 32s default; `with_timeout(Duration::from_secs(5))`
    overrides; returned `IncomingResponse` carries inbound
    `Allow`/`Supported`/`Server`.
30. `cargo test -p rvoip-sip --test auto_emit_headers` — Config-level
    `auto_emit_extra_headers` rides session-timer auto-BYE without
    application code; per-call `prepare_auto_emitted_options`
    overrides for one session.
31. `cargo test -p rvoip-sip --test trace_redaction` — redactor
    strips `Authorization` from `SipTraceEvent` payload; wire output
    unaffected; non-redacted traces include the header.

---

## Fourth-round refinements — implementation readiness (2026-05-11)

The earlier rounds locked in API shape, layer respect, and operational
corners. This section closes the **implementation-readiness** gaps: the
error model an implementer reaches for, cancel-safety on `.send().await`,
two concrete builder struct sketches the implementer can copy-shape,
and a developer-facing decision chart so a reader of this document
knows immediately whether to keep using `Config`, adopt builders for
one call site, or migrate end-to-end.

### Error model — integration with the existing `SessionError`

**Verified state.** `rvoip-sip` exposes one error enum:
`SessionError` (`errors.rs:8-85`). Notable variants already present:
`InvalidInput`, `ProtocolError`, `Timeout(String)`,
`MissingCredentialsForInviteAuth`, `InviteAuthRetryExhausted`,
`UnreliableProvisionalsNotSupported`, `DialogError`. `From<Box<dyn
Error>>` flatteners exist; `From<rvoip_auth_core::AuthError>` exists.

**Resolution.** Add three variants to `SessionError` (additive,
non-breaking):

```rust
#[error("header policy violation on {method}: {header} — {reason}")]
HeaderPolicy {
    method: rvoip_sip_core::Method,
    header: rvoip_sip_core::types::headers::HeaderName,
    reason: ViolationReason,
},

#[error("required application header missing for {method}: {names:?}")]
MissingRequiredHeader {
    method: rvoip_sip_core::Method,
    names: Vec<rvoip_sip_core::types::headers::HeaderName>,
},

#[error("send cancelled mid-flight")]
SendCancelled,
```

`HeaderPolicyViolation` already defined in §2 maps cleanly:
`From<HeaderPolicyViolation> for SessionError` produces the
`HeaderPolicy` variant. The `?` operator on every builder setter
(`.with_header(...)?`, `.with_headers_from(...)?`) flows the policy
violation into the surrounding `Result<_, SessionError>` without
custom adapter code.

**`OPTIONS` timeout** (third-round) maps to the existing
`SessionError::Timeout(String)` variant with a structured message
`"OPTIONS to {target} timed out after {duration:?}"`. No new variant
required.

**Auth retry exhaustion** continues to surface as
`InviteAuthRetryExhausted` — builders do not introduce a new error
for this; the existing variant covers the case.

### Cancel-safety on `.send().await`

**Requirement.** B2BUA / call-center applications drop futures
routinely — they race `tokio::select!` between an inbound CANCEL on
leg A and the outbound INVITE `.send().await` on leg B. The builders
must be cancel-safe: if `.send().await` is dropped before completion,
no SIP message has gone on the wire **OR** the state machine is left
in a clean state.

**Resolution.** Two-phase semantics:

1. **Pre-await preparation** is synchronous (no `.await` points). The
   builder validates the policy, runs `HeaderPolicy::validate_outbound`,
   stamps the `Arc<XxxRequestOptions>` onto `SessionState`, and
   queues the `Action::Send*` event. None of these can be cancelled.
2. **Post-await response wait** is the cancel-safe slice. If the
   future is dropped here, the wire message has already gone out and
   the state machine handles the response (or timeout) as usual; the
   caller simply doesn't observe the result. `SessionError::SendCancelled`
   is **not** returned in this case — the future just resolves
   nothing on the dropped side, while the dialog still settles.

**Exception.** For `MessageBuilder` (fire-and-forget per RFC 3428),
`.send().await` completes immediately after queuing — no response
wait, fully cancel-safe regardless of caller behavior.

**Test addition (verification matrix #32):**
`cargo test -p rvoip-sip --test cancel_safety_integration` —
drop the future at various stages and assert no leaked
`SessionState` stash, no leaked transaction, no panic in the state
machine.

### Concrete builder struct sketches (representative two)

The plan lists 12 outbound + 5 response builders in tables; this
subsection materializes two representative implementations so the
shape is unambiguous for the implementer. The remaining builders
follow the same pattern.

#### `OutboundCallBuilder` (UAC INVITE — the most-touched UAC path)

```rust
// src/api/send/outbound_call.rs

pub struct OutboundCallBuilder {
    coordinator: Arc<UnifiedCoordinator>,    // for state-machine dispatch
    from: Option<String>,                    // None → use surface/Config local_uri
    to: String,
    sdp: Option<String>,
    credentials: Option<Credentials>,
    pai_override: PaiOverride,               // Suppress | Use(String) | Default
    contact_uri: Option<String>,             // second-round audit addition
    outbound_proxy_override: ProxyOverride,  // Suppress | Use(String) | Default
    subject: Option<String>,
    from_display: Option<String>,
    precomputed_auth: Option<String>,
    transfer_leg: Option<SessionId>,
    supported_100rel: bool,                  // third-round audit addition
    strictness: BuilderStrictness,
    staged: BuilderHeaderState,              // application-staged TypedHeaders
}

impl OutboundCallBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, from: Option<String>, to: String) -> Self {
        Self {
            coordinator: coord,
            from, to,
            sdp: None,
            credentials: None,
            pai_override: PaiOverride::Default,
            contact_uri: None,
            outbound_proxy_override: ProxyOverride::Default,
            subject: None,
            from_display: None,
            precomputed_auth: None,
            transfer_leg: None,
            supported_100rel: false,
            strictness: BuilderStrictness::Strict,
            staged: BuilderHeaderState::default(),
        }
    }

    // Method-shaped setters
    pub fn with_sdp(mut self, s: impl Into<String>) -> Self { self.sdp = Some(s.into()); self }
    pub fn with_credentials(mut self, c: Credentials) -> Self { self.credentials = Some(c); self }
    pub fn with_pai(mut self, uri: impl Into<String>) -> Self { self.pai_override = PaiOverride::Use(uri.into()); self }
    pub fn without_pai(mut self) -> Self { self.pai_override = PaiOverride::Suppress; self }
    pub fn with_contact_uri(mut self, uri: impl Into<String>) -> Self { self.contact_uri = Some(uri.into()); self }
    pub fn with_outbound_proxy(mut self, uri: impl Into<String>) -> Self {
        self.outbound_proxy_override = ProxyOverride::Use(uri.into()); self
    }
    pub fn without_outbound_proxy(mut self) -> Self {
        self.outbound_proxy_override = ProxyOverride::Suppress; self
    }
    pub fn with_subject(mut self, s: impl Into<String>) -> Self { self.subject = Some(s.into()); self }
    pub fn with_from_display(mut self, s: impl Into<String>) -> Self { self.from_display = Some(s.into()); self }
    pub fn with_precomputed_authorization(mut self, h: impl Into<String>) -> Self {
        self.precomputed_auth = Some(h.into()); self
    }
    pub fn as_transfer_leg(mut self, transferor: &SessionId) -> Self {
        self.transfer_leg = Some(transferor.clone()); self
    }
    pub fn with_supported_100rel(mut self, on: bool) -> Self { self.supported_100rel = on; self }

    // Terminal — runs HeaderPolicy::validate_outbound, builds
    // OutboundCallOptions, stashes on SessionState, emits
    // EventType::MakeCall, returns Result<SessionId>.
    pub async fn send(self) -> Result<SessionId, SessionError> {
        let options = self.build_options()?;
        self.coordinator.dispatch_invite(options).await
    }

    fn build_options(self) -> Result<OutboundCallOptions, SessionError> {
        HeaderPolicy::validate_outbound(Method::Invite, &self.staged.headers)
            .map_err(|missing| SessionError::MissingRequiredHeader {
                method: Method::Invite,
                names: missing.into_iter().map(|m| m.name).collect(),
            })?;
        Ok(OutboundCallOptions { /* … fields → struct fields */ })
    }
}

// SipRequestOptions impl uses the default impls operating on
// `self.staged: BuilderHeaderState`. with_header / with_headers /
// with_raw_header / strip_header / with_headers_from / staged_headers
// / with_strictness all flow through the shared trait default impls.
impl SipRequestOptions for OutboundCallBuilder {
    fn method(&self) -> Method { Method::Invite }
    // (default impls cover the rest, parameterised by &mut self.staged)
}
```

#### `RejectBuilder` (UAS — 4xx/5xx/6xx response authoring)

```rust
// src/api/respond/reject.rs

pub struct RejectBuilder {
    // Either path: respond to an inbound INVITE (IncomingCall) OR
    // to an established session (UnifiedCoordinator::reject).
    target: RejectTarget,
    status: u16,                             // default 486 Busy Here
    reason: Option<String>,
    retry_after: Option<u32>,
    warning: Option<Warning>,
    strictness: BuilderStrictness,
    staged: BuilderHeaderState,
}

enum RejectTarget {
    Incoming(Arc<IncomingCall>),
    Session { coord: Arc<UnifiedCoordinator>, session: SessionId },
}

impl RejectBuilder {
    pub fn with_status(mut self, code: u16) -> Self {
        debug_assert!(code >= 400 && code < 700, "reject code must be 4xx/5xx/6xx");
        self.status = code; self
    }
    pub fn with_reason(mut self, r: impl Into<String>) -> Self { self.reason = Some(r.into()); self }
    pub fn with_retry_after(mut self, secs: u32) -> Self { self.retry_after = Some(secs); self }
    pub fn with_warning(mut self, code: u16, agent: impl Into<String>, text: impl Into<String>) -> Self {
        self.warning = Some(Warning { code, agent: agent.into(), text: text.into() });
        self
    }

    pub async fn send(self) -> Result<(), SessionError> {
        let response = self.build_response()?;
        match self.target {
            RejectTarget::Incoming(call) => call.send_response_with_options(response).await,
            RejectTarget::Session { coord, session } => coord.send_response_with_options(&session, response).await,
        }
    }

    fn build_response(&self) -> Result<rvoip_sip_core::Response, SessionError> {
        // composes Response via SimpleResponseBuilder, applies retry_after / warning,
        // appends self.staged.headers, runs HeaderPolicy::validate_outbound for
        // the response-side check.
        unimplemented!("see implementation in src/api/respond/reject.rs")
    }
}

impl SipRequestOptions for RejectBuilder {
    fn method(&self) -> Method { Method::Invite }  // method is the INVITE being rejected
    // default trait impls cover with_header / with_headers / with_raw_header /
    // strip_header / with_headers_from / staged_headers / with_strictness
}
```

The remaining 10 outbound + 4 response builders follow the same
template — only the method-shaped setters and the terminal `.send()`
return type vary.

### Developer decision chart — which path to use?

A reader scanning this doc should be able to pick their path in 30
seconds. The chart below sits in the crate-level `//!` block in
condensed form.

| If the developer says… | Recommended path | What it looks like |
|---|---|---|
| "I just want to make a call, library handles SIP" | **Pure Config (Path 1)** — unchanged from today | `Config::default().with_local_uri(...)` → `coord.make_call(target)` |
| "I need credentials on outbound calls" | **Pure Config + one builder** — minimal builder use | `coord.invite(from, to).with_credentials(c).send()` |
| "I need to attach one custom X-* header" | **Pure Config + one builder** | `coord.invite(from, to).with_raw_header("X-Foo", "bar")?.send()` |
| "I'm building a B2BUA — must carry headers across legs" | **Mixed (Path 3)** — Config for identity, builder per request, `with_headers_from` for carry-through | `coord.invite(...).with_headers_from(&inbound, &[...])?.send()` |
| "I need to validate before sending; reject bad headers" | **Default** — `BuilderStrictness::Strict` rejects via `Result` | every `.with_header(...)?` already enforces |
| "I need lenient parsing for messy upstream" | **Builder with `with_strictness(Lenient)`** | `coord.invite(...).with_strictness(BuilderStrictness::Lenient).send()` |
| "I need to inspect every inbound header" | **`SipHeaderView` on IncomingCall** | `incoming.header(&HeaderName::Diversion)` |
| "I'm authoring custom 4xx responses with Retry-After" | **`RejectBuilder`** | `incoming.reject_builder().with_status(503).with_retry_after(120).send()` |
| "I'm a registrar and need Service-Route on 200 OK" | **`RegisterResponseBuilder`** (third-round addition) | `incoming.accept_builder().with_service_route(...).send()` |
| "I have an existing app on `make_call_with_pai` — will it still work?" | **Yes, unchanged** — deprecated but functional | call sites compile, emit deprecation warning, behave identically |

This chart drives the crate-level documentation. Implementers writing
the `//!` block can lift it verbatim.

### Phase ownership for second/third/fourth-round additions

To prevent the additive scope from drifting between phases, this
subsection assigns each post-original addition to a specific phase.
Implementers map their PR boundaries to this table.

| Addition | Origin | Phase |
|---|---|---|
| `with_contact_uri` on `OutboundCallBuilder` / `RegisterBuilder` | Second-round | C (Phase C — outbound builders) |
| `with_outbound_proxy(uri)` on all outbound builders | Second-round | C (shared trait helper); `DialogAdapter` consult: Phase C |
| `IncomingResponse` for 1xx provisional | Second-round | A (rides Phase A `IncomingResponse` introduction) |
| `GenericResponseBuilder` | Second-round | D (Phase D — response builders) |
| `bytes = workspace = true` in `infra-common/Cargo.toml` | Second-round | A (first phase touching cross_crate.rs) |
| `Send + Sync` bound on `SipRequestOptions` | Second-round | C (trait definition) |
| Case-insensitive header name canonicalization | Second-round | C (policy module) |
| Auth-retry preserves staged headers | Second-round | C (state machine integration) |
| Trace assertion in carry-through test | Second-round | C (test) |
| CANCEL header-policy decision | Second-round | C (policy table) |
| Phase E three-stage plumbing clarification | Second-round | E (Phase E — already named) |
| Trait-name corrections | Second-round | E (renames `_full` companion methods) |
| `StreamPeer::call_with_auth` to deprecation list | Second-round | C (deprecation pass) |
| Stash lifecycle (set-once / consumed-once / cleared-at-termination) | Third-round | C (state machine integration) |
| Multipart body convenience module | Third-round | C (convenience module under `api::headers`) |
| `with_supported_100rel(bool)` + `IncomingResponse::is_reliable_provisional()` | Third-round | C (outbound builder) + A (inbound view) |
| Topology-hiding guarantee | Third-round | C (assertion + test) |
| Trust-boundary doc patterns | Third-round | C (crate-level `//!` block) |
| `RegisterResponseBuilder` (Service-Route, Path echo, P-Associated-URI) | Third-round | D (Phase D — response builders) |
| `OptionsBuilder::with_timeout` + Timer F default | Third-round | C (outbound builder) |
| Auto-emit message header injection (3 patterns) | Third-round | C (proactive stash) + E (CallHandler hook) + B (Config field) |
| Re-INVITE glare options preservation | Third-round | C (state machine integration) |
| `NotifyBuilder::for_subscription(SubscriptionId)` | Third-round | C (outbound builder; possible Phase B if dialog-core extension required) |
| Body convenience helpers (`sdp`, `dtmf_relay`, `pidf_xml`, `simple_message_summary`, `isup`) | Third-round | C (convenience module) |
| `Config.trace_redaction` | Third-round | A (Config field; activates with traces) |
| `SessionError::{HeaderPolicy, MissingRequiredHeader, SendCancelled}` | Fourth-round | C (error model) |
| Cancel-safety semantics | Fourth-round | C (builder `.send().await` contract) |
| Concrete builder struct sketches | Fourth-round | C (implementation reference) |
| Developer decision chart | Fourth-round | A (crate-level `//!` block, first phase to ship) |

### Acceptance criteria — what "done" looks like

The implementer signs off on each phase by the following objective
checks (in addition to the verification matrix #1-#32):

**Phase A (Inbound inspection)**
- [ ] `IncomingCall.headers: HashMap` populated from the parsed INVITE.
- [ ] `IncomingCall::header(&HeaderName::Diversion)` returns typed access for inbound INVITE carrying Diversion.
- [ ] `Event::CallProgressDetailed(IncomingResponse)` fires on every 1xx, not just finals.
- [ ] `Config.trace_redaction` is a no-op when `None` (default).
- [ ] `infra-common/Cargo.toml` includes `bytes = { workspace = true }`.
- [ ] Crate-level `//!` block includes the developer decision chart.

**Phase B (Dialog-core options extension)**
- [ ] Every `*RequestOptions` struct derives `Default`.
- [ ] `RegisterRequestOptions` derives `Default` and has `extra_headers`.
- [ ] `send_options_out_of_dialog_with_options` exists and is layer-parallel to `send_message_out_of_dialog`.
- [ ] `cargo test -p rvoip-sip-dialog` is green (no regressions).

**Phase C (Send-side builders)**
- [ ] Every builder is `Send + Sync + Sized`.
- [ ] `BuilderStrictness::Strict` is default.
- [ ] `SessionError::HeaderPolicy` and `MissingRequiredHeader` exist.
- [ ] Stash fields are consumed-once and cleared at session termination.
- [ ] Cancel-safety integration test (#32) passes.
- [ ] Every concrete builder has a struct + doc-test per the §"Concrete builder struct sketches" template.

**Phase D (Response builders)**
- [ ] `AcceptBuilder`, `RejectBuilder`, `RedirectBuilder`, `ProvisionalBuilder`, `AuthChallengeBuilder`, `GenericResponseBuilder`, `RegisterResponseBuilder` all implement `SipRequestOptions`.
- [ ] `DialogAdapter::send_response_with_options(session_id, Response)` exists.

**Phase E (In-dialog request surface)**
- [ ] `CallHandler::on_transfer_request_full`, `on_refer_notify_full`, `on_notify_full`, `on_info_full`, `on_message_full`, `on_options_full`, `on_update_full` all exist with default impls.
- [ ] Cross-crate variants `InfoReceived`, `MessageReceived`, `OptionsReceived` (and `UpdateReceived` xor enriched `ReinviteReceived`) all carry `raw_request: Arc<Bytes>`.
- [ ] `event_hub.rs::convert_session_coordination_to_cross_crate` bridges all five inbound mid-dialog methods (REFER + NOTIFY + INFO + MESSAGE + OPTIONS + UPDATE).
- [ ] Inbound OPTIONS reaches `CallHandler::on_options_full` end-to-end (today it is dropped at dialog-core).

When all five checklists pass, the design is shipped end-to-end and
the developer-facing surface promised at the top of this document is
real.

---

## Design decisions (recorded from review)

- **Dialog-core extends additively.** New `*RequestOptions` structs +
  `send_*_with_options` methods on `UnifiedDialogApi`. Existing methods
  stay and delegate to the new options forms with empty `extra_headers`.
  Layer-correct: dialog-core remains authoritative over dialog/transaction
  headers; application headers ride alongside.
- **`with_header` returns `Result`.** Forced acknowledgement of policy at
  the call site; `?` chains cleanly. No silent drops, no debug-only
  checks. The `Err` value names the dedicated setter where one applies.
- **Full RFC-3261 + extensions in phase 1.** INVITE, re-INVITE, REGISTER,
  REFER, BYE, CANCEL, NOTIFY, SUBSCRIBE, INFO, MESSAGE, OPTIONS, UPDATE.
  Comprehensive for SBC / B2BUA / call-center use cases.
- **`AuthChallengeBuilder` is in scope.** Wraps sip-core's existing
  `www_authenticate_digest` / `bearer` helpers for typed 401/407
  authoring — needed by registrars and B2BUA auth-relay code.
- **Compile-time prevention rejected.** Wrapping `TypedHeader` in a
  newtype that excludes stack-managed variants is impractical given
  `TypedHeader::Other(_)` carry-through. Runtime policy with `Result`
  return is the pragmatic choice.

## What this design deliberately does NOT do

- Add new SIP methods beyond those listed (MESSAGE, OPTIONS, UPDATE are
  in; PUBLISH stays out). The trait shape makes adding more trivial
  later.
- Migrate the in-tree examples and tests onto the builder API.
  Follow-up sweep PR once the builders are stable.
- Remove any deprecated method. Breaking-change PR, later release.
- Touch `dialog-core`'s dialog state machine, route-set logic,
  transaction core, or CSeq management. The dialog-core changes are
  strictly additive options structs.
- Touch `rvoip-sip-core` or `rvoip-sip-transport`.
- Expose direct `Request` *construction* (only consumption via
  `SipHeaderView::raw_request`). Authors compose via builders; raw
  request authorship stays inside `rvoip-sip-core` for power users.
- Expose `ACK` (2xx) or `PRACK` builders. State machine emits these
  automatically; no application headers need to ride on them.
