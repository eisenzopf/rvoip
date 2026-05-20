# Gateway-grade SIP header API for `rvoip-sip`

**Status:** approved. Single-pass implementation plan. Supersedes
`SIP_API_DESIGN.md` (which captured the design across four audit rounds
and is preserved for historical reference).

This document is the canonical spec. Where prior wording conflicts,
this document wins.

---

## 1. Why this exists

`rvoip-sip` claims to serve "softphones, test clients, IVRs, B2BUA
legs, routing servers, and PBX/SBC interop tools" but its public API is
shaped for endpoint use cases only. Gateways, B2BUAs, SBCs, and
call-center applications need the opposite of what endpoints want: full
inspection of inbound SIP, full authorship of outbound SIP, and easy
composition so an inbound request can be transformed and re-sent on the
other leg — without breaking RFC 3261 correctness or layer boundaries.

The four current public surfaces — `UnifiedCoordinator`, `Endpoint`,
`StreamPeer` (`PeerControl`), and `CallbackPeer` (`CallbackPeerControl`)
— must all expose the same shape consistently.

Today's blockers:

- **Inbound dead-ends:** `IncomingCall.headers: HashMap<String,String>`
  is never populated (`src/api/incoming.rs:80`). `IncomingCallInfo`
  carries only `from`, `to`, `sdp`, `p_asserted_identity`. Inbound
  REFER / NOTIFY / INFO reach the application only through pre-decoded
  events with fixed fields; original headers are dropped. Inbound
  OPTIONS is not routed at all.
- **Outbound explosion:** every public method is a fixed-parameter
  variant — `make_call_with_auth`, `make_call_with_pai`,
  `make_call_with_headers`, `send_refer_with_replaces`,
  `send_notify(event, body, state)`, etc. Compound combinations
  (auth + headers + PAI, NOTIFY with custom headers, REFER with
  Referred-By) require explosive `_with_X_and_Y_and_Z` overloads.
- **Response side empty:** `reject_call`, `redirect_call`,
  `accept_call_with_sdp` take fixed parameters with no path to attach
  custom headers (`Retry-After`, `Warning`, vendor routing hints,
  401/407 challenges).
- **Layer pass-through gap:** dialog-core's public API accepts
  `Vec<TypedHeader>` only for INVITE
  (`make_call_with_extra_headers_for_session`). Every other request
  method — REGISTER, REFER, NOTIFY, INFO, BYE, UPDATE, SUBSCRIBE,
  MESSAGE, CANCEL — takes fixed parameters. OPTIONS doesn't exist at
  all in dialog-core.
- **Safety gap:** nothing prevents an application from attaching
  `Call-ID`, `CSeq`, `Via`, `Max-Forwards`, or `From` via the raw
  `extra_headers` channel. Each desyncs the dialog and produces
  non-RFC wire output.

The goal is a uniform builder-shaped request/response API where
"inspect, add, modify, delete SIP fields" is the same shape across
every method and every direction, with first-class composition for
B2BUA carry-through — and guardrails so applications cannot
accidentally desync dialogs.

---

## 2. Layer architecture

| Crate | Owns | Builds messages? | Holds dialog state? |
|---|---|---|---|
| `rvoip-sip-transport` | UDP/TCP/TLS/WS sockets, `TransportEvent`, `SipTraceEvent` | No (production); only test code | No |
| `rvoip-sip-core` | `Request`/`Response`, `TypedHeader`, `HeaderName`, parser, `SimpleRequestBuilder` / `SimpleResponseBuilder`, RFC validators | Yes — raw, generic | No |
| `rvoip-sip-dialog` | Dialog state, CSeq, Route-Set, in-dialog request authorship via `transaction/utils/{request,response}_builders.rs` + `transaction/dialog/request_builder_from_dialog_template` | Yes — dialog-bound | Yes |
| `rvoip-sip` | Session lifecycle, state machine, four public API surfaces, `DialogAdapter` | No — delegates | No |
| `infra-common` | `GlobalEventCoordinator`, `EventBus`, cross-crate event enums (`DialogToSessionEvent` / `SessionToDialogEvent` at `events/cross_crate.rs`) | No | No |

**Hard layer rules this design honors:**

1. `rvoip-sip-transport` is never modified.
2. `rvoip-sip-core` is never modified (no new `TypedHeader` variants;
   missing types like `Diversion` ride on `TypedHeader::Other`).
3. `rvoip-sip-dialog`'s dialog state machine, CSeq counter
   (`DialogImpl::local_cseq` at `src/dialog/dialog_impl.rs:47`),
   Route-Set (`route_set: Vec<Uri>` line 56), and transaction core are
   not modified. Dialog-core gains only additive options structs and
   `*_with_options` methods that append application headers **after**
   stack-managed headers are stamped.
4. `infra-common` has no `rvoip-sip-core` dependency. Cross-crate event
   payloads cannot carry `Arc<rvoip_sip_core::Request>`. The bus
   carries `Arc<bytes::Bytes>` and the receiving side reconstructs the
   typed Request.

---

## 3. Public API surface

### 3.1 Inbound: `SipHeaderView` — uniform header inspection

Trait implemented by every wrapper for a received SIP message. Lives in
`src/api/headers/view.rs`.

```rust
pub trait SipHeaderView {
    /// First header matching `name`, typed when sip-core has a variant
    /// for it. Case-insensitive per RFC 3261 §7.3.1.
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader>;

    /// Every header matching `name`, in wire order. Returns empty when
    /// none. Boxed for object safety.
    fn headers_named<'a>(&'a self, name: &HeaderName)
        -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a>;

    /// All headers in wire order.
    fn headers<'a>(&'a self)
        -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a>;

    /// Header value as a string via `TypedHeader::Display`. For
    /// `TypedHeader::Other`, this reproduces the inbound wire value.
    fn header_str(&self, name: &HeaderName) -> Option<String> {
        self.header(name).map(|h| h.to_string())
    }

    /// All header names present, deduped, in first-seen order.
    fn header_names(&self) -> Vec<HeaderName>;
}
```

**Raw message access is via concrete inherent accessors, not the
trait.** Because `IncomingCall` / `IncomingRequest` store
`Option<Arc<Request>>` and `IncomingResponse` stores
`Option<Arc<Response>>`, the trait cannot return a single typed
reference object-safely. Each concrete type instead exposes:

```rust
impl IncomingCall {
    /// Underlying parsed Request. `None` when synthesized or under
    /// the lean-mode feature flag (§13.3).
    pub fn raw_request(&self) -> Option<&Arc<rvoip_sip_core::Request>>;

    /// Zero-alloc header iteration — preferred over the boxed
    /// trait method on hot paths.
    pub fn headers_named_iter<'a>(&'a self, name: &HeaderName)
        -> impl Iterator<Item = &'a TypedHeader> + 'a;
}

impl IncomingRequest {
    pub fn raw_request(&self) -> Option<&Arc<rvoip_sip_core::Request>>;
    pub fn headers_named_iter<'a>(&'a self, name: &HeaderName)
        -> impl Iterator<Item = &'a TypedHeader> + 'a;
}

impl IncomingResponse {
    pub fn raw_response(&self) -> Option<&Arc<rvoip_sip_core::Response>>;
    pub fn headers_named_iter<'a>(&'a self, name: &HeaderName)
        -> impl Iterator<Item = &'a TypedHeader> + 'a;
}

impl IncomingRegister {
    pub fn raw_request(&self) -> Option<&Arc<rvoip_sip_core::Request>>;
    pub fn headers_named_iter<'a>(&'a self, name: &HeaderName)
        -> impl Iterator<Item = &'a TypedHeader> + 'a;
}
```

**Performance note:** the trait's `headers_named` returns a boxed
iterator for object-safety. Concrete types' `headers_named_iter()`
returns an unboxed iterator with no allocation. Hot paths use the
inherent method; generic code (`with_headers_from<S: SipHeaderView>`)
uses the trait. See §13 for performance budget.

**Implementors:**

| Type | Wraps |
|---|---|
| `IncomingCall` | Inbound INVITE — applies for every received call |
| `IncomingRequest` | In-dialog received REFER / NOTIFY / INFO / OPTIONS / UPDATE / MESSAGE |
| `IncomingResponse` | Every inbound response (1xx provisional, 2xx success, 3xx redirect, 4xx-6xx final). Needed for B2BUA response carry-through (§11.5 propagates Allow/Supported/Server from upstream 200 OK to downstream 200 OK), redirect handling, and final-failure inspection (Retry-After, Warning, Reason). |
| `IncomingRegister` | Inbound REGISTER on registrar surfaces |

The existing `IncomingCall.headers: HashMap<String,String>` field is
populated from the parsed INVITE (the empty-default bug is fixed) and
marked deprecated with a doc-note pointing at the trait. Removed in a
future breaking-change release.

### 3.2 Outbound: `SipRequestOptions` — the shared builder trait

```rust
pub trait SipRequestOptions: Sized + Send + Sync {
    /// The SIP method this builder will emit. Drives HeaderPolicy.
    fn method(&self) -> Method;

    /// Append one header. Errors for stack-managed or method-shaped
    /// names; the Err names the dedicated setter when one exists.
    fn with_header(self, header: TypedHeader)
        -> Result<Self, HeaderPolicyViolation>;

    /// Batch form. Fails fast on the first violation.
    fn with_headers(self, headers: Vec<TypedHeader>)
        -> Result<Self, HeaderPolicyViolation>;

    /// Parse `value` as the body of header `name` and append a
    /// `TypedHeader::Other`. Same policy check as `with_header`.
    fn with_raw_header(
        self,
        name: impl Into<HeaderName>,
        value: impl Into<String>,
    ) -> Result<Self, HeaderPolicyViolation>;

    /// Drop any header named `name` that was added earlier in the
    /// builder chain (or via carry-through). Infallible.
    fn strip_header(self, name: &HeaderName) -> Self;

    /// B2BUA carry-through. Copy the listed headers from `source`.
    /// Stack-managed names are filtered automatically and reported.
    fn with_headers_from<S: SipHeaderView>(
        self,
        source: &S,
        names: &[HeaderName],
    ) -> Result<(Self, HeaderCarryThroughReport), HeaderPolicyViolation>;

    /// Inspect headers staged so far.
    fn staged_headers(&self) -> &[TypedHeader];

    /// Validation strictness. Defaults to Config.default_builder_strictness
    /// (Strict).
    fn with_strictness(self, mode: BuilderStrictness) -> Self;
}

/// Shared per-builder state for the default trait implementations.
/// Every concrete builder embeds this and exposes it to the trait
/// defaults via the only mandatory non-`method()` impl.
#[derive(Default, Debug, Clone)]
pub struct BuilderHeaderState {
    pub headers: Vec<TypedHeader>,
    pub strictness: BuilderStrictness,
}

pub struct HeaderPolicyViolation {
    pub method: Method,
    pub header: HeaderName,
    pub reason: ViolationReason,
}

#[non_exhaustive]
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

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum BuilderStrictness {
    /// Default. Any HeaderPolicyViolation is a hard `Err`. StackManaged
    /// is always a hard `Err` (it would desync the dialog).
    #[default]
    Strict,
    /// MethodShaped and WrongMethod violations downgrade to a
    /// `tracing::warn!` and the offending header is silently dropped.
    /// StackManaged remains a hard `Err`.
    Lenient,
}
```

The `?` operator chains cleanly:

```rust
coord.invite(from, to)
    .with_credentials(creds)
    .with_header(typed_pai)?
    .with_raw_header("X-Customer-ID", customer_id)?
    .send().await?;
```

Default implementations of `with_headers`, `with_raw_header`,
`strip_header`, `with_headers_from`, and `with_strictness` live on the
trait and operate on a shared `BuilderHeaderState` field. Each
concrete builder only implements `method()` and `with_header()`.

### 3.3 Outbound builders

Twelve primary builders entered from `UnifiedCoordinator` (and
proxied through the other surfaces — see §4), plus two refresh
builders entered from their respective handles:

| Builder | Coordinator entry | `.send()` returns | Method-specific setters |
|---|---|---|---|
| `OutboundCallBuilder` | `coord.invite(from, to)` | `Result<SessionId>` | `with_sdp`, `with_credentials`, `with_pai` / `without_pai`, `with_contact_uri`, `with_outbound_proxy` / `without_outbound_proxy`, `with_subject`, `with_from_display`, `with_precomputed_authorization`, `as_transfer_leg(&SessionId)`, `with_supported_100rel(bool)` |
| `ReInviteBuilder` | `coord.reinvite(&session)` | `Result<()>` | `with_sdp`, `as_session_timer_refresh`, `with_precomputed_authorization` |
| `RegisterBuilder` | `coord.register(registrar, user, pw)` | `Result<RegistrationHandle>` | `with_expires`, `with_from_uri`, `with_contact_uri`, `with_outbound_proxy` / `without_outbound_proxy`, `with_path(uri)` (RFC 3327), `with_q_value(f32)`, `with_sip_instance(urn)`, `with_reg_id(u32)`, `with_precomputed_authorization` |
| `RegisterRefreshBuilder` | `registration.refresh()` | `Result<()>` | `with_expires`, `with_credentials` |
| `ReferBuilder` | `coord.refer(&session, refer_to)` | `Result<()>` | `with_replaces(value)`, `with_referred_by(uri)` (RFC 3892), `with_target_dialog(&IncomingRequest)` (RFC 4538) |
| `ByeBuilder` | `coord.bye(&session)` | `Result<()>` | `with_reason(SipReason)` |
| `CancelBuilder` | `coord.cancel(&session)` | `Result<()>` | `with_reason(SipReason)` |
| `NotifyBuilder` | `coord.notify(&session, event_package)` | `Result<()>` | `with_body`, `with_content_type`, `with_subscription_state`, `with_retry_after(u32)`, `for_subscription(SubscriptionId)` |
| `SubscribeBuilder` | `coord.subscribe(target, event_package)` | `Result<SubscriptionHandle>` | `with_from_uri`, `with_contact_uri`, `with_expires`, `with_accept(content_type)`, `with_credentials` |
| `SubscribeRefreshBuilder` | `subscription.refresh()` | `Result<()>` | `with_expires`, `with_credentials` |
| `InfoBuilder` | `coord.info(&session, content_type)` | `Result<()>` | `with_body` |
| `UpdateBuilder` | `coord.update(&session)` | `Result<()>` | `with_sdp`, `as_session_timer_refresh` |
| `MessageBuilder` | `coord.message(target)` | `Result<()>` | `with_body`, `with_content_type`, `with_credentials`, `with_from_uri` |
| `OptionsBuilder` | `coord.options(target)` | `Result<IncomingResponse>` | `with_from_uri`, `with_accept`, `with_credentials`, `with_timeout(Duration)` |

**ACK (2xx) and PRACK** are emitted by the state machine automatically
per RFC 3261 §13.2.2.4 and RFC 3262. No public builder; no application
header injection.

**`SessionHandle` shorthand for in-dialog builders.** Once a call is
established (post-`invite().send().await` or post-`accept().await`),
the in-dialog builders — `reinvite`, `refer`, `bye`, `cancel`,
`notify`, `info`, `update` — are also reachable directly on the
returned `SessionHandle`:

```rust
session.bye().send().await?;
session.refer("sip:bob@example").with_replaces(rep).send().await?;
session.info("application/dtmf-relay").with_body(dtmf).send().await?;
```

Equivalent to `coord.bye(session.id()).send()` etc., but doesn't
require reaching back through the coordinator. This is the canonical
shape for application code that already holds a `SessionHandle`,
which is the common case after a call is up. The coordinator-keyed
entries (`coord.<verb>(&session_id, …)`) remain available for code
that holds only a `CallId` / `SessionId`. The out-of-dialog
builders (`subscribe`, `message`, `options`) and call-creation
builders (`invite`, `register`) stay on the coordinator / surface
types — they're not session-bound.

Every builder consumes `self` per setter (chaining compiles to one
struct literal after monomorphization). Every builder is `Send + Sync`
(applications cross `.await` and may `tokio::spawn` per-leg work).

**Setter semantics — quick reference for the less-obvious ones:**

| Setter | Effect |
|---|---|
| `as_session_timer_refresh()` (ReInvite, Update) | Adds RFC 4028 `Session-Expires` + `Min-SE` headers and `Supported: timer` for session-timer keepalive refresh |
| `as_transfer_leg(&SessionId)` (OutboundCall) | Marks this INVITE as the B leg of a `transferor`-initiated attended transfer; used for media-bridging and REFER-completion NOTIFY |
| `with_target_dialog(&IncomingRequest)` (Refer) | Stamps the RFC 4538 `Target-Dialog` header from the request's dialog identifiers — used by attended transfer to identify which dialog the transferee replaces |
| `with_path(uri)` (Register) | Stamps the RFC 3327 `Path` header so the registrar records a routing waypoint (typical for SBC-fronted REGISTER) |
| `with_q_value(f32)` (Register) | Sets the Contact `q` parameter for forking priority (RFC 3261 §10.2.1.2); typical values 0.0–1.0 |
| `with_sip_instance(urn)` (Register) | Sets the RFC 5626 `+sip.instance` Contact parameter for outbound flows / multi-device registration |
| `with_reg_id(u32)` (Register) | Sets the RFC 5626 `reg-id` Contact parameter pairing with `sip.instance` for managed flows |
| `with_precomputed_authorization(s)` (OutboundCall, ReInvite, Register) | Bypasses 401-driven digest computation by attaching a pre-computed `Authorization` header string. Used when the caller has already run challenge / hash via an external auth service |
| `for_subscription(SubscriptionId)` (Notify) | Targets a specific subscription on a multi-subscription dialog (RFC 6665 §4.5.2). When omitted, the single-subscription default applies. See §12.5 for the implementer verification flag |
| `with_supported_100rel(bool)` (OutboundCall) | Advertises RFC 3262 reliable provisional support on the outbound INVITE. Default `false` |
| `with_require_100rel(bool)` (Provisional) | Stamps `Require: 100rel` + `RSeq` on the outbound 1xx; the state machine arms the PRACK-await timer per RFC 3262 |

### 3.4 Response builders

Seven builders for response authoring. All implement `SipRequestOptions`.

| Builder | Entry | `.send()` returns | Method-specific setters |
|---|---|---|---|
| `AcceptBuilder` | `incoming.accept_builder()` or `coord.accept(&session)` | `Result<SessionHandle>` | `with_sdp` |
| `RejectBuilder` | `incoming.reject_builder()` or `coord.reject(&session)` | `Result<()>` | `with_status(u16)`, `with_reason`, `with_retry_after`, `with_warning(code, agent, text)` |
| `RedirectBuilder` | `incoming.redirect_builder()` or `coord.redirect(&session)` | `Result<()>` | `with_status(u16)` (default 302), `with_contact(uri)`, `with_contacts(Vec)` |
| `ProvisionalBuilder` | `incoming.send_provisional_builder(code)` | `Result<()>` | `with_sdp`, `with_require_100rel(bool)` |
| `AuthChallengeBuilder` | `incoming.challenge_builder(scheme)` | `Result<()>` | `with_realm`, `with_nonce`, `with_algorithm`, `with_qop`, `with_stale`, `with_opaque`, `as_proxy_challenge(bool)` |
| `GenericResponseBuilder` | `incoming.respond_builder(status)` | `Result<()>` | `with_reason`. Status must be 3xx/4xx/5xx/6xx (1xx via `ProvisionalBuilder`, 2xx INVITE via `AcceptBuilder`). |
| `RegisterResponseBuilder` | `incoming_register.accept_builder()` | `Result<()>` | `with_expires(u32)`, `with_min_expires(u32)`, `with_service_route(Vec<Uri>)` (RFC 3608), `with_path_echo()` (RFC 3327), `with_associated_uri(Vec<Uri>)` (RFC 3455), `with_contact_from_binding(binding)` |

Each builder composes a `Response` in `rvoip-sip` via
`SimpleResponseBuilder` from sip-core, runs the response-side header
policy check, and hands the `Response` to
`UnifiedDialogApi::send_response` (dialog-core's existing
`unified.rs:784-790` entry point that takes a fully-built `Response`).
No new dialog-core entry point is required for responses.

`100 Trying` is emitted by the transaction layer per RFC 3261 §17.2.1
and is not authoring-exposed. `200 OK` for non-INVITE in-dialog
methods (BYE, INFO, UPDATE, MESSAGE, OPTIONS) is synthesized by the
transaction layer unless the application calls `respond_builder` first
to override.

`AuthChallengeBuilder` is **UAS-only**. It is reachable from
`IncomingCall::challenge_builder(scheme)`,
`IncomingRequest::challenge_builder(scheme)`,
`IncomingRegister::challenge_builder(scheme)`, and
`UnifiedCoordinator::challenge(&session, scheme)`. It is **not**
exposed as a top-level entry on `Endpoint`, `StreamPeer`, or
`CallbackPeer` because those surfaces are UAC-shaped; their callback
handlers receive the `Incoming*` wrapper and call the builder from
there.

**Response builder `method()` semantics.** Each response builder
implements `SipRequestOptions::method()` and returns the SIP method
of the *request being responded to*, so `HeaderPolicy::classify`
uses the right column of the matrix in §5.1. Construction sites
already know the request's method:

| Builder | `method()` returns |
|---|---|
| `AcceptBuilder` / `RejectBuilder` / `RedirectBuilder` / `ProvisionalBuilder` / `AuthChallengeBuilder` / `GenericResponseBuilder` from `IncomingCall` | `Method::Invite` |
| Same builders from `IncomingRequest` | The method carried in the IncomingRequest (REFER / NOTIFY / INFO / UPDATE / MESSAGE / OPTIONS) |
| `RegisterResponseBuilder` / `AuthChallengeBuilder` from `IncomingRegister` | `Method::Register` |

### 3.5 Referenced types

The setter signatures above reference the following types. Existing
sip-core types (`HeaderName`, `Method`, `Uri`, `TypedHeader`, `Bytes`)
and existing rvoip-sip types (`SessionId`, `SessionHandle`,
`Credentials`, `SubscriptionHandle`, `RegistrationHandle`) are reused
unchanged.

**Implementer verification — `SubscriptionId`:** referenced by
`NotifyBuilder::for_subscription(SubscriptionId)` and
`NotifyRequestOptions.subscription_id`. If the type already exists
in rvoip-sip or dialog-core's subscription manager, reuse it. If
not, introduce a thin newtype (e.g., `pub struct SubscriptionId(pub String)`)
in `rvoip-sip` and thread it through. See §12.5 for the dialog-core
wiring decision this depends on.

New types introduced by this design:

```rust
/// RFC 3326 `Reason` header for BYE / CANCEL.
#[derive(Debug, Clone)]
pub struct SipReason {
    pub protocol: String,    // typically "SIP" or "Q.850"
    pub cause: Option<u16>,
    pub text: Option<String>,
}

/// RFC 3261 §20.43 `Warning` header for response authoring.
/// `RejectBuilder::with_warning(code, agent, text)` constructs this
/// internally from positional args; applications can also pass a
/// pre-built `Warning` via `with_warning_struct(Warning)` when
/// composing multiple warnings.
#[derive(Debug, Clone)]
pub struct Warning {
    pub code: u16,           // 3-digit warning code
    pub agent: String,       // agent host or pseudonym
    pub text: String,        // warning text
}

/// Auth scheme for AuthChallengeBuilder.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AuthScheme {
    Digest,
    Bearer,                  // RFC 8898
}

/// Tri-state override for builders that can either inherit from
/// Config, explicitly suppress, or override per-request. Public
/// because `OutboundCallOptions` (§7.1) exposes it on its surface.
/// Applications use `with_pai(uri)` / `without_pai()` setters rather
/// than constructing these variants directly.
#[derive(Default, Debug, Clone)]
pub enum PaiOverride {
    #[default] Default,      // use Config.pai_uri
    Suppress,                // emit no PAI even if Config has one
    Use(String),             // override Config.pai_uri
}

#[derive(Default, Debug, Clone)]
pub enum ProxyOverride {
    #[default] Default,      // use Config.outbound_proxy_uri
    Suppress,
    Use(String),
}
```

### 3.6 Convenience modules

Two public modules ship typed constructors for the common cases where
`with_raw_header` / hand-rolled `TypedHeader::Other` is too verbose.

**`rvoip_sip::api::headers::convenience`** — headers without a
`TypedHeader` variant in sip-core today. Each returns a
`TypedHeader::Other(HeaderName::Other(canonical_name), value)`:

```rust
pub fn diversion(value: impl Into<String>) -> TypedHeader;
pub fn history_info(value: impl Into<String>) -> TypedHeader;
pub fn privacy(value: impl Into<String>) -> TypedHeader;
pub fn replaces(value: impl Into<String>) -> TypedHeader;
pub fn target_dialog(value: impl Into<String>) -> TypedHeader;       // RFC 4538
pub fn session_expires(value: impl Into<String>) -> TypedHeader;     // RFC 4028
pub fn min_se(seconds: u32) -> TypedHeader;                          // RFC 4028
pub fn p_charging_vector(value: impl Into<String>) -> TypedHeader;   // RFC 7315
pub fn p_called_party_id(value: impl Into<String>) -> TypedHeader;   // RFC 3455

/// RFC 5621 multipart/mixed body construction. Returns
/// (Content-Type with boundary, body bytes) to plug into
/// `with_content_type(s)` + `with_body(b)`.
pub fn multipart_mixed(parts: &[(&str, Bytes)]) -> (String, Bytes);

/// Inverse of `multipart_mixed` — for inbound multipart bodies.
pub fn multipart_parse(content_type: &str, body: &Bytes)
    -> Result<Vec<(String, Bytes)>, MultipartParseError>;

/// Errors from `multipart_parse`. Public, `#[non_exhaustive]`.
#[non_exhaustive]
#[derive(Debug)]
pub enum MultipartParseError {
    MissingBoundary,
    MalformedHeader(String),
    UnterminatedPart,
    Io(std::io::Error),
}
```

`HeaderPolicy::classify` canonicalizes the header name before
classification, so if sip-core later promotes any of these to a
typed `HeaderName` variant the application code keeps working
unchanged.

**`rvoip_sip::api::bodies`** — body factories for common SIP body
types. Each returns `(content_type, body_bytes)`:

```rust
pub fn sdp(s: impl Into<String>) -> (String, Bytes);
    // "application/sdp"
pub fn dtmf_relay(signal: char, duration_ms: u32) -> (String, Bytes);
    // "application/dtmf-relay"
pub fn pidf_xml(presence: &Presence) -> (String, Bytes);
    // "application/pidf+xml" — RFC 3863
pub fn simple_message_summary(/* ... */) -> (String, Bytes);
    // "application/simple-message-summary" — RFC 3842
pub fn isup_l3(bytes: impl Into<Bytes>) -> (String, Bytes);
    // "application/isup" — RFC 3204
```

Both modules are entirely additive — pure helper functions, no new
types, no policy interaction. Tests that exercise them are
`multipart_body_integration` (test #24) and embedded doctest
examples in each function's docstring.

### 3.7 Forward-compatibility hygiene

Every public enum this design introduces is marked `#[non_exhaustive]`
so future variants can be added without breaking downstream
exhaustive matches:

- `ViolationReason` (§3.2)
- `BuilderStrictness` (§3.2)
- `HeaderRole` (§5)
- `AuthScheme` (§3.5)
- `PaiOverride` (§3.5)
- `ProxyOverride` (§3.5)
- `RedactionDecision` (§12.4)

The new `SessionError` variants (`HeaderPolicy`, `MissingRequiredHeader`,
`Conflict`) inherit whatever marking `SessionError` already has; if
the enum is not `#[non_exhaustive]` today, Phase C adds it (additive
on a public error enum is safe under the workspace's existing
`deprecated = "allow"` lint level).

The `SipRequestOptions` trait is **not** marked extensible —
deliberately. Adding a method to this trait without a default impl
breaks every concrete builder. New behavior should land as new
*setters* on individual builders, not new trait methods. If a
trait-wide capability is genuinely needed in the future, it ships
behind a default impl on the trait itself (e.g.,
`with_strictness` shipped this way in v1 of the trait).

---

## 4. Surfaces — one pattern, four entry points

The four public surfaces are concrete structs with inherent impls
(not traits): `UnifiedCoordinator`, `Endpoint`, `StreamPeer` /
`PeerControl`, `CallbackPeer` / `CallbackPeerControl`. The
slash-separated pairs name the same surface from two angles:
`StreamPeer` and `CallbackPeer` are the user-facing handle types,
`PeerControl` and `CallbackPeerControl` are inherent-impl trait-like
bundles of methods reachable via deref or as inherent calls on the
peer struct. `Endpoint` is a single struct; `EndpointControl`
appears in the deprecation table as the legacy trait of method
names some external code may use (kept as deprecated aliases for
the new inherent builders). Implementer verification: confirm
whether `EndpointControl` is a trait or an alias, and whether
`PeerControl` / `CallbackPeerControl` are traits or inherent-impl
bundles; treat the design's "deprecate the legacy methods on each"
prescription identically either way.

To avoid 12 × 4 = 48 hand-written wrapper types, surfaces share a
generic adapter:

```rust
pub trait Surface: Send + Sync + 'static {
    /// What `.send()` returns on this surface (SessionId vs SessionHandle).
    type SessionRef: Send + Sync;

    /// Convert the coordinator-level SessionId to the surface's ref.
    fn into_session_ref(&self, id: SessionId) -> Self::SessionRef;

    /// Pre-populate `from` from the surface's local URI when caller
    /// passes `None`. Endpoint additionally runs `resolve_target` on
    /// bare extensions.
    fn resolve_from(&self, from: Option<String>) -> String;
    fn resolve_target(&self, target: &str) -> String;
}

pub struct SurfaceBuilder<B: SipRequestOptions, S: Surface> {
    inner: B,
    surface: Arc<S>,
}

impl<B: SipRequestOptions, S: Surface> SipRequestOptions
    for SurfaceBuilder<B, S>
{
    // Forwards every method to `self.inner` and re-wraps Self.
}
```

`UnifiedCoordinator` exposes the bare builders (returns `SessionId`).
The other three surfaces expose `SurfaceBuilder<B, ThisSurface>`,
returning `SessionHandle`. New builders extend automatically — adding
PUBLISH later is one new entry point on each surface plus one new
inner builder type.

Per-surface entry-point shapes:

| Surface | Entry shape | Pre-fills |
|---|---|---|
| `UnifiedCoordinator` | `coord.invite(from, to)` | nothing |
| `Endpoint` | `endpoint.invite(to)` | `from = endpoint.local_uri`; `resolve_target` runs on bare extensions |
| `StreamPeer` / `PeerControl` | `peer.invite(to)` | `from = peer.local_uri` |
| `CallbackPeer` / `CallbackPeerControl` | `peer.invite(to)` | `from = peer.local_uri` |

In-dialog builders (REFER, NOTIFY, INFO, BYE, CANCEL, UPDATE,
re-INVITE) are reached via `SessionHandle::<verb>(...)` directly —
see the "`SessionHandle` shorthand" note in §3.3. Surface types
(`Endpoint`/`StreamPeer`/`CallbackPeer`) expose only the
call-creation and out-of-dialog entry points (`invite`, `register`,
and where applicable `subscribe`, `message`, `options`) that need
surface-level pre-fill of `from`/`local_uri`. In-dialog operations
do not need surface forwarders because by the time they're sent the
caller is already holding a `SessionHandle`, not the surface that
produced it. The coordinator-keyed `coord.<verb>(&session_id, …)`
entries remain available for code that holds only a `CallId`.

---

## 5. `HeaderPolicy` — layer-boundary enforcement

`src/api/headers/policy.rs`. Three roles:

```rust
pub enum HeaderRole {
    StackManaged,
    MethodShaped { setter: &'static str },
    ApplicationControlled,
}

pub fn classify(method: Method, name: &HeaderName) -> HeaderRole;

/// Whether `name` should be silently filtered when carrying through
/// from an inbound message. StackManaged names are always filtered;
/// the trace logs `tracing::warn!` listing skipped names.
pub fn forbidden_for_carry_through(name: &HeaderName) -> bool;

/// Method-specific check that all required application-supplied
/// headers are present. Run by every builder's `.send()` before
/// dispatch.
pub fn validate_outbound(method: Method, headers: &[TypedHeader])
    -> Result<(), Vec<MissingRequiredHeader>>;

#[derive(Debug, Clone)]
pub struct MissingRequiredHeader {
    pub method: Method,
    pub name: HeaderName,
    pub reason: &'static str,    // why this header is required for this method
}
```

### 5.1 Classification matrix (method-aware)

| Header | INVITE | re-INVITE | REGISTER | BYE / CANCEL / NOTIFY / INFO / UPDATE (in-dialog) | MESSAGE (out-of-dialog) | SUBSCRIBE (init) | SUBSCRIBE (refresh) | OPTIONS | 3xx response |
|---|---|---|---|---|---|---|---|---|---|
| `Contact` | shaped (`with_contact_uri`, initial only) | stack | shaped (`with_contact_uri`) | stack | shaped | shaped | stack | n/a | shaped |
| `Authorization` | shaped (`with_credentials`) | shaped | shaped | stack | shaped (`with_credentials`) | shaped | stack | shaped (`with_credentials`) | n/a |
| `Expires` | n/a | n/a | shaped (`with_expires`) | n/a | n/a | shaped (`with_expires`) | shaped | n/a | n/a |
| `Refer-To` | n/a | n/a | n/a | shaped (REFER ctor) | n/a | n/a | n/a | n/a | n/a |
| `Event`, `Subscription-State` | n/a | n/a | n/a | shaped (NOTIFY setter) | n/a | shaped | shaped | n/a | n/a |
| `Call-ID`, `CSeq`, `Via`, `Max-Forwards`, `Record-Route`, `Content-Length` | stack | stack | stack | stack | stack | stack | stack | stack | stack |
| `Route` | stack | stack | stack | stack | stack | stack | stack | stack | n/a |

Always application-controlled regardless of method:
`Diversion`, `History-Info`, `Referred-By`, `Replaces`,
`P-Asserted-Identity`, `P-Preferred-Identity`, `Privacy`, `Reason`,
`Retry-After`, `Warning`, `Subject`, `Date`, `User-Agent`, `Server`,
`Accept`, `Allow`, `Supported`, `Require`, `Path`, `Service-Route`,
`Reply-To`, `Target-Dialog`, `Session-Expires`, `Min-SE`, all `X-*`,
and every `Other(_)` not listed above.

### 5.2 CANCEL specifics

RFC 3261 §9.1 requires CANCEL copy `Call-ID`, `From`, `To` (without
tag), `CSeq` (with method changed), and `Route` from the INVITE.
These are all `StackManaged` for CANCEL — the dialog layer
(`transaction/utils/request_builders.rs::create_cancel_from_invite`)
clones them deterministically. Applications can still attach
`Reason` and arbitrary `X-*` via the builder.

### 5.3 Header-name canonicalization

`HeaderName` already canonicalizes via `as_str()`. The policy module
canonicalizes mixed-case raw strings before classification so
`"X-Customer-ID"`, `"x-customer-id"`, and `"X-CUSTOMER-ID"` all map
to the same `Other` variant. `with_raw_header(name, value)` normalizes
`name` to canonical cased form at staging time so wire output is
RFC-tidy.

### 5.4 Validation runs on the application-staged slice

`HeaderPolicy::validate_outbound` runs in the builder's `.send()`
before the options struct crosses into dialog-core. Only
application-staged headers are visible at that point; CSeq, Call-ID,
Via, From-tag, To-URI, Max-Forwards, Content-Length, and in-dialog
Contact have not yet been stamped. This is by design — the builder
enforces the application-controlled slice; the stack-managed slice
cannot be corrupted because it does not exist in the staged vector.

`rvoip_sip_core::validation::{validate_wire_request, validate_wire_response}`
already runs inside the transaction layer after the full message is
assembled. That is the belt-and-braces check and runs regardless of
`BuilderStrictness`.

---

## 6. Config + builder coexistence

Existing `Config` (`src/api/unified.rs:185`) keeps working. Builders
inherit Config defaults at the `DialogAdapter` boundary and override
them with builder-supplied values when present.

### 6.1 Merge precedence (highest priority first)

| Field | Resolution |
|---|---|
| `From` URI / display | `builder.with_from_uri` / `with_from_display` → surface local URI → `Config.local_uri` |
| `Contact` URI (initial INVITE / REGISTER) | `builder.with_contact_uri` → surface contact override → `Config.sip_contact_mode` |
| `P-Asserted-Identity` | `builder.without_pai` suppresses; `builder.with_pai` overrides; else `Config.pai_uri` |
| `Authorization` (UAC) | `builder.with_precomputed_authorization` → `builder.with_credentials` → `Config.credentials` |
| `Route` (outbound proxy) | `builder.with_outbound_proxy` overrides; `builder.without_outbound_proxy` suppresses; else `Config.outbound_proxy_uri` |
| `User-Agent` | `builder.with_header(UserAgent)` → `Config`-driven default |
| `Allow`, `Max-Forwards`, `Via`, `Call-ID`, `CSeq`, `Content-Length` | stack-managed by dialog/transaction layer; not reachable via builder |

### 6.2 Three paths, three examples

**Path 1 — Pure Config (today's experience, unchanged).** Existing
flat methods stay (deprecated wrappers; see §10).

```rust
let cfg = Config::default()
    .with_local_uri("sip:alice@pbx.example")
    .with_pai_uri("sip:+15551234@pbx.example")
    .with_outbound_proxy_uri("sip:proxy.carrier.net");
let coord = UnifiedCoordinator::start(cfg).await?;
let id = coord.make_call("sip:bob@pbx.example").await?;
```

**Path 2 — Pure builder.** Builders override everything for a single
request without mutating Config.

```rust
let id = coord.invite(local_from, "sip:bob@pbx.example")
    .with_credentials(creds)
    .with_pai("sip:+15551234@pbx.example")
    .with_raw_header("X-Customer-ID", customer_id)?
    .send().await?;
```

**Path 3 — Mixed (the B2BUA / SBC case).**

```rust
let (outbound, report) = coord
    .invite(/* from */ None, upstream)    // None → Config.local_uri
    .with_headers_from(&incoming, &[
        HeaderName::HistoryInfo,
        HeaderName::Diversion,
    ])?
    .strip_header(&HeaderName::Privacy)
    .with_raw_header("P-Asserted-Identity", rewritten_pai)?
    .without_pai()                        // suppress Config.pai_uri this call
    .with_strictness(BuilderStrictness::Lenient);
let session = outbound.send().await?;
```

---

## 7. Cross-layer plumbing

### 7.1 Unified outbound dispatch — every method through the state machine

**The eight in-dialog outbound methods route through
`Action::Send*WithOptions`.** This consolidation was previously split
across "state-machine path" (INVITE / REGISTER refresh / etc.) and
"direct DialogAdapter dispatch" (REFER / INFO / UPDATE). The split
made auto-emit infrastructure (§7.4) impossible to apply uniformly
and forfeited observability — the implementer of any future "trace
every outbound" hook would have to instrument two paths.

The four out-of-dialog methods — OPTIONS, MESSAGE, initial REGISTER,
initial SUBSCRIBE — are an explicit carve-out (see the OOB note
below); they call the `DialogAdapter` mirror methods directly. Their
options structs and `extra_headers` plumbing are identical to the
in-dialog path; only the dispatch entry point differs.

The state machine's transition table (`state_table/yaml_loader.rs`) is
**not** modified. Only the `Action` enum (`state_table/types.rs:469-631`)
gains new variants:

```rust
// Existing variants — widened payload, no semantic change:
Action::SendINVITEWithOptions(Arc<OutboundCallOptions>)
Action::SendReINVITEWithOptions(Arc<ReInviteRequestOptions>)
Action::SendREGISTERWithOptions(Arc<RegisterRequestOptions>)
Action::SendSUBSCRIBEWithOptions(Arc<SubscribeRequestOptions>)
Action::SendMESSAGEWithOptions(Arc<MessageRequestOptions>)
Action::SendNOTIFYWithOptions(Arc<NotifyRequestOptions>)
Action::SendBYEWithOptions(Arc<ByeRequestOptions>)
Action::SendCANCELWithOptions(Arc<CancelRequestOptions>)

// New variants (no pre-existing analog):
Action::SendREFERWithOptions(Arc<ReferRequestOptions>)
Action::SendINFOWithOptions(Arc<InfoRequestOptions>)
Action::SendUPDATEWithOptions(Arc<UpdateRequestOptions>)
Action::SendOPTIONSWithOptions(Arc<OptionsRequestOptions>)
```

**REGISTER and SUBSCRIBE refresh** reuse the parent action
(`SendREGISTERWithOptions` / `SendSUBSCRIBEWithOptions`)
respectively. The options struct carries a `refresh: bool` field;
the state-machine handler dispatches to dialog-core's
`send_register_with_options` (initial) or
`send_register_refresh_with_options` (refresh) based on the flag.
This avoids action-variant proliferation while preserving the
dialog-core API distinction. `Action::SendSubscribeRefreshWithOptions`
is **not** a separate variant — `Action::SendSUBSCRIBEWithOptions`
carries the `refresh: bool` flag and routes accordingly.

`OutboundCallOptions` is a **rvoip-sip-side** options struct (not in
dialog-core) — INVITE carries rvoip-sip concerns dialog-core doesn't
need (PAI mode, credentials, transfer-leg tracking,
`supported_100rel`). The DialogAdapter unpacks it at the boundary and
calls dialog-core's existing
`make_call_with_extra_headers_for_session`:

```rust
// src/api/send/outbound_call.rs (rvoip-sip)
#[derive(Default, Debug, Clone)]
pub struct OutboundCallOptions {
    pub from: Option<String>,
    pub to: String,
    pub sdp: Option<String>,
    pub credentials: Option<Credentials>,
    pub pai_override: PaiOverride,             // Default | Suppress | Use(String)
    pub contact_uri: Option<String>,
    pub outbound_proxy_override: ProxyOverride,
    pub subject: Option<String>,
    pub from_display: Option<String>,
    pub precomputed_auth: Option<String>,
    pub transfer_leg: Option<SessionId>,
    pub supported_100rel: bool,
    pub extra_headers: Vec<TypedHeader>,       // application-staged
}
```

The other eleven options structs (`ReInviteRequestOptions`,
`RegisterRequestOptions`, etc.) live in dialog-core's `api/unified.rs`
per §7.2 and ride the Action payload directly.

Each `Action::Send*WithOptions` handler in
`state_machine/actions.rs` reads the `Arc`, calls the matching
`DialogAdapter::send_*_with_options` method, and on dispatch clears
the stashed options (see §7.3 lifecycle).

The state-machine table's existing transitions stay; only the action
payloads widen. No new states. The four methods without state-machine
entry triggers today (REFER, INFO, UPDATE, OPTIONS — all bypass the
state machine in current code via direct `DialogAdapter` calls) gain
new entry events (`EventType::SendREFER`, `EventType::SendINFO`, etc.)
that drive the new actions. REFER's existing
`transfer_target` / `replaces_header` / `transfer_state` stash fields
on `SessionState` collapse into `ReferRequestOptions`; the direct
`dialog_adapter.send_refer_session` call site at `unified.rs:2260` is
retired in favor of the state-machine route.

**Out-of-dialog methods (OPTIONS, MESSAGE, initial REGISTER, initial
SUBSCRIBE) carve out from the unified state-machine route.** These
methods have no established SIP dialog when they're sent, and the
response shape is synchronous (final 2xx / 4xx-6xx without a long
session lifecycle). Routing them through ephemeral `SessionState`
entries solely for transaction tracking would yield no externally
observable benefit and adds per-request session-allocation overhead.

The shipped implementation calls the `DialogAdapter::send_*_oob_with_options`
mirror methods directly from each OOB builder's `.send()`; auth retry,
response correlation, and trace emission flow through dialog-core
unchanged (this is the same path the legacy flat methods used). All
four OOB builders pass `extra_headers` through to dialog-core, so the
wire-output contract — application headers reach the wire — is
identical to the in-dialog state-machine path.

In-dialog methods (INVITE, re-INVITE, BYE, CANCEL, REFER, NOTIFY, INFO,
UPDATE, REGISTER refresh, SUBSCRIBE refresh) route through
`Action::Send*WithOptions`. The observability and auto-emit
contracts (§7.4) apply to those eight in-dialog paths. The OOB
carve-out is documented again in §15.

### 7.2 Dialog-core extensions (additive)

`src/api/unified.rs` in `rvoip-sip-dialog` gains new options structs
and `*_with_options` methods. Every options struct derives `Default`.

```rust
#[derive(Default, Debug, Clone)]
pub struct ReferRequestOptions {
    pub refer_to: String,
    pub replaces: Option<String>,
    pub referred_by: Option<String>,
    pub target_dialog: Option<String>,           // RFC 4538
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct NotifyRequestOptions {
    pub event: String,
    pub subscription_state: String,
    pub content_type: Option<String>,
    pub body: Option<Bytes>,
    pub subscription_id: Option<SubscriptionId>,
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct InfoRequestOptions {
    pub content_type: String,
    pub body: Bytes,
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct ByeRequestOptions {
    pub reason: Option<String>,                  // RFC 3326
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct CancelRequestOptions {
    pub reason: Option<String>,
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct UpdateRequestOptions {
    pub sdp: Option<String>,
    pub session_timer_refresh: bool,
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct ReInviteRequestOptions {
    pub sdp: Option<String>,
    pub session_timer_refresh: bool,
    pub precomputed_authorization: Option<String>,
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct SubscribeRequestOptions {
    pub event: String,
    pub expires: u32,
    pub accept: Option<String>,
    pub from_uri: Option<String>,
    pub contact_uri: Option<String>,
    pub credentials: Option<Credentials>,
    /// false = initial out-of-dialog SUBSCRIBE; true = in-dialog
    /// refresh. State-machine handler routes to
    /// `send_subscribe_with_options` or
    /// `send_subscribe_refresh_with_options` accordingly.
    pub refresh: bool,
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct MessageRequestOptions {
    pub from_uri: String,
    pub to_uri: String,
    pub content_type: String,
    pub body: Bytes,
    pub credentials: Option<Credentials>,
    pub extra_headers: Vec<TypedHeader>,
}

#[derive(Default, Debug, Clone)]
pub struct OptionsRequestOptions {
    pub from_uri: String,
    pub to_uri: String,
    pub accept: Option<String>,
    pub timeout: Option<Duration>,
    pub extra_headers: Vec<TypedHeader>,
}

// Existing `RegisterRequestOptions` (unified.rs:228-239) currently
// derives `Debug, Clone` only. This phase adds `Default`,
// `extra_headers: Vec<TypedHeader>`, AND `refresh: bool` (false for
// initial REGISTER, true for refresh — state-machine handler routes
// to `send_register_with_options` or
// `send_register_refresh_with_options` accordingly).

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
    pub async fn send_message_out_of_dialog_with_options(&self, MessageRequestOptions)
        -> ApiResult<TransactionKey>;
    pub async fn send_options_out_of_dialog_with_options(&self, OptionsRequestOptions)
        -> ApiResult<TransactionKey>;
}
```

**Implementation:** each new method delegates to the existing internal
request-builder path. For in-dialog methods, the path is
`transaction/dialog/request_builder_from_dialog_template`, which
already accepts `extra_headers: Option<Vec<TypedHeader>>`
(`transaction/dialog/mod.rs:107-118`) and appends them **after** all
stack-managed headers are stamped. For out-of-dialog methods
(MESSAGE, OPTIONS), the path is `transaction/utils/request_builders.rs`.
OPTIONS is brand-new authorship on top of the transaction layer
(today no `send_options*` exists in dialog-core at all).

Dialog-core's legacy public methods on `UnifiedDialogApi`
(`send_refer`, `send_notify`, `send_info`, `send_info_with_content_type`,
`send_bye`, `send_cancel`, `send_update`, `send_reinvite`,
`send_subscribe_out_of_dialog`, `send_subscribe_out_of_dialog_for_session`,
`send_subscribe_refresh`, `send_message_out_of_dialog`) were
**removed** in the 2026-05-18 / Gap 1 revision after the
`DialogAdapter` cutover proved no rvoip-sip caller reached them. The
`*_with_extras` helpers backing the `*_with_options` family
(`send_subscribe_out_of_dialog_with_extras`,
`send_message_out_of_dialog_with_extras`) stayed because the new
canonical methods delegate to them internally. `rvoip-sip-registrar`
does not depend on any legacy method. Dialog-core's internal parallel
handle APIs (`DialogHandle` in `api/common.rs`, `DialogClient` in
`api/client.rs`) still go directly to the manager layer — they are
out of scope for this design and are tracked as Gap 2 / Gap 3 in
the post-shipping audit.

Dialog state machine, route-set logic, transaction core, CSeq
management are untouched. `DialogImpl.local_cseq` line 47,
`increment_local_cseq()` line 706, `route_set: Vec<Uri>` line 56 stay
authoritative.

### 7.3 Stash lifecycle on `SessionState`

`SessionState` (`session_store/state.rs`) carries existing stash
fields (`extra_headers`, `pending_bye_reason`, `pending_reinvite`).
This design adds twelve sibling fields:

```rust
pub struct SessionState {
    // existing fields...
    pub pending_invite_options: Option<Arc<OutboundCallOptions>>,
    pub pending_reinvite_options: Option<Arc<ReInviteRequestOptions>>,
    pub pending_register_options: Option<Arc<RegisterRequestOptions>>,
    pub pending_refer_options: Option<Arc<ReferRequestOptions>>,
    pub pending_bye_options: Option<Arc<ByeRequestOptions>>,
    pub pending_cancel_options: Option<Arc<CancelRequestOptions>>,
    pub pending_notify_options: Option<Arc<NotifyRequestOptions>>,
    pub pending_subscribe_options: Option<Arc<SubscribeRequestOptions>>,
    pub pending_info_options: Option<Arc<InfoRequestOptions>>,
    pub pending_update_options: Option<Arc<UpdateRequestOptions>>,
    pub pending_message_options: Option<Arc<MessageRequestOptions>>,
    pub pending_options_options: Option<Arc<OptionsRequestOptions>>,
}
```

**Lifecycle invariants:**

1. **Set-once, consumed-once.** The `.send()` builder writes the
   `Arc<XxxRequestOptions>` to the stash; the `Action::Send*WithOptions`
   handler reads it, dispatches, and clears the field back to `None`
   **once the transaction reaches a final response** (success,
   terminal failure including `InviteAuthRetryExhausted`, or hard
   timeout). The clear happens at the response-resolution point, not
   at action-dispatch — auth retry needs the stash to persist across
   multiple dispatches.
2. **Auth retry re-reads, never re-sets.** The 401/407 retry loop
   reads the same `Arc<XxxRequestOptions>` for the retry transaction
   (only swapping in the computed `Authorization`). The options struct
   stays in the stash until the transaction reaches a final response.
   This applies to every UAC method that exposes `with_credentials`
   — INVITE / REGISTER / SUBSCRIBE / MESSAGE / OPTIONS / re-INVITE.
   Today's `invite_auth_retry_count` (state.rs:122) is the INVITE
   counter; Phase C adds sibling counters
   (`register_auth_retry_count`, etc.) and the state-machine retry
   handler treats them uniformly.
3. **491 re-INVITE glare retry re-reads, never re-sets.**
   `pending_reinvite_options` survives the RFC 3261 §14.1 retry the
   same way.
4. **Session teardown clears all stashes.** On entry to `Terminated`,
   every `pending_*_options` is set to `None`.
5. **Single in-flight outbound per session per method.** The stash
   is a single-slot `Option<Arc<...>>` per method. If an application
   calls `.bye().send()` while another `.bye().send()` future on the
   same session is still in-flight, the second `.send()` sees
   `pending_bye_options: Some(_)` and returns
   `Err(SessionError::Conflict { method: Method::Bye })` — a new
   variant added to `SessionError`. The application must `await` the
   first `.send()` (or drop it cleanly per §12.1) before starting
   another of the same method. Different methods on the same session
   are independent (e.g., simultaneous `.info()` and `.notify()` are
   fine because they use different stash slots).

### 7.4 Auto-emitted outbound — one mechanism

The state machine emits BYE on session-timer expiry, CANCEL on
`dialog_terminated_during_INVITE`, NOTIFY on subscription state
change driven by upstream REFER. These auto-emissions inherit
application defaults via **one** `Config` field:

```rust
pub struct Config {
    // ...existing fields...

    /// Headers injected into every outbound SIP message the state
    /// machine emits automatically. StackManaged names rejected at
    /// Config-construction time. Applies to auto-BYE, auto-CANCEL,
    /// auto-NOTIFY; ignored on application-initiated builders (those
    /// inherit Config defaults via the merge table in §6.1 only).
    pub auto_emit_extra_headers: Vec<TypedHeader>,
}
```

Per-call overrides for auto-emitted messages are not needed in the
common case. If an application requires per-session control (e.g.,
"this leg's auto-BYE should carry a specific X-Trace-ID"), it stashes
the options proactively via a normal `.bye()` builder before the
state machine reaches the auto-emit point — the existing stash
lifecycle (§7.3) handles it. No new public API for per-call
auto-emit control is introduced.

**Precedence when both apply:** the state machine's auto-emit handler
checks the `pending_<method>_options` stash first; if `Some(opts)`,
those win and `Config.auto_emit_extra_headers` is **not** appended
(the application has expressed an explicit per-call intent). If
`None`, `Config.auto_emit_extra_headers` is used. The two never
combine — applications get pure-Config defaults *or* per-call
options, never an automatic merge of both.

### 7.5 Inbound enrichment — preserve original bytes through the bus

Phase E (§9.5) needs the parsed `Request` for inbound REGISTER, REFER,
NOTIFY, INFO, OPTIONS, MESSAGE, UPDATE, and the inbound INVITE so the
new `IncomingRequest` / `IncomingResponse` / `IncomingRegister` types
can wrap a typed Request.

**Constraint:** `infra-common` cannot depend on `rvoip-sip-core`. The
bus payload is `Arc<bytes::Bytes>`, not `Arc<rvoip_sip_core::Request>`.

**Implementation — no double-parse (shipped):** the transport
layer preserves the original wire bytes from the inbound parse on
`TransportEvent::MessageReceived.raw_bytes: Option<Arc<Bytes>>`
(UDP/TCP/TLS/WS — see `crates/rvoip-sip-transport/src/transport/mod.rs`).
Dialog-core's transaction manager caches those bytes on a
per-transaction key in `pending_inbound_bytes`. The cross-crate
event bridges (`events/event_hub.rs`, `events/adapter.rs`,
`protocol/register_handler.rs`, `manager/protocol_handlers.rs`)
call `TransactionManager::take_inbound_bytes(&transaction_id)` to
source the upstream byte form when constructing
`DialogToSessionEvent::{IncomingCall, IncomingRegister, CallProgress,
CallEstablished, CallFailed, ReinviteReceived, InfoReceived,
MessageReceived, OptionsReceived, TransferRequested, NotifyReceived}`.
Synthetic / mock-transport paths (raw_bytes = None) fall back to
re-serialising the parsed Request/Response.

This avoids the round-trip the earlier design proposed (serialize
parsed Request → ship bytes → re-parse) which would have double-parsed
every inbound mid-dialog request for the system's lifetime. **It also
enables STIR/SHAKEN Identity verification (RFC 8224)** — the signed
canonical form survives end-to-end, so downstream verifiers recompute
the signature against the upstream signer's exact bytes.

**Plumbing (additive to transport + dialog-core):**

1. `TransportEvent::MessageReceived` gained
   `raw_bytes: Option<Arc<Bytes>>`. Each transport
   (`udp.rs`, `transport/udp/mod.rs`, `transport/tcp/connection.rs`,
   `transport/tls/mod.rs`, `transport/ws/connection.rs`) freezes
   the parser's input buffer via `Bytes::copy_from_slice` (TCP/TLS/WS)
   or `packet.clone()` (UDP) and ships it on the event.
2. `TransactionManager::handle_transport_event` keys the bytes by
   `transaction_key_from_message(&message)` and inserts them into
   `pending_inbound_bytes: DashMap<TransactionKey, Arc<Bytes>>`.
3. Cross-crate bridge sites consume via
   `take_inbound_bytes(&transaction_id)` (or `peek_inbound_bytes`
   when multiple bridges fire for one inbound message).
4. Cache entries are swept on `TransactionEvent::TransactionTerminated`
   so unconsumed bytes don't leak.

**Cross-crate variant changes in `infra-common::events::cross_crate.rs`:**

```rust
// Phase A — additive enrichment on the IncomingCall / IncomingRegister
// / response-inspection variants:
DialogToSessionEvent::IncomingCall      { ..existing.., raw_request:  Arc<Bytes> }
DialogToSessionEvent::IncomingRegister  { ..existing.., raw_request:  Arc<Bytes> }
DialogToSessionEvent::CallProgress      { ..existing.., raw_response: Arc<Bytes> }
DialogToSessionEvent::CallEstablished   { ..existing.., raw_response: Arc<Bytes> }
DialogToSessionEvent::CallFailed        { ..existing.., raw_response: Arc<Bytes> }

// Phase E — additive enrichment on the mid-dialog inbound variants:
DialogToSessionEvent::TransferRequested { ..existing.., raw_request: Arc<Bytes> }
DialogToSessionEvent::NotifyReceived    { ..existing.., raw_request: Arc<Bytes> }
DialogToSessionEvent::ReinviteReceived  { ..existing.., raw_request: Arc<Bytes>, method: String }
//                                                                    ^ "INVITE" or "UPDATE"

// Phase E — new variants (today these inbound methods are dropped
// at dialog-core or not bridged to the bus):
DialogToSessionEvent::InfoReceived    { session_id: String, raw_request: Arc<Bytes> }
DialogToSessionEvent::MessageReceived { session_id: String, raw_request: Arc<Bytes> }
DialogToSessionEvent::OptionsReceived { session_id: String, raw_request: Arc<Bytes> }
```

**OPTIONS today is dropped at the dialog-core layer entirely.** The
internal `SessionCoordinationEvent::CapabilityQuery`
(`protocol_handlers.rs:277, 526`) is not bridged to the cross-crate
bus. Phase E performs new authorship for OPTIONS bridging.

**For UPDATE:** the existing `ReinviteReceived` variant is enriched
with `raw_request` and a `method: String` field so consumers can
branch on INVITE vs UPDATE. No dedicated `UpdateReceived` variant is
added — smaller bus payload, single handler path.

The `infra-common/Cargo.toml` adds `bytes = { workspace = true }`
(needed for the `Bytes` type). No new transitive dependencies; `bytes`
is already pulled in by `rvoip-sip-core` and `rvoip-sip-transport`.

### 7.6 Outbound verbatim bytes — `Transport::send_message_raw`

Symmetric to §7.5 on the receive side: the `Transport` trait
gained `send_message_raw(bytes: Bytes, dest: SocketAddr) -> Result<()>`
so applications can ship a pre-built byte buffer to the wire without
round-tripping through `Message::to_bytes()`. Each transport
implements:

- **UDP** — `socket.send_to(&bytes, destination)` directly.
- **TCP** — resolve/open a pooled connection and call
  `TcpConnection::send_raw_bytes(&bytes)` (existing helper).
- **TLS** — route through the per-connection mpsc channel
  (`send_to_addr(bytes, dest, server_name=None)`); auto-dials when
  no pooled connection exists.
- **WebSocket** — wrap in a `WsMessage::Binary` frame and write to
  the sink.

This is distinct from the existing `Transport::send_raw`, which is
the RFC 5626 §3.5.1 CRLFCRLF keep-alive ping path (works only on
already-open connection-oriented transports).

**Use cases unlocked:**

- **Signature-preserving SBC pass-through.** Receive `raw_bytes`
  from `IncomingCall::raw_request`, rewrite only the headers the
  SBC owns (Via, Record-Route, Contact), forward via
  `send_message_raw`. Identity, P-Asserted-Identity, and other
  signed/opaque headers retain their exact upstream byte form
  (RFC 8224).
- **Stateless proxy forwarding** — RFC 3261 §16 minimal mutations,
  no AST round-trip.
- **Fuzz testing and compliance suites** — ship arbitrary byte
  buffers (including malformed) without transport-layer validation.
- **Replay tooling** — feed captured `raw_bytes` (pcap-extracted or
  recorded from a previous session) back through the wire.

---

## 8. Error model

`rvoip-sip` exposes one error enum, `SessionError`
(`errors.rs:8-85`). This design adds three variants (additive,
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

/// Returned when a second `.send()` is attempted on the same session
/// for a method whose `pending_<method>_options` stash slot is still
/// occupied by an in-flight prior `.send()`. See §7.3 invariant #5.
#[error("another {method} is already in flight on this session")]
Conflict {
    method: rvoip_sip_core::Method,
},
```

`HeaderPolicyViolation` maps cleanly: `From<HeaderPolicyViolation> for
SessionError` produces the `HeaderPolicy` variant. The `?` operator on
every builder setter flows the policy violation into the surrounding
`Result<_, SessionError>` without custom adapter code.

**Existing variants reused without change:**

- `OPTIONS` timeout → existing `SessionError::Timeout(String)` with a
  structured message.
- Auth retry exhaustion → existing `InviteAuthRetryExhausted`.
- General dispatch errors → existing `DialogError`, `ProtocolError`,
  `InvalidInput`.

---

## 9. Implementation phases

Five phases. Each shippable as a separate PR in this order. Every
phase preserves existing behavior — no breaking changes within this
design's scope.

### Phase A — Inbound inspection

Files:
- `src/api/headers/mod.rs`, `view.rs` (new) — `SipHeaderView` trait
- `src/api/incoming.rs` — add `request: Option<Arc<rvoip_sip_core::Request>>`
  field to `IncomingCall` (Option-typed for lean-mode readiness per
  §13.3; populated as `Some(...)` by default in Phase A), implement
  `SipHeaderView`, populate the field at session creation, fix the
  empty-HashMap bug, deprecate the field
- `src/api/events.rs` — add `IncomingResponse`, `IncomingRequest`,
  `IncomingRegister` types; introduce
  `Event::CallProgressDetailed(IncomingResponse)` for every inbound
  response (1xx / 2xx / 3xx / 4xx-6xx). The legacy
  `Event::CallProgress` (pre-decoded fields) stays in parallel and
  continues to fire on every 1xx — non-migrated callers see no
  behavior change; B2BUA-style callers subscribe to the new
  Detailed variant. Same coexistence pattern for
  `Event::CallEstablishedDetailed` / `Event::CallFailedDetailed`
  alongside their legacy versions.
- `infra-common/Cargo.toml` — add `bytes = { workspace = true }`
- `infra-common/src/events/cross_crate.rs` —
  enrich `DialogToSessionEvent::IncomingCall` AND
  `DialogToSessionEvent::IncomingRegister` with
  `raw_request: Arc<Bytes>`. (IncomingRegister enrichment lands in
  Phase A so Phase D's `RegisterResponseBuilder` can be shipped
  without requiring Phase E. Inbound CallProgress and inbound 1xx /
  2xx / 3xx-6xx response inspection use the existing
  `CallProgress`/`CallEstablished`/`CallFailed` payloads enriched in
  Phase A with `raw_response: Arc<Bytes>`.)
- `rvoip-sip-dialog/src/manager/...` — preserve original `Bytes` at
  the inbound parse site; thread them to the INVITE and REGISTER
  publish sites (and the response publish sites in dialog-core for
  IncomingResponse)
- `src/lib.rs` — re-export `SipHeaderView` + the new inbound types;
  add a "Gateway / B2BUA / SBC Authoring" section to the crate-level
  `//!` containing:
  1. The developer decision chart (§11.1)
  2. The B2BUA litmus-test example (§11.2)
  3. The three trust-boundary patterns (§11.3)
  4. A header-classification reference summary
     (StackManaged / MethodShaped / ApplicationControlled)
  5. Cross-links to `SipHeaderView`, `SipRequestOptions`,
     `HeaderPolicy` API pages

Acceptance:
- `IncomingCall.header(&HeaderName::Diversion)` returns typed access
  for inbound INVITE carrying Diversion.
- `Event::CallProgressDetailed(IncomingResponse)` fires on every 1xx,
  not just finals.
- `IncomingCall.headers: HashMap` populated from the parsed INVITE
  (back-compat for existing readers).
- Crate-level `//!` includes all five items above; verified by test
  #32 (manual doc inspection).

### Phase B — Dialog-core options extension

Files (`rvoip-sip-dialog`):
- `src/api/unified.rs` — add 11 new options structs + 11
  `*_with_options` methods. Of those:
  - 10 (REFER / NOTIFY / INFO / BYE / CANCEL / UPDATE / re-INVITE /
    SUBSCRIBE / SUBSCRIBE-refresh / MESSAGE) wrap existing internal
    request-builder paths
  - 1 (`send_options_out_of_dialog_with_options`) is brand-new
    authorship
  Add `Default`, `extra_headers: Vec<TypedHeader>`, and `refresh: bool`
  to the existing `RegisterRequestOptions` (lines 228-239).
- `src/transaction/utils/request_builders.rs` — author the new
  out-of-dialog OPTIONS request-builder helper. Layer-parallel to
  the existing `send_message_out_of_dialog` helper.
- `src/manager/dialog_operations.rs` (or sibling) — internal
  glue connecting the new `*_with_options` API methods to the
  internal builder paths. The dialog template
  (`transaction/dialog/mod.rs:107-118`) already appends
  `extra_headers` after stack-managed headers; no change required
  there.

Acceptance:
- Every `*RequestOptions` struct derives `Default`.
- `RegisterRequestOptions` has `Default` and `extra_headers`.
- `send_options_out_of_dialog_with_options` is layer-parallel to
  `send_message_out_of_dialog_with_options`.
- `cargo test -p rvoip-sip-dialog` passes (no regressions).

### Phase C — Send-side builders

Files:
- `src/api/headers/options.rs` — `SipRequestOptions` trait,
  `BuilderHeaderState`, `HeaderPolicyViolation`,
  `HeaderCarryThroughReport`, `ViolationReason`,
  `BuilderStrictness`
- `src/api/headers/policy.rs` — `classify`,
  `forbidden_for_carry_through`, `validate_outbound`, const lookup
  tables
- `src/api/headers/convenience.rs` — typed helper constructors for
  headers without `TypedHeader` variants in sip-core (`Diversion`,
  `History-Info`, `Privacy`, `Replaces`, `Target-Dialog`,
  `Session-Expires`, `Min-SE`, `P-Charging-Vector`,
  `P-Called-Party-ID`); body helpers (`sdp`, `dtmf_relay`,
  `pidf_xml`, `simple_message_summary`, `isup`, `multipart_mixed`,
  `multipart_parse`)
- `src/api/surface.rs` — `Surface` trait, `SurfaceBuilder<B, S>`
  generic adapter (§4)
- `src/api/send/mod.rs` + 12 builder modules
- `src/api/unified.rs` — 12 entry points (`invite`, `reinvite`,
  `register`, `refer`, `bye`, `cancel`, `notify`, `subscribe`,
  `info`, `update`, `message`, `options`); mark ~22 existing methods
  across four surfaces `#[deprecated(since = "0.3.0", note = "use <surface>.<verb>(...).send().await — see SIP_API_DESIGN_2.md")]`:

  | Surface | Methods deprecated |
  |---|---|
  | `UnifiedCoordinator` | `make_call`, `make_call_with_auth`, `make_call_with_pai`, `make_call_with_headers`, `register`, `register_with`, `send_refer`, `send_refer_with_replaces`, `send_notify`, `send_info`, `hangup_with_reason`, `reject_call`, `redirect_call`, `subscribe_dialogs` |
  | `PeerControl` (`stream_peer.rs`) | `call`, `call_with_auth`, `call_with_headers` |
  | `CallbackPeerControl` (`callback_peer.rs`) | `call`, `call_with_auth`, `call_with_headers` |
  | `Endpoint` / `EndpointControl` (`endpoint.rs`) | `call`, `call_with_headers` |

  `make_transfer_leg` is **not** deprecated — its signature is
  preserved for the b2bua wrapper crate and its body becomes
  `self.invite(from, to).as_transfer_leg(transferor).send().await`.
  The workspace already sets `deprecated = "allow"` at the lint
  level (`Cargo.toml:54-67`), so internal callers compile without
  warnings; external callers see standard deprecation warnings.
- `src/api/{endpoint,stream_peer,callback_peer}.rs` — surface entry
  points via `SurfaceBuilder`; deprecate legacy methods
- `src/api/subscription.rs` (or equivalent) — add
  `SubscriptionHandle::refresh()` returning `SubscribeRefreshBuilder`
- `src/state_table/types.rs` — extend `Action` enum with 12
  `Send*WithOptions` variants (8 widened from existing
  `Action::Send*` payloads, 4 brand-new: REFER, INFO, UPDATE,
  OPTIONS). REGISTER and SUBSCRIBE refresh reuse the parent action
  via a `refresh: bool` flag on the options struct (per §7.1)
- `src/state_machine/actions.rs` — handlers for the 12 actions
- `src/state_machine/helpers.rs` — collapse the 5 INVITE helpers
  into `make_call_inner(opts)`; author `send_xxx_inner(opts)` for
  the other 11 methods
- `src/session_store/state.rs` — 12 `pending_*_options` fields with
  lifecycle invariants (§7.3)
- `src/adapters/dialog_adapter.rs` — 12 `send_*_with_options`
  mirror methods; each translates `SessionId → DialogId`, runs
  `HeaderPolicy::validate_outbound`, applies the Config merge table
  (§6.1), prepends outbound-proxy Route (reusing
  `prepend_outbound_proxy_route` at `dialog_adapter.rs:2086`), and
  forwards to dialog-core
- `src/errors.rs` — add `HeaderPolicy` and `MissingRequiredHeader`
  variants

Acceptance:
- Every builder is `Send + Sync + Sized`.
- `BuilderStrictness::Strict` is default.
- Stash fields are set-once / consumed-once / cleared at session
  termination, and survive auth retry intact.
- All 12 outbound methods route through `Action::Send*WithOptions`.
- Cancel-safety integration test (§10 test #29) passes.

### Phase D — Response builders

Files:
- `src/api/respond/mod.rs` + 7 builder modules (`accept`, `reject`,
  `redirect`, `provisional`, `challenge`, `generic`, `register_response`)
- `src/api/incoming.rs` — entries on each inbound type:
  - `IncomingCall`: `accept_builder()` (→ AcceptBuilder),
    `reject_builder()`, `redirect_builder()`,
    `send_provisional_builder(code)`, `challenge_builder(scheme)`,
    `respond_builder(status)`
  - `IncomingRequest`: `challenge_builder(scheme)`,
    `respond_builder(status)` (generic 3xx-6xx responses for
    in-dialog REFER / NOTIFY / INFO / OPTIONS / UPDATE / MESSAGE)
  - `IncomingRegister`: `accept_builder()` (→
    RegisterResponseBuilder), `reject_builder()`,
    `challenge_builder(scheme)`
- `src/adapters/dialog_adapter.rs` — `send_response_with_options`
  proxy that forwards a pre-built `Response` to
  `UnifiedDialogApi::send_response` and resolves the session's
  pending transaction key
- Existing `IncomingCall::{accept, accept_with_sdp, reject,
  reject_busy, reject_decline, redirect_to, redirect_with_contacts}`
  become one-line wrappers over the builders (kept for back-compat).
  Implementer verification: confirm `accept_with_sdp` exists at the
  shown name (today's variants may include `accept_call_with_sdp` or
  similar) and add it to the wrapper list as found.

Acceptance:
- Every response builder implements `SipRequestOptions`.
- `DialogAdapter::send_response_with_options(session_id, Response)`
  exists.

### Phase E — In-dialog request surface

Files:
- `src/api/events.rs` — enrich `Event::ReferReceived`,
  `Event::NotifyReceived` with `request: IncomingRequest` field
  (additive); add `Event::InfoReceived`, `Event::MessageReceived`,
  `Event::OptionsReceived`, `Event::UpdateReceived` variants
- `src/api/callback_peer.rs` — `CallHandler` trait gains new methods
  taking `IncomingRequest`. Existing positional-arg methods become
  `#[deprecated]` with default implementations that decode the
  `IncomingRequest` into legacy fields. **No `_full` suffix** —
  the new methods are the canonical names going forward.
- `infra-common/src/events/cross_crate.rs` —
  enrich `TransferRequested`, `NotifyReceived`, `IncomingRegister`,
  `ReinviteReceived` with `raw_request: Arc<Bytes>` (and `method`
  on `ReinviteReceived`); add `InfoReceived`, `MessageReceived`,
  `OptionsReceived` variants
- `rvoip-sip-dialog/src/events/session_coordination.rs` — internal
  variants for the inbound methods that today don't surface
  (OPTIONS specifically) gain a Request-carrying form
- `rvoip-sip-dialog/src/events/event_hub.rs:182+` — extend
  `convert_session_coordination_to_cross_crate` to bridge all six
  inbound mid-dialog methods (REFER, NOTIFY, INFO, MESSAGE, OPTIONS,
  UPDATE)
- `src/state_machine/...` — handlers that re-parse `Arc<Bytes>` via
  `rvoip_sip_core::parse_message` into `IncomingRequest`

**Callback rename strategy.** The trait method names in
`callback_peer.rs:814+` change:

| Old (deprecated) | New (canonical) |
|---|---|
| `on_transfer_request(handle, target: String)` | `on_refer_received(handle, request: IncomingRequest)` |
| `on_refer_notify(handle, status, reason, sub_state, body)` | `on_refer_notify(handle, request: IncomingRequest)` |
| `on_notify(handle, event, sub_state, content_type, body)` | `on_notify_received(handle, request: IncomingRequest)` |
| — (today routes through `on_event` only) | `on_info_received(handle, request)` |
| — | `on_message_received(handle, request)` |
| — | `on_options_received(handle, request)` |
| — | `on_update_received(handle, request)` |

Each new method has a default no-op implementation. The old methods
are `#[deprecated]` default-implemented adapters that decode the
typed `Request` into the legacy positional arguments and forward.
Existing `CallHandler` implementations continue to compile and run.
**The deprecated methods are scheduled for removal in the next
breaking release** — the parallel two-method-per-event surface is
not preserved indefinitely.

Acceptance:
- Inbound OPTIONS reaches `CallHandler::on_options_received`
  end-to-end (today it's dropped at dialog-core).
- Inbound INFO, MESSAGE, UPDATE reach typed handlers.
- Cross-crate variants `InfoReceived`, `MessageReceived`,
  `OptionsReceived` and enriched `ReinviteReceived` carry
  `raw_request: Arc<Bytes>`.
- `event_hub.rs::convert_session_coordination_to_cross_crate`
  bridges all six inbound mid-dialog methods (REFER, NOTIFY, INFO,
  MESSAGE, OPTIONS, UPDATE).

---

## 10. Verification

End-to-end test plan, run in order; each must pass before the next:

1. `cargo build -p rvoip-sip-dialog` — additive options structs
   compile; no existing call sites broken.
2. `cargo test -p rvoip-sip-dialog` — full dialog-core suite passes.
3. `cargo build -p rvoip-sip` — builders + policy compile.
4. `cargo doc -p rvoip-sip --no-deps` — clean
   (`#![deny(rustdoc::broken_intra_doc_links)]`).
5. `cargo test --doc -p rvoip-sip` — every new setter has a doc-test
   (~60 new doc-tests).
6. `cargo test -p rvoip-sip --test header_policy_unit` — policy
   table covers every `TypedHeader` variant + every method's
   `MethodShaped` overrides.
7. `cargo test -p rvoip-sip --test header_inspection_integration` —
   inbound INVITE / REFER / NOTIFY / INFO / failure response surfaces
   have `Diversion`, `History-Info`, `Referred-By`, `Retry-After`
   accessible via `SipHeaderView`.
8. `cargo test -p rvoip-sip --test forbidden_header_guard_integration` —
   `with_header(TypedHeader::CallId(...))` returns
   `Err(StackManaged)`; `with_header(TypedHeader::Authorization(...))`
   on `RegisterBuilder` returns
   `Err(UseDedicatedSetter("with_credentials"))`;
   `with_headers_from` reports Via/CSeq/Call-ID/Max-Forwards in
   `report.skipped`.
9. `cargo test -p rvoip-sip --test outbound_request_builders_integration` —
   each of the 12 builders sends an asserted-on-wire custom
   `X-Test` header.
10. `cargo test -p rvoip-sip --test response_builders_integration` —
    reject with `Retry-After`, redirect with multiple Contact +
    q-values, accept with custom header, 401 with `WWW-Authenticate`,
    407 with `Proxy-Authenticate`.
11. `cargo test -p rvoip-sip --test b2bua_carry_through_integration` —
    the B2BUA example (§11) executes; inbound INVITE → outbound
    INVITE carries `History-Info` and `Diversion`, strips `Privacy`,
    rewrites PAI; wire traces validate ordering and assert that
    Via/CSeq/Call-ID/Max-Forwards/Content-Length appear in
    `report.skipped` and not on the outbound wire.
12. `cargo test -p rvoip-sip --test builder_strictness_integration` —
    Strict rejects `with_header(Authorization(...))` on
    `RegisterBuilder` with `Err(UseDedicatedSetter)`; Lenient drops
    silently with `tracing::warn!`; both modes reject
    `with_header(CallId(...))` as hard `Err(StackManaged)`.
13. `cargo test -p rvoip-sip --test config_builder_coexistence` —
    Path 1 / Path 2 / Path 3 examples from §6.2 produce wire output
    matching expected fixtures under `tests/fixtures/`.
14. `cargo test -p rvoip-sip` — full suite including legacy
    `pai_integration.rs` and `extra_headers_integration.rs` (proves
    deprecated wrappers still work).
15. `cargo build --examples -p rvoip-sip` — examples compile despite
    emitting deprecation warnings.
16. `cargo test -p rvoip-sip --test b2bua_contact_rewrite_integration`
    — `OutboundCallBuilder::with_contact_uri` rewrites Contact on
    the outbound INVITE; dialog-core accepts it as the local target.
17. `cargo test -p rvoip-sip --test per_leg_outbound_proxy_integration`
    — `with_outbound_proxy(uri)` on one leg uses a different proxy
    than `Config.outbound_proxy_uri`.
18. `cargo test -p rvoip-sip --test provisional_carry_through_integration`
    — inbound 183 triggers `Event::CallProgressDetailed`; B2BUA
    carries `Contact`/`Allow`/`Server` to downstream 183.
19. `cargo test -p rvoip-sip --test third_party_register_integration`
    — `coord.register(...).with_from_uri(behalf).with_contact_uri(proxy).with_raw_header("P-Asserted-Identity", proxy_pai)?.send()`
    produces a wire REGISTER with rewritten From/Contact/PAI.
20. `cargo test -p rvoip-sip --test generic_response_integration` —
    `incoming.respond_builder(491).with_raw_header("Retry-After", "5")?.send()`
    produces a valid 491 Request Pending with the custom header;
    `respond_builder(1xx)` and `respond_builder(2xx_for_INVITE)` are
    rejected as out-of-range.
21. `cargo test -p rvoip-sip --test builder_auth_retry_preserves_headers`
    — `coord.invite(...).with_credentials(creds).with_raw_header("X-Trace", id)?.send()`
    sees the X-Trace header on both the 401-drawing INVITE and the
    credentialed retry.
22. `cargo test -p rvoip-sip --test header_case_insensitive_lookup`
    — `with_raw_header("x-customer-id", ...)` and
    `headers_named(&HeaderName::Other("X-CUSTOMER-ID".into()))`
    resolve to the same staged header.
23. `cargo test -p rvoip-sip --test stash_lifecycle_integration` —
    (a) after a successful `coord.invite(...).with_raw_header("X-Trace", id)?.send().await`
    the session's `pending_invite_options` is `None`; a subsequent
    re-INVITE does not see the stale `X-Trace` header;
    (b) two concurrent `.bye().send()` calls on the same session: the
    second returns `Err(SessionError::Conflict { method: Method::Bye })`;
    (c) simultaneous `.info()` and `.notify()` on the same session
    both succeed (independent stash slots).
24. `cargo test -p rvoip-sip --test multipart_body_integration` —
    multipart/mixed body with SDP + ISUP parts produces a wire
    INVITE with correct multipart structure; inbound parses back out.
25. `cargo test -p rvoip-sip --test reliable_provisional_bridge` —
    upstream 18x with `Require: 100rel` bridged to downstream;
    `with_supported_100rel(true)` advertises on outbound INVITE.
26. `cargo test -p rvoip-sip --test topology_hiding_guarantee` —
    every Via/Record-Route/Call-ID/CSeq on the inbound leg is
    reported as skipped in `HeaderCarryThroughReport.skipped` when
    `with_headers_from(...)` requests everything; none appears on
    the outbound wire trace.
27. `cargo test -p rvoip-sip --test registrar_response_builder` —
    `IncomingRegister::accept_builder().with_expires(3600).with_service_route(routes).with_path_echo().send()`
    produces a wire 200 OK with all three headers correctly placed.
28. `cargo test -p rvoip-sip --test options_timeout` — OPTIONS times
    out after 32s default; `with_timeout(Duration::from_secs(5))`
    overrides; returned `IncomingResponse` carries Allow/Supported/Server.
29. `cargo test -p rvoip-sip --test cancel_safety_integration` —
    `.send().await` futures dropped at various stages don't leak
    `SessionState` stash; no leaked transaction; no panic.
30. `cargo test -p rvoip-sip --test auto_emit_headers` — Config-level
    `auto_emit_extra_headers` applied to session-timer auto-BYE
    without application code.
31. `cargo test -p rvoip-sip --test trace_redaction` — redactor
    strips `Authorization` from `SipTraceEvent` payload; wire output
    unaffected.
32. Manual: open `target/doc/rvoip_sip/index.html`. Crate-level
    `//!` has a "Gateway / B2BUA / SBC Authoring" section with the
    B2BUA example (§11), the developer decision chart, the
    classification reference, and the three trust-boundary patterns.

---

## 11. Developer guide

### 11.1 Decision chart

| If you say… | Use | Example |
|---|---|---|
| "I just want to make a call, library handles SIP" | Pure Config — unchanged | `coord.make_call(target)` |
| "I need credentials on outbound calls" | One builder | `coord.invite(from, to).with_credentials(c).send()` |
| "I need to attach one custom X-* header" | One builder | `coord.invite(from, to).with_raw_header("X-Foo", "bar")?.send()` |
| "I'm building a B2BUA — carry headers across legs" | Builder + carry-through | `coord.invite(...).with_headers_from(&inbound, &[...])?.send()` |
| "I need lenient validation for messy upstream" | `with_strictness(Lenient)` | `coord.invite(...).with_strictness(BuilderStrictness::Lenient).send()` |
| "I need to inspect every inbound header" | `SipHeaderView` | `incoming.header(&HeaderName::Diversion)` |
| "I'm authoring custom 4xx with Retry-After" | `RejectBuilder` | `incoming.reject_builder().with_status(503).with_retry_after(120).send()` |
| "I'm a registrar with Service-Route on 200 OK" | `RegisterResponseBuilder` | `incoming.accept_builder().with_service_route(...).send()` |
| "I have an app on `make_call_with_pai` — will it work?" | Yes — deprecated but functional | call sites compile, emit deprecation warning |
| "I need to verify STIR/SHAKEN Identity signatures on incoming calls" | `IncomingCall::raw_request()` | upstream byte-exact form survives end-to-end; recompute signature over `raw_request` bytes |
| "I'm building a signature-preserving SBC pass-through" | `Transport::send_message_raw(bytes, dest)` | take inbound `raw_request`, rewrite only Via/Record-Route, forward verbatim |

### 11.2 B2BUA composition (the litmus test)

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

// Bridge media — existing UnifiedCoordinator method, out of scope
// for this design. Signature here is illustrative; consult the
// current rvoip-sip docs for the canonical form.
coord.bridge(&incoming.call_id, &session).await?;
```

### 11.3 Trust-boundary patterns

```rust
// 1. Trusted → untrusted egress: strip PAI, strip History-Info,
//    keep Diversion only if regulator-required.
let (out, _) = coord.invite(from, untrusted_target)
    .with_headers_from(&inbound, &[HeaderName::Diversion])?;
let out = out
    .strip_header(&HeaderName::Other("History-Info".into()))
    .without_pai();

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

### 11.4 Migration guide — top use cases before / after

| Today | Tomorrow |
|---|---|
| `coord.make_call(target)` | unchanged (Config path stays) |
| `coord.make_call_with_auth(target, creds)` | `coord.invite(None, target).with_credentials(creds).send()` |
| `coord.make_call_with_pai(target, pai)` | `coord.invite(None, target).with_pai(pai).send()` |
| `coord.make_call_with_headers(target, hdrs)` | `coord.invite(None, target).with_headers(hdrs)?.send()` |
| `coord.register_with(reg)` | `coord.register(reg.registrar, reg.user, reg.pw).with_expires(reg.expires).send()` |
| `coord.send_refer(&session, target)` | `coord.refer(&session, target).send()` |
| `coord.send_refer_with_replaces(&session, target, replaces)` | `coord.refer(&session, target).with_replaces(replaces).send()` |
| `coord.send_notify(&session, event, body, state)` | `coord.notify(&session, event).with_body(body).with_subscription_state(state).send()` |
| `coord.send_info(&session, ctype, body)` | `coord.info(&session, ctype).with_body(body).send()` |
| `coord.hangup_with_reason(&session, reason)` | `coord.bye(&session).with_reason(reason).send()` |
| `coord.make_transfer_leg(from, to, transferor)` | unchanged signature (the b2bua wrapper crate calls this); body becomes `self.invite(from, to).as_transfer_leg(transferor).send().await` |
| `coord.reject_call(&session, code, reason)` | `coord.reject(&session).with_status(code).with_reason(reason).send()` |
| `incoming.accept()` / `incoming.accept_with_sdp(sdp)` / `incoming.reject()` / `incoming.reject_busy()` / `incoming.reject_decline()` / `incoming.redirect_to(uri)` / `incoming.redirect_with_contacts(list)` | unchanged signatures — all become one-line wrappers over the builders (per Phase D). New code prefers `incoming.accept_builder().with_sdp(sdp).send()` etc. for header authoring. |
| `incoming.headers.get("Diversion")` (today returns `None` — bug) | `incoming.header_str(&HeaderName::Other("Diversion".into()))` |

### 11.5 Response-side carry-through

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
        skipped = ?report.skipped,
        "downstream 200 OK carry-through audit"
    );
    builder.send().await.map(|_| ())
}
```

### 11.6 3rd-party REGISTER (PBX/SBC pattern)

```rust
coord.register(registrar, behalf_user, pw)
    .with_from_uri(behalf_aor)
    .with_contact_uri(my_proxy_contact)
    .with_raw_header("P-Asserted-Identity", proxy_pai)?
    .send().await?;
```

### 11.7 Reliable provisional (RFC 3262) bridging

```rust
// Advertise 100rel support on outbound INVITE
let session = coord.invite(from, to)
    .with_supported_100rel(true)
    .send().await?;

// On inbound 1xx, check reliability
if upstream_resp.is_reliable_provisional() {
    incoming.send_provisional_builder(183)
        .with_sdp(early_media_sdp)
        .with_require_100rel(true)
        .send().await?;
}
// PRACK itself stays under state-machine control — no PrackBuilder.
```

---

## 12. Cross-cutting concerns

### 12.1 Cancel-safety on `.send().await`

Two-phase semantics:

1. **Pre-await preparation** is synchronous. The builder validates
   policy, runs `HeaderPolicy::validate_outbound`, stashes the
   `Arc<XxxRequestOptions>` on `SessionState`, and queues the
   `Action::Send*WithOptions` event. None of these can be cancelled.
2. **Post-await response wait** is the cancel-safe slice. If the
   future is dropped here, the wire message has already gone out;
   the state machine handles the response (or timeout) normally;
   the caller simply doesn't observe the result. The dialog still
   settles.

**Exception:** `MessageBuilder` (fire-and-forget per RFC 3428)
completes immediately after queuing — fully cancel-safe regardless of
caller behavior.

### 12.2 Outbound wire-validity guarantees

Three layered checkpoints ensure outbound messages are RFC-valid:

1. **Builder layer.** `HeaderPolicy::validate_outbound` runs on
   `.send()`. Rejects stack-managed and (under Strict) method-shaped
   violations.
2. **Dialog/transaction layer.** The dialog template
   (`transaction/dialog/mod.rs:102-196`) and out-of-dialog request
   builder (`transaction/utils/request_builders.rs`) stamp
   `Via`, `Max-Forwards`, `Call-ID`, `CSeq`, `From`-tag, `To`-URI,
   `Content-Length`, and (for in-dialog) `Route-Set` and `Contact`
   in RFC-required order, then append application headers.
3. **Wire layer.**
   `rvoip_sip_core::validation::{validate_wire_request, validate_wire_response}`
   runs in the transaction layer after the full message is assembled.
   This is the final correctness gate; runs regardless of
   `BuilderStrictness`.

`Content-Length` is never stageable through `with_header`. It's
`StackManaged` and stamped by `SimpleRequestBuilder` from the body
length immediately before serialization. Bodies travel on the options
struct's `body: Bytes` field; `with_body(impl Into<Bytes>)` accepts
`String`, `&str`, `Vec<u8>`, `Bytes` with no copy on assignment.

### 12.3 Topology hiding (automatic)

Because `Via`, `Record-Route`, in-dialog `Contact`, `Call-ID`,
`CSeq`, and `Max-Forwards` are `StackManaged`, and each B2BUA leg is
an independent session with its own dialog, **topology hiding is
automatic**. Dialog-core generates fresh values for the downstream
leg; the upstream leg's identifiers never leak.

`with_headers_from(&inbound, names)` rejects stack-managed names
automatically via `HeaderPolicy::forbidden_for_carry_through`, so a
naïvely-written B2BUA cannot accidentally leak topology. The
`HeaderCarryThroughReport.skipped` list logs every filtered header
for audit.

### 12.4 Trace redaction (compliance / PII)

`SipTraceEvent` routinely carries `Authorization`, custom
`X-Auth-Token`, and PII headers like `P-Asserted-Identity`. Operators
need a redaction hook before traces hit logs:

```rust
pub trait TraceRedactor: Send + Sync {
    /// Called once per outbound or inbound message before the trace
    /// event is published. `msg` is sip-core's `Message` enum
    /// (Request or Response).
    fn redact(&self, msg: &rvoip_sip_core::Message) -> RedactionDecision;
}

pub enum RedactionDecision {
    Allow,                            // publish trace unchanged
    Redact(Vec<HeaderName>),           // publish trace with these headers masked
    Drop,                              // suppress the trace entirely
}

pub struct Config {
    // ...
    pub trace_redaction: Option<Arc<dyn TraceRedactor>>,
}
```

Default `None` preserves today's behavior. **Wire output is
unaffected** — redaction applies only to the trace event payload. The
emission site in `DialogAdapter` consults the redactor before
publishing.

### 12.5 Subscription multiplex on a single dialog

Multiple subscriptions can ride one dialog (RFC 6665 §4.5.2). The
`NotifyBuilder::for_subscription(SubscriptionId)` setter targets a
specific subscription; when omitted, the single-subscription-on-dialog
default applies.

**Implementer verification required.** Today's
`UnifiedDialogApi::send_notify` (`unified.rs:1449`) takes a
`DialogId`, not a `SubscriptionId`. Phase B / C must verify whether
dialog-core's subscription manager already accepts a subscription
identifier on the NOTIFY path:

- If it does, `NotifyRequestOptions.subscription_id` rides through as
  an optional field with no dialog-core API change.
- If it does not, Phase B adds a subscription-id parameter to
  `send_notify_with_options` and threads it into the subscription
  manager. This is the only Phase B / C item whose scope cannot be
  fully predicted from the design alone.

`SubscribeRefreshBuilder` is reached from `SubscriptionHandle::refresh()`,
sets `SubscribeRequestOptions.refresh = true`, and dispatches via
`Action::SendSUBSCRIBEWithOptions` (which inspects the flag and
routes to dialog-core's refresh path). `RegisterRefreshBuilder`
follows the same pattern via `RegistrationHandle::refresh()`.

---

## 13. Performance and memory budget

The new API retains more data per session than the legacy flat-method
API. This section quantifies the cost and identifies the trade-offs.

### 13.1 Per-session retention

| Retention | Source | Lifetime | Approx. size |
|---|---|---|---|
| `Option<Arc<rvoip_sip_core::Request>>` | `IncomingCall.request` | Until session terminates | `Some` by default — one typed Request per call (~4–8 KB for INVITE); `None` under the lean-mode feature flag (§13.3) |
| `Arc<bytes::Bytes>` | Cross-crate bus events for mid-dialog requests / responses | Until subscribers drop | Original inbound bytes (~1–4 KB per message) |
| `Arc<XxxRequestOptions>` | `SessionState.pending_*_options` | One per outbound request, cleared after final response | 200 B – 4 KB depending on `extra_headers` content |

### 13.2 Cost model

- **Cheap:** `Arc::clone` for cross-thread sharing.
- **One-time:** parse cost on inbound (already happens today).
  Phase E preserves the original `Bytes` from the inbound parse and
  re-parses them once when constructing `IncomingRequest`. No
  serialize-and-re-parse round-trip; the inbound parse is the only
  parse.
- **Per-call:** `HeaderPolicy::validate_outbound` is an O(staged_headers)
  scan over a small `Vec<TypedHeader>` (typically < 10 entries).

### 13.3 Lean-mode feature flag (planned, not in initial phases)

For high-throughput PBX deployments with thousands of concurrent
sessions, retaining `Arc<Request>` on every `IncomingCall` may be
unwanted. A future `cargo` feature `no-incoming-request-retention`
will:

- Skip the `IncomingCall.request` field (set to `None`).
- Drop `Arc<Bytes>` on cross-crate variants once they're consumed.

This is out of scope for the initial implementation but the
`Option<Arc<Request>>` field shape leaves room for it without
breaking the API.

### 13.4 Allocation hot-paths

- `SipHeaderView::headers_named` returns a boxed iterator
  (object-safe). For zero-alloc iteration, concrete inbound types
  expose `headers_named_iter()` returning `impl Iterator<Item = &TypedHeader>`.
- `with_headers_from` allocates a `Vec<HeaderName>` for the report's
  `skipped` field. Applications calling this on every inbound request
  should reuse a buffer or drop the report if not needed.

---

## 14. Out of scope

- ~~Removing deprecated methods. They stay through at least version
  0.3.0; a separate breaking-change PR removes them later.~~
  **Brought forward in the 2026-05-18 revision** — `register_with`,
  `subscribe_dialogs` / `unsubscribe_dialogs`, and the corresponding
  surface forwarders were removed outright rather than `#[deprecated]`-
  wrapped. The workspace `deprecated = "allow"` lint setting was also
  removed.
- ~~Migrating in-tree examples and tests onto the builder API. Follow-up
  sweep PR once the builders are stable.~~ **Brought forward in the
  2026-05-18 revision** — `examples/pbx/common.rs`,
  `examples/stream_peer/04_registration/client.rs`, and the ~20
  call sites in `tests/register_423_retry.rs` +
  `tests/generated_sip_compliance.rs` now use the explicit builder
  chain.
- ~~Migrating `rvoip-sip-registrar` onto `IncomingRegister` /
  `RegisterResponseBuilder`. The registrar crate today reads inbound
  REGISTER directly; the new types are additive, so registrar
  continues to compile and run unchanged. Migration is a follow-up PR.~~
  **Audited in the 2026-05-18 revision** — registrar calls no
  legacy dialog-core methods (it never did); inbound REGISTER reads
  remain a follow-up if the new typed surface is desired.
- New SIP methods beyond INVITE / re-INVITE / REGISTER / REFER / BYE
  / CANCEL / NOTIFY / SUBSCRIBE / INFO / UPDATE / MESSAGE / OPTIONS.
  The trait shape makes adding PUBLISH trivial later; not in this
  release.
- Stateless SIP proxy mode (forwarding requests with `Via` push/pop
  only, no dialog or session creation). Stateful B2BUA proxying
  (each leg is a session, both legs bridged) is in scope and is the
  primary use case this design serves.
- Changes to `rvoip-sip-core` or `rvoip-sip-transport`.
- Changes to dialog-core's dialog state machine, route-set logic,
  transaction core, or CSeq management. Dialog-core changes are
  strictly additive options structs and `*_with_options` methods.
- Re-architecting media or RTP. SDP/RTP are unaffected.

---

## 15. Design decisions (record)

- **Dialog-core extends additively.** New `*RequestOptions` structs
  + `send_*_with_options` methods on `UnifiedDialogApi`. Existing
  methods stay and delegate. Dialog-core remains authoritative over
  dialog/transaction headers; application headers ride alongside.
- **`with_header` returns `Result`.** Forced acknowledgement of
  policy at the call site; `?` chains cleanly. No silent drops, no
  debug-only checks. The `Err` value names the dedicated setter when
  one exists.
- **Unified state-machine dispatch — in-dialog only.** The eight
  in-dialog outbound methods route through `Action::Send*WithOptions`
  for one observability path, one auto-emit mechanism, one stash
  lifecycle. The four out-of-dialog methods (OPTIONS, MESSAGE, initial
  REGISTER, initial SUBSCRIBE) call `DialogAdapter` mirror methods
  directly. The `extra_headers` plumbing is identical on both paths,
  so the wire-output contract is preserved; only the dispatch entry
  point differs. The carve-out avoids creating ephemeral `SessionState`
  entries solely for transaction tracking, which would add per-request
  allocation overhead without observable benefit.
- **`ClearPending<Method>Options` YAML rows are not authored.** The
  eleven non-INVITE `Action::Send*WithOptions` handlers consume the
  stash with `.take()` semantics on dispatch, and the executor's
  `Terminated` backstop sweeps any residue on session teardown.
  Adding explicit `ClearPending*` action rows to YAML on final-response
  transitions would today be no-ops. These rows become load-bearing
  only once auth-retry is generalized beyond INVITE / REGISTER and the
  handlers switch from `.take()` to `.clone()` semantics; the YAML
  authoring lands as part of that future generalization, not now.
- **Generic surface adapter.** `SurfaceBuilder<B, S: Surface>`
  collapses 4 surfaces × 12 builders into one generic adapter. New
  builders extend automatically.
- **Preserved original inbound bytes.** Cross-crate event payload is
  the *original* `Bytes` from the inbound parse. No
  serialize-and-re-parse round-trip.
- **Single auto-emit mechanism.** `Config.auto_emit_extra_headers`
  is the one knob. Per-session per-method overrides ride the normal
  stash via a pre-emptive builder call.
- **CallHandler trait deprecation, not duplication.** Legacy
  positional-arg methods get `#[deprecated]` and a default-impl
  forwarder; canonical names take typed `IncomingRequest`. Scheduled
  for removal in next breaking release, not parallel-kept forever.
- **Bus stays SIP-agnostic.** `infra-common` carries `Arc<bytes::Bytes>`,
  not `Arc<rvoip_sip_core::Request>`. Foundation-crate isolation
  preserved.
- **Compile-time prevention rejected.** Wrapping `TypedHeader` in a
  newtype that excludes stack-managed variants is impractical given
  `TypedHeader::Other(_)` carry-through. Runtime policy with `Result`
  return is the pragmatic choice.
- **Full RFC 3261 + key extensions in this design.** INVITE,
  re-INVITE, REGISTER, REFER, BYE, CANCEL, NOTIFY, SUBSCRIBE, INFO,
  MESSAGE, OPTIONS, UPDATE. Comprehensive for SBC / B2BUA /
  call-center use cases.
- **`AuthChallengeBuilder` is in scope.** Wraps sip-core's existing
  `www_authenticate_digest` / `bearer` helpers for typed 401/407
  authoring — needed by registrars and B2BUA auth-relay code.

---

## 16. Revision log

### 2026-05-20 — `SessionHandle` in-dialog method ergonomics

Added `SessionHandle::{bye, cancel, refer, notify, info, update,
reinvite}` so application code reaches in-dialog requests directly
from the session handle it already holds:

```rust
// Before
coord.bye(session.id()).with_reason(reason).send().await?;
// After
session.bye().with_reason(reason).send().await?;
```

Matches the existing `SessionHandle::hangup()` / `transfer_blind()`
shape — no setup args, pulls `call_id` from `&self`. The
coordinator-keyed entries (`coord.bye(&session_id, …)`) remain
available for code that holds only a `CallId`.

**Inspector rename to free `info`.** The existing
`SessionHandle::info() -> Result<SessionInfo>` (state inspector) was
renamed to `session_info()` because the new SIP `info(content_type)`
builder needed the slot. Single in-tree caller (a doc example)
migrated; external callers see a one-name rename with the same
return type.

**Surface forwarders deliberately not added.** §4's earlier wording
implied `Endpoint`/`StreamPeer`/`CallbackPeer` would project every
in-dialog verb. Audit found callers don't typically hold the peer
at the moment they want to send an in-dialog request — they hold
the `SessionHandle` returned from `invite().send().await` or
delivered to the inbound callback. §3.3 and §4 updated to make
`SessionHandle::<verb>` the canonical in-dialog entry shape.

**Gap D-1 closed.** `IncomingCall::accept` and
`IncomingCall::accept_with_sdp` now route through
`self.accept_builder().send()` instead of the legacy
`coordinator.accept_call(...)` direct path. Completes the §11.4
migration intent — one code path for INVITE acceptance.

### 2026-05-18 — Transport `raw_bytes` end-to-end

The transport layer now preserves the parser's input buffer on
`TransportEvent::MessageReceived.raw_bytes: Option<Arc<Bytes>>`,
populated by every shipping transport (UDP/TCP/TLS/WS). Dialog-core's
transaction manager caches the bytes per-transaction key
(`pending_inbound_bytes`), and the cross-crate bridges in
`events/event_hub.rs`, `events/adapter.rs`,
`protocol/register_handler.rs`, and `manager/protocol_handlers.rs`
source those bytes via `take_inbound_bytes` instead of
re-serialising via `Message::to_bytes()` / `Request::to_string()`.

This closes the §7.5 promise to ship original wire bytes
end-to-end. Concrete outcome: **STIR/SHAKEN Identity-header
signatures (RFC 8224) now validate** — applications consuming
`IncomingCall::raw_request` see the upstream signer's exact
canonical form, not the normalised reprint sip-core's `Display`
impl produced. Synthetic / mock-transport paths fall back to
re-serialisation (`raw_bytes: None`).

Companion: §7.6 adds `Transport::send_message_raw(bytes, dest)` so
applications can ship pre-built byte buffers to the wire without
round-tripping through the typed AST. Unlocks signature-preserving
SBC pass-through, stateless proxy forwarding, fuzz harnesses, and
replay tooling.

### 2026-05-18 — Migration finalized; deferrals brought forward

This revision brings forward §14 deferrals and removes legacy entries
that previous revisions left behind with `#[deprecated]` intent (which
the workspace's `[workspace.lints.rust] deprecated = "allow"` had been
masking). Net effect: workspace builds `--all-features --workspace`
green with zero deprecation warnings.

**Workspace surgery.**
- `crates/orchestration-core` moved from `members` to `exclude` in
  the root `Cargo.toml`. The crate still depends on `make_call` and
  `reject_call` on `UnifiedCoordinator` — entries removed in an
  earlier revision without migrating callers. Re-include once the
  orchestrator is migrated.

**rvoip-sip surface removals (no deprecation grace period).**
- `UnifiedCoordinator::subscribe_dialogs` and
  `UnifiedCoordinator::unsubscribe_dialogs` removed. The new entry is
  `coord.subscribe(target, "dialog").send_dialog_events().await?`
  returning a `DialogSubscriptionHandle` that wraps the generic
  `SubscriptionHandle` (`src/api/send/subscribe.rs`).
- `PeerControl::subscribe_dialogs`, `StreamPeer::subscribe_dialogs`
  removed. Use the builder via `coord.subscribe(...)`.
- `CallbackPeerControl::register_with`, `CallbackPeer::register_with`,
  `PeerControl::register_with` removed. `peer.register(registrar,
  user, pw).with_expires(..).with_from_uri(..).with_contact_uri(..).send()`
  is the canonical path. `PeerControl::register` and
  `CallbackPeerControl::register` builder entries added.
- `Endpoint::register` body rewritten to use the builder chain
  directly (no longer delegates to `register_with`).
- `DialogAdapter::send_dialog_subscribe` and
  `DialogAdapter::unsubscribe_dialog_package` removed (callers gone).

**rvoip-sip → dialog-core wire migration.** `DialogAdapter` now
calls dialog-core's `*_with_options` exclusively. Migrated sites:
- `send_bye` → `send_bye_with_options` (2 sites)
- `send_update` (via re-INVITE adapter) → `send_update_with_options`
- `send_refer` → `send_refer_with_options` (3 sites)
- `send_cancel` → `send_cancel_with_options`
- `send_info_with_content_type` → `send_info_with_options`
- `send_subscribe_out_of_dialog` → `send_subscribe_with_options`
- `send_subscribe_refresh` → `send_subscribe_refresh_with_options`
- `send_notify` → `send_notify_with_options`
- `send_message_out_of_dialog` → `send_message_out_of_dialog_with_options`
- `send_bye_with_reason` (via `dialog_manager().core()`) →
  `send_bye_with_options { reason: Some(reason.to_string()), .. }`

The `send_refer(dialog, target, refer_body)` call site that
previously passed the literal string `"attended"` as the REFER body
was dropped — the legacy code had no on-wire effect (attended
transfers belong in the Refer-To header's Replaces parameter, not the
body). Attended transfers now route through the proper
`send_refer_with_options` paths that populate
`ReferRequestOptions.replaces`.

**Examples and tests migration (§14 deferral, brought forward).**
- `examples/pbx/common.rs:1053, 1086` — explicit builder chain
- `examples/stream_peer/04_registration/client.rs:23` — explicit
  builder chain (no `Registration` struct needed)
- `tests/register_423_retry.rs` — 9 call sites rewritten by paren-
  balanced script
- `tests/generated_sip_compliance.rs:276` — explicit builder chain

**Workspace lint.** `[workspace.lints.rust] deprecated = "allow"`
removed. The workspace now surfaces deprecation warnings if any
future caller drifts back to a deprecated method, instead of hiding
them.

**What was NOT removed (out of scope for this revision).** Dialog-core
parallel handle APIs (`DialogHandle` in `api/common.rs`,
`DialogClient` in `api/client.rs`, `CallHandle` in dialog-core)
go directly to the manager layer with their own legacy method
shape. External callers (rvoip-sip) no longer touch any legacy path.
Removing dialog-core's internal parallel handles is a separate
refactor (Gap 2 / Gap 3 in the post-shipping audit).

### 2026-05-18 (later) — Gap 1: UnifiedDialogApi legacy method removal

After confirming the workspace migration in the previous revision
left dialog-core's `UnifiedDialogApi` with a half-old/half-new
public surface, this addendum removed the 12 legacy methods that
no rvoip-sip caller could reach.

**Methods removed from `UnifiedDialogApi`** (`src/api/unified.rs`):
- `send_bye(dialog_id)`
- `send_refer(dialog_id, target, body)`
- `send_notify(dialog_id, event, body, sub_state)`
- `send_update(dialog_id, sdp)`
- `send_reinvite(dialog_id, sdp)`
- `send_info(dialog_id, body)`
- `send_info_with_content_type(dialog_id, ct, body)`
- `send_cancel(dialog_id)`
- `send_subscribe_out_of_dialog(target, from, contact, event, expires)`
- `send_subscribe_out_of_dialog_for_session(session, target, from, contact, event, expires)`
- `send_subscribe_refresh(dialog_id, event, expires)`
- `send_message_out_of_dialog(target, from, body)`

**Kept** (internal infrastructure backing the `*_with_options` API):
- `send_subscribe_out_of_dialog_with_extras` — called by
  `send_subscribe_with_options`
- `send_message_out_of_dialog_with_extras` — called by
  `send_message_out_of_dialog_with_options`
- `send_refer_notify(dialog_id, status, reason)` — REFER-progress
  NOTIFY (RFC 3515 §2.4.5), special-case, not a UAC method

**Dialog-core's own callers migrated:**
- `tests/unified_api_tests.rs:539-570` — 5 sites migrated to
  `*_with_options` family.
- `examples/global_events_test.rs:159, 174` — `send_info` /
  `send_update` migrated.
- `examples/phase3_integration_showcase.rs:200-312` — 7 sites
  migrated. (`self.client_api` here is `Arc<UnifiedDialogApi>`,
  not `DialogClient`.)

`tests/phase3_integration_tests.rs` and `unified_api_tests.rs` calls
through `env.client: Arc<DialogClient>` — Layer B — are unchanged.
DialogClient is a separate parallel surface and was not in Gap 1
scope.

**Verification.** `cargo build --workspace --all-features` clean;
`cargo test --all-features -p rvoip-sip-dialog` 280+ tests pass;
`cargo test --all-features -p rvoip-sip` all green.

**Verification.** `cargo build --workspace --all-features` clean.
`cargo test --all-features -p rvoip-sip --no-run` clean. Workspace
metadata excludes orchestration-core. No `register_with` or
`subscribe_dialogs` references remain in `crates/rvoip-sip{,-dialog,-registrar}/src/`.
