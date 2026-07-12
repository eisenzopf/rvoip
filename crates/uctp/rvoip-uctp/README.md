# rvoip-uctp

> ⚠️ **Alpha** (`0.2.x`) — early and API-unstable; expect breaking changes before `1.0`.

UCTP (Universal Conversation Transport Protocol) — envelopes, state machine, capability negotiation, and substrate helpers shared by rvoip-quic and rvoip-webtransport

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `uctp`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## Compatibility and media framing

`rvoip_uctp::UCTP_COMPATIBILITY` is the authoritative, serializable
compatibility descriptor for diagnostics. It reports the exact crate release,
accepted envelope and datagram versions, raw-QUIC and WebTransport ALPNs, and
the media profile. Crate semver is deliberately separate from wire versions.

QUIC and WebTransport media use an eight-byte UCTP header followed by one
complete RTP packet. New adapters should use `RtpDatagram`,
`pack_rtp_datagram`, and `unpack_rtp_datagram`; these APIs generate or validate
the RTP header and cannot accidentally emit codec payload bytes alone. The raw
`MediaDatagram` helpers remain hidden compatibility APIs for the alpha adapter
line.

For packet-capture conformance runs, set `SSLKEYLOGFILE` and explicitly call
`enable_server_key_log_from_env` / `enable_client_key_log_from_env` on the
rustls configurations before constructing the quinn endpoints. The helpers do
nothing unless both conditions are met; rvoip never turns traffic-secret
logging on implicitly. Supply that key log to Wireshark or tshark when
inspecting the encrypted QUIC capture. The checked-in
`tests/fixtures/uctp_full_rtp.pcap.hex` fixture separately pins the decrypted
application-datagram bytes used by the automated conformance suite.

## Authenticated resource and Stream binding

`PeerResourceBindings` maps peer-controlled wire Session/Connection IDs to
canonical application resources through a `SessionBindingResolver`. It checks
principal expiry on cached as well as new bindings, prevents ownership changes
during refresh, permits sibling wire Connections to share one core leg only
inside the same wire Session, and rejects cross-Session aliasing. The
`BoundSubscriptionHandler` removes unreachable wire mappings before exact
registry cleanup. Publisher teardown includes the expected publisher
Connection, so it cannot delete a same-named replacement.

QUIC and WebTransport install that same binding authority directly on the
`UctpCoordinator` before ingress starts. Initial authentication and refresh
update it before peer auth state changes, and `session.invite` resolution must
succeed before a Session machine, timer, or `InboundInvite` event is created.
The substrate event pump repeats authentication and lookup idempotently only
to construct its core Route.

QUIC and WebTransport enable the coordinator's external `BindMediaStreams`
event. The adapter must return an all-or-nothing vector containing one unique,
nonzero peer-global ID per negotiated Stream. The coordinator validates the
entire batch before reserving IDs and emits no `stream.opened` on failure;
another `connection.ready` may retry. Announced IDs are not reused during the
physical peer lifetime.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
