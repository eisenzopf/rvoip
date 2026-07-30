# rvoip-rtp-core 0.3.5 semver exceptions

Command run from the rvoip workspace:

```text
cargo semver-checks check-release -p rvoip-rtp-core --baseline-version 0.3.4
```

Result with `cargo-semver-checks 0.49.0`: 196 checks; 193 passed, 3 failed,
0 warned, and 57 were skipped. The tool therefore classifies this coordinated
0.3.5 repair as requiring a major version. The release intentionally accepts
only the following fail-closed exceptions:

1. `enum_variant_added`
   - `api::common::error::SecurityError::UnsupportedFeature`
   - `srtp::SrtpEncryptionAlgorithm::{AeadAes128Gcm, AeadAes256Gcm}`

   A typed unsupported error is required so retained incomplete security APIs
   cannot panic, downgrade, or masquerade as generic configuration failures.
   Distinct GCM identities are required to prevent the previous conversion to
   AES-CM. Downstream exhaustive matches must add these variants.

2. `inherent_method_missing`
   - `dtls::connection::DtlsConnection::new`

   The 0.3.4 constructor returned `Self`, so it cannot report the required
   typed unsupported error. Restoring it would expose the incomplete DTLS
   state machine and bypass the fail-closed `dtls::create_connection` entry
   point.

3. `struct_missing`
   - `dtls::handshake::HandshakeState`

   The public structure exposed an incomplete handshake state machine with
   construction paths that could not enforce the supported-profile boundary.
   It is crate-private until the DTLS implementation is complete and
   interoperable.

No other semver lint failed. In particular, the public
`KeyExchangeConfig` variant shapes and the public fields of the server-managed
`DefaultClientSecurityContext` were preserved after the compatibility audit.

Some source migrations, including newly fallible security setters and helper
conversions, are not reported by this cargo-semver-checks run. They are listed
explicitly in `MIGRATION_0.3.5.md` because ignoring those errors would permit a
silent security downgrade.
