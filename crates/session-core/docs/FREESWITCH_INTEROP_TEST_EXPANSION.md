# FreeSWITCH Interop Test Expansion

## Summary

The FreeSWITCH examples now target the same endpoint layout as the Asterisk
StreamPeer examples:

| Users | FreeSWITCH profile | Transport / media | Purpose |
| --- | --- | --- | --- |
| `1001-1004` | `rvoip_tls_srtp` | SIP TLS + mandatory SDES-SRTP | secure parity scenarios |
| `2001-2004` | `rvoip_udp` | SIP UDP/TCP + plain RTP | plaintext parity scenarios |

The FreeSWITCH Docker/profile configuration remains outside this repository in
`/Users/jonathan/Developer/freeswitch`. The rvoip repository contains only the
session-core examples, runners, and this design note.

## FreeSWITCH Profile Requirements

The local FreeSWITCH container must provide:

- `rvoip_udp` on `5062` for users `2001-2004`.
- `rvoip_tls_srtp` on `5063` for users `1001-1004`.
- A `rvoip` dialplan context that directly bridges those extension ranges with
  `user/${dialed_extension}@${domain_name}`.
- User password `1234` unless `FS_DEFAULT_PASSWORD` overrides it.
- TLS server certs generated under the FreeSWITCH config tree.
- Mandatory SRTP on the TLS profile without pinning a crypto suite list.

The TLS/SRTP profile intentionally sets `rtp_secure_media=mandatory` but does
not set `rtp_secure_media_suites` and does not append suites to
`rtp_secure_media`. That keeps the tests focused on SDP negotiation with
FreeSWITCH defaults.

## Example Coverage

| Scenario | Users | Asterisk analogue | Assertions |
| --- | --- | --- | --- |
| Registration | `2001`, `1001` | `asterisk/registration` | REGISTER and unregister over UDP and TLS |
| Basic UDP call | `2001 -> 2002` | FreeSWITCH-specific smoke | call answers and tears down |
| UDP hold/resume | `2001 -> 2002` | `asterisk/udp_hold_resume` | local hold/resume state plus WAV tone analysis |
| UDP ring/cancel | `2001 -> 2003` | `asterisk/udp_ring_remote` | target rings, caller cancels, target never answers |
| UDP DTMF | `2001 -> 2002` | `asterisk/udp_dtmf` | ordered RFC 4733 digits are received |
| UDP blind transfer | `2001`, `2002`, `2003` | `asterisk/udp_blind_transfer_remote` | REFER completion and transferred call answer |
| TLS/SRTP hold/resume | `1001 -> 1002` | `asterisk/tls_srtp_hold_resume` | TLS/SRTP wire evidence, SRTP context installation, WAV tone analysis |
| TLS/SRTP ring/cancel | `1001 -> 1003` | `asterisk/tls_srtp_ring_remote` | TLS/SRTP INVITE evidence and clean CANCEL |
| TLS/SRTP DTMF | `1001 -> 1002` | `asterisk/tls_srtp_dtmf` | DTMF sequence plus SRTP audio analysis |
| TLS/SRTP blind transfer | `1001`, `1002`, `1003` | `asterisk/tls_srtp_blind_transfer_remote` | REFER completion plus transferred-leg SRTP audio analysis |

TLS/SRTP answered scenarios assert:

- `sips:` URI usage.
- `transport=tls`.
- `SIP/2.0/TLS`.
- `RTP/SAVP`.
- At least one `a=crypto`.
- `SRTP_DIAG sdes_*` suite negotiation diagnostics.
- `SRTP_DIAG srtp_contexts_installed`.
- No `proceeding plaintext` fallback.

Ring/cancel is signaling-only, so it asserts the TLS/SRTP offer evidence but
does not require installed SRTP contexts.

## Runners

Top-level default sequence:

```sh
crates/session-core/examples/freeswitch/run.sh
```

Runs:

1. Registration smoke test.
2. Basic UDP call.
3. UDP hold/resume.
4. TLS/SRTP hold/resume.

Extended sequence:

```sh
crates/session-core/examples/freeswitch/run_remote.sh
```

Runs:

1. TLS/SRTP ring/cancel.
2. UDP ring/cancel.
3. TLS/SRTP DTMF.
4. UDP DTMF.
5. TLS/SRTP blind transfer.
6. UDP blind transfer.

Set `FREESWITCH_RUN_EXTENDED_TESTS=1` to include the extended sequence from
`run.sh`.

## Environment

The examples load the generated container profile defaults from:

```sh
/Users/jonathan/Developer/freeswitch/freeswitch-local.env
```

The checked-in example defaults assume localhost, but the generated
`freeswitch-local.env` normally supplies the Docker/Colima addresses, for
example `192.168.64.2:5062` and `192.168.64.2:5063` plus an advertised rvoip
host address such as `192.168.64.1`. Process environment values take
precedence over `.env` files.

Important variables:

```sh
FREESWITCH_UDP_ADDR=<freeswitch-host>:5062
FREESWITCH_TLS_ADDR=<freeswitch-host>:5063
FREESWITCH_PASSWORD=1234
RVOIP_LOCAL_IP=<rvoip-bind-ip>
RVOIP_ADVERTISED_IP=<rvoip-contact-ip>
RVOIP_MEDIA_ADVERTISED_IP=<rvoip-rtp-ip>
FREESWITCH_TEST_TIMEOUT_SECS=60
FREESWITCH_TEST_DIGITS=1234#
FREESWITCH_EXPECT_REMOTE_HOLD_EVENTS=0
FREESWITCH_RING_CANCEL_DELAY_SECS=2
FREESWITCH_TRANSFER_SETTLE_SECS=3
FREESWITCH_TLS_CONTACT_MODE=reachable-contact
FREESWITCH_TLS_SRTP_REQUIRED=1
```

`examples/freeswitch/.env` and process environment values may override these
defaults.

The FreeSWITCH UDP blind-transfer scenario intentionally waits briefly before
sending REFER. FreeSWITCH can accept an immediate REFER before the bridge has
settled and report success without re-entering the dialplan for the transfer
target; `FREESWITCH_TRANSFER_SETTLE_SECS` keeps this parity example stable.

## Acceptance Criteria

- The FreeSWITCH container starts with `internal`, `rvoip_udp`, and
  `rvoip_tls_srtp` profiles running.
- `show registrations` shows the expected users during scenario execution.
- All Cargo examples listed in `crates/session-core/Cargo.toml` build.
- `examples/freeswitch/run.sh` passes.
- `examples/freeswitch/run_remote.sh` passes.
- `FREESWITCH_RUN_EXTENDED_TESTS=1 examples/freeswitch/run.sh` passes if the
  full sequence is preferred from one entry point.
- Compatibility docs should only move FreeSWITCH from planned to validated
  after those commands pass repeatably on a clean container restart.
