---
section: Security
breaking: false
---
**Dependency advisories.**

- `libsqlite3-sys` moves from 0.30 to 0.40, bundling SQLite 3.54, through
  sqlx 0.9.1. This fixes RUSTSEC-2026-0459 (CVE-2026-119375): opening a
  crafted database together with a `-wal` file of a different page size could
  return uninitialized memory. It affects `rvoip-users-core` and the crates
  built on it (SAML, SCIM, WebAuthn).
- DTLS-SRTP (`rvoip-rtp-core`'s `dtls-webrtc` feature) moves to `dtls` 0.17.3.
  Its P-256/P-384 0.14 stack uses the fixed `crypto-bigint` 0.7, which removes
  the DTLS path of RUSTSEC-2026-0387 (non-constant-time arithmetic).
- `cargo deny` and the release audit no longer ignore RUSTSEC-2026-0104,
  -0098 and -0099 (`rustls-webpki`) or RUSTSEC-2026-0185 (`quinn-proto`). The
  locked versions already carry the fixes.

Two advisories remain accepted. Both come only through `rvoip-webauthn`'s
`crypto-glue` 0.1, and neither is reachable as a WebAuthn relying party uses
it, because it only verifies signatures: RUSTSEC-2023-0071 (`rsa` 0.9) and
RUSTSEC-2026-0387 (`crypto-bigint` 0.5). They go once `crypto-glue` supports
`rsa` 0.10.0 final.

Applications with their own lockfile can take the fixes with
`cargo update -p sqlx -p libsqlite3-sys -p dtls`.
