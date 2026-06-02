# rvoip-client

> ⚠️ **Alpha** (`0.1.x`) — early and API-unstable; expect breaking changes before `1.0`.

Client-side SDK for mobile / web / desktop / embedded apps that speak the Universal Conversation Transport Protocol (UCTP). Wraps rvoip-uctp + rvoip-sip + rvoip-webrtc behind one Client / SessionHandle / InboundEvent surface.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `client`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
