# rvoip-core

[![Crates.io](https://img.shields.io/crates/v/rvoip-core.svg)](https://crates.io/crates/rvoip-core)
[![Documentation](https://docs.rs/rvoip-core/badge.svg)](https://docs.rs/rvoip-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

Transport-agnostic spine for [rvoip](https://github.com/eisenzopf/rvoip).
Defines the rvoip 3 conversation model (`Conversation`, `Session`,
`Connection`, `Stream`, `Message`, `Participant`), the
`ConnectionAdapter` trait that substrate crates implement, the
`BridgeManager` for cross-substrate bridging, and the `Orchestrator`
entry point.

`rvoip-core` is **substrate-agnostic** — it never imports adapter
crates. SIP, WebRTC, QUIC, WebTransport, and WebSocket all sit *above*
`rvoip-core` and register themselves via `ConnectionAdapter`.

## Status

**Release-gated SIP dependency** — published in the unified `0.3.x` workspace release. The
type surface and `Orchestrator` are stable for the SIP path; the optional
`vcon-signing` feature (vCon JWS signing) is alpha-quality and may evolve.

### Cargo features

Default features are `uctp`, `sip`, `rtp`, `media`, `vcon`, and `identity`.
The `uctp`/`sip`/`rtp`/`media`/`identity` flags are advisory shape flags and
gate no code; `vcon` pulls the optional `rvoip-vcon` document model. The
opt-in features are:

- `vcon-signing` — enables the vCon JWS signing API (implies `vcon`).
- `harness` — compatibility no-op. Provider traits live in
  `rvoip-core-traits`; the concrete `rvoip-harness` crate depends on
  `rvoip-core` and supplies the in-process AI `ConnectionAdapter`.
- `webrtc` — advisory shape flag for WebRTC consumers; gates no code here.
- `client` — advisory shape flag for `rvoip-client`; gates no code here.
- `aauth-experimental` — advisory flag reserved for actor-authentication
  work; gates no code here.
- `identity-fingerprint-binding` — advisory flag reserved for identity
  fingerprint binding; gates no code here today.
- `test-hooks` — deterministic, bounded orchestration gates used only by
  integration tests; never enable in release builds.
- `g729`, `opus`, `amr-nb`, `amr-wb`, `amr` (= both AMR variants),
  `all-codecs` — forward the matching `rvoip-media-core` codec backends.
  G.711 is always available.
- `full` — `webrtc`, `harness`, `client`, `aauth-experimental`,
  `identity-fingerprint-binding`, `vcon-signing`, and `all-codecs`.

The rvoip 3 vision and rationale live alongside this crate's source:

- [`voip-3-conversation-model.md`](../../../docs/voip-3-conversation-model.md) — vocabulary
- [`PRD.md`](../../../docs/PRD.md) — product scope
- [`INTERFACE_DESIGN.md`](../../../docs/INTERFACE_DESIGN.md) — crate architecture
- [`GAP_PLAN.md`](../../../docs/GAP_PLAN.md) — implementation status
- [`CONVERSATION_PROTOCOL.md`](../../../docs/CONVERSATION_PROTOCOL.md) — UCTP wire spec

## Install

Most users don't depend on `rvoip-core` directly — depend on
[`rvoip-sip`](https://crates.io/crates/rvoip-sip) (or eventually the
[`rvoip`](https://crates.io/crates/rvoip) umbrella) and the spine comes
along transitively.

```toml
[dependencies]
rvoip-core = "0.3.12"
```

## Examples

- [`sip_only_orchestrator`](examples/sip_only_orchestrator.rs) — wire
  `rvoip-sip`'s SipAdapter into a `rvoip-core` Orchestrator.
- [`cross_transport_bridge`](examples/cross_transport_bridge.rs) —
  SIP + WebRTC + QUIC adapters registered with a single Orchestrator,
  bridged via `BridgeManager`. (Pre-alpha — WebRTC/QUIC paths are
  pinned to upstream alpha crates.)
- [`checked_rtp_boundary`](examples/checked_rtp_boundary.rs) — convert between
  validated RTP packets and payload-only `MediaFrame`s with explicit
  negotiated codec/PT identity, bounded allocation, exact packet preservation,
  and deterministic egress packetization. The SIP, WebRTC, and UCTP crates
  include matching gateway examples rather than rebuilding RTP headers.

## Waiting for media readiness

Signaling completion and usable media are separate states. Applications can
await the exact state they need without writing a transport-specific polling
loop:

```rust,no_run
use std::time::Duration;
use rvoip_core::{MediaReadiness, StreamKind, StreamSelector};
use tokio_util::sync::CancellationToken;

# async fn wait(orchestrator: &rvoip_core::Orchestrator, connection_id: rvoip_core::ConnectionId) -> Result<(), rvoip_core::StreamWaitError> {
let stream = orchestrator
    .wait_for_stream(
        connection_id,
        StreamSelector::new(StreamKind::Audio)
            .with_codec("opus")
            .with_readiness(MediaReadiness::Bidirectional),
        tokio::time::Instant::now() + Duration::from_secs(5),
        CancellationToken::new(),
    )
    .await?;
# let _ = stream;
# Ok(())
# }
```

The readiness levels are deliberately distinct:

- **Signaling connected** is a connection lifecycle event; it does not prove a
  media stream exists.
- **Registered** means the adapter has published a matching stable stream
  identity.
- **Source ready** additionally means its negotiated codec and inbound producer
  are authoritative and ready for a consumer.
- **Bidirectional** additionally means the stream accepts outbound frames.

Call the orchestrator surface when lifecycle safety matters. It fences the
wait to the captured connection generation and returns typed errors for
terminal teardown or replacement. Direct adapter waits are intended for
adapter-level integration and retain a source-compatible bounded fallback.

## Mid-call codec changes

`Orchestrator::renegotiate_media` waits for the selected adapter's protocol-
native negotiation to commit before updating a live bridge. Cross-transport
media graphs receive the complete negotiated `CodecInfo`, including the exact
dynamic payload type and fmtp, rather than reconstructing codec identity from
the name. An adapter error leaves the graph on its prior codec generation.

With the matching features enabled, G.729 (static PT 18) and AMR-NB / AMR-WB
(negotiated PT only) flow through the media graph alongside G.711 and Opus;
see [`docs/MEDIA_GRAPH_CODECS.md`](docs/MEDIA_GRAPH_CODECS.md) for the
admission rules.

## Participant roles and bridge handoff

`Orchestrator::set_participant_role`, `take_over`, and `hand_off` change a
Participant's role within a Session (for example promoting a human to
`Agent` and demoting the previous Agent to `Observer`) without moving
Connections or bridges. `Orchestrator::replace_bridge_destination` swaps one
exact far leg of a live bridge, such as handing a caller from an in-process
AI adapter to a human endpoint, while preserving the ingress Connection and
Session; the expected bridge ID acts as a generation fence and a fresh ID is
returned for the next replacement.

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
