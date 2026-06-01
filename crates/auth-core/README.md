# rvoip-auth-core

[![Crates.io](https://img.shields.io/crates/v/rvoip-auth-core.svg)](https://crates.io/crates/rvoip-auth-core)
[![Documentation](https://docs.rs/rvoip-auth-core/badge.svg)](https://docs.rs/rvoip-auth-core)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/eisenzopf/rvoip)

OAuth2 and token-based authentication primitives for [rvoip](https://github.com/eisenzopf/rvoip).
Used by `rvoip-sip-registrar`, `rvoip-vcon` (JWS signing), and any rvoip
service that authenticates incoming requests via Bearer tokens or RFC 8898
SIP/OAuth profiles.

This crate depends on the trait-only `rvoip-core-traits`, not on
`rvoip-core` itself — that's what breaks the
`rvoip-core` → `rvoip-vcon` → `rvoip-auth-core` → `rvoip-core` cycle and
lets `rvoip-core` take `rvoip-vcon` as an optional dep.

## Status

**Beta candidate** — part of the `rvoip-sip` 0.2.0-beta closure. API may
adjust for incoming review feedback before 0.2.0 ships, but no
restructure is planned.

## Install

```toml
[dependencies]
rvoip-auth-core = "0.2.0-beta.1"
```

## Where to start

- Token verification: see [`bearer_stub`](src/bearer_stub.rs) for the
  minimal JWK/JWS verifier `rvoip-vcon` and `rvoip-sip-registrar` plug
  into.
- Integration examples live in the [rvoip-sip
  README](../rvoip-sip/README.md) and in
  [`crates/rvoip-sip/examples/callback_peer/`](../rvoip-sip/examples/callback_peer/).

## License

Licensed under either of MIT or Apache-2.0 at your option.
