# Per-call media quality

rvoip-sip reports media quality for every call that has an RTP session:
what this endpoint measures on the stream it receives, and, when RTCP flows,
what the peer reports about the stream this endpoint sends. A SIP server can
read it on demand with no configuration, or receive it periodically as an
event by setting one `Config` field.

## Quick start

### Read quality on demand

No configuration is needed. Use the coordinator, or a call's
`SessionHandle`:

```rust,no_run
# async fn example(
#     coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>,
#     call: rvoip_sip::SessionHandle,
# ) {
if let Some(q) = coordinator.media_quality(call.id()).await {
    println!(
        "rx: {} pkts, {:.1}% lost, {:.1} ms jitter, MOS {:?}",
        q.packets_received, q.packet_loss_percent, q.jitter_ms, q.mos
    );
    println!(
        "peer view of our stream: loss {:?}%, jitter {:?} ms, RTT {:?} ms",
        q.remote_packet_loss_percent, q.remote_jitter_ms, q.rtt_ms
    );
}

// The same, bound to this exact call:
let q = call.media_quality().await;
# let _ = q;
# }
```

`media_quality` returns `None` when the call has no active media session:
an unknown or ended call, or a coordinator in signaling-only media mode.

### Receive periodic events

Set `Config::media_quality_interval` (default `None`). Every interval, each
call that has sent or received RTP produces one `Event::MediaQualityChanged`:

```rust,no_run
use std::time::Duration;
use rvoip_sip::{Config, Event, StreamPeer};

# async fn example() -> rvoip_sip::Result<()> {
let mut config = Config::local("pbx", 5060)
    .with_media_quality_interval(Duration::from_secs(5));
// Negotiate RTP/RTCP multiplexing so the peer's RTCP reaches us; without it
// the peer-reported fields stay `None`.
config.rtcp_mux_required = true;

let peer = StreamPeer::with_config(config).await?;
let mut events = peer.coordinator().events().await?;
while let Some(event) = events.next().await {
    if let Event::MediaQualityChanged { call_id, quality, .. } = event {
        if quality.remote_packet_loss_percent.is_some_and(|loss| loss > 5.0) {
            println!("{call_id}: peer is losing {:?}% of our audio", quality.remote_packet_loss_percent);
        }
    }
}
# Ok(())
# }
```

`MediaQualityChanged` keeps its earlier `packet_loss_percent`, `jitter_ms`
(both truncated to integers) and `mos` fields; `quality` carries the full
sample, with the same type and values the getter returns.

## Fields

`MediaQualityStats` (`rvoip_sip::MediaQualityStats`):

| Field | Unit | Source | `None` / zero when |
|---|---|---|---|
| `packets_sent` | packets | local RTP sender | `0` before we send |
| `packets_received` | packets | local RTP receiver | `0` before the peer's RTP arrives |
| `packets_lost` | packets | sequence-number gaps in received RTP | `0` with no gaps |
| `packet_loss_percent` | percent, 0–100 | `lost / (received + lost)` on the received stream | `0.0` with no gaps |
| `jitter_ms` | milliseconds | RFC 3550 §6.4.1 interarrival jitter of the received stream | `0.0` before RTP arrives |
| `mos` | 1.0–5.0 | estimate from local loss, jitter and RTT | `None` until RTP is received |
| `rtt_ms` | milliseconds | RTCP LSR/DLSR in the peer's report block about us | `None` without RTCP, or before the peer has reflected one of our sender reports |
| `remote_packet_loss_percent` | percent, 0–100 | peer's RTCP *fraction lost* for our stream, over its last report interval | `None` without RTCP |
| `remote_packets_lost` | packets (signed) | peer's RTCP *cumulative lost* for our stream; negative if it saw duplicates | `None` without RTCP |
| `remote_jitter_ms` | milliseconds | peer's RTCP interarrival jitter for our stream, converted with the RTP clock rate | `None` without RTCP |

`MediaQualityStats::has_peer_report()` is true once any `remote_*` value is
present.

### When peer-reported values appear

- RTCP must reach this endpoint. rvoip-sip uses one media socket per call,
  so that requires negotiated RTP/RTCP multiplexing (`a=rtcp-mux`, RFC 5761).
  Set `Config::rtcp_mux_required = true` to require it; a peer that declines
  then fails negotiation instead of silently losing RTCP. On a call without
  mux, local values work normally and every `rtt_ms` / `remote_*` field
  stays `None`.
- The peer sends reports on the RFC 3550 interval, so allow a few seconds of
  media for the first one.
- `rtt_ms` additionally needs the peer to echo one of our sender reports,
  which happens only once we have sent RTP.
- Values describe the latest report. If the peer stops sending RTCP the
  last values are kept; with several remote sources the most recent report
  about our stream wins.

## Configuration

| Field | Default | Meaning |
|---|---|---|
| `Config::media_quality_interval` | `None` | Period of `Event::MediaQualityChanged`. `Some(Duration::ZERO)` is rejected by `Config::validate`. Builder: `Config::with_media_quality_interval`. |
| `Config::rtcp_mux_required` | `false` | Require `a=rtcp-mux`; needed for the peer-reported fields. |

Periodic events are opt-in because they add one event per active call per
interval to the shared application event stream, which also carries call
lifecycle events to every subscriber. The getter costs nothing until called.
Five seconds is a reasonable interval: the RTCP reports behind the
`remote_*` fields arrive on a multi-second cadence themselves.

## rvoip-core and the `rvoip` facade

With the interval set, `SipAdapter` records each sample on the call's SIP
media stream, so `MediaStream::has_quality_measurement()` becomes true and
`MediaStream::quality_snapshot()` returns the latest reading. The
orchestrator receives each sample as an adapter quality event and emits
`rvoip_core::Event::MediaQuality`, whose `QualitySnapshot` carries
`rtt_ms`, `remote_packet_loss_pct` and `remote_jitter_ms` as `Option`s.
`Orchestrator::spawn_media_quality_sampler` averages those readings per
connection on its own cadence.

`RvoipAppBuilder::media_quality_interval` enables both: the core heartbeat
and, at the same cadence, the SIP sampler.

## Lower layers

- `rvoip_rtp_core::RtpSessionStats::peer_report` holds the latest
  `PeerReceptionReport` (reporter SSRC, fraction and cumulative loss,
  extended highest sequence, jitter in timestamp units and ms, RTT, receive
  time) and `peer_bye` records an inbound RTCP BYE.
- `MediaSessionController::get_media_quality(&DialogId)` returns media-core
  `QualityMetrics` for one dialog, including `remote_report_age`, and
  `publish_media_quality_updates()` publishes one
  `MediaToSessionEvent::MediaQualityUpdate` per mapped call through the
  `MediaEventHub`.
