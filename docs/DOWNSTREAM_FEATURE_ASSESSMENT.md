# Downstream feature assessment

Reviewed 2026-09-24 against origin/main `c69a427d` (0.3.10).
The local integration branch also preserves the three existing Parley commits
and the pre-existing uncommitted WebSocket media bridge change.

Baseline verification: `cargo build --locked --workspace` passed on the local
integration branch. This covers workspace default features, not all optional
feature combinations or downstream regression/interop tests. `git diff --check`
also passed. GitHub CLI active identity was switched to `eisenzopf` before fetch.

## Recommended library changes

| Capability | Source | Destination and rationale |
|---|---|---|
| Recording route drain and retained delivery receipts | Thelve `vendor/rvoip-core/src/media_graph.rs` | Core: opt-in recording queues must drain on EOF/stop, bound blocked consumers, and account for undelivered frames. Channel delivery is not proof of durable recording storage. |
| Generation-qualified peer handoff | Thelve core, core-traits, SIP, QUIC, Vapi; vapi-central core | Shared traits and core with transport implementations: stage replacements silently, quiesce old sends, commit ownership, reject stale frames, retain the caller, and emit a typed handoff receipt/event. Reconcile the two designs rather than expose competing mechanisms. |
| In-process AI adapter and acknowledged lifecycle | `rvoip-vapi-central/crates/extensions/rvoip-harness` | Harness extension: bounded PCM boundary, dormant preparation, provider-acknowledged hold/resume, stable AI session identity, generation-qualified rebind and deterministic cleanup. Provider/business implementations remain downstream. |
| Exact SIP/WebRTC activation, rejection and teardown | `rvoip-vapi-central` core/SIP/WebRTC changes | Transport-neutral lifecycle guarantees, bounded cleanup, media/data fencing, and exact WebRTC close observation belong in the library. Port regression tests with the changes. |
| Existing Vapi call attachment and call-local credentials | Thelve `vendor/rvoip-vapi` | Vapi extension: attach a verified existing call without another create request; isolate credentials per call; shared lifecycle observation and handoff-aware termination. Host receipt authorization stays in the application. |
| Inbound session-scoped ordered codec policy | Thelve SIP/core admission changes | Pin codec allowlist/preference before the first answer; preserve it for re-offers without leaking across session reuse. Preserve payload mappings and security negotiation. |
| Completed and deduplicated DTMF output | Thelve `vendor/rvoip-sip/src/media_stream.rs` | Accept valid nonzero initial durations, suppress repeat/old events, and hold delivery ownership until the tone completes. Cancellation must not falsely claim an already spawned tone stopped. |
| Generation-aware bounded outbound audio queue | vapi-ref-harness SIP override and `crates/audio-send-queue` | Library-owned primitive and SIP API: flush stale speech on interruption, expose queue/pump metrics and typed overload. Make capacity/policy configurable; do not hardcode the application's nine-frame capacity as a universal default. |
| Bounded RTP reordering and paced decoded delivery | vapi-ref-harness media-core override | Media layer: reject duplicates/late packets, conceal confirmed gaps, preserve talkspurt pauses, and reset ordering state on SSRC changes. Reconcile with existing jitter-buffer configuration so two independent buffers do not add latency. |
| G.729 speech-plus-SID receive handling | vapi-ref-harness media-core override | Codec runtime: support speech aggregates with a terminal SID and reject malformed remainders. The fork's Annex-A-only transmit workaround needs an explicit policy/design; it does not implement Annex B DTX transmission. |
| Codec framing and G.729 RTP packetization | vapi-central media/core changes | Preserve RTP timing across suppressed Annex B no-data intervals, mark new talkspurts, frame short PCM chunks for Opus, discard partial target frames on timestamp discontinuities, and select negotiated dynamic payload codecs correctly. Reconcile the G.729 packetizer with the harness workaround instead of silently disabling DTX. |
| Accept/CANCEL race transition | vapi-ref-harness `vendor/rvoip-sip-state-table/default.yaml` | Default SIP state table: accept authoritative `DialogCANCEL` while Answering, release resources and publish cancellation. This is a separate override, not the table inside its vendored SIP crate. |
| Exact retired-BYE event handling | vapi-ref-harness sip-dialog override | Classify a proven terminal BYE cleanup race as benign while retaining other errors. Tighten any broad routing-error classification against authoritative cleanup evidence. |
| Registration outbound-proxy propagation | vapi-ref-harness SIP override | Thread builder proxy selection into the initial request and retain it across refresh/unregister on that registration only. The current builder drops the configured value. |
| Read-only inbound offer SDP | vapi-ref-harness SIP `api/endpoint.rs` | Small Endpoint incoming-call accessor; useful for callers that need offer inspection before answering. |
| Safe structured SIP diagnostics | Thelve `response_diagnostics.rs` | Reusable redacted status/cause/correlation observations; keep carrier-specific header selection configurable rather than hardcode Thelve's projection. |

## Avoid duplicate or inappropriate imports

- vapi-central points to `../rvoip-vapi-central`, a clean checkout of
  `codex/vapi-central-transport-neutral` at `c18c932e`. Its history includes
  codec wiring and PCM-to-Opus framing changes. Compare each hunk with 0.3.10:
  branch-only commit IDs do not establish that functionality is missing.
- Preserve existing Parley conversation-role/UCTP dispatch and SIP source-budget
  work. Older vendor snapshots omit some of it; copying whole source trees would
  remove local functionality.
- Do not import registry-normalized manifests, vendoring fixture paths, old
  dependency pins, deployment configs, carrier credentials, application tenant
  policy, provider runtimes, evidence publishing, or harness infrastructure.
- Thelve's fork notes contain historical statements about unfinished work;
  current source includes strict handoff entry points and transport queues.
  Their presence does not establish live carrier acceptance.
- A transport fence covers the library's submission boundary. It cannot recall
  packets already accepted by Quinn/the OS or audio already played remotely.

## Implementation order and verification

1. Small independent SIP/codec corrections: registration proxy, offer accessor,
   CANCEL race, BYE cleanup, G.729 parsing and DTMF regressions.
2. Recording drain and configurable playout/audio-queue primitives, with
   EOF/backpressure/overflow/interruption and SSRC-change tests.
3. Reconcile handoff ownership and transport-neutral lifecycle, then implement
   SIP/QUIC/Vapi fences and the in-process AI adapter with bidirectional media,
   failure rollback, stale-generation, provider-acknowledgement and teardown tests.
4. Add inbound codec policy, Vapi existing-call attachment and safe diagnostics;
   verify feature-enabled builds and focused integration fixtures.

This is the pre-implementation assessment requested by the user. Downstream
features have not been imported as part of the baseline synchronization.
