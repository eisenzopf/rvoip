# moq-relay

A server that connects publishing clients to subscribing clients.
All subscriptions are deduplicated and cached, so that a single publisher can serve many subscribers.

## Cargo features

The default `runtime` feature preserves the complete relay library and binary.
Applications that only implement relay admission can omit the HTTP, relay, and
metrics runtime:

```toml
moq-relay-ietf = { path = "../moq-rs/moq-relay-ietf", default-features = false }
```

This admission-only surface exports `SessionAdmission`, `AdmissionLease`,
`AdmissionSessionId`, and their supporting request, decision, and lifecycle
types. The `moq-relay-ietf` binary requires `runtime`. Enabling
`metrics-prometheus` also enables `runtime`.

## Usage

The publisher must choose a unique name for their broadcast, sent as the WebTransport path when connecting to the server.
Connection paths are normalized and validated: trailing slashes are trimmed, dot segments and percent-encoded characters are rejected, and empty segments are not allowed. Capitalization matters.

For example: `CONNECT https://relay.quic.video/BigBuckBunny`

The MoqTransport handshake includes a `role` parameter, which must be `publisher` or `subscriber`.
The specification allows a `both` role but you'll get an error.

You can have one publisher and any number of subscribers connected to the same path.
If the publisher disconnects, then all subscribers receive an error and will not get updates, even if a new publisher reuses the path.

## Secure embedding and migration

The security-bearing rustls fields in `moq_native_ietf::tls::Config` are intentionally private. Embedders, including rvoip, should construct TLS through `tls::Args::load()` so the resulting configuration retains trustworthy evidence about server verification and inbound client authentication:

```rust,ignore
let tls = moq_native_ietf::tls::Args {
    root: vec![relay_ca],
    client_cert: Some(origin_cert),
    client_key: Some(origin_key),
    cert: vec![listener_cert],
    key: vec![listener_key],
    client_auth: moq_native_ietf::tls::ClientAuthMode::Required,
    client_ca: vec![origin_ca],
    ..Default::default()
}.load()?;
let endpoint = moq_native_ietf::quic::Endpoint::new(
    moq_native_ietf::quic::Config::new(bind, None, tls)?
)?;
```

Legacy `Server::accept`, `Client::connect`, and `SessionConnection::into_parts` retain their three-element tuples. Identity-aware applications use `accept_connection`, `connect_target`, or `into_parts_with_identity`.

Production relay embedders must now configure:

- explicit listener security and a `SessionAdmission` policy;
- fingerprint-to-scope mappings such as `SHA256=/tenant/live`, not independent fingerprint and scope lists;
- a bounded `max_active_sessions` and policy-owned `AdmissionLease` capacity;
- setup, admission, cleanup, token-revalidation, and admitted-session close deadlines.

The built-in fingerprint policy supports publisher bindings through
`new_bindings_with_limit` and subscribe-only relay/upstream bindings through
`new_relay_subscriber_bindings_with_limit`. These are deliberately separate
certificate roles: a publisher certificate cannot subscribe, a relay
subscriber certificate cannot publish, and neither role can be elevated by a
forged admission decision or scope. Production token listeners require an
external replay-, expiry-, revocation-, and capacity-aware policy. That policy
must atomically return an `AdmittedSession` from
`SessionAdmission::admit_session`, and its lease must implement periodic
revalidation plus idempotent, cancellation-safe `close`. Admission runs in a
supervised owned task: a client deadline does not cancel a policy after it may
have claimed replay state, and a late grant is immediately sent through the
same bounded finalizer. Policy I/O must be internally bounded and eventually
settle. The relay keeps global and policy capacity held until the close hook
either completes or reaches `session_close_timeout`; backend timeout and
cancellation paths must remain fail-closed. A finalization guard transfers
ownership to the reaper if a connection task is cancelled or unwinds. Legacy
policies retain composed admission and no-op close defaults, but cannot
advertise the capability flags required by a production token listener.

Every accepted session receives a fresh 128-bit server-generated
`AdmissionSessionId`. It is independent of peer-controlled QUIC connection IDs
and is available to the admission backend for replay ownership. The
listener/substrate matrix is intentionally strict: mTLS publisher listeners
accept raw QUIC; mTLS relay-subscriber listeners accept raw QUIC with the
pinned `moqt-19` ALPN and no SETUP authorization; `token-subscriber` listeners
accept WebTransport; `raw-quic-token-subscriber` listeners accept native raw
QUIC; and development listeners may accept either. The certificate roles use
verified peer fingerprints and exact scope bindings. Both token listener
variants require a non-empty SETUP authorization value, subscribe-only claims,
and the pinned `moqt-19` protocol. A substrate, protocol, authorization, role,
or scope mismatch is rejected before replay or distributed quota state is
mutated.

For an external relay tier, run the upstream listener with
`--listener-security mutual-tls-relay-subscriber`, one or more
`--admit-relay-subscriber SHA256=/tenant/broadcast` bindings, required client
authentication, and a relay-specific session cap. Configure the downstream
relay's native QUIC client with the matching client certificate/key and verified
upstream roots. Keep this endpoint separate from the origin publisher endpoint;
the relay-subscriber identity is receive-only and cannot announce or publish a
namespace. The outbound `RemoteManager` subscriber session is the intended
integration seam for rvoip.

The two production token modes deliberately use the same atomic admission and lease lifecycle. The raw-QUIC variant does not enable publishing for an anonymous TLS peer: an admitted decision containing a publish claim is rejected and its lease is finalized before coordinator or media mutation. Run browser and native subscriber listeners as separate relay processes or endpoints when their exposure or rate limits differ.

Use `Relay::run_until` with a `CancellationToken` to drain gracefully. Cancellation stops new accepts, closes active admitted sessions with `RelayShutdown`, awaits their bounded admission finalizers, and only then releases process capacity and shuts down relay dependencies. The CLI wires this path to Ctrl-C. Observe `moq_relay_admission_close_total{outcome,reason}` and the bounded admission-close error stages for finalizer health.

Per-session qlog/mlog, TLS key logging, disabled stateless retry, anonymous development admission, and `--insecure-development` are rejected or explicitly local-only in production.

Raw `SessionTarget` values retain queries for trusted routing and canonical serialization. Logs must use `SessionTarget::redacted_for_logging()` or `redact_url_for_logging()`; bearer query values and authorization parameters are never diagnostic output.

## Retained-state limits

Production relays must also configure bounded request and cache retention. `RelayConfig` exposes:

- `capacity_limits`: process, authenticated-principal, and resolved-scope totals plus independent PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE, TRACK_STATUS, and FETCH limits;
- `remote_limits`: global upstream connection/track caps and 30-second track/60-second connection idle defaults;
- `tracks_limits`: per-published-namespace cached-track and pending-request caps.
- `request_limits`: transport request queues plus per-session and process-wide retained FETCH byte budgets.

The relay CLI exposes each limit explicitly; use `moq-relay-ietf --help` for the full flag set. Limits are validated at startup. Admission is fail-fast and occurs before coordinator/media mutation. Every admitted request retains an RAII permit until its task completes, so cancellation, failure, and panic release capacity. Saturation is returned as retryable `EXCESSIVE_LOAD` with a 1001 ms retry interval rather than being reported as missing media.

The built-in API coordinator supervises one bounded refresh/cleanup task per registration. The file coordinator limits both total entries and serialized bytes and validates a write before truncating existing state. Upstream caches are keyed by resolved scope, and active calls to one scope cannot reuse another scope's authenticated session.

Prometheus metrics use only fixed `level`, `resource`, and `kind` labels. Principal, scope, namespace, track, and request IDs are intentionally omitted. Retain `Relay::diagnostics()` before moving the server into `run` to query aggregate runtime snapshots; component snapshots remain available from `RelayCapacity` and `RemoteManager`.

Direct embedders should construct `Consumer` and `Producer` with `new_admitted`, retaining the authenticated `RelayIdentity` and sharing one process-wide `RelayCapacity`. The shorter `new` constructors are compatibility helpers with isolated operator capacity and are not production admission boundaries.
