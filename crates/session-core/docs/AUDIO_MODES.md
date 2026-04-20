# Audio Modes: PassThrough (endpoint) vs Bridge (b2bua)

`session-core` supports two distinct audio topologies. They operate at
**different layers** and coexist cleanly — one call can use either mode, and
a b2bua can switch between them per leg as the call evolves.

This document is the reference for app authors choosing between them and
for b2bua authors layering them together.

---

## TL;DR

| | Endpoint (softphone, IVR, agent) | B2BUA (forwarding leg ↔ leg) |
|---|---|---|
| **Mode** | `AudioSource::PassThrough` on the transmitter | `UnifiedCoordinator::bridge()` at the packet layer |
| **Audio path** | App ⇄ `AudioStream` ⇄ encoder/decoder ⇄ RTP | RTP packets forwarded payload-for-payload between two legs |
| **Transcoding** | Yes (the app's PCM samples are encoded to the negotiated codec) | No (both legs must match codec) |
| **Source of truth for audio** | App feeds / consumes samples via `AudioStream` | Neither session has "its own" audio — both legs carry the other's RTP |
| **CPU cost** | One encode + one decode per direction | Zero decode, zero encode |
| **Tone / announcement injection** | `set_audio_source(AudioSource::Tone/CustomSamples)` | Not directly — see "Mixing the two" below |

---

## Endpoint mode — `AudioSource::PassThrough`

The **default** audio mode for every session. The transmitter runs in
`PassThrough` which means "don't generate any audio, just forward whatever
the app feeds in via `AudioStream`."

### Typical consumer

Softphones, IVRs, agents, voicemail systems — anything that *terminates*
the RTP at this process and wants to read/write PCM samples from/to the
app.

### API

```rust
use rvoip_session_core::{StreamPeer, AudioSource};

let mut peer = StreamPeer::new("alice").await?;
let handle = peer.call("sip:bob@example.com").await?;
let handle = peer.wait_for_answered(handle.id()).await?;

// Bidirectional audio via the handle's AudioStream. PassThrough is already
// the active source — no explicit setup needed.
let mut stream = handle.audio_stream().await?;
stream.send(pcm_samples).await?;
let inbound = stream.recv().await?;
```

### Early-media: custom source during `EarlyMedia`, auto-switchback on `Active`

For UAS flows that want to play a ringback tone or announcement before
answering:

```rust
let incoming = peer.wait_for_incoming().await?;
incoming
    .send_early_media_with_source(
        None,
        AudioSource::Tone { frequency: 440.0, amplitude: 0.5 },
    )
    .await?;
// Tone plays during EarlyMedia.

// After the app calls accept(), the state machine auto-switches the
// transmitter back to AudioSource::PassThrough on the DialogACK → Active
// transition (see state_tables/default.yaml transitions that include the
// SwitchToPassThroughOnActive action). No manual reset needed.
let handle = incoming.accept().await?;
// Bidirectional audio flows normally — feed samples via AudioStream.
```

Want a custom source to *keep* playing after answer (e.g., hold music for
a "please hold" call)? Call `set_audio_source` again after you see
`Event::CallEstablished`:

```rust
// App subscribes to events and reacts post-answer:
let coordinator = peer.control().coordinator();
coordinator.set_audio_source(&call_id, AudioSource::CustomSamples {
    samples: hold_music_wav,
    repeat: true,
}).await?;
```

### Where it lives

- Transmitter: `crates/media-core/src/relay/controller/rtp_management.rs`
  (`start_audio_transmission_with_config`, `set_audio_source`,
  `set_pass_through_mode`).
- State-machine auto-switchback: `crates/session-core/state_machine/actions.rs`
  (`Action::SwitchToPassThroughOnActive`), wired in `state_tables/default.yaml`.
- Tests: `crates/session-core/tests/early_media_tests.rs` (state-table
  wiring), `crates/session-core/examples/streampeer/prack/` (end-to-end).

---

## B2BUA mode — `UnifiedCoordinator::bridge()`

A transparent RTP relay between **two** sessions in the same process. No
decoding, no re-encoding, no AudioFrame traversal — the forwarder
subscribes to the source session's `RtpSessionEvent::PacketReceived`
broadcast and replays each packet's payload + timestamp + marker bit
directly onto the destination session's RTP socket.

### Typical consumer

B2BUA wrappers, call-recording forks, transparent proxies — anything
forwarding media between two SIP legs without processing the samples.

### API

```rust
use rvoip_session_core::{UnifiedCoordinator, AudioSource};

// Two legs, both Active:
let inbound = coordinator.accept_call(&inbound_id).await?;
let outbound = coordinator.make_call(local_uri, outbound_uri).await?;
let events = coordinator.events_for_session(&outbound).await?;
// …wait for outbound CallAnswered…

// Bridge at the packet level. Dropping the handle tears it down.
let bridge = coordinator.bridge(&inbound_id, &outbound).await?;
// Audio now flows leg-to-leg without traversing any audio pipeline.

// When either leg ends, drop the bridge and hang up the partner:
drop(bridge);
coordinator.hangup(&inbound_id).await?;
coordinator.hangup(&outbound).await?;
```

### Preconditions enforced at `bridge()` call time

- Both sessions must be `Active` (remote RTP address known) — else
  `BridgeError::SessionNotActive`.
- Negotiated payload types must match — else `BridgeError::CodecMismatch
  { a_pt, b_pt }`. **No transcoding is performed.**
- Neither session may already be bridged — else `BridgeError::AlreadyBridged`.

### What's forwarded and what isn't

- **Forwarded:** RTP payload + timestamp + marker bit. Each destination RTP
  session assigns its own sequence number and SSRC.
- **Also forwarded (transparently):** RFC 2833 DTMF events — they ride
  the same payload stream.
- **NOT forwarded:** RTCP. Per RFC 3550 §7.2, each leg generates its own
  reports. Reciprocally relaying them would break loss/jitter calculations.

### Where it lives

- Primitive: `crates/media-core/src/relay/controller/bridge.rs`
  (`MediaSessionController::bridge_sessions`, `BridgeHandle`).
- Public API: `UnifiedCoordinator::bridge` in
  `crates/session-core/src/api/unified.rs`.
- Tests: `crates/session-core/tests/bridge_roundtrip_integration.rs`
  (3-peer end-to-end with Goertzel-asserted tones).
- Skeleton: `examples/streampeer/bridge/bridge_peer.rs` (the b2bua
  skeleton the roadmap calls out as Item 6 — lift into `crates/b2bua`).

---

## Mixing the two

The transmitter and the bridge **operate at different layers** on the
same RTP session. They are not mutually exclusive:

- Bridge uses packet-level forwarding on `dst_session.send_packet(...)`.
- Transmitter in PassThrough mode emits *only* what the app feeds in
  via `AudioStream`. With no samples fed in, it produces nothing.

So for a **pure b2bua** (no audio injection), leave both legs in
`PassThrough` — the transmitter is dormant, and the bridge carries the
actual audio. There's no collision because no packets are emitted from
the transmitter side.

### B2BUA that plays an announcement before bridging

```rust
// 1. Install a tone on the inbound leg during EarlyMedia.
incoming.send_early_media_with_source(
    None,
    AudioSource::Tone { frequency: 440.0, amplitude: 0.5 },
).await?;

// 2. Place the outbound call, wait for answer.
let outbound = coordinator.make_call(local_uri, outbound_uri).await?;
let mut out_events = coordinator.events_for_session(&outbound).await?;
loop {
    match out_events.next().await {
        Some(Event::CallAnswered { .. }) => break,
        Some(Event::CallFailed { .. }) | Some(Event::CallEnded { .. }) => {
            return Err(/* outbound didn't answer */);
        }
        Some(_) => continue,
        None => return Err(/* stream closed */),
    }
}

// 3. Accept the inbound leg. The state machine's
//    SwitchToPassThroughOnActive action auto-resets the transmitter,
//    removing the tone and arming the inbound leg for bridging.
incoming.accept().await?;

// 4. Bridge the two legs. Audio now flows end-to-end.
let bridge = coordinator.bridge(&inbound_id, &outbound).await?;
```

Follow-up A (auto-switchback) is what makes step 3 work without the
b2bua having to remember to `set_audio_source(PassThrough)` before
bridging. Without it, the tone would keep playing onto the bridged
stream.

### B2BUA that injects announcements mid-call

Not supported by the packet-level bridge (would require the transcoded-
bridge upgrade the roadmap lists as a future follow-up to Item 2). For
today, the b2bua would have to:

1. `drop(bridge)` the existing bridge.
2. `set_audio_source(Tone/CustomSamples)` on one leg.
3. Wait for the tone to finish.
4. `set_audio_source(PassThrough)` to re-arm.
5. Re-`bridge()` the two legs.

Mid-bridge injection is on the "nice to have" list for the transcoded
bridge. Apps that need it today should terminate one leg at the app
process (endpoint mode), handle the announcement there, and forward
samples explicitly.

---

## Cross-reference

| Concern | Look here |
|---------|-----------|
| Full roadmap incl. Items 1–5 and the two follow-ups | `PRE_B2BUA_ROADMAP.md` |
| Which peer type to use (`CallbackPeer` / `StreamPeer` / `UnifiedCoordinator`) | `TELCO_USE_CASE_ANALYSIS.md` |
| B2BUA skeleton (~157 LOC) ready for extraction into `crates/b2bua` | `examples/streampeer/bridge/bridge_peer.rs` |
| End-to-end RTP test for the bridge primitive | `tests/bridge_roundtrip_integration.rs` |
| State-table wiring for auto-switchback | `tests/early_media_tests.rs` |
| 422 session-timer retry (different hardening track, not audio-mode related) | `tests/session_422_retry.rs` |
