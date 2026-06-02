# rvoip — Universal real-time gateway library

[![Crates.io](https://img.shields.io/crates/v/rvoip.svg)](https://crates.io/crates/rvoip)
[![Documentation](https://docs.rs/rvoip/badge.svg)](https://docs.rs/rvoip)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

> **Maturity tiers (plain numeric — no `-alpha`/`-beta` suffixes):** `0.1.x` = alpha,
> `0.2.x` = beta, `1.0` = stable. The **`sip`** surface is **beta (`0.2.0`)**; the other
> surfaces (`webrtc`, `uctp`, the `voip-3` extensions, `client`) are **alpha (`0.1.0`)** —
> expect breaking changes before `1.0`. Pin exact versions.

`rvoip` is the **facade crate** for the rvoip workspace. It always compiles the **voip-3
substrate** (`rvoip-core`'s `Orchestrator` + the shared `Conversation`/`Session`/
`Connection`/`Stream`/`Message`/`Participant` model) and lets you opt into transports and
extensions behind cargo features — defaulting to the SIP product. One process, one
`Orchestrator`, many protocols, bridged through a single conversation model.

## Quick start

```toml
[dependencies]
rvoip = "0.2.0"   # default feature: sip
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

The unifying nouns are re-exported at the crate root from [`rvoip-core-traits`](../foundation/rvoip-core-traits) as `rvoip::core_traits`.
For a SIP softphone with microphone/speaker audio, see the `rvoip-sip` examples — [`sip_client`](../sip/rvoip-sip/examples/sip_client) (a terminal softphone with CPAL device I/O) and [`pbx`](../sip/rvoip-sip/examples/pbx).

## Cargo features

| Feature | Default | Tier | Pulls in |
|---|:---:|:---:|---|
| `sip` | ✅ | beta | SIP interop adapter (`rvoip::sip`) |
| `webrtc` | | alpha | WebRTC interop adapter (`rvoip::webrtc`) |
| `uctp` | | alpha | UCTP substrate adapters — QUIC / WebTransport / WebSocket (`rvoip::uctp`) |
| `sip-stir-shaken` | | alpha | RFC 8224 caller-ID attestation; **requires `sip`** (`rvoip::stir_shaken`) |
| `voip-3` | | alpha | The full experience: every transport **+** the vCon / identity / AI-harness extensions |
| `client` | | alpha | Cross-transport client SDK (`rvoip::client`) |
| `full` | | | `voip-3` + `sip-stir-shaken` + `client` |

The transport-agnostic conversation-model extensions — **vCon** emission, **identity**
backends, and the **AI harness** — are reachable only through the `voip-3` feature.

```toml
# e.g. the full multi-transport rvoip-3 experience
rvoip = { version = "0.2.0", features = ["voip-3"] }
```

## Module layout

| Path | Feature | Source crate |
|---|---|---|
| `rvoip::{Orchestrator, Config}` | always | [`rvoip-core`](../foundation/rvoip-core) |
| `rvoip::core_traits` | always | [`rvoip-core-traits`](../foundation/rvoip-core-traits) |
| `rvoip::sip` | `sip` | [`rvoip-sip`](../sip/rvoip-sip) |
| `rvoip::stir_shaken` | `sip-stir-shaken` | [`rvoip-stir-shaken`](../extensions/rvoip-stir-shaken) |
| `rvoip::webrtc` | `webrtc` | [`rvoip-webrtc`](../webrtc/rvoip-webrtc) |
| `rvoip::uctp` (`::protocol`/`::quic`/`::webtransport`/`::websocket`) | `uctp` | [`rvoip-uctp`](../uctp/rvoip-uctp) + substrates |
| `rvoip::vcon` / `::identity` / `::harness` | `voip-3` | [`rvoip-vcon`](../extensions/rvoip-vcon) · [`rvoip-identity`](../identity/rvoip-identity) · [`rvoip-harness`](../extensions/rvoip-harness) |
| `rvoip::client` | `client` | [`rvoip-client`](../rvoip-client) |

## Crate map

**Beta — published at `0.2.0`** (the SIP product + shared spine): `rvoip` (facade),
[`rvoip-core`](../foundation/rvoip-core), [`rvoip-core-traits`](../foundation/rvoip-core-traits),
[`rvoip-infra-common`](../foundation/infra-common), the media engine
([`rvoip-media-core`](../media/media-core) · [`rvoip-rtp-core`](../media/rtp-core) · [`rvoip-codec-core`](../media/codec-core)),
the SIP stack ([`rvoip-sip`](../sip/rvoip-sip) + [`sip-core`](../sip/sip-core) · [`sip-transport`](../sip/sip-transport) · [`sip-dialog`](../sip/sip-dialog) · [`sip-proxy`](../sip/sip-proxy) · [`sip-registrar`](../sip/sip-registrar)),
and [`rvoip-auth-core`](../identity/auth-core).

**Alpha — published at `0.1.0`** (opt-in surfaces): [`rvoip-webrtc`](../webrtc/rvoip-webrtc);
the UCTP family ([`rvoip-uctp`](../uctp/rvoip-uctp) · [`rvoip-quic`](../uctp/rvoip-quic) · [`rvoip-webtransport`](../uctp/rvoip-webtransport) · [`rvoip-websocket`](../uctp/rvoip-websocket));
[`rvoip-vcon`](../extensions/rvoip-vcon) · [`rvoip-harness`](../extensions/rvoip-harness) · [`rvoip-stir-shaken`](../extensions/rvoip-stir-shaken);
[`rvoip-identity`](../identity/rvoip-identity) · [`rvoip-users-core`](../identity/users-core); [`rvoip-client`](../rvoip-client).

## Documentation

- API docs: [docs.rs/rvoip](https://docs.rs/rvoip)
- Workspace overview: [repository README](../../README.md)
- Architecture & protocol design: [`PRD.md`](../../docs/PRD.md), [`INTERFACE_DESIGN.md`](../../docs/INTERFACE_DESIGN.md), [`CONVERSATION_PROTOCOL.md`](../../docs/CONVERSATION_PROTOCOL.md)

## License

Licensed under the [MIT License](../../LICENSE).
