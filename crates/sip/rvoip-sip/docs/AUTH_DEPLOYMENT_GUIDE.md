# RVoIP SIP Authentication Deployment Guide

This guide covers production deployment shapes for SIP header authentication in
`rvoip-sip`. SIP auth is carried in `WWW-Authenticate`,
`Proxy-Authenticate`, `Authorization`, and `Proxy-Authorization`. SDP only
participates when Digest `qop=auth-int` hashes the request body.

## Secure Defaults

- Prefer Bearer/JWT or OIDC-backed Bearer for first-party services.
- Use SIP Digest for PBX interoperability and store dedicated HA1 material, not
  login password hashes.
- Prefer Digest SHA-256 or SHA-512-256 when peers support them. Keep MD5 only
  for legacy PBX compatibility.
- Keep Basic disabled over cleartext. Use TLS/WSS or an explicit lab-only
  cleartext opt-in.
- Use shared replay storage for clustered UAS Digest deployments.
- Treat audit sinks and rate limiters as part of the auth boundary. Rate-limit
  provider failures should fail closed.

## Single-Node Digest UAS

Use this mode for one SIP process, local tests, or simple PBX-compatible UAS
deployments.

Recommended setup:

```rust,no_run
use std::sync::Arc;
use rvoip_sip::{DigestAlgorithm, SipAuthService};
use users_core::UsersCoreAuthProvider;

fn auth(provider: Arc<UsersCoreAuthProvider>) -> SipAuthService {
    SipAuthService::new()
        .with_digest_provider("pbx.example.com", provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256)
}
```

Operational notes:

- Use `SipAuthService::challenges_async` or inbound `authenticate_with` helpers
  so issued nonces are tracked correctly.
- Single-process in-memory nonce-count tracking is acceptable only when all
  authenticated requests for a realm land on the same process.
- Rotate SIP Digest credentials independently from login passwords.

## Clustered SIP UAS With Redis

Use Redis when multiple UAS processes can validate requests for the same realm.
The Redis extension supplies shared nonce/replay, token revocation, and
rate-limit state.

Local fixture:

```bash
cd ~/Developer/redis
docker compose up -d
RVOIP_REDIS_URL=redis://127.0.0.1:6379 cargo test -p rvoip-redis --test redis_live
```

Recommended setup:

```rust,no_run
use std::sync::Arc;
use rvoip_auth_core::DigestSecretProvider;
use rvoip_redis::{RedisAuthConfig, RedisAuthProvider};
use rvoip_sip::{DigestAlgorithm, SipAuthService};

async fn auth() -> anyhow::Result<SipAuthService> {
    let redis = Arc::new(RedisAuthProvider::from_config(
        RedisAuthConfig::new("redis://127.0.0.1:6379")
            .with_namespace("rvoip:prod:sip-auth"),
    )?);
    let digest_provider: Arc<dyn DigestSecretProvider> =
        todo!("supply users-core or external HA1/plaintext Digest provider");

    Ok(SipAuthService::new()
        .with_digest_provider("pbx.example.com", digest_provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256)
        .with_digest_replay_store(redis.clone())
        .with_rate_limiter(redis))
}
```

If your Digest secrets come from users-core or another database, use that
provider for `with_digest_provider(...)` and Redis only for
`with_digest_replay_store(...)` / `with_rate_limiter(...)`.

## Users-Core Storage

SQLite is the default users-core reference store and remains the simplest local
development option.

PostgreSQL support is available behind the `postgres` feature for
`UserStore`, `ApiKeyStore`, and auth-service security storage:

```bash
RVOIP_USERS_POSTGRES_URL='postgresql:///postgres?host=/tmp' \
  cargo test -p rvoip-users-core --features postgres --test postgres_store_tests
```

Supported auth-service backing:

- refresh-token storage and revocation;
- access-token JTI revocation checks;
- password hash updates and last-login updates;
- SIP Digest HA1 credential create, rotate, lookup, and delete.

SQLite remains the default reference/dev store. Use PostgreSQL when production
operational requirements call for an external database, or implement
`auth-core` provider traits directly against an existing identity system.

## Keycloak And Generic OIDC Bearer

Use OIDC Bearer for enterprise identity provider integration. Prefer JWKS
validation for JWT access tokens and introspection for opaque tokens or when
immediate revocation is required.

Local Keycloak fixture:

```bash
cd ~/Developer/keycloak
docker compose up -d
RVOIP_KEYCLOAK_ENV=~/Developer/keycloak/keycloak-local.env \
  cargo test -p rvoip-keycloak --test keycloak_live
```

Generic OIDC discovery:

```bash
RVOIP_OIDC_ISSUER=https://idp.example.com/realms/rvoip \
RVOIP_OIDC_AUDIENCE=rvoip-sip \
  cargo run -p rvoip-sip --example auth_generic_oidc_provider
```

Deployment requirements:

- Pin issuer and audience.
- Configure JWKS cache TTLs and key rotation procedures.
- Use introspection when access-token revocation must take effect immediately.
- Map only required scopes/roles into SIP authorization decisions.

## OpenLDAP Basic Over TLS

Basic auth exists for legacy compatibility. Use it only over TLS/WSS or LDAPS
backed verification, and prefer stronger schemes for new systems.

Local OpenLDAP fixture:

```bash
cd ~/Developer/openldap
docker compose up -d
RVOIP_LDAP_URL=ldap://127.0.0.1:1389 \
RVOIP_LDAP_BIND_DN='cn=admin,dc=rvoip,dc=local' \
RVOIP_LDAP_BIND_PASSWORD=adminpassword \
RVOIP_LDAP_USER_BASE_DN='ou=users,dc=rvoip,dc=local' \
  cargo test -p rvoip-ldap
```

Production requirements:

- Prefer LDAPS or StartTLS.
- Use a least-privilege bind account.
- Keep `allow_basic_over_cleartext(true)` out of production.
- Rate-limit Basic attempts and audit failures without logging passwords.

## Active Directory Notes

The OpenLDAP verifier covers LDAP simple-bind behavior. Active Directory
compatibility needs separate validation because AD deployments often add domain
policy, lockout behavior, referrals, UPN formats, and group/role mapping rules.

Recommended AD posture:

- Use LDAPS.
- Validate against Samba AD DC or a real AD lab before claiming AD support.
- Map AD groups to SIP scopes outside protocol code.
- Keep account lockout and password-expiry behavior controlled by AD policy.

## Verification Checklist

- `cargo test -p rvoip-sip --test register_423_retry`
- `cargo test -p rvoip-sip --test oob_auth_retry`
- `cargo test -p rvoip-sip --test update_notify_auth_retry`
- `cargo test -p rvoip-sip --features generated-validation --test generated_sip_compliance`
- `cargo test -p rvoip-sip --test endpoint_unified_auth`
- `cargo test -p rvoip-users-core --features auth-core --test auth_core_bridge_tests`
- `RVOIP_USERS_POSTGRES_URL='postgresql:///postgres?host=/tmp' cargo test -p rvoip-users-core --features postgres --test postgres_store_tests`
- `RVOIP_REDIS_URL=redis://127.0.0.1:6379 cargo test -p rvoip-redis --test redis_live`
- `cargo test -p rvoip-keycloak --test keycloak_live`
- `cargo test -p rvoip-ldap`
