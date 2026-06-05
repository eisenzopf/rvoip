# Auth Data Flows

## UAC Challenge Retry

```mermaid
sequenceDiagram
    participant UAC as rvoip-sip UAC
    participant Peer as UAS/Proxy
    UAC->>Peer: Initial SIP request
    Peer-->>UAC: 401 WWW-Authenticate or 407 Proxy-Authenticate
    UAC->>UAC: Parse all challenges and select strongest configured scheme
    UAC->>UAC: Enforce Basic/Bearer transport policy
    UAC->>Peer: Retry with Authorization or Proxy-Authorization
```

`401` always maps to `Authorization`. `407` always maps to
`Proxy-Authorization`. Digest nonce-count increments per `(realm, nonce)`.
Digest stale retry is allowed only for `stale=true` with a new nonce.

## UAS Digest

```mermaid
sequenceDiagram
    participant Peer
    participant SIP as rvoip-sip SipAuthService
    participant Secret as DigestSecretProvider
    participant Replay as DigestReplayStore
    Peer->>SIP: Request without Authorization
    SIP->>Replay: Record issued nonce (async path)
    SIP-->>Peer: 401/407 Digest challenge
    Peer->>SIP: Request with Digest response
    SIP->>Replay: Validate nonce and nonce-count
    SIP->>Secret: Lookup HA1/plain secret
    SIP-->>Peer: Authorized or challenge/reject
```

Clustered UAS deployments must configure a shared `DigestReplayStore`.

## UAS Bearer

```mermaid
sequenceDiagram
    participant Peer
    participant SIP as rvoip-sip
    participant Validator as BearerValidator
    participant Revocation as TokenRevocationChecker
    Peer->>SIP: Bearer token
    SIP->>SIP: Enforce TLS/WSS unless explicit cleartext opt-in
    SIP->>Validator: Validate token
    Validator->>Revocation: Check jti/opaque token revocation when configured
    Validator-->>SIP: IdentityAssurance and scopes
```

Bearer validators must enforce issuer, audience/resource, expiry, algorithms,
`kid`, revocation/introspection strategy, and scopes.

## UAS Basic With LDAP

```mermaid
sequenceDiagram
    participant Peer
    participant SIP as rvoip-sip
    participant LDAP as rvoip-ldap
    Peer->>SIP: Basic credentials over TLS/WSS
    SIP->>LDAP: PasswordVerifier.verify_password
    LDAP->>LDAP: Search one user DN
    LDAP->>LDAP: Simple bind as user DN
    LDAP-->>SIP: IdentityAssurance or invalid
```

Basic is legacy compatibility. Cleartext Basic requires explicit UAC and UAS
opt-ins.

## Enterprise Hooks

```mermaid
flowchart LR
    auth["SipAuthService"] --> limiter["AuthRateLimiter"]
    auth --> audit["AuthAuditSink"]
    auth --> replay["DigestReplayStore"]
    auth --> validator["Bearer/Password/Digest Providers"]
```

Rate limit checks happen before credential validation and fail closed.
Audit events are redacted and do not contain secrets.
