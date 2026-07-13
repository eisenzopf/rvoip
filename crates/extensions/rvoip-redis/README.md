# rvoip-redis

Redis-backed implementations of `rvoip-auth-core` provider contracts.

This crate is optional. Protocol crates depend on `rvoip-auth-core` traits and
applications can choose this crate when they need shared state for clustered
authentication:

- SIP Digest nonce and nonce-count replay through `DigestReplayStore`;
- JWT or opaque-token revocation through `TokenRevocationChecker`;
- auth failure counters and lockout decisions through `AuthRateLimiter`.

## Connections and deadlines

The provider lazily creates one shared single-node connection manager or one
shared cluster topology connection and reuses it across cloned providers and
commands. Every constructor applies finite connection, response, retry, and
per-command operation limits. The defaults are available from
`RedisAuthRuntimeConfig::default`; deployments can supply reviewed limits with
`from_config_with_runtime` or `from_cluster_config_with_runtime`.

`redis://` and certificate-verified `rediss://` URLs are supported for both
single-node and cluster seeds. All cluster seeds must use compatible TLS and
authentication settings. Bundled public Web PKI roots are the default.
Private deployments can provide a PEM trust bundle and optional PEM client
identity through `RedisAuthTlsConfig` and the additive `from_config_with_tls` or
`from_cluster_config_with_tls` constructors. Certificate, credential, and
private-key bytes are never included in provider TLS `Debug` output.

## Digest replay admission

`RedisAuthProvider` implements the bounded, client-aware Digest extensions in
`DigestReplayStore`. Nonces are atomically admitted into a tenant-namespaced
pool and reused when that pool is full. Nonce-count state is retained through
the server nonce's validity and stale window, even when `nonce_count_ttl` is
configured shorter. Default fair-share limits prevent one username or one
shared nonce from consuming the complete tenant replay budget; applications
can set explicit limits with `with_digest_replay_limits`.

The legacy `record_nonce` and `(username, nonce)` `accept_nonce_count` methods
retain their original source signature. New
clustered listeners call `admit_nonce` and `accept_client_nonce_count`. A
custom replay store that implements only the legacy methods now fails closed
on those secure paths until it implements the two additive methods; this is
intentional migration behavior and avoids an unbounded compatibility fallback.

## Atomic authentication rate limits

`RedisAuthProvider` implements the atomic `reserve_auth_attempt` and
`complete_auth_attempt` contract. Admission reserves both a peer aggregate
and a subject aggregate in one Lua operation. A failure retains one count; a
success releases only the matching reservation, and repeated completion is
idempotent. This avoids check-then-record races, double counting, username
or realm rotation around peer limits, and peer or realm rotation around
subject limits. The configured provider namespace is the trusted tenant
boundary; pre-authentication realm values never shard these aggregates.

Rate-limit state uses a fixed set of Redis keys with bounded peer, subject,
and incomplete-reservation cohorts. Configure those bounds with
`with_auth_rate_limit_limits` and inspect aggregate-safe counts with
`auth_rate_limit_cardinality`. The legacy `check_auth_attempt` and
`record_auth_result` entry points now fail closed on this provider because
they cannot provide atomic admission.

Local live tests use `RVOIP_REDIS_URL`, for example:

```sh
RVOIP_REDIS_URL=redis://127.0.0.1:6379 cargo test -p rvoip-redis
```

## Redis Cluster

Single-node construction remains unchanged through `RedisAuthProvider::new`
and `RedisAuthProvider::from_config`. Clustered deployments opt in explicitly
with `RedisAuthProvider::new_cluster` or
`RedisAuthProvider::from_cluster_config`, passing one or more seed URLs. The
provider uses the Redis asynchronous cluster client, follows slot redirects,
and keeps every key touched by each Digest Lua script in one cluster hash slot.

The disposable Docker-backed cluster test can be run with:

```sh
crates/extensions/rvoip-redis/tests/run_redis_cluster.sh
```

The fixture creates an ephemeral CA plus server and client identities, requires
password-authenticated mTLS for both a standalone Redis server and a
three-primary cluster, and rejects an unrelated CA. It also initializes the
cluster client from one seed, reassigns an empty slot between primaries after
the connection has cached its topology, and verifies that Digest Lua execution
follows the real `MOVED` redirect. The fixture restores the slot before it exits
and deletes its certificate material. Constructor or operation errors never
turn into a skipped test once a cluster environment variable has been
configured.

For an existing cluster, set `RVOIP_REDIS_CLUSTER_URLS` to a comma-separated
seed list and run the `redis_cluster_live` integration test. Each provider
namespace must remain tenant-specific. The single-node-only
`clear_namespace_for_tests` helper intentionally rejects cluster mode because
cluster-wide key enumeration is not a safe production operation.

Authenticated TLS clusters can be covered with credentialed `rediss://` URLs
whose server certificate chains to a trust root available to rustls:

```sh
RVOIP_REDIS_CLUSTER_TLS_URLS='rediss://:password@redis-one.example.test:6379,rediss://:password@redis-two.example.test:6379' \
    cargo test -p rvoip-redis --test redis_cluster_live \
    authenticated_rediss_cluster_operates_when_configured -- --nocapture
```

The TLS test skips only when `RVOIP_REDIS_CLUSTER_TLS_URLS` is absent. If it is
present, empty seeds, non-`rediss` URLs, missing credentials, constructor
errors, certificate errors, authentication errors, and operation errors fail
the test. Set `RVOIP_REDIS_TLS_CA_CERT` for a private CA and set both
`RVOIP_REDIS_TLS_CLIENT_CERT` and `RVOIP_REDIS_TLS_CLIENT_KEY` when the server
requires mTLS. `RVOIP_REDIS_SINGLE_TLS_URL` enables the corresponding
single-node live test. The provider intentionally does not disable certificate
verification.
