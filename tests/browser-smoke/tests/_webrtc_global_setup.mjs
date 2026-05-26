// Global setup for the WebRTC RFC suite. Spawns the cargo demo ONCE
// across all six spec files instead of paying for a (re-)build per file.
//
// Writes the demo URLs and PID to `.webrtc-demo.json` in this directory;
// the spec fixture reads from it. Teardown lives in
// `_webrtc_global_teardown.mjs`.

import { spawn } from "node:child_process";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { writeFileSync } from "node:fs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..", "..", "..");
const STATE_PATH = resolve(HERE, ".webrtc-demo.json");
const READY_RE =
  /\[webrtc_browser_demo\] READY whip=(\S+) ws=(\S+) static=(\S+)/;

export default async function globalSetup() {
  const child = spawn(
    "cargo",
    [
      "run",
      "--quiet",
      "-p",
      "rvoip-webrtc",
      "--example",
      "webrtc_browser_demo",
      "--features",
      "comprehensive,signaling-whip,signaling-ws",
    ],
    {
      cwd: REPO_ROOT,
      stdio: ["ignore", "pipe", "pipe"],
      detached: true, // own process group so we can kill the whole tree
      env: { ...process.env, RUST_LOG: process.env.RUST_LOG || "warn" },
    }
  );

  const urls = await new Promise((res, rej) => {
    const deadline = setTimeout(
      () => rej(new Error("webrtc_browser_demo did not become ready in 180s")),
      180_000
    );
    let buffer = "";
    const onChunk = (chunk) => {
      const s = chunk.toString();
      process.stdout.write(`[demo] ${s}`);
      buffer += s;
      const m = buffer.match(READY_RE);
      if (m) {
        clearTimeout(deadline);
        res({ whip: m[1], ws: m[2], static: m[3] });
      }
    };
    child.stdout.on("data", onChunk);
    child.stderr.on("data", (chunk) => process.stderr.write(`[demo] ${chunk}`));
    child.on("exit", (code) => {
      clearTimeout(deadline);
      rej(new Error(`webrtc_browser_demo exited prematurely (code=${code})`));
    });
  });

  // Detach stdio so the child survives globalSetup exit and we don't
  // hold an EOF that the cargo task might block on.
  child.stdout.removeAllListeners("data");
  child.stderr.removeAllListeners("data");
  child.stdout.resume();
  child.stderr.resume();
  child.unref();

  writeFileSync(STATE_PATH, JSON.stringify({ pid: child.pid, urls }), "utf8");

  // Brief grace period — accept loops are bound by the time READY prints
  // but onboarding cargo is faster than the OS scheduling them onto their
  // own runqueues.
  await new Promise((r) => setTimeout(r, 250));
}
