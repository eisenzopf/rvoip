# session-core Release Hardening Plan

This tracks the work needed before the next `rvoip-session-core` release that
highlights the unified, stream, and callback API surfaces.

## Goals

- Position `session-core` as the application-facing SIP session layer above
  `dialog-core`, `media-core`, and `rtp-core`.
- Make the public API story explicit:
  - `StreamPeer` for sequential clients, scripts, softphones, and tests.
  - `CallbackPeer` for reactive servers, IVR, routing, and endpoint apps.
  - `UnifiedCoordinator` for lower-level orchestration, bridges, gateways, and
    B2BUA-style code.
  - `SessionHandle` for per-call control: audio, transfer, DTMF, hold/resume,
    and teardown.
- Remove fragile debug-string event routing from the cross-crate event path.
- Document what is validated today, what is alpha, and what belongs to the next
  interop hardening cycle.

## Progress

| Area | Status | Notes |
|------|--------|-------|
| Typed cross-crate event handling | Done | Normal dialog/media event routing now uses typed `RvoipCrossCrateEvent` dispatch. |
| REFER metadata propagation | Done | `Referred-By` and `Replaces` flow from dialog-core to public `Event::ReferReceived`. |
| session-core README | Done | Added API selection, layering, examples, and alpha status. |
| Compatibility matrix | Done | Added validated/planned profile table. |
| Topology profiles | Done | Documented LAN, Asterisk, proxy, carrier, and WebRTC edge profiles. |
| Release notes draft | Done | Drafted next release notes around API surfaces and Asterisk evidence. |
| Interop CI plan | Done | Defined SIPp/Asterisk/FreeSWITCH/proxy lab phases. |
| Tests | Done | `cargo check -p rvoip-session-core` and `cargo test -p rvoip-session-core` pass. `rvoip-infra-common` tests compile but still have unrelated task-management assertion failures. |

## Implementation Checklist

1. Finish typed event migration.
   - Match on `RvoipCrossCrateEvent::DialogToSession` and
     `RvoipCrossCrateEvent::MediaToSession`.
   - Remove normal-path `format!("{:?}", event)` dispatch.
   - Keep only a narrow unknown-event fallback that logs and drops.
   - Preserve existing public event semantics.

2. Propagate transfer metadata.
   - Extend `DialogToSessionEvent::TransferRequested` with
     `referred_by: Option<String>` and `replaces: Option<String>`.
   - Populate those fields from dialog-core's `SessionCoordinationEvent`.
   - Publish them through `Event::ReferReceived`.

3. Add release documentation.
   - `crates/session-core/README.md`
   - `crates/session-core/docs/COMPATIBILITY_MATRIX.md`
   - `crates/session-core/docs/TOPOLOGY_PROFILES.md`
   - `crates/session-core/docs/RELEASE_NOTES_NEXT.md`
   - `crates/session-core/docs/INTEROP_CI_PLAN.md`

4. Update existing docs.
   - Top-level `README.md`
   - `crates/session-core/examples/README.md`
   - `crates/session-core/examples/asterisk/README.md`
   - `crates/session-core/examples/asterisk_callback/README.md`
   - `crates/session-core/docs/RFC_COMPLIANCE_STATUS.md`
   - `crates/session-core/docs/ATTENDED_TRANSFER_IMPLEMENTATION_PLAN.md`

5. Verify.
   - `cargo fmt --check`
   - `cargo test -p rvoip-infra-common`
   - `cargo test -p rvoip-dialog-core`
   - `cargo test -p rvoip-media-core`
   - `cargo test -p rvoip-session-core`
   - Manual release gates remain the Asterisk StreamPeer and CallbackPeer
     example suites.

## Release Framing

Lead with:

> `session-core` now provides Rust-native programmable SIP session
> orchestration through three API surfaces: `StreamPeer`, `CallbackPeer`, and
> `UnifiedCoordinator`.

Validated claims:

- Local and multi-process examples.
- Asterisk UDP/RTP and TLS/SDES-SRTP scenarios.
- Registration/unregistration, hold/resume, CANCEL, DTMF, blind transfer,
  REFER/NOTIFY progress, registered-flow reuse, and audio verification.

Do not overclaim:

- FreeSWITCH/Sofia, Kamailio/OpenSIPS, RTPengine, carrier SBC, ICE,
  DTLS-SRTP, and WebRTC edge support remain future validation or future
  feature work.
- Blind transfer is validated; attended transfer should be described as
  primitives unless a full orchestration layer is added.
