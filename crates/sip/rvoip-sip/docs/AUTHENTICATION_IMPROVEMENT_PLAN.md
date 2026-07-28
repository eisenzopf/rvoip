# rvoip-sip Authentication Improvement Plan

## Summary

`rvoip-sip` already has real SIP Digest authentication support for the most
important PBX flows: REGISTER, challenged outbound INVITE, and several
in-dialog retry paths. The Asterisk and FreeSWITCH examples prove those flows
work, but they also show that developers must understand too many separate
concepts: `EndpointAccount`, `Registration`, `Config.credentials`, auth
usernames, contact overrides, and challenge retry behavior.

The goal is to make SIP authentication boring and obvious:

- one account/auth configuration model that can be reused across public
  surfaces;
- consistent `with_credentials` behavior across outbound builders;
- no fake or partial `Authorization` headers;
- reusable server-side digest helpers backed by `rvoip-auth-core`;
- docs and tests that match the real supported behavior.

## Implementation Status

Implemented in the authentication-improvement change:

- `SipAccount` is available as the shared account/auth model and converts to
  `Registration`, `EndpointAccount`, and `Credentials`.
- `EndpointBuilder::sip_account`, `StreamPeer::register_account`,
  `PeerControl::register_account`, `CallbackPeer::register_account`, and
  `CallbackPeerControl::register_account` derive registration from one account.
- MESSAGE, OPTIONS, and SUBSCRIBE `with_credentials` use real one-shot digest
  retry after 401/407; they no longer emit partial digest headers or discard
  credentials.
- `SipDigestAuthService` exposes server-side digest challenge and validation
  helpers through `rvoip_sip::auth`.
- `rvoip-auth-core` implements SHA-512-256 and SHA-512-256-sess digest
  algorithms instead of falling back to MD5.
- Unknown digest algorithm tokens now fail clearly instead of downgrading to
  MD5; omitted algorithm still defaults to MD5 for legacy compatibility.
- UAC retry supports `401`/`407`, `Authorization`/`Proxy-Authorization`,
  qop `auth`, qop `auth-int` when a request body is available, and exactly one
  `stale=true` recovery retry with a fresh nonce.
- `SipDigestAuthService` rejects realm mismatch, invalid qop, malformed
  nonce-count, and nonce-count replay per `(username, nonce, cnonce)`, and returns a
  stale challenge for expired issued nonces.

The original assessment below is retained as design history.

## Pre-Implementation Assessment

### What Worked Before This Change

- PBX REGISTER flows work in the interop examples. `examples/pbx/common.rs`
  sets `Config.credentials`, builds `Registration`, and builds
  `EndpointAccount` from the same endpoint config.
- `Endpoint` account registration is easy once `EndpointAccount` has been
  configured.
- `StreamPeer` and `CallbackPeer` registration work through
  `register(registrar, username, password).send()`.
- Challenged INVITE auth retry works through `Config.credentials` or
  per-call `with_credentials`.
- In-dialog BYE, REFER, INFO, UPDATE, and NOTIFY auth retry is wired through
  `Action::SendRequestWithAuth`.
- `rvoip-auth-core` owns digest challenge parsing, response computation,
  nonce-count support, qop handling, and server-side validation primitives.

### Original Problems To Fix

- The account model was fragmented. PBX examples manually translated one logical
  account into `Config.credentials`, `Registration`, and `EndpointAccount`.
- `MessageBuilder::with_credentials` and `SubscribeBuilder::with_credentials`
  created only `Digest username="..."`, which is not a valid digest
  authorization response.
- `OptionsBuilder::with_credentials` accepted credentials but discarded them.
- The lower layers already contain auth retry dispatch for MESSAGE, OPTIONS,
  and SUBSCRIBE, but the public builders bypassed that path.
- Server-side auth was not first-class in `rvoip-sip`. Applications could send
  challenges, but validation required direct `rvoip-auth-core` usage and raw
  header plumbing.
- `rvoip_sip::auth` re-exports only client-side digest helpers, not the
  server-side authenticator and parsed response types.
- `Credentials::realm` is misleading. Digest retry uses the realm from the
  received challenge; the credentials realm is not authoritative.
- Top-level docs claimed SHA-512-256 digest support while the original
  `auth-core` digest code recognized that token and fell back to MD5.

## Design Principles

- Challenge-response auth must be stack-managed. Application code should
  provide credentials, not hand-author `Authorization`.
- A method named `with_credentials` must either produce a real challenge
  response when challenged or fail clearly. It must never emit fake digest
  headers or silently ignore credentials.
- High-level account setup should configure both registration and challenged
  outbound request auth.
- Low-level auth-core primitives should remain available for advanced users,
  but normal SIP applications should not need to manually parse or format
  digest headers.
- Existing public APIs should remain source-compatible where possible.

## Proposed Public API Changes

### `SipAccount`

Add a canonical account type in `rvoip-sip`:

```rust
pub struct SipAccount {
    pub registrar: String,
    pub username: String,
    pub auth_username: Option<String>,
    pub password: String,
    pub from_uri: Option<String>,
    pub contact_uri: Option<String>,
    pub expires: u32,
}
```

Add builder-style methods:

```rust
impl SipAccount {
    pub fn new(
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self;

    pub fn auth_username(self, username: impl Into<String>) -> Self;
    pub fn from_uri(self, uri: impl Into<String>) -> Self;
    pub fn contact_uri(self, uri: impl Into<String>) -> Self;
    pub fn expires(self, seconds: u32) -> Self;

    pub fn effective_auth_username(&self) -> &str;
    pub fn credentials(&self) -> Credentials;
    pub fn registration(&self) -> Registration;
    pub fn endpoint_account(&self) -> EndpointAccount;
}
```

Use `SipAccount` as the canonical account object for:

- `EndpointBuilder::sip_account(account)`;
- PBX examples;
- test fixtures that need a registered account;
- future config-file driven account setup.

Keep `EndpointAccount` for compatibility and implement conversions:

```rust
impl From<SipAccount> for EndpointAccount;
impl From<EndpointAccount> for SipAccount;
```

`EndpointAccount` should become a compatibility wrapper around the same
semantic fields, not a separate concept developers have to learn first.

### StreamPeer and CallbackPeer Account Helpers

Add convenience helpers that consume `SipAccount`:

```rust
impl StreamPeer {
    pub fn register_account(&self, account: &SipAccount) -> RegisterBuilder;
}

impl PeerControl {
    pub fn register_account(&self, account: &SipAccount) -> RegisterBuilder;
}

impl CallbackPeerControl {
    pub fn register_account(&self, account: &SipAccount) -> RegisterBuilder;
}
```

These helpers should apply registrar, auth username, password, expires, From
URI, and Contact URI exactly once.

### Consistent `with_credentials`

Keep existing `with_credentials` methods, but make their behavior consistent:

- initial request goes without `Authorization`;
- a 401/407 challenge is parsed by the stack;
- a full digest response is computed by `rvoip-auth-core`;
- retry uses `Authorization` or `Proxy-Authorization`;
- application-staged headers survive the retry.

This must apply to:

- INVITE;
- REGISTER;
- BYE, REFER, NOTIFY, INFO, UPDATE;
- MESSAGE;
- OPTIONS;
- SUBSCRIBE.

If a method cannot support this behavior, its builder must not expose
`with_credentials`.

### Server-Side Digest Facade

Add a first-class server auth helper in `rvoip-sip`, backed by
`rvoip-auth-core`:

```rust
pub struct SipDigestAuthService { ... }

pub enum AuthDecision {
    Authorized {
        username: String,
        realm: String,
    },
    Rejected {
        challenge: DigestChallenge,
        www_authenticate: String,
    },
}

impl SipDigestAuthService {
    pub fn new(realm: impl Into<String>) -> Self;
    pub fn with_algorithm(self, algorithm: DigestAlgorithm) -> Self;
    pub fn add_user(&self, username: impl Into<String>, password: impl Into<String>);
    pub fn challenge(&self) -> DigestChallenge;
    pub fn www_authenticate(&self, challenge: &DigestChallenge) -> String;

    pub fn validate_authorization(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> Result<AuthDecision>;

    pub fn authenticate_authorization(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> Result<AuthDecision>;
}
```

The `Rejected` decision should intentionally include a fresh challenge so
callers can respond without constructing raw `WWW-Authenticate` strings.

### Inbound Auth Convenience

Add helpers on inbound surfaces:

```rust
impl IncomingRegister {
    pub async fn authenticate_with(
        &self,
        auth: &SipDigestAuthService,
    ) -> Result<AuthDecision>;
}

impl IncomingRequest {
    pub async fn authenticate_with(
        &self,
        auth: &SipDigestAuthService,
    ) -> Result<AuthDecision>;
}
```

Behavior:

- `IncomingRegister` uses its `Authorization` field and REGISTER request URI.
- `IncomingRequest` reads `Authorization` from the parsed request.
- If a parsed request is available, use the parsed request URI and body.
- If an inbound REGISTER is synthetic, use the exposed `to_uri` as the request
  URI and no body.
- Missing auth returns `AuthDecision::Rejected` with a fresh challenge.

### Challenge Builder Integration

Add challenge-builder helpers that accept an auth-core challenge:

```rust
impl AuthChallengeBuilder {
    pub fn with_digest_challenge(self, challenge: &DigestChallenge) -> Self;
}

impl RegisterResponseBuilder {
    pub fn with_digest_challenge(self, challenge: &DigestChallenge) -> Self;
}
```

These should populate realm, nonce, algorithm, qop, opaque, and stale where
available, avoiding raw string construction in application code.

## Implementation Plan

### Phase 1: Account Model

1. Add `SipAccount`.
2. Add conversions between `SipAccount` and `EndpointAccount`.
3. Add `EndpointBuilder::sip_account`.
4. Add `register_account` helpers to `StreamPeer`, `PeerControl`, and
   `CallbackPeerControl`.
5. Refactor PBX examples to create a `SipAccount` once and derive:
   - default `Config.credentials`;
   - `Registration`;
   - `EndpointAccount`.
6. Add tests for conversion and auth-username behavior.

### Phase 2: Outbound Builder Auth Semantics

1. Remove fake digest header generation from MESSAGE and SUBSCRIBE.
2. Remove credential discard behavior from OPTIONS.
3. Route MESSAGE, OPTIONS, and SUBSCRIBE builders through the state machine:
   - create a UAC session;
   - store `session.credentials` when credentials were provided;
   - set `session.remote_uri` to the request target;
   - stage the matching pending options slot;
   - dispatch `SendOutboundMessage`, `SendOutboundOptions`, or
     `SendOutboundSubscribe`.
4. Add default state-table rows:
   - `Idle + SendOutboundMessage -> SendMESSAGEWithOptions`;
   - `Idle + SendOutboundOptions -> SendOPTIONSWithOptions`;
   - `Idle + SendOutboundSubscribe -> SendSUBSCRIBEWithOptions`.
5. Preserve current response-returning APIs where required:
   - MESSAGE can remain `Result<()>`;
   - SUBSCRIBE must still return a `SubscriptionHandle`;
   - OPTIONS must still return `IncomingResponse`.
6. If returning synchronous responses through the state-machine path is too
   invasive, add a small coordinator-side direct retry helper instead:
   - first send directly without auth;
   - if 401/407, compute a real digest response and retry through the
     existing `send_*_oob_with_auth` methods;
   - never emit fake digest.

Recommended implementation: use the coordinator-side direct retry helper first
for MESSAGE, OPTIONS, and SUBSCRIBE, then migrate to state-table dispatch if a
future lifecycle requirement needs it. This preserves current return values and
minimizes churn while still satisfying the auth contract.

### Phase 3: Server-Side Auth Service

1. Add `SipDigestAuthService` and `AuthDecision` under `rvoip-sip::auth`.
2. Re-export from crate root and prelude.
3. Add inbound `authenticate_with` helpers.
4. Add challenge-builder `with_digest_challenge` helpers.
5. Update registrar examples and tests to use the service where appropriate.

### Phase 4: auth-core Alignment

1. Add `DigestAuthenticator::with_algorithm(DigestAlgorithm)`.
2. Implement `SHA-512-256` and `SHA-512-256-sess` in `DigestAlgorithm`, or
   remove the public docs claim.
3. Preferred: implement SHA-512-256 using `sha2::Sha512_256`.
4. Update parser tests so SHA-512-256 no longer falls back to MD5.
5. Update docs to state the exact supported algorithms.

### Phase 5: Docs and Cleanup

1. Update `SECURITY_POSTURE.md`.
2. Update `COMPATIBILITY_MATRIX.md`.
3. Update PBX docs to show `SipAccount` as the canonical account setup.
4. Update `state-machine-wiring.md` if MESSAGE/OPTIONS/SUBSCRIBE routing
   changes.
5. Remove or rewrite stale test-helper comments that say non-INVITE auth retry
   does not exist.

## Detailed Behavior Requirements

### REGISTER

- Preserve existing REGISTER behavior.
- `SipAccount::registration()` must use `auth_username` when present.
- `SipAccount::registration()` must preserve From URI, Contact URI, and
  Expires.
- Registration auth retry must continue using the challenge realm and nonce.

### INVITE

- Preserve existing INVITE behavior.
- `SipAccount::credentials()` must configure default challenged INVITE auth.
- Per-call `with_credentials` remains the override.

### In-Dialog Requests

- Preserve existing BYE, REFER, NOTIFY, INFO, and UPDATE auth retry behavior.
- Replace `MissingCredentialsForInviteAuth` usage in generic request auth
  paths with a method-neutral error name.
- Keep retry cap at one authenticated retry, except allow one additional
  retry when a second challenge carries `stale=true` with a fresh nonce.

### MESSAGE / OPTIONS / SUBSCRIBE

- First request must not carry `Authorization` unless the caller explicitly
  provides a precomputed auth value through a dedicated advanced API.
- On 401, use `WWW-Authenticate` and retry with `Authorization`.
- On 407, use `Proxy-Authenticate` and retry with `Proxy-Authorization`.
- MESSAGE body must be included for `qop=auth-int`.
- OPTIONS has no body.
- SUBSCRIBE must preserve target, event package, Accept, Contact, Expires, and
  staged extra headers across retry.
- Application-supplied headers must survive retry.

### Server-Side Validation

- Missing authorization should produce `AuthDecision::Rejected` with a fresh
  challenge.
- Unknown user should produce `Rejected`, not reveal whether the username
  exists.
- Wrong password should produce `Rejected`.
- Valid response should produce `Authorized`.
- Request URI mismatch should reject.
- `qop=auth-int` should validate with the request body.

## Test Plan

### Unit Tests

- `SipAccount::new` defaults expires to 3600.
- `SipAccount::effective_auth_username` returns username when no auth username
  is set.
- `SipAccount::credentials` uses auth username when present.
- `SipAccount::registration` carries registrar, auth username, password,
  expires, From URI, and Contact URI.
- `EndpointAccount -> SipAccount -> EndpointAccount` preserves all fields.
- `DigestAuthenticator::with_algorithm` changes generated challenge algorithm.
- SHA-512-256 parser and digest computation work.

### Builder Tests

- `MessageBuilder::with_credentials` no longer emits
  `Digest username="..."` on the first request.
- `SubscribeBuilder::with_credentials` no longer emits
  `Digest username="..."` on the first request.
- `OptionsBuilder::with_credentials` no longer discards credentials.
- Authorization remains blocked through generic `with_header` and points users
  to dedicated auth APIs.

### Integration Tests

- REGISTER 401 -> digest retry -> 200 with auth username different from AOR
  username.
- INVITE 401 -> digest retry -> 200 using account default credentials.
- INVITE 407 -> proxy digest retry -> 200.
- MESSAGE 401 -> digest retry -> 200.
- SUBSCRIBE 401 -> digest retry -> 200.
- OPTIONS 401 -> digest retry -> 200.
- MESSAGE `qop=auth-int` includes the MESSAGE body in digest computation.
- MESSAGE / OPTIONS / SUBSCRIBE 407 retry uses `Proxy-Authorization`.
- OOB stale nonce recovery retries once with the fresh nonce and resets
  nonce-count.
- Unsupported digest algorithms fail without sending an invalid retry.
- BYE, REFER, and INFO auth retry tests continue to pass.

### Server Auth Tests

- `SipDigestAuthService` returns `Rejected` for missing auth.
- `SipDigestAuthService` validates a correct digest response.
- Wrong password rejects.
- Unknown user rejects without revealing existence.
- Request URI mismatch rejects.
- Realm mismatch rejects.
- Expired issued nonce returns a `stale=true` challenge.
- Nonce-count replay rejects even when the cnonce changes.
- `qop=auth-int` validates with body.
- Challenge builders can be populated from a generated digest challenge.

### PBX Interop Tests

- Asterisk registration works through Endpoint.
- Asterisk registration works through StreamPeer.
- Asterisk registration works through CallbackPeer.
- FreeSWITCH registration works through Endpoint.
- FreeSWITCH registration works through StreamPeer.
- FreeSWITCH registration works through CallbackPeer.
- A challenged outbound call after registration succeeds using account
  credentials.

## Acceptance Criteria

- A developer can configure a PBX account once and use it for registration and
  challenged outbound calls.
- No public `with_credentials` method emits fake or partial digest headers.
- No public builder silently ignores credentials.
- Server-side challenge and validation can be implemented without raw
  `WWW-Authenticate` string construction.
- Docs accurately describe supported digest algorithms.
- Existing Asterisk and FreeSWITCH interop examples continue to pass.
- Existing REGISTER, INVITE, BYE, REFER, INFO auth retry tests continue to
  pass.

## Compatibility Notes

- Keep `EndpointAccount` public and source-compatible.
- Keep `Credentials::new` and `Credentials::with_realm` for now.
- Document `Credentials::realm` as advisory/deprecated for new code because
  challenge realm is authoritative.
- Do not remove existing `register(registrar, username, password)` builders.
- New `SipAccount` helpers should be additive.

## Risks

- Routing MESSAGE/OPTIONS/SUBSCRIBE through the state machine may affect current
  APIs that return immediate responses. If this becomes too invasive, implement
  direct challenged retry helpers first.
- SUBSCRIBE handle construction currently depends on the response Call-ID. Any
  state-machine migration must preserve that behavior.
- Changing SHA-512-256 behavior from fallback to real support may expose
  previously hidden interop differences; add focused tests.
- Server-side validation must avoid leaking user-existence information.

## Recommended Implementation Order

1. Add `SipAccount` and conversion tests.
2. Add `SipDigestAuthService` and server-side tests.
3. Fix SHA-512-256 support or correct the docs claim.
4. Fix MESSAGE/SUBSCRIBE/OPTIONS `with_credentials` behavior.
5. Refactor PBX examples to use `SipAccount`.
6. Update docs and stale test comments.
7. Run focused auth tests, then PBX interop tests.
