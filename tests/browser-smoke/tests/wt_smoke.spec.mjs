// Gap plan §3.2 v1 punch list — Playwright browser smoke for the UCTP
// WebTransport path with Chromium SPKI pinning.
//
// Companion to `ws_smoke.spec.mjs` (the WebSocket path). Architecture:
//
// 1. `beforeAll` spawns `cargo run --example orchestrator_bridge` and
//    waits for both `wrote demo cert` and `wrote spki hash` log lines
//    so we know the SPKI fingerprint is on disk at /tmp/uctp_demo_cert.spki.
// 2. The test opens two browser WebTransport peers, completes authentication,
//    sends RFC 8785-canonicalized Ed25519-signed control envelopes, creates and
//    answers Connections, and sustains bidirectional RTP datagrams.
// 3. The browser sends DTMF and quality controls, closes both peers, then
//    reconnects through a two-peer capacity limit to prove cleanup.
// 4. `afterAll` kills the cargo child gracefully.
//
// Cert pinning: the playwright.config.mjs `chromium-wt` project reads
// /tmp/uctp_demo_cert.spki at config-load time. For first runs (when
// no SPKI file exists yet) the spec falls back to relaunching the
// browser context with the correct flag after orchestrator_bridge
// has written the file (Playwright `chromium.launch()` plus a fresh
// context). This makes the spec robust to a cold CI cache.
//
// CI gating: enabled when `RVOIP_WT_SMOKE=1`. The repository's
// `browser-smoke` gate sets this variable unconditionally; developers can
// use it locally to select the Chromium WebTransport project.

import { test, expect, chromium } from "@playwright/test";
import { spawn } from "node:child_process";
import { generateKeyPairSync } from "node:crypto";
import { resolve } from "node:path";
import { createServer } from "node:http";
import { readFileSync, existsSync } from "node:fs";

const REPO_ROOT = resolve(import.meta.dirname, "..", "..", "..");
const SPKI_PATH = "/tmp/uctp_demo_cert.spki";
const ACCEPTANCE_MODULE_PATH = resolve(
  REPO_ROOT,
  "tests",
  "browser-smoke",
  "fixtures",
  "uctp_wt_acceptance.mjs",
);
const browserSigningKeyPair = generateKeyPairSync("ed25519");
const browserPrivateKeyPkcs8B64 = browserSigningKeyPair.privateKey
  .export({ format: "der", type: "pkcs8" })
  .toString("base64");
const browserPublicKeySpki = browserSigningKeyPair.publicKey.export({
  format: "der",
  type: "spki",
});
// Ed25519 SubjectPublicKeyInfo ends with the 32-byte raw public key expected
// by ring's verifier.
const browserPublicKeyB64 = browserPublicKeySpki.subarray(-32).toString("base64");

const PAGE_HTML = `<!doctype html><html><body>
<pre id="log"></pre>
<script type="module">
  import { runWebTransportAcceptance } from "/uctp_wt_acceptance.mjs";
  window.__smokeResult = null;
  window.__smokeError = null;
  runWebTransportAcceptance(${JSON.stringify(browserPrivateKeyPkcs8B64)})
    .then((result) => {
      window.__smokeResult = result;
      console.log("smoke: full WebTransport acceptance complete");
    })
    .catch((error) => {
      window.__smokeError = String(error);
      console.log("smoke: error " + error);
    });
</script>
</body></html>`;

let cargoChild;
let httpServer;
let pageOrigin;
let spkiPin = "";
const observedServerEvents = {
  bridged: false,
  dtmf: false,
  quality: false,
};

test.beforeAll(async () => {
  // Serve the smoke page from a real http://127.0.0.1 origin so the
  // WT call satisfies Chromium's PNA policy.
  httpServer = createServer((req, res) => {
    if (req.url === "/uctp_wt_acceptance.mjs") {
      res.writeHead(200, { "content-type": "text/javascript; charset=utf-8" });
      res.end(readFileSync(ACCEPTANCE_MODULE_PATH));
      return;
    }
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    res.end(PAGE_HTML);
  });
  await new Promise((r) => httpServer.listen(0, "127.0.0.1", r));
  const addr = httpServer.address();
  pageOrigin = `http://127.0.0.1:${addr.port}`;

  cargoChild = spawn(
    "cargo",
    [
      "run",
      "--quiet",
      "-p",
      "rvoip-uctp",
      "--example",
      "orchestrator_bridge",
    ],
    {
      cwd: REPO_ROOT,
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        RUST_LOG: process.env.RUST_LOG || "warn",
        RVOIP_WT_BROWSER_SIGNATURES: "1",
        RVOIP_WT_BROWSER_PUBLIC_KEY_B64: browserPublicKeyB64,
      },
    }
  );

  // Wait for `wrote demo cert`, `wrote spki hash`, and `ws_bind` so
  // we know all listeners are up.
  const ready = new Promise((resolveReady, rejectReady) => {
    const deadline = setTimeout(
      () =>
        rejectReady(new Error("orchestrator_bridge did not become ready in 90s")),
      90_000
    );
    let sawCert = false;
    let sawSpki = false;
    let sawWsBind = false;
    const onLine = (chunk) => {
      const s = chunk.toString();
      process.stdout.write(`[orch] ${s}`);
      if (s.includes("ConnectionsBridged")) observedServerEvents.bridged = true;
      if (s.includes("DtmfReceived")) observedServerEvents.dtmf = true;
      if (s.includes("MediaQuality")) observedServerEvents.quality = true;
      if (s.includes("wrote demo cert")) sawCert = true;
      if (s.includes("wrote spki hash")) sawSpki = true;
      if (s.includes("ws_bind")) sawWsBind = true;
      if (sawCert && sawSpki && sawWsBind) {
        clearTimeout(deadline);
        resolveReady();
      }
    };
    cargoChild.stdout.on("data", onLine);
    cargoChild.stderr.on("data", (chunk) => process.stderr.write(`[orch] ${chunk}`));
    cargoChild.on("exit", (code) => {
      clearTimeout(deadline);
      rejectReady(new Error(`orchestrator_bridge exited prematurely (code=${code})`));
    });
  });

  await ready;
  await new Promise((r) => setTimeout(r, 250));

  if (existsSync(SPKI_PATH)) {
    spkiPin = readFileSync(SPKI_PATH, "utf8").trim();
    console.log(`[wt_smoke] using SPKI pin: ${spkiPin}`);
  }
});

test.afterAll(async () => {
  if (httpServer) {
    await new Promise((r) => httpServer.close(r));
  }
  if (!cargoChild) return;
  cargoChild.kill("SIGTERM");
  await new Promise((r) => {
    let done = false;
    cargoChild.once("exit", () => {
      done = true;
      r();
    });
    setTimeout(() => {
      if (!done) {
        try {
          cargoChild.kill("SIGKILL");
        } catch {}
        r();
      }
    }, 5_000);
  });
});

// Every phase is required: TLS/SPKI, auth, signed controls, Connection setup,
// sustained RTP in both directions, DTMF/quality events, teardown, and
// capacity-releasing reconnect.
test("browser WebTransport: signed controls, bidirectional RTP, teardown + reconnect", async () => {
  // If the SPKI wasn't available at config-load time (cold cache),
  // launch our own Chromium with the right flags now. Otherwise the
  // project config already has them and we can use Playwright's
  // default page fixture — but for robustness we always launch our
  // own context here so the SPKI value is correct.
  const args = [
    "--disable-features=BlockInsecurePrivateNetworkRequests,PrivateNetworkAccessSendPreflights,PrivateNetworkAccessRespectPreflightResults",
    "--webtransport-developer-mode",
  ];
  if (spkiPin) {
    args.push(`--ignore-certificate-errors-spki-list=${spkiPin}`);
  }

  const browser = await chromium.launch({ headless: true, args });
  const context = await browser.newContext();
  const page = await context.newPage();

  try {
    page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
    page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

    await page.goto(pageOrigin);
    await expect
      .poll(
        async () => {
          const reply = await page.evaluate(() => window.__smokeResult);
          if (reply?.complete === true) return "complete";
          const err = await page.evaluate(() => window.__smokeError);
          return err || null;
        },
        { timeout: 30_000, intervals: [100, 250, 500] }
      )
      .toBe("complete");
    const result = await page.evaluate(() => window.__smokeResult);
    expect(result).toMatchObject({
      complete: true,
      signedControls: true,
      reconnected: true,
    });
    expect(result.aliceReceived).toBeGreaterThanOrEqual(24);
    expect(result.bobReceived).toBeGreaterThanOrEqual(24);
    await expect.poll(() => observedServerEvents).toMatchObject({
      bridged: true,
      dtmf: true,
      quality: true,
    });
  } finally {
    await context.close();
    await browser.close();
  }
});
