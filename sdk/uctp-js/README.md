# Experimental UCTP WebSocket client

A dependency-free ESM control client for browsers and Node 22+, extracted from
Parley's UCTP application integration. It implements the UCTP v1 bearer handshake,
request/reply correlation, unsolicited events, bounded request capacity, deadlines,
explicit reconnect, and redacted trace hooks. TypeScript declarations accompany
the JavaScript implementation.

The package is private and is not published to npm. Import it from this checkout:

```js
import { UctpClient } from './sdk/uctp-js/client.mjs';

const client = new UctpClient('wss://your-host.example/uctp', token, {
  applicationProfile: 'your-application/1',
});
await client.connect();
const unsubscribe = client.onEvent(frame => handleEvent(frame));
try {
  const request = client.command('your-operation', { body: 'hello' }, { cid });
  // Persist this exact request before sending a command with external effects.
  await savePending(request);
  const result = await client.request(request);
  await saveResult(result);
} finally {
  unsubscribe();
  client.close();
}
```

`your-operation`, profile names, history cursors and application payloads belong
to the host. The client does not prescribe the experimental Parley
`conversation-control/1` schema or implement WebRTC/media. Omit
`applicationProfile` for profile-free UCTP controls. When a profile is supplied,
the client requires it in the correlated `auth.challenge` before submitting the
bearer credential and adds it to commands. Rvoip's opt-in application dispatch
was introduced in rvoip 0.4.0 (PR #262); a host advertises a profile only when
it installs an application handler.
QUIC and WebTransport are separate clients, outside this package.

## Recover a lost response

`UctpError.requestId` identifies the affected command. `outcomeUnknown` is true
after a send failure, deadline, or disconnect while a request was outstanding.
It does not mean the server rejected the command or that repeating its effects
is safe. A local validation/capacity error or a correlated protocol rejection
does not mark the outcome unknown.

The client never automatically resends. Load the original envelope from your
store, reconnect and authenticate, then inspect or explicitly replay according
to the host's durable idempotency contract. Do not create a new command ID for
an uncertain effect. A host can reject replay; authentication/signature replay
rules still apply. Reconnect establishes a fresh physical connection and does
not claim to resume an existing logical Session or restore media.

## Limits and diagnostics

Defaults are a 10-second request/connection deadline, 128 outstanding requests,
and 128 KiB per outgoing/incoming JSON text frame. Set `timeoutMs`, `maxPending`
and `maxFrameBytes` to match the server. An invalid inbound envelope closes the
peer with code 1002 and rejects its outstanding requests. Stale events from a
previous physical socket cannot resolve requests after reconnect.

Cleartext WebSocket is allowed only for `localhost`, `127.0.0.1` and `[::1]`.
Remote hosts require WSS; URL userinfo is refused. For a browser, serve the page
in a context permitted to connect to the configured endpoint.

The optional `trace(direction, frame)` receives a copy with auth payloads,
signatures, recognized credential fields and SDP key/authentication lines
redacted. Observer exceptions cannot interrupt request correlation.
Application message bodies, transcripts and endpoint data remain content;
redaction does not make arbitrary traces safe to publish. Event listeners
receive independent copies and return an unsubscribe function.

## Verify

```sh
npm --prefix sdk/uctp-js ci
npm --prefix sdk/uctp-js test
npm --prefix sdk/uctp-js run test:types
```

The hermetic Node suite covers negotiation, reversed replies, events,
unknown outcomes, explicit replay, capacity, malformed frames, diagnostics and
cleanup using a controllable WebSocket double. This suite is not a browser/media
interoperability qualification. Real WebSocket integration is exercised by the
separate application-profile example.

When the separate `rvoip-websocket` application-profile example is available,
start its host, set `RVOIP_EXAMPLE_URL` to the printed address and
`RVOIP_EXAMPLE_TOKEN` to its development credential, then run:

```sh
node sdk/uctp-js/examples/profile-echo.mjs
```

This checks the native Node WebSocket handshake, echo and exact duplicate-ID
refusal against the real Rust adapter. The example host is a separate change
stacked on #262; it is not a requirement for the hermetic package tests.
