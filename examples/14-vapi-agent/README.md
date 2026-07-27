# 14 — SIP or WebRTC callers to a Vapi voice agent

This is one high-level `rvoip` server with a transport flag:

```text
SIP or WebRTC caller → rvoip::app → shared Orchestrator bridge → Vapi WebSocket
```

The listener is the only transport-specific part. Call admission,
`ConnectionId`, Vapi attachment, media bridging, events, and symmetric teardown
use the same `rvoip` APIs in both modes.

## Configure Vapi

Use a saved assistant:

```sh
export VAPI_API_KEY="..."
export VAPI_ASSISTANT_ID="..."
```

Or omit `VAPI_ASSISTANT_ID` and provide a complete transient assistant object
following Vapi's
[transient configuration documentation](https://docs.vapi.ai/assistants/concepts/transient-vs-permanent-configurations):

```sh
export VAPI_TRANSIENT_ASSISTANT_JSON="$(cat assistant.json)"
```

## Accept SIP calls

```sh
cargo run -- --transport sip
```

The safe default listens on loopback. For another machine on your LAN, bind all
interfaces while explicitly supplying the concrete address rvoip should put in
SIP headers and RTP SDP:

```sh
cargo run -- \
  --transport sip \
  --bind 0.0.0.0:5060 \
  --sip-advertise 192.168.1.50:5060
```

Send an INVITE to `sip:vapi@SERVER_IP:5060`. Auto audio mode uses raw PCMU at
8 kHz, giving a PCMU caller a pass-through path. Use `--audio pcm16` to exercise
the PCM-to-G.711 transcoder instead.

For a 1:1 NAT, `--sip-advertise` may be the public address if the same mapped
RTP ports reach this process. It also supplies the default advertised RTP IP.
Use `--rtp-advertise MEDIA_IP:0` when signaling and media use different public
addresses. More complex Internet edges generally need an SBC or explicit
NAT/media routing.

## Accept WebRTC calls

```sh
cargo run -- --transport webrtc --bind 127.0.0.1:8081
```

Connect an audio caller to the printed WebSocket signaling URL. Auto audio mode
uses signed little-endian PCM at 16 kHz so the WebRTC Opus leg keeps a wideband
path. `--audio mulaw` is available when narrowband interoperability is desired.

The WebSocket signaler accepts rvoip offer/answer messages. The simplest
legacy-compatible socket sends:

```json
{"type":"offer","sdp":"..."}
```

It returns:

```json
{"type":"answer","sdp":"...","connection_id":"..."}
```

If the client negotiates the `rvoip.webrtc.v1` WebSocket subprotocol, include a
bounded client-generated `request_id` in the offer; rvoip echoes it in the
answer:

```json
{"type":"offer","sdp":"...","request_id":"call-1"}
```

For microphone access, serve your browser client from `localhost` or HTTPS.

## What to look for

The shared event handler receives `AppEvent::InboundCallAccepted` for either
transport and runs:

```rust
vapi.attach_agent(&orchestrator, connection_id, options).await?
```

`attach_agent` originates the Vapi WebSocket leg, bridges full-duplex audio,
and supervises both sides. If the caller hangs up, rvoip sends Vapi
`end-call`; if Vapi ends first, the caller is ended by the default peer policy.

Run `cargo run -- --help` for all flags. `RUST_LOG=debug` enables more
transport diagnostics; credentials, socket URLs, assistant content,
transcripts, and audio are redacted from default adapter diagnostics.
