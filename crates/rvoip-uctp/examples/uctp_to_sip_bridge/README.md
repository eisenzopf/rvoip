# uctp_to_sip_bridge — Phase 4 demo

The v0 spike's headline deliverable: a cross-transport bridge that
registers `UctpQuicAdapter`, `UctpWtAdapter`, `UctpWsAdapter`, and
`SipAdapter` against the same `rvoip_core::Orchestrator` and exercises
all four from separate client binaries.

## Architecture

```
                       ┌─────────────────────────┐
                       │  orchestrator_bridge    │
                       │  (one process)          │
                       │                         │
   uctp_agent_quic ───►│  UctpQuicAdapter        │
   (raw QUIC, ALPN     │     ALPN=uctp/1         │
    uctp/1)            │                         │
                       │  UctpWtAdapter          │
   uctp_agent_wt ─────►│     ALPN=h3 + WT        │
   (HTTP/3 + WT,       │     upgrade on /uctp    │
    ALPN h3)           │                         │
                       │  UctpWsAdapter          │
   uctp_agent_ws ─────►│     plain ws://         │
   (WebSocket text)    │     port 7777           │
                       │                         │
                       │  SipAdapter             │
   sip_caller ────────►│     UDP, port 5072      │
   (UDP SIP)           │                         │
                       │                         │
                       │  Cross-transport event  │
                       │  bus (Event::*)         │
                       └─────────────────────────┘
```

The orchestrator subscribes to its own normalized `rvoip_core::Event`
stream and logs every event. **Frame-pump bridging itself is v0.x** —
`Orchestrator::bridge_connections` is still stubbed in rvoip-core and
the design doc §6.2 calls out the manual pump as a separate work item.
v0 proves: adapter wiring, dual-ALPN shared-endpoint deployment, and
cross-transport event normalization.

## Run the demo

In four terminals:

**Terminal 1 — bridge:**
```bash
cargo run -p rvoip-uctp --example orchestrator_bridge
```
Writes the self-signed TLS cert to `/tmp/uctp_demo_cert.der` so the
agent binaries can pin against it. Listens on:
- `127.0.0.1:4433` for UDP (QUIC + WebTransport, dual-ALPN)
- `127.0.0.1:5072` for UDP (SIP)

**Terminal 2 — UCTP-over-QUIC agent:**
```bash
cargo run -p rvoip-uctp --example uctp_agent_quic
```
Dials raw QUIC, runs the auth handshake, sends `session.invite`,
streams inbound envelopes. Watch terminal 1 for matching events.

**Terminal 3 — UCTP-over-WebTransport agent:**
```bash
cargo run -p rvoip-uctp --example uctp_agent_wt
```
Same flow but through the HTTP/3 + extended-CONNECT upgrade path.
Stands in for a modern browser client.

**Terminal 4 — UCTP-over-WebSocket agent:**
```bash
cargo run -p rvoip-uctp --example uctp_agent_ws
```
Same flow over plain `ws://127.0.0.1:7777`. Stands in for an older
browser without WebTransport support. **Media plane is stubbed in v0**
— signaling works end-to-end; the co-located WebRTC PeerConnection
(`WebRtcMediaBridge` in `rvoip-websocket/src/media_bridge.rs`) is
deferred to v0.x pending the `webrtc-rs` crate's stable release.

**Terminal 5 — SIP caller:**
```bash
cargo run -p rvoip-uctp --example sip_caller
```
Dials `sip:agent@127.0.0.1:5072`. The bridge currently logs the
`Event::ConnectionInbound` event but does not yet auto-answer (v0.x
work — depends on `Orchestrator::bridge_connections` being un-stubbed
and a manual frame-pump being wired up).

## Browser smoke (manual)

See [`browser/`](browser/README.md). A real browser would call
`new WebTransport("https://localhost:4433/uctp")`. The HTML page in
this directory demonstrates the JavaScript handshake; it isn't run in
CI because the self-signed cert ergonomics for browsers are brittle.

## Integration test

```bash
cargo test -p rvoip-uctp --test bridge_smoke
```

Brings the orchestrator + all three adapters up in-process, dials a
UCTP-over-QUIC client at it, and asserts that the normalized
`Event::ConnectionInbound` fires on the orchestrator's event bus.

## What's still v0.x

- Frame-pump bridging (UCTP ↔ SIP audio via `rvoip_media_core::Transcoder`).
- `Orchestrator::bridge_connections` real implementation.
- `Orchestrator::route_inbound_connection` auto-answer policy.
- Multi-party routing (`stream.subscribe` / `stream.active-speaker`).
- vCon emission at `session.ended`.

See `crates/rvoip-uctp/UCTP_IMPLEMENTATION_PLAN.md` §6 for the full
design doc and §1.4 for the v0.x roadmap.
