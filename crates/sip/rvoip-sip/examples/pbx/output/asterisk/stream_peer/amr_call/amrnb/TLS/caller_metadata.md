# PBX Cell Metadata

- provider: asterisk
- api: stream_peer
- scenario: amr_call
- transport: TLS
- role: caller
- codec: amrnb
- started_at_utc: 2026-08-12T06:18:37Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/stream_peer/amr_call/amrnb/TLS
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/stream_peer/amr_call/amrnb/TLS/caller.log

## Command

```sh
PBX_PROVIDER=asterisk PBX_SCENARIO=amr_call PBX_TRANSPORT=TLS SIP_TRANSPORT=TLS PBX_ROLE=caller PBX_CODEC_PROFILE=amrnb AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/stream_peer/amr_call/amrnb/TLS /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_stream_peer
```

## Redacted Environment

```text
ASTERISK_TLS_CONTACT_MODE=reachable-contact
ASTERISK_TLS_SRTP_REQUIRED=1
AUDIO_OUTPUT_DIR=examples/asterisk/udp_hold_resume/output
IDLE_SECS=30
PBX_CODEC_PROFILE=amrnb
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
