# Client Integration Feedback

Friction, footguns, and gaps discovered while building a **real softphone client** on
`rvoip-sip` (0.2.x) and taking it all the way to a live, two-way audio call against a
stock Asterisk PBX.

## Where this came from

A Dioxus desktop softphone ([`rvoip_sip_client`]) was migrated from the removed
`rvoip::sip_client::*` API onto the current `StreamPeer` / `PeerControl` / `SessionHandle`
surface, then driven end-to-end:

- registered extension `2001` to Asterisk over plain UDP (Digest auth),
- placed a call to the `600` echo test,
- captured/played real audio through a hand-written **cpal** bridge.

**The signaling and media core were excellent** — transactions were clean and RTP was
gap-free (no packet loss, correct sequence/timestamps) once things were wired correctly.
Almost everything below is about the **"first 30 minutes" of a client author**: the
defaults, missing conveniences, and one example that teaches a bug. Each item lists the
symptom we hit, the root cause in the codebase, the workaround the reference client had to
write, and a suggested fix.

Severity: **P0** = silently breaks a working-looking client · **P1** = ergonomics / missing
piece · **P2** = polish.

| # | Severity | Area | One-liner |
|---|----------|------|-----------|
| 1 | P0 | Auth | REGISTER creds aren't reused for INVITE auth → registered client can't call |
| 2 | P0 | Registration | Default `Contact` is the port-less AOR → inbound calls misroute |
| 3 | P0 | Observability | Per-packet DEBUG/INFO logging starves real-time audio threads |
| 4 | P0 | Example | `examples/sip_client` teaches `sleep`-based mic pacing (drifts, choppy audio) |
| 5 | P1 | API ergonomics | No `session(call_id)` on `StreamPeer`/`PeerControl` |
| 6 | P1 | Types | Two `SessionId` types; public `CallId` has no obvious constructor |
| 7 | P1 | Audio | `AudioFrame` not re-exported → forces a second crate dependency |
| 8 | P1 | Audio | No reusable device helper → every client re-writes the cpal bridge |
| 9 | P1 | Registration | No `register_and_wait` on the StreamPeer/coordinator path |
| 10 | P1 | Transport | Re-binding the same port after teardown races the socket release |
| 11 | P1 | Packaging | `rvoip-client` is a misleading stub |
| 12 | P1 | API clarity | `Endpoint`'s feature ceiling (no attended transfer / REFER) is hidden |
| 13 | P2 | Errors | Host-less request URI yields a cryptic DNS error |

---

## P0 — Footguns that silently break a working-looking client

### 1. Registration credentials are not reused for outbound (INVITE) auth

**Symptom.** A client that registers successfully still cannot place a call. The INVITE is
challenged with `401`, and the authenticated retry fails:

```
ERROR rvoip_sip::state_machine::executor: Failed to execute action SendINVITEWithAuth:
    server challenged INVITE but no credentials are on file
State transition: Initiating -> Failed(Other)
```

**Root cause.** The credentials passed to `coordinator.register(registrar, user, pass)` are
used only for the REGISTER transaction. UAC auth for other in-account requests (INVITE,
re-INVITE, BYE, REFER…) is driven by `Config.credentials` /  `Config.auth`
(`src/api/unified.rs:283` and `:292`). `Action::SendINVITEWithAuth`
(`src/state_machine/actions.rs:~1534`) finds no stashed credentials and aborts. Most PBXes
(Asterisk, FreeSWITCH) challenge INVITE as well as REGISTER, so the default "register, then
call" path fails out of the box.

**Reference-client workaround.** Populate `Config.credentials` separately, duplicating the
registration creds:
```rust
config.credentials = Some(rvoip::sip::types::Credentials::new(username, password));
```

**Suggested fix.** When digest credentials are supplied for an account/registration, default
`Config.credentials` (or a per-account auth context) from them so challenged in-account
requests authenticate automatically. At minimum: surface a single "account" concept that
wires REGISTER + UAC auth together, and document this sharp edge prominently in the auth
guide. **Effort: S–M.**

### 2. `RegisterBuilder` defaults the `Contact` to the port-less AOR → inbound calls misroute

**Symptom.** Registration succeeds and shows `Avail`, but **incoming** calls to the
extension never reach the client. Asterisk stored:
```
2001/sip:2001@192.168.1.104      <-- no :port
```
With `rewrite_contact=no`, the PBX routes inbound INVITEs to `192.168.1.104:5060` (the
default SIP port — in our lab, Asterisk's *own* port), not the client on `:5070`.

**Root cause.** `RegisterBuilder::send()` (`src/api/send/register.rs:~111`) defaults
`contact_uri` to `from_uri`, which defaults to `config.local_uri` — typically the AOR
(`sip:user@domain`) with no transport host/port. A Contact's entire purpose is to be the
reachable transport address, so the AOR default is almost always wrong for receiving calls.

**Reference-client workaround.** Always set an explicit reachable Contact:
```rust
builder = builder.with_contact_uri(format!("sip:{user}@{bind_ip}:{port}"));
```

**Suggested fix.** Default the REGISTER `Contact` to the actual bound transport address
(`bind_ip:port`, scheme/transport per config), not the AOR. Keep `with_contact_uri()` as
the override. **Effort: S.**

### 3. Per-packet DEBUG/INFO logging starves real-time audio threads

**Symptom.** On a debug build, a live call had **audible audio artifacts** that vanished
when logging was reduced to `warn`. During a call the log emits a firehose: RTP header
parsing logs ~12 `debug!` lines per packet, plus per-packet `info!` lines in the media
relay.

**Root cause.** Per-packet logging in the media path, e.g. `crates/media/rtp-core/src/
packet/header.rs:131+` (`debug!` for every field of every packet) and
`crates/media/media-core/src/relay/controller/mod.rs:1133` (`info!` `📦 Received RTP
packet #…` per packet), plus `📤 Sending audio frame` per packet in the SIP media adapter.
At 50 pkt/s that is hundreds of synchronous log writes per second; on an unoptimized build
the CPU/IO contention misses cpal's real-time deadlines → playback underruns.

**Reference-client workaround.** Default `RUST_LOG=warn,sip_client=info`.

**Suggested fix.** Demote per-packet parse/transport/relay logs from `debug!`/`info!` to
`trace!`, and/or gate them behind a `media-trace` feature. `debug` should remain usable
during an active call. **Effort: S.**

### 4. The `examples/sip_client` example teaches `sleep`-based mic pacing (drift → choppy audio)

**Symptom.** A client that ports the example's capture loop sends RTP every **~22 ms**
instead of 20 ms (~10% slow), starving the far end and growing latency → choppy audio.

**Root cause.** `examples/sip_client/audio.rs:159` paces the mic pump with
`tokio::time::sleep(20ms)` *per frame*, on top of the real-time cpal capture. `sleep`
accumulates its own latency (deadline = elapsed + 20 ms + timer slop), so the effective
cadence drifts. This is the canonical "don't pace a real-time source with `sleep`" mistake,
and the example is the first thing client authors copy.

**Reference-client fix (recommended for the example).** Drive sends from a drift-free
`tokio::time::interval(20ms)` (`MissedTickBehavior::Delay`): capture fills a bounded buffer;
each tick emits exactly one 20 ms frame (silence on underrun to hold cadence).

**Suggested fix.** Correct the example and add a short "audio pacing contract" note: emit one
frame per interval tick, never per `sleep`; keep a small jitter buffer on playback (the
example's `VecDeque` has a 2 s cap and a hard-zero underrun that clicks). **Effort: S.**

---

## P1 — Ergonomics & missing pieces

### 5. No `session(call_id)` on `StreamPeer` / `PeerControl`

**Symptom.** Per-call control (hold/resume/mute/DTMF/transfer/audio) lives entirely on
`SessionHandle`, but a reactive client only has the `CallId` returned by `invite().send()`.
The only public way to turn that into a `SessionHandle` is `peer.coordinator().session(id)`
(`src/api/unified.rs:4283`) — i.e. reaching into the "advanced" `UnifiedCoordinator`.
`PeerControl::accept()` returns a handle for inbound calls, but outbound calls have no
equivalent.

**Root cause.** `StreamPeer`/`PeerControl` expose `accept`, `reject`, `invite`, `register`,
`subscribe_events`, `coordinator` — but no `session()`. The reference client calls
`coordinator.session(&id)` ~15 times for ordinary per-call operations.

**Suggested fix.** Add `PeerControl::session(&CallId) -> SessionHandle` (and re-expose via
`StreamPeer`) delegating to `coordinator.session()`. This keeps clients on the "peer"
surface for the whole call lifecycle. **Effort: S.**

### 6. Two `SessionId` types; the public `CallId` has no obvious constructor

**Symptom.** `SessionId::from_string(s)` (the natural call, documented on the *other*
`SessionId`) does not compile for the public `CallId`. The working form is non-obvious:
```rust
let id = rvoip::sip::SessionId(s.to_string());   // tuple-struct field
```

**Root cause.** `CallId = SessionId` where `SessionId` is `state_table::types::
SessionId(pub String)` (`src/state_table/types.rs:7`) — a public tuple struct whose only
"constructor" is the field. Meanwhile `rvoip_core_traits::SessionId`
(`crates/foundation/rvoip-core-traits/src/ids.rs`) is a *different* type with
`new()`/`from_string()`/`as_str()`. Two same-named ID types with different APIs is a footgun;
round-tripping `String ↔ CallId` shouldn't require knowing which one the public alias points
at.

**Suggested fix.** Give the public `CallId`/`SessionId` clear, documented constructors
(`from_string`, `parse`, `as_str`) — ideally by unifying on the core-traits type — and add a
doc line on `CallId` showing the round-trip. **Effort: S–M.**

### 7. `AudioFrame` is not re-exported — forces a second crate dependency

**Symptom.** `SessionHandle::audio()` yields an `AudioStream` of
`rvoip_media_core::types::AudioFrame`, but that type isn't reachable through `rvoip-sip` or
the `rvoip` facade, so a client that feeds/reads audio must add a direct dependency:
```toml
rvoip-media-core = { path = "../rvoip/crates/media/media-core" }   # just for AudioFrame
```

**Root cause.** `src/lib.rs:514` re-exports `AudioReceiver`, `AudioSender`, `AudioStream` —
but not the `AudioFrame` they carry. You can `recv()` a frame without naming the type, but
you cannot *construct* one (mic → RTP) without it.

**Suggested fix.** Re-export `rvoip_media_core::types::AudioFrame` from `rvoip-sip` next to
the audio stream types (and from the `rvoip` facade). **Effort: S.**

### 8. No reusable audio-device helper — every client re-writes the cpal bridge

**Symptom.** Because the audio API is purely frame-based (the library intentionally no longer
owns OS devices), a real softphone must hand-write OS capture/playback. The reference client
wrote **~600 lines** (`src/audio/mod.rs`): cpal device selection, format conversion,
resampling, a jitter buffer, 20 ms send pacing, mute-as-silence, VU metering, and a
dedicated `!Send` thread for the cpal streams. The first cut shipped with both a pacing bug
(see #4) and resampling aliasing.

**Root cause.** `src/api/audio.rs` exposes only the raw duplex frame stream; the only
worked example is `examples/sip_client/audio.rs`, which (a) is copy-paste boilerplate and
(b) carries the pacing/jitter issues above.

**Suggested fix.** Ship an optional `rvoip-audio-device` helper (separate crate or a cargo
feature) that wraps cpal and offers something like
`DeviceBridge::attach_default(audio_stream)` — handling device selection, resampling
(band-limited), 20 ms pacing via `interval`, a small jitter buffer with click-free
underrun, mute, and the `!Send`-stream thread. ~90% of this already exists in the example;
making it a supported module would save every client author from reimplementing (and
mis-implementing) real-time audio. **Effort: M.**

### 9. No `register_and_wait` on the StreamPeer/coordinator path

**Symptom.** `register().send()` returns a handle immediately; success/failure arrives later
as `RegistrationSuccess`/`RegistrationFailed` events. It's easy to advance a UI to
"connected" on transport init before registration actually lands (the reference client did
exactly this and had to re-gate navigation on the event).

**Root cause.** `Endpoint` has `register_and_wait()` (`src/api/endpoint.rs:155`, `:337`), but
the `StreamPeer`/`PeerControl`/`UnifiedCoordinator` path has no await-able equivalent — the
reactive surface is the one most likely to want it.

**Suggested fix.** Add `PeerControl::register_and_wait(..) -> Result<RegistrationInfo>` (and
`StreamPeer`), mirroring `Endpoint`. **Effort: S.**

### 10. Re-binding the same port after teardown races the socket release

**Symptom.** Tearing a peer down and immediately rebuilding it on the same port (e.g. a
re-login after a failed REGISTER) fails:
```
Failed to bind UDP transport to 192.168.1.104:5070: Address already in use (os error 48)
```

**Root cause.** `UnifiedCoordinator::shutdown_gracefully()` (`src/api/unified.rs:3602`)
returns before the underlying UDP socket is guaranteed released, so a subsequent
`StreamPeer::with_config` on the same port can lose the race.

**Reference-client workaround.** A 5× bind-retry loop with 300 ms backoff around
`with_config`.

**Suggested fix.** Make `shutdown`/`shutdown_gracefully` await full transport teardown (so
the socket is closed before it returns), and/or set `SO_REUSEADDR` on the bind in
`sip-transport`. **Effort: S–M.**

### 11. `rvoip-client` is a misleading stub

**Symptom.** The crate advertised as the desktop/mobile/web **client SDK** can't actually
make a call: `Client::connect()` / `Client::call()` are stubs and most `SessionHandle`
methods `return NotImplemented` (`crates/rvoip-client/src/lib.rs`). A client author who
starts there (its stated purpose) hits a wall.

**Suggested fix.** Implement it, or clearly mark it experimental/not-ready in its README and
crate docs and point client authors at `rvoip-sip` (`StreamPeer`/`Endpoint`) until it lands.
**Effort: S (labeling) / L (implementation).**

### 12. `Endpoint`'s feature ceiling is hidden

**Symptom.** `Endpoint` is documented as the "most applications should start here" API, but
it cannot do **attended transfer** or **inbound-REFER** control — `EndpointCall::transfer`
is blind-only (`src/api/endpoint.rs:809`) and `EndpointEvent` has no REFER/transfer variants.
Those live only on `SessionHandle` (StreamPeer/coordinator path). A project that starts on
`Endpoint` for simplicity can hit this wall mid-way.

**Suggested fix.** Add attended transfer + `accept_refer`/`reject_refer` + REFER events to
`EndpointCall`/`EndpointEvent`, or state the ceiling explicitly in the API-selection table
("Endpoint: blind transfer only; use StreamPeer/SessionHandle for attended/REFER").
**Effort: S (docs) / M (parity).**

---

## P2 — Polish

### 13. Host-less request URI yields a cryptic DNS error

**Symptom.** Dialing a bare extension (`sip:600`, no host) produces a confusing DNS
NAPTR/SRV/A failure rather than a clear routing error:
```
Default resolver returned error for sip:600: No candidates after NAPTR/SRV/A ladder
Failed to send INVITE …: No remote target address available
```
(The correct address is `sip:600@registrar` — a client bug — but the diagnostic points at
DNS.)

**Suggested fix.** When a request URI has no host (and no outbound proxy/route is set), fail
early with a clear message ("request URI `sip:600` has no host; address it to the registrar,
e.g. `sip:600@pbx`") instead of attempting DNS resolution of the user part. **Effort: S.**

---

## Reference client

A complete, working example that exercises every item above (registration, in/out calls,
hold/resume, mute, DTMF, blind + attended transfer, inbound REFER, and a real cpal audio
bridge) lives at **`rvoip_sip_client`** (sibling repo). It is built on
`StreamPeer` + `PeerControl` + `SessionHandle` and is a useful end-to-end smoke test for
these changes.

**Bottom line:** items **1, 2, 7, and 8** account for the large majority of the integration
pain — fixing the auth-credential reuse, the Contact default, the `AudioFrame` re-export, and
shipping a device helper would roughly halve the time-to-first-call for the next client
author.
