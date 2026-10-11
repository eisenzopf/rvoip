# rvoip-webtransport

> ⚠️ **Experimental surface** (unified `0.4.x` release) — API-unstable; expect breaking changes before `1.0`.

rvoip-core ConnectionAdapter implementation over WebTransport (HTTP/3 + QUIC) for the UCTP application protocol

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `uctp`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## Lossless RTP ingress observation

Use `UctpWtAdapter::new_with_rtp_ingress_observer` when packet-level logic
needs RTP sequence, SSRC, marker, CSRC, or parsed extension values before the
adapter creates payload-only `MediaFrame`s. Supply a bounded Tokio MPSC sender;
full or closed observer channels drop observations without delaying media.
`UctpWtAdapter::new` remains unchanged for payload-only consumers. This
contract is identical to the raw-QUIC adapter.

## Browser origin allowlist

`UctpWtConfig::with_allowed_origins(impl IntoIterator<Item = String>)` installs
an exact-match `Origin` policy that the server enforces before a WebTransport
CONNECT is accepted (`src/server.rs`). With a policy configured, a CONNECT whose
`Origin` header is missing, duplicated, or not in the allowlist is rejected with
`403 Forbidden`. With no policy configured, requests are accepted as before, so
non-browser clients that send no `Origin` header are unaffected.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
