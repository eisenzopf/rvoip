# RTP Core Security Work Remaining

This status replaces the obsolete 2025 “all options complete” notes. Those
notes described prototypes as production-ready and must not be used as an
availability or interoperability claim.

## rvoip 0.3.7 availability

- Direct SRTP supports one explicitly selected AES-CM-128/HMAC-SHA1 profile
  with exactly 30 bytes of provisioned key and salt material.
- SDES is the implemented signaling exchange.
- DTLS-SRTP, all MIKEY modes, and ZRTP are retained only as source-compatible
  public types/configuration identifiers. Construction or validation returns a
  typed unsupported error.
- AES-GCM profiles retain distinct public identities but cannot be advertised,
  negotiated, or constructed.
- RTCP fails closed whenever SRTP is required because authenticated SRTCP is not
  yet implemented.
- Placeholder key rotation and multi-stream derivation fail closed until a
  standard, reviewed KDF is implemented.

See [`MIGRATION_0.3.5.md`](MIGRATION_0.3.5.md) for call-site changes.

## Required before enabling retained protocols

- Complete protocol implementation with published known-answer vectors.
- State-mutation and downgrade-resistance tests.
- External interoperability evidence.
- Explicit advertisement/negotiation tests.
- Security review and updated documentation that links the evidence.

No unavailable protocol may fall back to plaintext or to a different key
exchange method.
