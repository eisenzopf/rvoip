# rvoip-audio-core

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/eisenzopf/rvoip)

Audio device management, format conversion, and pipeline glue for
developers building SIP clients on top of
[`rvoip-sip`](https://crates.io/crates/sip/rvoip-sip).

The intended integration point is `rvoip-sip`'s **CallbackPeer** API
(see [`crates/sip/rvoip-sip/examples/callback_peer/`](../rvoip-sip/examples/callback_peer/)):
audio-core owns the capture/playback device pair and bridges into the
CallbackPeer's media events, leaving call-control logic to the
application.

## Status

**Alpha — held back from crates.io.** This crate is in the workspace as
a member (so it builds with the rest of rvoip) but `publish = false` is
set. The current source predates the rvoip 3 / `rvoip-sip` 0.2 API
surface and still uses the legacy `rvoip-client-core` integration hooks
(now removed). The CallbackPeer rewrite is tracked in the release plan
and will land in a follow-up alpha bump.

Until then, treat audio-core as **scaffolding**, not a stable API.

## What's here

- `device/` — `cpal`-backed audio device wrappers
- `codec/` — codec adapters (currently G.711 only)
- `format/` — sample-rate / channel conversion via `dasp`
- `rtp/` — payload framing utilities shared with `rvoip-rtp-core`
- `pipeline/` — capture → encode → ship and receive → decode → play
- `processing/` — placeholders for AEC / AGC / VAD / noise suppression

## License

Licensed under the MIT license. See the repository [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
