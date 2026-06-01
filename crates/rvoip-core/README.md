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

**Beta candidate** — part of the `rvoip-sip` 0.2.0-beta closure. The
type surface and `Orchestrator` are stable for the SIP path; optional
features `vcon-signing` (vCon JWS signing) and `harness`
(ASR/TTS/DialogManager dispatch) are alpha-quality and may evolve.

The rvoip 3 vision and rationale live alongside this crate's source:

- [`voip-3-conversation-model.md`](voip-3-conversation-model.md) — vocabulary
- [`PRD.md`](PRD.md) — product scope
- [`INTERFACE_DESIGN.md`](INTERFACE_DESIGN.md) — crate architecture
- [`GAP_PLAN.md`](GAP_PLAN.md) — implementation status
- [`CONVERSATION_PROTOCOL.md`](CONVERSATION_PROTOCOL.md) — UCTP wire spec

## Install

Most users don't depend on `rvoip-core` directly — depend on
[`rvoip-sip`](https://crates.io/crates/rvoip-sip) (or eventually the
[`rvoip`](https://crates.io/crates/rvoip) umbrella) and the spine comes
along transitively.

```toml
[dependencies]
rvoip-core = "0.2.0-beta.1"
```

## Examples

- [`sip_only_orchestrator`](examples/sip_only_orchestrator.rs) — wire
  `rvoip-sip`'s SipAdapter into a `rvoip-core` Orchestrator.
- [`cross_transport_bridge`](examples/cross_transport_bridge.rs) —
  SIP + WebRTC + QUIC adapters registered with a single Orchestrator,
  bridged via `BridgeManager`. (Pre-alpha — WebRTC/QUIC paths are
  pinned to upstream alpha crates.)

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
