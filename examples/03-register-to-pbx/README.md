# 03 · Register to a registrar / PBX and place a call

> **Beta status: Supported.** REGISTER is interop-tested; UDP primary, TCP/TLS
> supported. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

The everyday softphone-account flow: REGISTER with credentials (SIP digest
auth), dial an extension through the PBX, then unregister. Uses the [`Endpoint`]
builder — the simplest account/profile surface.

This example talks to a **real PBX** (Asterisk, FreeSWITCH, a cloud PBX, …), so
it's driven by environment variables and is single-process. A copy-paste
Asterisk-in-docker quickstart is below.

## Configuration (environment)

| Variable | Required | Example |
|----------|----------|---------|
| `SIP_REGISTRAR` | yes | `sip:127.0.0.1:5060` or `sips:pbx.example.com:5061` |
| `SIP_USERNAME` | yes | `1001` |
| `SIP_PASSWORD` | yes | `verysecret` |
| `SIP_TARGET` | yes | `1002` or `sip:1002@pbx.example.com` |
| `SIP_ADVERTISED_ADDR` | no | `127.0.0.1:5060` (default) |
| `RUST_LOG` | no | `info` for stack tracing |

## Quick start

```sh
export SIP_REGISTRAR=sip:127.0.0.1:5060
export SIP_USERNAME=1001
export SIP_PASSWORD=verysecret
export SIP_TARGET=1002
cargo run
```

### Asterisk-in-docker in 60 seconds

```sh
# Minimal PJSIP config with two UDP accounts 1001/1002 (password "verysecret").
mkdir -p /tmp/ast && cat > /tmp/ast/pjsip.conf <<'EOF'
[transport-udp]
type=transport
protocol=udp
bind=0.0.0.0:5060

[1001](!)
type=endpoint
context=demo
disallow=all
allow=ulaw
auth=1001
aors=1001
[1001-auth](!)
type=auth
auth_type=userpass
username=1001
password=verysecret
[1001-aor](!)
type=aor
max_contacts=1

; repeat the three sections for 1002 (see Asterisk docs)
EOF
docker run --rm -p 5060:5060/udp -v /tmp/ast:/etc/asterisk/pjsip.conf.d \
  andrius/asterisk
```

Then run this example with `SIP_USERNAME=1001`, `SIP_TARGET=1002`.

## Expected output

```text
registering 1001 with the PBX…
✅ registered; calling 1002…
✅ connected call session-…
✅ unregistered, shutting down
```

## Beta scope notes

- REGISTER + digest auth + outbound INVITE through a registrar are beta-supported.
- Media is PCMU/PCMA. Configure your PBX account for `ulaw`/`alaw`.
- For the full Asterisk/FreeSWITCH interop matrix, see the in-crate `examples/pbx`.

## Troubleshooting

- **`... environment variable is required`** — set all four `SIP_*` vars.
- **Registration fails (401/403)** — username/password mismatch, or the PBX
  account isn't configured for UDP/ulaw.
- **No answer** — `SIP_TARGET` extension isn't registered or routable on the PBX.

## Next steps

- [01-quickstart-p2p](../01-quickstart-p2p/) — the no-server version.
- [08-tls-transport](../08-tls-transport/) — register over TLS (`sips:`).
- In-crate reference: `endpoint/03_registered_account`.

[`Endpoint`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.Endpoint.html
