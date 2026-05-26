// RFC 9725 (WHEP — receiver side).
//
// Drives whep-subscribe.html, which:
//  1. POSTs an empty body to `/whep/{tag}`, expecting 201 + Location +
//     server-generated offer SDP with audio+video m-lines,
//  2. createAnswer() → PATCH the answer with `Content-Type: application/sdp`,
//  3. observes `pc.ontrack` and `getStats()` show growing bytesReceived
//     on inbound audio+video.

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

test("WHEP POST → 201 + Location + offer with A+V m-lines", async ({ page }) => {
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

  const url = pageUrl(demo.urls.static, "whep-subscribe.html", {
    whep: demo.urls.whip + "/whep/demo",
    autostart: "1",
  });
  await page.goto(url);

  const r = await waitForRfcResults(
    page,
    (s) => s && s.postStatusCode != null && s.offerSdp,
    { timeout: 30_000 }
  );
  expect(r.postStatusCode).toBe(201);
  expect(r.locationHeader).toBeTruthy();
  expect(r.sdpHasAudio).toBe(true);
  expect(r.sdpHasVideo).toBe(true);
});

test("WHEP PATCH(answer) → 204 and inbound media bytes grow", async ({ page }) => {
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

  const url = pageUrl(demo.urls.static, "whep-subscribe.html", {
    whep: demo.urls.whip + "/whep/recv-demo",
    autostart: "1",
  });
  await page.goto(url);

  // First gate: PATCH answer accepted, ICE up.
  const r1 = await waitForRfcResults(
    page,
    (s) => s && s.iceConnected && s.patchStatusCode != null,
    { timeout: 45_000 }
  );
  expect([200, 204]).toContain(r1.patchStatusCode);

  // Second gate: bytesReceived growing on inbound audio AND video.
  const r2 = await waitForRfcResults(
    page,
    (s) => s.remoteAudioBytes > 0 && s.remoteVideoBytes > 0,
    { timeout: 20_000 }
  );
  expect(r2.remoteAudioBytes).toBeGreaterThan(0);
  expect(r2.remoteVideoBytes).toBeGreaterThan(0);
});
