# rvoip-vapi

Developer-preview Vapi integration for rvoip. rvoip owns the caller-facing SIP
or WebRTC leg; Vapi owns the full ASR, model, synthesis, and interruption
pipeline through its bidirectional raw-audio WebSocket transport.

The primary API attaches an agent to a caller connection that is already bound
to an active rvoip session:

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

    // Registers this adapter when no Vapi adapter is registered, originates
    // the remote agent, bridges both audio directions, and supervises teardown.
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
