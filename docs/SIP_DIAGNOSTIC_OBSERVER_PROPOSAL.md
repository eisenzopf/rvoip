# Application-scoped SIP diagnostic observation

Status: design proposal for review; no production observer is installed by this PR.

## Integration finding

Thelve needed observation at the decrypted SIP boundary to correlate incoming
setup refusal, transmitted responses and in-dialog teardown with application
call records. The downstream prototype exposes message, rejection and optional
reference callbacks. Application capture is bounded, time-limited, uses
non-blocking queues and records dropped observations separately from call state.

The prototype uses a process-global `OnceLock<Arc<dyn DiagnosticObserver>>`.
That prevents separate endpoints from owning independent observers and provides
no replacement or teardown. Submitting that lifetime model as a default public
library contract would force every embedding application to inherit it.
This PR proposes the reviewable contract before choosing runtime plumbing.

## Proposed boundary

An endpoint/transaction-manager builder owns an optional observer handle.
Installing or replacing it must not affect another endpoint. Dropping its owner
ends future observation; define the disposition of already queued observations.
No callback changes SIP routing, admission, TLS/SRTP policy or causal event
ownership. Observation loss is separate from protocol failure.

Prefer a typed bounded subscription over synchronous arbitrary application
callbacks on signaling tasks. A minimal conceptual record contains direction,
actual transport kind, local/peer addresses, a bounded transaction correlation
reference, timestamp and a fixed event kind. Setup failures use a fixed class
and allowlisted reason; arbitrary errors must not be formatted.

Default records contain metadata only. Raw decrypted SIP messages require a
separate explicit opt-in and host authorization/capture lifetime. They can contain
credentials, caller information and SDP; generic logging must not render them.
Raw byte and parsed-message representations must have defined size bounds,
redacted Debug behavior and clear ownership. The host owns retention and tenant
policy. A diagnostic reference is correlation, never call authority.

Observe received messages after transport attribution and accepted writes after
the send succeeds. Preserve the distinction between local send acceptance and
remote receipt. Do not reparse/clone payloads when no observer is installed.
Retransmissions and multiple transport send paths need explicitly documented
coverage; do not present a best-effort subscription as a complete wire capture.

Existing transaction observers preserve causal event ownership and already use
bounded fanout. Evaluate extending those mechanisms rather than creating a
second dispatch topology. Their current payload/lifetime boundaries need review
before treating them as the decrypted-message capture API.

## Decisions requested

- Builder scope: endpoint, manager, transport, or shared explicitly named owner.
- Typed queue versus callbacks, record representation and capacity limits.
- Overflow counters, gap records and whether coverage loss is sticky per capture.
- Install/replace/drop semantics under simultaneous receive/send and shutdown.
- Default correlation redaction and separate raw-capture capability.
- Coverage of incoming refusals, raw responses, retransmissions, proxy sends,
  target refreshes, TLS flow reuse and malformed messages.
- Compatibility and release shape for configuration structs.

## Acceptance

Run two independent endpoints and prove observer isolation. Saturate one observer
without blocking signaling or the other observer. Drop/replace an observer during
TLS send and teardown. Verify actual ingress transport cannot be spoofed by Via.
Check credential/content canaries in Debug, logs and metadata projections.
Assert byte/queue bounds and explicit drop evidence. Cover rejected initial
INVITEs, successful setup, local and remote BYE, retransmission, target refresh
and closed TLS flow. Every observer task must have bounded join/cleanup.

This proposal supplements the small setup-failure diagnostics fix. It does not
provide a complete CDR service, mandatory persistence or a new protocol gate.
