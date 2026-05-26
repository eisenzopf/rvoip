// Per-spec fixture for the WebRTC RFC suite. Reads the URLs that
// `_webrtc_global_setup.mjs` already wrote and returns them. Specs use
// this so they don't need to know about the file location or the cargo
// process lifecycle.

import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

const HERE = dirname(fileURLToPath(import.meta.url));
const STATE_PATH = resolve(HERE, ".webrtc-demo.json");

export function loadDemoUrls() {
  const raw = readFileSync(STATE_PATH, "utf8");
  return JSON.parse(raw).urls;
}

// Compatibility shim — older versions of this file spawned cargo per
// spec via startWebrtcDemo/stopWebrtcDemo. Specs still call these names
// in beforeAll/afterAll; turn them into thin no-ops that just hand back
// the globally-set URLs.
export async function startWebrtcDemo() {
  return { child: null, urls: loadDemoUrls() };
}

export async function stopWebrtcDemo(_child) {
  // No-op: the cargo child is owned by globalTeardown.
}

export function pageUrl(staticBase, file, query = {}) {
  const u = new URL(file, staticBase + "/");
  for (const [k, v] of Object.entries(query)) u.searchParams.set(k, v);
  return u.toString();
}

export async function waitForRfcResults(page, predicate, { timeout = 20_000 } = {}) {
  const deadline = Date.now() + timeout;
  let last = null;
  while (Date.now() < deadline) {
    last = await page.evaluate(() => window.__rfcResults);
    if (predicate(last)) return last;
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(
    "waitForRfcResults timed out; last __rfcResults=" + JSON.stringify(last, null, 2)
  );
}
