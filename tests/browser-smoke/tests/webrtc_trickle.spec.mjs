// RFC 8840 (Trickle ICE over WHIP PATCH).
//
// Asserts the browser issues PATCH requests against the WHIP resource URL
// with `Content-Type: application/trickle-ice-sdpfrag`, each body parses
// as an SDP fragment carrying at least one `a=candidate:...` line, and
// the server replies 204 for every PATCH.

import { test, expect } from "@playwright/test";
import {
  pageUrl,
  startWebrtcDemo,
  stopWebrtcDemo,
  waitForRfcResults,
} from "./_webrtc_fixture.mjs";

let demo;

test.beforeAll(async () => {
  demo = await startWebrtcDemo();
});

test.afterAll(async () => {
  await stopWebrtcDemo(demo?.child);
});

test("each gathered candidate triggers a PATCH application/trickle-ice-sdpfrag → 204", async ({
  page,
}) => {
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

  const patchRequests = [];
  page.on("request", (req) => {
    if (req.method() === "PATCH" && req.url().includes("/whip/")) {
      patchRequests.push({
        url: req.url(),
        contentType: req.headers()["content-type"] || "",
        body: req.postData() || "",
      });
    }
  });

  const url = pageUrl(demo.urls.static, "whip-publish.html", {
    whip: demo.urls.whip + "/whip/trickle-demo",
    autostart: "1",
  });
  await page.goto(url);

  // Wait until at least one candidate has been gathered and ICE is up.
  const r = await waitForRfcResults(
    page,
    (s) => s && s.iceCandidatesSent >= 1 && s.status === "connected",
    { timeout: 30_000 }
  );

  expect(r.iceCandidatesSent).toBeGreaterThanOrEqual(1);
  // Every PATCH the server saw should have come back 204.
  expect(r.trickleStatusCodes.length).toBeGreaterThanOrEqual(1);
  for (const code of r.trickleStatusCodes) {
    expect(code, "trickle PATCH must return 204").toBe(204);
  }

  // Wire-level checks: at least one PATCH actually used the
  // RFC 8840 content type and carried an `a=candidate:` line.
  expect(patchRequests.length).toBeGreaterThanOrEqual(1);
  const sdpfragPatches = patchRequests.filter((p) =>
    p.contentType.toLowerCase().includes("application/trickle-ice-sdpfrag")
  );
  expect(sdpfragPatches.length).toBeGreaterThanOrEqual(1);
  for (const p of sdpfragPatches) {
    expect(p.body, "sdpfrag must include a candidate line").toMatch(/a=candidate:/);
    expect(p.body, "sdpfrag must include mid").toMatch(/a=mid:/);
  }
});
