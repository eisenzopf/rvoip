# Changelog

## 0.2.0 — 2026-07-11

- Moved WebTransport media to one authenticated `PeerMediaRouter` and one
  datagram reader per physical peer.
- Media Streams now use the offered wire Stream ID and are created atomically
  during `connection.ready`, before `stream.opened` is sent.
- Full RTP packets are checked on both outbound and inbound UCTP media paths.
- Added explicit Session binding resolution, canonical fanout keys, exact
  Connection/Session cleanup, and peer-global subscriber Stream allocation.
- Added multi-Session routing, teardown-isolation, failed-batch rollback, and
  fresh-ID retry tests.
- Kept the old direct vector-reader entry point as a hidden alpha compatibility
  wrapper; production server paths no longer use it.

This release depends on `rvoip-uctp` 0.2.0 and inherits its breaking wire and
resource-binding changes.
