---
section: Breaking changes
breaking: true
---
**parking_lot 0.12 → 0.13 in public fields.** `rvoip-media-core`'s
relay-controller `vad`/`agc`/`aec` handles, `rvoip-websocket`'s adapter
`bridge` and `rvoip-webrtc`'s media-source `last_payload` are
`parking_lot` 0.13 locks. Migration: depend on parking_lot 0.13 where you
name those types.
