// Browser smoke for the v0 UCTP-over-WebTransport path.
//
// Opens a WT session, sends one `auth.hello` envelope on a bidi stream,
// awaits `auth.challenge`, and logs both. Matches the wire format the
// `uctp_agent_wt` Rust binary uses so server-side code paths are
// identical.

const URL = "https://localhost:4433/uctp";
const log = (msg, cls = "") => {
  const pre = document.getElementById("log");
  const line = document.createElement("span");
  if (cls) line.className = cls;
  line.textContent = msg + "\n";
  pre.appendChild(line);
};

// Length-prefixed (4-byte BE) framing matching
// rvoip_uctp::substrate::framing.
function frame(envelope) {
  const json = new TextEncoder().encode(JSON.stringify(envelope));
  const out = new Uint8Array(4 + json.byteLength);
  const view = new DataView(out.buffer);
  view.setUint32(0, json.byteLength, false);
  out.set(json, 4);
  return out;
}

async function readFramed(reader) {
  // Read until we have 4 bytes for the length, then read that many bytes.
  let acc = new Uint8Array(0);
  const append = (chunk) => {
    const merged = new Uint8Array(acc.byteLength + chunk.byteLength);
    merged.set(acc, 0);
    merged.set(chunk, acc.byteLength);
    acc = merged;
  };
  while (acc.byteLength < 4) {
    const { value, done } = await reader.read();
    if (done) throw new Error("stream closed before length prefix");
    append(value);
  }
  const len = new DataView(acc.buffer, acc.byteOffset, 4).getUint32(0, false);
  while (acc.byteLength < 4 + len) {
    const { value, done } = await reader.read();
    if (done) throw new Error("stream closed mid-frame");
    append(value);
  }
  const payload = acc.subarray(4, 4 + len);
  return JSON.parse(new TextDecoder().decode(payload));
}

async function runSmoke() {
  log(`opening WebTransport: ${URL}`);
  const wt = new WebTransport(URL);
  await wt.ready;
  log("WT session open", "ok");

  const stream = await wt.createBidirectionalStream();
  const writer = stream.writable.getWriter();
  const reader = stream.readable.getReader();

  const envId = "env_" + crypto.randomUUID().replaceAll("-", "");
  const hello = {
    v: 1,
    type: "auth.hello",
    id: envId,
    ts: new Date().toISOString(),
    payload: {
      device: {
        id: "dev_browser",
        kind: "web",
        platform: navigator.userAgent,
        sdk_version: "uctp-browser-smoke/0.1",
      },
      auth_methods: ["bearer"],
      capabilities: {},
    },
  };
  log("sending auth.hello id=" + envId);
  await writer.write(frame(hello));

  const reply = await readFramed(reader);
  log("received: " + JSON.stringify(reply, null, 2), "ok");

  await wt.close();
  log("WT session closed", "ok");
}

document.getElementById("run").addEventListener("click", () => {
  runSmoke().catch((e) => log("ERROR: " + e, "err"));
});
