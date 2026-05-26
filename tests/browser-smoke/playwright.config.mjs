// Playwright config for the rvoip browser smoke (gap plan §3.2).
//
// The smoke specs spawn `cargo run --example orchestrator_bridge`
// directly in a beforeAll hook (rather than via Playwright's
// `webServer` field, which expects an HTTP server). One worker so
// the cargo child is shared across tests.
//
// Two projects:
//   - `chromium-ws` — runs ws_smoke.spec.mjs with PNA flags off so
//     the WebSocket from 127.0.0.1 to 127.0.0.1 isn't preflighted.
//   - `chromium-wt` — runs wt_smoke.spec.mjs (gap plan §3.2 v1 punch
//     list) with the cert's SPKI pinned via
//     `--ignore-certificate-errors-spki-list`. The spec reads the
//     SPKI from /tmp/uctp_demo_cert.spki at runtime; the launch args
//     here are templated through `process.env.RVOIP_DEMO_SPKI` so
//     the spec's beforeAll can pass the value in.
//
// The wt project is gated by `RVOIP_WT_SMOKE=1` so CI can opt in
// once the SPKI flow is stable (gap plan §3.2 mitigation).

import { defineConfig } from "@playwright/test";
import { readFileSync, existsSync } from "node:fs";

// Lazy-read the SPKI hash if it's already on disk (a previous run of
// `orchestrator_bridge` left it). If absent the wt_smoke spec's
// beforeAll computes it from a fresh run; Playwright will re-merge
// launch args at that point via `test.use`.
let spkiPin = "";
const spkiPath = "/tmp/uctp_demo_cert.spki";
if (existsSync(spkiPath)) {
  try {
    spkiPin = readFileSync(spkiPath, "utf8").trim();
  } catch {
    spkiPin = "";
  }
}

const wtProject = {
  name: "chromium-wt",
  testMatch: /wt_smoke\.spec\.mjs$/,
  use: {
    browserName: "chromium",
    launchOptions: {
      args: [
        // PNA — same disable as the ws project: WT loads from
        // http://127.0.0.1 origin → https://127.0.0.1:4433 target.
        "--disable-features=BlockInsecurePrivateNetworkRequests,PrivateNetworkAccessSendPreflights,PrivateNetworkAccessRespectPreflightResults",
        // SPKI pinning for the self-signed demo cert. If the env var
        // isn't set yet (first run), the spec re-launches Chromium
        // with the correct value after orchestrator_bridge writes it.
        ...(spkiPin ? [`--ignore-certificate-errors-spki-list=${spkiPin}`] : []),
        // WebTransport-specific dev mode helps when the SPKI flag
        // doesn't take effect in headless mode (documented fallback
        // per gap plan §3.2).
        "--webtransport-developer-mode",
      ],
    },
  },
};

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false,
  workers: 1,
  timeout: 120_000,
  reporter: process.env.CI ? "github" : "list",
  // Only the WebRTC suite needs the shared cargo demo; the UCTP smokes
  // still spawn their own bridge in beforeAll. We still set them
  // unconditionally because globalSetup is a single hook — it bails
  // immediately when the state path can be skipped (i.e. when the
  // chromium-webrtc project isn't selected, no spec reads the file).
  ...(process.env.RVOIP_WEBRTC_SMOKE === "1"
    ? {
        globalSetup: "./tests/_webrtc_global_setup.mjs",
        globalTeardown: "./tests/_webrtc_global_teardown.mjs",
      }
    : {}),
  use: {
    headless: true,
    actionTimeout: 15_000,
    navigationTimeout: 15_000,
  },
  projects: [
    {
      name: "chromium-ws",
      testMatch: /ws_smoke\.spec\.mjs$/,
      use: {
        browserName: "chromium",
        launchOptions: {
          // Chromium's Private Network Access policy blocks WS from a
          // public-origin context (e.g. about:blank) to 127.0.0.1
          // without a preflight. Disable in test-only.
          args: [
            "--disable-features=BlockInsecurePrivateNetworkRequests,PrivateNetworkAccessSendPreflights,PrivateNetworkAccessRespectPreflightResults",
          ],
        },
      },
    },
    // WT smoke is opt-in: gate behind RVOIP_WT_SMOKE so existing CI
    // doesn't break if Chromium's SPKI pinning regresses.
    ...(process.env.RVOIP_WT_SMOKE === "1" ? [wtProject] : []),
    // WebRTC RFC suite: drives rvoip-webrtc's whip-publish / ws-signaling
    // / whep-subscribe pages from a real Chromium against an in-process
    // `webrtc_browser_demo`. Opt-in via RVOIP_WEBRTC_SMOKE=1 because the
    // cargo build is heavier than the other smokes (pulls in
    // comprehensive + signaling-whip + signaling-ws).
    ...(process.env.RVOIP_WEBRTC_SMOKE === "1"
      ? [
          {
            name: "chromium-webrtc",
            testMatch: /webrtc_.*\.spec\.mjs$/,
            use: {
              browserName: "chromium",
              launchOptions: {
                args: [
                  // Merge all disabled features into one flag — Chromium
                  // only honors the LAST `--disable-features=` it sees, so
                  // splitting into multiple flags silently drops earlier
                  // ones. `WebRtcHideLocalIpsWithMdns` is the key one for
                  // the WebRTC suite: by default Chromium replaces host
                  // candidate IPs with `<uuid>.local` mDNS hostnames the
                  // server's `mdns_candidate_policy=Drop` filter discards
                  // → ICE never connects on loopback.
                  "--disable-features=BlockInsecurePrivateNetworkRequests,PrivateNetworkAccessSendPreflights,PrivateNetworkAccessRespectPreflightResults,WebRtcHideLocalIpsWithMdns",
                  "--use-fake-ui-for-media-stream",
                  "--use-fake-device-for-media-stream",
                  "--autoplay-policy=no-user-gesture-required",
                  // Required when running inside most CI containers; harmless
                  // on a developer laptop.
                  "--no-sandbox",
                ],
              },
            },
          },
        ]
      : []),
  ],
});
