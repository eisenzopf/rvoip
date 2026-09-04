# rvoip-quic

> ⚠️ **Experimental surface** (unified `0.3.x` release) — API-unstable; expect breaking changes before `1.0`.

rvoip-core ConnectionAdapter implementation over raw QUIC for the UCTP application protocol

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `uctp`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## Lossless RTP ingress observation

Use `UctpQuicAdapter::new_with_rtp_ingress_observer` when packet-level logic
needs RTP sequence, SSRC, marker, CSRC, or parsed extension values before the
adapter creates payload-only `MediaFrame`s. Supply a bounded Tokio MPSC sender;
full or closed observer channels drop observations without delaying media.
`UctpQuicAdapter::new` remains unchanged for payload-only consumers.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
