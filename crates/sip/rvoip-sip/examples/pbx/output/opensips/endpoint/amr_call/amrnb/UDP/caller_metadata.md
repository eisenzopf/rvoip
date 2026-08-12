# PBX Cell Metadata

- provider: opensips
- api: endpoint
- scenario: amr_call
- transport: UDP
- role: caller
- codec: amrnb
- started_at_utc: 2026-08-12T06:39:57Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb/UDP
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb/UDP/caller.log

## Command

```sh
PBX_PROVIDER=opensips PBX_SCENARIO=amr_call PBX_TRANSPORT=UDP SIP_TRANSPORT=UDP PBX_ROLE=caller PBX_CODEC_PROFILE=amrnb AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb/UDP /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_endpoint
```

## Redacted Environment

```text
KAMAILIO_POST_REGISTER_SETTLE_SECS=1
KAMAILIO_RTP_END=23200
KAMAILIO_RTP_START=23000
OPENSIPS_PASSWORD=<redacted>
OPENSIPS_POST_REGISTER_SETTLE_SECS=1
OPENSIPS_RTP_END=23500
OPENSIPS_RTP_START=23300
OPENSIPS_UDP_ADDR=192.168.64.2:5074
PBX_CODEC_PROFILE=amrnb
PBX_REPEAT_INDEX=1
RVOIP_ADVERTISED_IP=192.168.64.1
RVOIP_LOCAL_IP=0.0.0.0
RVOIP_MEDIA_ADVERTISED_IP=192.168.64.1
```
