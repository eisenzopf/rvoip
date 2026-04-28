# Asterisk Interop Examples

These examples validate `rvoip-session-core` against a real Asterisk PBX. They
use `examples/asterisk/.env` for PBX address, credentials, bind addresses,
advertised addresses, media ports, and TLS options.

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
| `./udp_hold_resume/run.sh` | `2001` calls `2002` over UDP/RTP, holds, resumes, exchanges tones | Optional remote hold/resume events pass when enabled, pre/post-resume audio passes analysis |
| `./run.sh` | Full default sequence | Registration plus both hold/resume variants pass |

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
| `./tls_srtp_dtmf/run.sh` | TLS/SRTP | none | `1001` sends `REMOTE_TEST_DIGITS` to `1002` through Asterisk and `1002` receives them |
| `./udp_dtmf/run.sh` | UDP/RTP | none | `2001` sends `REMOTE_TEST_DIGITS` to `2002` through Asterisk and `2002` receives them |
| `./tls_srtp_blind_transfer_remote/run.sh` | TLS/SRTP | `1003` registers and answers the transferred call | `1001` calls `1002`, sends REFER to transfer `1002` to `1003`, and observes REFER NOTIFY completion |
| `./udp_blind_transfer_remote/run.sh` | UDP/RTP | `2003` registers and answers the transferred call | `2001` calls `2002`, sends REFER to transfer `2002` to `2003`, and observes REFER NOTIFY completion |
| `./run_remote.sh` | TLS/SRTP and UDP/RTP | `1003`/`2003` are rvoip-controlled | All extended scenarios pass |

## Required Extended Test Environment

Add these values to `.env` or export them before running extended tests:

```sh
REMOTE_TLS_USER=1003
REMOTE_UDP_USER=2003
REMOTE_TEST_DIGITS=1234#
REMOTE_TEST_TIMEOUT_SECS=60
ASTERISK_CALL_RETRY_ATTEMPTS=8
```

The `REMOTE_*` names are retained for compatibility with the original test
plan. In the current suite they identify rvoip-controlled third endpoints and
shared test inputs, not external softphones.

`ASTERISK_CALL_RETRY_ATTEMPTS` covers transient PBX lookup windows immediately
after registration, where Asterisk can briefly return `404 Not Found` for an
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
