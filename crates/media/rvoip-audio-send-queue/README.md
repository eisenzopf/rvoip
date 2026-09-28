# rvoip-audio-send-queue

Bounded, generation-aware queue for outbound media frames.

Transports that hand off a caller's audio between destinations (a bridge
replacement, an AI agent taking over from a human, a peer handoff) need the
old destination's frames to stop the instant the new one is committed, without
a lock on the media path. This crate provides that primitive:

- `Sender::try_send(generation, frame)` accepts a frame only if it carries the
  current generation; frames from a retired generation are rejected with
  `SendError::StaleGeneration` instead of being delivered late.
- `Sender::advance_to(generation)` retires the previous generation and reports
  how many queued frames were discarded, so the caller can record the cutover.
- The queue is bounded; when full it drops the oldest frame and counts it, so a
  slow consumer degrades audio rather than growing memory.
- `Receiver` yields `Delivery { generation, value }` and exposes a `watch`
  channel of generation changes for consumers that need to resync.
- `Sender::metrics()` returns accepted, dropped, and stale counts for evidence.

The crate depends only on `tokio` (sync primitives) and `thiserror`. It is
used by `rvoip-sip` for transport-fenced bridge replacement; see
[`Orchestrator::replace_bridge_destination_transport_fenced`](../../foundation/rvoip-core)
for the higher-level operation.

```toml
[dependencies]
rvoip-audio-send-queue = "0.3.10"
```

Licensed under the [MIT License](../../../LICENSE).
