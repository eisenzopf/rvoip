# Draft Release Notes: session-core

## Headline

`session-core` now presents a clear programmable SIP session API with three
surfaces: `StreamPeer`, `CallbackPeer`, and `UnifiedCoordinator`.

## What changed

- `StreamPeer` is the sequential API for clients, scripts, softphones, and
  integration tests.
- `CallbackPeer` is the reactive API for servers, IVR, routing, and endpoint
  applications.
- `UnifiedCoordinator` is the lower-level orchestration API for bridges,
  gateways, custom peer types, and future B2BUA-style crates.
- `SessionHandle` centralizes per-call control for call lifecycle, audio, DTMF,
  hold/resume, transfer, NOTIFY, and INFO.
- Cross-crate event handling now routes normal dialog/media events through typed
  `RvoipCrossCrateEvent` variants instead of debug-string matching.
- REFER transfer metadata now preserves `Referred-By` and `Replaces` when
  dialog-core receives them.

## Validated behavior

The Asterisk suites provide executable evidence for:

- registration and clean unregister
- TLS/SDES-SRTP call setup
- registered TLS flow reuse
- hold/resume
- CANCEL
- DTMF
- blind transfer
- REFER/NOTIFY progress
- SRTP audio verification

See `docs/COMPATIBILITY_MATRIX.md` for the exact support matrix.

## Still alpha

- Asterisk is the primary external PBX target validated for this release.
- FreeSWITCH/Sofia and Kamailio/OpenSIPS plus RTPengine are planned next
  interop targets.
- Carrier SBC readiness is partial and not certified.
- Service-Route/Path, outbound proxy registration, multi-contact/multi-flow,
  ICE, DTLS-SRTP, and WebRTC edge behavior remain future work.
- Blind transfer is validated; attended transfer is available as primitives,
  not as full consultation-call orchestration.

## Upgrade guidance

- Start with `StreamPeer` for scripts, examples, tests, and simple clients.
- Start with `CallbackPeer` for endpoint servers, IVR, and routing apps.
- Start with `UnifiedCoordinator` for multi-session orchestration, bridge, or
  gateway behavior.
