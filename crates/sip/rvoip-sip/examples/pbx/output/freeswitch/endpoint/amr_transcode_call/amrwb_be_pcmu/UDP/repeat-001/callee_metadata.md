# PBX Cell Metadata

- provider: freeswitch
- api: endpoint
- scenario: amr_transcode_call
- transport: UDP
- role: callee
- codec: amrwb_be_pcmu
- started_at_utc: 2026-08-12T01:59:39Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/UDP/repeat-001
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/UDP/repeat-001/callee.log

## Command

```sh
PBX_PROVIDER=freeswitch PBX_SCENARIO=amr_transcode_call PBX_TRANSPORT=UDP SIP_TRANSPORT=UDP PBX_ROLE=callee PBX_CODEC_PAIRING=amrwb_be_pcmu AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/UDP/repeat-001 /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_endpoint
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
RVOIP_ADVERTISED_IP=192.168.64.1
RVOIP_LOCAL_IP=192.168.64.1
RVOIP_MEDIA_ADVERTISED_IP=192.168.64.1
TLS_INSECURE=1
```
