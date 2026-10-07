import test from 'node:test';
import assert from 'node:assert/strict';
import { UctpClient, UctpError, envelope, redactEnvelope } from '../client.mjs';

function fixture({ profiles = ['example/1'], authError = false, stalled = false } = {}) {
  return class Socket extends EventTarget {
    static instances = [];
    constructor() {
      super(); this.readyState = 0; this.sent = []; this.closeCodes = [];
      this.constructor.instances.push(this);
      if (!stalled) queueMicrotask(() => {
        this.readyState = 1; this.dispatchEvent(new Event('open'));
      });
    }
    send(serialized) {
      const request = JSON.parse(serialized); this.sent.push(request);
      if (request.type === 'auth.hello') queueMicrotask(() => this.reply(request, 'auth.challenge', {
        accepted_methods: ['bearer'], server_capabilities: { application_profiles: profiles },
      }));
      if (request.type === 'auth.response') queueMicrotask(() => this.reply(request,
        authError ? 'error' : 'auth.session', authError ? { code: 401, credential: 'AUTH_CANARY' } : { identity_id: 'participant' }));
    }
    emit(frame) { this.raw(JSON.stringify(frame)); }
    raw(data) { this.dispatchEvent(new MessageEvent('message', { data })); }
    reply(request, type = 'ack', payload = {}) {
      this.emit(envelope(type, payload, { in_reply_to: request.id }));
    }
    close(code) {
      this.closeCodes.push(code);
      if (this.readyState === 3) return;
      this.readyState = 3; this.dispatchEvent(new Event('close'));
    }
  };
}

async function connected(options = {}) {
  const Socket = fixture();
  const client = new UctpClient('ws://localhost:1', 'BEARER_CANARY', {
    WebSocketImpl: Socket, applicationProfile: 'example/1', ...options,
  });
  await client.connect();
  return { client, socket: Socket.instances[0], Socket };
}

test('negotiate before credentials, then correlate reversed replies around events', async t => {
  const trace = [];
  const { client, socket } = await connected({ trace: (direction, frame) => trace.push({ direction, frame }) });
  t.after(() => client.close());
  assert.equal(client.identity, 'participant');
  assert.equal(socket.sent[1].in_reply_to, trace[1].frame.id);
  const events = [];
  client.onEvent(frame => events.push(frame));
  const a = client.command('example.echo', { value: 1 }, { cid: 'conv_a' });
  const b = client.command('example.echo', { value: 2 }, { cid: 'conv_a' });
  assert.equal(a.payload.profile, 'example/1');
  const first = client.request(a), second = client.request(b);
  socket.emit(envelope('example.event', { value: 3 }));
  socket.reply(b, 'ack', { value: 2 }); socket.reply(a, 'ack', { value: 1 });
  assert.equal((await first).payload.value, 1);
  assert.equal((await second).payload.value, 2);
  assert.equal(events.length, 1);
  assert.ok(!JSON.stringify(trace).includes('BEARER_CANARY'));
});

test('profile mismatch closes before any bearer credential is submitted', async () => {
  const Socket = fixture({ profiles: [] });
  const client = new UctpClient('ws://localhost:1', 'BEARER_CANARY', { WebSocketImpl: Socket, applicationProfile: 'example/1' });
  await assert.rejects(client.connect(), /advertise/);
  assert.deepEqual(Socket.instances[0].sent.map(frame => frame.type), ['auth.hello']);
  assert.equal(client.authenticated, false);
});

test('legacy UCTP does not require an application profile', async t => {
  const Socket = fixture({ profiles: [] });
  const client = new UctpClient('ws://localhost:1', 'token', { WebSocketImpl: Socket });
  t.after(() => client.close());
  await client.connect();
  assert.deepEqual(Socket.instances[0].sent[0].payload.capabilities, {});
  assert.equal(client.command('message.history').payload.profile, undefined);
});

test('authentication rejection carries redacted response and no raw request', async () => {
  const Socket = fixture({ authError: true });
  const client = new UctpClient('ws://localhost:1', 'BEARER_CANARY', { WebSocketImpl: Socket });
  await assert.rejects(client.connect(), error => {
    assert.equal(error.code, 401);
    assert.ok(!JSON.stringify(error).includes('CANARY'));
    assert.equal(error.outcomeUnknown, false);
    return true;
  });
  assert.equal(client.authenticated, false);
});

test('timeout retains command ID and does not repeat effects; reconnect permits explicit replay', async t => {
  const { client, socket, Socket } = await connected({ timeoutMs: 25 });
  t.after(() => client.close());
  const request = client.command('example.effect', { body: 'one' });
  await assert.rejects(client.request(request), error => error instanceof UctpError
    && error.requestId === request.id && error.outcomeUnknown);
  assert.equal(socket.sent.filter(frame => frame.id === request.id).length, 1);
  client.close();
  await client.connect();
  const next = Socket.instances[1];
  const replay = client.request(request);
  assert.deepEqual(next.sent.at(-1), request);
  next.reply(request);
  await replay;
});

test('disconnect rejects pending commands; stale peer messages cannot resolve new requests', async t => {
  const { client, socket, Socket } = await connected();
  t.after(() => client.close());
  const request = client.command('example.effect');
  const first = client.request(request);
  socket.close();
  await assert.rejects(first, error => error.requestId === request.id && error.outcomeUnknown);
  await client.connect();
  const replay = client.request(request);
  socket.reply(request, 'ack', { stale: true });
  Socket.instances[1].reply(request, 'ack', { current: true });
  assert.deepEqual((await replay).payload, { current: true });
});

test('capacity and duplicate-ID refusal do not overwrite the first waiter', async t => {
  const { client, socket } = await connected({ maxPending: 1 });
  t.after(() => client.close());
  const request = client.command('example.effect');
  const pending = client.request(request);
  await assert.rejects(client.request(request), /already pending/);
  await assert.rejects(client.request(client.command('example.effect')), /capacity/);
  socket.reply(request); await pending;
  assert.equal(socket.sent.filter(frame => frame.type === 'example.effect').length, 1);
});

test('invalid, binary and oversized frames close the peer and reject outstanding requests', async () => {
  for (const data of ['{', 'null', JSON.stringify({ v: 2 }), new Uint8Array([1]), 'x'.repeat(2049)]) {
    const { client, socket } = await connected({ maxFrameBytes: 2048 });
    const request = client.command('example.effect');
    const pending = client.request(request);
    socket.raw(data);
    await assert.rejects(pending, error => /Invalid UCTP frame/.test(error.message) && error.outcomeUnknown);
    assert.equal(socket.closeCodes[0], 1002);
    assert.equal(client.authenticated, false);
  }
});

test('observer and trace exceptions cannot change correlation or other observers', async t => {
  const { client, socket } = await connected({ trace: () => { throw new Error('observer failure'); } });
  t.after(() => client.close());
  client.onEvent(frame => { frame.payload.value = 'modified'; throw new Error('observer failure'); });
  const received = [];
  const unsubscribe = client.onEvent(frame => received.push(frame.payload.value));
  socket.emit(envelope('example.event', { value: 'original' }));
  unsubscribe(); socket.emit(envelope('example.event', { value: 'second' }));
  const request = client.command('example.effect');
  const pending = client.request(request); socket.reply(request); await pending;
  assert.deepEqual(received, ['original']);
});

test('outbound frame limits and encoding errors fail before sending', async t => {
  const { client, socket } = await connected({ maxFrameBytes: 2048 });
  t.after(() => client.close());
  const count = socket.sent.length;
  await assert.rejects(client.request(client.command('example.effect', { body: 'x'.repeat(2048) })), /frame limit/);
  const cyclic = {}; cyclic.self = cyclic;
  await assert.rejects(client.request(client.command('example.effect', cyclic)), /encoding/);
  assert.equal(socket.sent.length, count);
});

test('connection deadline closes an unopened socket', async () => {
  const Socket = fixture({ stalled: true });
  const client = new UctpClient('ws://localhost:1', 'token', { WebSocketImpl: Socket, timeoutMs: 10 });
  await assert.rejects(client.connect(), /timed out/);
  assert.equal(Socket.instances[0].readyState, 3);
});

test('reject remote cleartext, userinfo, invalid limits, and requests before authentication', async () => {
  assert.throws(() => new UctpClient('ws://example.com', 'token'), /wss/);
  assert.throws(() => new UctpClient('wss://token@example.com', 'token'), /credentials/);
  assert.throws(() => new UctpClient('wss://example.com', 'token', { maxPending: 0 }), /positive/);
  assert.throws(() => new UctpClient('wss://example.com', 'token', { applicationProfile: '' }), /profile/);
  const client = new UctpClient('ws://localhost:1', 'token');
  await assert.rejects(client.request(null), /Invalid UCTP request/);
  await assert.rejects(client.request(client.command('example.effect')), /Authenticate/);
});

test('redaction preserves negotiation while removing nested credentials, signatures and SDP keys', () => {
  const original = envelope('connection.offer', {
    nested: [{ credential: 'CANARY_TURN', api_key: 'CANARY_API', access_token: 'CANARY_ACCESS', client_secret: 'CANARY_CLIENT' }],
    substrate_setup: { sdp: 'v=0\r\na=ice-ufrag:CANARY_USER\r\na=ice-pwd:CANARY_ICE\r\na=crypto:CANARY_KEY\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n' },
  });
  original.signature = 'CANARY_SIGNATURE';
  const redacted = redactEnvelope(original);
  assert.ok(!JSON.stringify(redacted).includes('CANARY'));
  assert.ok(JSON.stringify(original).includes('CANARY'));
  assert.ok(redacted.payload.substrate_setup.sdp.includes('m=audio'));
});
