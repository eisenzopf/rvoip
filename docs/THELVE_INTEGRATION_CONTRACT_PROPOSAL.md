# Downstream integration contracts: proposal for review

Status: design proposal; no runtime behavior changes. Based on integration work
in Thelve and source review of rvoip 0.3.12. The proposals below supplement the
existing issue register rather than reopening implemented features.

## Shared conformance fixtures

Related: [testkit #116](https://github.com/eisenzopf/rvoip/issues/116).

Downstream applications repeatedly implement adapter and stream doubles for
activation, cancellation, handoff, recording drain and media negotiation.
Different fixtures can accidentally test different interpretations of completion.
A dev-only testkit should provide controllable sources, bounded sinks, exact
lifecycle identities, deterministic time and observable task completion.

Start with a small suite usable by third-party ConnectionAdapter and MediaStream
implementations; do not require application database schemas or provider accounts.

| Fixture | Required assertion |
| --- | --- |
| Variable packet time | Encode/decode 2.5/5/10/20/40/60/120 ms Opus; preserve sample order and per-channel RTP time; retain less than one encoder frame |
| Source switch | Independent forward/backward SSRC epochs, overlap, old-source return, sequence/timestamp wrap, invalid replacement decode and unchanged transmit encoder |
| Media quality | Nonzero decoded tone energy, known frequency and duration; packet counts alone do not prove useful audio |
| Codec negotiation | Dynamic payload types, channel/rate/fmtp identity, generation changes and unavailable-codec refusal |
| Handoff | Staged audio stays silent, old blocked sends are fenced, exactly one owner commits, transport admission and remote playback have separate claims |
| Recording drain | Offered/delivered/dropped counts converge, blocked sink deadlines fail, source EOF and explicit drain preserve complete artifacts |
| Teardown | Cancellation, late/duplicate terminal events, hung adapter, closed receiver, no detached task or false success |
| Diagnostics | Credential/content canaries, bounded projection and explicit observation-loss counters |

The first implementation should replace representative fixtures in core, SIP,
WebRTC and one downstream consumer. Existing real UDP/TLS fixtures remain
necessary: fake conformance does not prove wire interoperability or browser
playback. Feature-gated codec cases need an explicit CI gate that enables them.
Every fixture should expose a join or task-leak assertion, not sleep and assume
that a task stopped.

Decisions: fixture crate ownership, which traits are stable enough to share,
whether conformance is a reusable test function or macro, and how to expose
cancellation and failpoints without production coupling.

## Completion stages for shutdown, recording and handoff

Related: [recording #99](https://github.com/eisenzopf/rvoip/issues/99),
[Session shutdown #269](https://github.com/eisenzopf/rvoip/issues/269),
[AI teardown #104](https://github.com/eisenzopf/rvoip/issues/104), and
[peer handoff #92](https://github.com/eisenzopf/rvoip/issues/92).

Applications need to know which boundary an acknowledgment proves. A frame
accepted into a graph queue, accepted by a transport, persisted by a recording
sink and heard by a remote endpoint are different outcomes. Make the documented
boundary part of each receipt or completion type.

Proposed vocabulary, subject to API review:

| Stage | Evidence | Does not by itself prove |
| --- | --- | --- |
| Accepted | Command admitted to its exact lifecycle | Execution or media delivery |
| OwnershipCommitted | The selected graph/bridge owner changed | Old transport buffers or remote playback stopped |
| LocalMediaQuiesced | Promised local queues/pumps completed or fenced | Remote termination |
| TransportCompleted | Adapter's defined signaling/send boundary completed | Human audibility or durable storage |
| ArtifactPersisted | Sink write/close and application durability succeeded | Completeness unless loss accounting also settled |
| RemoteConfirmed | A defined remote acknowledgment was observed | Completion beyond that protocol's guarantee |
| Failed / Uncertain | Bounded stage/reason and affected resources retained | A safe blind retry |

A bounded Session shutdown should fence new admissions and prepared activation,
attempt every owned Connection despite individual failures, reconcile duplicate
shutdown, and retain failed/uncertain cleanup for inspection. Recording/vCon
finalization must use the agreed cutoff and distinguish complete from partial
artifacts. A strict pause needs an acknowledged media boundary; buffered tail
handling must be explicit. Keep existing APIs compatible through additive
operations or clearly versioned semantics.

Acceptance: concurrent admission/shutdown, one hung adapter, cancellation after
partial work, late terminal callbacks, peer replacement during drain, blocked
storage, and retry of the same operation. Assert promised stages and exact frame
counts; do not infer success from a command enqueue or timeout alone.

Decisions: which stages are queryable, receipt retention, cutoff semantics,
default deadlines, idempotency/content-conflict behavior, and owner responsibility
for persistence. This proposal does not claim these APIs already implement the
full contract.

## Recoverable provider operations

Related: [Vapi creation #103](https://github.com/eisenzopf/rvoip/issues/103) and
[durable orchestration #112](https://github.com/eisenzopf/rvoip/issues/112).

Existing-call attachment is a useful primitive: the host can verify a recovered
provider call and attach it without another create request. It does not define
a complete crash-safe create/reconcile protocol.

Propose optional host-owned contracts rather than a database dependency:

1. Freeze a caller operation ID and canonical request identity before dispatch.
2. Persist the dispatch state before an external effect may occur.
3. Expose a redacted provider reference and an explicit accepted/failed/uncertain
   result. A timeout after a possible provider effect is uncertain.
4. Reconcile by exact provider reference or provider-supported idempotency key;
   absence of usable creation credentials must not force a new create request.
5. Attach only a verified current result. Attachment failure must not silently
   fall back to creation.
6. Publish terminal receipts idempotently; reject same-ID requests with different
   content. Define retention and process-incarnation behavior.

The host remains responsible for authorization, tenant isolation, credential
resolution and durable storage. rvoip owns exact transport/lifecycle identities
and honest effect boundaries. No general exactly-once promise is proposed for a
provider that supplies neither idempotency nor authoritative reconciliation.

Acceptance: cancellation before/after dispatch, provider timeout after create,
receipt-write failure, process restart, lost response, duplicate request,
content conflict, revoked authority, unavailable credentials during receipt
recovery, and attachment failure. A retry must either recover the verified
existing call, report a definite no-effect refusal, or preserve uncertainty.

Decisions: portable receipt shape, provider capabilities, recovery hooks,
which lifecycle queries survive finalization, and whether these contracts live
in core traits or a separate optional integration crate.
