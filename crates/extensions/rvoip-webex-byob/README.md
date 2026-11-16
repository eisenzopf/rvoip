# rvoip-webex-byob

Provider-neutral Webex Contact Center BYOVA protocol engines (WebSocket and optional gRPC) for rvoip 0.4.

Experimental library integration: this crate can be used as a workspace/path or
Git dependency, but does not yet register an rvoip `ConnectionAdapter` or expose
a facade feature. Crates.io publishing is disabled while redistribution terms
for the vendored Cisco protobuf schemas are unresolved. Authentication, network
servers, provider credentials and deployment remain host responsibilities.

This prototype parses Webex JSON requests and returns typed `Input` values. One owner holds `Session`, passes authenticated org/agent bindings, serializes returned response batches in order, and supplies authoritative input/reply boundaries. The crate does not do network I/O, authenticate upgrades, infer voice activity or depend on Vapi.

After accepting SESSION_START, call `begin_reply`, feed raw 8 kHz PCMU through `audio`, and call `finish` exactly once. `ReplyId` rejects stale audio after finality or interruption. `interrupt` discards pending tail audio and returns START_OF_INPUT followed by FINAL for an active reply. `terminal` ends the session. `is_ended` lets the host close the transport promptly.

`start_parameters()` exposes optional application data captured only from a
validated SESSION_START: an object or null, bounded to 16 KiB. Duplicate starts
cannot replace it. The host must validate its own business schema and bind any
external tool result to this conversation. `complete_with_metadata(name, object)`
returns one FINAL containing a CUSTOM_EVENT and SESSION_END, both with the result
in `metadata`. A retained audio tail shares that FINAL. The event name is bounded
to 128 bytes and the metadata to 16 KiB; repeated completion is rejected. This is
a terminal result, not an arbitrary mid-call messaging channel.

Output chunks are 320–32,000 bytes. A 320-byte lookbehind preserves arbitrary input partition boundaries without padding. An entire reply smaller than 320 bytes returns ShortAudio. The host owns backpressure and must not pace output for playback: Webex performs pacing.

The opt-in WAV compatibility path aggregates 8,000 PCMU samples per streaming
prompt (one second, 16,044 PCM16 WAV bytes), retaining the minimum tail for FINAL.
Short replies and the final remainder flush on completion; interruption discards
the pending tail. This avoids wrapping every small provider frame as a separate
playback prompt. Raw output keeps its existing chunking policy.

No full ConnectionAdapter registration exists yet. The intended adapter contract, known upstream schema contradictions and qualification gates are in `docs/WEBEX_BYOB_IMPLEMENTATION_SPEC.md` at the workspace root. The sibling `webex-vapi-connector` repository contains a private Axum host and independent simulator. Passing its simulator does not prove Cisco conformance.

Run `cargo test -p rvoip-webex-byob` from the workspace root.
The crate-level Rustdoc includes a runnable session/reply example.
Default builds do not compile protobuf tooling; it is enabled only by `grpc`.


Enable `features = ["grpc"]` for Cisco protobuf types, request validation,
ordered DTMF decoding, the 100–64,000-byte gRPC chunker, and bounded whole-WAV
responses that decode PCMU to mono 8 kHz PCM16 with rvoip codec-core. The host supplies
transport, authentication and conversation ownership; gRPC uses a new RPC per
interaction. See section 20 of the implementation specification for this distinct
lifecycle and the sibling application's `docs/GRPC.md` for running both simulators.
Run `cargo test -p rvoip-webex-byob --features grpc` for both wire variants.

`AudioOutput::Pcm16WavWhole` is an explicit deployment override for Cisco-approved
whole-reply output. It emits no audio until finish, then one WAV on FINAL.
Both this mode and gRPC `WavReply` are bounded to 60 seconds (480,000 PCMU input
samples); oversize replies fail without truncation or splitting. Whole WebSocket
prompts exceed the public raw-profile chunk ceiling and require deployment
qualification. Buffering increases generation-to-playback latency.

`grpc::WavChunkReply` provides an opt-in streaming WAV profile: each 8,000
PCMU samples produces one PCM16/8 kHz WAV CHUNK immediately; finish flushes
the remaining samples without padding and emits one empty FINAL. This keeps
less than one second of pending samples between provider frames. Qualify WAV
CHUNK playback against the target Cisco deployment before enabling it.

`grpc::ChunkReply::one_second()` aggregates headerless PCMU/8 kHz into
8,000-byte CHUNKs, retaining a 100-byte minimum tail. This follows Cisco's
published chunked-media profile. The separate `WavChunkReply` experiment was
silent in live gRPC testing on November 2; do not assume WebSocket WAV
compatibility applies to gRPC. See the connector's streaming validation report.

`AudioOutput::Pcm16WavFastStart` emits the first legal audio prompt as soon as
640 input samples are available (a 320-sample prompt plus retained tail), then
aggregates subsequent prompts. `with_answer_audio_output` can apply that mode
after the first reply, preserving the greeting's original buffering. This
reduces connector buffering; it cannot guarantee when Cisco starts playback.

For review, run the default and optional profiles independently:

```sh
cargo test --locked -p rvoip-webex-byob
cargo test --locked -p rvoip-webex-byob --features wav-compat
cargo test --locked -p rvoip-webex-byob --all-features
cargo clippy --locked -p rvoip-webex-byob --all-features --all-targets -- -D warnings
cargo test --locked -p rvoip-vapi
```
