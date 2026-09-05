const WT_URL = "https://127.0.0.1:4433/uctp";
const KEY_ID = "browser-smoke-key";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function randomId(prefix) {
  return `${prefix}_${crypto.randomUUID().replaceAll("-", "")}`;
}

function fromBase64(value) {
  return Uint8Array.from(atob(value), (character) => character.charCodeAt(0));
}

function toBase64Url(value) {
  let binary = "";
  for (const byte of new Uint8Array(value)) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

// RFC 8785 uses ECMAScript primitive serialization and UTF-16 key ordering.
// The acceptance payloads contain only JSON-safe finite numbers.
function canonicalize(value) {
  if (value === null || typeof value === "boolean" || typeof value === "number" || typeof value === "string") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalize).join(",")}]`;
  const keys = Object.keys(value).sort();
  return `{${keys.map((key) => `${JSON.stringify(key)}:${canonicalize(value[key])}`).join(",")}}`;
}

async function readAll(reader) {
  const chunks = [];
  let length = 0;
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    chunks.push(value);
    length += value.byteLength;
  }
  const joined = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    joined.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return joined;
}

function decodeEnvelopeFrame(frame) {
  if (frame.byteLength < 4) throw new Error("truncated envelope length prefix");
  const length = new DataView(frame.buffer, frame.byteOffset, 4).getUint32(0, false);
  if (length > 1024 * 1024) throw new Error("oversized envelope frame");
  if (frame.byteLength !== 4 + length) throw new Error("invalid envelope frame length");
  return JSON.parse(decoder.decode(frame.subarray(4)));
}

function encodeEnvelopeFrame(envelope) {
  const body = encoder.encode(JSON.stringify(envelope));
  const frame = new Uint8Array(4 + body.length);
  new DataView(frame.buffer).setUint32(0, body.length, false);
  frame.set(body, 4);
  return frame;
}

function makeRtpDatagram(streamLocalId, datagramSequence, marker) {
  const payload = encoder.encode(marker);
  const frame = new Uint8Array(8 + 12 + payload.length);
  const view = new DataView(frame.buffer);
  frame[0] = 1; // UCTP datagram version.
  frame[1] = 0;
  view.setUint16(2, streamLocalId, false);
  view.setUint32(4, datagramSequence, false);
  frame[8] = 0x80; // RTP v2, no padding/extensions/CSRCs.
  frame[9] = 111; // Opus dynamic payload type.
  view.setUint16(10, datagramSequence & 0xffff, false);
  view.setUint32(12, datagramSequence * 960, false);
  view.setUint32(16, 0x42524f57, false); // "BROW"
  frame.set(payload, 20);
  return frame;
}

function rtpPayload(datagram) {
  if (datagram.byteLength < 20 || datagram[0] !== 1 || (datagram[8] >> 6) !== 2) return null;
  const csrcCount = datagram[8] & 0x0f;
  const headerLength = 20 + csrcCount * 4;
  if (datagram.byteLength < headerLength) return null;
  return decoder.decode(datagram.subarray(headerLength));
}

class UctpBrowserPeer {
  constructor(label, signingKey) {
    this.label = label;
    this.signingKey = signingKey;
    this.inbox = [];
    this.waiters = [];
    this.mediaPayloads = [];
  }

  async connect() {
    this.transport = new WebTransport(WT_URL);
    await this.transport.ready;
    this.controlPump = this.pumpControls();
    this.mediaPump = this.pumpMedia();
  }

  async pumpControls() {
    const incoming = this.transport.incomingUnidirectionalStreams.getReader();
    try {
      while (true) {
        const { value: stream, done } = await incoming.read();
        if (done) return;
        const envelope = decodeEnvelopeFrame(await readAll(stream.getReader()));
        const waiterIndex = this.waiters.findIndex((waiter) => waiter.predicate(envelope));
        if (waiterIndex >= 0) {
          const [waiter] = this.waiters.splice(waiterIndex, 1);
          clearTimeout(waiter.timer);
          waiter.resolve(envelope);
        } else {
          this.inbox.push(envelope);
        }
      }
    } catch (error) {
      if (!this.closed) this.failWaiters(error);
    }
  }

  async pumpMedia() {
    const reader = this.transport.datagrams.readable.getReader();
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) return;
        const payload = rtpPayload(value);
        if (payload !== null) this.mediaPayloads.push(payload);
      }
    } catch (error) {
      if (!this.closed) this.failWaiters(error);
    }
  }

  failWaiters(error) {
    for (const waiter of this.waiters.splice(0)) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
  }

  waitFor(predicate, description, timeoutMs = 10_000) {
    const existingIndex = this.inbox.findIndex(predicate);
    if (existingIndex >= 0) return Promise.resolve(this.inbox.splice(existingIndex, 1)[0]);
    return new Promise((resolve, reject) => {
      const waiter = { predicate, resolve, reject, timer: null };
      waiter.timer = setTimeout(() => {
        const index = this.waiters.indexOf(waiter);
        if (index >= 0) this.waiters.splice(index, 1);
        reject(new Error(`${this.label}: timed out waiting for ${description}`));
      }, timeoutMs);
      this.waiters.push(waiter);
    });
  }

  waitType(type, fields = {}) {
    return this.waitFor(
      (envelope) =>
        envelope.type === type &&
        Object.entries(fields).every(([key, value]) => envelope[key] === value),
      `${type} ${JSON.stringify(fields)}`,
    );
  }

  async sign(envelope) {
    const canonical = encoder.encode(canonicalize(envelope));
    const signature = await crypto.subtle.sign("Ed25519", this.signingKey, canonical);
    envelope.signature = { keyid: KEY_ID, alg: "EdDSA", sig: toBase64Url(signature) };
  }

  async send(type, payload, fields = {}, signed = false) {
    const envelope = {
      v: 1,
      type,
      id: randomId("env"),
      ts: new Date().toISOString(),
      ...fields,
      payload,
    };
    if (signed) await this.sign(envelope);
    const stream = await this.transport.createUnidirectionalStream();
    const writer = stream.getWriter();
    await writer.write(encodeEnvelopeFrame(envelope));
    await writer.close();
    return envelope;
  }

  async authenticate() {
    const hello = await this.send("auth.hello", {
      device: {
        id: `dev_${this.label}`,
        kind: "web",
        platform: "playwright",
        sdk_version: "rvoip-browser-smoke/0.2",
      },
      auth_methods: ["bearer"],
      capabilities: {},
    });
    await this.waitType("auth.challenge", { in_reply_to: hello.id });
    const response = await this.send("auth.response", {
      method: "bearer",
      credential: `test-token-${this.label}`,
    });
    const session = await this.waitType("auth.session", { in_reply_to: response.id });
    this.participantId = session.payload.participant_id;
  }

  async establishMedia() {
    this.sid = randomId("sess");
    this.cid = randomId("conv");
    this.connid = randomId("conn");
    this.streamId = randomId("strm");
    await this.send(
      "session.invite",
      {
        from: this.participantId,
        to: ["part_browser_smoke_destination"],
        medium: "voice",
        intent: "synchronous-engagement",
        capabilities_offer: {},
      },
      { cid: this.cid, sid: this.sid },
      true,
    );
    await this.waitType("session.accept", { sid: this.sid });
    await this.send(
      "connection.offer",
      {
        by_participant: this.participantId,
        substrate: "webtransport",
        capabilities: {},
        streams_offered: [
          {
            id: this.streamId,
            kind: "audio",
            direction: "sendrecv",
            codec_preferences: ["opus"],
          },
        ],
        substrate_setup: null,
      },
      { cid: this.cid, sid: this.sid, connid: this.connid },
      true,
    );
    await this.send(
      "connection.answer",
      {
        by_participant: this.participantId,
        substrate: "webtransport",
        capabilities: {},
        streams_answered: [
          {
            id: this.streamId,
            kind: "audio",
            direction: "sendrecv",
            codec: { name: "opus", params: { sample_rate: 48000, channels: 2 } },
          },
        ],
        substrate_setup: null,
      },
      { cid: this.cid, sid: this.sid, connid: this.connid },
      true,
    );
    await this.send(
      "connection.ready",
      {},
      { cid: this.cid, sid: this.sid, connid: this.connid },
      true,
    );
    const opened = await this.waitType("stream.opened", {
      sid: this.sid,
      connid: this.connid,
    });
    this.streamLocalId = opened.payload.stream.stream_local_id;
    if (!this.streamLocalId) throw new Error(`${this.label}: invalid stream_local_id`);
  }

  async sendMedia(prefix, count) {
    const writer = this.transport.datagrams.writable.getWriter();
    try {
      for (let index = 1; index <= count; index += 1) {
        await writer.write(makeRtpDatagram(this.streamLocalId, index, `${prefix}-${index}`));
        await new Promise((resolve) => setTimeout(resolve, 20));
      }
    } finally {
      writer.releaseLock();
    }
  }

  async waitForMedia(prefix, minimum, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const count = this.mediaPayloads.filter((payload) => payload.startsWith(prefix)).length;
      if (count >= minimum) return count;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    throw new Error(`${this.label}: did not receive ${minimum} ${prefix} RTP payloads`);
  }

  async reportControls() {
    await this.send(
      "dtmf.send",
      { digits: "5", duration_ms: 120, method: "rfc4733" },
      { cid: this.cid, sid: this.sid, connid: this.connid },
      true,
    );
    await this.send(
      "connection.quality",
      {
        interval_ms: 1000,
        streams: [
          {
            strm_id: this.streamId,
            loss_pct: 0.1,
            jitter_ms: 3,
            rtt_ms: 12,
            mos: 4.2,
            bitrate_bps: 64000,
            packets_sent: 30,
            packets_received: 30,
          },
        ],
      },
      { cid: this.cid, sid: this.sid, connid: this.connid },
      true,
    );
  }

  async close() {
    if (this.sid) {
      await this.send(
        "session.end",
        { by: this.participantId, reason_code: 200, reason: "browser-smoke-complete" },
        { cid: this.cid, sid: this.sid },
        true,
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
    this.closed = true;
    this.transport.close({ closeCode: 0, reason: "browser-smoke-complete" });
    await this.transport.closed;
  }
}

export async function runWebTransportAcceptance(privateKeyPkcs8B64) {
  const signingKey = await crypto.subtle.importKey(
    "pkcs8",
    fromBase64(privateKeyPkcs8B64),
    "Ed25519",
    false,
    ["sign"],
  );

  const alice = new UctpBrowserPeer("alice", signingKey);
  const bob = new UctpBrowserPeer("bob", signingKey);
  await Promise.all([alice.connect(), bob.connect()]);
  await Promise.all([alice.authenticate(), bob.authenticate()]);
  await Promise.all([alice.establishMedia(), bob.establishMedia()]);

  // The harness bridges the first two inbound browser connections. Give the
  // stream graph one scheduling turn before sending a sustained 600 ms burst.
  await new Promise((resolve) => setTimeout(resolve, 250));
  await Promise.all([alice.sendMedia("alice", 30), bob.sendMedia("bob", 30)]);
  const [aliceReceived, bobReceived] = await Promise.all([
    alice.waitForMedia("bob", 24),
    bob.waitForMedia("alice", 24),
  ]);
  await Promise.all([alice.reportControls(), bob.reportControls()]);
  await Promise.all([alice.close(), bob.close()]);

  // The server admits only two simultaneous peers in smoke mode. A complete
  // third connection after both close proves the connection permits and peer
  // coordinators were released instead of leaking at capacity.
  const reconnected = new UctpBrowserPeer("reconnected", signingKey);
  await reconnected.connect();
  await reconnected.authenticate();
  await reconnected.establishMedia();
  await reconnected.close();

  return {
    complete: true,
    signedControls: true,
    aliceReceived,
    bobReceived,
    reconnected: true,
  };
}
