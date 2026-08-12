# PBX Cell Metadata

- provider: asterisk
- api: endpoint
- scenario: b2bua_call
- transport: UDP
- role: b2bua
- codec: amrwb
- started_at_utc: 2026-08-12T04:38:28Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/b2bua_call/amrwb/UDP
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/b2bua_call/amrwb/UDP/b2bua.log

## Command

```sh
PBX_PROVIDER=asterisk PBX_SCENARIO=b2bua_call PBX_TRANSPORT=UDP SIP_TRANSPORT=UDP PBX_ROLE=b2bua PBX_CODEC_PROFILE=amrwb AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/b2bua_call/amrwb/UDP /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_endpoint
```

## Redacted Environment

```text
ASTERISK_TLS_CONTACT_MODE=reachable-contact
ASTERISK_TLS_SRTP_REQUIRED=1
AUDIO_OUTPUT_DIR=examples/asterisk/udp_hold_resume/output
IDLE_SECS=30
PBX_CODEC_PROFILE=amrwb
PBX_REPEAT_INDEX=1
PBX_REQUIRE_AMR=1
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
