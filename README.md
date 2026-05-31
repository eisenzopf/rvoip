<div align="center">
  <img src="rvoip-banner.svg" alt="rvoip - The modern VoIP stack" width="50%" />
</div>

<div align="center">

[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/eisenzopf/rvoip/blob/main/LICENSE)
[![Repository](https://img.shields.io/badge/github-eisenzopf%2Frvoip-24292f.svg)](https://github.com/eisenzopf/rvoip)
[![docs.rs (rvoip-sip)](https://docs.rs/rvoip-sip/badge.svg)](https://docs.rs/rvoip-sip)

**A modern, 100% pure Rust SIP/VoIP stack**

[🚀 Quick Start](#-quick-start) • [📦 Workspace](#-workspace) • [📚 rvoip-sip docs](https://docs.rs/rvoip-sip) • [💡 Examples](crates/rvoip-sip/examples/README.md)

</div>

---

> **Alpha moving toward beta.** This repository is not yet a broad production
> release. The **`rvoip-sip`** crate is the most mature surface and is a
> **beta candidate** for bounded SIP client, server, PBX, gateway, and B2BUA
> scenarios; the other crates in the workspace are earlier-stage. Full-media
> performance claims are bounded to the documented beta profiles (≤ 2,000 CPS);
> higher numbers are tuned profiles with explicit caveats. WebRTC-grade media,
> DTLS-SRTP, ICE, TURN, browser interop, and carrier-SBC certification are
> post-beta unless separately completed and tested.

## 📋 Table of Contents

- [🚀 Quick Start](#-quick-start)
- [🎯 What is rvoip?](#-what-is-rvoip)
- [📦 Workspace](#-workspace)
- [🚀 SIP Protocol Features](#-sip-protocol-features)
- [🧪 Testing & Evidence](#-testing--evidence)
- [📋 Status & Roadmap](#-status--roadmap)
- [🤝 Contributing](#-contributing)
- [📄 License](#-license)

---

rvoip is a modern, 100% pure Rust implementation of a SIP/VoIP stack — no FFI,
no C libraries. It is built as a set of focused crates so you can depend on just
the SIP session layer, or compose the broader real-time gateway. The goal is a
robust, safe foundation for VoIP applications ranging from softphones to
PBX/gateway back-ends.

## 🚀 Quick Start

The recommended entry point is **[`rvoip-sip`](crates/rvoip-sip/README.md)** —
the application-facing SIP session layer. It gives you programmable SIP
endpoints (registration, calls, hold/resume, transfer, DTMF, custom headers,
events) without owning SIP transaction or RTP details directly.

```toml
[dependencies]
rvoip-sip = "0.2"
tokio = { version = "1", features = ["full"] }
```

Place a registered PBX call with the `Endpoint` facade:

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

let call = endpoint
    .call_and_wait("1002", Some(Duration::from_secs(30)))
    .await?;

call.send_dtmf('1').await?;
call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
endpoint.shutdown().await?;
# Ok(())
# }
```

Or run a complete local two-endpoint call with no external infrastructure:

```sh
cargo run -p rvoip-sip --example endpoint_local_call
```

`rvoip-sip` exposes four API surfaces — pick one by how you want to drive it:

| Need | Start with |
| --- | --- |
| Softphone or PBX account | [`Endpoint`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.Endpoint.html) |
| Sequential client, script, or test | [`StreamPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.StreamPeer.html) |
| Reactive server, IVR, router, queue | [`CallbackPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.CallbackPeer.html) |
| B2BUA, gateway, multi-leg orchestration | [`UnifiedCoordinator`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.UnifiedCoordinator.html) |

See the **[rvoip-sip README](crates/rvoip-sip/README.md)** and
**[examples guide](crates/rvoip-sip/examples/README.md)** for the full tour.

### The `rvoip` facade (multi-protocol gateway)

The umbrella [`rvoip`](crates/rvoip/README.md) crate is an earlier-stage facade
that bundles the SIP, WebRTC, and UCTP interop adapters behind a single
`Orchestrator`, exposing per-protocol surfaces at `rvoip::sip`, `rvoip::webrtc`,
and `rvoip::uctp` (feature-gated). It targets gateway/bridging use cases that
span more than one transport:

```rust,no_run
use rvoip::{Orchestrator, Config};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let orchestrator = Orchestrator::new(Config::default());

// Register interop adapters (e.g. a SIP adapter built from a configured
// rvoip-sip UnifiedCoordinator) via `orchestrator.register(adapter)?`.

let mut events = orchestrator.subscribe_events();
while let Ok(event) = events.recv().await {
    drop(event); // handle each orchestrator event
}
# Ok(()) }
```

## 🎯 What is rvoip?

- 🦀 **Pure Rust** — zero FFI dependencies; memory safety and predictable performance.
- 🧩 **Modular** — focused crates with clean separation of concerns; depend on only what you need.
- 📋 **Standards-oriented** — RFC-tracked SIP with a published [compliance matrix](crates/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md).
- 🔎 **Evidence-backed** — the SIP layer ships with reproducible interop and performance gates rather than unverified claims.
- 👩‍💻 **Multiple altitudes** — from low-level message parsing (`rvoip-sip-core`) to a programmable session API (`rvoip-sip`) to a multi-protocol gateway facade (`rvoip`).

## 📦 Workspace

rvoip is a Cargo workspace. The crates below are the current members, grouped by
area. Maturity varies — the **SIP stack** is the beta-candidate surface; other
groups are in active development.

### SIP stack — beta candidate

| Crate | Role |
| --- | --- |
| [`rvoip-sip`](crates/rvoip-sip/README.md) | **Application-facing SIP session layer** — the recommended entry point. `Endpoint`, `StreamPeer`, `CallbackPeer`, `UnifiedCoordinator`. |
| `rvoip-sip-core` | RFC 3261 SIP message parsing, serialization, and SDP. |
| `rvoip-sip-transport` | SIP transport: UDP, TCP, TLS, WebSocket. |
| `rvoip-sip-dialog` | RFC 3261 dialog and transaction state machines. |
| `rvoip-sip-registrar` | Registrar / location service. |
| `rvoip-sip-proxy` | Stateful SIP proxy primitives. |

### Media

| Crate | Role |
| --- | --- |
| `rvoip-media-core` (`media-core`) | Media session coordination and audio processing. |
| `rvoip-rtp-core` (`rtp-core`) | RTP/RTCP transport and SRTP. |
| `rvoip-codec-core` (`codec-core`) | Audio codec implementations (e.g. G.711). |

### Gateway facade & core

| Crate | Role |
| --- | --- |
| [`rvoip`](crates/rvoip/README.md) | Feature-gated facade re-exporting the protocol surfaces (`rvoip::sip`, `rvoip::webrtc`, `rvoip::uctp`, …) over an `Orchestrator`. |
| `rvoip-core` | The orchestrator / conversation engine. |
| `rvoip-core-traits` | Shared trait + data surface (`Conversation`, `Session`, `Connection`, `Stream`, `Message`, `Participant`). |

### UCTP & WebRTC — earlier-stage / experimental

| Crate | Role |
| --- | --- |
| `rvoip-uctp` | Universal Conversation Transport Protocol (wire format + session state). |
| `rvoip-quic`, `rvoip-webtransport`, `rvoip-websocket` | UCTP substrate adapters. |
| `rvoip-webrtc` | WebRTC interop adapter (DTLS-SRTP / ICE); off by default. |

### Supporting

| Crate | Role |
| --- | --- |
| `rvoip-vcon` | IETF vCon conversation-container builder, signer, and store. |
| `rvoip-identity` | Identity-provider backends. |
| `rvoip-harness` | In-process AI voice (ASR/TTS/dialog) harness. |
| `rvoip-client` | Client-side SDK surface. |
| `rvoip-stir-shaken` | STIR/SHAKEN caller-ID attestation. |
| `auth-core`, `users-core` | Authentication and user backends. |
| `infra-common` | Shared infrastructure (event bus, runtime helpers). |

## 🚀 SIP Protocol Features

The matrices below describe the **`rvoip-sip`** surface. "✅" features are
covered by runnable examples or regression fixtures; see the
[RFC compliance matrix](crates/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md) and
[compatibility matrix](crates/rvoip-sip/docs/COMPATIBILITY_MATRIX.md) for the
exact claim boundaries.

### Core SIP Methods

| Method | Status | RFC | Notes |
|--------|--------|-----|-------|
| **INVITE / ACK / BYE / CANCEL** | ✅ | RFC 3261 | Full call setup, teardown, and cancellation |
| **REGISTER** | ✅ | RFC 3261 | Registration lifecycle with refresh |
| **OPTIONS** | ✅ | RFC 3261 | Capability query |
| **SUBSCRIBE / NOTIFY** | ✅ | RFC 6665 | Event subscriptions; REFER progress, dialog package |
| **MESSAGE** | ✅ | RFC 3428 | In-dialog and out-of-dialog messaging |
| **UPDATE** | ✅ | RFC 3311 | Mid-session updates |
| **INFO** | ✅ | RFC 6086 | Mid-session information |
| **PRACK** | ✅ | RFC 3262 | Reliable provisional responses |
| **REFER** | ✅ | RFC 3515 | Blind transfer; attended-transfer primitives |
| **PUBLISH** | Post-beta | RFC 3903 | Parser support exists; application flow is not a beta claim |

### Authentication & Security

| Feature | Status | RFC | Notes |
|---------|--------|-----|-------|
| **Digest Authentication** | ✅ | RFC 3261 | MD5 / SHA-256 challenge-response |
| **TLS Transport** | ✅ | RFC 3261 | TLS 1.2 / 1.3 for `sips:` |
| **SDES-SRTP** | Partial | RFC 4568 | PBX/profile-specific beta evidence |
| **DTLS-SRTP / ZRTP / MIKEY** | Post-beta | RFC 5763 / 6189 / 3830 | Not a beta claim |

### Transport

| Transport | Status | RFC | Notes |
|-----------|--------|-----|-------|
| **UDP** | ✅ | RFC 3261 | Primary SIP transport |
| **TCP** | ✅ | RFC 3261 | Reliable transport |
| **TLS** | ✅ | RFC 3261 | Secure transport (TLS 1.2/1.3) |
| **WebSocket** | Partial | RFC 7118 | WSS-outbound / browser interop is post-beta |

### Dialog, Media & Call Control

| Feature | Status | Notes |
|---------|--------|-------|
| **Early / confirmed dialogs** | ✅ | RFC 3261 1xx/2xx handling |
| **Session timers** | ✅ | RFC 4028 keep-alive + refresh |
| **RTP media + bidirectional audio** | ✅ | RTP sessions with audio frames |
| **DTMF** | ✅ | RFC 4733 telephone-event |
| **Hold / resume** | ✅ | Mid-call re-INVITE |
| **Blind transfer** | ✅ | REFER + NOTIFY progress |
| **Attended transfer** | Partial | Exposed as primitives, not a full consultation workflow |
| **B2BUA / gateway helpers** | ✅ | `server::*` bridge, contact resolution, transfer orchestration |
| **Conference mixing (3+ party)** | Post-beta | Not a beta claim |

## 🧪 Testing & Evidence

The SIP layer is gated by a reproducible evidence suite rather than unverified
claims. From the most recent beta-candidate reference run:

- **Unit / integration / doctests** — `rvoip-sip` passes its full test suite (700+ tests and 200+ doctests) with examples building clean.
- **PBX interop** — 192 / 192 Asterisk and FreeSWITCH scenario rows passed (registration, calls, hold/resume, ring/cancel, DTMF, reject/busy, blind transfer; UDP + TLS).
- **SIPp performance** — 30, 100, 300, 1,000, and 2,000 CPS matrix passed.
- **Soak** — a 30-minute soak completed 35,109 / 35,109 calls with a bounded RSS slope.
- **Security** — dependency advisory audit and parser fuzz smoke passed.

Reproduce locally with the gate script (external PBX/SIPp deps are opt-in):

```sh
crates/rvoip-sip/scripts/beta_gate.sh --local
```

Claim boundaries are documented in
[`crates/rvoip-sip/docs/`](crates/rvoip-sip/docs/): `BETA_RELEASE_CHECKLIST.md`,
`COMPATIBILITY_MATRIX.md`, `RFC_COMPLIANCE_MATRIX.md`, `SECURITY_POSTURE.md`,
and `BETA_PERFORMANCE_REPORT.md`.

## 📋 Status & Roadmap

| Area | Status |
|------|--------|
| **`rvoip-sip` SIP session layer** | Beta candidate for bounded SIP client / server / PBX / gateway / B2BUA scenarios |
| SIP core / transport / dialog / registrar / proxy | Support the beta-candidate SIP surface |
| Media (media-core, rtp-core, codec-core) | Functional for the tested audio/SRTP profiles |
| Gateway facade (`rvoip`, `rvoip-core`) | In development |
| UCTP family, WebRTC, vCon, identity, harness, client | Earlier-stage / experimental |

**Known limits** (see the [rvoip-sip README](crates/rvoip-sip/README.md#known-limits)
for the authoritative list): carrier-SBC readiness is partial and uncertified;
WebRTC/browser interop, ICE, TURN, DTLS-SRTP, and WSS-outbound are out of the
beta-candidate claim; performance claims are bounded to the documented profiles.

## 🤝 Contributing

Contributions are welcome — bug reports, interop findings, and documentation
fixes especially. Use the [issue tracker](https://github.com/eisenzopf/rvoip/issues).
When reporting SIP interop behavior, include the peer, transport, media-security
mode, a relevant SIP trace, and the smallest example that reproduces it.

## 📄 License

Licensed under either of [Apache License, Version 2.0](https://github.com/eisenzopf/rvoip/blob/main/LICENSE)
or [MIT License](https://github.com/eisenzopf/rvoip/blob/main/LICENSE), at your option.

<div align="center">
<sub>Built with ❤️ in Rust</sub>
</div>
