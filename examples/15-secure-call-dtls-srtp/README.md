# 15 · Secure media with DTLS-SRTP

> **Status: Experimental, not beta-scoped.** DTLS-SRTP lives behind
> rvoip-sip's off-by-default `dtls-srtp` feature. The handshake is real
> (a genuine DTLS 1.2 exchange over the RTP port), but it's not part of
> the `rvoip-sip` 0.2.x beta contract yet. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).
> Compare with [07-secure-call-srtp](../07-secure-call-srtp/), the beta
> SDES-SRTP equivalent.

## Overview

A call secured with **DTLS-SRTP** (RFC 5763/5764) instead of SDES. Setting
`Config::srtp_keying = SrtpKeyingMode::DtlsSrtp` (alongside `offer_srtp =
true`) makes the media adapter advertise `m=audio … UDP/TLS/RTP/SAVP …`
with `a=fingerprint`/`a=setup` instead of `a=crypto:` lines. Unlike SDES —
where the SRTP keys are carried directly in the SDP and are ready the
moment signaling completes — DTLS-SRTP's keys come from a real DTLS 1.2
handshake run over the RTP port **after** the 200 OK/ACK, asynchronously,
in the background. Call setup is never gated on it: both sides answer the
call first, then separately observe `Event::MediaSecurityNegotiated` once
the handshake actually finishes.

## Demo flow

1. **server** (`:5060`) listens with DTLS-SRTP offered (a `CallbackPeer`).
2. **client** (`:5062`) places a DTLS-SRTP call.
3. Signaling completes (200 OK/ACK) with fingerprints exchanged in the SDP.
4. Both sides run the DTLS 1.2 handshake over the RTP port in the
   background, verify the peer's certificate against the fingerprint the
   SDP promised, and install paired `SrtpContext`s.
5. Both sides observe `Event::MediaSecurityNegotiated { keying: DtlsSrtp,
   .. }` and print it. Client hangs up.

## Architecture

```
   client (:5062)                                    server (:5060)
        │  ── INVITE  m=audio UDP/TLS/RTP/SAVP      ▶ │
        │     a=fingerprint:sha-256 …  a=setup:actpass
        │  ◀── 200 OK  a=fingerprint:sha-256 … a=setup:active │
        │  ── ACK ─────────────────────────────────▶ │
        │  ◀═══ DTLS 1.2 handshake (over RTP port) ══▶ │   (background)
        │  ══ SRTP (encrypted RTP) ══════════════════▶ │
        │  ◀════════════ SRTP (encrypted RTP) ═══════ │
        │  ── BYE ─────────────────────────────────▶ │
```

## Quick start

```sh
./run_demo.sh
```

Or manually:

```sh
cargo run --bin server
cargo run --bin client
```

## Expected output

```text
  Placing DTLS-SRTP call to sip:server@127.0.0.1:5060…
  Call answered — waiting for the DTLS-SRTP handshake to complete…
  media secured — keying=DtlsSrtp suite=AesCm128HmacSha1_80 profile=UdpTlsRtpSavp contexts_installed=true
  DTLS-SRTP call done.

  Listening on 5060 with DTLS-SRTP (UDP/TLS/RTP/SAVP + a=fingerprint)…
  [SERVER] Incoming DTLS-SRTP call: … -> …
  [SERVER] Call … established (signaling only — DTLS handshake runs in the background)
  [SERVER] media secured — keying=DtlsSrtp suite=AesCm128HmacSha1_80 profile=UdpTlsRtpSavp contexts_installed=true
  [SERVER] Call … ended: …

DEMO SUCCESSFUL — DTLS-SRTP handshake completed on both sides
```

## Experimental scope notes

- **Requires the `dtls-srtp` feature** (declared on this example's own
  `rvoip-sip` path dependency) — off by default.
- **Certificate fingerprint pinning only.** No long-lived certificate
  identity across calls; a fresh identity is generated per session.
- **No SRTP-mandatory refusal path for DTLS specifically** — unlike
  [07-secure-call-srtp](../07-secure-call-srtp/)'s `srtp_required`, which
  only governs the SDES branch. Offering DTLS-SRTP always attempts it.
- Combine with real ICE via
  [14-ice-nat-traversal](../14-ice-nat-traversal/)'s `Config::enable_ice`
  (not shown here — kept separate to keep each example focused); the two
  features ride the same shared RTP socket without interfering — see
  `crates/sip/rvoip-sip/tests/ice_call_integration.rs`.

## Troubleshooting

- **`Address already in use`** — another process holds `:5060`/`:5062`.
- **`no MediaSecurityNegotiated event observed`** — the handshake didn't
  resolve within 10s. Check `RUST_LOG=debug` output for
  `run_dtls_handshake_and_record` errors.

## Next steps

- [07-secure-call-srtp](../07-secure-call-srtp/) — the beta SDES-SRTP
  equivalent, no feature flag required.
- API-tier reference: `cargo run -p rvoip-sip --example
  stream_peer_dtls_srtp_alice --features dtls-srtp` (this example is the
  two-process productization of that in-crate example).
