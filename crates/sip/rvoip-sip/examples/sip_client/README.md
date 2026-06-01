# RVoIP Softphone Example

Terminal SIP softphone built only on the `rvoip_sip::Endpoint` facade. It
uses `Endpoint` for SIP signalling, registration, call control, audio frames,
DTMF, hold/resume, transfer, SRTP, and NAT/STUN settings. The example owns only
the CLI, terminal UI, and CPAL device I/O.

## Run a Local Call

Print the two commands for a loopback call:

```sh
./crates/sip/rvoip-sip/examples/sip_client/run_loopback.sh
```

Or run them directly in two terminals:

```sh
cargo run -p rvoip-sip --example sip_client -- --preset bob-loopback
```

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --preset alice-loopback \
  --dial sip:bob@127.0.0.1:5081
```

In Bob's terminal, select `Answer` and press `Enter`. Use the contextual action
menu to hang up or quit.

The loopback presets are equivalent to:

- `alice.loopback.json`: local caller on `127.0.0.1:5080`
- `bob.loopback.json`: local callee on `127.0.0.1:5081`

## Use a PBX Account

Presets are available for the checked-in PBX templates:

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --preset asterisk-2001 \
  --dial 2002
```

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --preset freeswitch-1001 \
  --dial 1002
```

The Asterisk presets match the local Docker PBX in
`/Users/jonathan/Developer/asterisk`: extensions `2001` and `2002`, password
`password123`, SIP UDP, and plain RTP. The FreeSWITCH presets use extensions
`1001` and `1002`, password `1234`, and the local internal profile.

If your PBX addresses differ, use a JSON config or override individual fields:

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --preset asterisk-2001 \
  --registrar sip:192.168.64.2:5060 \
  --advertise 192.168.5.2:5080 \
  --media-public 192.168.5.2 \
  --dial 2002
```

`--preset` and `--config` are mutually exclusive. Explicit CLI flags override
the preset or config file.

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
cargo run -p rvoip-sip --example sip_client -- --config alice.json
```

## Keyboard

The softphone shows only actions that make sense for the current call state.

- `Up` / `Down`: select an action
- `Enter`: choose the selected action
- `Esc`: cancel a prompt, confirmation, or detail view

When a prompt is open:

- Dial and transfer prompts send on `Enter`
- DTMF sends each `0-9`, `*`, or `#` digit immediately

Legacy letter shortcuts such as `d`, `a`, `h`, `m`, `o`, `t`, `s`, and `q` still
work as hidden accelerators, but the menu is the primary interface.

## SIP Trace

Enable SIP message inspection from the Endpoint event stream:

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --preset alice-loopback \
  --dial sip:bob@127.0.0.1:5081 \
  --sip-trace
```

When trace is enabled, the contextual menu includes `SIP Trace`. It shows recent
inbound and outbound SIP messages, filters to the current call when the session
mapping is known, and opens the full raw message with `Enter`.

Auth-bearing headers are redacted by default. To write the same trace stream to
a file:

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --preset alice-loopback \
  --dial sip:bob@127.0.0.1:5081 \
  --sip-trace \
  --sip-trace-file /tmp/alice.siptrace
```

Useful trace flags:

```sh
--sip-trace
--sip-trace-file /tmp/client.siptrace
--sip-trace-no-redact
--sip-trace-capacity 512
```

## Audio Devices

List devices:

```sh
cargo run -p rvoip-sip --example sip_client -- --list-devices
```

Use an index or a device-name substring:

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --preset alice-loopback \
  --input-device "MacBook" \
  --output-device 0
```

On macOS, allow terminal microphone access for CPAL audio.

## CI / Interop Smoke Tests

Smoke mode is noninteractive and uses deterministic 8 kHz mono synthetic audio
by default. The caller sends a 440 Hz tone, the callee sends a 660 Hz tone, and
each side verifies the expected remote tone.

Local two-process loopback:

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --test callee --preset bob-loopback
```

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --test caller --preset alice-loopback \
  --dial sip:bob@127.0.0.1:5081
```

PBX two-process smoke, with Asterisk or FreeSWITCH already running locally:

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --test pbx-callee --preset asterisk-2002
```

```sh
cargo run -p rvoip-sip --example sip_client -- \
  --test pbx-caller --preset asterisk-2001 \
  --dial 2002
```

Useful smoke flags:

```sh
--test-duration 5
--test-timeout 30
--test-dtmf 5
--test-audio synthetic
```

Use `--test-audio cpal` to exercise real microphone/speaker devices instead of
deterministic tone checks.

## Network Notes

- Open the SIP TCP/UDP bind port, commonly `5060` or the selected preset port.
- Open the RTP range used by the config, defaulting to rvoip-sip's media
  range when not specified.
- When binding to `0.0.0.0`, pass `--advertise` with the LAN address peers can
  actually reach.
- STUN here is rvoip-sip's best-effort advertised media address support, not
  full ICE/WebRTC traversal.
