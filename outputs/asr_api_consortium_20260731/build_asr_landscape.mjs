import fs from "node:fs/promises";
import { SpreadsheetFile, Workbook } from "@oai/artifact-tool";

const outputDir = "/Users/jonathan/Developer/rvoip/outputs/asr_api_consortium_20260731";
const outputFile = `${outputDir}/ASR_API_Surface_Transport_Landscape_2026-07-31.xlsx`;

const vendors = [
  "Deepgram",
  "Speechmatics",
  "AssemblyAI",
  "ElevenLabs",
  "Soniox",
  "Gladia",
  "OpenAI",
  "Cartesia",
  "Google Cloud",
  "Azure Speech",
];

const Y = (note) => `✓ Native — ${note}`;
const L = (note) => `△ Limited — ${note}`;
const X = (note) => `✕ Explicitly unsupported — ${note}`;
const D = (note = "No public support documented") => `— ${note}`;
const statusCountFormula = (refs, symbol) =>
  `=SUM(${refs.map((ref) => `--(LEFT(${ref},1)="${symbol}")`).join(",")})`;

const featureRows = [
  // Transport and lifecycle
  ["Transport & lifecycle", "Asynchronous batch/file API", "Core", Y("REST prerecorded"), Y("REST batch"), Y("REST async transcript"), Y("REST sync/async"), Y("REST async"), Y("REST prerecorded"), Y("multipart file transcription"), Y("POST /stt"), Y("BatchRecognize"), Y("fast + batch REST")],
  ["Transport & lifecycle", "Synchronous file API", "Optional", Y("REST listen"), L("batch job API"), L("async-first REST"), Y("sync file conversion"), L("async-first REST"), L("async-first REST"), Y("file transcription"), Y("POST /stt"), Y("Recognize"), Y("fast transcription")],
  ["Transport & lifecycle", "Real-time bidirectional streaming", "Core", Y("WSS /v1; Flux /v2"), Y("WSS realtime"), Y("WSS /v3/ws"), Y("WSS realtime Scribe"), Y("WSS realtime"), Y("signed WSS live session"), Y("Realtime transcription"), Y("manual + auto-turn WSS"), Y("gRPC StreamingRecognize"), Y("Speech SDK stream")],
  ["Transport & lifecycle", "Public WebSocket API", "Compatibility binding", Y("binary audio + JSON events"), Y("binary audio + JSON protocol"), Y("binary audio + JSON messages"), Y("JSON/base64 audio events"), Y("binary audio + token events"), Y("binary audio + JSON events"), Y("JSON/base64 Realtime protocol"), Y("binary audio + JSON control"), D(), L("SDK-managed; raw protocol not primary API")],
  ["Transport & lifecycle", "Public gRPC streaming API", "Compatibility binding", D(), D(), D("internal/self-host transport only"), D(), D(), D(), D(), D(), Y("StreamingRecognize over HTTP/2"), D()],
  ["Transport & lifecycle", "WebRTC client transport", "Optional binding", D(), D(), D(), D("available in ElevenAgents, not direct Scribe"), D(), D(), Y("Realtime transcription media + data channel"), D(), D(), L("adjacent Voice Live, not standard STT")],
  ["Transport & lifecycle", "Direct SIP ingress", "Optional binding", D("self-host SIPREC add-on only"), D(), D(), D("ElevenAgents only"), D(), D(), Y("Realtime SIP"), D(), D(), D()],
  ["Transport & lifecycle", "Direct RTP/SRTP media API", "Optional binding", D("enterprise integrations, not public cloud API"), D(), D(), D("agent platform only"), D(), D(), L("SIP service terminates media; no raw RTP API"), D(), D(), D()],
  ["Transport & lifecycle", "Raw UDP API", "Not recommended", D(), D(), D(), D(), D(), D(), D("WebRTC may use UDP/SRTP; not raw UDP"), D(), D(), D()],
  ["Transport & lifecycle", "QUIC / WebTransport API", "Primary proposed binding", D(), D(), D(), D(), D(), D(), D(), D(), D(), D()],
  ["Transport & lifecycle", "Temporary client credentials", "Core security", Y("temporary/JWT token options"), Y("temporary JWT"), Y("temporary token query"), Y("single-use token"), Y("scoped temporary keys"), Y("temporary signed WebSocket URL"), Y("ephemeral client token"), Y("short-lived access token"), Y("OAuth access token"), Y("auth token / Entra ID")],
  ["Transport & lifecycle", "Application keepalive", "Optional", Y("KeepAlive message"), D(), Y("heartbeat/timeout controls"), D(), Y("keepalive message"), D(), D(), D("3-minute idle timeout documented"), L("generic gRPC keepalive, not STT message"), L("SDK-managed connection")],
  ["Transport & lifecycle", "Reconnect / resume same session", "Optional", D("reconnect creates new session"), D("no session resume"), D("no STT resume"), D("no resume protocol"), L("SDK reconnect starts a new session"), Y("reuse signed URL; preserves live context"), D("no audio/transcript resume"), D("reopen; no resume"), D("restart/audio bridging required"), D("no lossless resume semantics")],
  ["Transport & lifecycle", "Mid-session configuration update", "Core", Y("Flux Configure"), L("SetRecognitionConfig: selected fields"), Y("UpdateConfiguration"), D(), D(), D(), Y("session.update"), Y("turn thresholds via config"), D("config only in first request"), L("phrase updates; most settings recreate recognizer")],
  ["Transport & lifecycle", "Webhook / callback delivery", "Optional", Y("batch + streaming callbacks"), Y("batch notifications"), Y("batch + streaming webhooks"), Y("async webhooks"), Y("async webhooks"), Y("live callbacks + async"), L("SIP lifecycle; not standard file-STT completion"), D(), D("long-running operation polling"), L("selected batch/custom notifications")],

  // Audio
  ["Audio input", "Raw PCM", "Core", Y("linear16/linear32"), Y("s16le/f32le"), Y("PCM s16le"), Y("PCM multiple rates"), Y("many PCM encodings"), Y("PCM"), Y("PCM 24 kHz shown for Realtime"), Y("s16le/s32le/f16le/f32le"), Y("LINEAR16 and others"), Y("PCM via SDK/REST")],
  ["Audio input", "G.711 μ-law / A-law", "Core telephony", Y("mulaw/alaw"), Y("mulaw"), Y("mulaw streaming"), L("ulaw 8 kHz; A-law not live documented"), Y("mulaw/alaw"), Y("mulaw/alaw"), D("Realtime guide focuses on PCM"), Y("mulaw/alaw 8 kHz"), Y("MULAW/ALAW"), Y("G.711 through SDK/container")],
  ["Audio input", "Opus", "Optional codec", Y("Opus/Ogg-Opus"), Y("Ogg containers"), Y("raw/Ogg Opus"), D("live formats list PCM/ulaw"), Y("OGG/WebM autodetect"), D("live raw codecs PCM/G.711"), L("WebRTC media negotiates codecs; WS guide uses PCM"), D("Ink 2 realtime raw PCM/G.711"), Y("OGG_OPUS/WEBM_OPUS"), Y("compressed input through GStreamer")],
  ["Audio input", "Compressed/container audio", "Batch core", Y("broad file/container support"), Y("WAV/MP3/AAC/Ogg/FLAC/etc."), Y("AAC/ADTS + common file formats"), Y("broad batch formats"), Y("AAC/FLAC/MP3/OGG/WAV/WebM"), Y("pre-recorded common formats"), Y("file formats via transcription endpoint"), Y("batch compressed formats"), Y("broad encoded audio support"), Y("compressed audio via SDK/REST")],
  ["Audio input", "Explicit sample-rate configuration", "Core", Y("required for raw audio"), Y("raw audio config"), Y("8–96 kHz"), Y("8–48 kHz options"), Y("raw audio config"), Y("8/16/32/44.1/48 kHz"), Y("session audio format"), Y("sample_rate parameter"), Y("config / header inference"), Y("stream format / SDK config")],
  ["Audio input", "Automatic audio-format detection", "Optional", Y("container headers"), Y("file containers"), Y("file/batch detection"), Y("batch file detection"), Y("audio_format=auto"), Y("batch upload detection"), Y("file endpoint"), Y("batch file input"), Y("AutoDetectDecodingConfig"), L("file/SDK dependent")],
  ["Audio input", "Multichannel audio", "Optional", Y("independent channel transcripts"), Y("AddChannelAudio"), L("batch native; realtime separate sockets recommended"), L("batch up to 5 channels; realtime mono only"), D("no channel-index response documented"), Y("1–8 channels"), D("Realtime transcription channel labels not documented"), X("Ink 2 realtime mono only"), Y("up to 8 channels, model dependent"), L("batch stereo; standard realtime effectively mono")],
  ["Audio input", "Per-channel recognition / labels", "Optional", Y("channel_index"), Y("channel labels"), L("batch channel separation"), Y("batch channel index"), D(), Y("channel attribution"), D(), D(), Y("channel_tag / separate recognition"), L("batch channel support")],
  ["Audio input", "URL / cloud-storage audio input", "Batch optional", Y("URL callback/source options"), Y("job/file upload patterns"), Y("audio_url"), Y("cloud URL in batch request"), Y("file URL in async API"), Y("URL upload workflow"), L("client fetch/upload required for normal file endpoint"), L("client upload"), Y("GCS URI for batch"), Y("URL/SAS inputs for batch")],

  // Result events
  ["Results & events", "Interim / partial transcript", "Core", Y("interim_results"), Y("AddPartialTranscript"), Y("Turn events/revisions"), Y("partial_transcript"), Y("non-final tokens"), Y("is_final=false"), Y("transcript.delta"), Y("manual endpoint chunks; auto turn updates"), Y("is_final=false + stability"), Y("Recognizing event")],
  ["Results & events", "Final transcript", "Core", Y("is_final / speech_final"), Y("AddTranscript"), Y("final Turn"), Y("final_transcript"), Y("is_final tokens"), Y("is_final=true"), Y("transcription.completed"), Y("is_final / turn.end"), Y("is_final=true"), Y("Recognized event")],
  ["Results & events", "Immutable committed segment", "Core", Y("finalized results"), Y("final transcript"), Y("final turn"), Y("committed_transcript"), Y("final tokens"), Y("final utterance"), Y("completed item"), Y("auto-turn text never revised"), Y("final result"), Y("final recognition result")],
  ["Results & events", "Manual commit / finalize / flush", "Core", Y("Finalize; continue or close"), Y("ForceEndOfUtterance / EndOfStream"), Y("ForceEndpoint / Terminate"), Y("commit=true"), Y("finalize emits <fin> and continues"), L("stop_recording ends session; no per-turn commit"), Y("input_audio_buffer.commit"), Y("finalize command; close drains"), L("half-close stream; no per-turn commit"), L("stop/close input; no per-turn commit")],
  ["Results & events", "Word timestamps", "Core", Y("start/end per word"), Y("word/punctuation timing"), Y("word timing"), Y("word + character timing"), Y("token timing"), Y("word timing"), L("file whisper-1 only; live model lacks word timing"), Y("manual streaming + batch"), Y("optional word offsets"), Y("optional word offsets/durations")],
  ["Results & events", "Utterance / segment timestamps", "Core", Y("result timing"), Y("segment timing"), Y("turn timing"), Y("transcript event timing"), Y("token spans/fin markers"), Y("utterance timing"), Y("Realtime item/event timing"), Y("turn events"), Y("result_end_offset"), Y("offset/duration")],
  ["Results & events", "Word confidence", "Optional", Y("per-word confidence"), Y("alternative/word confidence"), Y("per-word confidence"), L("word log-probability, not normalized confidence"), Y("token confidence"), Y("word confidence"), X("unavailable for gpt-live-transcribe"), D(), Y("optional word confidence"), Y("detailed output; unavailable with semantic segmentation")],
  ["Results & events", "Utterance / transcript confidence", "Optional", Y("transcript confidence"), Y("alternative confidence"), Y("end-of-turn confidence"), D(), L("token confidence only"), Y("utterance confidence"), D(), D(), Y("top-alternative confidence"), Y("detailed output")],
  ["Results & events", "N-best alternatives", "Optional", X("only best transcript returned"), L("alternative arrays; no max-N knob guaranteed"), D(), D(), D(), D(), D(), D(), Y("max_alternatives up to 30"), Y("detailed output NBest")],
  ["Results & events", "Speech-start event", "Core", Y("SpeechStarted / Flux turn start"), D("no distinct start event documented"), Y("SpeechStarted"), D("VAD acts internally; no distinct event"), D(), Y("speech_start per channel"), Y("input_audio_buffer.speech_started"), Y("turn.start"), Y("SPEECH_ACTIVITY_BEGIN"), D("no comparable standard event documented")],
  ["Results & events", "Speech-stop / end event", "Core", Y("speech_final / EOT"), Y("EndOfUtterance"), Y("end_of_turn"), D("final event only"), Y("<end> endpoint token"), Y("speech_end per channel"), Y("input_audio_buffer.speech_stopped"), Y("turn.end"), Y("SPEECH_ACTIVITY_END"), L("recognized/segmentation event")],
  ["Results & events", "Machine-readable errors and usage", "Core", Y("error + metadata/usage"), Y("Error + recognition metadata"), Y("error + session statistics"), Y("error events"), Y("error + token metadata"), Y("error + session events"), Y("error + usage"), Y("error events"), Y("gRPC status + metadata"), Y("SDK cancellation/error + properties")],

  // Endpointing
  ["Turn detection", "VAD-based endpointing", "Core", Y("endpointing / Flux"), Y("silence trigger"), Y("turn detection VAD"), Y("VAD commit strategy"), Y("endpoint detection"), Y("VAD endpointing"), Y("server_vad"), Y("auto-turn; manual Whisper VAD"), Y("voice activity events/timeouts"), Y("silence-based segmentation")],
  ["Turn detection", "Configurable silence timeout", "Core", Y("endpointing + utterance_end_ms"), Y("0–2 s silence trigger"), Y("min/max turn silence"), Y("silence/min-speech controls"), Y("max_endpoint_delay_ms"), Y("0.01–10 s endpointing"), Y("silence_duration_ms"), Y("turn_end_timeout_ms"), Y("speech begin/end timeouts"), Y("initial + segmentation silence timeout")],
  ["Turn detection", "Semantic end-of-turn detection", "Recommended core", Y("Flux model-native turn detection"), D("external composition"), Y("Universal-3 Pro semantic turn detection"), D(), Y("semantic endpointing"), X("documented as silence/VAD, not semantic"), Y("semantic_vad"), Y("Ink 2 linguistic/conversational completeness"), D(), Y("semantic segmentation; locale/continuous-mode limits")],
  ["Turn detection", "Endpoint strategy selector", "Core", Y("endpointing vs Flux controls"), L("flex/fixed delay modes"), Y("model/mode/configuration choices"), Y("manual vs VAD commit"), Y("endpointing on/off + sensitivity"), L("VAD parameters only"), Y("none/server_vad/semantic_vad"), Y("manual vs auto-turn endpoint"), L("single-utterance model/timeouts"), Y("silence vs semantic segmentation")],
  ["Turn detection", "No-input timeout", "Optional", L("session/Flux lifecycle controls"), D(), Y("inactivity_timeout"), D(), D(), D(), Y("server VAD + application timer"), D(), Y("speech_begin_timeout"), Y("initial silence timeout")],
  ["Turn detection", "Maximum utterance / session duration", "Core", Y("service/session limits"), Y("max_delay/session limits"), Y("session duration controls/limits"), L("auto commit near 36 s"), Y("service limits"), Y("maximum utterance 5–60 s"), Y("session limits documented"), Y("3-minute idle; endpoint limits"), Y("streaming quotas/limits"), Y("service and segmentation limits")],
  ["Turn detection", "Eager end-of-turn + resume", "Optional", Y("EagerEndOfTurn + TurnResumed"), D(), L("turn revisions/interruption controls"), D(), D(), D(), L("semantic VAD eagerness; no explicit resume event"), Y("turn.eager_end + turn.resume"), D(), D()],
  ["Turn detection", "Barge-in / interruption signal", "Core voice-agent", Y("SpeechStarted / TurnResumed"), L("speech activity inferred externally"), Y("SpeechStarted + interruption delay"), L("partial/VAD commit flow"), L("endpoint tokens"), Y("speech_start"), Y("speech_started"), Y("turn.start / resume"), Y("speech activity begin"), L("Recognizing or ConversationTranscriber events")],

  // Recognition controls
  ["Recognition controls", "Model/profile selection", "Core", Y("model/version/tier"), Y("operating point/config"), Y("model + speech_model"), Y("model_id"), Y("model"), Y("model"), Y("transcription model"), Y("Ink 2 vs Ink Whisper endpoints"), Y("model/recognizer"), Y("base/custom endpoint/model")],
  ["Recognition controls", "Explicit language selection", "Core", Y("BCP-47 language"), Y("required language or pack"), Y("language code"), Y("language_code"), Y("language hints/strict mode"), Y("language config"), Y("language hints"), L("Ink 2 live is English; batch multilingual"), Y("language_codes"), Y("recognition language")],
  ["Recognition controls", "Automatic language detection", "Optional", Y("batch detect_language"), Y("batch auto; multi packs"), Y("language detection"), Y("auto detection"), Y("automatic multilingual ID"), Y("automatic detection"), L("committed-turn detection for selected model"), D(), Y("automatic recognition config"), Y("at-start/continuous LID")],
  ["Recognition controls", "Intra-utterance code switching", "Optional", Y("supported multilingual models"), Y("bilingual/multi language packs"), Y("Universal-3.5 code switching"), Y("mixed-language support"), Y("token-level language + code switching"), Y("code_switching per utterance"), L("multiple expected languages; no per-word guarantee"), D(), L("dominant-language selection; not guaranteed true switching"), L("continuous LID / selected multilingual models")],
  ["Recognition controls", "Keyterms / phrase biasing", "Core", Y("keyterm/keywords"), Y("additional_vocab + sounds_like"), Y("keyterms_prompt"), Y("keyterms"), Y("context.terms"), Y("custom vocabulary + pronunciations"), Y("prompt/keywords"), Y("up to 100 terms / 1,200 chars"), Y("PhraseSets + CustomClasses"), Y("PhraseListGrammar + weight")],
  ["Recognition controls", "Free-text prompt / context", "Core", Y("keyterms and hints"), L("additional vocabulary, not free-form prompt"), Y("prompt / agent_context"), Y("previous_text first chunk"), Y("context.general/text"), L("custom vocabulary/spelling"), Y("prompt"), L("keyterms, not broad prompt"), L("adaptation resources, not free-form prompt"), L("phrase lists, not free-form prompt")],
  ["Recognition controls", "Midstream vocabulary/context update", "Optional", Y("Flux Configure keyterms/hints"), L("selected config fields only"), Y("UpdateConfiguration"), D(), D(), D(), Y("session.update"), L("thresholds update; keyterms fixed at connect"), D(), Y("runtime phrase-list add/clear")],
  ["Recognition controls", "Automatic punctuation / casing", "Core", Y("punctuate"), Y("punctuation config"), Y("format_turns / formatting"), Y("model-generated"), Y("smart formatting"), Y("model formatting"), Y("model-generated"), Y("model-generated"), Y("automatic/spoken punctuation"), Y("punctuation modes")],
  ["Recognition controls", "Smart formatting / inverse text normalization", "Core", Y("smart_format"), Y("ITN"), Y("formatted turns"), Y("no_verbatim cleanup"), Y("built in"), Y("formatting"), Y("model-generated formatted transcript"), Y("structured formatting"), Y("transcript normalization"), Y("lexical/ITN/masked-ITN forms")],
  ["Recognition controls", "Profanity filtering", "Optional", Y("profanity_filter"), L("profanity metadata tags; no masking"), Y("filter_profanity"), L("batch content controls"), D(), D(), D(), D(), Y("profanity_filter"), Y("mask/remove/tag controls")],
  ["Recognition controls", "PII / sensitive-data redaction", "Optional", Y("PII/PCI/PHI/number redaction"), D("no native masking/PII redaction"), Y("PII redaction"), L("batch redaction; not realtime"), D(), L("pre-recorded PII redaction; not live"), D(), D(), D(), L("requires separate Azure AI Language PII service")],
  ["Recognition controls", "Speaker diarization", "Optional", Y("diarize"), Y("speaker/channel diarization"), Y("realtime speaker labels + revisions"), L("batch only, up to 32 speakers"), Y("realtime diarization"), D("no live diarization"), L("file-only diarization model; not live"), D(), Y("model/method dependent; not all streaming models"), Y("ConversationTranscriber + batch")],
  ["Recognition controls", "Speaker-count hints / bounds", "Optional", L("speaker identification without explicit count bounds"), Y("diarization configuration"), L("speaker label controls"), Y("batch speaker count"), L("diarization without count hint documented"), D(), L("file diarization options"), D(), Y("min/max speaker count"), Y("batch/ConversationTranscriber settings")],
  ["Recognition controls", "Non-speech / audio event tags", "Optional", D(), Y("applause/laughter/music"), L("batch audio intelligence only"), L("batch audio events/entity detection"), L("optional audio_event field; taxonomy/control unclear"), X("live audio events not supported"), D(), D(), D(), D()],
  ["Recognition controls", "Native translation", "Optional extension", D(), Y("batch + realtime translation"), L("batch translation; streaming needs separate model/gateway"), D("separate products/workflows"), Y("realtime translation"), Y("realtime translation"), L("separate file/realtime translation model"), D(), Y("translation config, model dependent"), Y("TranslationRecognizer/API")],
  ["Recognition controls", "Entity detection / NER", "Optional extension", D(), D(), L("batch audio intelligence"), Y("batch entities; realtime entity events"), D(), Y("live NER; redaction separate"), D(), D(), D(), D()],

  // Deployment
  ["Deployment & governance", "Customer-trained custom ASR model", "Optional extension", Y("enterprise custom model options"), D("vocabulary adaptation, not training API"), D("no customer-training API documented"), D(), D(), D(), D(), D(), L("Custom Speech-to-Text models are Preview"), Y("Custom Speech models/endpoints")],
  ["Deployment & governance", "Self-host / on-prem deployment", "Optional extension", Y("self-hosted enterprise"), Y("self-host/on-prem"), Y("self-hosted streaming"), Y("private deployment"), D(), D(), D(), Y("private cloud/on-prem/Kubernetes/air-gapped"), L("private offering; no general on-prem product"), Y("speech-to-text containers")],
  ["Deployment & governance", "Regional endpoints / data residency", "Core governance", Y("global/EU/AU"), Y("regional deployments"), Y("global/US/EU data zones"), Y("enterprise residency options"), Y("US/EU/Japan"), Y("US/EU live regions"), D("not specified on reviewed speech API pages"), Y("enterprise regional endpoints"), Y("regional + US/EU multi-region"), Y("region-specific and sovereign cloud")],
  ["Deployment & governance", "Private networking / customer storage", "Optional extension", L("enterprise/self-host options"), L("private deployment options"), L("self-host/private infrastructure"), L("private deployment"), D(), D(), D(), Y("private-cloud/self-host options"), Y("VPC Service Controls/private access patterns"), Y("Private Link/container patterns")],
  ["Deployment & governance", "Zero-retention / logging opt-out", "Core governance", L("enterprise data controls"), L("deployment-dependent"), L("data-zone/self-host controls"), L("enterprise controls"), L("residency controls"), L("regional/data controls"), L("API data-control policy dependent"), Y("zero-retention/private deployment options"), L("logging/data controls by configuration"), L("resource/container controls")],
  ["Deployment & governance", "Telephony codecs / integrations", "Core voice-agent", Y("G.711/G.729 + SIPREC/UniMRCP add-ons"), Y("μ-law + telephony workflows"), Y("μ-law streaming"), Y("μ-law + ElevenAgents SIP"), Y("μ-law/A-law"), Y("μ-law/A-law"), Y("direct SIP Realtime"), Y("μ-law/A-law 8 kHz"), Y("μ-law/A-law/AMR + telephony models"), Y("8 kHz PCM/G.711 via SDK/container")],
];

const transportRows = [
  ["Deepgram", "WSS /v1/listen; Flux /v2/listen", "WebSocket over TCP/TLS", "Binary audio frames", "JSON config/control/results", "API token; temporary/JWT options", "TCP loss can delay later audio on same connection", "Reconnect starts a new session", "Enterprise SIPREC/UniMRCP; broad telephony codecs", "Good semantic-profile reference; add QUIC binding"],
  ["Speechmatics", "Realtime /v2", "WebSocket over TCP/TLS", "Binary audio / channel messages", "JSON protocol messages", "Bearer API key or temporary JWT", "TCP head-of-line behavior", "No session resume", "μ-law and self-hosted deployment", "Strong events, diarization, translation, audio tags"],
  ["AssemblyAI", "/v3/ws", "WebSocket over TCP/TLS", "Binary audio", "JSON configuration and Turn events", "API key or temporary token", "TCP head-of-line behavior", "No STT resume", "μ-law; self-host option", "Strong turn schema and live reconfiguration"],
  ["ElevenLabs", "Scribe Realtime", "WebSocket over TCP/TLS", "Base64 audio in JSON events", "JSON transcript/commit events", "xi-api-key or single-use token", "TCP HOL plus base64 overhead", "No resume protocol", "μ-law; SIP in ElevenAgents rather than direct Scribe", "Committed-transcript semantics are useful"],
  ["Soniox", "Realtime STT", "WebSocket over TCP/TLS", "Binary audio after initial config", "JSON token events and finalize controls", "API key or scoped temporary key", "TCP head-of-line behavior", "SDK reconnect opens new session", "μ-law/A-law", "Token stream + semantic endpointing reference"],
  ["Gladia", "Live v2 signed session", "WebSocket over TCP/TLS", "Binary audio", "JSON events and callbacks", "API key for init; signed WSS URL", "TCP head-of-line behavior", "Reconnect to signed URL can retain context", "μ-law/A-law; 1–8 channels", "Best resume behavior among WSS specialists"],
  ["OpenAI", "Realtime transcription", "WebSocket or WebRTC; SIP ingress", "WS: base64 append; WebRTC: SRTP media", "JSON events over WS/data channel", "Bearer key; ephemeral client token", "WS has TCP HOL; WebRTC media is loss-tolerant/UDP-capable", "No audio/transcript resume", "Direct SIP; WebRTC browser path", "Closest deployed example of split media/control"],
  ["Cartesia", "Ink 2 manual + auto-turn", "WebSocket over TCP/TLS", "Binary raw audio", "JSON config and turn/transcript events", "X-API-Key or short-lived access token", "TCP head-of-line behavior", "Reopen required after disconnect", "μ-law/A-law 8 kHz; mono", "Excellent native turn event vocabulary"],
  ["Google Cloud", "StreamingRecognize", "gRPC over HTTP/2/TCP/TLS", "Protobuf audio bytes", "Protobuf config/results/status", "OAuth/ADC + IAM", "HTTP/2 multiplexing still inherits TCP connection HOL", "Restart/audio bridging for stream limits", "Telephony encodings/models", "Useful typed-IDL precedent; map protobuf to QUIC"],
  ["Azure Speech", "Speech SDK continuous recognition", "SDK-managed service stream (generally WebSocket/TCP)", "Push/pull audio stream through SDK", "SDK events/properties", "Key/token/Entra ID", "Underlying TCP stream can exhibit HOL", "No lossless standard resume", "G.711 through SDK/container; containers available", "Useful SDK abstraction and deployment precedent"],
];

const proposedRows = [
  ["Session", "session.create", "Client → server", "Reliable + ordered", "Bidirectional control stream", "JSON text message", "SIP/SDP binding establishes equivalent session", "session_id, version, auth scope, capabilities", "Capability negotiation; vendor extensions namespaced"],
  ["Session", "session.created", "Server → client", "Reliable + ordered", "Control stream", "JSON text message", "200/SDP response mapping", "session_id, negotiated profile, limits", "Returns selected codec, model, languages, extensions"],
  ["Session", "session.update", "Client → server", "Reliable + ordered", "Control stream", "JSON text message", "Control-channel extension", "update_id, changed controls", "Atomic updates with applied/rejected response"],
  ["Session", "session.close", "Either direction", "Reliable + ordered", "Control stream + CONNECTION_CLOSE after drain", "JSON close then WebSocket close", "BYE mapping", "reason, drain_results", "Graceful drain is distinct from cancel"],
  ["Audio", "audio.configure", "Client → server", "Reliable + ordered", "Control stream", "JSON text message", "SDP/RTP payload mapping", "codec, sample_rate, channels, clock_rate", "Creates a codec epoch referenced by datagrams"],
  ["Audio", "audio.frame", "Client → server", "Unreliable, congestion-controlled", "QUIC DATAGRAM with context ID", "Binary WebSocket fallback (reliable)", "RTP/SRTP packet", "stream_id, seq, media_timestamp, codec_epoch, payload", "Primary low-latency path; never retransmit stale audio"],
  ["Audio", "audio.commit", "Client → server", "Reliable + ordered", "Control stream", "JSON text message", "Marker/control mapping", "audio_seq_end, commit_id", "For manual turns; server acknowledges final boundary"],
  ["Audio", "audio.clear", "Client → server", "Reliable + ordered", "Control stream", "JSON text message", "Control mapping", "through_seq, reason", "Drop queued, unprocessed audio after barge-in/cancel"],
  ["Recognition", "recognition.start", "Client → server", "Reliable + ordered", "Control stream", "JSON text message", "MRCP/RTP control mapping", "recognition_id, controls", "May be implicit at session.create in simple profile"],
  ["Recognition", "recognition.cancel", "Client → server", "Reliable + ordered", "Control stream", "JSON text message", "MRCP STOP mapping", "recognition_id, reason", "Stops decoding without closing the media session"],
  ["Input events", "input.speech_started", "Server → client", "Reliable + ordered", "Result/event stream", "JSON text message", "Control event", "recognition_id, audio_seq, media_timestamp", "Canonical barge-in signal"],
  ["Input events", "input.speech_stopped", "Server → client", "Reliable + ordered", "Result/event stream", "JSON text message", "Control event", "recognition_id, audio_seq, media_timestamp", "VAD boundary, not necessarily semantic turn end"],
  ["Turn events", "turn.eager_end", "Server → client", "Reliable + ordered", "Result/event stream", "JSON text message", "Control event", "turn_id, confidence, transcript_revision", "Speculative agent start; must be reversible"],
  ["Turn events", "turn.resumed", "Server → client", "Reliable + ordered", "Result/event stream", "JSON text message", "Control event", "turn_id, reason", "Cancels speculative response when speaker continues"],
  ["Turn events", "turn.ended", "Server → client", "Reliable + ordered", "Result/event stream", "JSON text message", "Control event", "turn_id, reason, final_audio_seq", "Reason enum: silence, semantic, manual, max_duration"],
  ["Transcript", "transcript.delta", "Server → client", "Reliable + ordered", "Dedicated result stream", "JSON text message", "Control event", "item_id, revision, text_delta, timing optional", "Revision rules prevent vendor-specific ambiguity"],
  ["Transcript", "transcript.committed", "Server → client", "Reliable + ordered", "Result stream", "JSON text message", "Control event", "item_id, revision, immutable_text", "Immutable segment; may precede whole-turn final"],
  ["Transcript", "transcript.final", "Server → client", "Reliable + ordered", "Result stream", "JSON text message", "Control event", "turn_id, text, words?, confidence?, alternatives?", "Normalized optional result fields"],
  ["Recognition", "recognition.completed", "Server → client", "Reliable + ordered", "Result stream", "JSON text message", "MRCP COMPLETE mapping", "recognition_id, reason, usage", "Completion is distinct from transport closure"],
  ["Recognition", "recognition.failed", "Server → client", "Reliable + ordered", "Result stream", "JSON text message", "MRCP error mapping", "code, retryable, details", "Portable error taxonomy"],
  ["Operations", "usage", "Server → client", "Reliable + ordered", "Result stream", "JSON text message", "Control extension", "audio_ms, billed_ms, model_units", "Common observability and billing counters"],
  ["Operations", "ping / pong", "Either direction", "Unreliable or reliable", "QUIC PING or control event", "WebSocket ping/pong", "Binding-specific", "nonce, timestamp", "Application event only if RTT/health is exposed"],
];

const evidenceRows = [
  ["Deepgram", "Streaming API", "https://developers.deepgram.com/reference/speech-to-text/listen-streaming", "2026-07-31", "Transport, auth, realtime request/response"],
  ["Deepgram", "Prerecorded API", "https://developers.deepgram.com/reference/speech-to-text/listen-pre-recorded", "2026-07-31", "Batch REST surface"],
  ["Deepgram", "Audio encodings", "https://developers.deepgram.com/docs/encoding", "2026-07-31", "Raw codecs and sample rates"],
  ["Deepgram", "Endpointing", "https://developers.deepgram.com/docs/endpointing", "2026-07-31", "VAD and silence controls"],
  ["Deepgram", "Utterance end", "https://developers.deepgram.com/docs/utterance-end", "2026-07-31", "UtteranceEnd event"],
  ["Deepgram", "Finalize", "https://developers.deepgram.com/docs/finalize", "2026-07-31", "Manual finalization"],
  ["Deepgram", "Flux configuration", "https://developers.deepgram.com/docs/flux/configuration", "2026-07-31", "Model-native turn controls"],
  ["Deepgram", "Flux dynamic configure", "https://developers.deepgram.com/docs/flux/configure", "2026-07-31", "Mid-session updates"],
  ["Deepgram", "Confidence", "https://developers.deepgram.com/docs/confidence", "2026-07-31", "Confidence and alternatives behavior"],
  ["Deepgram", "Redaction", "https://developers.deepgram.com/docs/redaction", "2026-07-31", "PII/PCI/PHI controls"],
  ["Deepgram", "Channels and diarization", "https://developers.deepgram.com/docs/multichannel-vs-diarization", "2026-07-31", "Speaker and channel semantics"],
  ["Speechmatics", "Realtime quickstart", "https://docs.speechmatics.com/speech-to-text/realtime/quickstart", "2026-07-31", "Realtime WebSocket flow"],
  ["Speechmatics", "Realtime API reference", "https://docs.speechmatics.com/api-ref/realtime-transcription-websocket", "2026-07-31", "Messages, audio, results, configuration"],
  ["Speechmatics", "Turn detection", "https://docs.speechmatics.com/speech-to-text/realtime/turn-detection", "2026-07-31", "End-of-utterance controls"],
  ["Speechmatics", "Realtime diarization", "https://docs.speechmatics.com/speech-to-text/realtime/realtime-diarization", "2026-07-31", "Speaker/channel modes"],
  ["Speechmatics", "Custom dictionary", "https://docs.speechmatics.com/speech-to-text/features/custom-dictionary", "2026-07-31", "Vocabulary and sounds-like"],
  ["Speechmatics", "Translation", "https://docs.speechmatics.com/speech-to-text/features/translation", "2026-07-31", "Native translation"],
  ["Speechmatics", "Audio events", "https://docs.speechmatics.com/speech-to-text/features/audio-events", "2026-07-31", "Non-speech tags"],
  ["AssemblyAI", "Streaming WebSocket API", "https://www.assemblyai.com/docs/streaming/api-spec/streaming-websocket", "2026-07-31", "Transport, audio, configuration"],
  ["AssemblyAI", "Streaming message sequence", "https://www.assemblyai.com/docs/streaming/message-sequence", "2026-07-31", "Session and result events"],
  ["AssemblyAI", "Turn detection", "https://www.assemblyai.com/docs/streaming/turn-detection", "2026-07-31", "VAD/semantic controls"],
  ["AssemblyAI", "Dynamic configuration", "https://www.assemblyai.com/docs/streaming/updating-configuration-mid-stream", "2026-07-31", "Mid-session updates"],
  ["AssemblyAI", "Speaker/channel labels", "https://www.assemblyai.com/docs/streaming/label-speakers-and-separate-channels", "2026-07-31", "Diarization and channels"],
  ["AssemblyAI", "Streaming webhooks", "https://www.assemblyai.com/docs/streaming/webhooks", "2026-07-31", "Callbacks"],
  ["AssemblyAI", "Self-hosted streaming", "https://www.assemblyai.com/docs/streaming/self-hosted-streaming", "2026-07-31", "Deployment"],
  ["ElevenLabs", "Realtime Scribe API", "https://elevenlabs.io/docs/api-reference/speech-to-text/v-1-speech-to-text-realtime", "2026-07-31", "Realtime surface and controls"],
  ["ElevenLabs", "Commit strategies", "https://elevenlabs.io/docs/eleven-api/guides/how-to/speech-to-text/realtime/transcripts-and-commit-strategies", "2026-07-31", "Manual/VAD commits"],
  ["ElevenLabs", "Realtime event reference", "https://elevenlabs.io/docs/eleven-api/guides/how-to/speech-to-text/realtime/event-reference", "2026-07-31", "Partial/final/committed events"],
  ["ElevenLabs", "Batch conversion", "https://elevenlabs.io/docs/api-reference/speech-to-text/convert", "2026-07-31", "Batch formats and audio intelligence"],
  ["ElevenLabs", "Data residency", "https://elevenlabs.io/docs/overview/administration/data-residency", "2026-07-31", "Regional governance"],
  ["ElevenLabs", "Private deployment", "https://elevenlabs.io/docs/eleven-api/private-deployment/overview", "2026-07-31", "Self-host/private options"],
  ["Soniox", "WebSocket API", "https://soniox.com/docs/api-reference/stt/websocket-api", "2026-07-31", "Transport, configuration, token schema"],
  ["Soniox", "Realtime transcription", "https://soniox.com/docs/stt/rt/real-time-transcription", "2026-07-31", "Realtime workflow"],
  ["Soniox", "Endpoint detection", "https://soniox.com/docs/stt/rt/endpoint-detection", "2026-07-31", "Semantic endpointing"],
  ["Soniox", "Manual finalization", "https://soniox.com/docs/stt/rt/manual-finalization", "2026-07-31", "Finalize and continue"],
  ["Soniox", "Keepalive", "https://soniox.com/docs/stt/rt/connection-keepalive", "2026-07-31", "Connection lifecycle"],
  ["Soniox", "Context", "https://soniox.com/docs/stt/concepts/context", "2026-07-31", "Hints and terms"],
  ["Soniox", "Language identification", "https://soniox.com/docs/stt/concepts/language-identification", "2026-07-31", "Multilingual/code switching"],
  ["Soniox", "Speaker diarization", "https://soniox.com/docs/stt/concepts/speaker-diarization", "2026-07-31", "Speaker labels"],
  ["Gladia", "Live initialization", "https://docs.gladia.io/api-reference/v2/live/init", "2026-07-31", "Signed-session setup"],
  ["Gladia", "Live WebSocket", "https://docs.gladia.io/api-reference/v2/live/websocket", "2026-07-31", "Events and audio transport"],
  ["Gladia", "Live AsyncAPI", "https://docs.gladia.io/asyncapi.yaml", "2026-07-31", "Machine-readable message schema"],
  ["Gladia", "Endpointing", "https://docs.gladia.io/chapters/live-stt/features/endpointing", "2026-07-31", "VAD controls"],
  ["Gladia", "Partial transcripts", "https://docs.gladia.io/chapters/live-stt/features/partial-transcripts", "2026-07-31", "Interim/final semantics"],
  ["Gladia", "Live audio intelligence", "https://docs.gladia.io/chapters/live-stt/audio-intelligence", "2026-07-31", "NER/translation limitations"],
  ["OpenAI", "Realtime transcription", "https://developers.openai.com/api/docs/guides/realtime-transcription", "2026-07-31", "Session, audio, transcript events"],
  ["OpenAI", "Realtime VAD", "https://developers.openai.com/api/docs/guides/realtime-vad", "2026-07-31", "Server and semantic VAD"],
  ["OpenAI", "Realtime WebSocket", "https://developers.openai.com/api/docs/guides/realtime-websocket", "2026-07-31", "WebSocket binding"],
  ["OpenAI", "Realtime WebRTC", "https://developers.openai.com/api/docs/guides/realtime-webrtc", "2026-07-31", "WebRTC media/control binding"],
  ["OpenAI", "Realtime SIP", "https://developers.openai.com/api/docs/guides/realtime-sip", "2026-07-31", "SIP ingress"],
  ["OpenAI", "File speech-to-text", "https://developers.openai.com/api/docs/guides/speech-to-text", "2026-07-31", "Batch/file transcription"],
  ["Cartesia", "Batch transcription", "https://docs.cartesia.ai/api-reference/stt/transcribe", "2026-07-31", "Batch API"],
  ["Cartesia", "Automatic-turn WebSocket", "https://docs.cartesia.ai/api-reference/stt/turns/websocket", "2026-07-31", "Turn-native protocol"],
  ["Cartesia", "Manual WebSocket", "https://docs.cartesia.ai/api-reference/stt/websocket", "2026-07-31", "Manual finalize protocol"],
  ["Cartesia", "Endpoint comparison", "https://docs.cartesia.ai/use-the-api/stt/compare-endpoints", "2026-07-31", "Manual vs automatic turns"],
  ["Cartesia", "Semantic turns", "https://docs.cartesia.ai/use-the-api/stt/turns", "2026-07-31", "Native turn detection"],
  ["Cartesia", "Audio input", "https://docs.cartesia.ai/build-with-cartesia/stt/audio-input", "2026-07-31", "Codecs and sample rates"],
  ["Cartesia", "Keyterms", "https://docs.cartesia.ai/use-the-api/stt/keyterms", "2026-07-31", "Phrase biasing limits"],
  ["Cartesia", "Self-hosting", "https://docs.cartesia.ai/self-hosted/introduction", "2026-07-31", "Private deployment"],
  ["Google Cloud", "Streaming recognition", "https://docs.cloud.google.com/speech-to-text/docs/streaming-recognize", "2026-07-31", "gRPC streaming"],
  ["Google Cloud", "V2 RPC reference", "https://docs.cloud.google.com/speech-to-text/docs/reference/rpc/google.cloud.speech.v2", "2026-07-31", "Typed API surface"],
  ["Google Cloud", "Voice activity events", "https://docs.cloud.google.com/speech-to-text/docs/voice-activity-events", "2026-07-31", "Speech events/timeouts"],
  ["Google Cloud", "Audio encodings", "https://docs.cloud.google.com/speech-to-text/docs/encoding", "2026-07-31", "Codec support"],
  ["Google Cloud", "Chirp 3", "https://docs.cloud.google.com/speech-to-text/docs/models/chirp-3", "2026-07-31", "Model feature limits"],
  ["Google Cloud", "Locations", "https://docs.cloud.google.com/speech-to-text/docs/locations", "2026-07-31", "Regional endpoints"],
  ["Google Cloud", "Custom speech models", "https://cloud.google.com/speech-to-text/docs/custom-speech-models/train-model", "2026-07-31", "Preview customer-trained model lifecycle"],
  ["Azure Speech", "Realtime recognition", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/how-to-recognize-speech", "2026-07-31", "Speech SDK realtime surface"],
  ["Azure Speech", "REST speech-to-text", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/rest-speech-to-text", "2026-07-31", "Fast and batch REST"],
  ["Azure Speech", "Recognition results", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/get-speech-recognition-results", "2026-07-31", "Timestamps, confidence, N-best"],
  ["Azure Speech", "Phrase lists", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/improve-accuracy-phrase-list", "2026-07-31", "Runtime vocabulary"],
  ["Azure Speech", "Compressed audio", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/how-to-use-codec-compressed-audio-input-streams", "2026-07-31", "Codec input"],
  ["Azure Speech", "Speech translation", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/get-started-speech-translation", "2026-07-31", "Translation extension"],
  ["Azure Speech", "STT containers", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-container-stt", "2026-07-31", "Self-host/container"],
  ["Azure Speech", "Regions", "https://learn.microsoft.com/en-us/azure/ai-services/speech-service/regions", "2026-07-31", "Regional deployment"],
  ["Standards", "MRCPv2", "https://datatracker.ietf.org/doc/rfc6787/", "2026-07-31", "Legacy speech-resource control precedent"],
  ["Standards", "RTP", "https://www.rfc-editor.org/info/rfc3550/", "2026-07-31", "Real-time media framing precedent"],
  ["Standards", "Opus over RTP", "https://www.rfc-editor.org/info/rfc7587/", "2026-07-31", "Low-latency codec payload mapping"],
  ["Standards", "QUIC DATAGRAM", "https://www.rfc-editor.org/rfc/rfc9221.html", "2026-07-31", "Unreliable congestion-controlled datagrams without retransmission HOL"],
  ["Standards", "WebTransport over HTTP/3", "https://datatracker.ietf.org/wg/webtrans/documents/", "2026-07-31", "Browser-facing QUIC transport work"],
  ["Standards", "RTP over QUIC draft", "https://datatracker.ietf.org/doc/html/draft-ietf-avtcore-rtp-over-quic-12", "2026-07-31", "RTP/QUIC binding work"],
  ["Standards", "Media over QUIC WG", "https://datatracker.ietf.org/doc/charter-ietf-moq/01/", "2026-07-31", "Related real-time media standardization venue"],
];

const workbook = Workbook.create();
const summary = workbook.worksheets.add("Summary");
const matrix = workbook.worksheets.add("Feature Matrix");
const transport = workbook.worksheets.add("Transport Detail");
const proposed = workbook.worksheets.add("Proposed Standard");
const evidence = workbook.worksheets.add("Evidence");

const colors = {
  ink: "#111827",
  muted: "#4B5563",
  line: "#D1D5DB",
  header: "#E5E7EB",
  section: "#F3F4F6",
  green: "#DCFCE7",
  greenText: "#166534",
  amber: "#FEF3C7",
  amberText: "#92400E",
  red: "#FEE2E2",
  redText: "#991B1B",
  gray: "#F3F4F6",
  grayText: "#6B7280",
  blue: "#EFF6FF",
  blueText: "#1D4ED8",
};

function styleTitle(sheet, rangeAddress, title, subtitleRange, subtitle) {
  sheet.mergeCells(rangeAddress);
  const titleRange = sheet.getRange(rangeAddress);
  titleRange.values = [[title]];
  titleRange.format.font = { bold: true, size: 20, color: colors.ink };
  titleRange.format.fill = "#FFFFFF";
  titleRange.format.verticalAlignment = "center";
  titleRange.format.rowHeightPx = 38;
  sheet.mergeCells(subtitleRange);
  const sub = sheet.getRange(subtitleRange);
  sub.values = [[subtitle]];
  sub.format.font = { size: 10, color: colors.muted };
  sub.format.wrapText = true;
  sub.format.rowHeightPx = 34;
}

function styleHeader(range) {
  range.format.fill = colors.header;
  range.format.font = { bold: true, color: colors.ink };
  range.format.wrapText = true;
  range.format.verticalAlignment = "center";
  range.format.borders = { preset: "all", style: "thin", color: colors.line };
}

function styleBody(range) {
  range.format.font = { size: 9, color: colors.ink };
  range.format.wrapText = true;
  range.format.verticalAlignment = "top";
  range.format.borders = { preset: "all", style: "thin", color: colors.line };
}

// Summary
styleTitle(summary, "A1:H1", "ASR API Surface & Transport Landscape", "A2:H2", "Top 10 speech-to-text vendors • official public API documentation reviewed 2026-07-31 • scope: ASR/STT (not TTS)");
summary.getRange("A4:H4").values = [["Executive finding", "", "", "", "", "", "", ""]];
summary.mergeCells("A4:H4");
styleHeader(summary.getRange("A4:H4"));
summary.getRange("A5:H9").values = [
  ["1", "No reviewed vendor exposes a public QUIC/WebTransport ASR API; none exposes a raw UDP ASR API.", "", "", "", "", "", ""],
  ["2", "WebSocket is the specialist-vendor default. Google uses gRPC/HTTP/2; Azure primarily exposes an SDK-managed stream; OpenAI uniquely offers WebRTC and SIP in this set.", "", "", "", "", "", ""],
  ["3", "The portable semantic core is clear: session configuration, audio framing, partial/final/committed transcript states, VAD, silence endpointing, language, phrase biasing, timestamps, errors, and usage.", "", "", "", "", "", ""],
  ["4", "The least standardized areas are semantic end-of-turn, speculative/eager turns, reconnect/resume, dynamic configuration, diarization, translation, and redaction.", "", "", "", "", "", ""],
  ["5", "Recommended transport: reliable QUIC streams for control/results + QUIC DATAGRAM for loss-tolerant audio; WebSocket and RTP/SRTP remain compatibility bindings.", "", "", "", "", "", ""],
];
for (let r = 5; r <= 9; r++) summary.mergeCells(`B${r}:H${r}`);
styleBody(summary.getRange("A5:H9"));
summary.getRange("A5:A9").format.font = { bold: true, color: colors.blueText };
summary.getRange("A5:A9").format.horizontalAlignment = "center";
summary.getRange("B5:H9").format.fill = "#FFFFFF";
summary.getRange("A11:F11").values = [["Vendor", "Native", "Limited", "Explicit no", "Not documented", "Coverage score"]];
styleHeader(summary.getRange("A11:F11"));
const featureStart = 5;
const featureEnd = 4 + featureRows.length;
const vendorScoreRows = vendors.map((vendor, i) => {
  const col = String.fromCharCode("F".charCodeAt(0) + i);
  const refs = Array.from(
    { length: featureRows.length },
    (_, j) => `'Feature Matrix'!${col}${featureStart + j}`,
  );
  return [
    vendor,
    statusCountFormula(refs, "✓"),
    statusCountFormula(refs, "△"),
    statusCountFormula(refs, "✕"),
    `=${featureRows.length}-B${12 + i}-C${12 + i}-D${12 + i}`,
    `=(B${12 + i}+0.5*C${12 + i})/${featureRows.length}`,
  ];
});
summary.getRange(`A12:A${11 + vendors.length}`).values = vendorScoreRows.map((r) => [r[0]]);
summary.getRange(`B12:F${11 + vendors.length}`).formulas = vendorScoreRows.map((r) => r.slice(1));
styleBody(summary.getRange(`A12:F${11 + vendors.length}`));
summary.getRange(`F12:F${11 + vendors.length}`).setNumberFormat("0%");
summary.getRange("H11:J11").values = [["Legend", "Meaning", "Interpretation"]];
styleHeader(summary.getRange("H11:J11"));
summary.getRange("H12:J15").values = [
  ["✓ Native", "Documented directly in the public ASR surface", "Counts as native"],
  ["△ Limited", "Adjacent, batch-only, model-dependent, SDK-only, or narrower", "Counts half in coverage score"],
  ["✕ Explicitly unsupported", "Vendor documentation explicitly says it is unavailable", "Does not imply permanent product direction"],
  ["— Not documented", "No public support found in reviewed official docs", "Not the same as explicit unsupported"],
];
styleBody(summary.getRange("H12:J15"));
summary.getRange("H12:H12").format.fill = colors.green;
summary.getRange("H13:H13").format.fill = colors.amber;
summary.getRange("H14:H14").format.fill = colors.red;
summary.getRange("H15:H15").format.fill = colors.gray;
summary.getRange("H17:J17").values = [["Consortium sequence", "Deliverable", "Why now"]];
styleHeader(summary.getRange("H17:J17"));
summary.getRange("H18:J21").values = [
  ["0.1 — Semantic profile", "Vendor-neutral JSON/Protobuf event and control model over WebSocket", "Can prototype immediately against most vendors"],
  ["0.2 — Conformance", "Golden traces, event-state machine, error taxonomy, codec fixtures", "Prevents nominal compatibility with divergent semantics"],
  ["0.3 — QUIC binding", "Reliable control/result streams + QUIC DATAGRAM audio", "Addresses TCP head-of-line delay without raw-UDP tradeoffs"],
  ["0.4 — Telephony binding", "RTP/SRTP and SIP/MRCP interworking profile", "Connects PSTN/CCaaS deployments and existing media infrastructure"],
];
styleBody(summary.getRange("H18:J21"));
summary.getRange("A24:J27").values = [
  ["Method", "This is a documentation-based interoperability map, not a latency/accuracy benchmark. “Native” requires documented support in the provider's public ASR/STT surface; adjacent agent/translation products are marked limited.", "", "", "", "", "", "", "", ""],
  ["Freshness", "API surfaces change quickly. Evidence links and review date are in the Evidence tab; verify before procurement or protocol commitments.", "", "", "", "", "", "", "", ""],
  ["Transport thesis", "Raw UDP is intentionally not recommended: it lacks standardized security, congestion control, browser reach, and NAT behavior. QUIC DATAGRAM supplies loss-tolerant media while retaining TLS 1.3, congestion control, connection migration, and multiplexed reliable streams.", "", "", "", "", "", "", "", ""],
  ["Reading order", "Start with Feature Matrix, then Transport Detail, then Proposed Standard. Evidence is the audit trail.", "", "", "", "", "", "", "", ""],
];
for (let r = 24; r <= 27; r++) summary.mergeCells(`B${r}:J${r}`);
styleBody(summary.getRange("A24:J27"));
summary.getRange("A24:A27").format.font = { bold: true, color: colors.ink };
summary.freezePanes.freezeRows(2);
summary.showGridLines = false;
summary.getRange("A1:A27").format.columnWidthPx = 125;
summary.getRange("B1:G27").format.columnWidthPx = 82;
summary.getRange("H1:H27").format.columnWidthPx = 155;
summary.getRange("I1:I27").format.columnWidthPx = 200;
summary.getRange("J1:J27").format.columnWidthPx = 205;
summary.getRange("A5:J27").format.rowHeightPx = 36;

// Feature Matrix
styleTitle(matrix, "A1:O1", "Feature Matrix", "A2:O2", "Vendors in columns; features and controls in rows. Cells state both support level and implementation mechanism. Counts are formula-driven.");
matrix.getRange("A3:O3").values = [["Legend: ✓ Native", "△ Limited/adjacent", "✕ Explicitly unsupported", "— Not documented", "", "", "", "", "", "", "", "", "", "", ""]];
matrix.mergeCells("D3:O3");
matrix.getRange("A3").format.fill = colors.green;
matrix.getRange("B3").format.fill = colors.amber;
matrix.getRange("C3").format.fill = colors.red;
matrix.getRange("D3:O3").format.fill = colors.gray;
matrix.getRange("A3:O3").format.font = { bold: true, size: 9, color: colors.ink };
matrix.getRange("A4:O4").values = [["Category", "Feature / control", "Proposed profile", "Native count", "Limited count", ...vendors]];
styleHeader(matrix.getRange("A4:O4"));
matrix.getRange(`A5:C${featureEnd}`).values = featureRows.map((r) => r.slice(0, 3));
matrix.getRange(`F5:O${featureEnd}`).values = featureRows.map((r) => r.slice(3));
matrix.getRange(`D5:E${featureEnd}`).formulas = featureRows.map((_, i) => {
  const row = 5 + i;
  const refs = vendors.map((__, j) => `${String.fromCharCode("F".charCodeAt(0) + j)}${row}`);
  return [statusCountFormula(refs, "✓"), statusCountFormula(refs, "△")];
});
styleBody(matrix.getRange(`A5:O${featureEnd}`));
matrix.getRange(`A5:A${featureEnd}`).format.font = { bold: true, size: 9, color: colors.muted };
matrix.getRange(`C5:C${featureEnd}`).format.fill = colors.blue;
matrix.getRange(`D5:E${featureEnd}`).format.horizontalAlignment = "center";
matrix.getRange(`D5:E${featureEnd}`).format.font = { bold: true, size: 9, color: colors.ink };
const vendorGrid = matrix.getRange(`F5:O${featureEnd}`);
vendorGrid.conditionalFormats.add("beginsWith", { text: "✓", format: { fill: colors.green, font: { color: colors.greenText } } });
vendorGrid.conditionalFormats.add("beginsWith", { text: "△", format: { fill: colors.amber, font: { color: colors.amberText } } });
vendorGrid.conditionalFormats.add("beginsWith", { text: "✕", format: { fill: colors.red, font: { color: colors.redText } } });
vendorGrid.conditionalFormats.add("beginsWith", { text: "—", format: { fill: colors.gray, font: { color: colors.grayText } } });
let priorCategory = "";
for (let i = 0; i < featureRows.length; i++) {
  const category = featureRows[i][0];
  if (category !== priorCategory) {
    const row = 5 + i;
    matrix.getRange(`A${row}:O${row}`).format.borders = {
      top: { style: "medium", color: "#9CA3AF" },
      bottom: { style: "thin", color: colors.line },
      insideVertical: { style: "thin", color: colors.line },
    };
    priorCategory = category;
  }
}
matrix.freezePanes.freezeRows(4);
matrix.freezePanes.freezeColumns(5);
matrix.showGridLines = false;
matrix.getRange(`A1:A${featureEnd}`).format.columnWidthPx = 145;
matrix.getRange(`B1:B${featureEnd}`).format.columnWidthPx = 205;
matrix.getRange(`C1:C${featureEnd}`).format.columnWidthPx = 125;
matrix.getRange(`D1:E${featureEnd}`).format.columnWidthPx = 78;
matrix.getRange(`F1:O${featureEnd}`).format.columnWidthPx = 225;
matrix.getRange(`A5:O${featureEnd}`).format.rowHeightPx = 48;
matrix.getRange("A1:O4").format.rowHeightPx = 38;

// Transport detail
styleTitle(transport, "A1:J1", "Transport Detail", "A2:J2", "How realtime audio, control, results, authentication, loss behavior, reconnect, and telephony integration differ by provider.");
transport.getRange("A4:J4").values = [["Provider", "Primary realtime surface", "Transport", "Audio framing", "Control/results framing", "Authentication", "HOL / loss behavior", "Reconnect semantics", "Telephony path", "Standardization implication"]];
styleHeader(transport.getRange("A4:J4"));
transport.getRange("A5:J14").values = transportRows;
styleBody(transport.getRange("A5:J14"));
transport.getRange("A5:A14").format.font = { bold: true, size: 9, color: colors.ink };
transport.getRange("A16:J16").values = [["Observed market pattern", "8 vendors expose public WebSocket realtime APIs; Google exposes public gRPC; Azure abstracts its service stream behind SDKs. OpenAI also exposes WebRTC and SIP. No reviewed vendor exposes QUIC/WebTransport or raw UDP as a public ASR interface.", "", "", "", "", "", "", "", ""]];
transport.mergeCells("B16:J16");
styleBody(transport.getRange("A16:J16"));
transport.getRange("A16").format.font = { bold: true, color: colors.ink };
transport.getRange("A16:J16").format.fill = colors.blue;
transport.freezePanes.freezeRows(4);
transport.freezePanes.freezeColumns(1);
transport.showGridLines = false;
transport.getRange("A1:A16").format.columnWidthPx = 120;
transport.getRange("B1:C16").format.columnWidthPx = 190;
transport.getRange("D1:F16").format.columnWidthPx = 185;
transport.getRange("G1:J16").format.columnWidthPx = 210;
transport.getRange("A5:J16").format.rowHeightPx = 64;

// Proposed standard
styleTitle(proposed, "A1:I1", "Proposed Voice AI ASR Profile", "A2:I2", "Working consortium draft: transport-neutral semantics with QUIC/WebTransport as the primary low-latency binding and WebSocket + RTP/SRTP compatibility bindings.");
proposed.getRange("A4:I4").values = [["Layer", "Message / event", "Direction", "Reliability", "QUIC mapping", "WebSocket fallback", "Telephony mapping", "Required fields", "Notes"]];
styleHeader(proposed.getRange("A4:I4"));
const proposedEnd = 4 + proposedRows.length;
proposed.getRange(`A5:I${proposedEnd}`).values = proposedRows;
styleBody(proposed.getRange(`A5:I${proposedEnd}`));
proposed.getRange(`A5:A${proposedEnd}`).format.font = { bold: true, size: 9, color: colors.muted };
proposed.getRange(`B5:B${proposedEnd}`).format.font = { bold: true, size: 9, color: colors.blueText };
proposed.getRange(`E5:E${proposedEnd}`).format.fill = colors.blue;
proposed.getRange(`F5:F${proposedEnd}`).format.fill = colors.gray;
const principlesStart = proposedEnd + 2;
proposed.getRange(`A${principlesStart}:I${principlesStart}`).values = [["Design principle", "Requirement", "Rationale", "", "", "", "", "", ""]];
proposed.mergeCells(`C${principlesStart}:I${principlesStart}`);
styleHeader(proposed.getRange(`A${principlesStart}:I${principlesStart}`));
const principles = [
  ["No raw UDP profile", "Use QUIC DATAGRAM or RTP/SRTP, not bespoke UDP.", "Security, congestion control, NAT traversal, browser compatibility, and operations all need standardized behavior."],
  ["Split media from state", "Audio frames may be lost; session controls and transcript state may not.", "QUIC DATAGRAM removes retransmission delay for stale audio while reliable streams protect semantic state."],
  ["One state machine", "Define partial → committed → final and turn start → eager end → resume/end precisely.", "Most current integration bugs are semantic mismatch, not JSON syntax."],
  ["Monotonic sequencing", "Every audio datagram has seq + media_timestamp; every transcript item has revision.", "Enables loss accounting, deduplication, reordering tolerance, and deterministic conformance tests."],
  ["Codec epochs", "Configuration changes create a new codec_epoch referenced by following frames.", "Avoids ambiguous midstream sample-rate/channel/codec transitions."],
  ["MTU discipline", "Audio datagrams must fit the negotiated path MTU; fragmentation is forbidden at the profile layer.", "Prevents large-frame loss amplification and unpredictable latency."],
  ["Extensibility", "Unknown fields ignored; vendor extensions use reverse-domain namespaces and negotiated capabilities.", "Lets vendors innovate without forking the portable core."],
  ["Conformance before governance", "Publish schema, golden traces, emulator, and two-provider adapters before formal SDO submission.", "Interoperable running code gives a consortium credible scope and evidence."],
];
const pStart = principlesStart + 1;
const pEnd = pStart + principles.length - 1;
proposed.getRange(`A${pStart}:C${pEnd}`).values = principles;
for (let r = pStart; r <= pEnd; r++) proposed.mergeCells(`C${r}:I${r}`);
styleBody(proposed.getRange(`A${pStart}:I${pEnd}`));
proposed.getRange(`A${pStart}:A${pEnd}`).format.font = { bold: true, color: colors.ink };
proposed.freezePanes.freezeRows(4);
proposed.freezePanes.freezeColumns(2);
proposed.showGridLines = false;
proposed.getRange(`A1:A${pEnd}`).format.columnWidthPx = 120;
proposed.getRange(`B1:B${pEnd}`).format.columnWidthPx = 190;
proposed.getRange(`C1:D${pEnd}`).format.columnWidthPx = 145;
proposed.getRange(`E1:G${pEnd}`).format.columnWidthPx = 190;
proposed.getRange(`H1:H${pEnd}`).format.columnWidthPx = 240;
proposed.getRange(`I1:I${pEnd}`).format.columnWidthPx = 250;
proposed.getRange(`A5:I${pEnd}`).format.rowHeightPx = 58;

// Evidence
styleTitle(evidence, "A1:E1", "Evidence", "A2:E2", "Official documentation used for the matrix. Accessed 2026-07-31. URLs are intentionally visible for auditability.");
evidence.getRange("A4:E4").values = [["Provider / body", "Topic", "Official source URL", "Accessed", "Supports"]];
styleHeader(evidence.getRange("A4:E4"));
const evidenceEnd = 4 + evidenceRows.length;
evidence.getRange(`A5:E${evidenceEnd}`).values = evidenceRows;
styleBody(evidence.getRange(`A5:E${evidenceEnd}`));
evidence.getRange(`A5:A${evidenceEnd}`).format.font = { bold: true, size: 9, color: colors.ink };
evidence.getRange(`C5:C${evidenceEnd}`).format.font = { size: 9, color: colors.blueText, underline: true };
evidence.getRange(`D5:D${evidenceEnd}`).setNumberFormat("yyyy-mm-dd");
evidence.freezePanes.freezeRows(4);
evidence.freezePanes.freezeColumns(2);
evidence.showGridLines = false;
evidence.getRange(`A1:A${evidenceEnd}`).format.columnWidthPx = 135;
evidence.getRange(`B1:B${evidenceEnd}`).format.columnWidthPx = 200;
evidence.getRange(`C1:C${evidenceEnd}`).format.columnWidthPx = 520;
evidence.getRange(`D1:D${evidenceEnd}`).format.columnWidthPx = 105;
evidence.getRange(`E1:E${evidenceEnd}`).format.columnWidthPx = 235;
evidence.getRange(`A5:E${evidenceEnd}`).format.rowHeightPx = 36;

// Workbook checks and previews
const xlsx = await SpreadsheetFile.exportXlsx(workbook);
await xlsx.save(outputFile);

const previews = [
  ["Summary", "summary.png", 0.9],
  ["Feature Matrix", "feature_matrix.png", 0.42],
  ["Transport Detail", "transport_detail.png", 0.65],
  ["Proposed Standard", "proposed_standard.png", 0.55],
  ["Evidence", "evidence.png", 0.55],
];
for (const [sheetName, filename, scale] of previews) {
  const blob = await workbook.render({ sheetName, autoCrop: "all", scale, format: "png" });
  await fs.writeFile(`${outputDir}/${filename}`, new Uint8Array(await blob.arrayBuffer()));
}

const formulaScan = await workbook.inspect({
  kind: "match",
  searchTerm: "#REF!|#DIV/0!|#VALUE!|#NAME\\?|#N/A",
  options: { useRegex: true, maxResults: 100 },
  summary: "formula error scan",
  maxChars: 5000,
});
const matrixInspect = await workbook.inspect({
  kind: "sheet",
  sheetId: "Feature Matrix",
  range: `A1:O${Math.min(featureEnd, 14)}`,
  include: "values,formulas",
  maxChars: 5000,
});
console.log(JSON.stringify({
  outputFile,
  sheets: ["Summary", "Feature Matrix", "Transport Detail", "Proposed Standard", "Evidence"],
  featureCount: featureRows.length,
  evidenceCount: evidenceRows.length,
  formulaScan: formulaScan.ndjson,
  matrixInspect: matrixInspect.ndjson,
}, null, 2));
