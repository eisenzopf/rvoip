# Complete RVoIP Auth And User Service Plan

## Required First Artifact

Before code changes, create this Markdown plan in:

`/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/docs/COMPLETE_AUTH_USER_SERVICE_PLAN.md`

The file should contain this complete plan and serve as the reviewable source of truth for the implementation.

## Summary

Mature `rvoip-users-core` as the optional first-party auth/user database, while keeping RVoIP protocol crates provider-based so developers can use users-core, an external IdP, LDAP/AD, IMS infrastructure, a custom database, or their own service.

Responsibilities:

- `rvoip-auth-core`: common auth validation contracts and crypto/token primitives.
- `rvoip-users-core`: optional reference user/auth service with storage, passwords, API keys, JWT issuance, and SIP credential material.
- `rvoip-sip`: SIP auth header negotiation, retry, challenge, and UAS orchestration.
- External services: supported through stable traits and concrete validators/adapters.

## Key Design Changes

- Add `rvoip-users-core` to workspace dependencies, but do not make it a mandatory dependency of `rvoip-sip`.

- Define provider contracts in `rvoip-auth-core`:
  ```rust
  pub trait BearerValidator;          // existing
  pub trait PasswordVerifier;         // Basic/password validation
  pub trait DigestSecretProvider;     // SIP Digest HA1/plain secret lookup
  pub trait ApiKeyVerifier;           // API key validation where protocols need it
  ```

- Stabilize users-core extension points:
  - Document `UserStore` and `ApiKeyStore`.
  - Keep SQLite as the built-in reference store.
  - Let external systems either implement users-core storage traits or skip users-core and implement auth-core provider traits directly.

- Add a feature-gated users-core bridge:
  ```rust
  // feature = "auth-core"
  pub struct UsersCoreAuthProvider {
      auth_service: Arc<AuthenticationService>,
  }
  ```
  It implements Bearer JWT validation, password verification, API key verification, and SIP Digest secret lookup.

- Add users-core SIP Digest credential storage:
  - Store HA1 material per user, SIP username, realm, and algorithm family.
  - Do not reuse Argon2 login password hashes for SIP Digest.
  - Add create, rotate, delete, and lookup APIs.

- Fix users-core maturity gaps:
  - Make `UsersConfig::from_env` real or remove the claim.
  - Fix JWT/JWKS behavior for HS256 and RS256.
  - Document production signing key requirements.
  - Update examples that manually decode JWTs to use auth-core validators.

- Extend `rvoip-sip::SipAuthService`:
  - Keep `with_bearer_validator(...)`.
  - Add `with_basic_verifier(realm, Arc<dyn PasswordVerifier>)`.
  - Add `with_digest_provider(realm, Arc<dyn DigestSecretProvider>)`.
  - Keep in-memory Basic/Digest users as test/local shorthands.
  - Add UAS helpers that authenticate or emit correct 401/407 challenges without boilerplate.

## Supported Services Roadmap

First-class in this implementation:

- users-core SQLite user/auth service.
- users-core JWT Bearer validation.
- users-core Basic/password verification.
- users-core SIP Digest HA1 credential provider.
- users-core provider-neutral token issuance for externally authenticated
  users through `AuthenticationService::issue_tokens_for_user(...)`.
- users-core external identity links for OIDC/SAML/SCIM providers.
- users-core passkey credential storage for WebAuthn adapters.
- Static JWT validation with HMAC/RSA/EC keys.
- JWKS/OIDC-style Bearer validation.
- OAuth2 token introspection Bearer validation for opaque tokens.
- Keycloak/OIDC discovery and local integration fixture support through
  `rvoip-keycloak`.
- Keycloak/OIDC introspection validator construction, configurable JWKS cache
  TTL, and health output for issuer, JWKS reachability, introspection endpoint,
  revocation endpoint, and configured audience.
- Custom external providers via traits.

Implemented optional provider adapters and fixtures:

- Generic OIDC discovery, JWKS, introspection, issuer/audience enforcement,
  cache settings, and health checks through `rvoip-oidc`.
- Keycloak/OIDC discovery, JWKS, introspection, local fixture integration,
  role/scope mapping, and health checks through `rvoip-keycloak`.
- OpenLDAP-backed LDAP password verifier for portable Basic-over-TLS test
  coverage through `rvoip-ldap`.
- Active Directory-compatible LDAP simple-bind behavior through live-skipping
  `RVOIP_AD_*` tests; production AD deployments still require lab validation
  against Samba AD DC or a real AD environment.
- PostgreSQL users-core `UserStore`, `ApiKeyStore`, and auth-service security
  storage through the `postgres` feature.
- Redis-backed token revocation, rate limiting, and Digest nonce/replay state
  through `rvoip-redis`.
- JSON lines, tracing, fanout, OTLP/HTTP JSON logs, and SIEM-oriented audit
  event export through `rvoip-audit`.
- SCIM 2.0 Users/Groups provisioning adapter through `rvoip-scim`, backed by
  users-core users, roles, active state, external identity links, and
  Bearer-admin scope enforcement.
- SAML 2.0 service-provider adapter through `rvoip-saml`, backed by a
  required assertion verifier trait, users-core external identity links, replay
  checks, time/audience/recipient checks, and users-core token issuance.
- WebAuthn/passkey adapter through `rvoip-webauthn`, backed by `webauthn-rs`,
  server-side ceremony state, users-core passkey storage, and users-core token
  issuance after successful passkey authentication.
- IMS AKA provider adapter through `rvoip-ims-aka`, implementing
  `AkaClientProvider` and `AkaVectorProvider` for deterministic lab vectors,
  optional HTTP HSS/UDM broker validation, and lab vector wiring.

Future concrete adapters:

- OAuth2 token introspection convenience builders for additional named IdPs.
- OIDC discovery presets for Okta, Entra ID, Ping, and Auth0.
- Production SAML verifier integrations around a reviewed XML signature/SAML
  validation library or corporate SSO gateway.
- WebAuthn browser smoke harness for local passkey registration/login demos.
- Production HSS/UDM/UDR broker adapters beyond the generic HTTP AKA shape.

Out of scope for SIP auth:

- SAML as a direct SIP auth scheme.
- STIR/SHAKEN identity as user authentication.
- TLS certificate identity as SIP header auth.
- SDP auth negotiation except Digest `qop=auth-int`.

## Security Review Notes

- SIP Digest MD5 and MD5-sess are legacy compatibility mechanisms. New
  provider-backed deployments should prefer SHA-256, SHA-256-sess,
  SHA-512-256, or SHA-512-256-sess when peers support them.
- SIP Digest HA1 values are password-equivalent for their username, realm, and
  algorithm family. Store and rotate them as secrets.
- Users-core login password hashes are Argon2 hashes and must not be reused as
  SIP Digest material.
- Basic auth is legacy compatibility only. It must remain TLS-only unless an
  application explicitly opts into cleartext for a controlled environment.
- HS256 JWT signing is appropriate for in-process/shared-secret validation, but
  public JWKS validation requires asymmetric signing such as RS256.
- Generated HS256 secrets are development defaults. Production deployments
  need stable configured signing keys and a rotation plan.
- In-memory nonce and nonce-count replay tracking is correct for a single UAS
  process. Clustered UAS deployments need shared nonce/replay storage.
- JWT signature validation does not by itself solve immediate access-token
  revocation. Short TTLs, refresh revocation, introspection, or a revocation
  cache are required where immediate revocation matters.

## Runtime Wiring Status

Implemented runtime behavior:

- `SipAuthService` calls `AuthRateLimiter` before credential validation and
  records the result afterward. Rate-limiter provider errors fail closed.
- `SipAuthService` emits redacted `AuthAuditEvent` records for missing auth,
  success, invalid credentials, Basic cleartext policy rejection, stale Digest
  nonce, Digest replay rejection, provider unavailable, and unsupported scheme.
  Audit sink errors fail open by default and fail closed when
  `AuditFailurePolicy::FailClosed` is configured.
- `SipAuthService` uses configured `DigestReplayStore` on async challenge and
  validation paths for provider-backed Digest and Digest-only compatibility
  services.
- `SipTransportSecurityContext` is available as a public auth-policy input,
  and compatibility wrappers now convert legacy boolean TLS inputs into this
  context for UAC and UAS auth decisions.
- Inbound transport metadata from `sip-transport` is cached by dialog
  transaction id and propagated to `IncomingCall`, `IncomingRequest`, and
  `IncomingRegister` auth decisions. Basic and Bearer are denied over
  cleartext by default, with explicit opt-in APIs for controlled legacy
  environments.
- `SipAuthPolicy` is available on `SipAuthService` and enforces enabled
  schemes, minimum Digest algorithm strength, Basic/Bearer cleartext opt-ins,
  required shared Digest replay storage, and audit failure policy.
- UAC auth retry for REGISTER, INVITE, in-dialog requests, and direct
  out-of-dialog retry uses the dialog transport selector instead of raw
  `sips:` string checks, so URI parameters such as `;transport=tls` and
  `;transport=wss` participate in Basic/Bearer transport policy.
- `SipDigestAuthService` remains sync/single-process by default and exposes
  async replay-store helpers for deployments that need shared replay state
  without using the full multi-scheme facade.
- `rvoip-users-core` uses versioned SQLite migrations recorded in
  `schema_migrations`; existing old-shape databases receive auth security
  tables through `002_auth_security_tables`.
- `rvoip-keycloak` constructs both JWKS and OAuth2 introspection validators
  from discovered OIDC metadata and reports richer health metadata.

Provider extensions and examples:

- `AuthAuditSink`, `AuthRateLimiter`, and `DigestReplayStore` are stable
  provider contracts. `crates/extensions/rvoip-redis` now provides a
  Redis-backed `DigestReplayStore`, `TokenRevocationChecker`, and
  `AuthRateLimiter`.
- `crates/extensions/rvoip-audit` provides JSON-lines, tracing, and fanout
  `AuthAuditSink` implementations plus OTLP/HTTP JSON logs and SIEM-oriented
  exporters for redacted auth events.
- `crates/extensions/rvoip-oidc` provides IdP-neutral OIDC discovery, JWKS
  Bearer validator construction, OAuth2 introspection validator construction,
  issuer/audience enforcement, JWKS cache settings, and health metadata.
- `crates/extensions/rvoip-ldap` provides an OpenLDAP/LDAP simple-bind
  `PasswordVerifier` for legacy Basic-over-TLS deployments. The optional local
  fixture lives under `~/Developer/openldap`, AD-compatible live tests use
  `RVOIP_AD_*` variables, and tests skip unless LDAP environment variables are
  set.
- `rvoip-users-core` has a `postgres` feature with `PostgresUserStore`,
  PostgreSQL migrations, `AuthSecurityStore` backing for auth-service
  security tables, and live-skipping PostgreSQL integration tests. SQLite
  remains the default reference store.
- `docs/security-review/` contains the review packet: architecture diagrams,
  data-flow diagrams, control mapping, key-management runbook, incident
  response runbook, secure configuration checklist, and known limitations.
- `examples/auth/` now includes local Endpoint UAC to UnifiedCoordinator UAS
  INVITE flows for Bearer and users-core-backed Digest, plus Redis-backed
  hooks, generic OIDC, LDAP, and custom provider examples.
- SIP method-level UAC retry coverage now exercises REGISTER, INVITE,
  MESSAGE, OPTIONS, SUBSCRIBE, BYE, REFER, INFO, UPDATE, and NOTIFY. The tests
  assert first requests are unauthenticated, retries carry full auth headers,
  method-shaped fields survive retry, and `401`/`407` map to
  `Authorization`/`Proxy-Authorization` respectively.
- RFC 3261 auth retry identity is now asserted for the public out-of-dialog
  builders: MESSAGE, OPTIONS, and SUBSCRIBE retries preserve Call-ID, To, and
  From tag, generate a fresh Via branch through the normal sender, and
  increment CSeq on the initial auth retry and stale-nonce recovery retry.
- Multi-challenge negotiation now handles repeated challenge headers, quoted
  commas inside challenge parameters, case-insensitive Basic/Bearer scheme
  tokens, malformed Digest alternatives, downgrade protection, stronger Digest
  selection, stale/non-stale re-challenge behavior, unknown nonces, and replay
  rejection.
- SCIM provisioning, SAML SSO, WebAuthn/passkey token issuance, LDAP
  389DS/FreeIPA presets, and IMS AKA provider shape now have optional
  extension crates or adapter APIs. Remaining work is deeper live fixture
  coverage and production-provider specialization, not core API availability.
- Production Active Directory claims still need lab validation beyond the
  portable LDAP/AD-compatible test shape.

## Enterprise Completion Plan

The current implementation is a strong auth foundation. Enterprise-grade
completion means adding concrete production providers, enforcing policy from
actual transport context, proving every public API surface behaves consistently,
and publishing operational evidence that security reviewers can inspect.

Required workstreams:

- Transport-truth auth policy:
  - add an explicit `SipTransportSecurityContext` carried through inbound and
    outbound auth paths;
  - stop relying only on `sips:` URI text for Basic/Bearer transport safety;
  - enforce Basic and Bearer cleartext denial by default from actual TLS/WSS
    transport state;
  - add `SipAuthPolicy` for enabled schemes, minimum Digest algorithm,
    cleartext exceptions, issuer/audience requirements, replay requirements,
    and audit/rate-limit failure behavior.
- Production provider crates and storage:
  - add Redis-backed Digest replay, token revocation, rate-limit, and cache
    providers;
  - add the Redis test fixture as an optional local container under
    `~/Developer/redis`, matching the project pattern used for PBX fixtures;
    tests must skip cleanly when the fixture is not running;
  - add audit sinks for JSON lines, tracing, OpenTelemetry/OTLP, and
    SIEM-friendly redacted event export;
  - add a generic OIDC extension crate for discovery, JWKS, introspection, and
    provider health checks independent of Keycloak;
  - add an LDAP password verifier for legacy Basic-over-TLS deployments, with
    OpenLDAP as the required open-source local fixture under
    `~/Developer/openldap`;
  - keep Active Directory-specific behavior as a separate compatibility track
    validated later with Samba AD DC or a real AD lab;
  - add PostgreSQL users-core storage behind a feature flag while keeping
    SQLite as the default reference/dev store;
  - use the PostgreSQL service already running on this machine for PostgreSQL
    integration tests rather than adding a PostgreSQL container fixture in this
    pass.
- Complete API-surface runtime coverage:
  - test Digest, Bearer, Basic, and provider-backed AKA shape across
    `Endpoint`, `UnifiedCoordinator`, `StreamPeer`, and `CallbackPeer`;
  - cover UAC and UAS paths through `IncomingCall`, `IncomingRequest`, and
    `IncomingRegister`;
  - cover REGISTER, INVITE, MESSAGE, OPTIONS, SUBSCRIBE, BYE, REFER, INFO,
    UPDATE, and NOTIFY where each public surface supports the method;
  - prove `401` always uses `Authorization` and `407` always uses
    `Proxy-Authorization`.
- Expanded enterprise security tests:
  - add downgrade, malformed multi-challenge, quoted-comma, repeated-header,
    stale nonce, non-stale second challenge, unknown nonce, and replay tests;
  - add JWT/OIDC negative tests for issuer, audience, algorithm, `kid`,
    expiry, revocation, and unavailable provider states;
  - add inactive user, revoked/expired API key, disabled/suspended API key,
    deleted Digest credential, and rotated Digest credential tests;
  - add property/fuzz tests for auth challenge parsing and redaction;
  - add concurrency tests for shared replay and rate-limit providers.
- Operational docs, examples, and security review packet:
  - add deployment guides for single-node Digest, clustered SIP UAS with Redis,
    users-core PostgreSQL, Keycloak/OIDC Bearer, OpenLDAP Basic-over-TLS, and
    AD compatibility notes;
  - add examples for Endpoint UAC to Unified UAS with Bearer and Digest,
    Redis-backed enterprise hooks, generic OIDC, LDAP, and custom providers;
  - add a `docs/security-review/` packet with architecture diagrams,
    data-flow diagrams, threat model, control mapping, key-management runbook,
    incident response runbook, secure configuration checklist, and known
    limitations.

## Enterprise Security Acceptance Criteria

- Publish a threat model covering SIP Digest, Bearer/JWT, Basic, AKA providers,
  users-core storage, replay protection, token issuance, and external IdP
  integration.
- Document secure defaults and insecure opt-ins, including Basic over cleartext,
  dev-generated HS256 secrets, MD5 Digest, and local in-memory replay state.
- Provide production key-management guidance for HS256 shared secrets, RS256 key
  pairs, JWKS publication, key IDs, rotation, and emergency revocation.
- Provide an access-token revocation strategy: short access-token TTLs by
  default, refresh-token revocation, optional introspection for opaque tokens,
  and optional revocation cache hooks for JWT deployments.
- Add audit-event hooks for successful auth, failed auth, stale nonce, replay
  rejection, token validation failure, password verification failure, API key
  use, and credential rotation.
- Add rate-limit and lockout guidance for password, Basic, API-key, and
  registration attempts.
- Add cluster-safe nonce/replay provider interfaces for SIP Digest deployments
  that run more than one UAS process.
- Add compliance documentation mapping controls to common review expectations:
  least privilege, secret storage, cryptographic algorithm policy, transport
  protection, logging/auditability, revocation, and operational rotation.
- Add negative security tests for downgrade attempts, Basic cleartext policy,
  stale/replayed Digest, missing JWT audience/issuer, wrong `kid`, expired
  tokens, inactive users, revoked/expired API keys, deleted Digest credentials,
  rotated Digest credentials, and disabled/suspended API keys.

## Completion Tracking Checklist

### Implemented In This Pass

- [x] Create this plan file as the first implementation artifact.
- [x] Add `rvoip-users-core` to workspace dependencies.
- [x] Add `auth-core` provider contracts:
  `PasswordVerifier`, `DigestSecretProvider`, `ApiKeyVerifier`, and
  `DigestSecret`.
- [x] Add `JwtValidator::from_decoding_key(...)` for in-process auth-service
  integration.
- [x] Add Digest validation from HA1 material so providers do not need to store
  plaintext SIP secrets.
- [x] Add users-core `auth-core` feature and `UsersCoreAuthProvider` bridge.
- [x] Add users-core password-only verification that does not issue tokens.
- [x] Add users-core API-key-only verification and SIP API-key permissions.
- [x] Add users-core SIP Digest HA1 credential table and create, rotate, lookup,
  and delete APIs.
- [x] Make `UsersConfig::from_env()` load real environment overrides.
- [x] Fix users-core JWKS behavior so HS256 is not advertised as public JWKS and
  RS256 emits RSA public key parameters.
- [x] Add provider-backed Basic validation to `SipAuthService`.
- [x] Add provider-backed Digest validation to `SipAuthService`.
- [x] Add provider-backed Digest challenge algorithm selection.
- [x] Add `examples/auth/auth_users_core_service` demonstrating users-core
  Bearer, Basic, and Digest with SIP auth surfaces.
- [x] Add focused tests for auth-core HA1 validation and JWT decoding-key
  validation.
- [x] Add focused users-core tests for password-only verification, SIP Digest
  credentials, and the auth-core bridge.
- [x] Add focused rvoip-sip tests for provider-backed Basic and Digest.
- [x] Add a local Keycloak external-provider fixture under
  `/Users/jonathan/Developer/keycloak`.
- [x] Add an opt-in Keycloak JWKS integration test for real external OIDC token
  validation through `auth-core`.
- [x] Add `crates/extensions/rvoip-keycloak` as a reusable library fixture for
  Keycloak configuration, JWKS validator construction, and local password-grant
  integration tests.
- [x] Add a live `rvoip-keycloak` integration test that validates a Keycloak
  token through the reusable library fixture.
- [x] Add `auth-core` enterprise provider contracts for redacted audit events,
  rate limiting/lockout, token revocation, and cluster-safe Digest replay
  state.
- [x] Wire JWT and JWKS validators to optional `TokenRevocationChecker`
  providers using JWT `jti` plus subject/issuer/time context.
- [x] Add users-core access-token JTI revocation storage and connect it to
  `UsersCoreAuthProvider` Bearer validation.
- [x] Expand `rvoip-keycloak` with OIDC discovery, issuer-enforcing validator
  construction, and JWKS health checks.
- [x] Add provider-contract tests for audit, rate-limit, and Digest replay
  semantics.
- [x] Add JWT revocation tests and users-core revoked-token bridge coverage.
- [x] Add OAuth2 token introspection Bearer validation for opaque-token
  providers.
- [x] Add explicit downgrade-protection tests for multi-challenge negotiation
  and stronger Digest algorithm selection.
- [x] Add rustdoc examples for users-core-backed Bearer, Basic, Digest and
  external OAuth2 introspection providers.
- [x] Add Keycloak-compatible role claim mapping into Bearer scopes for JWT and
  JWKS validators.
- [x] Wire `AuthAuditSink`, `AuthRateLimiter`, and `DigestReplayStore` into
  `SipAuthService` runtime behavior.
- [x] Add `SipAuthContext` and
  `SipAuthService::authenticate_authorization_with_context(...)` for redacted
  audit/rate-limit metadata.
- [x] Add `SipAuthService::challenges_async(...)` so shared Digest replay
  stores record issued nonces.
- [x] Add async replay-store helpers to `SipDigestAuthService` for
  Digest-only compatibility deployments.
- [x] Add users-core `schema_migrations` and `002_auth_security_tables` for
  existing database upgrades.
- [x] Add Keycloak introspection validator construction, JWKS cache TTL
  configuration, and richer health output.

### Required For Enterprise Security Review

- [x] Publish a threat model for auth-core, users-core, rvoip-sip UAC/UAS auth,
  external IdPs, SIP Digest, Bearer/JWT, Basic, AKA providers, and API keys.
- [x] Add security architecture docs explaining crate boundaries, trust
  boundaries, secret material, and external-provider responsibilities.
- [x] Add production key-management docs for HS256, RS256, JWKS, `kid`, key
  rotation, emergency revocation, and signing-key storage.
- [x] Add first-class JWT access-token revocation hooks or a documented
  revocation-cache/introspection integration path.
- [x] Add OAuth2 token introspection support for opaque Bearer tokens.
- [x] Add production OIDC discovery configuration for issuer, audience, JWKS
  URL, cache settings, and provider health checks.
- [x] Expand `crates/extensions/rvoip-keycloak` with richer claim mapping and
  deeper enterprise deployment docs beyond the current OIDC discovery, issuer
  validation, local fixture, and JWKS health checks.
- [x] Add audit-event hooks for auth success, auth failure, stale nonce, replay
  rejection, token failure, password failure, API-key use, and credential
  rotation.
- [x] Add rate-limit and lockout guidance or hooks for SIP REGISTER, Basic,
  password verification, API keys, and token issuance.
- [x] Add cluster-safe nonce and nonce-count replay provider interfaces for SIP
  Digest deployments with more than one UAS process.
- [x] Add explicit downgrade-protection tests for multi-challenge negotiation.
- [x] Add negative tests for Basic cleartext policy, replayed Digest,
  stale/non-stale re-challenges, wrong JWT issuer, wrong JWT audience, wrong
  `kid`, expired JWTs, inactive users, revoked/expired API keys, deleted
  Digest credentials, rotated Digest credentials, and disabled/suspended API
  keys. Current users-core API keys support active-state suspension,
  revocation/deletion, and expiry.
- [x] Add logging guidance that avoids leaking passwords, HA1 values, bearer
  tokens, API keys, authorization headers, or full JWTs.
- [x] Add compliance mapping for least privilege, cryptographic policy,
  transport protection, secret storage, auditability, revocation, and key
  rotation.
- [x] Add docs.rs/rustdoc examples for users-core-backed Bearer, users-core
  Basic verifier, users-core Digest HA1 provider, external JWT/JWKS, and custom
  providers.
- [x] Add optional external-provider adapters on the roadmap: OpenLDAP-backed
  LDAP password verifier, Active Directory compatibility, PostgreSQL users-core
  store, Redis-backed replay/revocation state, SCIM provisioning, IMS AKA
  HSS/UDM vector provider, and WebAuthn/passkey token issuance.

### Remaining Enterprise-Grade Work

- [x] Add public `SipTransportSecurityContext` and compatibility wrappers for
  UAC and UAS auth decisions.
- [x] Pass actual TLS/WSS transport state through inbound `IncomingCall`,
  `IncomingRequest`, and `IncomingRegister` auth decisions.
- [x] Replace UAC retry raw `sips:` checks with dialog transport-selector
  context for REGISTER, INVITE, in-dialog, and direct OOB retry paths.
- [x] Add outbound post-send transport telemetry so UAC retry decisions can use
  the actual transport selected by DNS/route resolution, not only the dialog
  selector's pre-send decision. Dialog-core records accepted outbound
  transport context by transaction id and SIP request identity after
  `send_request` succeeds; rvoip-sip uses that context for REGISTER, INVITE,
  in-dialog, and OOB retry auth policy before falling back to selector hints.
- [x] Add `SipAuthPolicy` and enforce enabled schemes, minimum Digest
  algorithm, cleartext exceptions, shared replay requirements, and audit
  failure behavior.
- [x] Add explicit high-level policy helpers or docs for issuer/audience
  requirements on Bearer validators and rate-limit failure behavior.
- [x] Deny Basic and Bearer over cleartext by default on UAS inbound auth using
  actual transport state when available.
- [x] Deny Basic and Bearer over cleartext by default on UAC retry auth using
  dialog-selected transport context rather than URI text alone.
- [x] Promote UAC Basic/Bearer policy from selector-backed context to actual
  outbound post-send transport telemetry once the transport layer emits it.
  Basic/Bearer retry auth now prefers the challenged request's recorded
  post-send transport context and only falls back to pre-send selector context
  when older event paths cannot provide telemetry.
- [x] Add Redis-backed `DigestReplayStore`, `TokenRevocationChecker`, and
  `AuthRateLimiter` in `crates/extensions/rvoip-redis`.
- [x] Define whether a separate generic auth-cache provider contract is needed.
  Decision: do not add one now. Keep cache behavior attached to specific
  reviewed contracts: `DigestReplayStore`, `TokenRevocationChecker`,
  `AuthRateLimiter`, JWKS cache TTLs, and provider-specific health checks.
- [x] Add an optional Redis test container fixture under `~/Developer/redis`;
  tests must skip cleanly when the fixture is unavailable.
- [x] Add production audit sinks for JSON lines, tracing, and fanout redacted
  event export in `crates/extensions/rvoip-audit`.
- [x] Add OpenTelemetry/OTLP and vendor/SIEM-specific audit exporters.
  `crates/extensions/rvoip-audit` now includes `OtlpAuditSink` for
  OTLP/HTTP JSON logs and `SiemAuditSink` presets for generic webhooks,
  Splunk HEC, Elastic/ECS, Microsoft Sentinel, and Datadog Logs, with payload
  tests that assert only redacted `AuthAuditEvent` fields are serialized.
- [x] Add a generic OIDC extension crate for discovery, JWKS, introspection,
  audience/issuer enforcement, cache settings, and health checks.
- [x] Add OpenLDAP-backed `PasswordVerifier` support for legacy
  Basic-over-TLS deployments.
- [x] Add an optional OpenLDAP test container fixture under
  `~/Developer/openldap`, seeded with deterministic users; tests must skip
  cleanly when the fixture is unavailable.
- [x] Add a separate Active Directory compatibility test track using Samba AD DC
  or a real AD lab after the OpenLDAP baseline is passing. Coverage lives in
  `rvoip-ldap` as a live-skipping AD-compatible LDAP simple-bind test using
  `RVOIP_AD_*` environment variables; it supports UPN and `sAMAccountName`
  filters while keeping the OpenLDAP fixture as the portable default.
- [x] Add PostgreSQL users-core `UserStore`/`ApiKeyStore` storage behind a
  `postgres` feature flag.
- [x] Add a users-core auth-service database-pool abstraction so PostgreSQL can
  also back refresh-token revocation, access-token revocation, password-change
  updates, last-login updates, and SIP Digest HA1 credential storage.
  `AuthenticationService` now uses the provider-based `AuthSecurityStore`
  contract; SQLite and PostgreSQL stores implement it, and the live Postgres
  test covers token revocation, password change, last-login, and SIP Digest
  create/rotate/delete/lookup through the service.
- [x] Keep SQLite as the default users-core store and use the existing local
  PostgreSQL service on this machine for PostgreSQL integration tests. Verified
  with `RVOIP_USERS_POSTGRES_URL='postgresql:///postgres?host=/tmp' cargo test
  -p rvoip-users-core --features postgres --test postgres_store_tests`.
- [x] Add users-core enterprise identity foundations: provider-neutral token
  issuance for externally authenticated users, external identity link storage,
  and passkey credential storage. Coverage lives in
  `users-core/tests/enterprise_identity_tests.rs`.
- [x] Add `crates/extensions/rvoip-scim` with SCIM Users/Groups model types,
  service metadata, users-core user provisioning/linking, active-state updates,
  Bearer admin scope enforcement, and tests for provisioning plus read/write
  scope behavior.
- [x] Add `crates/extensions/rvoip-saml` as a SAML 2.0 service-provider
  adapter, not a SIP auth scheme. The crate requires a
  `SamlAssertionVerifier` for signed assertion/response validation and handles
  audience, recipient, time, replay, users-core linking, and token issuance.
- [x] Add `crates/extensions/rvoip-webauthn` using `webauthn-rs` for passkey
  registration/authentication ceremonies, server-side ceremony state,
  users-core passkey credential storage, and users-core token issuance on
  successful authentication.
- [x] Expand `crates/extensions/rvoip-ldap` with OpenLDAP, 389 Directory
  Server, FreeIPA, and Active Directory-compatible lookup presets plus
  live-skipping `RVOIP_389DS_*` and `RVOIP_FREEIPA_*` tests.
- [x] Add `crates/extensions/rvoip-ims-aka` implementing the existing SIP
  `AkaClientProvider` and `AkaVectorProvider` traits with deterministic lab
  vectors, optional HTTP broker validation, and lab-vector provider wiring
  without carrier IMS certification claims.
- [x] Add complete UAC/UAS auth parity tests across `Endpoint`,
  `UnifiedCoordinator`, `StreamPeer`, and `CallbackPeer`. Endpoint UAC to
  UnifiedCoordinator UAS INVITE retry is covered for Bearer,
  users-core-backed Digest, and Digest `407` proxy auth in
  `tests/endpoint_unified_auth.rs`; StreamPeer and CallbackPeer UAC to
  UnifiedCoordinator UAS INVITE retry are covered for Bearer,
  users-core-backed Digest, users-core-backed Digest `407` proxy auth, and
  users-core-backed Basic with explicit cleartext opt-in. Endpoint,
  StreamPeer, and CallbackPeer UAC to UnifiedCoordinator UAS INVITE retry are
  covered for provider-backed AKA shape. Non-INVITE request builders are on
  UnifiedCoordinator and call handles; those method-level auth retries are
  covered by the REGISTER/OOB/in-dialog suites listed below.
- [x] Add method coverage for REGISTER, INVITE, MESSAGE, OPTIONS, SUBSCRIBE,
  BYE, REFER, INFO, UPDATE, and NOTIFY where each public surface supports the
  method. Coverage lives in `register_423_retry.rs`,
  `builder_auth_retry_preserves_headers.rs`, `generated_sip_compliance.rs`,
  `oob_auth_retry.rs`, `bye_auth_retry.rs`, `refer_auth_retry.rs`,
  `info_auth_retry.rs`, and `update_notify_auth_retry.rs`.
- [x] Add tests proving `401` retries use `Authorization` and `407` retries use
  `Proxy-Authorization` across supported methods. REGISTER, INVITE,
  MESSAGE, OPTIONS, SUBSCRIBE, INFO, UPDATE, and NOTIFY have explicit 401/407
  coverage; BYE and REFER have 401 coverage plus explicit 407 proxy-auth
  coverage in `update_notify_auth_retry.rs`. Endpoint, StreamPeer, and
  CallbackPeer INVITE Digest proxy-auth coverage is in
  `endpoint_unified_auth.rs`.
- [x] Add tests proving no public builder emits partial/fake auth and no
  configured auth is silently ignored. Endpoint/Unified INVITE tests now assert
  full Bearer and Digest retry headers; OOB MESSAGE/OPTIONS/SUBSCRIBE coverage
  asserts first-send unauthenticated behavior, full Digest/Bearer/Basic retry
  headers, `401`/`407` header mapping, CSeq increments, fresh Via branches,
  and Call-ID/To/From-tag preservation.
- [x] Add malformed challenge, repeated header, quoted comma, downgrade,
  stale nonce, non-stale second challenge, unknown nonce, and replay tests.
  Coverage: repeated `WWW-Authenticate` INVITE coverage lives in
  `invite_repeated_challenge_auth.rs`; malformed/quoted-comma and
  case-insensitive challenge selection coverage lives in `auth::tests`;
  stale/non-stale re-challenge coverage lives in `generated_sip_compliance.rs`
  and `oob_auth_retry.rs`; nonce/replay coverage lives in `auth::tests` and
  `auth-core` provider-contract tests.
- [x] Add JWT/OIDC negative tests for issuer, audience, algorithm, `kid`,
  expiry, revocation, and unavailable provider states. Coverage lives in
  `auth-core/tests/jwt.rs`, `auth-core/tests/jwks.rs`, and
  `auth-core/tests/introspection.rs`; generic OIDC metadata errors are covered
  in `crates/extensions/rvoip-oidc/src/lib.rs`.
- [x] Add users-core negative tests for inactive users, revoked API keys,
  deleted Digest credentials, and rotated Digest credentials in auth-core
  bridge and SIP auth-service flows. Coverage lives in
  `users-core/tests/auth_core_bridge_tests.rs` and
  `rvoip-sip/tests/endpoint_unified_auth.rs`.
- [x] Decide whether users-core API keys need an explicit disabled/suspended
  state distinct from revocation, deletion, and expiry. Implemented as
  `api_keys.active` with SQLite/PostgreSQL migrations; disabled keys validate
  as absent while remaining visible in administrative list APIs.
- [x] Add property/fuzz tests for auth challenge parsing and auth redaction.
  Property coverage lives in `auth::tests::auth_challenge_splitter_preserves_quoted_commas`
  and `tests/trace_redaction.rs::auth_redactor_never_leaks_generated_authorization_values`.
- [x] Add concurrency tests for Redis/shared replay and rate-limit providers.
  Coverage lives in `rvoip-redis/tests/redis_live.rs` and skips cleanly unless
  `RVOIP_REDIS_URL` is configured.
- [x] Add fixture tests for Keycloak/OIDC, optional Redis under
  `~/Developer/redis`, local PostgreSQL, and OpenLDAP under
  `~/Developer/openldap`. Verified with `rvoip-keycloak` live test,
  `rvoip-oidc` metadata tests, `rvoip-redis` live test, `rvoip-ldap`, and the
  local users-core PostgreSQL store test.
- [x] Add deployment guides for single-node Digest, clustered SIP UAS with
  Redis, users-core PostgreSQL, Keycloak/OIDC Bearer, OpenLDAP
  Basic-over-TLS, and AD compatibility notes. See
  `docs/AUTH_DEPLOYMENT_GUIDE.md`.
- [x] Add enterprise examples for Endpoint UAC to Unified UAS with Bearer and
  Digest, Redis-backed hooks, generic OIDC, LDAP, and custom providers.
- [x] Add `docs/security-review/` with architecture diagrams, data-flow
  diagrams, control mapping, key-management runbook, incident response runbook,
  secure configuration checklist, and known limitations.

## Test Plan

- `auth-core`:
  - Provider trait tests with fake providers.
  - JWT validation from users-core-compatible keys.
  - JWKS/OIDC success/failure.
  - Digest validation from HA1 and plaintext secrets.

- `users-core`:
  - Password login, password-only verification, inactive user rejection.
  - JWT issuance validated through `BearerValidator`.
  - API key validation through `ApiKeyVerifier`.
  - SIP Digest credential create/rotate/delete/lookup.
  - Provider-neutral token issuance for externally authenticated users.
  - External identity link create/update/list/delete.
  - Passkey credential create/update/list/delete.
  - HS256 and RS256 validation/JWKS behavior.
  - SQLite migrations and config loading.

- `rvoip-scim`:
  - SCIM Users/Groups model serialization.
  - Bearer-admin scope enforcement for `scim.read` and `scim.write`.
  - users-core user provisioning, active-state updates, role/group mapping, and
    external identity linking.

- `rvoip-saml`:
  - Signed assertion verifier trait integration.
  - Audience, recipient, time-window, and replay rejection.
  - users-core external identity linking and token issuance.

- `rvoip-webauthn`:
  - WebAuthn relying-party configuration validation.
  - Server-side ceremony-state lifecycle.
  - users-core passkey credential storage wrappers.
  - Browser/passkey smoke flow for registration and login as a live optional
    example track.

- `rvoip-ldap`:
  - OpenLDAP, 389DS, FreeIPA, and AD-compatible filter presets.
  - Live-skipping fixture tests for `RVOIP_LDAP_*`, `RVOIP_389DS_*`,
    `RVOIP_FREEIPA_*`, and `RVOIP_AD_*`.

- `rvoip-ims-aka`:
  - Deterministic AKA challenge, UAC authorization, UAS validation, and
    rejection tests.
  - Optional HTTP broker validation compile coverage.
  - Lab-vector provider coverage without carrier certification claims.

- `rvoip-sip`:
  - UAS Bearer auth using users-core-issued JWTs.
  - UAS Basic auth using users-core password verifier.
  - UAS Digest auth using users-core HA1 provider.
  - UAC retries remain scheme-agnostic for Digest, Bearer, Basic, and AKA.
  - StreamPeer, CallbackPeer, UnifiedCoordinator, and Endpoint examples compile and run.

- Docs/examples:
  - Add `examples/auth/` with users-core Bearer, Basic, Digest, external JWKS,
    custom provider, Keycloak/OIDC, Redis hooks, LDAP, SCIM, SAML, WebAuthn,
    and IMS AKA provider-shape examples.
  - Update cargo docs for crate responsibilities and supported internal/external auth services.
  - Run `cargo doc`, doc tests, users-core tests, auth-core tests, and targeted SIP auth tests.

## Assumptions

- `users-core` becomes the first-party reference service, not the only supported service.
- Protocol crates depend on `auth-core` contracts, not users-core storage.
- Developers using external auth systems implement traits or use built-in JWT/JWKS/OIDC/introspection validators.
- Bearer JWT is the preferred modern users-core-backed SIP auth path.
- SIP Digest remains important for PBX compatibility and must use dedicated SIP credential material.
