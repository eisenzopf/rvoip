# Browser WebTransport smoke

A real browser client for the v0 demo. Exercises the same WT handshake
as `uctp_agent_wt` but from JavaScript instead of Rust — proves the
browser-reach claim end-to-end.

## Run

1. Start the `orchestrator_bridge` (writes the self-signed cert to
   `/tmp/uctp_demo_cert.der`):
   ```bash
   cargo run -p rvoip-uctp --example orchestrator_bridge
   ```

2. Compute the cert's SHA-256 hash so the browser can pin it:
   ```bash
   shasum -a 256 /tmp/uctp_demo_cert.der | awk '{print $1}'
   ```

3. Serve `index.html` from this directory:
   ```bash
   python3 -m http.server 8000
   ```

4. Launch Chrome with the cert pinned via SPKI hash. From the SHA-256
   above, compute the base64-encoded SPKI hash (see
   [Chrome's WebTransport docs](https://developer.chrome.com/docs/capabilities/web-apis/webtransport)
   for the exact algorithm), then:
   ```bash
   # Easier alternative for dev: skip cert verification entirely.
   open -a "Google Chrome" --args \
     --user-data-dir=/tmp/chrome-uctp-demo \
     --webtransport-developer-mode
   ```

5. Open `http://localhost:8000/index.html`. The page opens a WT session
   against `https://localhost:4433/uctp` and sends one `auth.hello`
   envelope, then prints the server's `auth.challenge` reply.

## Why not in CI

Browser cert ergonomics + WT spec churn make this hard to automate
robustly. The Rust `uctp_agent_wt` binary exercises the same wire path
in CI; this page exists as a manual sanity check that browsers haven't
broken something.
