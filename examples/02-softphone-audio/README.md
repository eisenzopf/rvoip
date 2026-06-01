# 02 · Softphone with real audio (PCMU)

> **Beta status: Supported.** PCMU/PCMA full-media over RTP is in the beta
> contract. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

Two softphone processes place a call and exchange **real PCMU audio** over RTP.
To make the demo self-verifying and hardware-free, each side sends a pure tone
(caller 440 Hz, callee 880 Hz) and runs a Goertzel filter over what it receives
to confirm the correct tone arrived with enough energy and low noise.

It uses the [`Endpoint`] surface and the duplex audio API: `call.audio()` →
`AudioStream::split()` gives a sender and receiver of `EndpointAudioFrame`s.

> **Want a real mic/speaker softphone?** The in-crate `sip_client` example is a
> full terminal softphone with CPAL audio I/O and a TUI:
> `cargo run -p rvoip-sip --example sip_client`.

## Demo flow

1. **callee** builds an `Endpoint` on `:5073` and waits for a call.
2. **caller** builds an `Endpoint` on `:5072` and dials the callee.
3. Both answer and split their audio stream into a sender + receiver.
4. caller streams 440 Hz; callee streams 880 Hz (~3 s, 20 ms frames).
5. Each side verifies the received tone (frequency, energy, dominance), prints a
   report, and the caller hangs up.

## Architecture

```
   caller (:5072)                          callee (:5073)
   media 17200-17249                        media 17250-17299
        │  ── INVITE / 200 OK / ACK ────────▶ │
        │  ══ RTP PCMU 440 Hz ══════════════▶ │   callee verifies 440 Hz
        │  ◀══════════════ RTP PCMU 880 Hz ══ │   caller verifies 880 Hz
        │  ── BYE ──────────────────────────▶ │
```

## Quick start

```sh
./run_demo.sh
```

Or manually, in two terminals:

```sh
cargo run --bin callee -- --port 5073
cargo run --bin caller -- --port 5072 --peer-port 5073
```

## Expected output

```text
  [caller] calling sip:callee@127.0.0.1:5073
  [caller] call answered as session-ff05ebad-…
  ✅ caller received callee's 880 Hz tone: samples=24000, dominant=880 Hz, dominance=…x, rms=0.213
  [callee] incoming call from User <sip:caller@127.0.0.1:5072>;tag=…
  ✅ callee received caller's 440 Hz tone: samples=24000, dominant=440 Hz, dominance=…x, rms=0.213

✅ DEMO SUCCESSFUL — bidirectional PCMU media verified
```

## Command-line options

| Binary | Flag | Default | Meaning |
|--------|------|---------|---------|
| `caller` | `--port` / `--peer-port` | `5072` / `5073` | Local / callee SIP port |
| `callee` | `--port` | `5073` | Local SIP port |

Each side uses a distinct media port range (caller `17200-17249`, callee
`17250-17299`) so both can run on one host.

## Beta scope notes

- Media is **PCMU (G.711 µ-law)**, the beta full-media codec; PCMA is also
  supported. **Opus / G.722 / G.729 are post-beta** and not used here.
- This is a headless tone exchange so it runs in CI. Real device capture/playback
  lives in the in-crate `sip_client` example.

## Troubleshooting

- **"received only N samples"** — media didn't flow long enough; ensure the
  callee was up before the caller dialed (`run_demo.sh` handles ordering).
- **Port conflicts** — change `--port`/`--peer-port`; the media ranges are fixed
  in code (`src/lib.rs`) if you need to move them too.

## Next steps

- [04-call-control](../04-call-control/) — hold, resume, DTMF on a live call.
- [07-secure-call-srtp](../07-secure-call-srtp/) — encrypt this media with SRTP.
- In-crate reference: `endpoint/04_audio_roundtrip` (the single-process original).

[`Endpoint`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.Endpoint.html
