# PBX Cell Metadata

- provider: asterisk
- api: callback
- scenario: dtmf
- transport: UDP
- role: callee
- codec: default
- started_at_utc: 2026-08-12T06:24:00Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/callback/dtmf/UDP
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/callback/dtmf/UDP/callee.log

## Command

```sh
PBX_PROVIDER=asterisk PBX_SCENARIO=dtmf PBX_TRANSPORT=UDP SIP_TRANSPORT=UDP PBX_ROLE=callee PBX_CODEC_PROFILE=default AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/callback/dtmf/UDP /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_callback_builder
```

## Redacted Environment

```text
ASTERISK_TLS_CONTACT_MODE=reachable-contact
ASTERISK_TLS_SRTP_REQUIRED=1
AUDIO_OUTPUT_DIR=examples/asterisk/udp_hold_resume/output
IDLE_SECS=30
PBX_PROVIDER=asterisk
PBX_REPEAT_INDEX=1
PBX_REQUIRE_AMR=1
PBX_TRANSPORT=TLS
SIP_AUTH_USERNAME=1001
SIP_PASSWORD=<redacted>
SIP_PORT=5060
SIP_SERVER=192.168.64.2
SIP_TLS_PORT=5061
SIP_TRANSPORT=TLS
SIP_USERNAME=1001
TLS_CA_PATH=/Users/jonathan/Developer/asterisk/certs/ca.pem
TLS_CERT_PATH=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/tls/asterisk/rvoip-asterisk-listener.pem
TLS_INSECURE=1
TLS_KEY_PATH=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/tls/asterisk/rvoip-asterisk-listener-key.pem
```
