# rvoip-codec-core

[![Crates.io](https://img.shields.io/crates/v/rvoip-codec-core.svg)](https://crates.io/crates/rvoip-codec-core)
[![Documentation](https://docs.rs/rvoip-codec-core/badge.svg)](https://docs.rs/rvoip-codec-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

G.711 (μ-law / A-law) audio codec implementation for
[rvoip](https://github.com/eisenzopf/rvoip). Pulled in transitively by
`rvoip-media-core` to provide the baseline narrow-band codec every
beta-tier SIP profile requires (RFC 3551).

## Status

**Beta candidate** — part of the `rvoip-sip` 0.2.0-beta closure. The
G.711 implementation is RFC-compliant and table-driven; broader codec
support (Opus, G.722, G.729) is post-beta and lives in
`rvoip-media-core` behind optional features.

## Install

You usually don't depend on this directly — depend on
[`rvoip-media-core`](https://crates.io/crates/rvoip-media-core) which
re-exports the codec surface. If you need the raw G.711 tables in
isolation:

```toml
[dependencies]
rvoip-codec-core = "0.2.0-beta.1"
```

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
