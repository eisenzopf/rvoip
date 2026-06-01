# rvoip-webrtc

WebRTC **interop adapter** for [`rvoip-core`](../../foundation/rvoip-core): terminates foreign WebRTC peers
(ICE/DTLS-SRTP, SDP offer/answer) and exposes voip-3 `Connection`s with channel-based
`MediaStream` flows.

Built on [webrtc-rs](https://webrtc.rs) **`0.20.0-alpha.1`** (Sans-I/O `rtc` core + async
`PeerConnectionBuilder` / `PeerConnectionEventHandler` API).

## Scope

- **Dual role:** gateway/interop adapter (`WebRtcAdapter` → orchestrator) **and** WebRTC server
  (WHIP/WHEP/WS signaling surfaces feeding the same adapter). See
  [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) §1.
- **In scope:** 1:1 audio + VP8 video interop, full-SDP ICE gathering, Opus + G.711,
  SCTP data channels, fixture-encoded RTP for deterministic tests, `ConnectionAdapter` for
  `Transport::WebRtc`, optional WHIP/WHEP and WebSocket JSON signalers, external TURN via
  [`IceServerConfig`](src/config.rs).
- **Out of scope / v1 gaps:** UCTP substrate (see `rvoip-websocket`), multi-party SFU,
  simulcast/SVC, trickle ICE over signaling (WS `ice-candidate` returns `NotImplemented`),
  Identity fingerprint binding, standalone TURN relay hosting. See
  [`WebRtcFeatureSupport`](src/peer/ice.rs) and `tests/webrtc_capability_gaps.rs`.

## Features

| Feature | Enables |
|---------|---------|
| `signaling-whip` | WHIP/WHEP HTTP endpoints (`signaling::whip`) |
| `signaling-ws` | WebSocket JSON SDP signaler |
| `client` | Native `WebRtcClient` surface |
| `comprehensive` | `client` + WS signaling + full WebRTC basics E2E (bidirectional audio/VP8, fixture RTP, SCTP DC chat, DTMF, gap tests) |
| `bridge-quic` | Real `rvoip-quic` cross-transport bridge demo + e2e test |

Enable both signaling features for the unified [`WebRtcServer`](src/server.rs) facade.

## Running as a WebRTC server

Dual-role deployment: one process runs WHIP/WHEP + WS signaling **and** registers the same
`WebRtcAdapter` with [`rvoip_core::Orchestrator`](../../foundation/rvoip-core).

```rust
use std::sync::Arc;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::config::Config;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

let server = WebRtcServerBuilder::new(WebRtcConfig::default())
    .with_whip("0.0.0.0:8080")
    .with_ws("0.0.0.0:8081")
    .build()
    .await?;

let orchestrator = Orchestrator::new(Config::default());
orchestrator.register(server.adapter() as Arc<dyn ConnectionAdapter>)?;

// Subscribe to orchestrator events; on ConnectionInbound call
// orchestrator.route_inbound_connection(..., InboundAction::Accept { ... })
```

Quick start:

```bash
./scripts/demo-webrtc-server.sh
# or
cargo run -p rvoip-webrtc --example webrtc_server --features signaling-whip,signaling-ws
```

### Bridge demo (Phase 9 — mock QUIC leg)

WHIP publish → orchestrator → synthetic QUIC leg (frame pump). Lightweight stand-in
before wiring real adapters:

```bash
./scripts/demo-webrtc-bridge.sh
# or
cargo run -p rvoip-webrtc --example webrtc_bridge_demo --features signaling-whip
```

Integration test: `cargo test -p rvoip-webrtc --features signaling-whip --test webrtc_bridge_e2e`

### Real QUIC bridge demo (Phase 11)

WHIP publish → orchestrator → **`rvoip-quic::UctpQuicAdapter`** (auth + session.invite +
datagram media):

```bash
./scripts/demo-webrtc-quic-bridge.sh
# or
cargo run -p rvoip-webrtc --example webrtc_quic_bridge_demo --features bridge-quic
```

Integration test:

```bash
cargo test -p rvoip-webrtc --features bridge-quic --test webrtc_quic_bridge_e2e
```

For a full multi-adapter stack (QUIC + WT + WS + SIP), see
[`rvoip-uctp/examples/uctp_to_sip_bridge/orchestrator_bridge.rs`](../../uctp/rvoip-uctp/examples/uctp_to_sip_bridge/orchestrator_bridge.rs).

Environment variables: `WHIP_BIND` (default `127.0.0.1:8080`), `WS_BIND` (default `127.0.0.1:8081`), `QUIC_BIND` (default `127.0.0.1:4433`).

### Comprehensive WebRTC client + server (audio / video / data channel)

Exercises [`WebRtcClient`](src/client/native.rs) against [`WebRtcServer`](src/server.rs) over
WebSocket signaling — SDP (`m=audio`, `m=video`), full-gather ICE, ICE/DTLS connect, SCTP
data-channel ping/pong + arbitrary chat echo (RFC 8831), fixture-encoded Opus/VP8 RTP bursts
(server→client and client→server video), RFC 4733 DTMF, and server-side remote-track
confirmation via `stats` JSON.

Optional env: `CHAT_MESSAGE` (custom chat body), `MEDIUM` (`audio`|`video`|`audiovideo`).

```bash
./scripts/test-webrtc-comprehensive.sh
# or separately:
cargo run -p rvoip-webrtc --example webrtc_comprehensive_server --features comprehensive
WS_URL=ws://127.0.0.1:8081 CHAT_MESSAGE="Hello team" \
  cargo run -p rvoip-webrtc --example webrtc_comprehensive_client --features comprehensive -- audiovideo
```

Integration tests:

```bash
cargo test -p rvoip-webrtc --features comprehensive
```

Capability gap tests (`trickle ICE`, simulcast, TURN config, WS `ice-candidate`):
`tests/webrtc_capability_gaps.rs`.

### Server API (`src/server.rs`)

| Type | Methods |
|------|---------|
| `WebRtcServerBuilder` | `new`, `with_whip`, `with_ws`, `build` |
| `WebRtcServer` | `adapter`, `whip_addr`, `ws_addr`, `shutdown` |

## Limitations

This crate is a **1:1 WebRTC gateway/server adapter**. It deliberately does not
implement SFU/MCU fan-out — every connection is an independent peer.

- **WHEP routing is one-connection-per-subscriber.** `POST /whep/{tag}` creates
  a fresh `PeerConnection` per subscriber. The crate does not share a single
  ingest publisher across multiple subscribers — each `whep_post` allocates its
  own `connection_id` and answers with its own SDP. Use [`mediasoup`](https://mediasoup.org),
  [`Galène`](https://galene.org), or [LiveKit](https://livekit.io) when you
  need one-to-many media fan-out.
- **No simulcast layer selection.** Simulcast offers are detected (see
  `sdp_indicates_simulcast()`) but not forwarded — there is no layer-picking
  logic because there is no fan-out.
- **No mixing / MCU.** Multi-party audio mixing belongs in a media server
  layered on top.

See [`docs/GAP_PLAN.md`](docs/GAP_PLAN.md) §4 for the complete
out-of-scope list.

## Future integration

[`rvoip-websocket`](../../uctp/rvoip-websocket) may replace its stub `WebRtcMediaBridge` with types
from this crate in a follow-up PR — WebRTC expertise stays here.

See [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) for the full design.
