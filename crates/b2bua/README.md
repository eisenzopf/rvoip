# rvoip-b2bua

`rvoip-b2bua` is the first platform crate above `session-core`. It proves the
two-leg call topology needed by contact-center, voice-ai, CPaaS, and QSRP
adapters without exposing `UnifiedCoordinator` as a product API.

The crate currently supports the minimum useful B2BUA flow:

1. Receive an inbound SIP call through `session-core`.
2. Ask a router for a route decision.
3. Dial an outbound SIP target.
4. Accept the inbound call after the outbound leg answers.
5. Wait for both legs to become active.
6. Bridge RTP with `UnifiedCoordinator::bridge`.
7. Keep the bridge alive until either leg ends.
8. Propagate teardown to the other leg.
9. Emit B2BUA-level events with one stable call id for both legs.

## Quick Example

```rust,no_run
use std::sync::Arc;

use rvoip_b2bua::{B2buaService, SessionConfig, StaticRouter};

#[tokio::main]
async fn main() -> rvoip_b2bua::Result<()> {
    let service = B2buaService::new(SessionConfig::local("b2bua", 5060)).await?;
    let router = Arc::new(StaticRouter::dial("sip:agent@127.0.0.1:5070"));

    service.serve(router).await
}
```

## Example Binary

Run a simple static bridge server:

```bash
B2BUA_TARGET=sip:agent@127.0.0.1:5070 \
cargo run -p rvoip-b2bua --example simple_bridge
```

Useful environment variables:

- `B2BUA_SIP_PORT`, default `5060`
- `B2BUA_NAME`, default `b2bua`
- `B2BUA_TARGET`, required
- `B2BUA_MEDIA_START`, default `16000`
- `B2BUA_MEDIA_END`, default `17000`
- `B2BUA_CALL_DURATION_SECS`, optional demo auto-hangup after bridge

## Public Surface

- `B2buaService` owns the B2BUA orchestration.
- `Router` lets higher layers choose a route.
- `RouteDecision` supports `Dial`, `Reject`, and `Redirect`.
- `B2buaEvent` exposes correlated call, leg, bridge, DTMF, transfer, end, and
  failure events.
- `B2buaCallSnapshot` exposes the current coarse lifecycle state.

## Current Non-Goals

- No contact-center queues.
- No voice-ai prompt, ASR, TTS, or bot runtime.
- No CPaaS HTTP API.
- No QSRP transport.
- No media transcoding.
- No independent telephony state machine separate from `session-core`.
