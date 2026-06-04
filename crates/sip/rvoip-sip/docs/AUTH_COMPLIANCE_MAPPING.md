# RVoIP Authentication Compliance Mapping

This document maps RVoIP auth design controls to common enterprise security
review expectations. It is not a certification claim; it is a review aid for
CSO, AppSec, and compliance teams.

## Control Matrix

| Review area | RVoIP control | Evidence |
|-------------|---------------|----------|
| Least privilege | Bearer scopes, API-key permissions, `AuthIdentity.scopes` | `rvoip-auth-core` validators, users-core API keys, SIP auth docs |
| Strong authentication | Digest, Bearer/JWT/JWKS, provider-backed AKA, Basic disabled over cleartext | `AUTHENTICATION.md`, `COMPATIBILITY_MATRIX.md` |
| Cryptographic policy | SHA-256/SHA-512-256 Digest support, unknown algorithm rejection, RS256/JWKS support | auth-core Digest/JWT/JWKS tests |
| Legacy compatibility isolation | MD5 and Basic documented as compatibility modes | `SECURITY_POSTURE.md`, `AUTH_KEY_MANAGEMENT.md` |
| Transport protection | Basic cleartext opt-in only, TLS guidance | SIP auth docs and Basic tests |
| Secret storage | HA1 separated from Argon2 login hashes; API keys hashed; signing-key guidance | users-core schema, `AUTH_KEY_MANAGEMENT.md` |
| Token validation | Signature, expiry, issuer, audience, `kid`, JWKS cache | `JwtValidator`, `JwksJwtValidator`, Keycloak extension |
| Revocation | refresh-token revocation, access-token JTI hooks, users-core JTI store | users-core auth service, auth-core provider tests |
| Replay protection | Digest nonce/nonces-count tracking, cluster replay interface | SIP auth service, `DigestReplayStore` |
| Auditability | Redacted audit event contract and logging guidance | `AuthAuditSink`, `AUTH_SECURITY_ARCHITECTURE.md` |
| Rate limiting | Rate-limit/lockout provider contract and deployment guidance | `AuthRateLimiter`, `AUTH_SECURITY_ARCHITECTURE.md` |
| External IdP support | OIDC/JWKS, Keycloak fixture, custom provider traits | `rvoip-keycloak`, auth-core providers |
| User lifecycle | inactive user rejection, credential rotation/delete APIs | users-core tests and APIs |

## Required Deployment Evidence

Enterprise deployments should record:

- enabled auth schemes and disabled legacy schemes;
- SIP Digest algorithm policy;
- Basic cleartext exception status;
- JWT issuer, audience, algorithm, `kid`, key owner, and rotation cadence;
- JWKS/OIDC discovery URL and health checks;
- token TTLs, refresh-token policy, and revocation mechanism;
- shared replay/revocation/rate-limit stores for clustered UAS deployments;
- audit log destination, retention, and redaction controls;
- incident response procedure for key or credential compromise.

## Non-Compliance Risks To Avoid

- Accepting JWTs without issuer and audience validation.
- Publishing HS256 secrets through JWKS.
- Using users-core Argon2 login password hashes as SIP Digest material.
- Enabling Basic over cleartext without a documented exception.
- Running multiple UAS nodes with only process-local Digest replay state.
- Logging full Authorization headers, JWTs, API keys, HA1 values, or passwords.
- Treating MD5 Digest as a preferred first-party algorithm.
- Relying only on JWT signature validation when immediate revocation is
  required.

## Roadmap Items For Additional Regulated Environments

- OAuth2 token introspection for opaque tokens.
- Redis-backed replay, revocation, and rate-limit stores.
- LDAP/Active Directory password verifier.
- PostgreSQL users-core store.
- SCIM provisioning into users-core.
- IMS AKA HSS/UDM vector provider adapters.
- WebAuthn/passkey login for users-core token issuance.
