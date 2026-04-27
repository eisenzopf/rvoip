# Hold/Resume SDP Consolidation Plan

## Summary

Production hold/resume must use the same SDP offer/answer architecture as initial call setup. The Asterisk compatibility work proved the failure mode: ad hoc hold SDP can pass a narrow test while still losing the real RTP port, public media address, codec policy, SRTP attributes, or future NAT/ICE/DTLS attributes.

The target architecture is:

- `dialog-core` owns RFC 3264 matching semantics.
- `session-core` owns local media policy, SDP construction, state-machine coordination, and API events.
- `media-core` owns runtime RTP/audio direction enforcement.

Hold/resume is therefore a direction-aware SDP renegotiation path, not a separate SDP generator.

## Ownership Boundaries

### dialog-core

- Keep `dialog_core::sdp::match_offer` as the canonical RFC 3264 matcher.
- Match codecs, transport policy, and media-line acceptance/rejection from normalized offer/capability inputs.
- Return structured match results only; do not learn session runtime details such as allocated RTP ports, NAT-mapped addresses, SRTP keys, or media-session identifiers.

### session-core

- Build all local SDP offers and answers using session runtime context.
- Pass local codec/SRTP policy into `dialog-core::sdp::match_offer`.
- Track per-session SDP origin identity and version.
- Coordinate hold/resume states and re-INVITE retries.
- Publish public API events for local and remote hold/resume changes.

### media-core

- Enforce the negotiated media direction at RTP/audio runtime.
- Gate outbound generated/app audio for `recvonly` and `inactive`.
- Gate inbound app delivery for `sendonly` and `inactive`.
- Keep RTP session lifecycle, transmitter management, and audio callback behavior inside the media layer.

## SDP Construction

Replace hold/resume-specific SDP generation with one session-aware builder:

```rust
generate_local_sdp_offer(session_id, MediaDirection::SendRecv)
generate_local_sdp_offer(session_id, MediaDirection::SendOnly)
generate_local_sdp_offer(session_id, MediaDirection::RecvOnly)
generate_local_sdp_offer(session_id, MediaDirection::Inactive)
```

Usage:

- Initial INVITE: `SendRecv`
- Resume: `SendRecv`
- Local hold: `SendOnly` by default
- Remote-hold answer: answer direction derived from the remote offer

Generated SDP must preserve:

- Active media session RTP port.
- Configured or discovered public RTP address.
- Negotiated codec policy and the full offered codec set where applicable.
- `telephone-event`.
- Comfort noise.
- SRTP profile and crypto attributes.
- Future ICE, TURN, and DTLS attributes without creating another hold-specific path.

Per-session SDP origin tracking:

- Stable numeric `o=` session id for the lifetime of a session.
- Monotonically increasing `o=` version for every locally generated offer/answer body that can be placed on the wire.
- Glare retries must reuse the original hold/resume intent while generating a fresh SDP version.

## Offer/Answer Matching

`session-core` parses inbound SDP, extracts session/media context, and delegates offer matching to:

```rust
dialog_core::sdp::match_offer
```

The integration rules are:

- Build `AnswerCapabilities` from session-core media policy.
- Let dialog-core decide RFC 3264 media-line acceptance and codec intersection.
- Let session-core construct the final SDP answer because it owns the active RTP port, advertised address, SRTP context, comfort-noise policy, and runtime media settings.
- Keep direction handling explicit in session-core until dialog-core exposes a richer structured direction result.
- Do not spread this behavior into `dialog-core::sdp::offer_answer` or `media_tracking` unless those modules become the implemented matching path.

## State Machine

Keep the existing state flow:

```text
Active -> HoldPending -> OnHold
OnHold -> Resuming -> Active
```

Rules:

- Local media direction is not committed until the re-INVITE receives `2xx`.
- Hold failure rolls back to `Active`.
- Resume failure rolls back to `OnHold`.
- Non-2xx, timeout, and retry exhaustion must leave the call alive unless the dialog layer reports a terminal condition.
- Pending re-INVITE state is cleared after accepted, rejected, or timed-out hold/resume transactions.
- 491 glare retry behavior is preserved.
- 491 retries reuse hold/resume intent and generate fresh SDP versions.
- SDP generation and re-INVITE send failures must propagate; silent `.ok()` handling is not acceptable for production signaling.

## Media Direction

`MediaAdapter::set_media_direction` must call through to media-core, and media-core must enforce the behavior:

| Direction | Send | Receive/app delivery |
| --- | --- | --- |
| `SendRecv` | enabled | enabled |
| `SendOnly` | enabled | suppressed |
| `RecvOnly` | suppressed | enabled |
| `Inactive` | suppressed | suppressed |

Local hold behavior:

- Generate `a=sendonly`.
- Keep existing media direction until the hold re-INVITE succeeds.
- After `2xx`, apply `SendOnly` in media-core and publish local hold accepted.

Local resume behavior:

- Generate `a=sendrecv`.
- Keep held media direction until the resume re-INVITE succeeds.
- After `2xx`, apply `SendRecv` in media-core and publish local resume accepted.

Remote hold/resume behavior:

- Inbound `sendonly`, `recvonly`, and `inactive` offers must produce correct answer directions.
- Accepted inbound remote hold/resume must update media direction independently from local hold state.
- API events must distinguish remote hold/resume from local hold/resume.

## API Events

Existing events must publish reliably:

- `Event::CallOnHold`
- `Event::CallResumed`

Remote semantics must also be exposed so applications do not poll `is_on_hold()`:

- Local hold accepted.
- Local resume accepted.
- Remote placed us on hold.
- Remote resumed.

The state machine can keep publishing local custom events, but `UnifiedCoordinator` must bridge those to the public `session_to_app` event stream. Remote hold/resume detection belongs near the inbound re-INVITE handler because that path can compare media direction before and after accepting the offer.

## Test Plan

### Unit Tests

- Hold SDP uses the real RTP port, current advertised IP, full codec list, and `a=sendonly`.
- Resume SDP uses the same media shape and `a=sendrecv`.
- SDP `o=` version increments for initial offer, hold, resume, and glare retry.
- Inbound `sendonly`, `recvonly`, and `inactive` offers produce correct answer directions.
- Codec mismatch produces rejection or `488`.
- SRTP-enabled hold/resume preserves `RTP/SAVP` and crypto handling.

### Integration Tests

- Hold accepted:
  - State reaches `OnHold`.
  - Pending re-INVITE clears.
  - Public event publishes.
  - Media direction changes after `2xx`.
- Hold rejected:
  - State rolls back to `Active`.
  - No terminal `CallFailed`.
  - Media remains active.
- Resume accepted:
  - State reaches `Active`.
  - Public event publishes.
  - Media resumes.
- Resume rejected:
  - State returns to `OnHold`.
  - Media remains held.
- 491 glare retry:
  - Preserves hold/resume intent.
  - Increments SDP version.
  - Does not tear down the call.

### End-to-End Tests

- Existing `streampeer/hold_resume/run.sh` passes.
- Existing Asterisk two-endpoint audio example passes.
- New Asterisk hold/resume example passes.
- Add SRTP hold/resume once plaintext behavior is stable.

## Current Implementation Checklist

- [x] Add one direction-aware local SDP offer builder in `session-core`.
- [x] Generate hold/resume SDP from active media-session context.
- [x] Track stable SDP origin id and monotonic version per session.
- [x] Keep `dialog-core::sdp::match_offer` as the codec/profile matcher.
- [x] Carry negotiated local/remote media directions in session state.
- [x] Clear pending re-INVITE state on accepted/rejected/timed-out hold/resume.
- [x] Remove silent hold/resume SDP `.ok()` failure handling from state actions.
- [x] Implement media-core direction gating for send and receive paths.
- [x] Bridge local hold/resume state-machine events to public API events.
- [x] Publish remote hold/resume API events from accepted inbound re-INVITEs.
- [x] Add focused unit tests for direction behavior.
- [x] Run streampeer and Asterisk hold/resume examples after the implementation settles.
