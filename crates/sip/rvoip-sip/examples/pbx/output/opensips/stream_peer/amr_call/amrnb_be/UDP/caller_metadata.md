# PBX Cell Metadata

- provider: opensips
- api: stream_peer
- scenario: amr_call
- transport: UDP
- role: caller
- codec: amrnb_be
- started_at_utc: 2026-08-12T06:40:31Z
- output_dir: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb_be/UDP
- log: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb_be/UDP/caller.log

## Command

```sh
PBX_PROVIDER=opensips PBX_SCENARIO=amr_call PBX_TRANSPORT=UDP SIP_TRANSPORT=UDP PBX_ROLE=caller PBX_CODEC_PROFILE=amrnb_be AUDIO_OUTPUT_DIR=/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb_be/UDP /Users/jonathan/Developer/rvoip/target/debug/examples/pbx_stream_peer
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
PBX_CODEC_PROFILE=amrnb_be
PBX_REPEAT_INDEX=1
RVOIP_ADVERTISED_IP=192.168.64.1
RVOIP_LOCAL_IP=0.0.0.0
RVOIP_MEDIA_ADVERTISED_IP=192.168.64.1
```
