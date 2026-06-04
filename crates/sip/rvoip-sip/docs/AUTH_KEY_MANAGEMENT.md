# RVoIP Authentication Key Management

## JWT Signing Keys

### HS256

HS256 uses one shared secret for signing and verification.

Requirements:

- Generate at least 256 bits of entropy.
- Store the secret in a secret manager or encrypted configuration.
- Do not publish HS256 material through JWKS.
- Use only for in-process or tightly controlled shared-secret deployments.
- Rotate with a planned overlap window and explicit `kid` policy when multiple
  validators are active.

users-core can generate an HS256 development secret, but production
deployments must configure a stable secret.

### RS256

RS256 uses a private key for signing and a public key for verification/JWKS.

Requirements:

- Generate and store the private key outside source control.
- Publish only the public JWK.
- Assign a stable `kid` per signing key.
- Keep the previous public key available until all tokens signed by it expire.
- Have an emergency revoke/retire process for compromised keys.

users-core requires a caller-supplied RSA private key for RS256. It does not
generate production RSA keys internally.

## JWKS And OIDC

Requirements:

- Validate issuer and audience, not just signature.
- Restrict accepted algorithms.
- Cache JWKS with a bounded TTL.
- Refresh JWKS on unknown `kid`.
- Reject tokens with missing `kid` when using JWKS.
- Monitor discovery and JWKS health.

The `rvoip-keycloak` extension discovers issuer, token endpoint, JWKS URI, and
optional introspection/revocation endpoints from Keycloak OIDC metadata, then
builds validators that enforce issuer and configured audience. Its JWKS
validator uses a bounded cache TTL, configurable through
`KeycloakConfig::with_jwks_cache_ttl(...)`, and its health check reports issuer,
JWKS reachability, optional introspection/revocation endpoints, and configured
audience.

## Token Revocation

Access tokens should be short-lived. Immediate revocation requires one of:

- JWT `jti` revocation checks through `TokenRevocationChecker`;
- opaque-token or OAuth2 introspection;
- a provider-side revocation cache;
- user/session revocation policy based on subject and issued-at context.

users-core access tokens include `jti`. Its SQLite reference service can store
revoked access-token JTIs until expiry, and the users-core auth bridge wires
that store into `JwtValidator`.

Refresh tokens must be revocable. users-core stores refresh-token JTIs when a
database pool is configured and rejects revoked refresh tokens.

## SIP Digest Secrets

Digest HA1 is derived from `username:realm:password` and is
password-equivalent for that tuple. Requirements:

- Store HA1 as secret material.
- Use dedicated SIP Digest credentials, not login password hashes.
- Rotate SIP Digest credentials independently from login passwords.
- Prefer SHA-256 or SHA-512-256 family algorithms for first-party systems.
- Keep MD5 only for PBX compatibility where required.

users-core stores SIP Digest HA1 per SIP username, realm, and algorithm family.

## Basic Passwords

Basic carries a reversible username/password value on the wire. Requirements:

- Permit Basic only over TLS unless an explicit cleartext exception is
  documented.
- Apply rate limits and lockout policy.
- Verify passwords through `PasswordVerifier`; do not issue tokens as a side
  effect of Basic validation.
- Prefer Bearer, Digest, or AKA where possible.

## API Keys

Requirements:

- Store only API-key hashes.
- Display raw API keys once at creation.
- Support disable/delete and expiry.
- Scope keys with least privilege.
- Audit key use without logging the raw key.

users-core API keys include SIP permissions such as `sip.register`,
`sip.call`, `sip.message`, and `sip.presence`.

## Emergency Rotation Checklist

1. Disable compromised users, API keys, or client credentials.
2. Revoke refresh tokens and known access-token JTIs.
3. Rotate JWT signing keys or HMAC secrets.
4. Publish new JWKS and retain old public keys only until old tokens expire.
5. Rotate SIP Digest HA1 credentials if SIP secrets may be exposed.
6. Rotate Keycloak/OIDC client secrets and update deployment configuration.
7. Verify audit logs and alerting captured the incident.
