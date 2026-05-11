# rvoip-sip

`rvoip-sip` is the application-facing SIP session layer for RVOIP. It
coordinates dialog, registration, media, call state, transfer, DTMF, hold/resume,
and app-visible events so Rust applications can behave like programmable SIP
endpoints without owning low-level SIP transaction and RTP details directly.

## Where it fits

| Crate | Responsibility |
|-------|----------------|
| `rvoip-sip-dialog` | SIP dialog, transaction, request/response, routing, and subscription machinery |
| `rvoip-media-core` / `rvoip-rtp-core` | RTP, SRTP, codecs, audio sources, streams, and media transport |
| `rvoip-sip` (this crate) | Application-facing call/session behavior and public call-control APIs |

## API surfaces

| API | Use it for | Shape |
|-----|------------|-------|
| `Endpoint` | softphones, PBX accounts, demos, and simple IVR legs | account/profile builder plus call helpers |
| `StreamPeer` | scripts, clients, softphones, examples, and tests | sequential calls plus event waits |
| `CallbackPeer` | servers, IVR, routing, and reactive endpoint apps | closure builder or `CallHandler` trait callbacks |
| `UnifiedCoordinator` | bridges, gateways, custom peer types, and B2BUA-style orchestration | lower-level multi-session control |
| `SessionHandle` | per-call operations | call control, audio, DTMF, transfer, hold/resume, NOTIFY, INFO |

`UnifiedCoordinator` is the core primitive. `Endpoint`, `StreamPeer`, and
`CallbackPeer` are ergonomic shells over it and should stay thin. Start with
`Endpoint` unless you already know you need event-stream ownership, callback
dispatch, or custom multi-leg orchestration.

## Quick start

```rust,no_run
use std::time::Duration;
use rvoip_sip::{Endpoint, EndpointProfile, Result};

# async fn example() -> Result<()> {
let mut endpoint = Endpoint::builder()
    .name("alice")
    .account("1001")
    .password("secret")
    .registrar("sips:pbx.example.com:5061")
    .profile(EndpointProfile::AsteriskTlsSrtpRegisteredFlow)
    .build()
    .await?;

endpoint.register().await?;
let call = endpoint.call("1002").await?;
call.wait_for_answered(Some(Duration::from_secs(30))).await?;
call.hangup().await?;
# Ok(())
# }
```

## Current validation

The current release line is alpha-quality but has real external PBX coverage.
The Asterisk suites validate registration/unregistration, TLS/SDES-SRTP,
registered-flow reuse, hold/resume, CANCEL, DTMF, blind transfer, REFER/NOTIFY
progress, and audio verification for both `StreamPeer` and `CallbackPeer`.

See:

- [`examples/README.md`](examples/README.md)
- [`examples/pbx/README.md`](examples/pbx/README.md) — Asterisk and FreeSWITCH interop matrix (run through `Endpoint`, `StreamPeer`, and `CallbackPeer::builder`)
- [`examples/sip_client/README.md`](examples/sip_client/README.md) — terminal softphone built on the `Endpoint` facade

## Known limits

- Asterisk and FreeSWITCH examples are deployment/interop recipes, not the
  beginner learning path.
- Kamailio/OpenSIPS plus RTPengine are planned validation targets, not release
  claims yet.
- Carrier SBC readiness is partial and not certified.
- ICE, DTLS-SRTP, and WebRTC edge behavior are future work.
- Blind transfer is validated; attended transfer is currently exposed as
  REFER-with-Replaces primitives rather than full consultation-call
  orchestration.

## Release tracking

The active release-hardening checklist lives in
[`RELEASE_HARDENING_PLAN.md`](RELEASE_HARDENING_PLAN.md).
