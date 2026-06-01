# rvoip-core-traits

[![Crates.io](https://img.shields.io/crates/v/rvoip-core-traits.svg)](https://crates.io/crates/rvoip-core-traits)
[![Documentation](https://docs.rs/rvoip-core-traits/badge.svg)](https://docs.rs/rvoip-core-traits)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/eisenzopf/rvoip)

Pure trait + type surface for the [rvoip](https://github.com/eisenzopf/rvoip)
ecosystem — IDs, errors, capability negotiation, identity contracts,
harness contracts. Has no runtime code and no transitive dependencies
on `rvoip-core` or any adapter.

This crate exists to **break dependency cycles**. Many consumer crates
(`rvoip-auth-core`, `rvoip-harness`, `rvoip-vcon`) need to refer to
rvoip's identity / session / capability types without pulling in the
`rvoip-core` implementation, which in turn lets `rvoip-core` take those
crates as optional deps.

## Status

**Beta candidate** — part of the `rvoip-sip` 0.2.0-beta closure. Trait
signatures are stable; new traits may be added but existing ones
won't change shape without a 0.3 bump.

## Install

You usually don't depend on this directly — it comes transitively via
`rvoip-core`, `rvoip-auth-core`, or `rvoip-harness`. If you're
implementing your own adapter and want only the trait surface:

```toml
[dependencies]
rvoip-core-traits = "0.2.0-beta.1"
```

## License

Licensed under either of MIT or Apache-2.0 at your option.
