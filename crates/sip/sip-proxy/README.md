# rvoip-sip-proxy

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip-proxy.svg)](https://crates.io/crates/sip/rvoip-sip-proxy)
[![Documentation](https://docs.rs/rvoip-sip-proxy/badge.svg)](https://docs.rs/rvoip-sip-proxy)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

Stateful SIP proxy primitives (RFC 3261 §16) for
[rvoip](https://github.com/eisenzopf/rvoip). Provides the
`ProxyTransaction` state machine, target-set processing, and Record-Route
/ Via handling consumed by the
[`rvoip-sip`](https://crates.io/crates/sip/rvoip-sip) umbrella's B2BUA helpers.

## Status

**Beta candidate** — part of the `rvoip-sip` 0.2.x beta train. The
RFC 3261 §16 stateful-proxy path is covered. Forking (parallel and
sequential) and the full failure-recovery matrix are post-beta scope.

## Install

You usually don't depend on this directly — depend on
[`rvoip-sip`](https://crates.io/crates/sip/rvoip-sip) which re-exports the
proxy primitives behind its `server::*` and `adapter::*` modules. If
you want the raw transaction-layer primitives:

```toml
[dependencies]
rvoip-sip-proxy = "0.2.2"
```

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
