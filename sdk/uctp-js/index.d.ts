export interface EnvelopeIds {
  cid?: string;
  sid?: string;
  connid?: string;
  in_reply_to?: string;
}
export interface UctpEnvelope extends EnvelopeIds {
  v: 1;
  type: string;
  id: string;
  ts: string;
  payload: Record<string, unknown>;
  signature?: unknown;
}
export function envelope(type: string, payload?: Record<string, unknown>, ids?: EnvelopeIds): UctpEnvelope;
export function redactEnvelope(frame: UctpEnvelope): UctpEnvelope;
export interface UctpErrorOptions {
  code?: number;
  requestId?: string;
  outcomeUnknown?: boolean;
  response?: UctpEnvelope;
}
export class UctpError extends Error {
  constructor(message: string, options?: UctpErrorOptions);
  code?: number;
  requestId?: string;
  outcomeUnknown: boolean;
  response?: UctpEnvelope;
}
export interface UctpClientOptions {
  WebSocketImpl?: typeof WebSocket;
  timeoutMs?: number;
  maxPending?: number;
  maxFrameBytes?: number;
  applicationProfile?: string;
  trace?: (direction: 'send' | 'recv', frame: UctpEnvelope) => void;
}
export class UctpClient {
  constructor(url: string, token: string, options?: UctpClientOptions);
  readonly applicationProfile?: string;
  readonly authenticated: boolean;
  readonly identity?: string;
  connect(): Promise<Record<string, unknown>>;
  close(): void;
  onEvent(listener: (frame: UctpEnvelope) => void): () => void;
  command(type: string, payload?: Record<string, unknown>, ids?: EnvelopeIds): UctpEnvelope;
  request(request: UctpEnvelope): Promise<UctpEnvelope>;
}
