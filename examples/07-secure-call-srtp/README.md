# 07 · Secure media with SDES-SRTP

> **Beta status: Partial — SDES-SRTP, limited suites.** SDES key exchange with
> AES-CM-128/HMAC-SHA1-80 (and -32) is supported; **DTLS-SRTP is post-beta**. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

A call with **mandatory SDES-SRTP** media encryption. Setting
`Config::offer_srtp = true` makes the media adapter advertise `RTP/SAVP` with
`a=crypto:` lines (RFC 4568); `srtp_required = true` refuses any plaintext
fallback — if the peer answers without accepting an offered suite, the call
aborts. This mirrors a carrier's `srtp=mandatory` profile.

## Demo flow

1. **server** (`:5060`) listens with SRTP mandatory (a `CallbackPeer`).
2. **client** (`:5062`) places an SRTP-mandatory call.
3. SDES negotiates a crypto suite; paired SRTP contexts are installed; RTP audio
   is AES-CM-128/HMAC-SHA1-80 encrypted on the wire. Client hangs up.

## Architecture

```
   client (:5062)                              server (:5060)
        │  ── INVITE  m=audio RTP/SAVP        ▶ │
        │     a=crypto:1 AES_CM_128_HMAC_SHA1_80 …
        │  ◀── 200 OK  a=crypto:1 (chosen suite) │
        │  ══ SRTP (encrypted RTP) ════════════▶ │
        │  ◀════════════ SRTP (encrypted RTP) ══ │
        │  ── BYE ─────────────────────────────▶ │
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
  [SERVER] Incoming SRTP-required call: … -> …
  [SERVER] ✅ Call … established with SRTP
  ✅ Call answered — SRTP negotiation completed, media is encrypted.
  SRTP call done.

✅ DEMO SUCCESSFUL — encrypted SRTP call established
```

## Beta scope notes

- **SDES-SRTP only**, with the tested suites AES-CM-128/HMAC-SHA1-80 and -32.
- **DTLS-SRTP, ICE/TURN, and WebRTC are post-beta** and not exercised here.
- `srtp_required = true` is the "no plaintext fallback" posture; drop it to allow
  negotiating down to RTP when a peer can't do SRTP.

## Troubleshooting

- **Call aborts immediately** — with `srtp_required = true`, a peer that doesn't
  accept an offered crypto suite causes a terminal failure. That's by design.
- **Port conflicts** — the ports are fixed in source (`5060` / `5062`); edit
  `src/server.rs` / `src/client.rs` to change them.

## Next steps

- [08-tls-transport](../08-tls-transport/) — encrypt the *signaling* with TLS.
- In-crate reference: `regression/03_srtp`.
