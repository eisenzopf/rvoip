---
section: Breaking changes
breaking: true
---
**jsonwebtoken 10 → 12** (security). jsonwebtoken types are part of these
APIs: `rvoip-vcon`'s `TrustedKey`, `JwsProtectedHeader::alg`, `sign_jws`
and `verify_jws`; `rvoip-auth-core`'s `JwtValidator::from_decoding_key` /
`with_algorithm`, `JwksJwtValidator::new_with_validation` /
`with_algorithms` and `DpopValidator::with_algorithms`; `rvoip-users-core`'s
`JwtIssuer::algorithm` / `decoding_key`; and `rvoip-webex-byova`'s
`WebexJwsConfig::algorithms`. `jsonwebtoken::Algorithm` is now
`#[non_exhaustive]`. Migration: move your own jsonwebtoken dependency to 12
and add a wildcard arm to `match`es on `Algorithm`.
