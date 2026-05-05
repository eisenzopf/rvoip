# Endpoint SIP Client

Interactive and noninteractive SIP client built only on the
`session-core::Endpoint` facade. The example owns terminal UI and CPAL device
I/O; SIP signalling, registration, calls, events, audio frames, DTMF,
hold/resume, transfer, SRTP, and NAT/STUN settings all go through Endpoint
types.

## JSON Config

```json
{
  "name": "alice",
  "profile": "asterisk-udp",
  "registerOnStart": true,
  "account": {
    "username": "1001",
    "password": "secret",
    "registrar": "sip:192.168.1.50:5060"
  },
  "network": {
    "bind": "0.0.0.0:5060",
    "advertise": "192.168.1.10:5060",
    "transport": "udp",
    "stun": "stun.l.google.com:19302"
  },
  "media": {
    "publicAddress": "192.168.1.10",
    "srtp": "off"
  }
}
```

Run:

```sh
cargo run -p rvoip-session-core --example sip_client -- --config alice.json
```

CLI flags override the JSON file.

## Sample Configs

Runnable loopback configs:

- `alice.loopback.json`: local caller on `127.0.0.1:5080`
- `bob.loopback.json`: local callee on `127.0.0.1:5081`

PBX templates:

- `pbx-2001.asterisk-udp.json`
- `pbx-2002.asterisk-udp.json`
- `pbx-1001.freeswitch.json`
- `pbx-1002.freeswitch.json`

The Asterisk UDP files match the local Docker PBX in
`/Users/jonathan/Developer/asterisk`: extensions `2001` and `2002`, password
`password123`, SIP UDP, and plain RTP. Edit the registrar, account
`contactUri`, `advertise`, and `publicAddress` fields if your generated
`rvoip-local.env` uses different addresses. The FreeSWITCH files are templates
for its local Docker profile.

## Interactive Use

Direct LAN receiver:

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  --name bob --bind 0.0.0.0:5071 --advertise 192.168.1.20:5071
```

Direct LAN caller:

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  --name alice --bind 0.0.0.0:5060 --advertise 192.168.1.10:5060 \
  --dial sip:bob@192.168.1.20:5071
```

PBX registration:

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  --profile asterisk-udp \
  --name alice \
  --username 2001 \
  --password password123 \
  --registrar sip:192.168.64.2:5060 \
  --bind 0.0.0.0:5080 \
  --advertise 192.168.5.2:5080 \
  --register
```

Keys:

- `d`: dial
- `a`: answer
- `r`: reject
- `h`: hang up
- `m`: mute/unmute microphone
- `o`: hold/resume
- `0-9 * #`: send DTMF
- `t`: blind transfer
- `q`: graceful quit

## Automated Smoke

Local two-process loopback:

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  --test callee --config crates/session-core/examples/sip_client/bob.loopback.json
```

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  --test caller --config crates/session-core/examples/sip_client/alice.loopback.json \
  --dial sip:bob@127.0.0.1:5081
```

PBX two-process smoke, with Asterisk or FreeSWITCH already running locally:

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  --test pbx-callee --config crates/session-core/examples/sip_client/pbx-2002.asterisk-udp.json
```

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  --test pbx-caller --config crates/session-core/examples/sip_client/pbx-2001.asterisk-udp.json \
  --dial 2002
```

For FreeSWITCH, use the matching `pbx-1001.freeswitch.json` and
`pbx-1002.freeswitch.json` configs.

Smoke mode uses synthetic 8 kHz mono audio by default. The caller sends a
440 Hz audio tone, the callee sends a 660 Hz audio tone, and each side analyzes
inbound PCM to verify the expected remote tone. Smoke exits nonzero on timeout,
failed registration, failed call setup, missing DTMF, missing hangup, missing
inbound media, or an incorrect/missing audio tone. Use `--test-audio cpal` to
exercise real microphone/speaker devices instead of deterministic tone checks.

Useful smoke flags:

```sh
--test-duration 5
--test-timeout 30
--test-dtmf 5
--test-audio synthetic
```

## Audio Devices

```sh
cargo run -p rvoip-session-core --example sip_client -- --list-devices
```

Use the printed index or a device-name substring with `--input-device` and
`--output-device`.

## Network Notes

- Open SIP TCP/UDP port `5060` or your chosen bind port.
- Open the RTP range used by session-core, default `16000-17000/udp`.
- On macOS, allow terminal microphone access for CPAL audio.
- When binding to `0.0.0.0`, pass `--advertise` with the LAN address peers can
  actually reach.
- STUN here is session-core's best-effort advertised media address support, not
  full ICE/WebRTC traversal.
