# rvoip — Universal real-time gateway library

[![Crates.io](https://img.shields.io/crates/v/rvoip.svg)](https://crates.io/crates/rvoip)
[![Documentation](https://docs.rs/rvoip/badge.svg)](https://docs.rs/rvoip)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

> ⚠️ **Beta (`0.2.0-beta.x`).** This release exposes the **SIP product** through the facade. APIs may shift before `0.2.0` — pin an exact version. The WebRTC, UCTP (QUIC/WebTransport/WebSocket), identity, and client substrates live in the workspace but are **not surfaced by this beta** and return in a later release.

`rvoip` is the **facade crate** for the rvoip workspace. It re-exports — behind cargo features so you pull in only what you need — the SIP product:

- the **voip-3 substrate** (`rvoip-core`'s `Orchestrator` + the shared `Conversation`/`Session`/`Connection`/`Stream`/`Message`/`Participant` model),
- the **SIP** interop adapter (`rvoip-sip`) — full RFC 3261 stack + RTP media,
- the **vCon** conversation-container builder and the in-process **AI voice harness**.

One process, one `Orchestrator`, bridged through a single conversation model.

## Quick start

```toml
[dependencies]
rvoip = "0.2.0-beta.1"   # default features: sip + vcon
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

For building a SIP softphone with microphone/speaker audio, see the `rvoip-sip` examples — [`sip_client`](../sip/rvoip-sip/examples/sip_client) (a full terminal softphone with CPAL device I/O) and [`pbx`](../sip/rvoip-sip/examples/pbx).

## Cargo features

| Feature | Default | Pulls in |
|---|:---:|---|
| `sip` | ✅ | SIP interop adapter (`rvoip::sip`) |
| `vcon` | ✅ | vCon container builder + JWS signing (`rvoip::vcon`) |
| `harness` | | In-process AI voice harness — ASR / TTS / dialog (`rvoip::harness`) |
| `full` | | Everything above |

Disable defaults to slim the build, e.g. SIP only without vCon:

```toml
rvoip = { version = "0.2.0-beta.1", default-features = false, features = ["sip"] }
```

## Module layout

| Path | Re-exports | Source crate |
|---|---|---|
| `rvoip::{Orchestrator, Config}` | command/event spine | [`rvoip-core`](../foundation/rvoip-core) |
| `rvoip::core_traits` | voip-3 nouns + traits | [`rvoip-core-traits`](../foundation/rvoip-core-traits) |
| `rvoip::sip` | SIP/RTP adapter + `UnifiedCoordinator` | [`rvoip-sip`](../sip/rvoip-sip) |
| `rvoip::vcon` | vCon builder + store + signing | [`rvoip-vcon`](../extensions/rvoip-vcon) |
| `rvoip::harness` | ASR / TTS / DialogManager traits | [`rvoip-harness`](../extensions/rvoip-harness) |

## Crate map

**Beta — published to crates.io as `0.2.0-beta.1`:**

| Crate | Role |
|---|---|
| [`rvoip`](.) | this facade (SIP product surface) |
| [`rvoip-core`](../foundation/rvoip-core) | Orchestrator, cross-adapter bridging, conversation/session state |
| [`rvoip-core-traits`](../foundation/rvoip-core-traits) | shared voip-3 nouns + traits (zero `rvoip-*` deps) |
| [`rvoip-infra-common`](../foundation/infra-common) | event bus, config, lifecycle |
| [`rvoip-media-core`](../media/media-core) · [`rvoip-rtp-core`](../media/rtp-core) · [`rvoip-codec-core`](../media/codec-core) | media engine, RTP/RTCP/SRTP, codecs |
| [`rvoip-sip`](../sip/rvoip-sip) + [`sip-core`](../sip/sip-core) · [`sip-transport`](../sip/sip-transport) · [`sip-dialog`](../sip/sip-dialog) · [`sip-proxy`](../sip/sip-proxy) · [`sip-registrar`](../sip/sip-registrar) | full SIP stack (RFC 3261) |
| [`rvoip-auth-core`](../identity/auth-core) | OAuth2 / JWT / DPoP / SIP-Digest validators |

**Published at `0.1.0-alpha.1`** (consumed as optional deps of `rvoip-core`): [`rvoip-vcon`](../extensions/rvoip-vcon) · [`rvoip-harness`](../extensions/rvoip-harness).

**In the workspace, not published in this release:** [`rvoip-webrtc`](../webrtc/rvoip-webrtc), the UCTP family ([`rvoip-uctp`](../uctp/rvoip-uctp) · [`rvoip-quic`](../uctp/rvoip-quic) · [`rvoip-webtransport`](../uctp/rvoip-webtransport) · [`rvoip-websocket`](../uctp/rvoip-websocket)), [`rvoip-identity`](../identity/rvoip-identity) · [`users-core`](../identity/users-core), [`rvoip-stir-shaken`](../extensions/rvoip-stir-shaken), and [`rvoip-client`](../rvoip-client).

## Documentation

- API docs: [docs.rs/rvoip](https://docs.rs/rvoip)
- Workspace overview: [repository README](../../README.md)
- Architecture & protocol design: [`PRD.md`](../../docs/PRD.md), [`INTERFACE_DESIGN.md`](../../docs/INTERFACE_DESIGN.md), [`CONVERSATION_PROTOCOL.md`](../../docs/CONVERSATION_PROTOCOL.md)

## License

Licensed under the [MIT License](../../LICENSE).
