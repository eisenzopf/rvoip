# rvoip-vapi

Developer-preview Vapi integration for rvoip. rvoip owns the caller-facing SIP
or WebRTC leg; Vapi owns the full ASR, model, synthesis, and interruption
pipeline through its bidirectional raw-audio WebSocket transport.

The primary API attaches an agent to a caller connection that is already bound
to an active rvoip session. `attach_agent` creates a new Conversation
Participant (`kind=Ai`, `role=Agent`) and originates the Vapi Connection
under that id — it does **not** reuse the caller's `participant_id`. Use
`attach_agent_for_participant` when Parley (or another Conversation server)
already allocated the AI Participant. `VapiAgentCall::ai_participant_id()`
returns that id.

```no_run
use std::sync::Arc;
use rvoip_core::{ConnectionId, Orchestrator};
use rvoip_vapi::{
    VapiAdapter, VapiApiKey, VapiAssistant, VapiCallOptions, VapiConfig,
};

async fn attach(
    orchestrator: Arc<Orchestrator>,
    caller: ConnectionId,
) -> Result<(), Box<dyn std::error::Error>> {
    let key = VapiApiKey::new(std::env::var("VAPI_API_KEY")?)?;
    let adapter = VapiAdapter::new(VapiConfig::new(key))?;
    let options =
        VapiCallOptions::new(VapiAssistant::saved(std::env::var("VAPI_ASSISTANT_ID")?));

    // Registers this adapter when no Vapi adapter is registered, joins a
    // distinct AI Participant, originates the remote agent, bridges audio,
    // and supervises teardown. The Vapi Connection is not attributed to the
    // caller.
    let mut call = adapter.attach_agent(&orchestrator, caller, options).await?;

    call.say("One moment while I look that up.", false, true).await?;
    let _outcome = call.wait().await;
    Ok(())
}
```

For a complete high-level server that accepts either SIP or WebRTC with one
transport flag, see
[`examples/14-vapi-agent`](../../../examples/14-vapi-agent/).

`VapiAudioFormat::MuLaw8Khz` is the default and gives SIP/PCMU a pass-through
path. `VapiAudioFormat::PcmS16Le16Khz` selects raw mono PCM and relies on
rvoip's media graph for conversion to the caller codec.

The low-level path is also available: register `VapiAdapter` as a
`ConnectionAdapter`, originate `Transport::Vapi` with a typed
`VapiCallOptions` context, then bridge the resulting connection yourself.

For a provider call that the host already created and verified,
`VapiAdapter::attach_existing_agent` bridges a `VapiExistingCall` without
creating one, using the same canonical connections, bridge, and paired
termination as `attach_agent`; the host owns durable admission and receipt
validation, and must never construct the handoff from public request
arguments. `VapiConfig::existing_calls_only()` builds a shared transport that
retains no default tenant key, so each handoff (or a `VapiCallOptions`)
supplies its own WebSocket credential through `with_api_key`, which never
becomes the adapter's default. `VapiAgentCall::wait_shared` observes paired
teardown while other owners retain call-control handles.

Call events are available through `VapiAgentCall::subscribe_events` and
`VapiAdapter::subscribe_vapi_events`. Unknown event types are preserved rather
than closing the audio session. The first per-call subscriber also receives
events buffered during activation. DTMF and transfer remain owned by the rvoip
caller leg; Vapi's WebSocket transport does not define an RTP/DTMF input.

Production configuration requires HTTPS and WSS. API keys, returned socket
URLs, assistant definitions, transcripts, audio, and tool payloads are omitted
from adapter diagnostics.

The adapter emits content-free counters for setup failures, remote
disconnects, bounded-queue overflow, audio-frame flow, unknown or malformed
events, dropped RTP DTMF frames, and terminated sessions. Metric labels use
only fixed stage, direction, and outcome values.

The live smoke test is ignored by default. Run it explicitly with
`VAPI_API_KEY` and either `VAPI_ASSISTANT_ID` or
`VAPI_TRANSIENT_ASSISTANT_JSON`:

```sh
cargo test -p rvoip-vapi --test live_smoke -- --ignored
```

## Ordered, unpaced media for protocol gateways

The default `VapiMediaDelivery::Paced` mode suits paced RTP legs: Vapi audio
is reframed into 20 ms frames, released on a real-time clock through a bounded
drop-oldest jitter buffer, and flushed on caller barge-in. Application-protocol
gateways that frame replies themselves, such as the
[Webex BYOVA adapter](../rvoip-webex-byova), need every byte in wire order
instead. Select it per call:

```no_run
use rvoip_vapi::{VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiMediaDelivery};

let options = VapiCallOptions::new(VapiAssistant::saved("assistant-id"))
    .with_audio_format(VapiAudioFormat::MuLaw8Khz)
    .with_media_delivery(VapiMediaDelivery::Ordered);
```

In ordered mode the call still registers with the `Orchestrator` and bridges
through the core media graph, but:

- each Vapi binary message is forwarded immediately and unchanged; nothing is
  paced, reframed or dropped. A slow consumer applies backpressure, buffered up
  to `VapiConfig::ordered_inbound_buffer_bytes` (256 KiB), after which the
  adapter stops reading the socket. No progress for
  `VapiConfig::ordered_media_stall_timeout` (5 s) fails the call;
- the media stream asks bridges for lossless delivery, so the core media graph
  holds its source instead of dropping frames;
- `VapiAdapter::take_ordered_events` (or `VapiAgentCall::take_ordered_events`)
  returns the call's exclusive event stream. Each `VapiOrderedEvent` carries
  `audio_offset`, the number of audio bytes forwarded before the event, so a
  consumer can tell exactly which audio preceded a speech or status update;
- caller barge-in does not flush queued audio; the consumer fences stale audio
  by position;
- caller audio toward Vapi is written unpaced; a stalled socket fails the call
  instead of dropping caller speech.

Positions count bytes in the call's audio format, so pair ordered mode with a
consumer using the same codec (PCMU/8 kHz for Webex) to avoid transcoding.
Vapi's own speech events remain provider signals, not guaranteed audio
completion barriers; the position tells a consumer where an event sits in the
stream, not that the provider has finished generating audio.

