# rvoip media fuzz targets

libFuzzer harnesses for the `rvoip-rtp-core` (and `rvoip-media-core` payload)
parse surfaces that face untrusted network input. Mirrors `crates/sip/fuzz`.

## Targets

| target           | entry point                          | surface |
|------------------|--------------------------------------|---------|
| `rtp_packet`     | `RtpPacket::parse`                   | inbound RTP |
| `rtcp_packet`    | `RtcpPacket::parse`                  | inbound RTCP (compound) |
| `srtp_unprotect` | `SrtpContext::unprotect` (fixed key) | SRTP decrypt + auth tag |
| `dtls_record`    | `Record::parse_multiple`             | DTLS record layer |
| `stun_response`  | `decode_binding_response`            | STUN / ICE NAT discovery |
| `g711_unpack`    | `G711UPayloadFormat::unpack`         | payload depacketize |

## Run

```sh
cargo +nightly fuzz run --fuzz-dir crates/media/fuzz <target>
```

Smoke run (matches `beta_gate.sh`):

```sh
cargo +nightly fuzz run --fuzz-dir crates/media/fuzz <target> -- -runs=1000 -max_total_time=10
```

## Follow-ups

- Opus / VP8 / VP9 payload targets (need the media-core `opus` feature + libopus).
- Dedicated DTLS handshake-message body targets (today reached via `dtls_record`).
