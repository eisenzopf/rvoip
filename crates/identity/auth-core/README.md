# rvoip-auth-core

[![Crates.io](https://img.shields.io/crates/v/rvoip-auth-core.svg)](https://crates.io/crates/rvoip-auth-core)
[![Documentation](https://docs.rs/rvoip-auth-core/badge.svg)](https://docs.rs/rvoip-auth-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

OAuth2 and token-based authentication primitives for [rvoip](https://github.com/eisenzopf/rvoip).
Used by `rvoip-sip-registrar`, `rvoip-vcon` (JWS signing), and any rvoip
service that authenticates incoming requests via Bearer tokens or RFC 8898
SIP/OAuth profiles.

This crate depends on the trait-only `rvoip-core-traits`, not on
`rvoip-core` itself — that's what breaks the
`rvoip-core` → `rvoip-vcon` → `rvoip-auth-core` → `rvoip-core` cycle and
lets `rvoip-core` take `rvoip-vcon` as an optional dep.

## Status

**Beta candidate** — part of the `rvoip-sip` 0.2.x beta train. API may
adjust for incoming review feedback before 0.2.x beta train ships, but no
restructure is planned.

## Install

```toml
[dependencies]
rvoip-auth-core = "0.2.2"
```

## Where to start

- Token verification: implement [`BearerValidator`](src/bearer.rs) or use
  `JwtValidator`, `JwksJwtValidator`, or `OAuth2IntrospectionValidator`.
  Existing validators that implement only `validate` remain compatible.
- Credential-aware integrations should call `validate_credential`. Its
  `ValidatedBearer` result retains the complete principal plus an optional
  bounded token ID and `SystemTime` issue time. JWT/JWKS use validated
  `jti`/`iat` claims. Introspection accepts `jti` or `token_id` and derives a
  SHA-256 credential fingerprint only when the provider supplies neither.
  Token IDs and fingerprints are correlation-sensitive and must never be
  logged; `ValidatedBearer` redacts them from `Debug` output.
- Production JWT/JWKS deployments can enable `with_required_jti`; configuring
  a revocation checker also requires `jti`. Introspection deployments can use
  `with_required_token_id` when a provider-issued identifier is mandatory.
- Integration examples live in the [rvoip-sip
  README](../../sip/rvoip-sip/README.md) and in
  [`crates/sip/rvoip-sip/examples/callback_peer/`](../../sip/rvoip-sip/examples/callback_peer/).

## Clustered SIP Digest replay migration

Existing `DigestReplayStore` implementations remain source compatible through
the original `record_nonce`, `nonce_status`, and `(username, nonce, cnonce)`
`accept_nonce_count` methods. Secure clustered listeners additionally call two
additive methods:

- `admit_nonce` atomically bounds challenge state and may return an existing
  active nonce when the tenant pool is full.
- `accept_client_nonce_count` atomically verifies nonce activity and tracks a
  monotonic `(username, nonce, cnonce)` sequence with fair cardinality limits.

Their default implementations deliberately return `PolicyRejected`. A legacy
store therefore compiles but fails closed when selected for a secure clustered
listener until it implements the new contract. Use `RedisAuthProvider` as the
first-party implementation and configure one namespace/provider per tenant.

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
