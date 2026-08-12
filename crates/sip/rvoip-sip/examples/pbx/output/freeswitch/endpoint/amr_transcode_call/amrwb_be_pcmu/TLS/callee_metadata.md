# PBX Cell Metadata

- provider: freeswitch
- api: endpoint
- scenario: amr_transcode_call
- transport: TLS
- role: callee
- codec: amrwb_be_pcmu
- started_at_utc: 2026-08-12T06:30:28Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/TLS
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/TLS/callee.log

## Command

```sh
PBX_PROVIDER=freeswitch PBX_SCENARIO=amr_transcode_call PBX_TRANSPORT=TLS SIP_TRANSPORT=TLS PBX_ROLE=callee PBX_CODEC_PAIRING=amrwb_be_pcmu AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/TLS /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_endpoint
```

## Redacted Environment

```text
FREESWITCH_ADDR=192.168.64.2:5060
FREESWITCH_IP=192.168.64.2
FREESWITCH_PASSWORD=<redacted>
FREESWITCH_RTP_END=16484
FREESWITCH_RTP_START=16384
FREESWITCH_SIP_PORT=5060
FREESWITCH_TLS_ADDR=192.168.64.2:5063
FREESWITCH_TLS_CONTACT_MODE=reachable-contact
FREESWITCH_TLS_SIP_PORT=5063
FREESWITCH_TLS_SRTP_REQUIRED=1
FREESWITCH_TLS_USERS=1001,1002,1003
FREESWITCH_UDP_ADDR=192.168.64.2:5062
FREESWITCH_UDP_SIP_PORT=5062
FREESWITCH_UDP_USERS=2001,2002,2003
FREESWITCH_XCODE_TLS_ADDR=192.168.64.2:5065
FREESWITCH_XCODE_UDP_ADDR=192.168.64.2:5064
PBX_CODEC_PAIRING=amrwb_be_pcmu
PBX_PROVIDER=freeswitch
PBX_REPEAT_INDEX=1
PBX_REQUIRE_AMR=1
PBX_TRANSPORT=TLS
RVOIP_ADVERTISED_IP=192.168.64.1
RVOIP_LOCAL_IP=192.168.64.1
RVOIP_MEDIA_ADVERTISED_IP=192.168.64.1
SIP_TRANSPORT=TLS
TLS_CERT_PATH=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/tls/freeswitch/rvoip-freeswitch-listener.pem
TLS_INSECURE=1
TLS_KEY_PATH=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/tls/freeswitch/rvoip-freeswitch-listener-key.pem
```
