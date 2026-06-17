# capture-chime — Chime signaling protocol oracle

Captures the **working** Amazon Connect voice widget's Chime signaling
WebSocket frames so we can diff them against what the rvoip gateway's native
Rust client (`signaling/chime.rs`) emits. This turns an opaque "handshake timed
out" into a precise, line-by-line fix.

## How it works

Playwright launches Chromium with a fake mic, loads the voice widget, and uses
the Chrome DevTools Protocol (`Network.webSocketFrame*`) to record every binary
WebSocket frame (the protobuf `SdkSignalFrame`s) as base64. Output feeds the
`chime-decode` Rust binary.

## Run

```bash
npm install            # installs Playwright + Chromium

# Option 1 — bundled minimal page (needs a JWT if the widget requires a token):
CONNECT_JWT="$(curl -s --cookie 'sc_session=...' https://<host>/api/connect/token | jq -r .data)" \
  node capture.mjs
# A Chromium window opens — click the widget's call button. Recording stops after
# DURATION_MS (default 60s).

# Option 2 — drive the already-running Standard Charter site (it mints the JWT
# and embeds the widget itself); just log in in the opened window, start a call:
TARGET_URL=https://<your-standardcharter-host>/ node capture.mjs

# Headless (CI): you must supply a selector for the call button.
HEADLESS=1 CALL_BUTTON_SELECTOR='button[aria-label="Start call"]' node capture.mjs
```

Then decode + diff against our client's frames:

```bash
# Our client's JOIN/SUBSCRIBE (from the repo root):
cargo run --bin connect-probe --features aws-control -- --dump-frames > ours.b64   # (also runs live)
# ...or just the static JOIN with no AWS env:
cargo run --bin connect-probe --features aws-control -- --dump-frames 2>/dev/null | grep '^tx:' > ours.b64

cargo run --bin chime-decode -- capture.b64   # the browser (ground truth)
cargo run --bin chime-decode -- ours.b64      # our gateway
```

Compare: **signaling-URL query params** (`# url=...` lines in `capture.b64`),
**JOIN** fields (`protocol_version`/`flags`/`client_details`/`wants_compressed_sdp`),
and **SUBSCRIBE** shape (`duplex`/`audio_host`/plain-vs-compressed SDP). Patch
`crates/webrtc/rvoip-amazon-connect/src/signaling/chime.rs` to match.

## Caveats

- The widget UI is Amazon's (rendered in an iframe). Auto-clicking the call
  button (`CALL_BUTTON_SELECTOR`) may need DevTools inspection to find the right
  selector; without it, run headful and click manually.
- The `# url=...` lines reveal the real Chime signaling URL + query string — the
  most likely first divergence from our `build_signaling_url`.
- Frames from non-Chime sockets (e.g. the widget's own control channel) will fail
  to decode in `chime-decode` — harmless; set `SIGNALING_FILTER` to the Chime
  host to keep only those.
