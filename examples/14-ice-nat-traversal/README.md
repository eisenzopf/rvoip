# 14 · ICE (RFC 8445) NAT traversal

> **Status: Experimental, not beta-scoped.** ICE lives behind rvoip-sip's
> off-by-default `ice` feature and the new, alpha-tier `rvoip-nat-core`
> crate. Host and server-reflexive candidates only — no TURN/relay, no
> trickle ICE. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md)
> for what beta actually claims.

## Overview

A **caller** dials a **callee** over loopback with `Config::enable_ice = true`.
Both sides gather ICE candidates, exchange them in the SDP offer/answer
(`a=ice-ufrag` / `a=ice-pwd` / `a=candidate`), and run a real `webrtc-ice`
connectivity check over the same RTP port used for media. Once each side's
check resolves, it overrides its RTP remote address with the selected
candidate pair and prints it — independent of call setup, which is never
gated on ICE completing.

It uses the [`StreamPeer`] surface on **both** sides (unlike most other
examples here, which pair a `CallbackPeer` server with a `StreamPeer`
client) so each side can subscribe to the raw event stream and observe
`Event::IceConnected` directly. The `CallHandler` callback surface doesn't
route this event to a dedicated hook today.

ICE is orthogonal to SRTP keying — it can combine with SDES, DTLS-SRTP, or
(as here) plaintext media.

## Demo flow

1. **callee** binds `sip:callee@127.0.0.1:5061`, enables ICE, and waits for
   an INVITE.
2. **caller** binds `:5060`, enables ICE, and sends
   `INVITE sip:callee@127.0.0.1:5061` with its gathered candidates.
3. The callee answers with its own candidates; both sides feed the peer's
   candidates into their `IceAgent` and run a connectivity check in the
   background.
4. Both sides observe `Event::IceConnected { selected_addr, .. }` and print
   the selected pair.
5. **caller** hangs up; the callee observes the call end. Both exit cleanly.

## Architecture

```
   caller (:5060)                                     callee (:5061)
        │  ── INVITE  a=ice-ufrag/pwd, a=candidate ──▶ │
        │  ◀───────────────────────────── 200 OK ───── │  (own ufrag/pwd/candidates)
        │  ── ACK ──────────────────────────────────▶ │
        │  ◀═══ RFC 8445 connectivity check (STUN) ══▶ │   (over the RTP port)
        │  ── BYE ──────────────────────────────────▶ │
```

## Quick start

```sh
./run_demo.sh
```

Or run the two sides by hand in separate terminals:

```sh
cargo run --bin callee -- --port 5061
cargo run --bin caller -- --port 5060 --peer-port 5061
```

## Expected output

```text
  [caller] inviting sip:callee@127.0.0.1:5061
  [caller] call connected as session-…
  [caller] waiting for ICE connectivity check…
  [caller] ICE connected — selected pair remote address: 127.0.0.1:…
  [caller] call completed, hung up cleanly
  [callee] listening on sip:callee@127.0.0.1:5061
  [callee] incoming call from User <sip:caller@127.0.0.1:5060>;tag=…
  [callee] answered session-…
  [callee] waiting for ICE connectivity check…
  [callee] ICE connected — selected pair remote address: 127.0.0.1:…
  [callee] call ended

DEMO SUCCESSFUL — ICE connectivity check completed on both sides
```

## Command-line options

| Binary | Flag | Default | Meaning |
|--------|------|---------|---------|
| `caller` | `--port` | `5060` | Local SIP/UDP port to bind |
| `caller` | `--peer-port` | `5061` | Callee's SIP port to dial |
| `callee` | `--port` | `5061` | Local SIP/UDP port to bind |

Set `RUST_LOG=info` (or `debug`) for stack-level tracing.

## Experimental scope notes

- **Host + server-reflexive candidates only.** No TURN/relay candidates, no
  trickle ICE — candidates are gathered in full before the offer/answer is
  sent.
- **Requires `Config::media_mode = MediaMode::Enabled`** (the default) — ICE
  needs a live RTP socket to bind to; `Config::validate` rejects
  `enable_ice = true` with `MediaMode::SignalingOnly`.
- Reuses `Config::stun_server` for server-reflexive candidate gathering —
  the same knob already used for the boot-time public-address probe. Not
  set here, so this demo only ever gathers host candidates (loopback in
  this case).
- No TURN, no ICE restart, no re-INVITE renegotiation of candidates.

## Troubleshooting

- **`Address already in use`** — another process holds `:5060`/`:5061`. Pass
  different `--port` / `--peer-port` values.
- **`no IceConnected event observed`** — the connectivity check didn't
  resolve within 10s. Check `RUST_LOG=debug` output for `webrtc_ice` state
  transitions.

## Next steps

- [07-secure-call-srtp](../07-secure-call-srtp/) — combine ICE with
  mandatory SDES-SRTP by also setting `Config::offer_srtp` /
  `srtp_required` (not shown here — plaintext keeps this example focused).
- API-tier reference: `cargo run -p rvoip-sip --example stream_peer_ice_alice
  --features ice` (this example is the two-process productization of that
  in-crate example).

[`StreamPeer`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.StreamPeer.html
