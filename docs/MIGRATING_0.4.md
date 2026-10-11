# Migrating from rvoip 0.3.12 to 0.4.0

0.4.0 is a breaking minor release of all 46 workspace crates. Most
applications need only mechanical edits: build configuration through
constructors, stop touching the inside of `ConnectionId` / `ConversationId`,
add a method to TTS playback types, and handle one new SIP event. A few
defaults also changed on the wire, chiefly RTCP. This guide shows each change
with before/after code. It is written for downstream applications such as
Thelve and vapi-central, but applies to any crate that depends on rvoip.

The [changelog](../CHANGELOG.md) lists every change, and the
[release notes](../crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md) describe
the features and the interop evidence.

## Contents

1. [Update the dependency](#1-update-the-dependency)
2. [`ConnectionId` and `ConversationId`](#2-connectionid-and-conversationid)
3. [`#[non_exhaustive]` configuration structs](#3-non_exhaustive-configuration-structs)
4. [TTS: `audio_format` and `destination_codec`](#4-tts-audio_format-and-destination_codec)
5. [SIP events and low-level SIP APIs](#5-sip-events-and-low-level-sip-apis)
6. [Quality and statistics structs](#6-quality-and-statistics-structs)
7. [RTCP defaults](#7-rtcp-defaults)
8. [Session timers](#8-session-timers)
9. [`Config::freeswitch_internal`](#9-configfreeswitch_internal)
10. [`UctpWsAdapter` lifetime](#10-uctpwsadapter-lifetime)
11. [Tenant quotas](#11-tenant-quotas)
12. [G.711 utilities](#12-g711-utilities)
13. [Pre-release APIs that did not ship](#13-pre-release-apis-that-did-not-ship)
14. [Worth adopting](#14-worth-adopting)
15. [Upgrade checklist](#15-upgrade-checklist)

## 1. Update the dependency

All rvoip crates move together. Bump every rvoip requirement at once; mixing
0.3 and 0.4 crates gives two copies of the shared types.

```toml
# Before
rvoip = { version = "0.3.12", features = ["sip"] }
rvoip-sip = "0.3.12"

# After
rvoip = { version = "0.4.0", features = ["sip"] }
rvoip-sip = "0.4.0"
```

If you patch rvoip crates to local `vendor/` copies, refresh every copy from
the same 0.4.0 source rather than only the crates you changed.

## 2. `ConnectionId` and `ConversationId`

Both IDs now carry a private, one-use lifecycle fence that lets
`rvoip-core` bound retained history (see
[bounded lifecycle cleanup](BOUNDED_LIFECYCLE_CLEANUP.md)). They are no
longer tuple structs, so the tuple constructor and `.0` are gone. `new()`,
`from_string()`, `as_str()`, `Default`, `Display`, `Eq`, `Hash`, `Ord` and
the serde string form are unchanged. `SessionId`, `StreamId` and the other
IDs keep their 0.3 shape.

```rust
use rvoip_core::ids::{ConnectionId, ConversationId};

// Before
let id = ConnectionId(format!("conn_{}", call_ref));
let text: &str = &id.0;
let conversation = ConversationId(row.conversation_id.clone());

// After
let id = ConnectionId::from_string(format!("conn_{}", call_ref));
let text: &str = id.as_str();
let conversation = ConversationId::from_string(row.conversation_id.clone());
```

Things to check:

- **Do not parse ID text.** Newly minted IDs look like
  `conn_<incarnation>_<sequence>` rather than `conn_<uuid>`. Treat them as
  opaque strings.
- **Mint, don't reconstruct, when you create.** In the default compatibility
  mode a `from_string` ID behaves as before. If you opt into bounded
  retention (`Orchestrator::configure_bounded_connection_lifecycles`), only
  an ID minted with `ConnectionId::new()` / `ConversationId::new()` in this
  process may create a connection or conversation. String and serde copies
  still work for lookups and callbacks on lifecycles that already exist.
  A gateway that receives IDs from another process must mint a local ID at
  its trusted boundary and keep a bounded wire-to-local map.

## 3. `#[non_exhaustive]` configuration structs

These structs can no longer be built with a struct literal outside their
own crate, including the `..Default::default()` form:

| Struct | Start from |
| --- | --- |
| `rvoip_core::Config` | `Config::default()` |
| `rvoip_core::TenantQuotas` | `TenantQuotas::default()` plus `with_max_concurrent_*` |
| `rvoip_sip::Config` (also `PeerConfig`) | `Config::local`, `Config::on`, or a profile constructor |
| `rvoip_sip_dialog::api::DialogConfig` | `DialogConfig::new(addr)` or `Default` |
| `rvoip_websocket::UctpWsConfig` | `UctpWsConfig::new(listener, validator)` |
| `rvoip_quic::UctpQuicConfig` | `UctpQuicConfig::new(endpoint, accept_rx, validator)` |
| `rvoip_uctp::state::UctpCoordinatorCaps` | `UctpCoordinatorCaps::default()` |

Reading and assigning fields still works, so the fix is to construct first
and assign second.

```rust
// Before
let core = rvoip_core::Config {
    max_concurrent_setups: 64,
    ..rvoip_core::Config::default()
};
let quotas = TenantQuotas {
    max_concurrent_sessions: Some(10),
    max_concurrent_recordings: Some(2),
    max_concurrent_ai_sessions: None,
};
let sip = rvoip_sip::Config {
    session_timer_secs: Some(1800),
    ..rvoip_sip::Config::on("pbx", ip, 5060)
};

// After
let mut core = rvoip_core::Config::default();
core.max_concurrent_setups = 64;

let quotas = TenantQuotas::default()
    .with_max_concurrent_sessions(10)
    .with_max_concurrent_recordings(2);

let mut sip = rvoip_sip::Config::on("pbx", ip, 5060);
sip.session_timer_secs = Some(1800);
```

New fields arrive with defaults that keep 0.3.12 behaviour, except the
RTCP defaults in [section 7](#7-rtcp-defaults):
`rvoip_core::Config::capture_session_vcon` (`true`); on `rvoip_sip::Config`,
`rtcp_mux_required`, `offer_rtcp_mux`, `rtcp_non_mux`,
`rtcp_reduced_minimum_interval`, `rtcp_xr_voip_metrics`,
`active_call_rtcp_counts_as_media`, `media_quality_interval`,
`options_keepalive_targets`, `options_keepalive_interval_secs` and
`sip_allow_tls_contact_on_sips`; `UctpWsConfig::application_handler`; and
`UctpCoordinatorCaps::application_handler_timeout`.

## 4. TTS: `audio_format` and `destination_codec`

`play_audio` used to forward whatever a `TtsPlayback` yielded straight into
the connection's stream. It now encodes and paces the audio for the
destination, so it has to know what the frames contain.

### Implement `TtsPlayback::audio_format`

```rust
use rvoip_harness::{TtsAudioFormat, TtsPlayback};

// Before
#[async_trait]
impl TtsPlayback for MyPlayback {
    async fn next_frame(&self) -> Option<MediaFrame> { /* ... */ }
    async fn cancel(&self) -> Result<()> { /* ... */ }
}

// After: the provider yields 16-bit mono PCM
#[async_trait]
impl TtsPlayback for MyPlayback {
    fn audio_format(&self) -> TtsAudioFormat {
        TtsAudioFormat::PcmS16Le { sample_rate_hz: 24_000 }
    }
    async fn next_frame(&self) -> Option<MediaFrame> { /* ... */ }
    async fn cancel(&self) -> Result<()> { /* ... */ }
}
```

- `PcmS16Le { sample_rate_hz }`: mono signed 16-bit little-endian PCM at
  8, 16, 24, 32 or 48 kHz. Frames may hold any number of samples; rvoip
  re-frames them into 20 ms chunks, zero-pads the last one, and encodes them
  with the same encoder as `Orchestrator::play_pcm`.
- `Encoded { codec }`: the provider already speaks the destination codec.
  `codec` must match `TtsRequest::destination_codec` by name, clock rate and
  channel count, and every frame must hold exactly one 20 ms packet. rvoip
  re-stamps the stream id, payload type and RTP timestamps.

A provider that used to emit G.711 bytes because it knew the call was PCMU
should either return `Encoded { codec }` with the request's
`destination_codec`, or switch to PCM and let rvoip encode. A mismatch fails
`play_audio` and cancels the provider instead of sending unusable audio.

### Fill in `TtsRequest::destination_codec`

```rust
// Before
let request = TtsRequest {
    voice: Some(voice),
    text,
    sample_rate_hz: Some(8_000),
};

// After
let request = TtsRequest {
    voice: Some(voice),
    text,
    sample_rate_hz: Some(8_000),
    destination_codec: None, // offline synthesis: no destination stream
};
```

Providers can read `request.destination_codec` to synthesize directly in the
negotiated codec. `play_audio` sets it and leaves `sample_rate_hz` as `None`,
because an RTP clock rate is not a PCM rate (Opus is 48000 on the wire,
G.722 is 8000 on the wire but 16 kHz audio); pick your native rate.

## 5. SIP events and low-level SIP APIs

`rvoip_sip::Event` is still an exhaustive enum.

### `MediaQualityChanged` gains `quality`

```rust
// Before
Event::MediaQualityChanged { call_id, packet_loss_percent, jitter_ms, mos } => { /* ... */ }

// After: bind the new field, or add `..`
Event::MediaQualityChanged { call_id, quality, .. } => {
    record(call_id, quality.packet_loss_percent, quality.rtt_ms, quality.remote_packet_loss_percent);
}
```

The existing integer fields are still there. The event now actually fires:
set `Config::media_quality_interval` (it defaults to `None`) or poll
`UnifiedCoordinator::media_quality(&session_id)` /
`SessionHandle::media_quality()`.

### New `PeerReachabilityChanged`

```rust
// Before
match event {
    Event::IncomingCall { .. } => { /* ... */ }
    // ... every other variant ...
}

// After
match event {
    Event::IncomingCall { .. } => { /* ... */ }
    // ... every other variant ...
    Event::PeerReachabilityChanged { target, reachable, status_code } => {
        health.set(&target, reachable, status_code);
    }
}
```

It is published only when `Config::options_keepalive_targets` is non-empty
(the `tls_direct_routing` profile sets it). A wildcard arm is also fine.

### Low-level INVITE authentication retries

Only code that drives dialog-core or `rvoip_sip::internals` directly is
affected; `UnifiedDialogApi`, `UnifiedCoordinator` and the peers are not.

```rust
use rvoip_sip_dialog::api::unified::InviteAuthRetryOptions;

// Before
manager
    .send_invite_with_auth_options(&dialog_id, body, auth_headers, extra_headers,
        from_display, contact_uri, outbound_proxy_uri, supported_100rel)
    .await?;
adapter.resend_invite_with_auth(&session_id, opts, apply_global_proxy).await?;

// After
let mut opts = InviteAuthRetryOptions::default();
opts.sdp = sdp; // was `body: Option<Bytes>`
opts.authorization_headers = auth_headers;
opts.extra_headers = extra_headers;
opts.from_display = from_display;
opts.contact_uri = contact_uri;
opts.outbound_proxy_uri = outbound_proxy_uri;
opts.supported_100rel = supported_100rel;
manager.send_invite_with_auth_options(&dialog_id, opts).await?;

// `learned_min_se` is the Min-SE from a 422 on this INVITE, if any.
adapter.resend_invite_with_auth(&session_id, opts, apply_global_proxy, None).await?;
```

Prefer keeping the options of the first attempt and passing them back
unchanged: they carry the RFC 5626 registered-flow routes, which a retry to
a registered contact must reuse.

## 6. Quality and statistics structs

These structs gained public fields. Literals need the new fields or
`..Default::default()`:

| Struct | New fields |
| --- | --- |
| `rvoip_core::QualitySnapshot` | `rtt_ms`, `remote_packet_loss_pct`, `remote_jitter_ms` |
| `rvoip_infra_common::events::cross_crate::MediaQualityMetrics` | packet counters, `rtt_ms`, `remote_*` |
| `rvoip_media_core` `QualityMetrics` (`types::stats`) | packet counters, `remote_*` |
| `rvoip_rtp_core` `RtpSessionStats` (session) | `rtcp_packets_received`, `rtcp_packets_rejected`, `peer_report`, `peer_bye` |
| `rvoip_media_core` `MediaSessionInfo` | `rtcp_port: Option<u16>` |

```rust
// Before
let snapshot = QualitySnapshot { jitter_ms: 4.0, packet_loss_pct: 0.5, mos: Some(4.2) };
let info = MediaSessionInfo { dialog_id, status, config, rtp_port: Some(port),
    rtp_stats: None, stats_updated_at: None, created_at: Instant::now() };

// After
let snapshot = QualitySnapshot {
    jitter_ms: 4.0,
    packet_loss_pct: 0.5,
    mos: Some(4.2),
    ..Default::default()
};
let info = MediaSessionInfo { dialog_id, status, config, rtp_port: Some(port),
    rtcp_port: None, rtp_stats: None, stats_updated_at: None, created_at: Instant::now() };
```

media-core's `QualityMetrics` has no `Default`; build it with
`QualityMetrics::from_rtp_stats(&stats)`. If you adapt your own
`ConnectionAdapter` to report quality, fill the `remote_*` fields only from
real RTCP data and leave them `None` otherwise.

## 7. RTCP defaults

No code change is required, but calls behave differently on the wire:

| Behaviour | 0.3.12 | 0.4.0 | Restore or tune |
| --- | --- | --- | --- |
| `a=rtcp-mux` in offers | only with `rtcp_mux_required` | always | `config.offer_rtcp_mux = false` |
| RTCP when the answer declines mux | none on calls rvoip offered | none, including no BYE | `config.rtcp_non_mux = true` for a separate port |
| Report interval | every 1 s | RFC 3550: 5 s minimum, randomised | `config.rtcp_reduced_minimum_interval = true` |
| RFC 3611 VoIP-metrics XR | appended to every report | off | `config.rtcp_xr_voip_metrics = true` (needs peer `a=rtcp-xr`) |
| SDES CNAME | `$USER@hostname` | random per session | read `RtpSession::cname()` |
| Inbound RTCP | accepted from anyone | only from the call's peer | none needed |

```rust
// Asterisk with default pjsip settings, or a carrier without rtcp-mux:
let mut config = rvoip_sip::Config::carrier_trunk_udp("trunk", bind, public, trunk_uri);
config.rtcp_non_mux = true; // RTCP on RTP + 1; two ports per call

// A peer that rejects or mishandles a=rtcp-mux in offers:
config.offer_rtcp_mux = false; // offers match 0.3.x
```

`rtcp_non_mux` doubles the ports a call uses and is ignored with ICE,
DTLS-SRTP keying, `rtcp_mux_required`, and signalling-only media. If your
dead-media watchdogs should treat RTCP as activity for held or silent calls,
set `config.active_call_rtcp_counts_as_media = true`. rtp-core users that
relied on `RTCP_MIN_INTERVAL` being one second should note it is now five.

## 8. Session timers

`Config::validate`, and so peer construction, now enforces RFC 4028's floor:
`session_timer_secs` and `session_timer_min_se` must each be at least 90 s,
and `session_timer_secs` must not be below `session_timer_min_se`.

```rust
// Before: accepted in 0.3.12
config.session_timer_secs = Some(30);
config.session_timer_min_se = 10;

// After: production
config.session_timer_secs = Some(1800);
config.session_timer_min_se = 90;
```

Tests that need second-scale timers enable rvoip-sip's `test-hooks` feature
(in `[dev-dependencies]`) and set the testing knob:

```rust
#[cfg(test)]
{
    config.session_timer_allow_short_intervals_for_testing = true;
    config.session_timer_secs = Some(4);
    config.session_timer_min_se = 2;
}
```

`Config::carrier_sbc` and `Config::proxy_rtpengine` now turn timers on
(1800 s, Min-SE 90), as do the new `carrier_trunk_udp`, `public_server` and
`tls_direct_routing` profiles. To keep the 0.3.12 behaviour:

```rust
let mut config = rvoip_sip::Config::carrier_sbc(/* ... */);
config.session_timer_secs = None;
```

Session-timer behaviour itself is now conformant: a rejected refresh no
longer hangs up the call, refreshes go to the peer's Contact, and the
non-refresher sends its BYE slightly before the interval ends. Tests that
asserted the old teardown timing or Request-URI need updating.

## 9. `Config::freeswitch_internal`

It is deprecated (`since = "0.4.0"`). It only set `strict_codec_matching`,
which every constructor already enables.

```rust
// Before
let config = Config::freeswitch_internal("ivr", bind);

// After
let config = Config::lan_pbx("ivr", bind, advertised);
```

`lan_pbx` also sets `sip_advertised_addr` and `media_public_addr` from
`advertised`; pass `bind` again if the bind address is the one the PBX
reaches.

## 10. `UctpWsAdapter` lifetime

Dropping the last `Arc<UctpWsAdapter>` now cancels its listener and inbound
peers. In 0.3.12 the accept loop was detached and kept accepting connections
after every adapter handle, including the orchestrator's, was gone. A
registered adapter is held by its orchestrator, so this matters when the
orchestrator (or an unregistered adapter) is dropped while you still expect
the port to serve.

```rust
// Before: the listener outlived every handle to the adapter
let adapter = UctpWsAdapter::new(config).await?;
orchestrator.register(adapter.clone() as Arc<dyn ConnectionAdapter>)?;
drop(orchestrator); // the WebSocket port kept accepting peers

// After: keep the adapter (or its orchestrator) alive while it serves, and
// stop it explicitly
let adapter = UctpWsAdapter::new(config).await?;
orchestrator.register(adapter.clone() as Arc<dyn ConnectionAdapter>)?;
// ... serve ...
adapter.begin_drain(); // stop admitting new peers, keep established ones
let clean = adapter.shutdown(Duration::from_secs(10)).await;
```

`shutdown` returns `true` only once inbound cleanup has finished; on `false`
cleanup continues and you can call it again. Outbound `originate` clients
are not owned by the listener. `UctpWsServer` is no longer a constructible
unit struct.

## 11. Tenant quotas

`Orchestrator::set_tenant_quotas` now reconciles against the configured
total. In 0.3.12 reapplying a limit while permits were held could grow
capacity. Now:

- repeating the same limit is a no-op, and increases add capacity at once;
- shrinking or removing a limit returns an error while its permits are held,
  so drain the tenant's recordings or AI sessions first;
- a limit beyond Tokio's semaphore capacity is rejected.

If you resize quotas on a live system, handle the error and retry after the
affected work drains.

## 12. G.711 utilities

The `rvoip_codec_core::utils` table, batch and SIMD helpers, and
media-core's tone generator, now produce standard G.711. Their signatures
are unchanged. They used to disagree with the canonical codec for every
input, so audio from them was corrupted on the wire. No change is needed to
talk to standard peers. Regenerate any golden files, fixtures or stored
recordings that captured the old utility output. A workaround that called
`codecs::g711::ulaw_compress` / `alaw_compress` directly can stay or return
to the utilities; both now give the same bytes.

## 13. Pre-release APIs that did not ship

If you built against `main` between 0.3.12 and 0.4.0, these never reached a
release:

- `TtsProvider::synthesize_for_codec`: use `TtsRequest::destination_codec`
  in `synthesize` instead.
- `media_graph::encode_pcm_prompt`: use `TtsAudioFormat::PcmS16Le` or
  `Orchestrator::play_pcm`.
- `Orchestrator::retained_lifecycle_counts`: use
  `connection_id_budget_usage()` or `resource_snapshot()`.

## 14. Worth adopting

Not required, but these address problems downstream workers have hit:

- **Bounded lifecycle retention** for long-running workers that used to
  exhaust the 262,144-entry connection-ID budget:

  ```rust
  orchestrator.configure_bounded_connection_lifecycles(4_096)?; // before registering adapters
  // ... after each conversation is torn down and archived:
  orchestrator.release_closed_conversation(&conversation_id)?;
  ```

  In bounded mode you must release closed conversations, or the budget fills
  with closed history. Watch `connection_id_budget_usage()` and
  `resource_snapshot()`. Set `capture_session_vcon = false` if no vCon
  exporter is provisioned.
- **Deployment profiles** (`carrier_trunk_udp`, `public_server`,
  `behind_nat`, `tls_direct_routing`, `lan_pbx`) in place of hand-assembled
  `Config`s. See the "Media options" table in the
  [rvoip-sip README](../crates/sip/rvoip-sip/README.md).
- **Media quality**: `media_quality()` and `Config::media_quality_interval`;
  see [MEDIA_QUALITY.md](../crates/sip/rvoip-sip/docs/MEDIA_QUALITY.md).
- **Supervised workers**: `try_spawn_media_quality_sampler`,
  `try_spawn_idle_closer` and `try_spawn_capacity_scheduler` return errors
  instead of logging them, and `drain_periodic_tasks()` /
  `drain_playback_tasks()` give a clean shutdown.
- **UCTP application profiles** over WebSocket and QUIC, and the
  experimental `sdk/uctp-js` client.

## 15. Upgrade checklist

Search your code for the patterns that need attention:

```sh
rg 'ConnectionId\(|ConversationId\(|\.0\b' --type rust      # section 2
rg 'Config \{|TenantQuotas \{|DialogConfig \{|UctpWsConfig \{|UctpQuicConfig \{|UctpCoordinatorCaps \{' --type rust  # section 3
rg 'impl .*TtsPlayback for|TtsRequest \{' --type rust        # section 4
rg 'MediaQualityChanged \{|match .*event' --type rust        # section 5
rg 'send_invite_with_auth_options|resend_invite_with_auth' --type rust  # section 5
rg 'QualitySnapshot \{|MediaSessionInfo \{|RtpSessionStats \{' --type rust  # section 6
rg 'session_timer_secs|session_timer_min_se' --type rust     # section 8
rg 'freeswitch_internal|UctpWsAdapter::new' --type rust     # sections 9 and 10
```

Then build with both default features and `--all-features`, and run your
interop tests against your PBX or carrier: the RTCP and session-timer changes
are visible on the wire.
