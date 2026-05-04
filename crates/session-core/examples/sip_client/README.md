# Session-Core SIP Client

Interactive terminal SIP client built directly on `session-core::Endpoint`.

This example is intentionally not based on the legacy `client-core` or
`sip-client` crates. `session-core` owns registration, call control, SDP, RTP,
codecs, DTMF, hold/resume, transfer, SRTP, and NAT/STUN knobs. The example owns
only terminal UI state and CPAL microphone/speaker I/O.

## List Audio Devices

```sh
cargo run -p rvoip-session-core --example sip_client -- --list-devices
```

Use the printed index or a device-name substring with `--input-device` and
`--output-device`.

## Direct LAN Calling

On computer B:

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  listen --name bob --bind 0.0.0.0:5060 --advertise 192.168.1.20:5060
```

On computer A:

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  lan --name alice --bind 0.0.0.0:5060 --advertise 192.168.1.10:5060 \
  --target sip:bob@192.168.1.20:5060
```

You can omit `--target` and press `d` inside the TUI.

## PBX Registration

```sh
cargo run -p rvoip-session-core --example sip_client -- \
  register \
  --profile asterisk-udp \
  --name alice \
  --username 1001 \
  --password secret \
  --registrar sip:192.168.1.50:5060 \
  --bind 0.0.0.0:5060 \
  --advertise 192.168.1.10:5060
```

After registration, press `d` and dial an extension such as `1002`.

## NAT / STUN

Static advertised media address:

```sh
--media-public 203.0.113.10
```

Best-effort STUN probe:

```sh
--stun stun.l.google.com:19302
```

This is not ICE. It uses session-core's existing static advertised-address and
STUN support, which is appropriate for simple NAT labs and directly reachable
PBX/SBC deployments.

## Keys

- `d`: dial
- `a`: answer
- `r`: reject
- `h`: hang up
- `m`: mute/unmute microphone
- `o`: hold/resume
- `0-9 * #`: send DTMF
- `t`: blind transfer
- `q`: graceful quit

## Config File

Default path:

```text
~/.config/rvoip/sip-client.toml
```

Example:

```toml
default_profile = "office"

[profiles.office]
mode = "register"
profile = "asterisk-udp"
name = "alice"
username = "1001"
password = "secret"
registrar = "sip:192.168.1.50:5060"
bind = "0.0.0.0:5060"
advertise = "192.168.1.10:5060"
input-device = "MacBook"
output-device = "External"
```

Run it:

```sh
cargo run -p rvoip-session-core --example sip_client
```

CLI flags override config-file values.

## Network Notes

- Open SIP TCP/UDP port `5060` or your chosen bind port.
- Open the RTP range used by session-core, default `16000-17000/udp`.
- On macOS, allow terminal microphone access.
- When binding to `0.0.0.0`, pass `--advertise` with the LAN address peers can
  actually reach.
