import { UctpClient, UctpError, envelope, redactEnvelope, type UctpEnvelope } from '@rvoip/uctp-websocket';

const client = new UctpClient('wss://host.example/uctp', 'development-token', {
  applicationProfile: 'example/1', WebSocketImpl: WebSocket,
  trace: (direction, frame) => { const dir: 'send' | 'recv' = direction; const version: 1 = frame.v; void [dir, version]; },
});
const request: UctpEnvelope = client.command('example.echo', { text: 'hello' }, { cid: 'conv_example' });
const response: Promise<UctpEnvelope> = client.request(request);
const unsubscribe: () => void = client.onEvent(frame => { const type: string = frame.type; void type; });
const safe: UctpEnvelope = redactEnvelope(envelope('example.echo'));
const unknown: boolean = new UctpError('timeout', { requestId: request.id, outcomeUnknown: true }).outcomeUnknown;
void [response, unsubscribe, safe, unknown];

// @ts-expect-error A request needs a complete v1 envelope.
client.request({ type: 'example.echo' });
// @ts-expect-error Limits are numeric.
new UctpClient('wss://host.example/uctp', 'token', { maxPending: 'unbounded' });
