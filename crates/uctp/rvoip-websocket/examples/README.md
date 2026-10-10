# Authenticated UCTP application profile

`application_profile.rs` is a small control-only host and client using the
experimental `ApplicationHandler` interface, introduced in rvoip 0.4.0 (PR #262).

From the repository root, build once:

```sh
cargo build --locked -p rvoip-websocket --example application_profile
```

In terminal one, choose a development-only credential and start the host:

```sh
export RVOIP_EXAMPLE_TOKEN=local-example-only
./target/debug/examples/application_profile --server
```

The host binds an available loopback port and prints `ws://127.0.0.1:<port>`.
In terminal two, set the same credential and use that printed address:

```sh
export RVOIP_EXAMPLE_TOKEN=local-example-only
./target/debug/examples/application_profile --client ws://127.0.0.1:<port>
```

Expected output:

```text
Authenticated, correlated echo: "Hello through UCTP"
Duplicate ID refused with 409; no durable replay store installed
```

The client negotiates `example.echo/1` before sending its credential, checks
reply correlation, sends one `example.echo`, and demonstrates the default
duplicate-ID refusal. The coordinator owns authentication and scope checks.
The handler validates the authenticated tenant/subject, operation and bounded
payload; it refuses all Conversation/Session/Connection IDs because this
example grants no resource membership.

Press Ctrl-C to exit the host. It is an example process, not a managed runtime
drain implementation. The configured opaque-token validator has one local user
and no revocation or expiry; use the real bearer providers and WSS for remote
deployments.

## Extend it

Replace `Echo` with your own handler and authorize every referenced resource and
recipient. Persist command intent and correlated results before external
effects if you need safe retry across reconnect or restart. A peer replay cache
is not a durable effect store. Keep provider credentials and endpoint resolution
inside the host, not in application clients.

`example.echo/1` is deliberately an example profile, not a new standard UCTP
operation or the experimental Parley Conversation schema. There are no carrier,
AI-provider, SQLite, browser media, QUIC or WebTransport dependencies in this
scenario. It demonstrates the smallest authenticated application control path;
it does not qualify telephone/WebRTC audio or durable recovery.

Run its focused tests with:

```sh
cargo test --locked -p rvoip-websocket --example application_profile
```

Or build and run both processes automatically with a fresh local credential,
ephemeral port, bounded process waits and cleanup:

```sh
python3 crates/uctp/rvoip-websocket/examples/run_application_profile.py
```

The runner verifies the real WebSocket handshake, echo correlation,
duplicate-ID refusal and rejection of a different credential. It needs Python
3 and the Rust toolchain, with no npm or live providers.
