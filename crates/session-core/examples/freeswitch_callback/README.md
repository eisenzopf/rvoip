# FreeSWITCH CallbackPeer Interop Examples

These examples validate the `CallbackPeer` API surface against the same local
FreeSWITCH/Sofia profiles used by `examples/freeswitch`.

The suite reuses `examples/freeswitch/.env`, `/Users/jonathan/Developer/freeswitch/freeswitch-local.env`,
the same endpoint map, and the same TLS/SRTP requirements:

| Users | Transport / media | Callback coverage |
|-------|-------------------|-------------------|
| `1001`, `1002`, `1003` | SIP TLS + mandatory SDES-SRTP | registration, hold/resume, ring/cancel, DTMF, reject, blind transfer, SRTP tone analysis |
| `2001`, `2002`, `2003` | UDP + RTP | registration, hold/resume, ring/cancel, DTMF, reject, blind transfer |

## Commands

| Command | Coverage |
|---------|----------|
| `./run.sh` | Callback registration plus TLS/SRTP and UDP hold/resume |
| `FREESWITCH_RUN_EXTENDED_TESTS=1 ./run.sh` | Full callback suite |
| `./run_extended.sh` | Extended callback scenarios only |

## Callback API Surfaces Exercised

The examples use `CallbackPeer`, `CallbackPeerControl`, `CallHandler`, and
`SessionHandle`. They intentionally do not use lower SIP dialog internals.
Callback hooks are the developer-facing observation path; the internal event
queue in these examples exists only so the runner can assert that each hook
fired.

This suite is the CallbackPeer parity gate for the StreamPeer FreeSWITCH suite:
registration, answered calls, reject, hold/resume, ring/cancel, DTMF, blind
transfer, and SRTP audio analysis should remain aligned unless a FreeSWITCH
profile limitation is called out explicitly.

FreeSWITCH blind transfer tests intentionally wait for the final REFER NOTIFY
with `transfer_blind_and_wait_for_outcome(..., TransferWaitMode::NotifyFinal, ...)`.
FreeSWITCH does not provide trustworthy target-answer evidence on this path, so
the examples validate `TransferOutcome::ReferCompleted`, transferred-leg media,
and channel cleanup rather than treating the REFER final NOTIFY as
replacement-call lifecycle proof.

The suite validates these callback hooks:

| Hook | Scenario |
|------|----------|
| `on_registration_success`, `on_unregistration_success` | registration and every registered endpoint |
| `on_call_established`, `on_call_ended` | answered calls |
| `on_call_progress` | ring/cancel caller progress (`180` / `183`) |
| `on_call_failed` | callback reject tests with `486 Busy Here` |
| `on_call_cancelled` | ring/cancel caller and target-side deferred-call cancellation |
| `on_call_on_hold`, `on_call_resumed` | hold/resume caller |
| `on_remote_call_on_hold`, `on_remote_call_resumed` | optional callee assertion when `FREESWITCH_EXPECT_REMOTE_HOLD_EVENTS=1` |
| `on_dtmf` | DTMF tests |
| `on_media_security_negotiated` | TLS/SRTP scenarios; typed SDES suite/profile/context state without key material |
| `on_transfer_accepted`, `on_refer_progress`, `on_refer_completed`, `on_transfer_failed` | blind transfer tests |

Commands issued from callbacks or scenario code still use handle-first APIs:
`SessionHandle::wait_for_media_security`, `hangup_and_wait`, and
`transfer_blind_and_wait_for_outcome`. Answer/progress observation stays
callback-native through `on_call_established` and `on_call_progress`.

## Audio Verification

TLS/SRTP answered scenarios verify decrypted media by recording WAV files and
checking expected tones:

| Test | WAV assertions |
|------|----------------|
| TLS/SRTP hold/resume | `1001` receives `880 Hz`; `1002` receives `440 Hz` before hold and `660 Hz` after resume |
| TLS/SRTP DTMF | `1001` receives `880 Hz`; `1002` receives `440 Hz` |
| TLS/SRTP blind transfer | `1001` receives `880 Hz`; `1003` receives `880 Hz`; `1002` receives `440 Hz` before transfer and `660 Hz` after transfer |

UDP hold/resume keeps the same RTP audio analyzer as the StreamPeer suite.
