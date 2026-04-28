# rvoip-session-core

`rvoip-session-core` is the application-facing SIP session layer for RVOIP. It
coordinates dialog, registration, media, call state, transfer, DTMF, hold/resume,
and app-visible events so Rust applications can behave like programmable SIP
endpoints without owning low-level SIP transaction and RTP details directly.

## Where it fits

| Crate | Responsibility |
|-------|----------------|
| `dialog-core` | SIP dialog, transaction, request/response, routing, and subscription machinery |
| `media-core` / `rtp-core` | RTP, SRTP, codecs, audio sources, streams, and media transport |
| `session-core` | Application-facing call/session behavior and public call-control APIs |

## API surfaces

| API | Use it for | Shape |
|-----|------------|-------|
| `StreamPeer` | scripts, clients, softphones, examples, and tests | sequential calls plus event waits |
| `CallbackPeer` | servers, IVR, routing, and reactive endpoint apps | `CallHandler` trait callbacks |
| `UnifiedCoordinator` | bridges, gateways, custom peer types, and B2BUA-style orchestration | lower-level multi-session control |
| `SessionHandle` | per-call operations | call control, audio, DTMF, transfer, hold/resume, NOTIFY, INFO |

`UnifiedCoordinator` is the core primitive. `StreamPeer` and `CallbackPeer` are
ergonomic shells over it and should stay thin.

## Current validation

The current release line is alpha-quality but has real external PBX coverage.
The Asterisk suites validate registration/unregistration, TLS/SDES-SRTP,
registered-flow reuse, hold/resume, CANCEL, DTMF, blind transfer, REFER/NOTIFY
progress, and audio verification for both `StreamPeer` and `CallbackPeer`.

See:

- [`examples/README.md`](examples/README.md)
- [`examples/asterisk/README.md`](examples/asterisk/README.md)
- [`examples/asterisk_callback/README.md`](examples/asterisk_callback/README.md)
- [`docs/COMPATIBILITY_MATRIX.md`](docs/COMPATIBILITY_MATRIX.md)
- [`docs/TOPOLOGY_PROFILES.md`](docs/TOPOLOGY_PROFILES.md)

## Known limits

- Asterisk is the primary validated external PBX target today.
- FreeSWITCH/Sofia and Kamailio/OpenSIPS plus RTPengine are planned validation
  targets, not release claims yet.
- Carrier SBC readiness is partial and not certified.
- ICE, DTLS-SRTP, and WebRTC edge behavior are future work.
- Blind transfer is validated; attended transfer is currently exposed as
  REFER-with-Replaces primitives rather than full consultation-call
  orchestration.

## Release tracking

The active release-hardening checklist lives in
[`RELEASE_HARDENING_PLAN.md`](RELEASE_HARDENING_PLAN.md).
