# rvoip-nat-core

[![Crates.io](https://img.shields.io/crates/v/rvoip-nat-core.svg)](https://crates.io/crates/rvoip-nat-core)
[![Documentation](https://docs.rs/rvoip-nat-core/badge.svg)](https://docs.rs/rvoip-nat-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip/blob/main/LICENSE)

> **Alpha** (`0.1.x`) — early and API-unstable; expect breaking changes before `1.0`.
> Not part of the `rvoip-sip` 0.2.x beta contract. Host and server-reflexive
> candidates only — no TURN/relay, no trickle ICE.

Real ICE (RFC 8445) NAT traversal for the [rvoip](https://github.com/eisenzopf/rvoip)
media plane, behind an SDP-agnostic API.

## What this is

A thin wrapper over [`webrtc-ice`](https://crates.io/crates/webrtc-ice) 0.17.x
— the same webrtc-rs "production" lineage `rvoip-rtp-core` already uses for
DTLS-SRTP and SRTP interop, not the pre-production `rtc` crate `rvoip-webrtc`
depends on — plus a bridge that lets connectivity checks share a socket this
crate doesn't own the read half of, instead of needing a dedicated port.

- [`agent::IceAgent`] — gathers host/server-reflexive candidates, exchanges
  credentials, and runs RFC 8445 connectivity checks. Has no notion of SDP:
  callers (e.g. `rvoip-sip`) carry [`IceAgent::local_credentials`] and
  [`IceAgent::gather_candidates`]'s output into `a=ice-ufrag`/`a=ice-pwd`/
  `a=candidate` lines themselves, and parse the peer's back into
  [`IceCandidate`].
- [`candidate::IceCandidate`] / [`CandidateKind`] — the handful of fields an
  `a=candidate:` line needs (RFC 8839 §5.1), independent of any particular
  SDP library's types.
- [`bridge::SharedIceMux`] — a `UDPMux`/`UDPMuxWriter` implementation for
  running ICE over a socket that's shared with other traffic (RTP/RTCP/DTLS).
  `webrtc-ice`'s own `UDPMuxDefault` assumes exclusive socket ownership and
  spawns its own read loop, which would steal every non-STUN datagram off a
  shared socket — this doesn't read the socket itself; the caller's own demux
  loop pushes classified STUN datagrams in via
  [`IceAgent::handle_incoming_stun`].

`rvoip-rtp-core`'s `ice` feature bridges this onto its shared RTP/RTCP/DTLS
socket (`UdpRtpTransport::ice_conn_adapter`/`subscribe_stun_datagrams`);
`rvoip-sip`'s `ice` feature wires it into SDP offer/answer generation and the
call state machine, behind `Config::enable_ice`.

## Install

You usually don't depend on this directly — enable `rvoip-sip`'s `ice`
feature, which forwards through `rvoip-media-core` and `rvoip-rtp-core`:

```toml
[dependencies]
rvoip-sip = { version = "0.2", features = ["ice"] }
```

Depending on it directly (e.g. to drive an `IceAgent` outside the SIP stack):

```toml
[dependencies]
rvoip-nat-core = { version = "0.1", features = ["ice"] }
```

The `ice` feature is required for anything beyond the SDP-agnostic
[`IceCandidate`]/[`CandidateKind`] types — it pulls in `webrtc-ice`,
`webrtc-util`, and `stun`, all optional and off by default.

## Example

Two independent agents completing a real connectivity check over loopback —
condensed from `tests/ice_agent_connectivity_test.rs`:

```rust,ignore
use rvoip_nat_core::{IceAgent, IceRole};

let controlling = IceAgent::new(IceRole::Controlling, &[]).await?;
let controlled = IceAgent::new(IceRole::Controlled, &[]).await?;

let (c_ufrag, c_pwd) = controlling.local_credentials().await;
let (d_ufrag, d_pwd) = controlled.local_credentials().await;

let c_candidates = controlling.gather_candidates().await?;
let d_candidates = controlled.gather_candidates().await?;
for c in &d_candidates {
    controlling.add_remote_candidate(c)?;
}
for c in &c_candidates {
    controlled.add_remote_candidate(c)?;
}

// Must be spawned, never awaited before both sides' SDP has actually gone
// out — the controlled side's checks can't succeed until the controlling
// side has started its own. See `IceAgent::connect`'s own doc comment.
let (a, b) = tokio::join!(
    controlling.connect(d_ufrag, d_pwd),
    controlled.connect(c_ufrag, c_pwd),
);
let (selected_remote_addr_a, selected_remote_addr_b) = (a?, b?);
```

For the socket-sharing path (`new_with_shared_socket`), see
`rvoip-rtp-core/tests/ice_transport_bridge_test.rs` and
`rvoip-sip/examples/stream_peer/07_ice/`.

## What's not here

- **TURN/relay candidates.** `candidate_types` never includes `Relay`.
- **Trickle ICE.** `gather_candidates` returns the full batch once gathering
  completes; there's no incremental candidate callback surface for callers.
- **ICE restart / re-INVITE renegotiation.** One agent, one connectivity
  check, for the life of a dialog.
- **STUN MESSAGE-INTEGRITY/FINGERPRINT for anything but ICE connectivity
  checks.** `rvoip-rtp-core`'s own `network::stun::StunClient` (a separate,
  older, boot-time public-address probe) is unrelated to this crate.

## License

Licensed under the MIT license. See the repository
[LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
