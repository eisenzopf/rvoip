/** Pair with rvoip-websocket's application_profile --server example. */
import assert from 'node:assert/strict';
import { UctpClient } from '../client.mjs';

const url = process.env.RVOIP_EXAMPLE_URL;
const token = process.env.RVOIP_EXAMPLE_TOKEN;
if (!url || !token) throw new Error('Set RVOIP_EXAMPLE_URL and RVOIP_EXAMPLE_TOKEN');

const client = new UctpClient(url, token, { applicationProfile: 'example.echo/1' });
try {
  await client.connect();
  const request = client.command('example.echo', { text: 'Hello from JavaScript' });
  const reply = await client.request(request);
  assert.equal(reply.type, 'ack');
  assert.equal(reply.in_reply_to, request.id);
  assert.equal(reply.payload.text, 'Hello from JavaScript');
  await assert.rejects(client.request(request), error => error.code === 409 && !error.outcomeUnknown);
  console.log('Authenticated, correlated echo; duplicate command refused with 409');
} finally {
  client.close();
}
