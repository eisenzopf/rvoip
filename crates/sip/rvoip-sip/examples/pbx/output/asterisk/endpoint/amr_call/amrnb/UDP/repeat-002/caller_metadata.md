# PBX Cell Metadata

- provider: asterisk
- api: endpoint
- scenario: amr_call
- transport: UDP
- role: caller
- codec: amrnb
- started_at_utc: 2026-08-12T02:03:38Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/amr_call/amrnb/UDP/repeat-002
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/amr_call/amrnb/UDP/repeat-002/caller.log

## Command

```sh
PBX_PROVIDER=asterisk PBX_SCENARIO=amr_call PBX_TRANSPORT=UDP SIP_TRANSPORT=UDP PBX_ROLE=caller PBX_CODEC_PROFILE=amrnb AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/amr_call/amrnb/UDP/repeat-002 /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_endpoint
```

## Redacted Environment

```text
ASTERISK_TLS_CONTACT_MODE=reachable-contact
ASTERISK_TLS_SRTP_REQUIRED=1
AUDIO_OUTPUT_DIR=examples/asterisk/udp_hold_resume/output
IDLE_SECS=30
PBX_REPEAT_INDEX=2
SIP_AUTH_USERNAME=1001
SIP_PASSWORD=<redacted>
SIP_PORT=5060
SIP_SERVER=192.168.64.2
SIP_TLS_PORT=5061
SIP_TRANSPORT=TLS
SIP_USERNAME=1001
TLS_CA_PATH=/Users/jonathan/Developer/asterisk/certs/ca.pem
TLS_INSECURE=1
```
