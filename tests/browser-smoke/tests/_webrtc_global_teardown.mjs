// Global teardown for the WebRTC RFC suite. Sends SIGTERM (then SIGKILL
// 5s later if it ignored us) to the cargo child started by
// `_webrtc_global_setup.mjs`, and removes the state file.

import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readFileSync, existsSync, unlinkSync } from "node:fs";

const HERE = dirname(fileURLToPath(import.meta.url));
const STATE_PATH = resolve(HERE, ".webrtc-demo.json");

export default async function globalTeardown() {
  if (!existsSync(STATE_PATH)) return;
  let pid;
  try {
    pid = JSON.parse(readFileSync(STATE_PATH, "utf8")).pid;
  } catch {
    // best-effort cleanup
  }
  try { unlinkSync(STATE_PATH); } catch {}

  if (!pid) return;
  try {
    // Negative pid = signal the whole process group spawned with `detached: true`.
    process.kill(-pid, "SIGTERM");
  } catch {
    try { process.kill(pid, "SIGTERM"); } catch {}
  }
  // Give it a moment, then SIGKILL if it's still around.
  await new Promise((r) => setTimeout(r, 2_000));
  try { process.kill(-pid, "SIGKILL"); } catch {}
  try { process.kill(pid, "SIGKILL"); } catch {}
}
