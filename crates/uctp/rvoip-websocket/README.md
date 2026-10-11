# rvoip-websocket

For a minimal authenticated application-control host and client, run the
[application-profile example](examples/README.md). It demonstrates opt-in
profile negotiation, scoped commands, correlation and duplicate-ID refusal
without allocating media or using live providers.

> ⚠️ **Experimental surface** (unified `0.4.x` release) — API-unstable; expect breaking changes before `1.0`.

rvoip-core ConnectionAdapter implementation over WebSocket (signaling) for the
UCTP application protocol, with an optional co-located WebRTC PeerConnection
(media) behind the `media-webrtc` Cargo feature.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `uctp`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## Cargo features

No features are enabled by default; the default build is signaling-only.

- `media-webrtc` — enables the real `WebRtcMediaBridge` (`src/media_bridge.rs`),
  which delegates ICE/DTLS-SRTP and RTP bridging to `rvoip-webrtc`. Without it
  the bridge's substrate setup methods return an error directing callers to
  enable the feature. The answerer built by `WebRtcMediaBridge::new_answerer`
  binds `0.0.0.0:0` and passes browser mDNS `.local` candidates through, so a
  dual-stack (IPv6 + IPv4) browser offer with mDNS-anonymised host candidates
  still pairs over IPv4 instead of having its candidates dropped.
- `wss` — TLS (`wss://`) support via `tokio-rustls`/`rustls` and
  `tokio-tungstenite`'s `rustls-tls-webpki-roots`.

## Listener lifecycle

The adapter owns its inbound listener and peer-task registry. Use
`adapter.begin_drain()` to stop admission and release the listening port while
existing peers finish naturally. `adapter.is_draining()` reports this state.

`adapter.shutdown(budget).await` stops admission, cancels unfinished TLS/WS
upgrades and active inbound peers, and waits for coordinator and route cleanup.
It returns `true` only after the listener and peer tasks finish. `false` means
the caller's wait budget elapsed; cleanup continues under the listener's
ownership, and another call may wait again. Concurrent calls are idempotent.
The coordinator's signaling timeout still bounds its graceful drain before the
existing supervisor aborts a stalled driver. Dropping the adapter requests the
same cancellation but cannot await cleanup; use explicit shutdown when completion
matters. Outbound clients created by `originate` are outside this inbound-listener
lifecycle. Application callback completion remains subject to its own contract.

```rust,ignore
adapter.begin_drain(); // Keep established peers usable during a rollout.
if !adapter.shutdown(std::time::Duration::from_secs(5)).await {
    // Do not report complete shutdown: keep waiting or record incomplete cleanup.
}
```

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
