# RTP Core API Security Status for 0.3.8

The previous status report claimed incomplete security prototypes were
production-ready. That report is superseded by this fail-closed status.

## Available API paths

- Explicitly provisioned direct SRTP with exactly one implemented AES-CM suite.
- SDES offer/answer exchange.
- Plain RTP only when explicitly selected with no latent security requirement
  or key material.

## Retained but unavailable

- DTLS-SRTP
- MIKEY-PSK, MIKEY-PKE, and MIKEY-DH
- ZRTP
- AES-GCM SRTP profiles
- SRTCP
- Placeholder automatic key rotation and multi-stream key derivation

These public identifiers remain for source compatibility. Checked constructors,
configuration validation, negotiation, and protocol operations return typed
unsupported errors. Infallible compatibility constructors do not make a
protocol available.

The examples named for unavailable protocols demonstrate rejection only. See
[`../../MIGRATION_0.3.5.md`](../../MIGRATION_0.3.5.md) for migration guidance.
