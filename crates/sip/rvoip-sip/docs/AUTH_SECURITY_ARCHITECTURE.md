# RVoIP Authentication Security Architecture

## Crate Responsibilities

| Crate | Responsibility | Must not own |
|-------|----------------|--------------|
| `rvoip-auth-core` | Provider traits, Digest crypto, JWT/JWKS/AAuth/DPoP primitives, revocation/audit/rate-limit/replay contracts | SIP transaction routing, user database schema |
| `rvoip-users-core` | Optional first-party user service, SQLite reference store, password hashing, API keys, JWT issuance, refresh-token revocation, SIP Digest HA1 credentials | Mandatory dependency of protocol crates |
| `rvoip-sip` | SIP auth headers, challenge negotiation, UAC retry, UAS challenge/validation orchestration, `Authorization` vs `Proxy-Authorization` routing | Password database, JWT signing, external IdP ownership |
| `rvoip-keycloak` | Optional Keycloak/OIDC discovery, JWKS and introspection validator construction, configurable JWKS cache TTL, local integration fixture client, provider health checks | Core SIP dependency, production login UX |
| Application or external service | Policy, IdP configuration, LDAP/AD, Redis, IMS HSS/AuC, secrets management, audit sink, rate limits | Protocol parsing internals |

Protocol crates depend on `auth-core` contracts. They do not depend on
users-core storage. Developers can use users-core, Keycloak, LDAP/AD, an
external OIDC provider, IMS infrastructure, or a custom service by implementing
the same traits.

## UAC Flow

1. The application configures `SipClientAuth`, `SipAccount`, or per-request
   auth helpers.
2. The first SIP request normally has no auth header unless the caller
   explicitly uses a preemptive mode supported by that scheme.
3. A remote UAS/proxy returns `401 WWW-Authenticate` or
   `407 Proxy-Authenticate`.
4. The UAC parses all challenges and chooses the strongest configured
   compatible scheme.
5. The retry uses `Authorization` for `401` and `Proxy-Authorization` for
   `407`.
6. Digest stale nonce recovery is allowed only for valid `stale=true`
   re-challenges with a new nonce.

## UAS Flow

1. The application builds `SipAuthService` with enabled schemes and provider
   trait objects.
2. Incoming surfaces call `authenticate_with(...)` or
   `authenticate_authorization(...)`.
3. Missing credentials return generated challenges.
4. Optional rate-limit providers check policy before credential validation.
5. Present credentials are validated by the configured scheme provider.
6. Optional shared Digest replay stores validate issued nonce state and
   nonce-count monotonicity.
7. Optional audit sinks receive redacted success/failure events.
8. Successful auth returns `AuthIdentity` with scheme, subject/username, realm,
   scopes, and origin/proxy source.
9. Rejected auth emits challenges without exposing whether the username exists.

## Secret Handling

- SIP Digest HA1 is password-equivalent. Store it as secret material.
- users-core Argon2 login password hashes are not SIP Digest material.
- JWT HMAC secrets and RSA private keys must come from a production secret
  manager or equivalent secure configuration.
- Bearer tokens, API keys, refresh tokens, passwords, HA1 values, full JWTs,
  and Authorization headers must be redacted.
- Audit events carry `jti`, subject, realm, peer, and non-secret metadata only.

## Provider Contracts

`rvoip-auth-core` exposes these provider interfaces:

- `BearerValidator` for Bearer/JWT/JWKS/AAuth/opaque-token validation.
- `PasswordVerifier` for Basic/password checks without token issuance.
- `DigestSecretProvider` for SIP Digest HA1/plain secret lookup.
- `ApiKeyVerifier` for API-key validation.
- `TokenRevocationChecker` for JWT/opaque-token revocation.
- `DigestReplayStore` for cluster-safe Digest nonce and nonce-count replay.
- `AuthAuditSink` for redacted auth/security audit events.
- `AuthRateLimiter` for rate-limit and lockout policy.

Provider failures should be treated as fail-closed for credential validation
unless a deployment has a documented exception.

Bearer validators must own token trust policy. A SIP realm is not enough to
validate a JWT or opaque token. Production validators should enforce issuer,
audience or resource indicators, expiry, accepted algorithms, `kid` behavior,
revocation or introspection strategy, and application-required scopes before
returning an authenticated identity.

## Logging And Audit

Log or audit:

- auth success by scheme, realm, subject/username, peer, and source;
- auth failure reason category;
- stale nonce and replay rejection;
- token validation failure and revoked token `jti`;
- password verification failure;
- API-key use by key id or subject, not raw key;
- credential creation, rotation, and deletion.

Never log:

- passwords;
- Basic base64 payloads;
- bearer tokens or refresh tokens;
- API keys;
- full JWTs;
- SIP HA1 values;
- full Authorization or Proxy-Authorization headers.

## Rate Limiting And Lockout

Use `AuthRateLimiter` or an equivalent provider for:

- SIP REGISTER attempts by peer, username, and realm;
- SIP request auth failures by peer and method;
- Basic/password verification by peer and username;
- API-key verification by key id or peer;
- token issuance and refresh;
- Digest replay/stale failure spikes.

Rate-limit decisions should be audited. Lockout policy should avoid user
enumeration and should distinguish temporary lockout from credential validity
only in internal logs.

`SipAuthService` treats rate-limiter provider failures as fail-closed. Audit
sink failures are fail-open by default and can be made fail-closed with
`AuditFailurePolicy::FailClosed` when audit delivery is a hard control.

## Clustered Deployment Requirements

Single-process in-memory replay tracking is acceptable only for standalone
UAS deployments. Multi-process or multi-node deployments need shared state for:

- Digest issued nonces and nonce expiry;
- nonce-count monotonicity per `(username, nonce)`;
- JWT access-token revocation;
- API-key disablement;
- rate-limit counters;
- audit-event delivery.

Redis, a database, or another strongly consistent store can implement the
provider traits without changing SIP protocol code.
