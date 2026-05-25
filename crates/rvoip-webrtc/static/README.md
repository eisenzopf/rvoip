# rvoip-webrtc browser demo pages

Two static HTML pages that drive a running `WebRtcServer` from a real browser
— useful for manual interop validation and for the automated headless-Chromium
test ([`../tests/browser_interop.rs`](../tests/browser_interop.rs)).

## Manual run

1. Start the server (from the repo root):

   ```bash
   cargo run -p rvoip-webrtc --example webrtc_server --features signaling-whip,signaling-ws
   ```

   Default binds: WHIP at `127.0.0.1:8080`, WS signaling at `127.0.0.1:8081`.

2. Serve this directory with any static HTTP server (browsers require a
   secure-ish origin for `getUserMedia`; `http://localhost` is treated as
   secure):

   ```bash
   python3 -m http.server -d crates/rvoip-webrtc/static 8090
   ```

3. Open one of:

   - http://localhost:8090/whip-publish.html — WHIP audio (+ optional video)
     publisher.
   - http://localhost:8090/ws-signaling.html — WebSocket signaling with
     a data-channel ping.

   Each page reads optional `?whip=...` / `?ws=...` query params to point at
   a non-default server URL.

## Automated test

`tests/browser_interop.rs` (feature `interop-browser`) wraps both pages in a
headless-Chromium harness using [`chromiumoxide`](https://docs.rs/chromiumoxide).
Marked `#[ignore]` by default because it requires a Chromium binary on
`PATH` (or `CHROME` env var). Run with:

```bash
cargo test -p rvoip-webrtc --features interop-browser \
    --test browser_interop -- --include-ignored --nocapture
```

The harness:

1. Spins up `WebRtcServer` (WHIP + WS) on ephemeral ports.
2. Serves these static files via a tiny in-process axum static server.
3. Launches headless Chromium pointed at the page, with `--use-fake-ui-for-media-stream`
   and `--use-fake-device-for-media-stream` so `getUserMedia` returns a
   synthetic A/V source without device access.
4. Waits for the `#status` element to flip to `connected`, asserts metrics
   reflect the inbound session, then tears down.
