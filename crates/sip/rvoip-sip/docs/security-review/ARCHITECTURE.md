# Auth Security Architecture

## Crate Boundaries

```mermaid
flowchart LR
    app["Application"] --> sip["rvoip-sip"]
    sip --> auth["rvoip-auth-core"]
    app --> users["rvoip-users-core (optional)"]
    app --> oidc["rvoip-oidc / rvoip-keycloak (optional)"]
    app --> ldap["rvoip-ldap (optional)"]
    app --> redis["rvoip-redis (optional)"]
    app --> audit["rvoip-audit (optional)"]
    users --> auth
    oidc --> auth
    ldap --> auth
    redis --> auth
    audit --> auth
```

`rvoip-sip` owns SIP authentication headers, challenge negotiation, UAC retry,
UAS challenge generation, and inbound authentication orchestration. It does not
own user databases, token issuance, LDAP, Redis, or IdP configuration.

`rvoip-auth-core` owns reusable primitives and provider traits:
`BearerValidator`, `PasswordVerifier`, `DigestSecretProvider`,
`TokenRevocationChecker`, `DigestReplayStore`, `AuthRateLimiter`, and
`AuthAuditSink`.

`rvoip-users-core` is the optional first-party user/auth service. SQLite is the
default reference store. PostgreSQL user/API-key storage is feature-gated;
full auth-service security-table support for PostgreSQL remains tracked.

Extension crates provide concrete providers without making protocol crates
depend on a specific deployment stack:

- `rvoip-redis`: Digest replay, token revocation, auth rate limiting.
- `rvoip-audit`: JSON-lines, tracing, and fanout audit sinks.
- `rvoip-oidc`: generic OIDC discovery, JWKS, introspection.
- `rvoip-keycloak`: Keycloak-specific fixture and adapter.
- `rvoip-ldap`: LDAP simple-bind password verifier.

## Trust Boundaries

```mermaid
flowchart TB
    sippeer["Remote SIP Peer"] --> transport["SIP Transport Boundary"]
    transport --> sipuas["rvoip-sip UAS Auth"]
    sipuas --> authcore["auth-core Traits"]
    authcore --> providers["External Providers"]
    providers --> idp["OIDC/Keycloak"]
    providers --> ldap["LDAP/AD"]
    providers --> redis["Redis"]
    providers --> users["users-core DB"]
    sipuas --> app["Application Authorization"]
```

Credential-bearing schemes must be protected by actual TLS/WSS transport state
or explicit cleartext opt-ins. Basic and Bearer cleartext are disabled by
default.

Provider errors that affect credential validation or rate limiting fail closed.
Audit sink errors fail open by default and can be configured fail-closed.
