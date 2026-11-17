---
section: Changed
breaking: false
---
**rand** 0.8 → 0.10, **validator** 0.20 → 0.21 and the RustCrypto wave
together, since the new RustCrypto releases share rand_core 0.10 and
hybrid-array: **sha1** / **sha2** / **md-5** 0.10 → 0.11, **hmac** 0.12 → 0.13,
**aes** 0.8 → 0.9, **ctr** 0.9 → 0.10, **aes-gcm** 0.10 → 0.11, **argon2** 0.5
→ 0.6 with **password-hash** 0.6, and **rsa** 0.9 → 0.10.

rand 0.10 renames: `thread_rng()` → `rng()`, `gen*` → `random*`,
`distributions` → `distr`, the extension trait `Rng` → `RngExt`, `RngCore`
→ `Rng`, and `OsRng` → `SysRng` (fallible; key material is drawn with
`UnwrapErr(SysRng)`, which panics on an OS RNG failure exactly as
`OsRng` did).

Digests and cipher blocks are `hybrid_array::Array`: `GenericArray`
conversions become `TryFrom` (each length is checked first, so none of
them can fail), `BlockEncrypt`/`BlockDecrypt` become
`BlockCipherEncrypt`/`BlockCipherDecrypt`, `Hmac::new_from_slice` comes
from `KeyInit`, and SRTP's SHA-1 fast path calls
`sha1::block_api::compress`. A digest no longer formats with `{:x}`, so
hex strings come from `hex::encode` (`rvoip-sip` adds `hex`).

argon2 0.6 draws the 16-byte salt itself (`hash_password(password)`); the
PHC strings are unchanged and a new test verifies a hash stored by argon2
0.5.

rsa 0.10: `PrivateKey` replaces `RsaPrivateKey`, PEM support is in the
default `encoding` feature, and the JWK `n`/`e` come from
`to_be_bytes_trimmed_vartime()` so they stay minimal octets (a test pins
`e = AQAB` and a 256-byte `n`).
