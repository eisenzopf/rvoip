# Auth Control Mapping

| Control Area | RVoIP Control | Evidence |
|--------------|---------------|----------|
| Least privilege | Bearer scopes, API-key permissions, `AuthIdentity.scopes` | `AUTHENTICATION.md`, auth-core validators |
| Strong authentication | Digest, Bearer/JWT/JWKS/OIDC, provider-backed AKA, Basic-over-TLS only | `COMPATIBILITY_MATRIX.md`, auth tests |
| Transport protection | Basic/Bearer cleartext denied by default; TLS/WSS context on UAS and selector-backed UAC retry | `SipTransportSecurityContext` tests |
| Replay protection | Digest nonce and nonce-count tracking; `DigestReplayStore`; Redis provider | `rvoip-redis`, auth tests |
| Revocation | users-core access-token JTI store, `TokenRevocationChecker`, introspection, Redis revocation provider | users-core/auth-core tests |
| Auditability | `AuthAuditSink`; JSON-lines/tracing/fanout sinks; redaction guidance | `rvoip-audit`, `AUTH_SECURITY_ARCHITECTURE.md` |
| Rate limiting | `AuthRateLimiter`; fail-closed integration; Redis provider | `rvoip-redis`, auth tests |
| Key management | HS256/RS256/JWKS rotation guidance | `KEY_MANAGEMENT_RUNBOOK.md` |
| Provider independence | auth-core traits for external IdP, LDAP, Redis, database, custom services | crate docs and extension crates |

Reviewer-required deployment evidence:

- configured JWT issuer and audience;
- accepted JWT algorithms and key IDs;
- JWKS cache TTL and rotation cadence;
- token revocation or introspection strategy;
- Redis or equivalent shared replay state for clustered UAS;
- audit sink destination and failure policy;
- rate-limit thresholds and lockout policy;
- TLS/WSS enforcement and any cleartext exceptions;
- Digest algorithm policy and MD5 compatibility exceptions.
