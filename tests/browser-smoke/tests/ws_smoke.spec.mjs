// Gap plan §3.2 — Playwright browser smoke for the UCTP WebSocket path.
//
// Architecture:
// 1. `beforeAll` spawns `cargo run --example orchestrator_bridge`,
//    inheriting its stdout/stderr so the cargo build log is visible.
//    Resolves once stdout shows the WS listener bound + the demo cert
//    written (`orchestrator_bridge` prints `wrote demo cert (DER)` and
//    a `ws_bind` line).
// 2. The test loads a small inline HTML page via `page.setContent()`
//    that drives the same UCTP wire path as
//    `examples/uctp_to_sip_bridge/browser/ws_smoke.html`: open a
//    `WebSocket("ws://127.0.0.1:7777")`, send one `auth.hello`
//    envelope, await `auth.challenge`. Asserts the reply's
//    `msg_type` is `"auth.challenge"`.
// 3. `afterAll` kills the cargo child (SIGTERM → wait → SIGKILL).
//
// The smoke uses the WS path because it has no TLS / cert pinning
// (the WebTransport variant needs SPKI pinning via Chrome flags,
// which is unreliable in CI). Once the WSS substrate gap (§2.3) is
// the default, swap to wss://.

import { test, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { createServer } from "node:http";

const REPO_ROOT = resolve(import.meta.dirname, "..", "..", "..");
const WS_URL = "ws://127.0.0.1:7777";

const PAGE_HTML = `<!doctype html><html><body>
<pre id="log"></pre>
<script>
  window.__smokeResult = null;
  window.__smokeError = null;
  const ws = new WebSocket(${JSON.stringify(WS_URL)});
  ws.onopen = () => {
    const envId = "env_" + crypto.randomUUID().replaceAll("-", "");
    const env = {
      v: 1,
      type: "auth.hello",
      id: envId,
      ts: new Date().toISOString(),
      payload: {
        device: { id: "dev_pw", kind: "web", platform: "playwright",
                  sdk_version: "rvoip-browser-smoke/0.1" },
        auth_methods: ["bearer"],
        capabilities: {},
      },
    };
    ws.send(JSON.stringify(env));
    console.log("smoke: sent auth.hello id=" + envId);
  };
  ws.onmessage = (ev) => {
    try {
      const parsed = JSON.parse(ev.data);
      window.__smokeResult = parsed;
      console.log("smoke: recv type=" + parsed.type);
    } catch (e) { window.__smokeError = String(e); }
  };
  ws.onerror = () => { window.__smokeError = "WebSocket error"; console.log("smoke: ws error"); };
  ws.onclose = (ev) => console.log("smoke: ws close code=" + ev.code);
</script>
</body></html>`;

let cargoChild;
let httpServer;
let pageOrigin;

test.beforeAll(async () => {
  // Serve the smoke page from a real http://127.0.0.1 origin. Without
  // this Chromium's Private Network Access blocks the about:blank →
  // 127.0.0.1 WebSocket as a cross-origin local-network request.
  httpServer = createServer((_req, res) => {
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
      env: { ...process.env, RUST_LOG: process.env.RUST_LOG || "warn" },
    }
  );

  // Drain the cargo child's stdout/stderr so it doesn't block on a
  // full pipe buffer. Also use it to detect readiness.
  const ready = new Promise((resolveReady, rejectReady) => {
    const deadline = setTimeout(
      () => rejectReady(new Error("orchestrator_bridge did not become ready in 90s")),
      90_000
    );

    let sawCert = false;
    let sawWsBind = false;

    const onLine = (chunk) => {
      const s = chunk.toString();
      process.stdout.write(`[orch] ${s}`);
      if (s.includes("wrote demo cert")) sawCert = true;
      if (s.includes("ws_bind")) sawWsBind = true;
      // The demo prints `ws_bind=` early; we still wait for the cert
      // write because that's the last init-step the demo logs before
      // it sits in its accept loop.
      if (sawCert && sawWsBind) {
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
  // Tiny grace period for accept loops to settle past the print.
  await new Promise((r) => setTimeout(r, 250));
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

test("browser WebSocket smoke: auth.hello → auth.challenge", async ({ page }) => {
  // Surface browser console + network errors so any breakage shows up
  // in test output rather than as an opaque poll-timeout.
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));
  page.on("websocket", (ws) => {
    console.log(`[browser:ws] opened url=${ws.url()}`);
    ws.on("framesent", (f) => console.log(`[browser:ws] sent: ${f.payload}`));
    ws.on("framereceived", (f) => console.log(`[browser:ws] recv: ${f.payload}`));
    ws.on("close", () => console.log(`[browser:ws] closed`));
    ws.on("socketerror", (e) => console.log(`[browser:ws] error: ${e}`));
  });

  // Navigate to the http://127.0.0.1 origin our beforeAll served. Both
  // page origin and WS target are private-network → Chromium's PNA
  // policy doesn't require a preflight.
  await page.goto(pageOrigin);

  // Poll for the result.
  await expect
    .poll(
      async () => {
        return await page.evaluate(
          () => window.__smokeResult?.type ?? window.__smokeError ?? null
        );
      },
      { timeout: 15_000, intervals: [100, 250, 500] }
    )
    .toBe("auth.challenge");

  const reply = await page.evaluate(() => window.__smokeResult);
  expect(reply).toMatchObject({ type: "auth.challenge", v: 1 });
  expect(typeof reply.id).toBe("string");
});
