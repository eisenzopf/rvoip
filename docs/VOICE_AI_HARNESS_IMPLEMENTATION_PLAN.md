# rvoip Voice AI Harness v2 — Detailed Implementation Plan

| Field | Value |
|---|---|
| Status | Detailed implementation plan; experimental baseline exists |
| Plan date | 2026-07-30 |
| Target | First production-capable voice-agent release |
| Release gate | Both a cascaded ASR → model → TTS agent and a Moshi-class full-duplex speech-native agent must work end to end |

This plan replaces the current experimental voice-AI API. It is intentionally
more detailed than a design sketch: public contracts, ownership, state
transitions, failure semantics, work packages, and acceptance gates are fixed
here so implementation does not have to rediscover product decisions.

## 1. Locked decisions

1. **Rust boundary**
   - rvoip, its voice-agent runtime, and all shipped provider adapters are Rust.
   - CUDA, Metal, ONNX Runtime, Candle, tract, and C/C++ inference libraries
     reached through Rust APIs are permitted.
   - No production runtime may require a Python interpreter, Python process,
     Python package environment, or Python sidecar.
   - Python is permitted only for developer-only research, model conversion, or
     evaluation tooling. A qualified customer build either uses Rust/native
     source builds or preverified native artifacts; it cannot invoke Python as
     part of deployment or ordinary runtime startup.
   - A remote service may be implemented by its operator in any language; the
     rvoip integration and all local deployment components remain Rust/native.

2. **First-release scope**
   - Cascaded and speech-native modes are co-equal release requirements.
   - Cascaded mode continuously listens while it generates and plays speech.
   - Speech-native mode continuously exchanges audio with a Moshi-class model
     and may listen and speak at the same time.
   - The public contracts for both modes land before provider-specific work.

3. **API compatibility**
   - The experimental `AsrProvider`, `AsrStream`, `TtsProvider`, `TtsPlayback`,
     `DialogManager`, `DialogAction`, `AttachAi` command, and `attach_ai` method
     are removed.
   - No compatibility adapter or deprecated alias is retained.
   - Existing examples, tests, events, and documentation move to the new API in
     the same release.
   - `RecordingSink` and `RecordingArtifact` are not AI APIs. Move them into a
     recording-specific module and preserve their behavior.

4. **Media model**
   - An in-process agent is a distinct `ParticipantKind::Ai` and
     `Transport::InProcessAi` Connection.
   - Normal rvoip media graphs bridge the caller and the AI connection.
   - Providers never receive an ambiguous encoded `MediaFrame`. The harness
     owns a typed PCM boundary and provider-specific resampling.

5. **Provider model**
   - Local and remote providers implement the same session contracts.
   - Provider-specific configuration is typed in the provider crate and is
     supplied when registering the provider. Agent definitions select provider
     IDs; they do not carry `HashMap<String, String>` configuration.
   - Moshi is a `RealtimeSpeechProvider`. It is not expressed as fake ASR,
     model, and TTS components.

## 2. Goals, non-goals, and release definition

### 2.1 Goals

- True asynchronous listening and speech on SIP, WebRTC, and UCTP calls.
- Deterministic, testable activity detection, endpointing, interruption, and
  playback behavior.
- Streaming ASR, model output, tool calls, and TTS.
- A direct full-duplex interface for Moshi and future speech-native models.
- Local, remote, or mixed provider placement without changing the agent API.
- Codec-correct, paced audio output for PCMU, PCMA, Opus, and internal PCM.
- Generation-scoped cancellation that prevents stale text or audio from
  reappearing after barge-in.
- Correlated lifecycle, transcript, latency, usage, quality, and failure
  telemetry, including vCon evidence.
- Bounded queues, explicit overload behavior, provider health, admission
  control, and deterministic teardown.

### 2.2 Non-goals for the first release

- Training or fine-tuning speech or language models.
- Bundling model weights in rvoip crates.
- General multiparty diarization or speaker separation.
- Automatic voice cloning. Provider support may exist but must remain disabled
  until a separate consent and abuse-control design is approved.
- Treating server-side AEC as universally reliable. The first release supplies
  the render-reference interface; validated AEC is optional.
- Replacing `rvoip-vapi`. Vapi remains the existing externally orchestrated
  agent-as-Connection topology.
- Using Gradio, FastRTC, Pipecat, or LiveKit as production dependencies. They
  remain references and evaluation clients.

### 2.3 Definition of the first release

The release is complete only when all of the following are true:

- A caller using SIP/PCMU and a caller using WebRTC/Opus can each talk to the
  cascaded reference agent.
- The cascaded agent continues processing input during output playback and can
  interrupt, cancel, flush, and resume or re-plan correctly.
- Cascaded providers can be mixed between local and remote placement.
- One no-Python all-local cascaded configuration is documented and tested.
- One mixed cascade—local VAD/ASR, remote model, and local TTS—is documented
  and tested.
- The official Rust/Candle Moshi backend can run as a full-duplex agent through
  the same `InProcessAi` media port on the qualified CUDA profile.
- A remote Moshi-protocol session can use the same `RealtimeSpeechProvider`
  contract and interoperates with the pinned official Rust server.
- A deterministic UCTP agent vertical and the existing Vapi external-agent
  regression pass alongside SIP and WebRTC coverage.
- No cancelled generation emits a later frame, text delta, or tool result.
- All mandatory contract, audio, transport, fault, and soak tests pass.

## 3. Current-state audit

### 3.1 Foundations to retain

- `crates/foundation/rvoip-core/src/media_graph.rs` already owns source
  receivers, fans out to bounded sinks, groups transcodes, and evicts slow
  consumers.
- media-core already has G.711, Opus, resampling, format conversion, jitter
  buffering, and an internal 16 kHz `pcm_s16le` codec.
- `ParticipantKind::Ai` and `Transport::InProcessAi` already exist.
- Orchestrator already has lifecycle guards, tenant admission semaphores,
  Connection cleanup, events, metrics, and vCon ownership.
- rvoip-vapi proves that an agent-side Connection can be bridged to a caller
  while an external service owns the higher-level voice loop.
- Existing recording and listener media taps are useful for raw recording,
  processed recording, monitoring, and evaluation.

### 3.2 Blocking correctness gaps

1. **The current AI loop is serial.** It pushes audio to ASR in one future, but
   the only ASR-event consumer blocks while draining TTS. Speech detected during
   playback is not acted on until playback has already ended.
2. **ASR partials are disabled by default.** The current attach path opens
   `AsrConfig::default()`, whose `partial_results` value is false.
3. **The provider audio contract is invalid.** `MediaFrame` is encoded payload
   in the negotiated connection codec, but providers are not told which codec
   they receive. TTS output is written directly into the negotiated stream
   without conversion or validation.
4. **Playback is not paced by the harness.** A provider can enqueue frames as
   fast as it produces them, creating bursts, excess buffering, and unusable
   interruption latency.
5. **The current default VAD path is disconnected.** Advanced VAD is constructed
   by default, while MediaSession calls the legacy path that only consults basic
   VAD. Advanced VAD also requires buffering before it accepts ordinary 20 ms
   telephony frames.
6. **Noise suppression is absent.** Existing modules explicitly defer it.
7. **AEC has no wired far-end render reference.** It cannot be considered a
   production voice-agent feature.
8. **DialogManager is one-shot.** It cannot stream tokens, call tools, consume
   DTMF/call events, accept heard-prefix corrections, or cancel speculative
   work.
9. **There is no playback ledger.** The system cannot distinguish generated,
   synthesized, queued, sent, or estimated-heard output.
10. **Observability is incomplete.** The attach path does not publish the
    complete turn lifecycle, provider failures, AI usage, latency stages, or
    vCon analyses.

### 3.3 Industry lessons applied

The design borrows behaviors, not production dependencies:

| Stack/reference | Useful behavior | rvoip implementation consequence |
|---|---|---|
| [Daily/Pipecat](https://www.daily.co/blog/daily-and-nvidia-collaborate-to-simplify-voice-agents-at-scale/) | Modular realtime pipeline, phrase endpointing, interruption handling, playout-aware context, tools, and noise processing | Typed frame/event pipeline, separate turn/interruption policies, heard-prefix ledger, async tools, pluggable DSP |
| [LiveKit Agents](https://docs.livekit.io/agents/logic/turns/) | VAD, transcription endpointing, turn models, and interruption/false-interruption policy are distinct signals | Evidence-fusing `TurnCoordinator`, explicit candidate/rejected states, configurable resume window |
| [Vapi](https://docs.vapi.ai/customization/voice-pipeline-configuration) | Provider/pipeline selection and voice behavior are configuration surfaces around telephony | Portable `AgentDefinition`, typed provider registry, independent transport/core lifecycle; preserve external Vapi topology |
| [Gradio FastRTC](https://fastrtc.org/userguide/audio/) | Per-session handlers, pause-triggered replies, interruptible streaming, and easy WebRTC demos | Deterministic session lifecycle and evaluation client; no Python/FastRTC production dependency |
| [Kyutai Moshi/Unmute](https://github.com/kyutai-labs/moshi) | Continuous bidirectional speech-native audio plus streaming cascaded STT/LLM/TTS references | Separate `RealtimeSpeechProvider`, concurrent input/output lanes, exact generation fencing, and protocol/in-process Rust adapters |

Daily/Pipecat's explicit distinction between transport, orchestration, model
services, context, and DSP reinforces the rvoip crate boundary. LiveKit and
Vapi reinforce policy configurability. FastRTC is a useful demo/evaluation
surface. Kyutai is the key evidence that a Rust-native full-duplex model path
should be first-class rather than forced through a transcript pipeline.

## 4. Target architecture

~~~mermaid
flowchart LR
    Caller["Caller Connection"] <--> Graph["rvoip MediaGraph"]
    Graph <--> AiPort["AI Participant + InProcessAi PCM Connection"]

    AiPort -->|"input always active"| Frontend["AEC reference · NS · AGC · VAD"]
    Frontend --> Asr["Streaming ASR"]
    Asr --> Turn["Turn + interruption coordinator"]
    Turn --> Model["Streaming model + application tools"]
    Model --> Tts["Streaming TTS"]
    Tts --> Playout["Paced playback + heard ledger"]
    Playout --> AiPort

    AiPort <-->|"alternative backend"| Realtime["RealtimeSpeechSession: Moshi"]
~~~

### 4.1 Crate and dependency boundaries

Keep dependency direction acyclic:

~~~text
rvoip-core-traits
    ↑
rvoip-harness
    ↑
rvoip-core

rvoip-core-traits ← rvoip-ai-local
rvoip-core-traits ← rvoip-ai-remote
rvoip-core-traits ← rvoip-ai-moshi

rvoip facade → optional core/provider crates
~~~

Planned responsibilities:

| Crate | Responsibility |
|---|---|
| rvoip-core-traits | Transport-neutral audio, provider, agent definition, event, error, and media-port contracts |
| rvoip-harness | Provider registry, session supervisor, turn/interruption policies, playback scheduler, testkit |
| rvoip-core | InProcessAi adapter/Connection, Orchestrator integration, lifecycle, bridges, events, quotas, vCon |
| media-core | Parameterized internal PCM codec, resampling, DSP primitives |
| rvoip-ai-local | Optional Silero/Smart Turn/DeepFilterNet, sherpa-onnx, mistral.rs, and experimental XN provider implementations |
| rvoip-ai-remote | Rust clients for remote streaming ASR/TTS and OpenAI-compatible model endpoints |
| rvoip-ai-moshi | Official Moshi/Candle in-process backend plus Moshi wire-protocol client |
| rvoip | Feature-gated facade and re-exports |

`rvoip-core` may depend on `rvoip-harness` behind the `voice-ai` feature.
`rvoip-harness` and provider crates must never depend on `rvoip-core`.

The executable ownership boundary is:

- `rvoip-core-traits` has no dependency on core, harness, media-core, HTTP, or
  model crates. It owns only lightweight IDs, PCM/configuration types, provider
  contracts/messages, runtime events, errors, and the channel-based media-port
  contract.
- `rvoip-harness` depends on core-traits as its only other rvoip crate. It knows
  nothing about
  `Orchestrator`, tenant storage, bridges, vCon builders, or concrete
  transports. Its default feature set contains no provider model runtime.
- Provider crates depend on core-traits as their only rvoip crate, implement
  its traits, and never
  register themselves globally. Applications construct typed providers and
  pass them to core registration methods.
- `rvoip-core`, behind `voice-ai`, stores an `Arc<ProviderRegistry>`, delegates
  provider registration to it, and owns all call-facing adapters and resource
  leases.
- With `voice-ai` disabled, core does not compile the internal adapter, live
  agent manager, harness dependency, or runtime methods. Core-traits may still
  expose the lightweight contracts.

### 4.2 Primary source changes

- Add `crates/foundation/rvoip-core-traits/src/voice_ai/` with submodules for
  audio, configuration, providers, events, errors, and media_port.
- Move `RecordingSink` and `RecordingArtifact` from `harness.rs` into
  `rvoip-core-traits/src/recording.rs`.
- Replace `crates/extensions/rvoip-harness/src/lib.rs` with the runtime facade
  and add `registry.rs`, `supervisor.rs`, `turn.rs`, `interruption.rs`,
  `playback.rs`, `session.rs`, and `testkit.rs`.
- Add `crates/foundation/rvoip-core/src/in_process_ai.rs`.
- Update `orchestrator.rs`, `commands.rs`, `events.rs`, `config.rs`, `vcon.rs`,
  and tenant-quota handling.
- Generalize `crates/media/media-core/src/codec/audio/pcm.rs` and the media graph
  codec factory to internal PCM at 16, 24, and 48 kHz.
- Replace `examples/11-ai-harness-demo` with a real concurrent deterministic
  harness example. Add separate cascaded and Moshi examples.

## 5. Public contract

The following shapes are normative. Exact module paths may change during
implementation, but fields and semantics must not be weakened without updating
this plan.

### 5.1 Identity and correlation

Add opaque, cloneable, non-secret ID newtypes:

- `AgentSessionId`: one attached agent runtime.
- `TurnId`: allocated when a caller or agent turn starts and retained through
  candidate, revision, commitment, generation, and completion.
- `GenerationId`: a monotonically increasing `u64` cancellation epoch scoped
  to one `AgentSessionId`, not a UUID.
- `ProviderId`: a registered provider instance.
- `ProviderSessionId`: one provider-side stream.
- `ProviderEpoch`: a monotonically increasing `u64` scoped to one
  `(AgentSessionId, ProviderKind)` lane.
- `RealtimeOutputId`: a provider-session-local opaque output sequence.
- `ToolCallId`: one application tool request.

Every provider command, provider event, `AgentEvent`, metric span, and vCon
analysis carries `AgentSessionId`. Turn-scoped work also carries `TurnId`.
Normalized cascaded output always carries `GenerationId`. Realtime provider
events carry `RealtimeOutputId`; the ordered harness consumer maps that ID to a
new `GenerationId` before emitting agent events or media. Provider restart or
failover increments `ProviderEpoch` before new work is admitted, so old events
are ignored. Checked epoch overflow terminates the session rather than wrapping.

`AgentSessionId` follows the repository's existing UUID-shaped ID contract:
wire and `Display` form are `agent_<32 lowercase UUID hex characters>`, while
`Debug` renders only `AgentSessionId([redacted])`. No ULID dependency is added.
`ProviderId` is configuration-stable across process restarts and must match
`[a-z0-9][a-z0-9._-]{0,63}`. Provider-session IDs may be ephemeral.

### 5.2 PCM audio

~~~rust
#[non_exhaustive]
pub enum SampleFormat {
    S16Le,
}

pub struct AudioFormat {
    sample_rate_hz: u32,
    channels: u16,
    sample_format: SampleFormat,
}

pub struct PcmFrame {
    format: AudioFormat,
    samples: Bytes,
    sequence: u64,
    start_sample: u64,
    capture_offset: Option<Duration>,
    participant_id: ParticipantId,
    discontinuity: Option<AudioDiscontinuity>,
}

pub struct AudioDiscontinuity {
    reason: AudioDiscontinuityReason,
    missing_samples: Option<Range<u64>>,
}
~~~

Rules:

- The transport-facing AI bus defaults to signed 16-bit little-endian, mono,
  48 kHz, 20 ms frames: 960 samples and 1,920 payload bytes per frame.
- The v1 public PCM contract is mono S16LE. Providers convert to private float
  tensors internally; no public float buffer crosses the provider boundary.
- Supported AI-bus rates are 16, 24, and 48 kHz. An explicit `AgentAudioConfig`
  may choose a lower rate, but 48 kHz is the default so full-band DSP and
  WebRTC audio are not prematurely discarded. The internal media codec also
  supports 8 kHz as a transport conversion boundary; 8 kHz is not an agent-bus
  rate.
- `ProviderDescriptor` declares accepted format constraints. The harness
  inserts exactly one resampler/reframer at each provider boundary.
- Moshi receives 24 kHz frames accumulated into its native model cadence. Mimi
  tokens and codebooks remain private to `rvoip-ai-moshi`.
- Audio duration derives from sample count, never wall-clock arrival spacing.
- Fields are private. `AudioFormat::pcm_s16le_mono(rate)` and validated
  `PcmFrame` constructors preserve payload, format, sample-clock, and sequence
  invariants.
- A generic `PcmFrame` accepts any non-empty, whole-sample-aligned duration up
  to one second. Exact 20 ms framing is enforced by `AgentMediaPort` and
  provider reframers, which may emit one shorter terminal frame.
- Empty, byte-misaligned, multi-channel, oversized, or unsupported-rate frames
  return a typed format error.
- `start_sample` is the authoritative monotonic clock. `capture_offset` is an
  optional correlation to the agent-local epoch, not a process-global
  `Instant`. Discontinuity includes a typed reason; an exact missing
  sample-position range is required when a detected sample gap makes it
  knowable and is `None` for resets/closures with unknown loss.
- Debug implementations report format, sample count, and IDs but never render
  sample bytes.

### 5.3 Provider metadata and registry

~~~rust
pub enum ProviderKind {
    ActivityDetector,
    TurnDetector,
    InterruptionDetector,
    SpeechRecognizer,
    AgentModel,
    TextToSpeech,
    RealtimeSpeech,
    AudioProcessor,
}

pub enum ProviderLocality {
    InProcess,
    LocalService,
    RemoteService,
}

pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub kind: ProviderKind,
    pub locality: ProviderLocality,
    pub input_formats: Vec<AudioFormatConstraint>,
    pub output_formats: Vec<AudioFormatConstraint>,
    pub languages: Vec<String>,
    pub models: Vec<String>,
    pub voices: Vec<String>,
    pub hardware: HardwareRequirements,
    pub supports_streaming: bool,
    pub supports_cancellation: bool,
    pub supports_alignment: bool,
    pub supports_provider_eot: bool,
    pub supports_idle_adaptation: bool,
    pub supports_tools: bool,
    pub supports_context_injection: bool,
    pub supports_full_duplex: bool,
    pub owns_turn_taking: bool,
    pub supports_harness_managed_overlap: bool,
    pub supports_context_correction: bool,
    pub max_concurrent_sessions: Option<usize>,
}
~~~

`ProviderDescriptor` is non-exhaustive. Add separate typed capability structs
for kind-specific data rather than adding loosely typed maps.

`ProviderRegistry`:

- Rejects an empty or duplicate `ProviderId`.
- Enforces the ID syntax in §5.1 and a hard maximum of 256 registered provider
  instances per registry.
- Validates `ProviderKind` at registration and selection.
- Stores factories/providers, descriptors, health, and admission semaphores.
- Selects the first healthy provider in `ProviderSelector.preferred` that
  satisfies all required capabilities.
- Never stores raw credentials in `AgentDefinition` or diagnostics.
- Provides unregister only when no live session references the provider.
- Exposes a redacted snapshot for operations and tests.

Registration allows at most 32 format constraints and 128 entries each for
languages, models, and voices. Every descriptor string is at most 255 bytes,
and the complete validated descriptor is at most 256 KiB after serialization.
Capability-specific collections have explicit ceilings within that aggregate.
Registration rejects a descriptor that exceeds any bound.

`AudioFormatConstraint` declares allowed sample formats, channels, discrete
rates or an inclusive rate range, and minimum/maximum accepted frame duration.
Selection resolves each constraint to one concrete `AudioFormat` and frame
duration before provider startup.

Provider-specific crates expose typed constructors such as
`KyutaiSttProvider::new(KyutaiSttConfig)` and register the constructed provider.

Every specialized provider also implements:

~~~rust
#[async_trait]
pub trait ProviderLifecycle: Send + Sync {
    fn descriptor(&self) -> &ProviderDescriptor;
    fn health_snapshot(&self) -> ProviderHealthSnapshot;
    async fn probe(
        &self,
        context: ProviderProbeContext,
    ) -> Result<ProviderHealthSnapshot, ProviderError>;
    async fn prepare(
        &self,
        context: ProviderPrepareContext,
    ) -> Result<ProviderPreparation, ProviderError>;
}
~~~

`ProviderPreparation` is an opaque, non-cloneable lease that owns the
provider/model permit and warm reservation and releases both on drop.
`open_session` consumes the matching preparation. Registry selection performs
at most one fresh bounded async probe per candidate. `selection_deadline` is a
sub-budget of `provider_start_deadline`; time spent probing reduces the
remaining session-open budget.

`ProviderProbeContext` supplies the remaining selection deadline and
cancellation only. `ProviderPrepareContext` adds selected provider kind,
model/voice/language, concrete audio formats, remaining start deadline, and
requested session capabilities; neither carries prompt/transcript/tool content
or core/tenant handles.

### 5.4 Agent definition

~~~rust
pub struct AgentDefinition {
    pub backend: AgentBackendDefinition,
    pub session: AgentSessionConfig,
    pub audio: AgentAudioConfig,
    pub turn: TurnConfig,
    pub interruption: InterruptionConfig,
    pub playback: PlaybackConfig,
    pub tools: ToolConfig,
    pub failure: AgentFailurePolicy,
    pub limits: AgentLimits,
    pub observability: AgentObservabilityConfig,
}

pub enum AgentBackendDefinition {
    Cascaded {
        activity_detector: ProviderSelector,
        turn_detector: Option<ProviderSelector>,
        interruption_detector: Option<ProviderSelector>,
        asr: ProviderSelector,
        model: ProviderSelector,
        tts: ProviderSelector,
    },
    RealtimeSpeech {
        provider: ProviderSelector,
        activity_detector: Option<ProviderSelector>,
        interruption_detector: Option<ProviderSelector>,
    },
}
~~~

`AgentDefinition` derives `Clone` and `Serialize`/`Deserialize` because it only
contains standard policy values, portable per-session instructions/language/
voice/model limits, and provider IDs. Provider credentials, model handles,
HTTP clients, and native runtimes remain in registered providers.
It uses a handwritten redacted `Debug`; instructions, initial context, tool
schemas, model/voice strings, and arbitrary provider-facing text are never
rendered.

Validation occurs before creating a Participant, Connection, media graph, task,
or quota permit. Validation checks provider existence, kinds, capabilities,
formats, language, cancellation requirements, tool support, queue limits, and
policy ranges.

Portable definition values have explicit limits so configuration cannot become
an unbounded allocation or logging surface:

- UTF-8 instructions: at most 64 KiB.
- Initial context: at most 256 items and 1 MiB after serialization.
- Provider preference route: at most 16 provider IDs per selector.
- Audio-processor chain: at most eight stages.
- Tool definitions: at most 128; each name at most 128 bytes and each JSON
  schema at most 64 KiB, with a 1 MiB aggregate schema ceiling.
- Language, model, voice, and provider identifiers: at most 255 bytes each.
- User-provided strings reject NUL and other disallowed control characters.

The concrete limits live in one versioned validation policy and have
boundary-value tests. Applications may choose stricter limits but cannot
weaken the library's safety ceiling.

### 5.5 Portable policy types

~~~rust
pub struct ProviderSelector {
    pub preferred: Vec<ProviderId>,
    pub requirements: ProviderRequirements,
    pub maximum_attempts: usize,
    pub selection_deadline: Duration,
    pub health_max_age: Duration,
}

pub struct ProviderRequirements {
    pub locality: Option<ProviderLocality>,
    pub language: Option<String>,
    pub streaming: bool,
    pub cancellation: bool,
    pub alignment: bool,
    pub provider_eot: bool,
    pub idle_adaptation: bool,
    pub tools: bool,
    pub context_injection: bool,
    pub full_duplex: bool,
    pub harness_managed_overlap: bool,
    pub context_correction: bool,
}

pub struct AgentSessionConfig {
    pub instructions: String,
    pub language: Option<String>,
    pub model: Option<String>,
    pub voice: Option<String>,
    pub initial_context: Vec<ConversationItem>,
    pub history: HistoryPolicy,
    pub response: ResponseLimits,
}

pub struct AgentAudioConfig {
    pub bus_format: AudioFormat,
    pub frame_duration: Duration,
    pub processors: Vec<AudioProcessorStage>,
}

pub struct AudioProcessorStage {
    pub selector: ProviderSelector,
    pub failure: AudioProcessorFailurePolicy,
}

pub struct TurnConfig {
    pub minimum_speech: Duration,
    pub minimum_post_speech_quiet: Duration,
    pub endpoint_silence: Duration,
    pub semantic_commit_threshold: f32,
    pub maximum_endpoint_wait: Duration,
    pub maximum_turn: Duration,
    pub asr_flush_deadline: Duration,
    pub allow_provider_eot: bool,
    pub allow_manual_commit: bool,
    pub dtmf_policy: DtmfTurnPolicy,
}

pub struct InterruptionConfig {
    pub enabled: bool,
    pub candidate_speech: Duration,
    pub confirmation_speech: Duration,
    pub lexical_confirmation: bool,
    pub false_interruption_window: Duration,
    pub backchannels: BackchannelPolicy,
    pub realtime_overlap: RealtimeOverlapPolicy,
}

pub enum RealtimeOverlapPolicy {
    ProviderNative,
    HarnessManaged,
}

pub struct PlaybackConfig {
    pub start_prebuffer: Duration,
    pub maximum_unsent_audio: Duration,
    pub output_frame_duration: Duration,
    pub estimated_playout_guard: Duration,
    pub underrun: PlaybackUnderrunPolicy,
}

pub struct ToolConfig {
    pub definitions: Vec<ToolDefinition>,
    pub policy: ToolPolicy,
}

pub struct AgentFailurePolicy {
    pub exhausted_turn: TurnFailureAction,
    pub required_detector_failure: FailureAction,
    pub media_failure: FailureAction,
}

pub struct HistoryPolicy {
    pub maximum_items: usize,
    pub maximum_bytes: usize,
    pub overflow: HistoryOverflowPolicy,
}

pub struct AgentLimits {
    pub input_queue_audio: Duration,
    pub replay_audio: Duration,
    pub maximum_playback_audio: Duration,
    pub provider_start_deadline: Duration,
    pub provider_close_deadline: Duration,
    pub session_stop_deadline: Duration,
    pub tool_deadline: Duration,
    pub maximum_tool_calls_per_turn: usize,
    pub maximum_evidence_record_bytes: usize,
}
~~~

`PlaybackConfig.maximum_unsent_audio` is requested behavior;
`AgentLimits.maximum_playback_audio` is the hard safety ceiling. Validation
requires the request not exceed the ceiling.

`PlaybackUnderrunPolicy` is one of:

- `InsertSilence { maximum: Duration }`;
- `InsertComfortNoise { maximum: Duration }`;
- `PauseGeneration { maximum: Duration }`;
- `FailGeneration`.

The first three must have a nonzero maximum no greater than
`maximum_playback_audio`. When the maximum expires, the policy becomes
`FailGeneration`; no variant can insert or pause indefinitely.

`HistoryOverflowPolicy` is `DropOldestAfterEvidence` or `FailTurn`. The default
model-context window is 256 items/1 MiB with a hard library ceiling of
512 items/2 MiB. Dropped items must already have crossed the lossless evidence
boundary and emit `HistoryWindowAdvanced`; no automatic model-generated
summary is invented. Applications may inject an explicit checkpoint/summary.

`maximum_evidence_record_bytes` has a 64 KiB hard ceiling, measured after the
canonical binary encoding. Provider messages, application inputs, transcript
revisions, and tool results are validated or chunked before a state mutation
could exceed it. Combined with the fixed journal record count, this keeps the
normal and terminal replay memory strictly bounded.

Selectors are tried in declared order. `maximum_attempts` includes the primary
attempt and cannot exceed the number of preferred providers. Selection ignores
stale health only after an explicit bounded async probe within
`selection_deadline`; it never waits indefinitely for health recovery.
Definition validation is pure and checks registered static descriptors only;
health probing, concrete selection, and provider admission occur during
`AgentRuntime::prepare`.

`ToolDefinition` contains a stable name, description, and JSON Schema and is
portable/serializable. `ToolPolicy` declares maximum concurrent calls,
per-tool idempotency, and whether outstanding work is cancelled or allowed to
finish after interruption. In v1, the harness emits
`AgentEvent::ToolCallRequested`, and the application returns
`AgentInput::ToolResult` through `AgentHandle`; tool implementations and
credentials never enter the definition or registry. Duplicate `ToolCallId`
results are idempotently ignored, unknown IDs are rejected, timed-out and
cancelled IDs are terminal, and late results cannot reopen a generation. An
in-process registered `ToolExecutor` is a later additive feature.

`AgentObservabilityConfig` selects transcript detail, recording/evaluation
taps, vCon projection, and content-redaction policy. Its `Debug` output never
renders instructions, context, voice names, tool names, or transcript content.
Production recording taps are typed `AgentRecordingRequest` values containing
the registered sink-factory ID, `RawInput | ProcessedInput | AgentOutput | Mix`
tap, source/channel policy, consent reference, retention-policy reference, and
`ContinueWithoutRecording | FailStart | FailSession` failure policy. At most
four requests are allowed. There are no recording booleans or implicit sink.

### 5.6 Standard configuration

Initial `Balanced` defaults:

| Setting | Default |
|---|---:|
| AI bus | mono S16LE, 48 kHz, 20 ms |
| Input pre-roll retained | 300 ms |
| Maximum replayable active turn | 15 s |
| Minimum speech for a normal user turn | 120 ms |
| Minimum post-speech quiet | 120 ms |
| Normal endpoint silence | 500 ms |
| Semantic commit threshold | 0.80 |
| Maximum endpoint wait after provider evidence | 1,500 ms |
| Maximum user turn | 60 s |
| ASR flush deadline | 500 ms |
| Model history window | 256 items / 1 MiB |
| Interruption candidate speech | 80 ms |
| Interruption confirmation | 200 ms or first stable lexical word |
| False-interruption resume window | 1,000 ms |
| Realtime overlap | Provider-native |
| Playback start prebuffer | 60 ms |
| Maximum unsent playback | 200 ms |
| Hard playback safety ceiling | 500 ms |
| Output frame cadence | 20 ms |
| Playback underrun | Insert silence for at most 200 ms, then fail generation |
| Exhausted ASR/turn failure | Emit failed turn and return to listening |
| Required detector failure | Fail current turn |
| Media failure | Fail session |
| AGC | Off |
| Server AEC | Off |
| Noise suppression | Off unless explicitly enabled |

Every value is configurable. Tuning may change only after the evaluation corpus
records the before/after false-cutoff, false-interruption, and latency results.

### 5.7 Channel-based provider sessions

Provider `open_session` methods return bounded command/event channels plus a
control handle. They do not expose one object that must be mutably borrowed for
both input and output.

~~~rust
pub struct ProviderSession<C, E> {
    pub id: ProviderSessionId,
    pub commands: mpsc::Sender<C>,
    pub events:
        mpsc::Receiver<Result<ProviderEventEnvelope<E>, ProviderError>>,
    pub control: Arc<dyn ProviderSessionControl>,
}

pub enum CancelScope {
    Turn(TurnId),
    Generation(GenerationId),
    RealtimeOutput(RealtimeOutputId),
    Session,
}

#[async_trait]
pub trait ProviderSessionControl: Send + Sync {
    fn request_cancel(
        &self,
        scope: CancelScope,
    ) -> Result<(), ProviderError>;
    async fn close(&self) -> Result<(), ProviderError>;
}
~~~

Requirements:

- Commands and events are ordered within a provider session.
- `close` is idempotent.
- After `close` completes, no new event may be enqueued. Events already in the
  bounded receiver may remain and are rejected by the harness session/epoch
  fence.
- `request_cancel` applies only to the supplied turn/generation/session scope.
- Provider tasks terminate when command receivers close or the parent session
  cancellation token fires.
- `request_cancel` is a synchronous out-of-band control operation. It must not
  wait behind queued audio, text, or ordinary provider commands. The harness
  advances its generation fence before calling it; provider acknowledgement is
  never part of the safety boundary.
- Ordered flush remains a typed provider command so it cannot overtake prior
  audio/text. Once close begins, new commands and cancel/flush requests fail
  with `Closing`; concurrent close calls await the same completion.
- Concurrent flush, cancellation, and close are contract-tested.
  Cancellation acknowledgement can emit diagnostics but can never reactivate
  a generation or weaken the harness fence.
- `ProviderError` is structured and redacted; it includes kind, retryability,
  provider ID, provider epoch, and safe diagnostics.

Provider factories expose the same session construction shape. The exact
command and event enums are defined in the contract crate:

~~~rust
#[async_trait]
pub trait SpeechRecognizerProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: SpeechRecognizerConfig,
    ) -> Result<
        ProviderSession<SpeechRecognizerCommand, SpeechRecognizerEvent>,
        ProviderError,
    >;
}

#[async_trait]
pub trait AgentModelProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: AgentModelConfig,
    ) -> Result<
        ProviderSession<AgentModelCommand, AgentModelEvent>,
        ProviderError,
    >;
}

#[async_trait]
pub trait TextToSpeechProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: TextToSpeechConfig,
    ) -> Result<
        ProviderSession<TextToSpeechCommand, TextToSpeechEvent>,
        ProviderError,
    >;
}

#[async_trait]
pub trait RealtimeSpeechProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: RealtimeSpeechConfig,
    ) -> Result<
        ProviderSession<RealtimeSpeechCommand, RealtimeSpeechEvent>,
        ProviderError,
    >;
}
~~~

Activity, turn, interruption, and audio-processor provider traits use the same
`ProviderLifecycle` supertrait and consume `ProviderPreparation` in
`open_session`. `ProviderOpenContext` carries `AgentSessionId`,
`ProviderSessionId`, the immutable provider epoch, selected input/output format
and frame-duration negotiation, bounded command/event capacities, language, a
parent cancellation token, and an injectable monotonic clock. It does not
carry credentials, tool implementations, the full `AgentDefinition`, or core
resource handles.

Returning from `open_session` is the readiness acknowledgement: the provider
has completed required allocation and can accept its first command. The
provider must use the requested channel bounds or reject the configuration.
`ProviderEventEnvelope` supplies the fixed session ID and provider epoch; the
event supplies its turn/generation or realtime-output scope.

### 5.8 Cascaded provider contracts

Activity commands and events are:

- Commands: `Audio(PcmFrame)` and `Reset { reason, discontinuity }`.
- Events: `Probability { sample_range, probability }`,
  `SpeechStarted { start_sample }`, `SpeechContinued { sample_range }`, and
  `SpeechStopped { end_sample }`.

Turn-detector commands and events are:

- Commands: `BeginTurn { turn_id }`, turn-scoped `Audio`,
  `TranscriptRevision`, generic `TurnEvidence`, `EndOfSpeech`, and `Reset`.
- Events: `Continue { turn_id, probability, horizon }`,
  `CommitRecommended { turn_id, probability, reason }`, and
  `Uncertain { turn_id, reason }`.

Interruption-detector commands and events are:

- Commands: `BeginOverlap { turn_id, generation_id, playback }`,
  overlap-scoped `Audio`, `TranscriptRevision`, `EndOverlap`, and `Reset`.
- Events: `Classification { turn_id, generation_id, class, confidence }`,
  where class is `Interruption`, `Backchannel`, `Noise`, or `Uncertain`.

Audio-processor commands and events are:

- Commands: `Capture(PcmFrame)`, optional aligned
  `RenderReference(PcmFrame)`, `Flush`, and `Reset`.
- Events: `ProcessedBatch { input_sample_range, frames, latency }`,
  `ReferenceAlignment`, and `ProcessorBypassed`. A processed batch contains
  zero to four ordered frames; zero explicitly means suppression, never a
  silently dropped input.

Their factory signatures are the same as the other provider families:

~~~rust
#[async_trait]
pub trait ActivityDetectorProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: ActivityDetectorConfig,
    ) -> Result<
        ProviderSession<ActivityCommand, ActivityEvent>,
        ProviderError,
    >;
}

#[async_trait]
pub trait TurnDetectorProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: TurnDetectorConfig,
    ) -> Result<
        ProviderSession<TurnDetectorCommand, TurnDetectorEvent>,
        ProviderError,
    >;
}

#[async_trait]
pub trait InterruptionDetectorProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: InterruptionDetectorConfig,
    ) -> Result<
        ProviderSession<InterruptionCommand, InterruptionEvent>,
        ProviderError,
    >;
}

#[async_trait]
pub trait AudioProcessorProvider: ProviderLifecycle {
    async fn open_session(
        &self,
        preparation: ProviderPreparation,
        context: ProviderOpenContext,
        config: AudioProcessorConfig,
    ) -> Result<
        ProviderSession<AudioProcessorCommand, AudioProcessorEvent>,
        ProviderError,
    >;
}
~~~

All detector/processor sessions negotiate format and frame-duration constraints
through `ProviderOpenContext`, preserve input ordering/sample positions, and
emit no result for an earlier sample range after a later range. Any skipped
range forces a typed reset before more stateful inference. Detector timeouts
fall back to the deterministic harness policy only when the selector is
optional; a required detector fails the turn/session according to
`AgentFailurePolicy`. Processor timeout follows its configured `Bypass`,
`FailTurn`, or `FailSession` policy, and bypass always emits a discontinuity/
latency event rather than silently changing audio.

Speech recognizer commands:

- `BeginTurn { turn_id, pre_roll }`
- `Audio { turn_id: TurnId, frame: PcmFrame }`
- `CommitTurn(TurnId)`
- `Flush { turn_id }`
- `Reset { reason, discontinuity }`

Speech recognizer events:

- `SpeechStarted` / `SpeechStopped` with `TurnId`
- `PartialTranscript` with turn, revision, stability, language, and time range
- `FinalTranscript` with turn, words, and word timestamps when supported
- `TurnEvidence` with turn, source, probability, and prediction horizon
- `ProviderEndpoint` with turn
- `Flushed` with turn
- `Usage`

In cascaded v1, the harness retains quiet/pre-roll audio locally and does not
send it to ASR. On speech-candidate activation it sends `BeginTurn` with the
pre-roll exactly once, then sends only new live turn-scoped `Audio`. A provider
that needs idle adaptation declares `supports_idle_adaptation`; adaptation
frames use a separate command and never duplicate samples in the transcribed
turn.

Agent model commands:

- `BeginGeneration` with turn, generation, canonical committed history, caller
  transcript, and redacted call context
- `Dtmf` with turn/generation scope
- `ToolResult` with generation and `ToolCallId`
- `CallEvent` with generation scope
- `CorrectAssistantHistory` with generation and the last heard assistant prefix

Agent model events:

- `GenerationStarted` with turn/generation
- `TextDelta` with turn/generation
- `ToolCall` / `ToolCallCancelled` with turn/generation
- `EndTurn` with turn/generation
- `HandoffRequested` with turn/generation
- `EndSession` with turn/generation
- `Usage`

TTS commands:

- `BeginGeneration` with turn, generation, voice, and output format
- `TextDelta` with generation
- `FlushText` with generation
- `EndInput` with generation

TTS events:

- `SynthesisStarted` with `GenerationId`
- `Audio` with `GenerationId` and `PcmFrame`
- `Alignment` with generation, UTF-8 byte offsets, optional word boundaries,
  and audio sample ranges
- `SynthesisFinished` with `GenerationId`
- `Usage` with generation scope

The supervisor may send model deltas into TTS as they arrive. It must use a
text chunker that respects Unicode, punctuation, abbreviations, and minimum
chunk size so TTS starts early without speaking unstable single-token
fragments.

### 5.9 Realtime speech contract

`RealtimeSpeechProvider` opens one continuous stateful session.

Commands:

- `Audio(PcmFrame)`
- `Dtmf`
- `ToolResult`
- `InjectContext`

Events:

- `UserSpeechState`
- `ModelSpeechState`
- `UserTranscriptDelta`
- `ModelTranscriptDelta`
- `OutputStarted { output_id: RealtimeOutputId }`
- `OutputAudio { output_id: RealtimeOutputId, frame: PcmFrame }`
- `OutputFinished { output_id: RealtimeOutputId }`
- `ToolCall` / `RetrievalRequest`
- `ContextAccepted`
- `Usage` / `Latency`

Transcript events are observability and tool-context aids. The harness must not
require an intermediate transcript before forwarding model output audio.

The ordered harness consumer allocates one `GenerationId` when it consumes
`OutputStarted` and maps every event with that `RealtimeOutputId` to the
generation. `OutputAudio` before its matching start is a provider protocol
error. An adapter for an upstream protocol without output IDs creates a
monotonic local ID and emits `OutputStarted` before releasing the first audio
frame. On cancellation, the harness first advances its generation fence, then
calls `request_cancel(CancelScope::RealtimeOutput(output_id))`; provider events
never allocate or retag harness generations.

Realtime providers declare whether they own endpointing and interruption.
Moshi uses `RealtimeOverlapPolicy::ProviderNative` by default: overlapping user
speech does not automatically pause or cancel model output. Harness VAD may
still run for metrics, echo diagnostics, and application policy, but it must
not impose cascaded silence endpointing or the cascaded candidate/pause
transaction on Moshi audio. `HarnessManaged` overlap is valid only when the
provider descriptor advertises that capability. Manual generation cancellation
remains available in either policy through the out-of-band control path.

### 5.10 Media port and public handle

`AgentMediaPort` is created by rvoip-core and consumed by rvoip-harness. It
owns:

- A bounded receiver for caller-to-agent PCM.
- A bounded, unpaced agent-to-caller PCM sink accepting
  `AgentOutputFrame { generation_id, pcm }`.
- A render-reference receiver forked only after the output frame is accepted at
  the core/media boundary.
- A synchronous generation-advance/flush control shared with the core sink.
- Route health and terminal-state watch channels.

The harness owns pacing. Core accepts or rejects one frame and never sleeps to
establish cadence. Before an interruption awaits provider cancellation, the
harness atomically advances the port generation and clears its capacity-one
source slot. Core compares every envelope with the shared generation again
immediately before MediaGraph injection, so a stale frame already handed to
the port cannot escape.

~~~rust
impl AgentMediaPort {
    pub fn advance_generation(
        &self,
        next: GenerationId,
    ) -> Result<MediaFlushReport, VoiceAiError>;

    pub async fn send_output(
        &self,
        frame: AgentOutputFrame,
    ) -> Result<MediaAcceptance, VoiceAiError>;

    pub async fn recv_input(&mut self) -> Option<PcmFrame>;
}
~~~

The output handoff is a custom replaceable one-slot buffer, not a sender-only
`mpsc`. Normal `send_output` backpressures while the slot is occupied;
`advance_generation` synchronously verifies a strictly increasing generation,
updates the shared epoch, takes/drops a stale slot, and reports dropped
frame/sample ranges. The core pump checks the generation when taking the slot
and again immediately before graph injection. `send_output` resolves with
`MediaAcceptance { generation_id, sample_range, injected_at,
downstream_budget }` only after that second check and successful injection.
Only then is the render-reference copy forked. A generation advance while a
send is waiting returns a typed stale-generation error.

`AgentHandle` is cloneable and replaces `AiAttachmentId`-only control:

- `id()`
- `participant_id()`
- `target_connection_id()`
- `ai_connection_id()`
- `state()`
- `subscribe()`
- `send(AgentInput)`
- `inject_dtmf()`
- `send_tool_result()`
- `inject_context()`
- `cancel_generation(GenerationId)`
- `stop(reason)`
- `wait_closed()`

The public control/state schemas are:

~~~rust
pub enum AgentInput {
    ManualCommit { expected_turn: Option<TurnId> },
    InjectDtmf { digit: DtmfDigit, duration: Duration },
    ToolResult { call_id: ToolCallId, result: ToolResult },
    InjectContext {
        items: Vec<ConversationItem>,
        mode: ContextInjectionMode,
    },
}

pub struct AgentEventEnvelope {
    pub sequence: u64,
    pub session_id: AgentSessionId,
    pub emitted_at: Duration,
    pub event: AgentEvent,
}

pub enum AgentEvent {
    StateChanged { previous: AgentState, current: AgentStateSnapshot },
    UserSpeech { turn_id: TurnId, state: SpeechState, samples: Range<u64> },
    TurnEvidence { turn_id: TurnId, evidence: TurnEvidence },
    Transcript { turn_id: TurnId, revision: TranscriptRevision },
    TurnCommitted { turn_id: TurnId, reason: TurnCommitReason },
    TurnFailed { turn_id: TurnId, reason: TurnFailureReason },
    Generation { generation_id: GenerationId, state: GenerationState },
    Playback { generation_id: GenerationId, position: PlaybackPosition },
    Interruption { turn_id: TurnId, generation_id: GenerationId, state: InterruptionState },
    ToolCallRequested { generation_id: GenerationId, call: ToolCall },
    ToolCallTerminal { generation_id: GenerationId, call_id: ToolCallId, outcome: ToolOutcome },
    Provider { kind: ProviderKind, epoch: ProviderEpoch, state: ProviderState },
    AudioDiscontinuity(AudioDiscontinuity),
    Terminal(Arc<AgentRuntimeOutcome>),
}

pub struct AgentStateSnapshot {
    pub session: AgentSessionState,
    pub input: AgentInputState,
    pub output: AgentOutputState,
    pub active_turn: Option<TurnId>,
    pub active_generation: Option<GenerationId>,
    pub providers: Vec<(ProviderKind, ProviderEpoch, ProviderState)>,
    pub playback: Option<PlaybackPosition>,
    pub terminal: Option<Arc<AgentRuntimeOutcome>>,
}

pub enum AgentStopReason {
    Application,
    TargetEnded,
    SessionEnded,
    MediaFailed,
    ProviderFailed,
    OrchestratorDrain,
}

pub struct AgentRuntimeOutcome {
    pub terminal_state: AgentTerminalState,
    pub reason: AgentStopReason,
    pub usage: AgentUsage,
    pub final_evidence: AgentEvidenceSnapshot,
    pub unacknowledged_evidence: Vec<AgentEvidence>,
    pub forced: bool,
}

pub struct AgentStopReport {
    pub outcome: Arc<AgentRuntimeOutcome>,
    pub already_terminal: bool,
    pub core_cleanup: CoreAgentCleanupReport,
}
~~~

All nested events carry their applicable turn/generation IDs; content-bearing
variants follow the §11.2 surface policy and use redacted `Debug`. `send`
returns an acknowledgement only after the bounded runtime command queue accepts
the input. On a terminal session it returns `SessionClosed`; an unknown or
terminal tool call returns its typed status. `cancel_generation` returns
`Cancelled`, `AlreadyTerminal`, or `Stale` and never cancels a newer
generation. `stop` is idempotent and every caller awaits the same outcome;
late `state`/`wait_closed` callers immediately observe the retained terminal
value.

The provider-neutral schemas through `AgentRuntimeOutcome` live in
core-traits. `AgentHandle`, `AgentStopReport`, and
`CoreAgentCleanupReport` live in core because they describe core-owned call
resources.

Dropping `AgentHandle` does not stop the agent. Orchestrator owns the registered
session; explicit `stop`, caller/Session termination, or orchestrator shutdown
converges all tasks and routes.

`AgentHandle` is defined in core. It contains IDs, retained state/event
receivers, and a weak reference to the core agent manager; the manager owns the
runtime and resource leases. This avoids a strong `Orchestrator` ownership
cycle. `subscribe()` returns a bounded broadcast receiver whose `recv()`
reports `Lagged { skipped }` and remains subscribed.

### 5.11 Error model

Add a non-exhaustive `VoiceAiError` with stable categories:

- `InvalidDefinition`
- `UnsupportedCapability`
- `UnsupportedAudioFormat`
- `ProviderNotFound`
- `ProviderUnavailable`
- `ProviderProtocol`
- `ProviderTimeout`
- `ProviderOverloaded`
- `ModelLoad`
- `ModelRuntime`
- `QueueOverflow`
- `MediaRoute`
- `Cancelled`
- `SessionClosed`
- `StaleGeneration`
- `ShuttingDown`
- `Internal`

Errors contain safe provider/session correlation and an optional retry
classification. They must not contain credentials, endpoint query strings,
prompt or transcript text, audio bytes, tool arguments, or arbitrary remote
response bodies in `Debug` or `Display`.

Provider APIs return `ProviderError`; harness preparation/runtime APIs return
`VoiceAiError`; Orchestrator methods return core's `Result<T>` after adding
`RvoipError::VoiceAi(VoiceAiError)`. Public examples never use an ambiguous
unqualified error type across a crate boundary.

## 6. Runtime design

### 6.1 Ownership split

`rvoip-core` owns:

- Creation and removal of the AI Participant and `InProcessAi` Connection.
- Caller/AI bridging and media-graph lifecycle.
- Call, Session, Conversation, tenant, quota, event, and vCon integration.
- Construction of `AgentMediaPort`.
- Connection-terminal and orchestrator-shutdown signals.

`rvoip-harness` owns:

- Validation against the provider registry.
- Provider session acquisition and release.
- The concurrent cascaded or realtime voice loop.
- Input preprocessing, turn/interruption decisions, text chunking, TTS
  scheduling, generation cancellation, and playback accounting.
- Provider failover and provider-local health updates.
- Provider-neutral `AgentEvent` production.

Core retains the tenant permit, route/resource handles, and authoritative
cleanup state. It starts the harness with a validated definition, registry
handle, media port, redacted call-context snapshot, and terminal-state watch.
The harness returns an `AgentRuntimeHandle`. Core wraps that with core IDs and
lifecycle behavior to expose `AgentHandle`.

This boundary keeps provider dependencies out of SIP-only builds while keeping
call resources and tenant policy under Orchestrator ownership.

The core-to-harness interface is explicitly two phase:

~~~rust
impl ProviderRegistry {
    pub fn validate(
        &self,
        definition: &AgentDefinition,
    ) -> Result<ValidatedAgentDefinition, VoiceAiError>;
}

impl AgentRuntime {
    pub async fn prepare(
        registry: Arc<ProviderRegistry>,
        definition: ValidatedAgentDefinition,
        context: AgentRuntimeContext,
    ) -> Result<PreparedAgentRuntime, VoiceAiError>;
}

impl PreparedAgentRuntime {
    pub async fn start(
        self,
        media: AgentMediaPort,
        terminal: watch::Receiver<AgentTerminalSignal>,
    ) -> Result<AgentRuntimeHandle, VoiceAiError>;
}
~~~

`prepare` acquires provider permits and warm reservations but does not open call
media. `start` opens provider sessions, launches owned tasks, and does not
return until its readiness barrier proves every critical lane can accept work.
Dropping an unstarted `PreparedAgentRuntime` releases all provider permits and
warm reservations. `AgentRuntimeContext` contains only agent/call correlation,
the redacted call-context snapshot, validated observability policy, and clock/
cancellation facilities; it never contains a tenant permit or core resource
handle.

`AgentRuntimeHandle` exposes:

- a bounded runtime-command sender;
- a retained state watch receiver;
- a factory for per-agent broadcast receivers;
- one dedicated `AgentEvidenceStream` for core, exposing bounded batches and
  monotonic contiguous acknowledgements;
- a retained
  `watch::Receiver<Option<Arc<AgentRuntimeOutcome>>>` completion value;
- an idempotent orderly-stop request;
- a deadline-only forced-abort method available to core cleanup.

The handle owns provider sessions/tasks after start. It has no callback or
reference to core or `Orchestrator`. Core takes and begins draining and
acknowledging the evidence stream before bridge activation. Observer broadcast
may lag without affecting evidence. The completion watch changes from `None`
to one immutable `Arc` exactly once, so late handles immediately observe the
outcome.

Evidence uses a single serialized, ACK-backed journal rather than a
`try_send` queue. The journal retains 512 records: 508 general records and four
slots reserved exclusively for fatal closure. `AgentEvidenceStream::next_batch`
returns bounded, ordered copies; core acknowledges only the highest contiguous
sequence that it has incorporated into its per-agent projection. An
acknowledgement releases that prefix. Every canonical harness state mutation
and its evidence append occur under the same coordinator; a mutation is not
committed if normal evidence admission has closed.

When 508 general records are simultaneously unacknowledged, or when the sole
core stream is dropped, the coordinator atomically closes general admission,
cancels the worker lanes, and uses the reserved slots for the fixed
`EvidenceSinkFailed`, `RuntimeFailing`, final-ledger, and `RuntimeTerminal`
records. The terminal outcome copies the complete remaining journal into
`unacknowledged_evidence`. Core merges that bounded tail with its projection by
sequence number, discards duplicates, verifies contiguity and the rolling hash,
then performs the terminal vCon projection. Thus slow or failed consumption
terminates the call, but cannot make the accepted canonical record
unreconstructible. Contract tests must prove that no other path can consume the
four reserved slots or create evidence after `RuntimeTerminal`.

The snapshot is bounded: last evidence sequence, rolling evidence hash,
terminal state, final model-context window, aggregate usage, active/last turn,
tool terminal states, and playback ledger. Core retains the complete per-turn
record as it drains evidence and verifies sequence/hash at terminal. The
snapshot is reconciliation material, while `unacknowledged_evidence` is only
the bounded replay tail; neither is an unbounded duplicate of a long call.

### 6.2 Orthogonal state machines

Do not model the session with one `Listening | Thinking | Speaking` enum. Input
and output progress independently.

Session state:

~~~text
Starting → Running → Stopping → Stopped
    └──────────────→ Failed
~~~

Input state for cascaded mode:

~~~text
Quiet → SpeechCandidate → InSpeech → EndpointPending → Committed → Quiet
          └──────────────→ Quiet
~~~

Output state:

~~~text
Idle → Generating → Synthesizing → Buffering → Playing → Idle
                    └────────────→ Cancelling → Idle
~~~

`InSpeech` and `Playing` may be active simultaneously. Realtime speech mode
replaces the cascaded input/output decision states with provider-reported user
and model speech states, but the session lifecycle remains the same.

Every transition:

- Is driven by one typed event.
- Records session/turn/generation IDs.
- Has one owner task.
- Emits a redacted `AgentEvent`.
- Updates metrics before any subsequent transition is processed.
- Is covered by deterministic state-machine tests.

### 6.3 Supervisor task tree

Use one parent supervisor and owned child tasks:

~~~text
AgentSupervisor
├── media_ingress
├── core_signals
├── render_reference
├── audio_frontend
├── activity_detector
├── asr_input
├── asr_events
├── turn_coordinator
├── model_commands
├── model_events
├── tts_commands
├── tts_events
├── playback_scheduler
├── tool_event_router
├── evidence_forwarder
└── observability_forwarder
~~~

Realtime mode replaces ASR/model/TTS tasks with:

~~~text
├── realtime_input
├── realtime_events
└── playback_scheduler
~~~

Rules:

- The supervisor owns every child join handle.
- Child tasks never detach themselves.
- Provider tasks receive the parent cancellation token.
- An expected lane EOF is translated into a typed internal event; it does not
  silently end the session.
- Harness shutdown closes producers first, drains bounded terminal events,
  closes providers with deadlines, stops playback, releases provider permits,
  and signals runtime termination.
- Core cleanup then removes media routes, the AI Connection and Participant,
  registry entries, and finally the tenant permit. Core performs this sequence
  after either orderly harness shutdown or a forced-shutdown deadline.
- Forced abort occurs only after the configured shutdown deadline and emits a
  forced-shutdown event and metric.

### 6.4 Internal channels and backpressure

All queues are bounded by audio duration or item count and report occupancy.

Initial capacities:

| Queue | Capacity | Overflow behavior |
|---|---:|---|
| AI media ingress | 25 × 20 ms frames | Mark discontinuity; reset affected provider lane |
| Frontend → activity | 5 frames | Drop oldest and inject discontinuity/reset before the next frame |
| Frontend → ASR/realtime | 50 frames | Never silently drop; declare discontinuity and restart/fail |
| ASR/model/TTS events | 128 events | Provider protocol failure if producer ignores backpressure |
| TTS audio before scheduler | 200 ms | Backpressure provider; never grow |
| Playback ready queue | 200 ms | Backpressure; cancellation can clear it atomically |
| Core lifecycle/DTMF signal queue | 64 events | High-priority overflow fails runtime; terminal signal also has a watch |
| Core evidence ACK journal | 512 records: 508 general + 4 fatal-closure reserve | Core drains bounded batches and ACKs contiguous sequences; an unacked-limit or closed stream atomically fails the runtime and copies the complete unacked tail into the outcome |
| AgentEvent broadcast ring | 256 events | Receiver reports `Lagged { skipped }` and continues |

Media ingress must never block rvoip's transport receive loop on a slow model.
When an ASR/realtime lane cannot keep up, the supervisor records the missing
sample interval, advances the provider epoch, and follows configured
restart/failover policy. It must not pretend that discontinuous audio is
continuous.

MediaGraph, the AI source slot, codec/packetizer, transport send queue, network
jitter, and receiver playout can all contribute hidden output latency. WP2
must inventory and bound each downstream queue; the capacity-one AI source
slot is one fence, not proof that the complete path has only one queued frame.

### 6.5 Cascaded turn flow

1. MediaGraph converts caller media to the configured AI-bus PCM.
2. The frontend always publishes processed PCM to activity/DSP and the
   authorized recording tap; it retains the bounded pre-roll locally.
3. Activity starts a candidate, allocates `TurnId`, sends ASR `BeginTurn` with
   pre-roll exactly once, and then streams new turn-scoped audio.
4. ASR partial revisions and provider EOT are evidence, not automatic commits.
5. `TurnCoordinator` combines:
   - activity start/stop;
   - elapsed silence;
   - provider endpoint/EOT;
   - ASR stability and punctuation;
   - optional semantic/acoustic turn evidence;
   - maximum wait and maximum turn duration;
   - manual/DTMF commit.
6. Eligible endpoint evidence creates a commit candidate. The coordinator sends
   ASR `CommitTurn` followed by ordered `Flush`, waits at most
   `asr_flush_deadline`, and applies the last in-deadline final revision.
7. It commits irreversibly, allocates `GenerationId`, sends
   `BeginGeneration` to the model, and keeps audio ingress live for the next
   turn candidate.
8. Model text deltas pass through the stable-prefix chunker into TTS.
9. TTS PCM and alignment enter the generation-scoped playback queue.
10. Playback begins after the start prebuffer or a provider end condition,
   whichever occurs first, and then runs at media cadence.
11. Model, TTS, and playback completion close the agent turn independently.

### 6.6 Endpointing policy

Acoustic VAD is a speech-presence sensor, not the endpointing implementation.

The initial hybrid detector creates a commit candidate when:

- minimum speech was observed; and
- either provider EOT is received and minimum post-speech quiet has elapsed,
  semantic EOT crosses `semantic_commit_threshold`, or normal endpoint silence
  expires; and
- the evidence remains eligible through the current event.

The maximum endpoint timer guarantees progress when provider/semantic evidence
never arrives. Maximum user-turn duration forces a commit with
`TurnCommitReason::MaximumDuration`.

Before irreversible commit, a new ASR revision or resumed speech may withdraw
the candidate. Once candidate conditions hold, the coordinator orders
`CommitTurn`/`Flush`, waits up to `asr_flush_deadline`, and applies the final
revision. A later revision is recorded as `LateTranscriptRevision` and never
reopens a committed turn. On flush timeout, use the latest stable revision only
when it meets configured stability; otherwise emit `AgentTurnFailed` and apply
`AgentFailurePolicy`.

Manual commit bypasses silence waiting but still flushes ASR. DTMF may be
configured as a separate event, as input appended to the current turn, or as a
commit trigger. These behaviors are explicit `TurnConfig` fields.

### 6.7 Cascaded interruption transaction

This transaction applies to cascaded mode by default. A realtime provider using
`ProviderNative` overlap follows its own simultaneous-speech behavior and is
not paused by these candidate rules; explicit application cancellation still
uses the same generation fence. `HarnessManaged` realtime mode opts into this
transaction only when its provider advertises support.

Cascaded barge-in has monitoring, candidate, confirmed, and rejected phases.

**Monitoring**

- Raw VAD onset while output is buffering/playing starts the continuous-speech
  timer and overlap ASR/pre-roll, but does not pause playback.
- Speech ending before `candidate_speech` is classified and recorded without
  changing output.

**Candidate**

- Starts after `candidate_speech` (80 ms by default) of continuous speech, or
  earlier when ASR supplies a stable lexical word that is not in the configured
  backchannel set.
- Pauses the playback scheduler at the next frame boundary; it does not yet
  destroy the current generation.
- Continues feeding activity and ASR.
- Records the last frame sent before the pause.

**Confirmed**

Confirmation occurs when continuous speech reaches `confirmation_speech`
(200 ms by default) from raw onset, a stable non-backchannel lexical word
arrives, DTMF policy requests interruption, or a turn detector explicitly
classifies the overlap as an interruption.
Configured backchannel words never provide lexical confirmation.

The coordinator atomically:

1. Advances the active generation epoch.
2. Marks the prior generation cancelled.
3. Cancels outstanding model tool work when safe/idempotent.
4. Requests out-of-band model and TTS cancellation for the prior
   `GenerationId`.
5. Clears all unsent frames and alignment entries for that generation.
6. Advances and flushes the core media-port generation so a frame already
   handed across the harness boundary is rejected.
7. Prevents every queue from accepting late prior-generation items.
8. Computes the last estimated-heard UTF-8 byte/word boundary.
9. Mutates the harness-owned canonical history to the heard prefix.
10. Sends the corrected projection to the active model session.
11. Emits `InterruptionConfirmed` and `PlaybackStopped`.
12. Opens or continues the user's active turn.

**Rejected / false interruption**

If speech ends before confirmation and is classified as noise or a backchannel:

- Emit `FalseInterruption`.
- Keep the original generation valid.
- Resume from the first not-yet-sent frame within the one-second resume window.
- Never replay a frame already sent.
- If the generation naturally finished while paused, drain its remaining valid
  queue normally.

Backchannel classification initially uses duration plus configurable lexical
acknowledgements. Smart Turn or provider acoustic evidence may improve the
decision later without changing the transaction. If classification remains
`Uncertain` when `false_interruption_window` reaches one second, confirm
conservatively and cancel; playback may never remain paused indefinitely.

### 6.8 Playback and heard ledger

Track these monotonically advancing stages per generation:

1. model text generated;
2. text accepted by TTS;
3. audio synthesized;
4. audio queued;
5. audio accepted by the core/media source;
6. audio accepted by a transport adapter when test/future feedback exists;
7. audio estimated as heard.

Elsewhere in the v1 interruption policy, “sent” means successfully
core-accepted unless a concrete transport adapter supplies stronger feedback.

The scheduler:

- Starts with a 60 ms prebuffer and holds at most 200 ms unsent audio.
- Emits one 20 ms frame per tick.
- Uses `MissedTickBehavior::Skip`; it never catches up by sending a burst.
- Keeps the harness-to-core AI source slot at one frame; WP2 separately bounds
  every downstream MediaGraph/codec/transport queue.
- Assigns monotonic PCM/RTP timestamps from the sample clock.
- Emits an underrun event and configured silence/comfort behavior instead of
  inventing timestamps.
- Receives a render-reference copy only after core accepts the
  generation-checked frame and before codec encoding.

RTP cannot recall already-sent packets. V1 has no general transport-send or
receiver-playout acknowledgement contract. `estimated_heard` is therefore
calculated from successful core/media-port acceptance plus an inventoried
downstream queue budget, configured jitter estimate, conservative playout
guard, and frame duration, and always carries reduced confidence. The public
feedback source reports only `Estimated` in v1. A later adapter feedback API
may add `TransportFeedback` or `ReceiverAcknowledged` without changing ledger
semantics.

### 6.9 Failure and failover semantics

Provider selection is ordered and bounded. No lane retries forever.
Each logical operation owns one selector cursor, tries a provider at most once,
and stops at `maximum_attempts`; a new caller turn resets the ASR/model/TTS
operation cursors.

**ASR**

- Retain 300 ms pre-roll and at most 15 seconds of the active turn.
- On retryable failure, start the next provider, increment provider epoch, and
  replay only the active turn buffer.
- Ignore late events from the failed epoch.
- If the turn exceeds replay capacity, fail the current turn explicitly rather
  than provide a partial audio history as complete.
- Exhausting ASR emits `AgentTurnFailed`, discards the failed active turn, and
  returns to listening by default. The session stops only when
  `AgentFailurePolicy.exhausted_turn` requests it.

**Model**

- Before accepted output or a non-idempotent tool call, retry on the next model
  with the committed context.
- After output begins, cancel the generation, correct context to the heard
  prefix, allocate a new `GenerationId`, and allow the fallback model to
  produce a recovery continuation.
- Tool calls require idempotency keys. A non-idempotent tool is never
  automatically replayed without application confirmation.

**TTS**

- Before any audio is sent, restart the complete stable text on the next TTS
  provider.
- After audio is sent, do not silently switch voices mid-utterance. Stop the
  generation, record the heard prefix, and let model/application policy choose
  a recovery utterance. Any recovery after heard audio receives a new
  `GenerationId`.

**Realtime speech**

- Hidden model state is not assumed portable.
- A fatal realtime failure terminates the v1 agent session. There is no
  automatic architecture transition or transparent Moshi failover in the first
  release.

**Media or caller failure**

- Caller/AI route terminal state cancels the complete agent session.
- No provider failover occurs after call termination.

### 6.10 Lifecycle and race guarantees

- Definition/provider validation precedes all side effects.
- Tenant and provider permits are acquired before model/session allocation.
- AI Participant and Connection are distinct from the caller's identity.
- If any start step fails, core rollback asks the harness to converge, then
  removes routes, Connection, Participant, registry entries, and the core-owned
  tenant permit in reverse order. The harness releases provider sessions and
  provider permits.
- A caller ending during start prevents a later `AgentStarted` event.
- `stop` and connection-terminal cleanup are idempotent and converge on the
  same retained completion outcome.
- One terminal `AgentStopped` or `AgentFailed` event is emitted.
- Reusing a Connection ID or provider epoch cannot let a stale handle remove a
  newer session.

### 6.11 Canonical conversation, tool, and call-signal state

The harness, not a model provider, owns the canonical bounded model-context
history. Core's lossless evidence/vCon path owns the complete call record. Each
model session receives a projection for one generation. On interruption, the
harness first replaces the assistant item with the estimated-heard UTF-8
prefix, records the correction event, and only then notifies the provider.
Failover receives this canonical corrected window; provider hidden state is
never authoritative or assumed portable. History eviction follows
`HistoryPolicy` only after the evicted item has crossed the evidence boundary.

Tool calls are generation-scoped state in the same store. Each `ToolCallId`
moves once through `Requested → Completed | TimedOut | Cancelled`. A duplicate
result returns the recorded terminal outcome, a late result is counted and
discarded, and a non-idempotent call cannot be replayed by failover without
explicit application confirmation.

Core automatically forwards target-originated DTMF, hold/resume, transfer, and
terminal lifecycle events over the high-priority signal queue. Terminal state
is duplicated on a watch channel so queue pressure cannot hide call end.
`AgentHandle::inject_dtmf` is explicitly application-synthesized input and is
distinguishable from caller-originated DTMF in events and model commands.

## 7. Audio frontend and DSP

### 7.1 Processing graph

The default processed input order is:

~~~text
media decode/resample
    → optional echo cancellation using render reference
    → optional noise suppression / background-speaker filtering
    → optional AGC
    → VAD + ASR + semantic turn detector
~~~

Raw recording taps attach before DSP. Processed recording/evaluation taps attach
after DSP. Both are explicit and independently permissioned.

### 7.2 Activity detection

- First production detector: Silero VAD through a Rust ONNX adapter.
- Input: mono 16 kHz frames buffered to the selected model's required window.
- Output: probability, speech start/continue/stop, model frame range, and
  processing latency.
- Reset model state on discontinuity, provider restart, or format change.
- Do not use the current advanced rvoip VAD as the release detector until it
  passes the same provider contract and corpus gates.

### 7.3 Semantic/acoustic turn detection

- First optional detector candidate: Pipecat Smart Turn, subject to an ONNX
  operator/latency feasibility spike and license verification.
- Kyutai STT semantic VAD maps into the same `TurnEvidence` event.
- Provider-native EOT and semantic models are independent evidence sources.
- Every detector exposes lookback/window requirements and confidence horizon.
- A detector failure falls back to deterministic activity/timer endpointing.

### 7.4 Noise suppression

- Implement `AudioProcessorProvider` before choosing a concrete model.
- First candidate: DeepFilterNet using its Rust/tract path at 48 kHz.
- Resample only once before/after the processor.
- Measure downstream ASR WER and clean-speech damage, not only perceptual
  denoising.
- Keep noise suppression opt-in until corpus results establish safe defaults.

### 7.5 Echo cancellation and AGC

- Define a duplex processor command that receives capture audio and the exact
  outward render-reference sample clock.
- Add delay-estimation and alignment instrumentation before enabling an AEC
  implementation.
- Prefer browser/device/WebRTC AEC when available; server AEC is a separate
  capability.
- Keep AGC off by default. A provider may request normalized input, but the
  chosen AGC must pass ASR and clipping evaluation.

## 8. Provider implementations

### 8.1 Deterministic reference providers

The `rvoip-harness/testkit` feature ships scriptable providers for activity,
turn, ASR, model, TTS, realtime speech, and DSP. They support exact delays,
revisions, errors, hangs, cancellation races, malformed output, and usage
events. The feature is enabled by tests/examples only and is never part of the
production `voice-ai` dependency closure.

These are required before runtime work so state-machine and media tests do not
depend on a network or model.

### 8.2 Local cascaded stack

The first documented no-Python local stack is:

- Activity: Silero VAD via Rust ONNX Runtime.
- ASR: sherpa-onnx streaming recognizer through its Rust API.
- Model: `mistral.rs` in-process provider with streaming text and tool events.
- TTS: a sherpa-onnx Rust TTS engine as the release baseline.
- Additional local/service ASR: Kyutai's production Rust STT service after it
  passes the same contract and concurrency gates.
- Experimental TTS: Laurent Mazaré's `xn-ptts`, exposed behind an explicit
  experimental feature until API stability, concurrency, and real-time tests
  pass.

Native C++/CUDA libraries behind Rust APIs are allowed. No component may spawn
Python.

### 8.3 Remote cascaded stack

`rvoip-ai-remote` initially supplies:

- A streaming Kyutai STT WebSocket client, including transcript revisions,
  word timing, provider endpointing, and semantic turn evidence when present.
- A streaming TTS WebSocket client matching the common incremental text/audio
  contract. Kyutai TTS is a valid remote service reference even though its
  current server internals are not eligible as rvoip's local no-Python path.
- An OpenAI-compatible streaming model client supporting text deltas, tool
  calls, cancellation, usage, timeouts, and a configurable base URL.
- Protocol-faithful local mock servers for CI.

HTTP/WebSocket clients use connection pooling, TLS by default, bounded message
sizes, redacted errors, deadlines, and cancellation-safe shutdown.

### 8.4 Moshi

`rvoip-ai-moshi` implements:

1. **In-process provider**
   - Official Moshi Rust/Candle backend.
   - CUDA and Metal feature sets; CPU may compile but is not claimed real-time
     until benchmarked.
   - Mimi encode/decode and Moshi model framing stay inside the crate.
   - Model weights are loaded/warmed once and shared through an admission pool.

2. **Remote provider**
   - Moshi WebSocket protocol client.
   - Opus 24 kHz protocol audio is decoded/resampled at the adapter boundary.
   - Text, metadata, error, and control messages map to typed realtime events.

3. **Tool/retrieval extension**
   - Use typed context-injection and retrieval events modeled on MoshiRAG.
   - Retrieval runs asynchronously; it cannot pause audio ingress.
   - Timeouts produce an explicit retrieval result and do not deadlock model
     generation.

The pinned Moshi revision, Candle revision, model repository, model hashes,
Mimi hashes, supported GPU runtimes, and CC-BY weight attribution are part of
release evidence.

### 8.5 Provider features

Replace the old `harness` feature name. The exact edges are:

~~~toml
# rvoip-core
recording = []
voice-ai = ["dep:rvoip-harness"]

# rvoip facade
recording = ["rvoip-core/recording"]
voice-ai = ["rvoip-core/voice-ai", "dep:rvoip-harness"]
voice-ai-remote = ["voice-ai", "dep:rvoip-ai-remote"]
voice-ai-local-onnx = [
    "voice-ai",
    "dep:rvoip-ai-local",
    "rvoip-ai-local/onnx",
]
voice-ai-local-dsp = [
    "voice-ai",
    "dep:rvoip-ai-local",
    "rvoip-ai-local/dsp",
]
voice-ai-local-cascade = [
    "voice-ai-local-onnx",
    "dep:rvoip-ai-local",
    "rvoip-ai-local/cascade",
]
voice-ai-xn-experimental = [
    "voice-ai",
    "dep:rvoip-ai-local",
    "rvoip-ai-local/xn-experimental",
]
voice-ai-moshi = [
    "voice-ai",
    "dep:rvoip-ai-moshi",
    "rvoip-ai-moshi/protocol",
]
voice-ai-moshi-cuda = ["voice-ai-moshi", "rvoip-ai-moshi/cuda"]
voice-ai-moshi-metal = ["voice-ai-moshi", "rvoip-ai-moshi/metal"]
voice-ai-testkit = ["voice-ai", "rvoip-harness/testkit"]
~~~

`rvoip-harness` has no model dependency in its default feature set.
`rvoip-harness/testkit` and an optional facade `voice-ai-testkit` are
dev-support only. `voip-3` replaces its direct `dep:rvoip-harness` member with
`recording` and `voice-ai`, so the high-level APIs remain available, but it
does not select a remote, local, Moshi, CUDA, or Metal provider. Existing
default features remain unchanged. Applications explicitly enable a provider
feature, construct typed providers, and register them.

Do not include mutually exclusive CUDA and Metal features in a single blanket
`--all-features` CI invocation. Define supported feature matrices explicitly.

### 8.6 Qualification profiles

WP0 creates `docs/voice-ai/qualification-profiles.toml`; release tooling
validates it and embeds its normalized form in the attestation. Each profile
contains:

- stable profile ID and maturity: `experimental`, `preview`, or `qualified`;
- whether it is deployment-supported and release-blocking;
- exact crate features, Rust target, providers, models, code revisions,
  artifact URLs, and SHA-256 hashes;
- OS/CPU/GPU, memory, driver/runtime, and native-library minimums;
- `qualified_languages` as a finite subset of advertised upstream languages;
- corpus IDs/hashes and absolute plus regression thresholds;
- required codec/transport scenarios and concurrency/load levels;
- network policy, artifact setup command, test command, and evidence paths.

The manifest rejects `latest`, version ranges, mutable artifact URLs without a
hash, an empty qualified-language set, or a release-blocking profile whose
hardware runner is unavailable. Runtime capability reporting may expose
broader upstream language support, but production documentation and release
claims use only `qualified_languages`.

The first-release profile set is:

| Profile ID | Maturity | Deployment | Release block | Purpose |
|---|---|---:|---:|---|
| `cascade-deterministic-ci` | qualified | no | yes | Test-only provider-neutral lifecycle, media, cancellation, tools, and all transport correctness |
| `cascade-local-cpu-linux-x86_64` | qualified | yes | yes | No-Python Silero + sherpa-onnx ASR/TTS + mistral.rs cascade |
| `cascade-mixed-linux-x86_64` | qualified | yes | yes | Local VAD/ASR + live remote model + local TTS |
| `cascade-remote-live` | qualified | yes | yes | Live remote ASR/model/TTS protocol and service compatibility |
| `moshi-remote-official` | qualified | yes | yes | Wire-protocol interoperability with the pinned official Rust server |
| `moshi-candle-cuda-linux-x86_64` | qualified | yes | yes | Required in-process official Rust/Candle full-duplex backend |
| `moshi-candle-metal-macos-arm64` | preview | no | no | Compile/contracts and nightly real-hardware smoke |
| `xn-pocket-tts` | experimental | no | no | Evaluation only |
| `smart-turn` | experimental | no | no | Evaluation only |

Metal becomes release-blocking only after its manifest entry is promoted to
`qualified` with the same real-hardware evidence. This keeps the user's
Moshi-class first-release requirement while avoiding an unapproved requirement
to ship two accelerator platforms simultaneously.

Required coverage is explicit:

| Scenario | PCMU | PCMA | Opus | SIP | WebRTC | UCTP |
|---|---:|---:|---:|---:|---:|---:|
| Deterministic cascade | yes | yes | yes | yes | yes | yes |
| Qualified local cascade | yes | yes | yes | yes | yes | yes |
| Mixed/live remote cascade | yes | smoke | yes | yes | yes | contract |
| In-process Moshi CUDA | yes | codec contract | yes | yes | yes | deterministic bridge |
| Remote Moshi protocol | protocol PCM/Opus mapping | protocol PCM/Opus mapping | yes | adapter contract | adapter contract | yes |
| Existing Vapi regression | existing negotiated matrix | existing negotiated matrix | existing negotiated matrix | external topology | external topology | not applicable |

“Contract” means deterministic end-to-end transport coverage plus live
provider-contract coverage; “smoke” means one real encoded call. The
attestation records the exact test case satisfying every cell.

## 9. rvoip-core integration

### 9.1 InProcessAi media stream

`InProcessAiAdapter` implements the existing `ConnectionAdapter` contract and
publishes one internal audio `MediaStream`.

When `rvoip-core/voice-ai` is enabled, core creates and reserves exactly one
internal adapter during Orchestrator construction. Add
`AdapterKind::Internal` for this non-network adapter. Applications neither
register nor replace it, and adapter registration rejects a second owner for
`Transport::InProcessAi`.

The channel directions are:

~~~text
caller MediaStream frames_in
    → caller MediaGraph
    → AI MediaStream frames_out sender
    → AgentMediaPort capture receiver

playback scheduler + AgentOutputFrame generation envelope
    → AI MediaStream private source sender
    → AI MediaStream frames_in receiver
    → AI MediaGraph
    → caller MediaStream frames_out
~~~

Implementation requirements:

- The AI stream codec is internal `pcm_s16le` at the configured bus rate.
- Internal PCM payload types are never advertised in SDP or sent on a network.
- The stream exposes each single-consumer receiver exactly once and supports
  rollback-capable reservation like other built-in streams.
- The AI-output source channel has capacity one frame. The paced scheduler,
  rather than MediaGraph, owns unsent buffering.
- Caller-to-AI routes may use the standard bounded media sink. Overflow becomes
  a typed `PcmFrame` discontinuity and cannot be silent. Add a per-route drop
  notification/sample counter to MediaGraph; the AI port also detects
  sample-position gaps so either path forces provider reset.
- The core output sink checks the generation envelope at acceptance and again
  immediately before graph injection. Render-reference audio is forked only
  after successful graph injection and `MediaAcceptance`.
- Source and sink timestamp translators preserve monotonic sample position
  through 8/16/24/48 kHz conversions and RTP wrap.
- Closing the AI Connection closes both media directions and wakes the harness.

### 9.2 Parameterized PCM codec

Generalize `PcmS16LeCodec`:

- Support mono 8, 16, 24, and 48 kHz.
- Keep it headerless, signed 16-bit little-endian, internal-only PCM.
- Validate sample count and byte alignment but do not require one fixed frame
  duration at the codec layer.
- Report the configured rate through `CodecInfo`.
- Teach codec factory, payload mapping, MediaGraph validation, and transcoder
  grouping to preserve the configured clock rate.
- Test PCM ↔ PCMU, PCM ↔ PCMA, and PCM ↔ Opus in both directions at all
  relevant rates.

Do not use one fixed global payload-type mapping to infer the PCM rate. The
`CodecInfo` accompanying an internal route is authoritative.

### 9.3 Orchestrator API

Remove string-based AI registration and attachment methods. Add:

~~~rust
pub fn register_activity_detector(
    &self,
    provider: Arc<dyn ActivityDetectorProvider>,
) -> Result<()>;

pub fn register_turn_detector(
    &self,
    provider: Arc<dyn TurnDetectorProvider>,
) -> Result<()>;

pub fn register_interruption_detector(
    &self,
    provider: Arc<dyn InterruptionDetectorProvider>,
) -> Result<()>;

pub fn register_asr_provider(
    &self,
    provider: Arc<dyn SpeechRecognizerProvider>,
) -> Result<()>;

pub fn register_model_provider(
    &self,
    provider: Arc<dyn AgentModelProvider>,
) -> Result<()>;

pub fn register_tts_provider(
    &self,
    provider: Arc<dyn TextToSpeechProvider>,
) -> Result<()>;

pub fn register_realtime_speech_provider(
    &self,
    provider: Arc<dyn RealtimeSpeechProvider>,
) -> Result<()>;

pub fn register_audio_processor(
    &self,
    provider: Arc<dyn AudioProcessorProvider>,
) -> Result<()>;

pub async fn start_agent(
    self: &Arc<Self>,
    target: ConnectionId,
    definition: AgentDefinition,
) -> Result<AgentHandle>;

pub fn agent(&self, id: &AgentSessionId) -> Option<AgentHandle>;

pub async fn stop_agent(
    &self,
    id: AgentSessionId,
    reason: AgentStopReason,
) -> Result<AgentStopReport>;

pub async fn drain_agents(
    &self,
    deadline: Duration,
) -> Result<AgentDrainReport>;
~~~

These are core `Result` aliases and map harness/provider validation failures
through `RvoipError::VoiceAi`. Registration delegates to core's shared
`Arc<ProviderRegistry>`; providers are never automatically discovered.

`start_agent` v1 accepts one connected target Connection that is not already
owned by an exclusive bridge. Listener/recording taps are allowed. To replace a
human or external agent, the application first performs the existing unbridge/
handoff operation, then starts the in-process agent. Agent-assist and arbitrary
multiparty subscription are later topologies.

### 9.4 Transactional start

`start_agent` performs these steps:

1. Resolve the target, capture its lifecycle generation, and perform pure
   ownership/Session/Conversation/bridge/tenant validation.
2. Validate `AgentDefinition`, resolve provider capabilities/formats, and
   reserve a pending agent-lifecycle ticket that fences concurrent starts.
3. Acquire the core-owned tenant AI permit.
4. Call `AgentRuntime::prepare`; acquire provider/model permits and warm slots
   without exposing call media.
5. Revalidate the target and pending lifecycle ticket.
6. Create/join the distinct AI Participant, create the internal Connection and
   media port, but do not bridge caller media yet.
7. Start provider sessions and supervisor, take/start the core evidence
   receiver, and await the runtime readiness barrier.
8. Activate both bridge directions and await route-active acknowledgement.
9. Revalidate target, route, runtime health, and lifecycle ticket.
10. Atomically install the live core entry, commit the pending reservation, and
    emit `AgentStarted`.

Any failure rolls completed steps back in reverse order. No `AgentStarted`
event, orphan Participant, partial Connection, route, task, or retained permit
may survive rollback.

Runtime completion racing steps 7–9 invalidates the pending lifecycle ticket,
so a dead runtime can never be committed live. Core first asks a started
harness to converge, then removes routes/Connection/Participant and releases
the tenant permit; harness releases provider sessions and permits.

### 9.5 Commands

Replace:

- `Command::AttachAi` with typed `Command::StartAgent`.
- AI use of generic `Command::Detach` with `Command::StopAgent`.
- String provider reference and `HashMap<String, String>` with
  `AgentDefinition`.

Add:

- `Command::StartAgent { request_id, target, definition }`.
- `Command::StopAgent { request_id, agent_id, reason }`.
- `Command::SendAgentInput { request_id, agent_id, input }` for application-
  injected DTMF, manual commit, context injection, and tool results.
- `Command::CancelAgentGeneration { request_id, agent_id, generation_id }`.
- `Command::DrainAgents { request_id, deadline }`.

Commands carry correlation IDs but redact definitions, instructions, text, and
tool payloads from `Debug`.

`Orchestrator::drain_agents(deadline)` stops accepting new starts, requests
orderly stop for every live and pending runtime, awaits shared completion,
forces only those that exceed the deadline, performs core cleanup, and returns
a report. The current Orchestrator has no general shutdown API, so v1 makes
calling `drain_agents` an explicit application obligation before dropping the
last owner; examples and server wrappers do so. A future general
`Orchestrator::shutdown` must invoke it. Drop performs best-effort cancellation
but cannot satisfy the bounded teardown/attestation contract.

### 9.6 Core and per-agent events

The global core event bus carries bounded, low-frequency summaries:

- `AgentStarted { agent_id, target, participant_id, ai_connection_id,
  backend_kind }`
- `AgentStopped { agent_id, reason, forced, elapsed }`
- `AgentFailed { agent_id, error_category, stage }`
- `AgentTurnCommitted { agent_id, turn_id, reason, endpoint_latency }`
- `AgentInterruptionConfirmed { agent_id, turn_id, generation_id,
  local_stop_latency }`
- `AgentProviderFailover { agent_id, kind, from_key, to_key, reason_category }`
- `AgentAudioDiscontinuity { agent_id, direction, reason, missing_samples }`

The per-agent subscription additionally carries high-frequency detail:

- input/output state transitions;
- speech and endpoint evidence;
- transcript revisions and finals;
- generation lifecycle;
- TTS alignment and playback progress;
- interruption candidate/rejected/confirmed;
- tool and retrieval lifecycle;
- provider health, usage, latency, and errors;
- queue pressure and stale-generation rejection.

Replace `AiAttached`, `AiDetached`, and `BargeInDetected`. Standard Participant,
Connection, bridge, and Session events still describe the AI resources.

Cross-crate normalized events omit transcript/prompt/tool content and expose
only redacted lifecycle, provider category, reason category, and aggregate
timing.

Core derives these summaries and all vCon analyses from the dedicated
lossless `AgentEvidence` stream, never the lagging observer broadcast. Each
core event uses the normal core event envelope for event ID/time plus the
listed voice-agent correlation. At termination, core sequence-merges
`AgentRuntimeOutcome.unacknowledged_evidence` with records already projected,
then compares the contiguous sequence and rolling hash with
`AgentRuntimeOutcome.final_evidence` before emitting the one terminal
projection. A duplicate replay is harmless; a gap or hash mismatch is a
terminal integrity error and blocks a falsely complete vCon marker.

### 9.7 Transcription migration

`start_transcription` migrates to `SpeechRecognizerProvider` and the typed PCM
tap:

- Add `TranscriptionDefinition` with provider selector, language, format, and
  partial/final event policy.
- Request a PCM sink codec from MediaGraph.
- Use the same ASR event/revision types as the agent runtime.
- Preserve transcription lifecycle and terminal cleanup.
- For a Session target, require an explicit source Connection in v2 rather than
  silently choosing the first Connection.

The replacement API is explicit:

~~~rust
pub async fn start_transcription(
    self: &Arc<Self>,
    source: ConnectionId,
    definition: TranscriptionDefinition,
) -> Result<TranscriptionHandle>;

pub async fn stop_transcription(
    &self,
    id: TranscriptionSessionId,
) -> Result<TranscriptionStopReport>;
~~~

`TranscriptionHandle` exposes ID/state, a lag-reporting typed revision/final
event receiver, `stop`, and a retained terminal-outcome watch. It has no agent
model, TTS, interruption, or playback surface.

### 9.8 Recording isolation

Move recording APIs out of the deleted harness module. Change registered
recording sinks from singleton mutable sink objects to factories that create
one `RecordingSinkSession` per recording. This prevents concurrent recordings
from sharing buffers or close state.

The module move and factory-based registration are intentional breaking API
changes in this release. Recording start/stop behavior, artifact contents, and
`RecordingArtifact` serialization remain semantically stable.

### 9.9 Quotas and admission

- Rename `max_concurrent_ai_sessions` internals and diagnostics to
  `max_concurrent_agent_sessions`.
- Tenant permit spans start through final cleanup.
- Local provider descriptors add their own session/model permits.
- Provider/model admission failure occurs before creating AI call resources.
- Warm pools may retain loaded model memory after a session, but they may not
  retain session state, audio, transcript, or tenant context.

### 9.10 External-agent regression

`rvoip-vapi` remains a `Transport::Vapi` Connection and normal bridge. Add
regression coverage proving the new in-process runtime does not change Vapi
framing, pacing, lifecycle, or registration. Future external full-stack agents
follow the Vapi topology rather than implementing `RealtimeSpeechProvider`
unless rvoip directly owns their PCM/model session.

## 10. Observability and evidence

### 10.1 Agent events and spans

Create one root span per `AgentSessionId`, child spans per `TurnId` and
`GenerationId`, and provider child spans per provider epoch.

Required timestamps:

- first input PCM;
- speech candidate/start/stop;
- endpoint evidence and commit;
- ASR first partial and final;
- model request and first token;
- TTS first accepted text and first audio;
- playback queued/start/last core-accepted;
- interruption candidate/confirmation/final output frame;
- tool request/result;
- provider failure/failover;
- stop requested/completed.

Transcripts, prompts, tool arguments/results, credentials, audio bytes, model
payloads, and endpoint query parameters are excluded from traces by default.

### 10.2 Metrics

Use bounded labels such as backend kind, an interned telemetry key assigned
from the at-most-256-entry registry, locality, codec, rate, qualified/other
language class, outcome, and error category. Never use raw user/provider text,
session/participant/turn IDs, model names, voices, or exact languages as metric
labels. Each metric family documents a worst-case series calculation and has a
hard cap of 4,096 series.

Required metric families:

- active agent/provider sessions and available permits;
- lane queue depth/capacity;
- input/output frames, discontinuities, drops, underruns, and stale rejections;
- turns started/committed/cancelled;
- interruptions candidate/confirmed/rejected/resumed;
- provider starts, failures, timeouts, failovers, and forced closes;
- ASR/model/TTS/realtime usage;
- VAD onset, endpoint, model TTFT, TTS TTFB, first audible sample, total turn,
  interruption-to-local-stop, and interruption-to-receiver-silence latency.

The observability contract suite asserts exact metric deltas, event ordering,
span parent/correlation IDs, redaction canaries across every sink, lag
reporting, and the worst-case series cap. Registering the 257th provider or
creating a label combination outside the declared enum fails validation rather
than creating a new series.

### 10.3 vCon projection

When `AgentObservabilityConfig` and tenant consent allow the applicable content,
append structured analyses for:

- caller transcript revisions and final;
- assistant generated text and estimated-heard prefix;
- provider/model identity and version;
- tool/retrieval request and result summary;
- interruption with detection, confirmation, and heard boundary;
- per-stage latency and usage;
- backend/provider failure and failover;
- recording references when enabled.

vCon stores no credential, raw provider response, or unapproved audio content.
The terminal projection is idempotent so a stop race cannot append duplicate
analyses.

### 10.4 Playback ledger confidence

Expose `PlaybackPosition` with:

- generation;
- synthesized/queued/core-accepted/estimated-heard samples;
- aligned text byte and word boundary;
- estimate confidence;
- source of feedback (`Estimated` in v1; later
  `TransportFeedback`/`ReceiverAcknowledged` variants are reserved).

Applications and model-history correction can distinguish precise alignment
from conservative estimates.

## 11. Security, privacy, and supply chain

### 11.1 Secrets and network policy

- Provider credentials are secret wrapper types with redacted `Debug`.
- Remote providers require TLS by default and bound DNS/connect/handshake/
  request/idle/close deadlines.
- Bound all WebSocket/HTTP message sizes and reject decompression expansion
  beyond configured limits.
- Do not follow arbitrary redirects for audio/model endpoints.
- Support application allowlists for remote base URLs in multi-tenant
  deployments.
- Never include bearer tokens or signed URLs in errors, metrics, events, or
  vCon.

### 11.2 Audio and transcript privacy

- Raw and processed recording are separately opt-in.
- Debug, error, and normal trace paths never render audio or transcript text.
- Detailed transcript events remain on the tenant-authorized per-agent surface;
  the global bus receives redacted lifecycle only.
- Provider buffers, `Bytes` clones, and active-turn replay are promptly dropped
  on terminal cleanup and are never intentionally persisted. Generic shared
  media buffers do not promise cryptographic zeroization; any future
  zeroization claim requires uniquely owned zeroizing storage and a dedicated
  test.
- Document provider retention/data-use expectations in configuration guides.

The enforced data-class matrix is:

| Data class | Authorized per-agent events | Consented vCon | Opt-in recording artifact | Global events/logs/metrics/traces/errors/attestation |
|---|---:|---:|---:|---:|
| Credentials, tokens, signed URLs | never | never | never | never |
| Transcript, prompt, tool arguments/results | policy-controlled | policy-controlled | metadata only unless separately consented | never |
| Raw/processed audio | never | reference only unless separately consented | allowed for the explicitly enabled stream | never |
| Correlation IDs and bounded reason/timing fields | allowed | allowed | allowed | allowed |

Contract tests use distinct canaries for every data class and surface. “Absent”
means absent from disallowed surfaces, not absent from the authorized
per-agent or consented evidence channel.

### 11.3 Models and licenses

- Do not bundle model weights in Rust crates.
- Model construction receives an explicit local path or configured cache.
- Downloads, when offered by a separate setup command, verify hashes before
  atomic installation and never occur implicitly during call attachment.
- Record code license, weight license, source URL, revision, checksum, and
  required attribution separately.
- Moshi Rust code and CC-BY weights require separate notices.
- Smart Turn, Silero, DeepFilterNet, sherpa-onnx, mistral.rs, XN, and each model
  weight receive an explicit license review before feature release.
- Generate an SBOM for each supported provider feature set.

### 11.4 Voice cloning

Voice cloning/provider prompt voices remain disabled in the standard API until
a separate design defines consent evidence, allowed voices, tenant policy,
abuse reporting, and retention. The initial TTS contract supports registered
voice IDs, not arbitrary caller-supplied voice samples.

## 12. Implementation work packages

Each work package must leave the workspace in a testable state. Temporary
parallel old/new modules may exist inside an implementation branch to keep
commits buildable, but the release contains only the new API and runtime.

### WP0 — Baseline, dependency, and artifact audit

**Purpose:** Establish reproducible input before changing public contracts.

Tasks:

1. Record the current voice-AI tests, examples, facade features, publish order,
   command/event variants, quotas, and docs that reference the old API.
2. Create a symbol-removal checklist using `rg` for every type and method named
   in §16.
3. Verify candidate dependency versions against Rust 1.88:
   - ONNX Runtime Rust crate;
   - sherpa-onnx Rust API/native artifacts;
   - mistral.rs;
   - DeepFilterNet/tract;
   - Moshi 0.6.x/Candle 0.9.x;
   - Laurent's XN and XN Pocket TTS.
4. Record source revision, code license, weight license, supported hardware,
   required native toolchain, and publishability for each candidate.
5. Decide the exact pinned Moshi crate/revision:
   - prefer the published Apache/MIT `moshi` crate at an exact compatible
     version;
   - if required runtime APIs are not published, vendor the minimal official
     Rust crates with notices rather than use a floating Git dependency in a
     published rvoip crate.
6. Add the small redistributable test-corpus manifest and artifact directory
   structure without adding model weights.
7. Add and schema-validate the qualification manifest in §8.6, including exact
   provider/model revisions, artifact hashes, hardware floors, qualified
   languages, transport cells, and release-blocking flags.
8. Confirm a maintained CUDA runner can load the pinned official Moshi/Candle
   stack at real time. Treat failure as a WP0 stop/go issue requiring a revised
   qualified in-process profile before API implementation proceeds.
9. Record Metal as preview unless the same evidence and runner availability
   justify promotion to qualified.
10. Capture baseline workspace build/test results and current AI demo behavior.

Exit criteria:

- Dependency/license table reviewed.
- No candidate silently requires Python for the local production path.
- Every old symbol/removal site is enumerated.
- Corpus files have hashes and redistribution provenance.
- The qualification manifest validates with no floating or unhashed artifact.
- Every release-blocking profile has an available runner and named owner.

### WP1 — Voice AI types and provider contracts

**Primary area:** `rvoip-core-traits`

Tasks:

1. Add validated ID types and `voice_ai` module structure.
2. Implement private-field `AudioFormat` and `PcmFrame` constructors/getters.
3. Add discontinuity, transcript, word timing, alignment, usage, latency, tool,
   call-context, and playback-position types.
4. Add typed definition/policy structs and complete range validation.
5. Add bounded provider descriptor, capability, health/lifecycle probe,
   preparation lease, locality, and error types.
6. Add channel-session structs, out-of-band control, and normative activity,
   turn, interruption, DSP, ASR, model, TTS, and realtime provider traits.
7. Add `AgentEvent`, `AgentInput`, state snapshots, runtime outcome/stop reason,
   evidence, and media-port contracts; core adds its cleanup/stop report in
   WP6.
8. Move recording contracts into `recording.rs`.
9. Add redaction-canary tests for every public `Debug` and `Display`.
10. Add rustdoc state/cancellation/ordering guarantees to every trait.
11. Add close/flush/cancel race tests and compile-time coverage for every
    `ProviderOpenContext` negotiation field.

Exit criteria:

- Contract crate compiles without core, media-core, HTTP, model, or GPU deps.
- Invalid audio/configuration cannot be constructed through safe APIs.
- Compile-time mock implementations prove every provider trait is object-safe
  and `Send + Sync`.
- Recording contracts compile from their new module.
- Descriptor/definition boundary and aggregate limits reject every over-limit
  value before allocation/start.

### WP2 — Internal PCM and InProcessAi media

**Primary areas:** media-core and rvoip-core

Tasks:

1. Generalize `PcmS16LeCodec` and internal codec construction.
2. Extend resampler/transcoder tests for 8/16/24/48 kHz.
3. Implement `InProcessAiMediaStream` with rollback-capable single receiver
   ownership and the channel directions in §9.1.
4. Implement `InProcessAiAdapter` Connection creation, activation, stream
   discovery, close, and liveness.
5. Implement `AgentMediaPort`, `AgentOutputFrame`, synchronous generation
   advance/flush, and the one-frame unpaced core output source.
6. Create a distinct AI Participant and Connection test fixture.
7. Bridge SIP-like PCMU and WebRTC-like Opus test streams to the AI port in both
   directions using real encoded payloads.
8. Add per-route drop notification and sample-gap detection.
9. Inventory and bound every output queue through codec/packetizer/transport;
   expose the configured budget to the heard ledger.
10. Cover route removal, receiver double-take, timestamp wrap, rate conversion,
    queue overflow/discontinuity, stale output at the core fence, and
    simultaneous close.

Exit criteria:

- Caller encoded audio arrives as valid configured PCM.
- AI PCM returns as valid negotiated encoded audio.
- No provider-facing type contains an encoded transport payload.
- The media port can read input while output is sent continuously.
- Closing either connection terminates both routes without task/receiver leaks.

### WP3 — Registry, validation, and deterministic testkit

**Primary area:** rvoip-harness

Tasks:

1. Implement `ProviderRegistry` with typed registration, duplicate rejection,
   descriptor validation, bounded async health probing, ordered route cursors,
   preparation leases, and provider permits.
2. Implement `AgentDefinition::validate_against`.
3. Add scriptable deterministic providers for every provider kind.
4. Allow scripts to emit exact-timestamp events, stall, fail, reorder protocol
   messages, ignore cancellation, and close unexpectedly.
5. Implement a virtual monotonic audio clock, scripted caller/agent endpoints,
   media probe, metrics/event recorder, impairment layer, and leak tracker.
6. Add provider common-contract test macros/helpers.
7. Add tests for concurrent isolated sessions on singleton provider factories.
8. Keep all deterministic providers/helpers behind `testkit` and assert they
   are absent from the normal production dependency tree.

Exit criteria:

- No runtime test needs arbitrary sleeps or external services.
- Provider selection and admission are deterministic.
- Every failure mode can be reproduced from a seed/script.
- Registry diagnostics are redacted and bounded.

### WP4 — Session supervisor and cancellation core

**Primary area:** rvoip-harness

Tasks:

1. Implement orthogonal session/input/output state storage and transitions.
2. Implement the owned task tree and root cancellation token.
3. Add typed internal event router with control events separated from media
   queues.
4. Implement bounded queue wrappers with duration accounting, occupancy
   metrics, and explicit overflow outcomes.
5. Implement provider epoch filtering.
6. Implement `GenerationId` allocation and synchronous stale-generation fences
   at model, TTS, playback, tool, and event boundaries.
7. Implement graceful close deadline, forced abort, child join collection, and
   one terminal outcome.
8. Implement `AgentRuntimeHandle`, state watch, event subscription, and
   idempotent stop.
9. Implement `validate → prepare → start`, warm-reservation rollback, readiness
   barrier, shared completion, and deadline-only forced abort.
10. Implement the serialized 508+4 ACK journal, bounded evidence batches,
    contiguous acknowledgements, outcome replay tail, lagging observer
    broadcast, retained terminal watch, and evidence snapshot/digest.

Exit criteria:

- Virtual-time tests cover every state transition and stop source.
- A cancelled generation cannot cross any downstream boundary even if a
  provider ignores cancellation.
- Child failure cannot orphan siblings.
- Shutdown releases all channels/tasks/permits and emits one terminal outcome.
- Slow observer subscribers cannot affect evidence; slow/missing core evidence
  consumption closes normal journal admission, emits the reserved fatal tail,
  and remains reconstructible from the retained outcome.

### WP5 — Frontend, turn, interruption, and playback

Tasks:

1. Implement provider-neutral audio frontend fan-out and render-reference lane.
2. Implement active-turn/pre-roll buffer and discontinuity handling.
3. Implement deterministic activity/timer turn detector using Balanced defaults.
4. Implement provider/semantic `TurnEvidence` fusion and maximum timers.
5. Implement monitoring/candidate/confirmed/rejected interruption transaction
   with raw-onset timing, backchannel exclusion, and bounded uncertain state.
6. Implement configurable lexical backchannel policy.
7. Implement playback queue, prebuffer, cadence scheduler, underrun behavior,
   flush, alignment map, and heard ledger.
8. Implement history correction from aligned or conservative estimated-heard
   boundaries.
9. Implement DTMF/manual commit paths.
10. Add local Silero provider only after the deterministic policy passes.
11. Prove provider-native realtime overlap does not enter the cascaded pause
    state; separately test explicit `HarnessManaged` opt-in.

Exit criteria:

- Input continues during buffering and playback.
- Candidate interruption pauses at a frame boundary.
- Confirmed interruption clears unsent output and fences late output.
- False interruption resumes without replaying heard frames.
- Turn completion always has a deterministic fallback and cannot hang.
- Playback never bursts after a missed tick.
- Requested playback buffering/underrun behavior cannot exceed hard limits.

### WP6 — Orchestrator integration and transactional lifecycle

Tasks:

1. Add typed provider-registration methods and `start_agent`/`stop_agent`.
2. Implement transactional start and rollback in §9.4.
3. Bind harness runtime state to tenant, target, AI participant/connection,
   media route, and lifecycle generation.
4. Add live agent registry and `AgentHandle` lookup.
5. Implement common terminal cleanup for explicit stop, caller end, AI route
   end, Session end, provider fatal error, and orchestrator shutdown.
6. Automatically reserve the sole internal adapter under `voice-ai`.
7. Add bounded `drain_agents` and exercise pending/live runtime shutdown.
8. Add new command/event variants without exposing an incomplete public facade.
9. Migrate transcription to typed PCM/new ASR sessions.
10. Move recording types and use per-recording sink factories.
11. Rename/update agent quota internals.
12. Drain and contiguously acknowledge runtime evidence before bridge
    activation; project global events/vCon from it and sequence-merge the
    outcome replay tail before reconciling the terminal snapshot.
13. Forward target DTMF/hold/resume/transfer/terminal signals over the
    high-priority runtime channel and implement application `inject_dtmf`.

Exit criteria:

- Start rollback and every stop race leave no partial AI lifecycle state.
- A runtime cannot receive bridged media before its readiness barrier.
- Orchestrator drain leaves no pending/live runtime or core-owned permit.
- Every server example explicitly calls `drain_agents`; drop-only cleanup is
  never used as release evidence.
- Existing non-AI recording, listener, bridge, and Vapi tests remain green.

### WP7 — Cascaded runtime

Tasks:

1. Implement exact-once ASR pre-roll, command/event lanes, revision store,
   commit-candidate `CommitTurn`/`Flush`, deadline, and irreversible commit.
2. Implement model command/event lanes, context, DTMF, streaming text, tool
   calls/results, handoff, and stop.
3. Implement Unicode-safe stable-prefix text chunker.
4. Implement TTS incremental input/audio/alignment lanes.
5. Connect provider usage and latency into agent events.
6. Implement exact ASR/model/TTS failover semantics in §6.9.
7. Add tool-call idempotency and cancellation bookkeeping.
8. Add per-operation provider cursors, exhausted-turn policy, and
   new-generation recovery after heard output.
9. Add deterministic end-to-end cascaded test through real PCMU and Opus media.
10. Replace `examples/11-ai-harness-demo` with this deterministic concurrent
   topology.
11. Switch the facade/core public exports and commands to the new API.
12. Remove the old traits, no-op providers, registries, serial loop, IDs,
    commands, events, examples, feature exports, and tests.
13. Update PRD and interface examples so the old API no longer appears as
    current guidance.

Exit criteria:

- A real encoded caller utterance produces decodable, paced response audio.
- Caller audio is observed by activity/ASR throughout response playback.
- Backchannel, real interruption, DTMF, tool, failover, and disconnect scenarios
  pass.
- Heard-prefix correction is reflected in subsequent model context.
- `rg` finds no old public symbol outside historical release notes, and no
  old/new runtime dual path remains.

### WP8 — Remote provider vertical

Tasks:

1. Implement shared bounded TLS HTTP/WebSocket transport utilities.
2. Implement protocol-faithful mock servers and malformed/slow/failing modes.
3. Implement Kyutai streaming STT mapping, including semantic evidence.
4. Implement streaming TTS service mapping.
5. Implement OpenAI-compatible model streaming/tool/usage mapping.
6. Add pooling, timeouts, close handshake, cancellation, circuit breaker,
   retry-after, health, and failover integration.
7. Run common provider contracts against mocks on every PR.
8. Run live provider qualification in secret-bearing release CI.
9. Run the required mixed profile with local VAD/ASR, live remote model, and
   local TTS.

Exit criteria:

- Remote providers pass their common contract suites.
- A mixed remote cascaded call passes the full vertical.
- No network task, connection, or credentials survive teardown/logging.

### WP9 — Local no-Python cascaded vertical

Tasks:

1. Implement Silero ONNX VAD with pinned model/checksum.
2. Implement sherpa-onnx streaming ASR and one TTS engine through Rust APIs.
3. Implement mistral.rs streaming model/tool provider.
4. Add explicit local model loading, warm pool, hardware descriptor, and
   concurrency admission.
5. Add no-implicit-download and checksum validation.
6. Build normally, then run the release artifact in a minimal container with no
   Python executable, no model network access, read-only verified artifacts,
   and child-process auditing that proves no Python sidecar is spawned.
7. Run provider contracts, small corpus, latency, memory, and repeated-session
   tests.
8. Add XN Pocket TTS behind `voice-ai-xn-experimental`; do not make it the
   release baseline until it independently passes the same gates.
9. Document a complete local configuration with artifact acquisition and
   license notices.

Exit criteria:

- The all-local cascade runs without Python installed.
- Local providers meet declared real-time/concurrency behavior.
- Model pool memory and session state are bounded and isolated.
- The local PCMU and Opus verticals pass.

### WP10 — Moshi full-duplex vertical

Tasks:

1. Implement common Moshi PCM/model frame adaptation at 24 kHz.
2. Implement the Moshi protocol client first and validate against the official
   Rust server.
3. Implement in-process official Rust/Candle provider using the same session
   contract.
4. Add the release-blocking CUDA backend and a separately feature-gated Metal
   preview, each with warm pools, model admission, and capability reporting.
5. Map user/model transcripts, speech state, output audio, metadata, errors,
   and usage.
6. Keep audio input and event/output reads in independent tasks.
7. Implement ordered `RealtimeOutputId → GenerationId` mapping and explicit
   output cancellation without blocking input or retagging stale audio.
8. Add typed context/retrieval injection following MoshiRAG.
9. Add ten-minute overlap/interruption/discontinuity/teardown tests on the
   qualified CUDA runner; run the same suite as non-blocking nightly evidence
   for the Metal preview.
10. Add SIP/PCMU and WebRTC/Opus end-to-end Moshi calls plus the UCTP bridge
    case required by the profile matrix.
11. Add a dedicated full-duplex example and operations guide.

Exit criteria:

- Moshi continuously consumes input while producing audio.
- Overlap does not deadlock or collapse into half-duplex operation.
- Mimi/Moshi private types never cross the public contract.
- The qualified CUDA profile and remote-official profile meet their documented
  performance/interoperability gates.
- Metal compiles, passes common contracts, and emits preview smoke evidence; it
  becomes a release gate only if promoted in the manifest.
- Model/session resources return to the warm-pool baseline after stop.

### WP11 — DSP and turn-quality providers

Tasks:

1. Implement optional Smart Turn adapter and feasibility/operator tests.
2. Implement DeepFilterNet Rust/tract audio processor.
3. Implement render-reference/delay instrumentation for future AEC.
4. Evaluate server AEC candidates; ship one only if it improves the defined echo
   corpus without unacceptable clean-speech/ASR damage.
5. Evaluate AGC; keep it off unless corpus evidence justifies a preset.
6. Tune Balanced turn/interruption defaults on the release corpus.
7. Record each parameter change with before/after quality and latency.

Exit criteria:

- Optional processor/detector failure falls back safely.
- DSP never changes sample-clock continuity.
- Default changes are evidence-backed and regression-gated.
- AEC/AGC claims match what is actually enabled and tested.

### WP12 — Observability, security, docs, and release qualification

Tasks:

1. Complete event, metric, trace, and vCon projections.
2. Add redaction/cardinality tests and security configuration.
3. Complete model/source/license/SBOM inventory.
4. Complete parser fuzzing, executable security gates, native/FFI qualification,
   candidate-specific fuzz profile, and no-Python child-process audit.
5. Run the complete corpus, fault, load, endurance, transport, and qualified
   hardware matrix.
6. Run MSRV/packaging/publish-order/offline packaged-source rebuild gates.
7. Generate the release attestation described in §15.
8. Update crate READMEs, root feature docs, PRD, interface design, examples,
   provider guides, operations, troubleshooting, and migration notes.
9. Publish supported hardware/provider matrices and known limitations.

Exit criteria:

- Every §15 release gate passes from a clean candidate revision.
- Documentation contains no old API.
- Release evidence is hash-bound and independently verifiable.

## 13. Sequencing and parallelism

~~~mermaid
flowchart LR
    WP0["WP0 Audit"] --> WP1["WP1 Contracts"]
    WP1 --> WP2["WP2 PCM + InProcessAi"]
    WP1 --> WP3["WP3 Registry + Testkit"]
    WP2 --> WP4["WP4 Supervisor"]
    WP3 --> WP4
    WP4 --> WP5["WP5 Turn + Playback"]
    WP5 --> WP6["WP6 Core Migration"]
    WP6 --> WP7["WP7 Cascaded Runtime"]
    WP7 --> WP8["WP8 Remote Providers"]
    WP7 --> WP9["WP9 Local Providers"]
    WP6 --> WP10["WP10 Moshi"]
    WP5 --> WP11["WP11 DSP + Turn Quality"]
    WP8 --> WP12["WP12 Release"]
    WP9 --> WP12
    WP10 --> WP12
    WP11 --> WP12
~~~

Parallel execution:

- WP2 and WP3 begin after contract freeze.
- Remote protocol utilities may be prototyped during WP4 but cannot define
  public contracts independently.
- WP8 and WP9 run in parallel after the cascaded runtime is stable.
- Moshi protocol investigation begins early, but WP10 integrates only through
  the frozen realtime contract and production media port.
- Observability and tests land within every package; WP12 is the final
  cross-cutting qualification, not the first time they are added.

## 14. Verification and evaluation

### 14.1 Reusable test rig

Build `VoiceAiTestRig` before the runtime:

- Manual monotonic sample/audio clock and Tokio paused time.
- Scripted caller and AI endpoints with independently readable PCM lanes.
- Deterministic activity, turn, ASR, model, TTS, realtime, and DSP providers.
- Exact provider barriers/notifications; no arbitrary short sleeps.
- Media probe recording direction, format, sample position, generation,
  enqueue, send, and estimated playout time.
- Ordered event/trace/metric recorder.
- Seeded impairment layer for loss, burst loss, jitter, reorder, duplicate,
  timestamp discontinuity, and channel closure.
- Leak tracker for tasks, channels, AI Connections, MediaGraph routes, provider
  sessions, warm-model permits, tenant permits, and queued frames.

Enable Tokio `test-util`. Use property tests for audio/timestamp/state
invariants and use Loom only for small cancellation, generation-epoch, and
queue-ownership primitives where exhaustive interleavings add value.

### 14.2 Unit and state-machine coverage

**Audio**

- Valid/invalid sample formats, channels, rates, byte lengths, and frame sizes.
- Sample-clock continuity across resampling and reframing.
- Partial final frames, long-duration arithmetic, RTP timestamp wrap.
- Discontinuity propagation on packet loss, queue overflow, source change, and
  provider reset.
- Proof that encoded payload is never delivered to a PCM provider and PCM is
  never sent directly to a negotiated network codec.

**Lifecycle**

- Listening while idle, generating, synthesizing, buffering, and playing.
- `stop_agent`/call end during provider lookup, warm-up, session open, tool call,
  TTS, realtime output, and cleanup.
- Idempotent cancel/close and one close per provider session.
- Concurrent ordered flush, synchronous generation cancel, and close at every
  boundary; close dominates new work and a cancel acknowledgement cannot
  reactivate output.
- No `AgentStarted` after target terminal state.
- Critical-lane failure and common terminal cleanup.
- Late/duplicate prior-epoch events ignored and counted.
- Lossless evidence sequencing/hash reconciliation, contiguous ACK release,
  508-record backpressure failure with an intact four-record closure reserve,
  outcome-tail replay/deduplication, and observer lag independence.
- Target DTMF/hold/resume/transfer/terminal forwarding versus application-
  injected DTMF.

**Generation and heard history**

- Every output-bearing normalized agent event/media frame carries the
  generation; raw realtime provider output carries an ordered
  `RealtimeOutputId` that maps exactly once.
- Epoch advances before awaited provider cancellation.
- Late text/audio/tool/completion cannot enter active state.
- Queue clearing preserves only sent frames.
- Alignment-based and conservative no-alignment history correction.
- False-interruption resume never replays sent audio.
- Races at every queue/control boundary.

**Turn and interruption**

- Silence, breath/noise, quiet and single-word replies, long monologues.
- Pauses immediately below/at/above every configured threshold.
- ASR final before/after VAD stop, revisions that withdraw a commit candidate,
  flush-deadline boundaries, and late revisions that cannot reopen an
  irreversible commit.
- Provider EOT/semantic evidence agreeing and disagreeing with timers.
- Maximum turn/forced commit/manual commit.
- DTMF during silence, caller speech, agent speech, and tool work.
- Backchannels, short false candidates, real barge-in, and simultaneous speech.
- Quiet audio never reaches v1 ASR; pre-roll is delivered exactly once before
  the first live turn-scoped frame.

### 14.3 Common provider contract suites

Every shipped provider supplies a constructor to a shared contract suite.

Common assertions:

- Descriptor matches observed formats, streaming, cancellation, alignment,
  tools, locality, hardware, and concurrency. Every profile-qualified language
  is exercised; broader advertised languages are capability metadata, not an
  unbounded release matrix.
- Sessions are isolated and bounded.
- Health snapshot/probe deadlines, one probe per candidate, preparation
  acquisition, matching open-session consumption, and drop-based reservation
  release.
- Startup/close/cancel deadlines and resource release.
- Structured redacted errors.
- No new event enqueue after close completes; already-buffered events remain
  harmless under the session/epoch fence.
- Health recovers after a transient startup or session failure.
- Construction does not implicitly download a model or contact a network.

Activity:

- Probability bounds, event/sample alignment, reset, lookback/window
  requirements, deterministic pinned-model behavior, real-time factor, and
  bounded long-silence memory.

Turn/interruption:

- Ordered audio/transcript/evidence fusion, probability bounds, continue/
  commit recommendation, interruption/backchannel/noise/uncertain
  classification, reset, timeouts, and deterministic fallback/error policy.

Audio processor:

- Capture/render-reference alignment, zero/one/many bounded output frames,
  sample-clock preservation, suppression declaration, bypass/failure policy,
  flush/reset, latency, and deadline behavior.

Observability:

- Exact event sequence and terminal-event uniqueness for every scripted path.
- Exact counter/gauge/histogram deltas and queue-occupancy convergence.
- Root/session/turn/generation/provider span parentage and correlation.
- Credential, transcript/tool, and audio canaries obey the §11.2 data-class
  matrix on every surface; authorized content is present where policy requires
  it and absent everywhere else.
- Registry maximum, metric-series cardinality cap, and lagging receiver
  behavior.

ASR:

- Format negotiation, ordered revisions, stability/final semantics, word
  timing, language, EOT, flush, close, cancel, malformed/out-of-order remote
  events, and active-turn-only replay.

Model:

- Streaming deltas, tools, DTMF, handoff, end-session, cancellation before and
  after first token, context correction, idempotency, usage, and failure.

TTS:

- Incremental Unicode text, punctuation/empty/long input, flush, finish,
  cancellation, valid PCM/sample clock, alignment, stream failure, and no stale
  output. Emitted speech must be non-silent, sample-valid, timestamp-contiguous,
  within clipping limits, and match deterministic duration/energy fingerprints
  where applicable. Provider output may be faster than real time; the harness
  must pace.

Realtime/Moshi:

- Simultaneous input writes and output/event reads.
- Continued input during model audio.
- Per-stream ordering for audio/transcripts/speech/tool/errors.
- `OutputStarted` precedes audio, one `RealtimeOutputId` maps to one
  `GenerationId`, cancellation cannot retag late output, and missing/duplicate
  starts are protocol errors.
- Private Mimi framing.
- Overlap, cancellation, backpressure, discontinuity, warm reuse, failure, and
  teardown on CPU capability-check, the qualified CUDA profile, and
  compile/contract-tested Metal preview.

### 14.4 Audio corpus

Maintain:

1. A small redistributable CI corpus checked into the repository.
2. A larger immutable release corpus fetched explicitly by tooling.

Each corpus uses a JSONL manifest containing:

- SHA-256, source/license, language, transcript, speaker count, sample rate,
  channels, and duration.
- Annotated speech, turn, backchannel, and interruption ranges.
- Expected commit/interruption outcome.
- Noise/SNR and echo delay/gain.
- Codec/network transformation and deterministic impairment seed.
- Applicable provider capabilities and scoring exclusions.

Required content:

- Clean 8/16/24/48 kHz speech.
- PCMU, PCMA, Opus, and internal PCM paths.
- Silence, breath, clipping, quiet speech, and microphone variation.
- 200/500/800/1,200 ms internal pauses.
- Short acknowledgements, corrections, backchannels, and real interruptions.
- Two-speaker overlap and background television speech.
- Café, vehicle, keyboard, music, and stationary noise at 20/10/5/0 dB SNR.
- Echo with 20–300 ms render-reference delays.
- 1/3/5/10% loss plus burst loss, jitter, reorder, duplicate, and timestamp
  discontinuity.
- DTMF mixed with silence and speech.
- Non-English samples for each `qualified_languages` entry in the applicable
  profile.

All release scoring is implemented in Rust. Optional offline research tools may
cross-check results but are not the release authority.

Metrics:

- VAD onset/offset error, missed speech, false-active time, and segment F1.
- Early-cut rate, endpoint decision latency, and turn macro F1.
- True-interruption recall, false-interruption rate, and backchannel retention.
- ASR WER/CER and endpoint latency by codec/noise/language.
- Noise suppression SI-SDR change, clean-speech attenuation, clipping, and
  downstream WER change.
- TTS first audio, real-time factor, underruns, invalid/NaN/clipped samples,
  alignment, and optional ASR round-trip intelligibility.
- Moshi first audio, sustained real-time factor, overlap progress, transcript
  completeness, and output continuity.

Absolute first-release quality floors apply before comparison with any
baseline. Each manifest profile names its applicable gate IDs; a provider-
native realtime profile does not inherit cascaded VAD/ASR/TTS/interruption
gates.

| Quality gate | Floor | Applies to |
|---|---:|---|
| Early caller-turn cutoff | ≤ 1% of annotated turns | Cascaded/harness-managed turn policy |
| True interruption recall | ≥ 95% | Cascaded/harness-managed interruption |
| False interruption confirmation, including cancelled backchannels | ≤ 5% | Cascaded/harness-managed interruption |
| Clean-speech VAD recall | ≥ 95% | Qualified cascaded VAD |
| Noisy-speech VAD recall at 10 dB SNR | ≥ 90% | Qualified cascaded VAD |
| Clean no-speech false-active time | ≤ 2% | Qualified cascaded VAD |
| Noisy 10 dB no-speech false-active time | ≤ 5% | Qualified cascaded VAD |
| Clean telephony ASR WER | ≤ 20% per qualified language | Qualified cascaded ASR |
| Noisy 10 dB ASR WER | ≤ 35% per qualified language | Qualified cascaded ASR |
| Controlled intent-label match | ≥ 90% | Qualified Moshi profiles |
| Generated output invalid/non-finite samples | Exactly zero | Cascaded TTS and realtime output |
| Generated output clipped samples | ≤ 0.1% of non-silent samples | Cascaded TTS and realtime output |
| Generated output audibility | Non-silent, expected duration range, continuous sample clock | Cascaded TTS and realtime output |

Release-corpus denominators are at least 500 annotated caller turns for early
cutoff, 200 true interruptions, 200 backchannel/non-interruption overlaps,
30 minutes each of active-speech and no-speech material in the clean and
10 dB VAD partitions, 5,000 reference words per qualified language for each ASR
profile, and 100 controlled Moshi intent fixtures. A smaller sample cannot
produce a passing release verdict.

Each profile may declare stricter ASR/turn/TTS thresholds and additional
qualified-language ceilings in the qualification manifest, but cannot weaken
these global floors. Partitions below 10 dB remain reported and
regression-gated until a reviewed absolute floor is added.

Baseline keys include corpus hash, model/provider hash, hardware class, and
feature set. A change fails quality regression when:

- WER worsens by more than one absolute percentage point.
- VAD/turn/interruption F1 falls by more than two percentage points.
- Latency or real-time factor worsens by more than 10% or 25 ms, whichever
  allowance is larger.
- Clean-speech processing worsens WER by more than 0.5 points.
- Any new invalid sample, stale-generation output, leak, panic, or crash occurs.

### 14.5 Mandatory cascaded vertical

Use real decoded/encoded audio:

1. Establish a real caller Connection and distinct `InProcessAi` Participant.
2. Feed the same licensed utterance through PCMU/PCMA 8 kHz, Opus 48 kHz, and
   the qualified UCTP mapping.
3. Observe continuous PCM at activity and ASR lanes.
4. Commit a turn, stream model text, stream TTS, pace output, transcode to the
   negotiated codec, and verify decoded output is non-silent, sample-valid,
   unclipped within the gate, timestamp-contiguous, and within the deterministic
   duration/energy fingerprint.
5. Inject caller audio during playback and prove zero unaccounted loss, no
   input-lane stall over 100 ms, and no statistically/absolutely disallowed
   degradation versus the output-disabled fixture.
6. Exercise a backchannel that resumes output.
7. Exercise an interruption that cancels model/TTS, flushes audio, fences stale
   work, and corrects heard context.
8. Exercise DTMF, one idempotent tool call, provider timeout/failover, and call
   disconnect.
9. Validate core events, per-agent events, metrics, traces, and vCon.

Run the vertical with:

- deterministic providers on every PR;
- the all-local no-Python providers;
- the required local-VAD/ASR + live-remote-model + local-TTS mixed profile;
- a live all-remote provider configuration during release qualification;
- the Vapi regression suite to prove its separate external-agent topology is
  unchanged.

### 14.6 Mandatory Moshi vertical

Run the in-process vertical on
`moshi-candle-cuda-linux-x86_64` and the protocol vertical on
`moshi-remote-official`:

1. Warm pinned Moshi/Candle/Mimi artifacts and record hashes.
2. Establish continuous two-way PCM through a real AI Connection.
3. Feed a recorded prompt and require valid model audio and typed transcript/
   speech events.
4. Count input frames and scheduling delay during output; no unexplained loss
   or ingress stall over 100 ms is allowed.
5. Inject overlapping caller speech and prove bidirectional progress.
6. Exercise output cancellation, discontinuity/reset, and orderly teardown.
   Exercise retrieval/tool/context injection only when the profile declares
   `supports_tools` and `supports_context_injection`; otherwise require
   `UnsupportedCapability` without interrupting duplex audio.
7. Run continuously for ten minutes without underrun trend, queue growth, task
   leak, or increasing per-session GPU allocation.
8. Repeat through SIP/PCMU, WebRTC/Opus, and the manifest's UCTP bridge case.
9. For the remote profile, run protocol interoperability against the pinned
   official Rust server revision, including setup, duplex audio, metadata,
   cancellation, error, and close frames.

Exact response wording is not a gate. Controlled semantic fixtures use a
pinned, deterministic Rust evaluator: the attested Rust ASR model/hash
transcribes the output and a fixed normalized-token/intent-label matcher scores
the annotated allowed labels. Open-ended samples are not hard semantic gates,
and no network LLM judge is permitted. Audio validity, controlled-fixture
classification, concurrency, ordering, latency, and resources are gates.

The Metal preview runs the same test for evidence but does not block the first
release unless its qualification entry is promoted before the candidate is
cut.

### 14.7 Fault-injection matrix

Inject each applicable fault at startup, mid-turn, during output, and teardown:

- Provider unavailable/authentication/rate limit/deadline/malformed response/
  clean EOF/abrupt disconnect/hung close.
- Model-load failure, bad checksum, unsupported operator, CUDA/Metal failure,
  and accelerator OOM.
- Slow ASR, stalled model, bursty TTS, non-cooperative cancel, output flood.
- Full/closed channel, receiver drop, caller disconnect, codec renegotiation,
  timestamp reset, and graph shutdown.
- Primary and fallback provider both failing.
- Disconnect racing with start, warm-up, failover, interruption, tool, or stop.
- Repeated start/stop and capacity churn.

Every scenario must:

- reach one explicit terminal state;
- emit one structured reason;
- avoid stale output;
- bound retry/failover;
- release all owned resources within the shutdown deadline.

### 14.8 Parser, native-runtime, and security qualification

Maintain bounded fuzz targets for:

- remote WebSocket/HTTP binary and JSON messages;
- Moshi protocol frames;
- PCM construction and resampling metadata;
- Unicode text chunking and UTF-8 alignment;
- streamed tool-call assembly;
- oversized, compressed, and decompression-amplified provider messages.

Seed regressions from every discovered crash. Pull requests run the curated
corpus; nightly runs coverage-guided campaigns with stored corpus/artifacts.

Every release candidate also runs the attested `voice-ai-fuzz-rc-v1` profile
from the exact candidate commit. Each target uses pinned engine/toolchain,
seed-corpus/dictionary hashes and deterministic initial seeds, runs for at
least 30 wall-clock minutes and 100,000 successful iterations, and has the
per-input timeout/RSS limits recorded in the profile. A target that cannot meet
both minimums within two hours fails qualification. Passing requires zero
crash, sanitizer finding, timeout/hang, or OOM; logs, final corpus, crashes, and
resource summaries are hash-bound into release evidence.

Executable security gates cover invalid TLS certificates and hostnames,
redirect rejection, endpoint allowlists, DNS/connect/handshake stalls,
oversized/compressed messages, credential canaries across logs/events/traces/
vCon/attestation, checksum mismatch, path traversal, read-only caches, and
atomic artifact installation.

For each qualified native/FFI provider, run supported ASan/LSan wrapper tests,
repeated model load/unload, cancellation during native inference, and runtime
heartbeat tests. Blocking native inference must use an explicitly bounded
worker pool and may not occupy Tokio core workers indefinitely.

## 15. Performance, CI, and release qualification

### 15.1 Runtime gates

Measure release builds on the unloaded hardware pinned by each qualification
profile with warm providers. Report harness overhead separately from
provider/model latency.

Measurement definitions are normative:

- Capture-to-provider is measured from AI-port receive on the harness clock to
  successful enqueue on the selected provider command channel.
- Turn lateness is decision time minus the earliest instant at which all
  configured commit conditions became eligible.
- Cascaded/harness-managed interruption timings include confirmed
  interruptions only. Speech onset is the annotated first caller-speech
  sample. “Local stop” ends at the last non-silent outbound frame accepted by
  the local transport adapter; “receiver silence” ends at the last non-silent
  decoded sample observed at the receiving endpoint.
- First audible output is the first non-silent decoded sample observed at the
  receiving test endpoint, not queue insertion, synthesis, or graph acceptance.
- Playback gap is the receiving endpoint's decoded inter-frame/sample gap
  during annotated active agent speech, excluding intentional silence and
  terminal padding.
- Moshi real-time factor is provider inference time divided by represented
  audio duration in consecutive ten-second windows over the ten-minute run;
  report every window and their P95.

| Gate | Release threshold | Applies to |
|---|---:|---|
| Capture-to-provider enqueue overhead | P95 ≤ 20 ms; P99 ≤ 40 ms | Cascaded and realtime input |
| Turn decision lateness beyond configured threshold | P95 ≤ 40 ms | Cascaded/harness-managed turn |
| Confirmed interruption to local queue clear | P95 ≤ 40 ms; P99 ≤ 80 ms | Cascaded/harness-managed interruption |
| Speech onset to local output stop | P95 ≤ 250 ms; P99 ≤ 400 ms | Cascaded/harness-managed interruption |
| Speech onset to receiver-observed silence | P95 ≤ 300 ms; P99 ≤ 500 ms | Cascaded/harness-managed interruption |
| Unaccounted source-frame loss | Exactly zero; every missing range emits discontinuity | Common |
| Input-lane scheduling delay while output is active | P99 ≤ 40 ms; maximum ≤ 100 ms; no more than 10 ms worse than output-disabled fixture | Common full duplex |
| Decoded playback gap on unloaded host | P99 ≤ 40 ms; maximum ≤ 100 ms for 20 ms frames | Common generated speech |
| Unexplained playback underruns | Exactly zero in mandatory unloaded verticals | Common generated speech |
| Burst catch-up after input/playback stall | Exactly zero | Common |
| Stale cancelled-generation frames/events | Exactly zero | Common |
| Warm end-of-turn to first audible output | P95 ≤ 1,000 ms | Qualified cascade profiles |
| Moshi sustained generation | P95 real-time factor ≤ 1.0 | Qualified in-process Moshi |
| Moshi warm input-to-first-audio | P95 ≤ 750 ms | Qualified CUDA Moshi |
| Live tasks/routes/permits after session drain | Exactly zero | Common |
| Provider-tracked live session allocations/handles after drain | Exactly zero | Common |
| Thirty-minute post-warm host memory slope | ≤ 15 MB/hour | Each qualified local profile |
| GPU allocation after drain/settling | No more than `min(5% of warm-pool baseline, 256 MiB)` above baseline | Qualified accelerator profiles |

Memory qualification separates active-load retention from structural cleanup.
Run for 35 minutes, discard the documented five-minute warm-up, and calculate
the following 30-minute slope. After each batch, wait a fixed 30-second
provider/device settling interval; then require zero live session allocations/
handles and take the post-drain sample. Sample host memory every five seconds,
retain at least 99% of expected samples, and calculate the reported slope with
a Theil–Sen estimator. Across ten repeated start/stop batches, a positive
one-sided Mann–Kendall trend with tie-corrected variance and `p < 0.05` in
post-drain host or device memory fails the gate even if the final percentage
threshold passes.

Release measurements:

- Fixed worker count, release profile, power/performance settings, model hashes,
  and no profiler.
- Warm providers/models before sampling.
- Three independent runs.
- At least 500 cascaded turns and 200 Moshi interactions.
- Every independent run must meet every applicable absolute correctness,
  quality, latency, and resource gate mapped to its profile.
- For a lower-is-better metric with baseline `B` and allowed regression `A`,
  the candidate median `C` must satisfy `C ≤ B + A`, and every run `R` must
  satisfy `R ≤ B + 1.2A`. For higher-is-better metrics, require
  `C ≥ B - A` and every `R ≥ B - 1.2A`. `A` is the gate-specific absolute
  allowance—for latency, `max(10% × B, 25 ms)`—so the 20% factor applies to
  the allowed delta, not the baseline or final threshold.
- Store raw samples and environment data, not only percentiles.

Run load profiles at 1, 10, and configured maximum concurrent sessions.
Admission beyond capacity must fail quickly without degrading active calls.

### 15.2 Pull-request CI

Add explicit jobs because workspace default-member testing does not currently
guarantee coverage of the extension harness.

Every PR:

- Rust 1.88 MSRV and stable Rust checks.
- Base contracts/harness with no model backend.
- Deterministic cascaded runtime and deterministic realtime runtime.
- ONNX CPU adapters using small pinned fixtures.
- PCMU, PCMA, Opus, and internal PCM integration.
- Fixed-seed property/race tests plus one rotating recorded seed.
- Formatting, Clippy, rustdoc, cargo-deny/advisories/licenses, and examples.
- Curated feature-powerset compilation excluding invalid accelerator pairs.
- Qualification-manifest schema, hash, profile/runner, and language/transport
  coverage validation.
- `cargo package`/publish-order dry runs and offline rebuilds from packaged
  sources for every non-accelerator qualified feature combination.
- Offline provider/model tests after an explicit checksum-verifying artifact
  fetch step.
- Build the local cascade normally, then execute the packaged release artifact
  in a minimal Linux container/restricted `PATH` with no Python executable,
  network-disabled model loading, read-only verified artifacts, and spawned-
  process auditing.
- Curated parser/PCM/text/tool fuzz corpus and security boundary tests.
- Dependency-tree assertion that production `voice-ai` does not include
  `testkit` or its helper dependencies.

Do not use a single workspace `--all-features` command for CUDA and Metal.

### 15.3 Nightly and hardware CI

Nightly:

- Small-corpus quality evaluation.
- SIP, UCTP, and headless-browser WebRTC agent integration.
- Network/DSP impairment matrix.
- Live remote-provider contract tests.
- DeepFilterNet and local cascade model tests.
- Ten-minute concurrency/load runs.
- Coverage-guided parser/protocol fuzzing.
- Loom cancellation primitives, supported sanitizer jobs, FFI load/unload and
  native-inference cancellation tests, and Tokio runtime-heartbeat checks.

Accelerator runners:

- Linux x86_64 CUDA: Rust 1.88/stable compile, package/publish-order/offline
  packaged-source rebuild, real Moshi smoke nightly, and full gates from the
  exact release candidate.
- macOS arm64 Metal: real Moshi preview smoke nightly; it is release-blocking
  only after manifest promotion to `qualified`; compile/package evidence still
  runs for the preview feature.
- Linux x86_64 CPU/native ONNX: mandatory per-PR provider tests.
- Linux aarch64 and Windows: compile and base contracts as Tier 2 until
  qualified for real local models.

Weekly/endurance:

- Two-hour cascaded and Moshi sessions.
- Repeated warm-pool churn.
- Remote failover matrix.
- Host/GPU retention and event-cardinality audit.

### 15.4 Release attestation

Generate a machine-verifiable `rvoip-voice-ai-release-attestation-v1` bundle:

- Clean source revision and source-tree fingerprint before/after testing.
- Cargo lockfile, feature set, Rust version, target, release profile, and binary
  hashes.
- Normalized qualification manifest, selected release-blocking profile IDs,
  maturity, and manifest hash.
- OS, CPU, RAM, GPU, driver/runtime, CUDA/Metal versions.
- Provider names/versions/endpoints without secrets.
- Model/tokenizer/codec/DSP hashes and license/attribution metadata.
- Corpus manifest/tree hashes and scenario seeds.
- Fuzz profile/engine/toolchain, seed and final corpus hashes, per-target
  iterations/runtime, and zero-finding summaries.
- Every command, timestamps, exit status, raw-result path, and gate result.
- No-Python/container child-process audit and packaged-source offline rebuild
  evidence.
- Raw metrics, summarized latency/quality, leak snapshots, and failure
  inventory.
- SHA-256 for every evidence artifact and the evidence tree.

The attestation format should follow rvoip's existing fail-closed, hash-bound
release evidence pattern.

### 15.5 Release gates

Release is blocked unless all pass from the exact candidate commit:

1. Base API/runtime and every bundled provider contract suite.
2. Deterministic, local, mixed, and live-all-remote cascaded verticals.
3. Qualified in-process Moshi CUDA vertical.
4. Remote Moshi interoperability against the pinned official Rust server.
5. PCMU, PCMA, Opus, SIP, WebRTC, UCTP, and unchanged Vapi regression paths.
6. Absolute corpus quality, relative regression, and runtime latency gates.
7. Fault-injection, parser/fuzz, native-runtime, security, redaction, and
   cardinality gates.
8. Thirty-minute cascade and Moshi retention runs plus repeated post-drain
   structural-cleanup checks.
9. Artifact integrity, model/source licenses, notices, SBOM, MSRV, packaging,
   and offline packaged-source rebuilds.
10. Documentation, examples, migration notes, and clean public API search.

Every manifest profile marked `qualified` and `release_blocking = true` must
pass; a preview/experimental profile cannot satisfy a gate. Both required
architectures, stale-generation safety, teardown/resource cleanup, redaction,
artifact integrity, and licensing are non-waivable.

No failed gate may be relabeled as passing. A signed, hash-bound exception may
document a failed internal candidate—failed gate, user impact, bounded
deployment scope, approver, and expiry—but the resulting attestation is
`NON-RC` and does not authorize a public release. An optional provider/platform
may instead be removed from the qualified support matrix before a new candidate
is built and the complete matrix is rerun.

## 16. Breaking removal and migration inventory

Remove without deprecated aliases or compatibility adapters:

- `AsrConfig`
- `AsrResult`
- old `AsrStream`
- old `AsrProvider::open_stream`
- `TtsRequest`
- `TtsPlayback`
- old `TtsProvider::synthesize`
- `DialogAction`
- `DialogManager`
- `register_dialog_manager`
- string-based ASR/TTS/dialog provider registration
- `attach_ai`
- `AiAttachmentId`
- `AttachmentRef::Ai`
- `Command::AttachAi`
- `AiAttached`
- `AiDetached`
- `BargeInDetected`
- old AI attachment registry/handle/supervision code
- `NoOpAsrProvider`
- `NoOpTtsProvider`
- `ListenOnlyDialog`

Update in the same change:

- `start_transcription` and its tests.
- Core/facade re-exports.
- `harness` feature naming and the `voip-3` feature composition.
- Tenant quota naming.
- Global and cross-crate event conversion.
- Command dispatchers and `Debug` matches.
- All AI tests in `recording_and_ai.rs`, `bridge_pump.rs`, and
  `p5_pause_and_listener.rs`.
- Example 11 and its standalone lockfile.
- PRD, interface design, crate READMEs, generated docs, and changelog.

Use the coordinated break sequence:

1. Add new unexposed contracts/runtime modules.
2. Migrate core and all callers/tests/examples.
3. Delete the old surface and temporary internal feature in one commit.
4. Run `rg` for every removed symbol and publish the migration table.

There is no release in which both public APIs are supported.

## 17. Definition of done

The first release is done only when:

- Cascaded and Moshi backends share one production AI media/session lifecycle.
- Caller input demonstrably continues during every form of agent output.
- Interruption cancellation and heard-prefix correction are race-safe.
- Every bundled provider passes its common contract.
- Real encoded media and real-model tests replace fake-payload evidence.
- Qualified local deployments and runtime startup require no Python
  interpreter or sidecar; packaged builds use only Rust/native inputs or
  preverified native artifacts.
- Queues, tasks, sessions, permits, CPU memory, and GPU memory remain bounded.
- Metrics, events, traces, and vCon reconstruct each turn without exposing
  sensitive content.
- All old experimental AI symbols and guidance are gone.
- The complete release attestation independently verifies.

## 18. Research and design references

Research was validated against primary project documentation as of 2026-07-30.

### rvoip

- [Product requirements](PRD.md)
- [Interface design](INTERFACE_DESIGN.md)
- [Conversation protocol](CONVERSATION_PROTOCOL.md)
- [Outstanding gap plan](GAP_PLAN.md)
- [Current provider traits](../crates/foundation/rvoip-core-traits/src/harness.rs)
- [Current media graph](../crates/foundation/rvoip-core/src/media_graph.rs)
- [Current AI attachment loop](../crates/foundation/rvoip-core/src/orchestrator.rs)
- [Existing Vapi transport](../crates/extensions/rvoip-vapi/README.md)

### Voice-agent orchestration

- [Daily/Pipecat and NVIDIA voice-agent architecture](https://www.daily.co/blog/daily-and-nvidia-collaborate-to-simplify-voice-agents-at-scale/)
- [LiveKit turn handling](https://docs.livekit.io/agents/logic/turns/)
- [LiveKit turn tuning](https://docs.livekit.io/agents/logic/turns/tuning/)
- [LiveKit adaptive interruption](https://docs.livekit.io/agents/logic/turns/adaptive-interruption-handling/)
- [LiveKit pipeline types](https://docs.livekit.io/agents/models/pipelines/)
- [Pipecat turn strategies](https://docs.pipecat.ai/api-reference/server/utilities/turn-management/user-turn-strategies)
- [Pipecat Smart Turn](https://github.com/pipecat-ai/smart-turn)
- [Pipecat frame model](https://docs.pipecat.ai/api-reference/server/frames/overview)
- [Vapi voice pipeline](https://docs.vapi.ai/customization/voice-pipeline-configuration)
- [FastRTC](https://github.com/gradio-app/fastrtc)
- [FastRTC audio guide](https://fastrtc.org/userguide/audio/)

### Kyutai and Laurent Mazaré

- [Moshi](https://github.com/kyutai-labs/moshi)
- [Moshi paper](https://arxiv.org/abs/2410.00037)
- [Moshi Rust protocol](https://github.com/kyutai-labs/moshi/blob/main/rust/protocol.md)
- [Moshi WebRTC proof of concept](https://github.com/kyutai-labs/moshi-webrtc)
- [MoshiRAG](https://github.com/kyutai-labs/moshi-rag)
- [Delayed Streams STT/TTS](https://github.com/kyutai-labs/delayed-streams-modeling)
- [Unmute cascaded reference](https://github.com/kyutai-labs/unmute)
- [Hibiki](https://github.com/kyutai-labs/hibiki)
- [LaurentMazare/xn](https://github.com/LaurentMazare/xn)
- [LaurentMazare/xn-ptts](https://github.com/LaurentMazare/xn-ptts)
- [LaurentMazare/xn-moshi-ws-server](https://github.com/LaurentMazare/xn-moshi-ws-server)

### Rust model and DSP candidates

- [Silero VAD](https://github.com/snakers4/silero-vad)
- [DeepFilterNet](https://github.com/Rikorose/DeepFilterNet)
- [sherpa-onnx Rust examples](https://github.com/k2-fsa/sherpa-onnx/tree/master/rust-api-examples)
- [Candle](https://github.com/huggingface/candle)
- [mistral.rs](https://github.com/EricLBuehler/mistral.rs)
- [ort](https://github.com/pykeio/ort)
