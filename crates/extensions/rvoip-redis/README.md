# rvoip-redis

Redis-backed implementations of `rvoip-auth-core` provider contracts.

This crate is optional. Protocol crates depend on `rvoip-auth-core` traits and
applications can choose this crate when they need shared state for clustered
authentication:

- SIP Digest nonce and nonce-count replay through `DigestReplayStore`;
- JWT or opaque-token revocation through `TokenRevocationChecker`;
- auth failure counters and lockout decisions through `AuthRateLimiter`.

Local live tests use `RVOIP_REDIS_URL`, for example:

```sh
RVOIP_REDIS_URL=redis://127.0.0.1:6379 cargo test -p rvoip-redis
```
