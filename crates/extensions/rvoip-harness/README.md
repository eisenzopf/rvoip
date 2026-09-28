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
Each media queue uses bounded backpressure: a full queue suspends its producer
and never silently drops or overwrites audio. `begin_drain` closes admission
atomically with route publication, and `drain` ends dormant and active sessions
within a caller-supplied budget.

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
or the reverse, while the candidate media route stays silent until promotion;
`replace_bridge_destination_transport_fenced` is the variant that succeeds
only when both the existing and candidate transports expose generation-aware
delivery queues.
Reusable provider state is keyed by `AiSessionId`, independently of the AI
adapter's `ConnectionId`. `AiOriginateContext` carries bounded provider
references and a resume policy. `rebind_session` sends a typed,
generation-qualified `AiMediaBinding` to the provider and publishes the new
binding only after the provider acknowledges it; stale, skipped, wrong-session,
and post-terminal requests fail closed. Core topology still owns the actual
media route: hold the AI connection, change the bridge, rebind the provider to
the authoritative peer generation, then resume it. The replacement primitive
retires its old destination, so use explicit unbridge/rebridge when the old AI
session must survive detachment.

The `test-reference` feature exposes deterministic `ReferenceAsrProvider`,
`ReferenceTtsProvider`, and `ReferenceDialogManager` implementations (with
`ReferenceProviderControl`, `ReferenceProviderSnapshot`, and `ReferenceStage`)
for examples and external tests; the module is excluded from normal builds.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `voip-3`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
