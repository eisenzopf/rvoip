# FreeSWITCH Interop Examples

This directory is the starter FreeSWITCH/Sofia profile for `session-core`.
It intentionally stays smaller than the Asterisk harness while the B2BUA
wrapper design is still being sketched.

## Current Scope

- UDP/RTP registration smoke test.
- UDP/RTP two-endpoint call smoke test.
- Config defaults target the FreeSWITCH internal profile on `127.0.0.1:5060`.

## Environment

```sh
FREESWITCH_ADDR=127.0.0.1:5060
FREESWITCH_USER=1000
FREESWITCH_PASSWORD=1234
FREESWITCH_CALLER_USER=1000
FREESWITCH_CALLER_PASSWORD=1234
FREESWITCH_CALLEE_USER=1001
FREESWITCH_CALLEE_PASSWORD=1234
FREESWITCH_TARGET_USER=1001
RVOIP_LOCAL_IP=127.0.0.1
FREESWITCH_TEST_TIMEOUT_SECS=30
```

## Commands

```sh
./registration/run.sh
./udp_call/run.sh
```

Next coverage should add DTMF, hold/resume, CANCEL, blind transfer, then
TLS/SDES-SRTP once the local FreeSWITCH profile is pinned down.
