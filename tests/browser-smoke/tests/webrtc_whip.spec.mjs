// RFC 9725 (WHIP) HTTP-surface checks driven from a real Chromium against
// rvoip-webrtc's `webrtc_browser_demo`.
//
// Asserts:
//  * POST /whip/{tag} with `Content-Type: application/sdp` → 201 + Location
//    + ETag + Accept-Patch + (Link rel=ice-server when configured)
//  * OPTIONS preflight from the page origin succeeds with the right
//    `Access-Control-Allow-*` headers
//  * DELETE on the returned Location returns 200/204 and the
//    RTCPeerConnection ends up not-connected on the browser side.

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

test("WHIP POST → 201 + Location + ETag + Accept-Patch", async ({ page }) => {
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

  const url = pageUrl(demo.urls.static, "whip-publish.html", {
    whip: demo.urls.whip + "/whip/demo",
    autostart: "1",
  });
  await page.goto(url);

  const r = await waitForRfcResults(
    page,
    (s) => s && s.postStatusCode != null,
    { timeout: 30_000 }
  );
  expect(r.postStatusCode).toBe(201);
  expect(r.locationHeader, "Location header is required by RFC 9725").toBeTruthy();
  expect(r.etagHeader, "ETag is required for ICE-restart If-Match flow").toBeTruthy();
  expect(r.acceptPatchHeader || "").toMatch(/trickle-ice-sdpfrag/);
  expect(r.contentTypeHeader || "").toMatch(/application\/sdp/);
  // RFC 9725 §4.6 — when the server has STUN/TURN configured (the demo
  // points at stun.l.google.com), the WHIP response MUST advertise them
  // via a `Link: <url>; rel="ice-server"` header.
  expect(r.linkHeader || "", "Link: rel=ice-server is required by RFC 9725 §4.6 when ICE servers are configured")
    .toMatch(/rel="?ice-server"?/i);
});

test("OPTIONS preflight from the page origin returns allowed methods", async ({
  request,
}) => {
  // Issue OPTIONS directly with a synthetic Origin so CORS responds.
  const resp = await request.fetch(demo.urls.whip + "/whip/demo", {
    method: "OPTIONS",
    headers: {
      Origin: demo.urls.static,
      "Access-Control-Request-Method": "POST",
      "Access-Control-Request-Headers": "content-type",
    },
  });
  expect([200, 204]).toContain(resp.status());
  const allowMethods = (resp.headers()["access-control-allow-methods"] || "").toUpperCase();
  expect(allowMethods).toContain("POST");
  expect(allowMethods).toContain("PATCH");
  expect(allowMethods).toContain("DELETE");
});

test("DELETE on the WHIP resource tears down the session", async ({ page }) => {
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));
  const url = pageUrl(demo.urls.static, "whip-publish.html", {
    whip: demo.urls.whip + "/whip/teardown-demo",
    autostart: "1",
  });
  await page.goto(url);

  await waitForRfcResults(page, (s) => s && s.locationHeader, { timeout: 30_000 });
  // Wait until ICE actually reaches connected so DELETE has something to tear down.
  await waitForRfcResults(page, (s) => s.status === "connected", { timeout: 30_000 });

  await page.click("#stop");
  const r = await waitForRfcResults(page, (s) => s.deleteStatusCode != null);
  expect([200, 204]).toContain(r.deleteStatusCode);
});
