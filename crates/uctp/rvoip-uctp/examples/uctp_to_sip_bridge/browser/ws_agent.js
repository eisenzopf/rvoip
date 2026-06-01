// Browser smoke for the v0 UCTP-over-WebSocket path.
//
// One UCTP envelope per WS text frame (no length prefix — WS provides
// framing). Matches the wire format the `uctp_agent_ws` Rust binary
// uses so server-side code paths are identical.

const URL = "ws://127.0.0.1:7777";
const log = (msg, cls = "") => {
  const pre = document.getElementById("log");
  const line = document.createElement("span");
  if (cls) line.className = cls;
  line.textContent = msg + "\n";
  pre.appendChild(line);
};

function runSmoke() {
  log(`opening WebSocket: ${URL}`);
  const ws = new WebSocket(URL);

  ws.onopen = () => {
    log("WS open", "ok");
    const envId = "env_" + crypto.randomUUID().replaceAll("-", "");
    const hello = {
      v: 1,
      type: "auth.hello",
      id: envId,
      ts: new Date().toISOString(),
      payload: {
        device: {
          id: "dev_browser_ws",
          kind: "web",
          platform: navigator.userAgent,
          sdk_version: "uctp-ws-browser-smoke/0.1",
        },
        auth_methods: ["bearer"],
        capabilities: {},
      },
    };
    log("sending auth.hello id=" + envId);
    ws.send(JSON.stringify(hello));
  };

  ws.onmessage = (ev) => {
    try {
      const env = JSON.parse(ev.data);
      log("received: " + JSON.stringify(env, null, 2), "ok");
    } catch (e) {
      log("ERROR parsing reply: " + e, "err");
    }
  };

  ws.onerror = (e) => log("ERROR: " + e, "err");
  ws.onclose = (ev) => log(`WS closed (code=${ev.code})`, ev.wasClean ? "ok" : "err");
}

document.getElementById("run").addEventListener("click", runSmoke);
