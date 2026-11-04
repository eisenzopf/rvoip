# rvoip-amazon-a2a

Amazon Connect **A2A external-agent adapter** for rvoip. It implements an inbound
`ConnectionAdapter` (`Transport::AmazonA2a`) and exposes the Connect caller as a
PCM media stream that the `Orchestrator` can bridge to `rvoip-vapi` or another
rvoip adapter.

**Developer preview:** tested against deterministic protocol fixtures and an
in-process Connect client, including a real rvoip media graph and the existing
Vapi adapter with a mock Vapi WebSocket. Live Amazon Connect qualification is
still required. This is a transport library, not a turnkey Vapi agent runner.

```text
Connect orchestrator → A2A/WSS → AmazonA2aAdapter
                                     ↕ rvoip Orchestrator / media graph
                                existing VapiAdapter → Vapi
```

This differs from `rvoip-amazon-connect`, which originates a WebRTC contact into
Connect via `StartWebRTCContact` and Chime. This crate accepts Connect-initiated
A2A collaborations and hands control back to Connect with `FINISH`.

## Supported contract

- A2A v1 JSON-RPC over WebSocket with the Connect v1 extension.
- Talk-to-customer voice/audio and text sessions. Behind-the-scenes invocation,
  HTTPS JSON-RPC polling, and arbitrary A2A agent discovery are not implemented.
- Bearer verification at upgrade, extension negotiation, propagated W3C trace
  IDs, and exact instance binding from trusted host configuration.
- `INIT_SESSION` / `INIT_SESSION_RESPONSE`; no customer output until `READY`.
- PCM16 mono at 8, 16 or 24 kHz. Both directions are counter-proposed to the
  configured rate (16 kHz by default). The rvoip stream is `pcm_s16le`; the
  negotiated rate remains fixed for that connection. Audio stays in independent
  chunks of at most 20 ms and is never transport-fragmented.
- Audio artifacts, progressive text, terminal tasks, `COMPLETE` / `ESCALATE`,
  incoming DTMF, silence text, interruption, and informational errors.
- Bounded fragment assembly (including out-of-order fragments), size/part
  limits, expiration, and outbound fragmentation for large non-audio payloads.
- Per-turn OTLP/JSON tracing with actual input/output and tool spans. Turn
  completion requires a `TurnTrace`; missing evidence is not synthesized.
- Bounded connection/media/control/input queues, admission and write deadlines,
  idle cleanup, generation-aware peer media, and terminal-event fallback.

## Host integration

1. Create the adapter and register it with the `Orchestrator`.
2. Open a conversation/session using your application's trusted tenant policy.
   Construct `CallBinding` from that session, participant, and allowed Connect
   instance ARN. **Never derive the rvoip tenant/session binding from the remote
   `contextId`, contact attributes, or model-generated arguments.**
3. Accept a TLS stream (or a private stream from your TLS ingress) and call
   `accept_websocket(stream, binding)`. It authenticates the upgrade and reads
   initialization, then publishes the native inbound connection. It returns an
   `AmazonA2aCall` with the initialization/history and a bounded input channel.
4. Wait for `Event::ConnectionInbound`, and accept through
   `Orchestrator::route_inbound_connection`. Only then is initialization
   acknowledged to Connect. This adapter does not advertise authenticated
   principal handoff for the core's optional fail-closed inbound-admission gate;
   use an explicit host binding and routing policy rather than claiming that
   gate is satisfied.
5. Wait for `InputEvent::Ready` before attaching an agent. Media is inaccessible
   before readiness. Consume `next_input()` continuously; an unread input queue
   fails the collaboration rather than silently losing DTMF or diagnostics.
6. Bridge the connection and drive turn controls from your agent's events.

The adapter validates the configured bearer key; it does not terminate TLS or
provision AWS. Do not attach it to a public plaintext listener. API keys require
at least 16 non-whitespace characters and are zeroized/redacted. Keep separate
adapter instances/keys for different admission domains. Transcripts, history,
DTMF and tool payloads are deliberately absent from diagnostic `Debug` output.

## Reuse the Vapi adapter

With an accepted, ready `AmazonA2aCall`, use the existing Vapi API:

```no_run
use std::sync::Arc;
use rvoip_core::Orchestrator;
use rvoip_amazon_a2a::AmazonA2aCall;
use rvoip_vapi::{VapiAdapter, VapiAgentCall, VapiAssistant, VapiAudioFormat,
    VapiCallOptions, VapiPeerFailurePolicy};

async fn attach(
    core: &Arc<Orchestrator>,
    vapi: &Arc<VapiAdapter>,
    connect: &AmazonA2aCall,
    assistant_id: String,
) -> rvoip_core::error::Result<VapiAgentCall> {
    let options = VapiCallOptions::new(VapiAssistant::saved(assistant_id))
        .with_audio_format(VapiAudioFormat::PcmS16Le16Khz)
        .with_peer_failure_policy(VapiPeerFailurePolicy::LeaveCallerConnected);
    vapi.attach_agent(core, connect.connection_id().clone(), options).await
}
```

`LeaveCallerConnected` lets the host finish the last task and hand control back
before ending the Connect leg. The same configuration works with
`attach_existing_agent` for a call already created and verified by the host.
The library's integration test exercises this path without a second Vapi
implementation or a live AWS account.

**The host still owns the agent turn controller.** Subscribe to Vapi events,
configure the required client messages, carry forward Connect history, and
supply real final transcripts and every tool input/result. A `speech-update`
alone is not proof that every audio buffer has drained. Stop and drain the
upstream producer/media graph before `complete_turn`; that method drains
frames already queued at this adapter, then sends the final artifact chunk,
trace, and terminal status. It never guesses a turn boundary from silence.
Production Vapi event-to-turn ordering and trace completeness need live
qualification; this crate does not silently supply a heuristic controller.

- `send_audio` / `send_text` provide ordered, application-driven output when a
  media graph is not used. Do not mix two producers on the same output stream.
- `complete_turn(TurnTrace)` finishes a response, not the collaboration. Each
  response has its own task and artifact; overlapping responses are rejected.
- `interrupt(kind)` discards this adapter's queued audio and sends Connect's
  playback-flush event. For locally detected audio barge-in it also closes the
  audio artifact. Stop/flush the upstream agent and graph as well, then complete
  the interrupted turn's trace before starting new output.
- Incoming interruption invalidates the artifact: no `lastChunk` is sent for
  it. The host receives `InputEvent::Interrupted` and must stop generation.
- `finish(kind, reason)` is legal only after the active turn is complete and
  only for a finish type offered by Connect. No further A2A messages follow.
  `ESCALATE` returns control to the Connect orchestrator; its flow owns the
  human-agent transfer. It does not originate a new Connect contact.
- `close` / `ConnectionAdapter::end` cancel transport ownership; they are not
  successful `FINISH` or implicit escalations. Normal socket closure alone is
  not a semantic completion signal.

`TraceContext` propagates `traceparent` IDs. `TurnTrace::with_tool` accepts
`trace::ToolTrace` entries with actual input/output and timestamps inside the
turn interval. Trace contents go to Connect as required; applications remain
responsible for their data policy. Even when `subscribeToTracingEvents` is
false, completed turns include their trace, following Connect's mandatory
per-turn trace requirement.

## Reconnect and ownership

A new connection always requires a new initialization. Concurrent sockets with
the same `contextId` are rejected. After the previous route is fully retired,
the context may initialize again; the host must restore its conversation and
make tool execution idempotent. This library does not replay audio or restart
agent/tool work automatically, and it has no durable session store. Use the
same adapter per admission domain to enforce active-context uniqueness.

Keep the returned call handle to control and observe the collaboration.
Dropping a handle does not detach cleanup: the adapter owns the socket task
until disconnect, timeout, or cancellation. `close().await` waits for retirement
and permit release. `shutdown().await` closes admission and retires live
routes; pre-initialization sockets remain bounded by their setup timeout.

## Run checks and a Connect smoke server

```sh
cargo test -p rvoip-amazon-a2a
cargo clippy -p rvoip-amazon-a2a --all-targets -- -D warnings
cargo check -p rvoip --no-default-features --features amazon-a2a

# Load both variables through your secret/configuration mechanism first:
# AMAZON_A2A_API_KEY and CONNECT_INSTANCE_ARN
cargo run -p rvoip-amazon-a2a --example text_echo
```

The text-only echo example listens on `127.0.0.1:8080`. Place a public WSS
endpoint in front of it with a TLS reverse proxy, and register it as an
`A2A_SERVER` application. Use **text** talk-to-customer mode for this smoke
example; it rejects audio sessions. It traces each echoed turn and sends
`FINISH COMPLETE` after `goodbye`.

For a voice deployment, create a Connect Customer instance and assistant,
configure the Lex/Nova Sonic voice prerequisite, store the bearer secret with a
customer-managed KMS key, associate the AppIntegrations application, configure
the agent security profile, and publish the orchestrator with audio streaming.
Keep provisioning outside this transport crate. Validate voice latency,
interruptions, traces, reconnect and hand-back with AWS before production.

References:
- [A2A protocol contract](https://docs.aws.amazon.com/connect/latest/devguide/a2a-developer-guide.html)
- [Sample events](https://docs.aws.amazon.com/connect/latest/devguide/a2a-sample-events.html)
- [Server lifecycle](https://docs.aws.amazon.com/connect/latest/adminguide/a2a-external-server.html)
- [AWS setup](https://docs.aws.amazon.com/connect/latest/adminguide/a2a-setup-external.html)
