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
- Static JWT validation with HMAC/RSA/EC keys.
- JWKS/OIDC-style Bearer validation.
- OAuth2 token introspection Bearer validation for opaque tokens.
- Keycloak/OIDC discovery and local integration fixture support through
  `rvoip-keycloak`.
- Keycloak/OIDC introspection validator construction, configurable JWKS cache
  TTL, and health output for issuer, JWKS reachability, introspection endpoint,
  revocation endpoint, and configured audience.
- Custom external providers via traits.

Future concrete adapters:

- OIDC discovery builder.
- LDAP/Active Directory password verifier.
- PostgreSQL users-core store.
- Redis-backed token revocation/cache/nonce storage.
- SCIM provisioning into users-core.
- IMS AKA HSS/UDM vector provider adapters.
- WebAuthn/passkey login for users-core token issuance.

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
- `SipDigestAuthService` remains sync/single-process by default and exposes
  async replay-store helpers for deployments that need shared replay state
  without using the full multi-scheme facade.
- `rvoip-users-core` uses versioned SQLite migrations recorded in
  `schema_migrations`; existing old-shape databases receive auth security
  tables through `002_auth_security_tables`.
- `rvoip-keycloak` constructs both JWKS and OAuth2 introspection validators
  from discovered OIDC metadata and reports richer health metadata.

Contract-only or roadmap items:

- `AuthAuditSink`, `AuthRateLimiter`, and `DigestReplayStore` are stable
  provider contracts; production storage backends such as Redis or SIEM
  adapters remain deployment-specific or future extension crates.
- LDAP/AD, PostgreSQL users-core storage, SCIM provisioning, WebAuthn/passkey
  token issuance, and IMS HSS/UDM vector adapters remain roadmap items.

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
  tokens, inactive users, disabled API keys, and deleted Digest credentials.

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
  `kid`, expired JWTs, inactive users, revoked/expired API keys, and deleted
  Digest credentials.
- [x] Add logging guidance that avoids leaking passwords, HA1 values, bearer
  tokens, API keys, authorization headers, or full JWTs.
- [x] Add compliance mapping for least privilege, cryptographic policy,
  transport protection, secret storage, auditability, revocation, and key
  rotation.
- [x] Add docs.rs/rustdoc examples for users-core-backed Bearer, users-core
  Basic verifier, users-core Digest HA1 provider, external JWT/JWKS, and custom
  providers.
- [x] Add optional external-provider adapters on the roadmap: LDAP/AD password
  verifier, PostgreSQL users-core store, Redis-backed replay/revocation state,
  SCIM provisioning, IMS AKA HSS/UDM vector provider, and WebAuthn/passkey token
  issuance.

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
  - HS256 and RS256 validation/JWKS behavior.
  - SQLite migrations and config loading.

- `rvoip-sip`:
  - UAS Bearer auth using users-core-issued JWTs.
  - UAS Basic auth using users-core password verifier.
  - UAS Digest auth using users-core HA1 provider.
  - UAC retries remain scheme-agnostic for Digest, Bearer, Basic, and AKA.
  - StreamPeer, CallbackPeer, UnifiedCoordinator, and Endpoint examples compile and run.

- Docs/examples:
  - Add `examples/auth/` with users-core Bearer, Basic, Digest, external JWKS, and custom provider examples.
  - Update cargo docs for crate responsibilities and supported internal/external auth services.
  - Run `cargo doc`, doc tests, users-core tests, auth-core tests, and targeted SIP auth tests.

## Assumptions

- `users-core` becomes the first-party reference service, not the only supported service.
- Protocol crates depend on `auth-core` contracts, not users-core storage.
- Developers using external auth systems implement traits or use built-in JWT/JWKS/OIDC/introspection validators.
- Bearer JWT is the preferred modern users-core-backed SIP auth path.
- SIP Digest remains important for PBX compatibility and must use dedicated SIP credential material.
