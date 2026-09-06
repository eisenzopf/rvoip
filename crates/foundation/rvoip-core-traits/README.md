# rvoip-core-traits

[![Crates.io](https://img.shields.io/crates/v/rvoip-core-traits.svg)](https://crates.io/crates/foundation/rvoip-core-traits)
[![Documentation](https://docs.rs/rvoip-core-traits/badge.svg)](https://docs.rs/rvoip-core-traits)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

Pure trait + type surface for the [rvoip](https://github.com/eisenzopf/rvoip)
ecosystem — IDs, errors, capability negotiation, identity contracts,
harness contracts. Has no runtime code and no transitive dependencies
on `rvoip-core` or any adapter.

The shared media surface includes `StreamSelector`, `MediaReadiness`, and
`StreamWaitError`, allowing applications and adapters to describe registered,
source-ready, and bidirectional media without importing a transport crate.

This crate exists to **break dependency cycles**. Many consumer crates
(`rvoip-auth-core`, `rvoip-harness`, `rvoip-vcon`) need to refer to
rvoip's identity / session / capability types without pulling in the
`rvoip-core` implementation, which in turn lets `rvoip-core` take those
crates as optional deps.

## Status

**Release-gated SIP dependency** — published in the unified `0.3.x` workspace release. Trait
signatures are stable; new traits may be added but existing ones
won't change shape without a 0.3 bump.

## Install

You usually don't depend on this directly — it comes transitively via
`rvoip-core`, `rvoip-auth-core`, or `rvoip-harness`. If you're
implementing your own adapter and want only the trait surface:

```toml
[dependencies]
rvoip-core-traits = "0.3.9"
```

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
