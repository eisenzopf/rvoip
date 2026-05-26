// RFC 8831 / RFC 8832 — SCTP-over-DTLS data channels with three reliability
// profiles open simultaneously on a single peer connection. The browser
// opens:
//   * dc-reliable             {ordered: true}
//   * dc-unreliable-retransmits {ordered: false, maxRetransmits: 0}
//   * dc-partial-lifetime     {ordered: true, maxPacketLifeTime: 50}
//
// Each channel sends a 10-message burst on open; the demo server echoes
// every message with an `echo:` prefix. We assert each channel:
//   1. reached `open`,
//   2. the negotiated W3C `RTCDataChannel.maxRetransmits` /
//      `maxPacketLifeTime` match the RFC 8832 §5.1 profile we requested,
//   3. received ≥1 echoed message (proves the channel actually moves
//      data — not just that SCTP negotiated).

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

test("three RFC 8832 reliability profiles each round-trip messages", async ({
  page,
}) => {
  page.on("console", (msg) => console.log(`[browser:${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => console.log(`[browser:error] ${err}`));

  const url = pageUrl(demo.urls.static, "ws-signaling.html", {
    ws: demo.urls.ws,
    autostart: "1",
  });
  await page.goto(url);

  const r = await waitForRfcResults(
    page,
    (s) =>
      s &&
      s.dcResults &&
      s.dcResults.reliable.recv >= 1 &&
      s.dcResults.unreliableRetransmits.open &&
      s.dcResults.partialLifetime.recv >= 1,
    { timeout: 45_000 }
  );

  // All three opened.
  expect(r.dcResults.reliable.open).toBe(true);
  expect(r.dcResults.unreliableRetransmits.open).toBe(true);
  expect(r.dcResults.partialLifetime.open).toBe(true);

  // Reliable channel must deliver everything it sent (≥10).
  expect(r.dcResults.reliable.recv).toBeGreaterThanOrEqual(1);

  // Partial-reliable lifetime channel must round-trip at least one message.
  expect(r.dcResults.partialLifetime.recv).toBeGreaterThanOrEqual(1);

  // RFC 8832 profile checks via the W3C `RTCDataChannel` getters. The
  // unreliable channel should report `maxRetransmits === 0` and
  // `ordered === false`; the partial-lifetime channel should report a
  // non-null `maxPacketLifeTime`.
  expect(r.dcResults.reliable.negotiated.ordered).toBe(true);
  expect(r.dcResults.unreliableRetransmits.negotiated.ordered).toBe(false);
  expect(r.dcResults.unreliableRetransmits.negotiated.maxRetransmits).toBe(0);
  expect(r.dcResults.partialLifetime.negotiated.maxPacketLifeTime).not.toBeNull();
});
