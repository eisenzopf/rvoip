# Asterisk Interop Examples

These examples validate `rvoip-session-core` against a real Asterisk PBX. They
use `examples/asterisk/.env` for PBX address, credentials, bind addresses,
advertised addresses, media ports, and TLS options.

This directory exercises the StreamPeer API. The companion
`examples/asterisk_callback` suite exercises the CallbackPeer API against the
same Asterisk profile and should also pass before release.

## Endpoint Map

| User | Role | Transport / media | Notes |
|------|------|-------------------|-------|
| `1001` | rvoip endpoint | SIP TLS + mandatory SDES-SRTP | caller in TLS/SRTP automated tests |
| `1002` | rvoip endpoint | SIP TLS + mandatory SDES-SRTP | callee in TLS/SRTP automated tests |
| `1003` | rvoip endpoint | SIP TLS + mandatory SDES-SRTP | third endpoint for ring/cancel and transfer tests |
| `1004` | optional rvoip endpoint | SIP TLS + SRTP | reserved for bridge/conference expansion |
| `2001` | rvoip endpoint | UDP + plain RTP | caller in UDP automated tests |
| `2002` | rvoip endpoint | UDP + plain RTP | callee in UDP automated tests |
| `2003` | rvoip endpoint | UDP + plain RTP | third endpoint for ring/cancel and transfer tests |
| `2004` | optional rvoip endpoint | UDP + plain RTP | reserved for bridge/conference expansion |

## Current Automated Tests

| Command | Coverage | Success criteria |
|---------|----------|------------------|
| `./registration/run.sh` | Registers TLS user `1001`, then UDP user `2001` | REGISTER succeeds, each endpoint unregisters cleanly |
| `./tls_srtp_hold_resume/run.sh` | `1001` calls `1002` over TLS/SRTP, holds, resumes, exchanges tones | TLS/SIPS and SRTP wire evidence is logged, optional remote hold/resume events pass when enabled, pre/post-resume audio passes analysis |
| `./tls_srtp_registered_flow/run.sh` | `1001` calls `1002` over TLS/SRTP while both endpoints receive inbound SIP requests on the REGISTER TLS flow | Registered-flow mode and keep-alive evidence is logged, no rvoip TLS listener cert is generated, pre/post-resume audio passes analysis |
| `./udp_hold_resume/run.sh` | `2001` calls `2002` over UDP/RTP, holds, resumes, exchanges tones | Optional remote hold/resume events pass when enabled, pre/post-resume audio passes analysis |
| `./run.sh` | Full default sequence | Registration plus reachable-contact TLS/SRTP hold/resume and UDP hold/resume pass |

Hold/resume re-INVITE propagation to the callee is PBX-profile dependent.
The current default verifies caller-side hold/resume plus audio continuity.
Set `ASTERISK_EXPECT_REMOTE_HOLD_EVENTS=1` only when the Asterisk profile is
expected to forward hold/resume target-refreshes to the callee.

## Extended Multi-Endpoint Tests

Extended tests use additional rvoip-controlled endpoints `1003` for TLS/SRTP
and `2003` for UDP/RTP. They are intentionally outside the default `run.sh`
sequence unless `ASTERISK_RUN_EXTENDED_TESTS=1` is set. The older
`ASTERISK_RUN_REMOTE_TESTS=1` flag is still accepted as an alias.

| Command | Transport | Endpoint behavior | Success criteria |
|---------|-----------|-----------------|------------------|
| `./tls_srtp_ring_remote/run.sh` | TLS/SRTP | `1003` registers but does not answer | `1001` calls `1003`, reaches `Ringing`, then cancels cleanly |
| `./udp_ring_remote/run.sh` | UDP/RTP | `2003` registers but does not answer | `2001` calls `2003`, reaches `Ringing`, then cancels cleanly |
| `./tls_srtp_dtmf/run.sh` | TLS/SRTP | none | `1001` sends `REMOTE_TEST_DIGITS` to `1002`, `1002` receives them, and the SRTP audio analyzer verifies exchanged tones |
| `./udp_dtmf/run.sh` | UDP/RTP | none | `2001` sends `REMOTE_TEST_DIGITS` to `2002` through Asterisk and `2002` receives them |
| `./tls_srtp_blind_transfer_remote/run.sh` | TLS/SRTP | `1003` registers and answers the transferred call | `1001` calls `1002`, sends REFER to transfer `1002` to `1003`, observes REFER NOTIFY completion, and the SRTP audio analyzer verifies initial and transferred-leg tones |
| `./udp_blind_transfer_remote/run.sh` | UDP/RTP | `2003` registers and answers the transferred call | `2001` calls `2002`, sends REFER to transfer `2002` to `2003`, and observes REFER NOTIFY completion |
| `./run_remote.sh` | TLS/SRTP and UDP/RTP | `1003`/`2003` are rvoip-controlled | All extended scenarios pass |

## SRTP Audio Verification

| Test | WAV assertions |
|------|----------------|
| TLS/SRTP hold/resume | `1001` receives `1002`'s `880 Hz` tone; `1002` receives `1001`'s pre-hold `440 Hz` and post-resume `660 Hz` tones |
| TLS/SRTP registered-flow | Same audio assertions as TLS/SRTP hold/resume, plus registered-flow mode and keep-alive log assertions |
| TLS/SRTP DTMF | `1001` receives `1002`'s `880 Hz` tone; `1002` receives `1001`'s `440 Hz` tone while DTMF is sent |
| TLS/SRTP blind transfer | `1001` receives `1002`'s `880 Hz` initial-leg tone; `1003` receives `1002`'s `880 Hz` transferred-leg tone; `1002` receives `1001`'s `440 Hz` before transfer and `1003`'s `660 Hz` after transfer |
| TLS/SRTP ring/cancel | Signaling-only; no media assertion because the target intentionally never answers |

## Extended Test Environment Knobs

These values are optional overrides. The examples default to the values shown:

```sh
REMOTE_TLS_USER=1003
REMOTE_UDP_USER=2003
REMOTE_TEST_DIGITS=1234#
REMOTE_TEST_TIMEOUT_SECS=60
ASTERISK_CALL_RETRY_ATTEMPTS=8
```

The `REMOTE_*` names are retained for compatibility with the original test
plan. In the current suite they identify rvoip-controlled third endpoints and
shared test inputs, not external softphones. `ASTERISK_CALL_RETRY_ATTEMPTS`
defaults to `8` and covers transient PBX lookup windows immediately after
registration, where Asterisk can briefly return `404 Not Found` for an
extension that has just registered.

Optional URI overrides are available when Asterisk dialplan routing differs
from direct extension dialing:

```sh
REMOTE_TLS_CALL_URI=sips:1003@192.168.1.103:5061;transport=tls
REMOTE_UDP_CALL_URI=sip:2003@192.168.1.103:5060
```

## TLS Notes

The TLS/SRTP examples require Asterisk PJSIP TLS transport and endpoint media
encryption configured for SDES-SRTP. The default contact mode is
`reachable-contact`, where rvoip registers over TLS and also listens on the
configured endpoint TLS Contact port so Asterisk can open inbound TLS requests.

Registered-flow modes are also supported with:

```sh
ASTERISK_TLS_CONTACT_MODE=registered-flow-rfc5626
ASTERISK_TLS_CONTACT_MODE=registered-flow-symmetric
```

These modes reuse the outbound registration flow and do not require an endpoint
listener certificate/key.

The registered-flow integration test is available as:

```sh
./tls_srtp_registered_flow/run.sh
```

It forces:

```sh
ASTERISK_TLS_CONTACT_MODE=registered-flow-symmetric
ASTERISK_TLS_FLOW_REUSE=1
SIP_TRANSPORT=TLS
ASTERISK_TLS_SRTP_REQUIRED=1
```

The test verifies that registered-flow mode is active, symmetric keep-alive is
started, no local rvoip TLS listener certificate is generated, and the usual
TLS/SRTP hold/resume audio assertions pass.

The default `./run.sh` sequence leaves this test disabled because the Asterisk
profile must route TLS Contacts back over the registration flow. Set
`ASTERISK_RUN_FLOW_REUSE_TESTS=1` to include it in the top-level runner. For
the local Docker Asterisk profile in `/Users/jonathan/Developer/asterisk`, use:

```sh
/Users/jonathan/Developer/asterisk/scripts/run-rvoip-flow-reuse-tests.sh
```

That helper temporarily enables `rewrite_contact = yes` for the TLS endpoint
template, runs this example, and restores the default reachable-contact
configuration afterward.
