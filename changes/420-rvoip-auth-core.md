---
section: Breaking changes
breaking: true
---
**`rvoip_auth_core::JwtValidator::from_hmac_secret` returns
`Result<Self, BearerAuthError>`.** jsonwebtoken 12 refuses empty HMAC
keys, so an empty secret is now `BearerAuthError::Unavailable` at
construction instead of a validator that accepts tokens signed with an
empty key. `rvoip-users-core`'s `JwtIssuer::new` likewise returns
`Error::Config` for an empty HS256 `signing_key`. Migration: add `?` (or
handle the error) after `from_hmac_secret(...)`.
