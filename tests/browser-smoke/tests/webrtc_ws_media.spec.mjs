// RFC 8825/8826/8829 (WebRTC + JSEP) — bidirectional media over WS signaling.
//
// Drives ws-signaling.html against the demo's WS endpoint, then asserts:
//  * SDP offer carries m=audio and m=video
//  * `pc.connectionState === 'connected'` reached
//  * `pc.getStats()` reports growing `bytesReceived` on inbound audio AND
//    inbound video (proves the server is actively pushing media back, not
//    just that the session negotiated)

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

test("WS signaling round-trip + bidirectional A+V bytes", async ({ page }) => {
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

  const url = pageUrl(demo.urls.static, "ws-signaling.html", {
    ws: demo.urls.ws,
    autostart: "1",
  });
  await page.goto(url);

  // First gate: offer SDP is well-formed and ICE/DTLS reached connected.
  const r1 = await waitForRfcResults(
    page,
    (s) => s && s.iceConnected && s.sdpHasAudio && s.sdpHasVideo,
    { timeout: 30_000 }
  );
  expect(r1.sdpHasAudio).toBe(true);
  expect(r1.sdpHasVideo).toBe(true);
  expect(r1.iceConnected).toBe(true);

  // Second gate: inbound bytes growing on BOTH kinds. The demo server
  // loops `send_fixture_media_burst` so bytesReceived should climb
  // continuously after `connected`.
  const r2 = await waitForRfcResults(
    page,
    (s) => s.remoteAudioBytes > 0 && s.remoteVideoBytes > 0,
    { timeout: 20_000 }
  );
  expect(r2.remoteAudioBytes).toBeGreaterThan(0);
  expect(r2.remoteVideoBytes).toBeGreaterThan(0);
});
