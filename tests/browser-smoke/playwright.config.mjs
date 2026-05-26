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
  ],
});
