# rvoip-redis

Redis-backed implementations of `rvoip-auth-core` provider contracts.

This crate is optional. Protocol crates depend on `rvoip-auth-core` traits and
applications can choose this crate when they need shared state for clustered
authentication:

- SIP Digest nonce and nonce-count replay through `DigestReplayStore`;
- JWT or opaque-token revocation through `TokenRevocationChecker`;
- auth failure counters and lockout decisions through `AuthRateLimiter`.

## Digest replay admission

`RedisAuthProvider` implements the bounded, client-aware Digest extensions in
`DigestReplayStore`. Nonces are atomically admitted into a tenant-namespaced
pool and reused when that pool is full. Nonce-count state is retained through
the server nonce's validity and stale window, even when `nonce_count_ttl` is
configured shorter. Default fair-share limits prevent one username or one
shared nonce from consuming the complete tenant replay budget; applications
can set explicit limits with `with_digest_replay_limits`.

The legacy `record_nonce` and `(username, nonce)` `accept_nonce_count` methods
remain for source compatibility. New clustered listeners call `admit_nonce`
and `accept_client_nonce_count`. A custom replay store that implements only the
legacy methods now fails closed on those secure paths until it implements the
two additive methods; this is intentional migration behavior and avoids an
unbounded compatibility fallback.

Local live tests use `RVOIP_REDIS_URL`, for example:

```sh
RVOIP_REDIS_URL=redis://127.0.0.1:6379 cargo test -p rvoip-redis
```
