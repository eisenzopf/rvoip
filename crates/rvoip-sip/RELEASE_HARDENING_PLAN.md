# rvoip-sip Release Hardening Plan

This tracks the work needed before the next `rvoip-sip` release that highlights
the unified, stream, and callback API surfaces. The crate was carved out of the
legacy `session-core` crate and now lives at `crates/rvoip-sip/` with package
name `rvoip-sip` (see [`docs/RELEASE_NOTES_NEXT.md`](docs/RELEASE_NOTES_NEXT.md)
for migration guidance).

## Goals

- Position `rvoip-sip` as the application-facing SIP session layer above
  `rvoip-sip-dialog`, `rvoip-media-core`, and `rvoip-rtp-core`.
- Make the public API story explicit:
  - `Endpoint` for simple softphones, PBX accounts, demos, and IVR legs.
  - `StreamPeer` for sequential clients, scripts, softphones, and tests.
  - `CallbackPeer` for reactive servers, IVR, routing, and endpoint apps.
  - `UnifiedCoordinator` for lower-level orchestration, bridges, gateways, and
    B2BUA-style code.
  - `SessionHandle` for per-call control: audio, transfer, DTMF, hold/resume,
    and teardown.
- Document what is validated today, what is alpha, and what belongs to the next
  interop hardening cycle.

## Progress

| Area | Status | Notes |
|------|--------|-------|
| Crate rename `session-core` → `rvoip-sip` | Done | Code, examples, doctests, and rustdoc prose migrated. Legacy shim crate deleted. |
| Typed cross-crate event handling | Done | Normal dialog/media event routing uses typed `RvoipCrossCrateEvent` dispatch. |
| REFER metadata propagation | Done | `Referred-By` and `Replaces` flow from dialog-core to public `Event::ReferReceived`. |
| README & module rustdoc | Done | `crates/rvoip-sip/README.md` plus all `//!`/`///` module headers use the new name. |
| Compatibility matrix | Done | `docs/COMPATIBILITY_MATRIX.md` covers validated/planned profiles. |
| Topology profiles | Done | `docs/TOPOLOGY_PROFILES.md` covers LAN, Asterisk, proxy, carrier, and WebRTC edge profiles. |
| Release notes draft | Done | `docs/RELEASE_NOTES_NEXT.md` framed around API surfaces and Asterisk evidence. |
| Interop CI plan | Done | `docs/INTEROP_CI_PLAN.md` defines SIPp/Asterisk/FreeSWITCH/proxy lab phases. |
| Tests | Done | `cargo check -p rvoip-sip` and `cargo test -p rvoip-sip --doc` pass (217 doctests). |

## Verification before release

```sh
cargo fmt --check
cargo test -p rvoip-infra-common
cargo test -p rvoip-sip-dialog
cargo test -p rvoip-media-core
cargo test -p rvoip-sip
cargo test -p rvoip-sip --doc
cargo doc -p rvoip-sip --no-deps
```

Manual release gates remain the Asterisk `StreamPeer` and `CallbackPeer`
example suites under `examples/pbx/`.

## Release framing

Lead with:

> `rvoip-sip` (formerly `session-core`) now provides Rust-native programmable
> SIP session orchestration through four API surfaces: `Endpoint`,
> `StreamPeer`, `CallbackPeer`, and `UnifiedCoordinator`.

Validated claims:

- Local and multi-process examples under `examples/`.
- Asterisk UDP/RTP and TLS/SDES-SRTP scenarios.
- Registration/unregistration, hold/resume, CANCEL, DTMF, blind transfer,
  REFER/NOTIFY progress, registered-flow reuse, and audio verification.

Do not overclaim:

- FreeSWITCH/Sofia, Kamailio/OpenSIPS, RTPengine, carrier SBC, ICE,
  DTLS-SRTP, and WebRTC edge support remain future validation or future
  feature work.
- Blind transfer is validated; attended transfer should be described as
  REFER-with-Replaces primitives unless a full orchestration layer is added.
