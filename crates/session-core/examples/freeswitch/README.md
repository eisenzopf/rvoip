# FreeSWITCH Interop Examples

These examples validate `session-core` against the local FreeSWITCH/Sofia
profiles in `/Users/jonathan/Developer/freeswitch`.

They mirror the Asterisk StreamPeer examples:

| Users | Profile | Transport / media |
| --- | --- | --- |
| `1001-1004` | `rvoip_tls_srtp` | SIP TLS + mandatory SDES-SRTP |
| `2001-2004` | `rvoip_udp` | SIP UDP/TCP + plain RTP |

The TLS/SRTP profile requires SRTP but leaves the crypto suite list at the
FreeSWITCH default so SDP negotiation is exercised instead of pinned.

## Environment

The examples automatically load:

```sh
/Users/jonathan/Developer/freeswitch/freeswitch-local.env
crates/session-core/examples/freeswitch/.env
```

Important defaults:

```sh
FREESWITCH_UDP_ADDR=127.0.0.1:5062
FREESWITCH_TLS_ADDR=127.0.0.1:5063
FREESWITCH_PASSWORD=1234
RVOIP_LOCAL_IP=127.0.0.1
RVOIP_ADVERTISED_IP=127.0.0.1
RVOIP_MEDIA_ADVERTISED_IP=127.0.0.1
FREESWITCH_TEST_TIMEOUT_SECS=60
FREESWITCH_TEST_DIGITS=1234#
```

## Commands

```sh
./registration/run.sh
./udp_call/run.sh
./udp_hold_resume/run.sh
./tls_srtp_hold_resume/run.sh
./run.sh
```

Extended scenarios:

```sh
./udp_ring_remote/run.sh
./tls_srtp_ring_remote/run.sh
./udp_dtmf/run.sh
./tls_srtp_dtmf/run.sh
./udp_blind_transfer_remote/run.sh
./tls_srtp_blind_transfer_remote/run.sh
./run_remote.sh
```

Set `FREESWITCH_RUN_EXTENDED_TESTS=1` when running `./run.sh` to include the
extended suite.
