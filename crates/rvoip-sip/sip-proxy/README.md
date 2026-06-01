# rvoip-sip-proxy

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip-proxy.svg)](https://crates.io/crates/rvoip-sip-proxy)
[![Documentation](https://docs.rs/rvoip-sip-proxy/badge.svg)](https://docs.rs/rvoip-sip-proxy)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/eisenzopf/rvoip)

Stateful SIP proxy primitives (RFC 3261 §16) for
[rvoip](https://github.com/eisenzopf/rvoip). Provides the
`ProxyTransaction` state machine, target-set processing, and Record-Route
/ Via handling consumed by the
[`rvoip-sip`](https://crates.io/crates/rvoip-sip) umbrella's B2BUA helpers.

## Status

**Beta candidate** — part of the `rvoip-sip` 0.2.0-beta closure. The
RFC 3261 §16 stateful-proxy path is covered. Forking (parallel and
sequential) and the full failure-recovery matrix are post-beta scope.

## Install

You usually don't depend on this directly — depend on
[`rvoip-sip`](https://crates.io/crates/rvoip-sip) which re-exports the
proxy primitives behind its `server::*` and `adapter::*` modules. If
you want the raw transaction-layer primitives:

```toml
[dependencies]
rvoip-sip-proxy = "0.2.0-beta.1"
```

## License

Licensed under either of MIT or Apache-2.0 at your option.
