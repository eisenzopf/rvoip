# rvoip-sip-proxy

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip-proxy.svg)](https://crates.io/crates/sip/rvoip-sip-proxy)
[![Documentation](https://docs.rs/rvoip-sip-proxy/badge.svg)](https://docs.rs/rvoip-sip-proxy)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

Transaction-stateful SIP proxy primitives for
[rvoip](https://github.com/eisenzopf/rvoip). Provides the
`StatefulProxy`, target-set processing, response-context aggregation,
and Via handling consumed by the
[`rvoip-sip`](https://crates.io/crates/sip/rvoip-sip) umbrella API.

## Status

**Partial RFC 3261 stateful-proxy implementation.** Published `0.3.3`
does not carry a bounded proxy-conformance claim. The coordinated
`0.3.4` candidate adds support for
transaction-stateful forwarding, parallel/sequential forking, Via and
Max-Forwards processing, response aggregation, CANCEL propagation, and
Timer C. Those features do not yet constitute a qualified RFC profile.

The applicable normative behavior, known gaps, and executable evidence
required for a bounded claim are tracked in
[`docs/RFC3261_CONFORMANCE.md`](docs/RFC3261_CONFORMANCE.md). Baseline and
gate provenance are in
[`docs/CONFORMANCE_STATUS.md`](docs/CONFORMANCE_STATUS.md). A green unit
suite alone must not be represented as RFC conformance.

## Install

You usually don't depend on this directly — depend on
[`rvoip-sip`](https://crates.io/crates/sip/rvoip-sip) which re-exports the
proxy primitives behind its `server::*` and `adapter::*` modules. If
you want the raw transaction-layer primitives:

```toml
[dependencies]
rvoip-sip-proxy = "0.3.4"
```

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
