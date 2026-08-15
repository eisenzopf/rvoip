# rvoip-codec-core

[![Crates.io](https://img.shields.io/crates/v/rvoip-codec-core.svg)](https://crates.io/crates/rvoip-codec-core)
[![Documentation](https://docs.rs/rvoip-codec-core/badge.svg)](https://docs.rs/rvoip-codec-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

G.711 (μ-law / A-law) plus optional G.729A/G.729AB, Opus and AMR-NB/AMR-WB
audio codec implementations for
[rvoip](https://github.com/eisenzopf/rvoip). Pulled in transitively by
`rvoip-media-core` to provide the baseline narrow-band codec every
beta-tier SIP profile requires (RFC 3551), and the optional codecs above it.

## Status

**Release-gated SIP dependency** — published in the unified `0.3.x` workspace
release. The G.711 implementation is RFC-compliant and table-driven, and is the
only codec in the default build; everything else is feature-gated.

| Codec | Feature | State |
| --- | --- | --- |
| G.711 μ-law / A-law | `g711`, default | RFC-compliant, table-driven |
| G.729A / G.729AB | `g729` | Annex A speech plus optional Annex B VAD/DTX/CNG; full-complexity base G.729 is not implemented |
| Opus | `opus` | Backed by libopus, which must be present on the build host |
| AMR-NB | `amr-nb` | All 8 modes, RFC 4867 framing, DTX, CMR |
| AMR-WB (G.722.2) | `amr-wb` | All 9 modes, RFC 4867 framing, DTX, CMR |

`amr` enables both AMR variants; `all-codecs` enables everything.

### AMR

Both AMR variants are complete: encoders and decoders bit-exact against the
3GPP reference implementations over the committed fixtures, the normative test
sequences the reference distributions ship, RFC 4867 octet-aligned and
bandwidth-efficient framing checked against Wireshark's dissector, IF1 and IF2
interface formats, DTX with both narrowband voice-activity detectors, comfort
noise, homing, concealment, and live calls through Asterisk, FreeSWITCH,
Kamailio and OpenSIPS.

Bit-exactness against the reference is not the same as certification, and the
evidence has boundaries worth reading before you rely on it. Both are set out
in [`docs/AMR_IMPLEMENTATION_STATUS.md`](docs/AMR_IMPLEMENTATION_STATUS.md).

No 3GPP material is redistributed in this repository. The reference sources and
specifications are fetched to develop against; only generated output is
committed.

## Install

You usually don't depend on this directly — depend on
[`rvoip-media-core`](https://crates.io/crates/rvoip-media-core) which
re-exports the codec surface. If you need the codecs in isolation:

```toml
[dependencies]
rvoip-codec-core = "0.3.8"
```

With AMR:

```toml
[dependencies]
rvoip-codec-core = { version = "0.3.5", features = ["amr"] }
```

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
