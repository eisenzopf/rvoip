# rvoip-harness

> ⚠️ **Experimental surface** (unified `0.3.x` release) — API-unstable; expect breaking changes before `1.0`.

Pluggable provider trait surfaces for ASR, TTS, `DialogManager`, and
`RecordingSink`, plus a first-party in-process AI connection adapter.

`InProcessAiAdapter` turns an AI runtime into an ordinary outbound
`Transport::InProcessAi` connection. Its audio boundary is bounded PCM16,
16 kHz, mono. rvoip-core's MediaGraph performs the transport conversion, so
the same AI implementation composes with SIP PCMU/PCMA, WebRTC Opus, and any
future transport that exposes a normal rvoip media stream.

Implement `InProcessAiSessionFactory` to allocate per-call state and
`InProcessAiSession` to consume caller frames, emit agent frames, and service
the supplied `InProcessAiSessionLifecycle` receiver. Session work starts only
after the orchestrator commits the outbound connection; a prepared route
remains dormant and can be cancelled without provider-visible media work.
`InProcessAiAdapter::echo` is a deterministic smoke-test factory.

`ConnectionAdapter::hold` and `resume` fence both media directions and require
a bounded provider acknowledgement. A session must select lifecycle requests
alongside every ASR, model, tool, TTS, and playback future. It acknowledges
`Paused` only after the competing future has been cancelled or drained and its
uncommitted turn state is safe to retry. It acknowledges `Running` when that
retained state can continue. A failed hold restores both provider and media to
running; if that rollback cannot be acknowledged, the adapter ends the route
instead of leaving a live silent session. Holding never recreates the provider,
so committed dialogue state remains attached to the stable AI connection.
`InProcessAiMedia::subscribe_lifecycle` remains an observation-only media-gate
signal; it does not satisfy the provider acknowledgement contract. `end` is
idempotent for a bounded history of completed connection IDs.
`resource_snapshot` exposes only aggregate live route, active/held session,
session-task, and media-task counts for leak checks.

```rust,no_run
use std::sync::Arc;

use rvoip_harness::{
    InProcessAiAdapter, InProcessAiConfig, InProcessAiSessionFactory,
};

fn adapter(factory: Arc<dyn InProcessAiSessionFactory>) {
    let adapter = InProcessAiAdapter::new(InProcessAiConfig::default(), factory)
        .expect("valid bounded AI adapter configuration");
    // Register `adapter` with rvoip-core before opening ingress.
    drop(adapter);
}
```

Transport handoff remains an rvoip-core topology operation. The harness does
not implement SIP REFER, WebRTC signaling, routing policy, or account policy.
Callers use the generation-fenced `Orchestrator::replace_bridge_destination`
primitive to replace an AI connection with a prepared SIP/WebRTC connection,
or the reverse, while the candidate media route stays silent until promotion.
For a reusable AI session, detach and rebind are expressed by core topology:
hold the AI connection, `unbridge_connections`, later bridge that same live
connection to a new peer, then resume it. The harness deliberately does not
create a second provider-level rebind API or carry transport identifiers into
provider code. The replacement primitive retires its old destination, so use
explicit unbridge/rebridge when the old AI session must survive detachment.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `voip-3`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
