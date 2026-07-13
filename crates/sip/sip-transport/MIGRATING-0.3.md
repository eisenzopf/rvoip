# Migrating `rvoip-sip-transport` to 0.3

Version 0.3 removes `WebSocketListener::accept` and introduces authority- and
flow-aware request, response, and lifecycle routing. This is an intentional
semver-breaking release. `rvoip-sip-transport` and the public transaction APIs
in `rvoip-sip-dialog` are both versioned at 0.3.0 for the coordinated change.

The old method returned a `WebSocketConnection` and an independently owned
`SplitStream`. Once that reader escaped, the listener could not enforce its
established-session limit, idle/authentication lifetime, close deadline, or
joined shutdown boundary. Keeping the signature while returning a runtime
error in a 0.2 patch release was source-compatible but behaviorally
incompatible, so 0.3 removes the trap and makes migration a compile-time
requirement.

## Server migration

Replace a manual accept loop:

```rust,ignore
loop {
    let (connection, mut reader) = listener.accept().await?;
    tokio::spawn(async move {
        // application-owned session
    });
}
```

with the supervised server surface:

```rust,no_run
use futures_util::StreamExt;
use rvoip_sip_transport::transport::ws::WebSocketListener;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let listener = Arc::new(
    WebSocketListener::bind("0.0.0.0:8080".parse()?, false, None, None).await?,
);

listener
    .serve_concurrent(|connection, mut reader| async move {
        while let Some(frame) = reader.next().await {
            let Ok(frame) = frame else {
                break;
            };
            let peer_closed = matches!(frame, WsMessage::Close(_));
            match connection.process_ws_message(frame) {
                Ok(Some((message, raw_bytes))) => {
                    // Dispatch `message`; retain `raw_bytes` only when an
                    // authorized byte-exact consumer requires it.
                    let _ = (message, raw_bytes);
                }
                Ok(None) => {}
                Err(_) => break,
            }
            if peer_closed {
                break;
            }
        }
    })
    .await?;
# Ok(())
# }
```

`serve_concurrent` owns every upgrade/session task, keeps handshake and
established-session admission separate, retries recoverable listener errors,
enforces lifecycle deadlines, and releases capacity when the handler, peer,
writer, or supervisor terminates. The handler should continue polling its
reader and pass frames through `process_ws_message`; a peer Close then drives a
prompt close acknowledgement and capacity release.

There is deliberately no unmanaged compatibility feature. Applications that
need a different dispatch model should build it inside the supervised handler
instead of detaching the socket reader from listener ownership.

## Response and raw-send migration

Connection-oriented receive events now expose an opaque `flow_id`. Retain that
identity with the source address and transport type, then return it in a
`TransportRoute` for structured responses, cached wire responses, and
keepalives:

```rust,ignore
let TransportEvent::MessageReceived {
    source,
    transport_type,
    flow_id,
    ..
} = event else {
    return Ok(());
};

let route = TransportRoute::new(source)
    .with_transport_type(transport_type);
let route = match flow_id {
    Some(flow_id) => route.with_flow_id(flow_id),
    None => route,
};

transport.send_message_via(Message::Response(response), route.clone()).await?;
transport.send_message_raw_via(cached_wire_bytes, route.clone()).await?;
transport.send_raw_via(route, Bytes::from_static(b"\r\n\r\n")).await?;
```

`flow_id` is an `Option<TransportFlowId>` because UDP has no connection
identity. TCP, TLS, WS, and WSS receive events provide `Some(flow_id)`. Do not
unwrap it for transport-agnostic ingress; retain `None` for UDP and require
`Some` when an operation specifically needs an established stream.

TLS/WSS requests also require the logical next-hop authority. Typed requests
derive it from the top `Route` URI (or Request-URI); resolver candidates carry
that same authority through SRV/A expansion. Address-only response selection is
retained only as a legacy multiplexer probe and must not be used by transaction
servers.

## Event shape migration

Code that constructs `TransportEvent` values directly (including mocks and
test fixtures) must initialize the complete 0.3 event shape:

- `MessageReceived` requires `message`, `source`, `destination`,
  `transport_type`, `flow_id`, `raw_bytes`, `timing`, and
  `connection_metadata`.
- `KeepAlivePongReceived` requires `source`, `destination`, and `flow_id`.
- `ConnectionClosed` requires `remote_addr`, `transport_type`, and `flow_id`.

Use `flow_id: None` only for connectionless or intentionally synthetic events.
Use `raw_bytes: None`, `timing: None`, and `connection_metadata: None` when a
synthetic event has no corresponding wire data or authenticated connection
metadata. Matchers that do not inspect the new fields can use `..`, but code
that returns a response must retain `transport_type` and `flow_id`.

## Resolver migration

Custom `Resolver` implementations must initialize the new
`ResolvedTarget::authority` field. A struct literal now requires `addr`,
`transport`, `authority`, and `expires`:

```rust,ignore
ResolvedTarget {
    addr,
    transport,
    authority: None,
    expires,
}
```

Use `None` to retain the typed request's top-Route or Request-URI authority.
Use `Some(TransportAuthority::dns(...))` when DNS expansion or a configured
logical proxy intentionally selects a specific authority. The
`ResolvedTarget::immediate` constructor defaults `authority` and `expires` to
`None`; attach a known authority with `with_authority`.

SIPS routing no longer permits a plaintext downgrade. `sips:` with `tcp` is
TLS-over-TCP, `tls` remains TLS, and `wss` remains WSS. Explicit insecure
`udp` or `ws` hints are rejected by resolver and send paths. Custom resolvers
must preserve that rule rather than returning an insecure candidate.

## Route-aware async transport APIs

The address-only methods remain available for UDP and compatibility callers,
but new transaction and stream-aware code should use `TransportRoute` and the
following async methods:

- `send_message_via` sends a structured SIP message on a supplied route.
- `prepare_message_route` selects and binds an exact stream before bytes are
  written. This closes the response-and-immediate-close race for client
  transactions.
- `send_message_on_route` sends and returns the concrete route that carried the
  request, including its selected `flow_id`.
- `resolve_flow_id_for_route` is the awaited flow lookup for authorization and
  response validation. Do not use the synchronous `flow_id_for_route` probe for
  security-sensitive decisions because it may conservatively return `None`
  during registry contention.
- `send_message_raw_via` and `send_raw_via` route cached SIP bytes and
  transport control bytes, respectively, on an exact flow.
- `forward_raw_with_via_rewrite_via` is the route-aware form of byte-preserving
  proxy forwarding.

Direct client code that must retain the selected connection uses the two-phase
sequence:

```rust,ignore
let route = transport.prepare_message_route(&message, route).await?;
let route = transport.send_message_on_route(message, route).await?;
// Retain `route` for response validation, retransmission, CANCEL, and teardown.
```

Custom connection-oriented `Transport` implementations should override these
hooks so route preparation, sending, lookup, and raw sends all use the same
opaque flow identity. The trait defaults preserve connectionless compatibility
and reject unsupported exact-flow operations.

## Route-aware transaction APIs

Resolver-selected client candidates must be passed to
`TransactionManager::create_client_transaction_on_route`; this retains the
candidate's transport and authenticated authority for the initial send,
retransmissions, CANCEL, and response validation.

Inbound server transactions must use
`TransactionManager::create_server_transaction_on_route` with the route built
from the receive event. The older
`create_server_transaction(request, remote_addr)` entry point is a deprecated
address-only compatibility surface and is valid only for UDP. It cannot safely
identify a TCP, TLS, WS, or WSS flow and must not be used for stream ingress.
