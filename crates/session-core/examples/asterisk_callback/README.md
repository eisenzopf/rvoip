# Asterisk CallbackPeer Interop Examples

These examples validate the `CallbackPeer` API surface against the same real
Asterisk PBX profile used by `examples/asterisk`.

The suite reuses `examples/asterisk/.env`, the same endpoint map, and the same
TLS/SRTP requirements:

| Users | Transport / media | Callback coverage |
|-------|-------------------|-------------------|
| `1001`, `1002`, `1003` | SIP TLS + mandatory SDES-SRTP | registration, hold/resume, ring/cancel, DTMF, reject, blind transfer, SRTP tone analysis |
| `2001`, `2002`, `2003` | UDP + RTP | registration, hold/resume, ring/cancel, DTMF, reject, blind transfer |

## Commands

| Command | Coverage |
|---------|----------|
| `./run.sh` | Callback registration plus TLS/SRTP and UDP hold/resume |
| `ASTERISK_RUN_EXTENDED_TESTS=1 ./run.sh` | Full callback suite |
| `./run_extended.sh` | Extended callback scenarios only |

## Callback API Surfaces Exercised

The examples use `CallbackPeer`, `CallbackPeerControl`, `CallHandler`, and
`SessionHandle`. They intentionally do not use `StreamPeer`.

This suite is the CallbackPeer parity gate for the StreamPeer Asterisk suite:
registration, answered calls, reject, hold/resume, ring/cancel, DTMF, blind
transfer, and SRTP audio analysis should remain aligned unless a PBX profile
limitation is called out explicitly.

The suite validates these callback hooks:

| Hook | Scenario |
|------|----------|
| `on_registration_success`, `on_unregistration_success` | registration and every registered endpoint |
| `on_call_established`, `on_call_ended` | answered calls |
| `on_call_failed` | callback reject tests with `486 Busy Here` |
| `on_call_cancelled` | ring/cancel caller; endpoint-side CANCEL is logged when Asterisk forwards it, but this PBX profile may complete cancellation server-side |
| `on_call_on_hold`, `on_call_resumed` | hold/resume caller |
| `on_remote_call_on_hold`, `on_remote_call_resumed` | optional callee assertion when `ASTERISK_EXPECT_REMOTE_HOLD_EVENTS=1` |
| `on_dtmf` | DTMF tests |
| `on_transfer_accepted`, `on_transfer_progress`, `on_transfer_completed`, `on_transfer_failed` | blind transfer tests |

## Audio Verification

TLS/SRTP answered scenarios verify decrypted media by recording WAV files and
checking expected tones:

| Test | WAV assertions |
|------|----------------|
| TLS/SRTP hold/resume | `1001` receives `880 Hz`; `1002` receives `440 Hz` before hold and `660 Hz` after resume |
| TLS/SRTP DTMF | `1001` receives `880 Hz`; `1002` receives `440 Hz` |
| TLS/SRTP blind transfer | `1001` receives `880 Hz`; `1003` receives `880 Hz`; `1002` receives `440 Hz` before transfer and `660 Hz` after transfer |

UDP hold/resume keeps the same RTP audio analyzer as the StreamPeer suite.
