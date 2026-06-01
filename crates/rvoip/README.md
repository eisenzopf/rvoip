# rvoip — Universal real-time gateway library

[![Crates.io](https://img.shields.io/crates/v/rvoip.svg)](https://crates.io/crates/rvoip)
[![Documentation](https://docs.rs/rvoip/badge.svg)](https://docs.rs/rvoip)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

> ⚠️ **Alpha.** The `rvoip` facade is held at `0.1.0-alpha.1` (`publish = false`) until the SIP product stabilizes and more substrates reach beta. APIs are unstable. Several underlying crates are already published at beta (see [Crate map](#crate-map)).

`rvoip` is the **facade crate** for the rvoip workspace. It re-exports — behind cargo features so you pull in only what you need — the pieces of a real-time communications gateway:

- the **voip-3 substrate** (`rvoip-core`'s `Orchestrator` + the shared `Conversation`/`Session`/`Connection`/`Stream`/`Message`/`Participant` model),
- the **UCTP** protocol and its QUIC / WebTransport / WebSocket substrate adapters,
- the **SIP** and **WebRTC** interop adapters,
- the **vCon** conversation-container builder, the in-process **AI voice harness**, and pluggable **identity** backends.

One process, one `Orchestrator`, many protocols — bridged through a single conversation model.

## Quick start

```toml
[dependencies]
rvoip = "0.1"   # default features: uctp + sip + vcon + identity
```

```rust
use rvoip::{Orchestrator, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `Orchestrator::new` returns an `Arc<Orchestrator>`.
    let orchestrator = Orchestrator::new(Config::default());

    // Register interop adapters (e.g. a SIP adapter built from a configured
    // `rvoip::sip::UnifiedCoordinator`) with `orchestrator.register(adapter)?`.

    let mut events = orchestrator.subscribe_events();
    while let Ok(event) = events.recv().await {
        // handle each orchestrator event (inbound call, bridge, recording, …)
        drop(event);
    }
    Ok(())
}
```

The unifying nouns (`Conversation`, `Session`, `Connection`, `Stream`, `Message`, `Participant`) are re-exported at the crate root from [`rvoip-core-traits`](../foundation/rvoip-core-traits) as `rvoip::core_traits`.

## Cargo features

| Feature | Default | Pulls in |
|---|:---:|---|
| `uctp` | ✅ | UCTP protocol + QUIC, WebTransport & WebSocket substrate adapters |
| `sip` | ✅ | SIP interop adapter (`rvoip::sip`) |
| `vcon` | ✅ | vCon container builder + JWS signing (`rvoip::vcon`) |
| `identity` | ✅ | Identity provider backends (`rvoip::identity`) |
| `webrtc` | | WebRTC interop adapter (`rvoip::webrtc`) |
| `harness` | | In-process AI voice harness — ASR / TTS / dialog (`rvoip::harness`) |
| `client` | | Single-Identity client SDK (`rvoip::client`) |
| `aauth-experimental` | | AAuth identity backend (experimental) |
| `identity-fingerprint-binding` | | DTLS-SRTP fingerprint binding |
| `full` | | Everything above |

Disable defaults to slim the build, e.g. a WebRTC-only gateway:

```toml
rvoip = { version = "0.1", default-features = false, features = ["webrtc"] }
```

## Module layout

Per-protocol native surfaces live under their own module; the unifying surface sits at the crate root.

| Path | Re-exports | Source crate |
|---|---|---|
| `rvoip::{Orchestrator, Config}` | command/event spine | [`rvoip-core`](../foundation/rvoip-core) |
| `rvoip::core_traits` | voip-3 nouns + traits | [`rvoip-core-traits`](../foundation/rvoip-core-traits) |
| `rvoip::sip` | SIP/RTP adapter + `UnifiedCoordinator` | [`rvoip-sip`](../sip/rvoip-sip) |
| `rvoip::webrtc` | DTLS-SRTP / ICE peer adapter | [`rvoip-webrtc`](../webrtc/rvoip-webrtc) |
| `rvoip::uctp::protocol` | envelope, capability negotiation, state machine | [`rvoip-uctp`](../uctp/rvoip-uctp) |
| `rvoip::uctp::quic` / `::webtransport` / `::websocket` | substrate adapters | [`rvoip-quic`](../uctp/rvoip-quic) · [`rvoip-webtransport`](../uctp/rvoip-webtransport) · [`rvoip-websocket`](../uctp/rvoip-websocket) |
| `rvoip::vcon` | vCon builder + store + signing | [`rvoip-vcon`](../extensions/rvoip-vcon) |
| `rvoip::harness` | ASR / TTS / DialogManager traits | [`rvoip-harness`](../extensions/rvoip-harness) |
| `rvoip::identity` | `IdentityProvider` backends | [`rvoip-identity`](../identity/rvoip-identity) |
| `rvoip::client` | single-Identity client SDK | [`rvoip-client`](../rvoip-client) |

## Architecture

`rvoip` is a thin re-export layer. The real work is split into role/product crate groups; protocol adapters plug into the `rvoip-core` orchestrator and speak the shared media plane.

```
                       ┌──────────────────────────────────────┐
                       │   rvoip  (this crate — the facade)    │
                       │        Orchestrator · Config          │
                       └───────────────────┬──────────────────┘
        ┌──────────────┬───────────────────┼───────────────────┬──────────────┐
        ▼              ▼                   ▼                   ▼              ▼
     sip/           webrtc/              uctp/             identity/     extensions/
  SIP product    WebRTC interop     UCTP protocol +     auth-core ·     vcon ·
  rvoip-sip +    rvoip-webrtc       QUIC/WT/WS          users-core ·    harness ·
  sip-core/…     (DTLS-SRTP/ICE)    substrate adapters  rvoip-identity  stir-shaken
        └──────────────┴───────────────────┼───────────────────┴──────────────┘
                                            ▼
              ┌────────────────────────────────────────────────────────┐
              │  foundation/  rvoip-core · rvoip-core-traits · infra-common
              │  media/       media-core · codec-core · rtp-core         │
              └────────────────────────────────────────────────────────┘
```

### Crate map

**Beta — published to crates.io as `0.2.0-beta.1`:**

| Crate | Role |
|---|---|
| [`rvoip-core`](../foundation/rvoip-core) | Orchestrator, cross-adapter bridging, conversation/session state |
| [`rvoip-core-traits`](../foundation/rvoip-core-traits) | shared voip-3 nouns + traits (zero `rvoip-*` deps) |
| [`rvoip-infra-common`](../foundation/infra-common) | event bus, config, lifecycle |
| [`rvoip-media-core`](../media/media-core) · [`rvoip-rtp-core`](../media/rtp-core) · [`rvoip-codec-core`](../media/codec-core) | media engine, RTP/RTCP/SRTP, codecs |
| [`rvoip-sip`](../sip/rvoip-sip) + [`sip-core`](../sip/sip-core) · [`sip-transport`](../sip/sip-transport) · [`sip-dialog`](../sip/sip-dialog) · [`sip-proxy`](../sip/sip-proxy) · [`sip-registrar`](../sip/sip-registrar) | full SIP stack (RFC 3261) |
| [`rvoip-auth-core`](../identity/auth-core) | OAuth2 / JWT / DPoP / SIP-Digest validators |

**Alpha — in the workspace (most are `publish = false`):**

| Crate | Role |
|---|---|
| [`rvoip-uctp`](../uctp/rvoip-uctp) + [`rvoip-quic`](../uctp/rvoip-quic) · [`rvoip-webtransport`](../uctp/rvoip-webtransport) · [`rvoip-websocket`](../uctp/rvoip-websocket) | UCTP signaling protocol + substrate adapters |
| [`rvoip-webrtc`](../webrtc/rvoip-webrtc) | WebRTC interop (WHIP / WebSocket signaling, RTP pumps) |
| [`rvoip-vcon`](../extensions/rvoip-vcon) · [`rvoip-harness`](../extensions/rvoip-harness) · [`rvoip-stir-shaken`](../extensions/rvoip-stir-shaken) | vCon container · AI voice harness · STIR/SHAKEN attestation |
| [`rvoip-identity`](../identity/rvoip-identity) · [`users-core`](../identity/users-core) | identity provider backends · user store |
| [`rvoip-audio-core`](../sip/audio-core) | audio device I/O for SIP clients |
| [`rvoip-client`](../rvoip-client) | single-Identity client SDK |

## Documentation

- API docs: [docs.rs/rvoip](https://docs.rs/rvoip)
- Workspace overview: [repository README](../../README.md)
- Architecture & protocol design: [`PRD.md`](../../docs/PRD.md), [`INTERFACE_DESIGN.md`](../../docs/INTERFACE_DESIGN.md), [`CONVERSATION_PROTOCOL.md`](../../docs/CONVERSATION_PROTOCOL.md)

## License

Licensed under the [MIT License](../../LICENSE).
