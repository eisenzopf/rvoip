---
section: Breaking changes
breaking: true
---
**web-transport 0.13 in the MoQ crates; `NegotiatedTransport` carries the
raw-QUIC URL.** web-transport 0.13 raw-QUIC sessions have no CONNECT
request, so `rvoip_moq_transport::session::NegotiatedTransport` gains
`raw_quic_url: Option<Url>` (set by `NegotiatedTransport::raw_quic`) and is
`Clone` instead of `Copy`; `rvoip-moq-native` fills it with the dialed
target or the accepted server name. `Writer::stopped` returns the widened
`Option<u32>` stream error code, and `SessionConnection` and the
`Session::connect*` / `accept*` functions take web-transport 0.13 sessions.
Migration: move to web-transport 0.13, build raw-QUIC values with
`NegotiatedTransport::raw_quic(protocol, url)` and `.clone()` a
`NegotiatedTransport` where it used to be copied.
