// Capture the Amazon Connect voice widget's Chime signaling WebSocket frames
// via Chrome DevTools Protocol (Network.webSocketFrame*), to use as a protocol
// oracle for the rvoip gateway's native Chime client.
//
// Output: one base64 frame per line, prefixed `tx:`/`rx:`, with `# url=...`
// comments per socket — directly consumable by the `chime-decode` Rust binary.
//
// Env:
//   CONNECT_JWT             JWT for the widget `authenticate` callback (if the
//                           widget requires a token). Fetch one from the running
//                           core: `curl -s --cookie "$COOKIE" .../api/connect/token`.
//   TARGET_URL              Drive an existing running site instead of the bundled
//                           widget.html (e.g. the live Standard Charter app, which
//                           already mints the JWT + embeds the widget).
//   OUT                     Output file (default: capture.b64).
//   DURATION_MS             How long to record after load (default: 60000).
//   CALL_BUTTON_SELECTOR    CSS selector to auto-click to start the call. Needed
//                           in headless mode; the widget UI is an Amazon iframe so
//                           this may require inspection (see README).
//   HEADLESS=1              Run headless (requires CALL_BUTTON_SELECTOR).
//   SIGNALING_FILTER        Only keep sockets whose URL contains this substring
//                           (default: keep all binary WS; non-Chime frames just
//                           fail to decode harmlessly).

import { chromium } from 'playwright';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { writeFileSync } from 'node:fs';

const __dirname = dirname(fileURLToPath(import.meta.url));

const JWT = process.env.CONNECT_JWT || '';
const TARGET_URL = process.env.TARGET_URL || `file://${join(__dirname, 'widget.html')}`;
const OUT = process.env.OUT || join(__dirname, 'capture.b64');
const DURATION_MS = parseInt(process.env.DURATION_MS || '60000', 10);
const CALL_BUTTON_SELECTOR = process.env.CALL_BUTTON_SELECTOR || '';
const HEADLESS = process.env.HEADLESS === '1';
const SIGNALING_FILTER = process.env.SIGNALING_FILTER || '';

const lines = [];
const seenSockets = new Map(); // requestId -> url

function record(dir, requestId, payloadBase64) {
  const url = seenSockets.get(requestId) || '';
  if (SIGNALING_FILTER && !url.includes(SIGNALING_FILTER)) return;
  lines.push(`${dir}:${payloadBase64}`);
}

const main = async () => {
  console.error(`[harness] target: ${TARGET_URL}`);
  const browser = await chromium.launch({
    headless: HEADLESS,
    args: [
      '--use-fake-ui-for-media-stream', // auto-grant mic
      '--use-fake-device-for-media-stream', // synthetic mic audio
      '--autoplay-policy=no-user-gesture-required',
    ],
  });
  const context = await browser.newContext({ permissions: ['microphone'] });
  if (JWT) {
    await context.addInitScript((tok) => {
      window.__CONNECT_JWT = tok;
    }, JWT);
  }
  const page = await context.newPage();

  // CDP: capture all WebSocket frames (works across iframes).
  const client = await context.newCDPSession(page);
  await client.send('Network.enable');
  client.on('Network.webSocketCreated', ({ requestId, url }) => {
    seenSockets.set(requestId, url);
    lines.push(`# url=${url}`);
    console.error(`[harness] ws opened: ${url}`);
  });
  // opcode 2 === binary (protobuf SdkSignalFrame). payloadData is base64 for
  // binary frames; text frames (opcode 1) are raw text — skip them.
  client.on('Network.webSocketFrameSent', ({ requestId, response }) => {
    if (response.opcode === 2) record('tx', requestId, response.payloadData);
  });
  client.on('Network.webSocketFrameReceived', ({ requestId, response }) => {
    if (response.opcode === 2) record('rx', requestId, response.payloadData);
  });

  await page.goto(TARGET_URL, { waitUntil: 'load' });

  if (CALL_BUTTON_SELECTOR) {
    console.error(`[harness] auto-clicking ${CALL_BUTTON_SELECTOR}`);
    try {
      // The widget renders in an iframe; try the page then any frame.
      await page.click(CALL_BUTTON_SELECTOR, { timeout: 10000 }).catch(async () => {
        for (const f of page.frames()) {
          if (await f.$(CALL_BUTTON_SELECTOR)) {
            await f.click(CALL_BUTTON_SELECTOR);
            return;
          }
        }
        throw new Error('selector not found in any frame');
      });
    } catch (e) {
      console.error(`[harness] auto-click failed (${e.message}); click the widget manually`);
    }
  } else if (!HEADLESS) {
    console.error('[harness] CLICK THE WIDGET CALL BUTTON NOW — recording for ' +
      `${DURATION_MS / 1000}s...`);
  } else {
    console.error('[harness] headless with no CALL_BUTTON_SELECTOR — no call will start');
  }

  await page.waitForTimeout(DURATION_MS);

  writeFileSync(OUT, lines.join('\n') + '\n');
  const frames = lines.filter((l) => !l.startsWith('#')).length;
  console.error(`[harness] wrote ${frames} binary frame(s) across ${seenSockets.size} socket(s) to ${OUT}`);
  console.error(`[harness] decode with:  cargo run --bin chime-decode -- ${OUT}`);
  await browser.close();
};

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
