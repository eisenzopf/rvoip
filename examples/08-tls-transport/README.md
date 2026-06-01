# 08 · SIP signaling over TLS

> **Beta status: Supported.** TLS client (SNI, server validation) and TLS server
> (cert/key) are supported. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

A `sips:` call placed over the **TLS** transport. The multiplexed transport
observes the `sips:` URI scheme and routes the INVITE through the TLS listener
(port `5061`) instead of UDP. The demo runs two passes:

1. **insecure** (`TLS_INSECURE=1`) — client skips server-cert validation (dev
   escape hatch).
2. **secure** (`TLS_INSECURE=0`) — client validates the server cert against the
   CA; hostname verification runs against `127.0.0.1` (the cert's SAN).

> ⚠️ This example enables the `dev-insecure-tls` Cargo feature so
> `tls_insecure_skip_verify` is available. **Production builds must omit it** so
> rustls always validates server certs.

## Demo flow

1. `run_demo.sh` generates a one-off CA + server cert (SAN=`127.0.0.1`) with
   openssl.
2. **server** binds `sip:5060` + `sips:5061` (TLS), presenting the cert.
3. **client** places a `sips:` call — once in insecure mode, once in secure mode.

## Architecture

```
   client (:5063 TLS)                          server (:5061 TLS listener)
        │  ══ TLS handshake (server cert) ═════▶ │
        │  ── INVITE sips:server@…:5061 ───────▶ │  (over TLS)
        │  ◀── 200 OK ─────────────────────────  │
        │  ── BYE ─────────────────────────────▶ │
```

## Quick start

```sh
./run_demo.sh          # needs `openssl` on PATH; runs both passes
```

The script generates certs into a temp dir and exports `TLS_CERT_PATH` /
`TLS_KEY_PATH` / `TLS_CA_PATH` / `TLS_INSECURE` for the binaries.

## Expected output

```text
▶ TLS pass: insecure mode (skip cert validation)
  [client] ✅ Call answered over TLS — holding for 500 ms…
  [server] ✅ Call … established over TLS
=== insecure mode …: sips: call established over TLS ===

▶ TLS pass: secure mode (CA validation)
  [client] Mode: secure (CA validation via tls_extra_ca_path) — full cert chain verified
  [client] ✅ Call answered over TLS …
=== secure mode …: sips: call established over TLS ===

✅ Both passes complete — TLS works with and without verify
```

## Beta scope notes

- TLS client + server are supported; **mTLS and WSS are partial/out of scope** for
  beta.
- Real deployments use CA-signed certs and **must not** enable `dev-insecure-tls`.

## Troubleshooting

- **`openssl not found`** — install openssl; the script needs it to mint the dev
  cert chain.
- **secure pass fails hostname check** — the cert SAN must cover the connect
  target (`127.0.0.1`); the generated cert sets `IP.1 = 127.0.0.1`.

## Next steps

- [07-secure-call-srtp](../07-secure-call-srtp/) — encrypt the *media* with SRTP.
- [03-register-to-pbx](../03-register-to-pbx/) — register over `sips:`.
- In-crate reference: `regression/02_tls`.
