// RFC 8445 / RFC 8839 — ICE candidate gathering and mDNS hostname handling.
//
// Asserts (from the WHIP page's gathered candidates):
//  * ≥1 host candidate was gathered (RFC 8445 §5)
//  * No `.local` mDNS hostname leaked into the *outgoing* candidate stream
//    we sent via trickle PATCH (RFC 8839 §3 — the crate's server-side
//    `mdns_candidate_policy = Drop` covers the inbound direction; this
//    spec is a defense-in-depth check that the *page* didn't emit any
//    .local candidates Chromium considers sensitive).

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

test("ICE gather produces ≥1 host candidate and no .local leak", async ({
  page,
}) => {
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

  const url = pageUrl(demo.urls.static, "whip-publish.html", {
    whip: demo.urls.whip + "/whip/ice-demo",
    autostart: "1",
  });
  await page.goto(url);

  const r = await waitForRfcResults(
    page,
    (s) => s && s.iceCandidatesGathered >= 1 && s.status === "connected",
    { timeout: 45_000 }
  );

  expect(r.iceCandidatesGathered).toBeGreaterThanOrEqual(1);
  expect(r.localHostCandidate, "expected at least one host-type candidate").toBe(true);
  expect(
    r.mdnsCandidatePresent,
    "no `.local` mDNS hostname should appear in the candidate stream"
  ).toBe(false);
});
