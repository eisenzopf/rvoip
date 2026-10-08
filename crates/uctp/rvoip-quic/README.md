# rvoip-quic

> ⚠️ **Experimental surface** (unified `0.3.x` release) — API-unstable; expect breaking changes before `1.0`.

rvoip-core ConnectionAdapter implementation over raw QUIC for the UCTP application protocol

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `uctp`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## Authenticated application profiles

`UctpQuicConfig::with_application_handler(Arc<dyn ApplicationHandler>)` opts
into the same `rvoip_uctp::application` contract as WebSocket. Install the handler
before constructing the adapter; every peer receives it before signaling ingress.
The coordinator advertises the profile in `auth.challenge`, validates bearer
identity, required scope and configured signatures, then dispatches commands with
an explicit `payload.profile`. Replies retain correlation and resource IDs.
Duplicate requests invoke the handler's replay hook instead of repeating effects.
The handler owns object authorization and durable idempotency; this adapter does
not supply a persistence backend. Profile-free signaling/media remains unchanged.
With a handler installed, unknown profiles return an explicit capability error;
without one, `payload.profile` is ordinary data and dispatch is unchanged. Each
handler call is bounded by `UctpCoordinatorCaps::application_handler_timeout`
(set through `UctpQuicConfig::coordinator_caps`).

```rust,ignore
let config = rvoip_quic::UctpQuicConfig::new(endpoint, accept_rx, bearer_validator)
    .with_application_handler(application);
let adapter = rvoip_quic::UctpQuicAdapter::new(config).await?;
```

## Lossless RTP ingress observation

Use `UctpQuicAdapter::new_with_rtp_ingress_observer` when packet-level logic
needs RTP sequence, SSRC, marker, CSRC, or parsed extension values before the
adapter creates payload-only `MediaFrame`s. Supply a bounded Tokio MPSC sender;
full or closed observer channels drop observations without delaying media.
`UctpQuicAdapter::new` remains unchanged for payload-only consumers.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
