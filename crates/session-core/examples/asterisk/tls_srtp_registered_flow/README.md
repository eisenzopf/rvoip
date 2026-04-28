# TLS/SRTP Registered-Flow Test

This example verifies TLS/SRTP interop when rvoip registers over SIP TLS and
then receives the Asterisk-originated INVITE on that same registered TLS flow.
It forces:

```sh
ASTERISK_TLS_CONTACT_MODE=registered-flow-symmetric
ASTERISK_TLS_FLOW_REUSE=1
SIP_TRANSPORT=TLS
ASTERISK_TLS_SRTP_REQUIRED=1
```

The script reuses the existing TLS/SRTP hold-resume endpoint binaries for
`1001` and `1002`, runs the same audio analyzer, and adds assertions that the
registered-flow mode and keep-alive were used. It should not generate or depend
on a local rvoip SIP TLS listener certificate.

The Asterisk side must route the registered Contact back over the registration
flow. For the local Docker Asterisk profile, use:

```sh
/Users/jonathan/Developer/asterisk/scripts/run-rvoip-flow-reuse-tests.sh
```
