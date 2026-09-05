# RVoIP facade feature bundles

RVoIP's Cargo features remain composable leaf features. The additive
`bundle-*` features are stable starting points for common deployment shapes;
they do not replace or rename any existing feature.

This document is generated from `crates/rvoip/Cargo.toml` by
`scripts/ci/check_facade_feature_bundles.py`. Edit the manifest metadata and
run the script with `--write`; CI rejects hand-edited or drifting output.

## Bundle matrix

| Cargo feature | Deployment shape | Direct members | Audio codecs | Extra system dependency | Maturity |
| --- | --- | --- | --- | --- | --- |
| `bundle-sip-endpoint` | SIP endpoint | `sip` | G.711 mu-law and A-law | None | SIP beta |
| `bundle-carrier-sip` | Carrier SIP | `sip`, `sip-stir-shaken`, `g729`, `amr` | G.711, G.729, AMR-NB, and AMR-WB | None | SIP beta plus preview add-ons |
| `bundle-browser-gateway` | Browser gateway | `app`, `opus` | G.711 and Opus | libopus | Developer preview |
| `bundle-ai-conversation` | AI conversation gateway | `voip-3`, `vapi`, `app`, `opus` | G.711 and Opus | libopus | Developer preview |
| `bundle-full-pure-rust` | Full pure-Rust facade | `full` | G.711, G.729, AMR-NB, and AMR-WB | None | Mixed: SIP beta and preview surfaces |
| `bundle-full-native` | Full facade with native codecs | `full`, `opus` | G.711, G.729, AMR-NB, AMR-WB, and Opus | libopus | Mixed: SIP beta and preview surfaces |

G.711 mu-law and A-law are the baseline SIP codecs and need no opt-in codec
feature. AMR-NB, AMR-WB, and G.729 are pure-Rust implementations. Opus is a
first-class codec, but its current backend links `libopus`, so bundles that
include it say so explicitly.

## Choosing a bundle

### `bundle-sip-endpoint` — SIP endpoint

A provider-neutral SIP endpoint or server with the standard G.711 codecs.

### `bundle-carrier-sip` — Carrier SIP

A carrier-facing SIP service with caller-identity attestation and the pure-Rust telephony codec set.

### `bundle-browser-gateway` — Browser gateway

The high-level SIP, WebRTC, and UCTP gateway used to connect browser media to telephony.

### `bundle-ai-conversation` — AI conversation gateway

Cross-transport conversations, identity, AI harness, Vapi, and the high-level gateway.

### `bundle-full-pure-rust` — Full pure-Rust facade

Every facade surface and pure-Rust codec without a native codec library.

### `bundle-full-native` — Full facade with native codecs

Every facade surface and every mainline audio codec, including native Opus.

## Cargo examples

```toml
# Small provider-neutral SIP service.
rvoip = { version = "0.3.9", default-features = false, features = ["bundle-sip-endpoint"] }

# Carrier-facing service with the pure-Rust telephony codec set.
rvoip = { version = "0.3.9", default-features = false, features = ["bundle-carrier-sip"] }

# Browser-to-SIP application gateway; install libopus on the build host.
rvoip = { version = "0.3.9", default-features = false, features = ["bundle-browser-gateway"] }
```

Advanced users may continue selecting leaf features directly. Start from
`default-features = false` when the dependency graph must contain only the
surfaces named by the application.
