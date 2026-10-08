/** Experimental UCTP v1 control over WebSocket; browser and Node 22+. */
export function envelope(type, payload = {}, ids = {}) {
  return {
    ...ids, v: 1, type, id: `env_${crypto.randomUUID().replaceAll('-', '')}`,
    ts: new Date().toISOString(), payload,
  };
}

/** Diagnostic projection only. Never use the result for negotiation or replay. */
export function redactEnvelope(frame) {
  const copy = structuredClone(frame);
  if (copy.type?.startsWith('auth.')) copy.payload = { redacted: true };
  if (copy.signature) copy.signature = '[redacted]';
  function scrub(value) {
    if (!value || typeof value !== 'object') return;
    for (const [key, child] of Object.entries(value)) {
      if (['credential', 'password', 'token', 'session_token', 'authorization', 'api_key', 'access_token', 'refresh_token', 'client_secret'].includes(key.toLowerCase())) {
        value[key] = '[redacted]';
      } else if (key.toLowerCase() === 'sdp' && typeof child === 'string') {
        value[key] = child.replace(/^(a=(?:ice-pwd|ice-ufrag|crypto|key-mgmt):|k=)[^\r\n]*/gmi, '$1[redacted]');
      } else scrub(child);
    }
  }
  scrub(copy);
  return copy;
}

export class UctpError extends Error {
  constructor(message, { code, requestId, outcomeUnknown = false, response } = {}) {
    super(message);
    this.name = 'UctpError';
    this.code = code;
    this.requestId = requestId;
    this.outcomeUnknown = outcomeUnknown;
    this.response = response;
  }
}

function validEnvelope(frame) {
  return frame && typeof frame === 'object' && !Array.isArray(frame)
    && frame.v === 1 && typeof frame.type === 'string' && frame.type.length > 0
    && typeof frame.id === 'string' && frame.id.length > 0
    && typeof frame.ts === 'string' && Number.isFinite(Date.parse(frame.ts))
    && frame.payload && typeof frame.payload === 'object' && !Array.isArray(frame.payload)
    && (frame.in_reply_to == null || typeof frame.in_reply_to === 'string');
}

export class UctpClient {
  // Private so the bearer credential never appears in JSON.stringify,
  // Object.keys/entries, structuredClone or console/util.inspect output.
  #token;

  constructor(url, token, {
    WebSocketImpl = globalThis.WebSocket, timeoutMs = 10000,
    maxPending = 128, maxFrameBytes = 131072, applicationProfile,
    trace = () => {},
  } = {}) {
    const parsed = new URL(url);
    if (!['ws:', 'wss:'].includes(parsed.protocol) || parsed.username || parsed.password || parsed.hash) {
      throw new Error('UCTP WebSocket URL required without credentials or fragment');
    }
    if (parsed.protocol === 'ws:' && !['localhost', '127.0.0.1', '[::1]'].includes(parsed.hostname)) {
      throw new Error('Remote UCTP requires wss');
    }
    if (typeof token !== 'string' || !token) throw new Error('Bearer credential required');
    if (typeof WebSocketImpl !== 'function') throw new Error('WebSocket implementation required');
    for (const value of [timeoutMs, maxPending, maxFrameBytes]) {
      if (!Number.isSafeInteger(value) || value <= 0) throw new Error('Client limits must be positive integers');
    }
    if (applicationProfile !== undefined && (typeof applicationProfile !== 'string' || !applicationProfile)) {
      throw new Error('Application profile must be a nonempty string');
    }
    this.url = parsed.href;
    this.#token = token;
    this.WebSocketImpl = WebSocketImpl;
    this.timeoutMs = timeoutMs;
    this.maxPending = maxPending;
    this.maxFrameBytes = maxFrameBytes;
    this.applicationProfile = applicationProfile;
    this.trace = trace;
    this.pending = new Map();
    this.listeners = new Set();
    this.ws = null;
    this.authenticated = false;
  }

  emitTrace(direction, frame) {
    try { this.trace(direction, redactEnvelope(frame)); } catch { /* Diagnostics do not own transport. */ }
  }

  async connect() {
    if (this.ws) throw new Error('Client already connected; close before reconnecting');
    const ws = new this.WebSocketImpl(this.url);
    this.ws = ws;
    ws.addEventListener('message', event => {
      if (this.ws !== ws) return;
      let frame;
      try {
        if (typeof event.data !== 'string' || new TextEncoder().encode(event.data).length > this.maxFrameBytes) {
          throw new Error('Invalid frame size or kind');
        }
        frame = JSON.parse(event.data);
        if (!validEnvelope(frame)) throw new Error('Invalid envelope');
      } catch {
        this.disconnect(ws, 'Invalid UCTP frame', 1002);
        return;
      }
      this.emitTrace('recv', frame);
      const pending = frame.in_reply_to && this.pending.get(frame.in_reply_to);
      if (pending) {
        clearTimeout(pending.timer);
        this.pending.delete(frame.in_reply_to);
        if (frame.type === 'error') {
          pending.reject(new UctpError('UCTP request rejected', {
            code: frame.payload.code, requestId: frame.in_reply_to, response: redactEnvelope(frame),
          }));
        } else pending.resolve(frame);
      } else {
        for (const listener of this.listeners) {
          try { listener(structuredClone(frame)); } catch { /* Observers do not own correlation. */ }
        }
      }
    });
    ws.addEventListener('close', () => {
      if (this.ws === ws) this.disconnect(ws, 'UCTP disconnected');
    });
    try {
      await new Promise((resolve, reject) => {
        const finish = error => {
          clearTimeout(timer);
          ws.removeEventListener('open', opened);
          ws.removeEventListener('error', failed);
          ws.removeEventListener('close', closed);
          error ? reject(error) : resolve();
        };
        const opened = () => finish();
        const failed = () => finish(new UctpError('UCTP connection failed'));
        const closed = () => finish(new UctpError('UCTP connection closed'));
        const timer = setTimeout(() => finish(new UctpError('UCTP connection timed out')), this.timeoutMs);
        ws.addEventListener('open', opened, { once: true });
        ws.addEventListener('error', failed, { once: true });
        ws.addEventListener('close', closed, { once: true });
      });
      const hello = await this.#send(envelope('auth.hello', {
        device: { id: `dev_${crypto.randomUUID()}`, kind: 'desktop', platform: 'javascript', sdk_version: '@rvoip/uctp-websocket/0.1.0' },
        auth_methods: ['bearer'],
        capabilities: this.applicationProfile ? { application_profiles: [this.applicationProfile] } : {},
      }));
      if (hello.type !== 'auth.challenge' || !hello.payload.accepted_methods?.includes('bearer')) {
        throw new UctpError('Server did not offer bearer authentication');
      }
      if (this.applicationProfile && !hello.payload.server_capabilities?.application_profiles?.includes(this.applicationProfile)) {
        throw new UctpError('Server did not advertise the requested application profile');
      }
      const auth = await this.#send(envelope('auth.response', {
        method: 'bearer', credential: this.#token,
      }, { in_reply_to: hello.id }));
      if (auth.type !== 'auth.session') throw new UctpError('Authentication did not establish a session');
      // A peer may close immediately after a correlated auth.session.
      if (this.ws !== ws || ws.readyState !== 1) throw new UctpError('UCTP disconnected during authentication');
      this.authenticated = true;
      this.identity = auth.payload.identity_id;
      return auth.payload;
    } catch (error) {
      if (this.ws === ws) this.disconnect(ws, 'UCTP connection closed');
      throw error;
    }
  }

  disconnect(ws, message, code = 1000) {
    if (this.ws !== ws) return;
    this.ws = null;
    this.authenticated = false;
    this.identity = undefined;
    for (const [requestId, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.reject(new UctpError(message, { requestId, outcomeUnknown: true }));
    }
    this.pending.clear();
    try { ws.close(code); } catch { /* The peer may already have closed. */ }
  }

  close() {
    if (this.ws) this.disconnect(this.ws, 'UCTP client closed');
  }

  onEvent(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Build a command; retain this exact envelope before submitting an effect. */
  command(type, payload = {}, ids = {}) {
    return envelope(type, this.applicationProfile ? { ...payload, profile: this.applicationProfile } : payload, ids);
  }

  /** Never automatically retries a command or resumes a logical Session. */
  request(request) {
    if (!validEnvelope(request)) return Promise.reject(new UctpError('Invalid UCTP request'));
    if (!this.authenticated || request.type?.startsWith('auth.')) {
      return Promise.reject(new UctpError('Authenticate before sending application requests', { requestId: request.id }));
    }
    return this.#send(request);
  }

  #send(request) {
    if (!validEnvelope(request)) return Promise.reject(new UctpError('Invalid UCTP request'));
    if (!this.ws || this.ws.readyState !== 1) return Promise.reject(new UctpError('UCTP is not connected', { requestId: request.id }));
    if (this.pending.has(request.id)) return Promise.reject(new UctpError('Request already pending', { requestId: request.id }));
    if (this.pending.size >= this.maxPending) return Promise.reject(new UctpError('Pending request capacity reached', { requestId: request.id }));
    let serialized;
    try { serialized = JSON.stringify(request); } catch { return Promise.reject(new UctpError('Request encoding failed', { requestId: request.id })); }
    if (new TextEncoder().encode(serialized).length > this.maxFrameBytes) return Promise.reject(new UctpError('Request exceeds frame limit', { requestId: request.id }));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(request.id);
        reject(new UctpError('UCTP request timed out', { requestId: request.id, outcomeUnknown: true }));
      }, this.timeoutMs);
      this.pending.set(request.id, { resolve, reject, timer });
      this.emitTrace('send', request);
      try { this.ws.send(serialized); } catch {
        clearTimeout(timer);
        this.pending.delete(request.id);
        reject(new UctpError('UCTP send failed', { requestId: request.id, outcomeUnknown: true }));
      }
    });
  }
}
