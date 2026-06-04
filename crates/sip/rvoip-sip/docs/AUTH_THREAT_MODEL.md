# RVoIP Authentication Threat Model

## Scope

This threat model covers SIP access authentication and the supporting RVoIP
auth/user libraries:

- SIP header auth in `rvoip-sip`: `WWW-Authenticate`,
  `Proxy-Authenticate`, `Authorization`, and `Proxy-Authorization`.
- Digest `qop=auth-int` body hashing when a SIP body is present.
- Bearer/JWT/JWKS validation through `rvoip-auth-core`.
- Basic username/password validation through provider traits.
- Provider-backed IMS AKA negotiation surfaces.
- `rvoip-users-core` as an optional first-party user/auth service.
- External providers such as Keycloak, OIDC/JWKS, LDAP/AD, IMS HSS/AuC, or a
  custom service.

Out of scope for SIP auth are STIR/SHAKEN identity, TLS certificate identity,
SAML as a SIP auth scheme, media encryption, and SDP negotiation except Digest
`qop=auth-int`.

## Assets

Protect these assets as secrets or security-sensitive state:

- SIP passwords and SIP Digest HA1 values.
- JWT signing keys, JWKS private keys, HMAC secrets, and `kid` rotation state.
- Bearer tokens, refresh tokens, API keys, and Authorization headers.
- Basic passwords and password-verification attempts.
- Digest nonce and nonce-count replay state.
- IMS AKA vectors, RES/CK/IK material, and provider credentials.
- User records, roles, API-key permissions, revocation state, and audit logs.

## Trust Boundaries

| Boundary | Trusted side | Untrusted side | Required controls |
|----------|--------------|----------------|-------------------|
| Network to SIP UAS | `rvoip-sip` inbound auth service | SIP peers and proxies | Challenge validation, replay checks, TLS policy, redacted logging |
| SIP UAC to remote PBX/proxy | Application credentials | Remote challenge headers | Algorithm negotiation, downgrade rejection, Basic policy |
| SIP crate to auth-core | Protocol orchestration | Provider outputs can fail | Typed errors, no raw secret logging |
| auth-core to users-core | Trait contracts | Store/service availability | Provider errors, revocation checks, audit hooks |
| auth-core to external IdP | Local validator | IdP metadata/JWKS/token endpoint | Issuer/audience validation, JWKS cache, health checks |
| Single process to cluster | Local memory | Other UAS nodes | Shared nonce/replay and revocation stores |

## Threats And Mitigations

### Credential Disclosure

Risk: passwords, HA1 values, API keys, bearer tokens, or full JWTs leak through
logs, traces, panic messages, packet captures, or diagnostics.

Mitigations:

- Treat HA1 as password-equivalent for its username, realm, and algorithm
  family.
- Redact Authorization, Proxy-Authorization, API-key values, bearer tokens,
  refresh tokens, passwords, and HA1 values from logs.
- Audit events use identifiers such as subject, realm, peer, or token `jti`,
  not credential material.
- Basic is rejected over cleartext unless both UAC and UAS explicitly opt in.

### Replay

Risk: an attacker replays Digest responses, DPoP proofs, API keys, bearer
tokens, or stale refresh tokens.

Mitigations:

- Digest nonce-count replay is tracked per `(username, nonce)`, not per cnonce.
- Stale Digest nonces are re-challenged only when the nonce was issued and
  expired; unknown nonce and wrong password are not reported as stale.
- Cluster deployments must use a shared `DigestReplayStore` implementation.
- JWT revocation checks use token `jti`; users-core provides reference
  access-token JTI revocation and refresh-token revocation.
- Bearer tokens should be short-lived. Immediate revocation requires a
  revocation checker, token introspection, or opaque-token validation.

### Downgrade

Risk: a malicious proxy or peer offers a weaker auth scheme or weaker Digest
algorithm to force credential exposure.

Mitigations:

- UAC multi-challenge negotiation prefers AKA, Bearer, Digest, then Basic.
- Digest prefers stronger algorithms when compatible and rejects unknown
  algorithms instead of downgrading to MD5.
- Basic remains disabled over cleartext unless explicitly allowed.
- Deployments that forbid MD5 or Basic should configure policy and tests that
  reject those challenges.

### Token Forgery Or Confused Issuer

Risk: a token from another issuer, tenant, realm, or audience is accepted.

Mitigations:

- `JwtValidator` and `JwksJwtValidator` support issuer and audience
  enforcement.
- The Keycloak extension builds validators from OIDC discovery metadata and
  enforces discovered issuer plus configured audience.
- Public JWKS validation is asymmetric. HS256 is only for in-process
  shared-secret validation and must not be published as JWKS.

### Brute Force And Credential Stuffing

Risk: repeated REGISTER, Basic, password, API-key, or token issuance attempts
are used to guess credentials.

Mitigations:

- `auth-core` exposes `AuthRateLimiter` and structured keys for SIP REGISTER,
  SIP request auth, Basic, password, API-key, Bearer, token issuance, and
  Digest attempts.
- users-core password verification uses Argon2id and constant-time/dummy-hash
  behavior for missing users.
- Production services should apply per-peer and per-subject lockouts and feed
  decisions into audit logs.

### Provider Or Store Outage

Risk: IdP, JWKS, LDAP, database, Redis, or IMS vector providers are unavailable.

Mitigations:

- Provider traits return explicit unavailable errors.
- SIP services can fail closed for credential validation while preserving
  useful challenge responses for missing credentials.
- JWKS validators cache keys with configurable TTL.
- External providers should have health checks and operational alerting.

## Security Review Requirements

A deployment is not enterprise-ready until it documents:

- Which schemes are enabled and which are disabled by policy.
- TLS policy for Basic and for bearer-token transport.
- Digest algorithm policy and whether MD5 is PBX-only.
- JWT issuer, audience, signing algorithm, `kid`, JWKS cache TTL, and
  revocation strategy.
- Shared replay/revocation storage when more than one UAS process is active.
- Audit event destinations, retention, and redaction policy.
- Rate-limit and lockout thresholds for registration and password flows.
