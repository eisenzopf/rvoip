# Migrating users-core 0.1 to 0.2

users-core 0.2 is a coordinated security-boundary release. It intentionally
does not present the tenant-claim additions as a patch-compatible change.

## Public Rust API

- `JwtConfig` includes `tenant_id`. Prefer `JwtConfig::new(...)` followed by
  `with_tenant_id`, `with_signing_key`, and `with_token_ttls` instead of a
  struct literal.
- `UserClaims` includes `tenant_id`. Prefer `UserClaims::new(...)` and its
  builder methods when constructing test or interoperability claims.
- `RateLimitError` is non-exhaustive. Add a wildcard arm to external matches.

The workspace dependency is pinned to `rvoip-users-core = "0.2.0"` so internal
consumers cannot accidentally resolve the old public contract.

## Token migration

A configured `jwt.tenant_id` is copied into access and refresh tokens and is
matched exactly during direct validation and refresh exchange. A tenant-bound
issuer rejects tokens with a missing or different tenant. An unbound issuer
also rejects tenant-bearing tokens.

Refresh tokens minted by 0.1 do not contain a tenant. When upgrading a
tenant-bound deployment, expire existing sessions or schedule a
reauthentication window. There is deliberately no compatibility flag that
silently accepts tenantless refresh tokens in a tenant-bound service.

## REST embedding

Custom servers must use one of:

- `api::create_make_service`
- `api::create_make_service_with_state`
- `api::into_peer_aware_make_service`
- Axum's equivalent `into_make_service_with_connect_info::<SocketAddr>()`

A bare router has no trustworthy peer identity and now returns `503` instead
of sharing an `unknown` rate-limit bucket. API keys cannot call `/auth/logout`;
revoke a key explicitly with `DELETE /api-keys/:id`.

## Rate-limit identity behavior

Native IPv6 identities are normalized to `/64`, IPv4-mapped IPv6 is folded
into canonical IPv4, and forwarding headers are honored only for declaratively
configured trusted proxy CIDRs. Saturated exact maps use a bounded,
process-secret hashed overflow tier and do not evict active lockout state.
Cleanup is scheduled on access and no background task retains a dropped
limiter or its maps.
