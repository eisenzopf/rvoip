# rvoip-client

> ⚠️ **Experimental stub (`0.1.x`)** — the public type surface is in place, but
> `Client::connect()` / `Client::call()` are **not yet wired to a live
> transport** (they return stub handles, and most `SessionHandle` methods return
> `NotImplemented`). **It cannot place a real call yet.**
>
> **Building a client today?** Use [`rvoip-sip`](https://crates.io/crates/rvoip-sip)
> directly — `StreamPeer` / `PeerControl` / `SessionHandle` (or the higher-level
> `Endpoint`) drive real SIP registration, calls, and media. This crate will
> point here until its per-protocol dispatch lands.

Client-side SDK for mobile / web / desktop / embedded apps that speak the Universal Conversation Transport Protocol (UCTP). Wraps rvoip-uctp + rvoip-sip + rvoip-webrtc behind one Client / SessionHandle / InboundEvent surface.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `client`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
