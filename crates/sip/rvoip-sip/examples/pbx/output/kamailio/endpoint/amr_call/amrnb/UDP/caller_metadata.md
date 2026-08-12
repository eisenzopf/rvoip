# PBX Cell Metadata

- provider: kamailio
- api: endpoint
- scenario: amr_call
- transport: UDP
- role: caller
- codec: amrnb
- started_at_utc: 2026-08-12T06:38:35Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb/UDP
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb/UDP/caller.log

## Command

```sh
PBX_PROVIDER=kamailio PBX_SCENARIO=amr_call PBX_TRANSPORT=UDP SIP_TRANSPORT=UDP PBX_ROLE=caller PBX_CODEC_PROFILE=amrnb AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb/UDP /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_endpoint
```

## Redacted Environment

```text
KAMAILIO_PASSWORD=<redacted>
KAMAILIO_POST_REGISTER_SETTLE_SECS=1
KAMAILIO_RTP_END=23200
KAMAILIO_RTP_START=23000
KAMAILIO_UDP_ADDR=192.168.64.2:5072
PBX_CODEC_PROFILE=amrnb
PBX_REPEAT_INDEX=1
RVOIP_ADVERTISED_IP=192.168.64.1
RVOIP_LOCAL_IP=0.0.0.0
RVOIP_MEDIA_ADVERTISED_IP=192.168.64.1
```
