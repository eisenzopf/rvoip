//! `WebRtcAdapter` — `rvoip_core::ConnectionAdapter` for WebRTC interop.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use parking_lot::{Mutex as SyncMutex, RwLock as SyncRwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{
    Connection, ConnectionState, Direction, Transport, TransportHandle,
};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;
use tokio::sync::{mpsc, Notify};
use tracing::{debug, info, instrument, warn};
use webrtc::data_channel::DataChannel;

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};
use crate::media::{from_tracks, WebRtcMediaStream};
use crate::peer::{PeerRole, RvoipPeerConnection};
use crate::sdp::{negotiate_audio, parse_sdp, sdp_to_string};

pub const ADAPTER_EVENT_CAP: usize = 256;

/// Background reaper poll interval.
const REAPER_TICK: Duration = Duration::from_secs(30);

/// Snapshot of operational metrics exposed by [`WebRtcAdapter::metrics`].
#[derive(Clone, Debug, Default)]
pub struct WebRtcMetrics {
    pub inbound_total: u64,
    pub outbound_total: u64,
    pub active_sessions: usize,
    pub signaling_errors_total: u64,
    pub sessions_rejected_over_cap: u64,
    pub reaped_total: u64,
}

/// Typed `TransportHandle` carrying the originating connection id and a weak
/// pointer to the adapter route table so orchestrators can introspect / clean
/// up without holding a strong reference.
pub struct WebRtcTransportHandle {
    pub connection_id: ConnectionId,
    routes: std::sync::Weak<DashMap<ConnectionId, Route>>,
    cancel: Arc<Notify>,
}

impl WebRtcTransportHandle {
    pub fn cancel(&self) {
        self.cancel.notify_waiters();
    }

    pub fn route_exists(&self) -> bool {
        self.routes
            .upgrade()
            .map(|r| r.contains_key(&self.connection_id))
            .unwrap_or(false)
    }
}

#[derive(Clone)]
pub struct Route {
    pub peer: Arc<RvoipPeerConnection>,
    pub streams: Arc<DashMap<StreamId, Arc<WebRtcMediaStream>>>,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub data_channel: Arc<DashMap<(), Arc<dyn DataChannel>>>,
    pub negotiated: NegotiatedCodecs,
    pub held: bool,
    /// Notify all per-route background tasks (track attacher, fail watcher, stats) to exit.
    pub cancel: Arc<Notify>,
    /// Set by the fail watcher when the underlying PC enters `Failed`/`Closed`.
    pub failed_at: Arc<SyncMutex<Option<Instant>>>,
}

/// D2 — per-route DTLS fingerprint pinning policy.
///
/// Implementations return the set of fingerprints allowed for a given
/// session. The adapter takes the **union** of this list with
/// [`WebRtcConfig::pinned_fingerprints`](crate::config::WebRtcConfig::pinned_fingerprints)
/// and, if the union is non-empty, rejects any peer whose negotiated
/// fingerprint isn't in the union with
/// [`WebRtcError::FingerprintNotPinned`](crate::errors::WebRtcError::FingerprintNotPinned).
///
/// `session_hint` is a free-form identifier the caller can use to scope
/// pinning per tenant / per call (e.g. a WHIP `session_id` or a UCTP
/// request id). Pass `None` when no hint is available.
#[async_trait]
pub trait FingerprintPolicyHook: Send + Sync {
    async fn allowed_fingerprints(
        &self,
        conn: &ConnectionId,
        session_hint: Option<&str>,
    ) -> Vec<crate::identity::DtlsFingerprint>;
}

pub struct WebRtcAdapter {
    config: WebRtcConfig,
    routes: Arc<DashMap<ConnectionId, Route>>,
    events_tx: mpsc::Sender<AdapterEvent>,
    events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
    /// Cancel for the global reaper task spawned in [`WebRtcAdapter::new`].
    reaper_cancel: Arc<Notify>,
    metrics_inbound: Arc<AtomicU64>,
    metrics_outbound: Arc<AtomicU64>,
    metrics_errors: Arc<AtomicU64>,
    metrics_rejected: Arc<AtomicU64>,
    metrics_reaped: Arc<AtomicU64>,
    /// Live session count incremented before any per-session work and
    /// decremented on route removal. Replaces `routes.len()` for cap checks
    /// so concurrent originate/apply_remote_offer can't race past the cap.
    live_sessions: Arc<std::sync::atomic::AtomicUsize>,
    /// D2 — optional per-route fingerprint pinning hook. Set via
    /// [`WebRtcAdapter::set_fingerprint_policy`]; `None` means "use only
    /// the static `WebRtcConfig::pinned_fingerprints` list".
    fingerprint_policy: SyncRwLock<Option<Arc<dyn FingerprintPolicyHook>>>,
}

impl WebRtcAdapter {
    pub fn new(config: WebRtcConfig) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(ADAPTER_EVENT_CAP);
        let reaper_cancel = Arc::new(Notify::new());
        let metrics_reaped = Arc::new(AtomicU64::new(0));
        let adapter = Arc::new(Self {
            config,
            routes: Arc::new(DashMap::new()),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
            reaper_cancel: Arc::clone(&reaper_cancel),
            metrics_inbound: Arc::new(AtomicU64::new(0)),
            metrics_outbound: Arc::new(AtomicU64::new(0)),
            metrics_errors: Arc::new(AtomicU64::new(0)),
            metrics_rejected: Arc::new(AtomicU64::new(0)),
            metrics_reaped: Arc::clone(&metrics_reaped),
            live_sessions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            fingerprint_policy: SyncRwLock::new(None),
        });

        // Spawn session reaper (idempotent: TTL=0 disables in-loop work).
        let ttl_secs = adapter.config.session_idle_ttl_secs;
        if ttl_secs > 0 {
            let routes = Arc::clone(&adapter.routes);
            let events_tx = adapter.events_tx.clone();
            let live = Arc::clone(&adapter.live_sessions);
            tokio::spawn(async move {
                Self::run_reaper(routes, events_tx, reaper_cancel, ttl_secs, metrics_reaped, live)
                    .await;
            });
        }

        // P12.8 — periodic per-Connection quality emitter. Walks the
        // routes table every 5 seconds and emits one
        // `AdapterEvent::Quality` per connection from the aggregated
        // per-stream snapshots already collected by
        // `crate::media::stats::spawn_webrtc_stats_collector`. The
        // orchestrator feeds these into its `QualityAggregator` so
        // `Event::SessionEnded` reports include WebRTC-side numbers.
        Self::spawn_quality_emitter(
            Arc::clone(&adapter.routes),
            adapter.events_tx.clone(),
        );

        adapter
    }

    fn spawn_quality_emitter(
        routes: Arc<DashMap<ConnectionId, Route>>,
        events_tx: mpsc::Sender<AdapterEvent>,
    ) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                for entry in routes.iter() {
                    let conn_id = entry.key().clone();
                    // Per-Connection aggregate: average jitter / loss
                    // across this connection's streams. MOS is dropped
                    // for now — the orchestrator's QualityAggregator
                    // only consumes jitter and loss fields. Skip
                    // connections with no streams to avoid emitting
                    // bogus zero snapshots.
                    let streams = &entry.value().streams;
                    if streams.is_empty() {
                        continue;
                    }
                    let mut count = 0u32;
                    let mut jitter_sum = 0.0f32;
                    let mut loss_sum = 0.0f32;
                    for stream in streams.iter() {
                        let snap = stream.value().webrtc_stats_snapshot();
                        jitter_sum += snap.jitter_ms;
                        loss_sum += snap.packet_loss_pct;
                        count += 1;
                    }
                    if count == 0 {
                        continue;
                    }
                    let snapshot = rvoip_core::stream::QualitySnapshot {
                        jitter_ms: jitter_sum / count as f32,
                        packet_loss_pct: loss_sum / count as f32,
                        mos: None,
                    };
                    let _ = events_tx
                        .send(AdapterEvent::Quality {
                            connection_id: conn_id,
                            snapshot,
                        })
                        .await;
                }
            }
        });
    }

    /// Snapshot of operational counters and live session count.
    pub fn metrics(&self) -> WebRtcMetrics {
        WebRtcMetrics {
            inbound_total: self.metrics_inbound.load(Ordering::Relaxed),
            outbound_total: self.metrics_outbound.load(Ordering::Relaxed),
            active_sessions: self.routes.len(),
            signaling_errors_total: self.metrics_errors.load(Ordering::Relaxed),
            sessions_rejected_over_cap: self.metrics_rejected.load(Ordering::Relaxed),
            reaped_total: self.metrics_reaped.load(Ordering::Relaxed),
        }
    }

    /// G12 — reset every counter to zero. Useful for operators that rotate
    /// Prometheus scrape windows or for hand-rolled rate-of-change graphs.
    /// Does **not** touch the live session count or running routes.
    pub fn reset_metrics(&self) {
        self.metrics_inbound.store(0, Ordering::Relaxed);
        self.metrics_outbound.store(0, Ordering::Relaxed);
        self.metrics_errors.store(0, Ordering::Relaxed);
        self.metrics_rejected.store(0, Ordering::Relaxed);
        self.metrics_reaped.store(0, Ordering::Relaxed);
    }

    /// Public accessor for the configured concurrent-session cap.
    pub fn max_concurrent_sessions(&self) -> usize {
        self.config.max_concurrent_sessions
    }

    /// Per-IP WHIP rate limit (POSTs/min). `0` = disabled.
    pub fn whip_rate_limit_cap_per_min(&self) -> u32 {
        self.config.whip_per_ip_per_min
    }

    /// CORS allow-list. Empty = no CORS layer.
    pub fn cors_origins(&self) -> &[String] {
        &self.config.cors_origins
    }

    /// ICE server URLs flattened from the config (for `Link: rel=ice-server`).
    pub fn ice_server_urls(&self) -> Vec<String> {
        self.config
            .ice_servers
            .iter()
            .flat_map(|s| s.urls.iter().cloned())
            .collect()
    }

    /// Configured ICE servers (with optional TURN credentials). Used by the
    /// WHIP handler to emit `Link: <url>; rel="ice-server"; username="…";
    /// credential="…"` headers per RFC 9725 §4.6.
    pub fn ice_servers(&self) -> &[crate::config::IceServerConfig] {
        &self.config.ice_servers
    }

    /// WebSocket max message size in bytes.
    pub fn ws_max_message_size(&self) -> usize {
        self.config.ws_max_message_size
    }

    /// WebSocket server-driven ping interval. `0` = disabled.
    pub fn ws_keepalive_secs(&self) -> u64 {
        self.config.ws_keepalive_secs
    }

    /// Whether the adapter was built in trickle-ICE mode.
    pub fn trickle_ice_enabled(&self) -> bool {
        self.config.trickle_ice
    }

    /// Policy applied to inbound mDNS (`.local`) trickle candidates.
    pub fn mdns_candidate_policy(&self) -> crate::config::MdnsCandidatePolicy {
        self.config.mdns_candidate_policy
    }

    /// Remote DTLS-SRTP fingerprints (one per `a=fingerprint:` line) from the
    /// stored remote SDP. Returns `Err(ConnectionNotFound)` if there is no
    /// such route, or `Ok(vec![])` if the route has no remote SDP yet (e.g.
    /// outbound originate before the answer arrives).
    ///
    /// D2 — [`ConnectionAdapter::verify_request_signature`] now surfaces
    /// the first canonical fingerprint here as
    /// [`IdentityAssurance::DtlsFingerprint`].
    pub fn remote_dtls_fingerprint(
        &self,
        conn: &ConnectionId,
    ) -> Result<Vec<crate::identity::DtlsFingerprint>> {
        let route = self.route(conn)?;
        Ok(route
            .remote_sdp
            .as_deref()
            .map(crate::identity::extract_fingerprints)
            .unwrap_or_default())
    }

    /// D2 — register a per-route fingerprint pinning hook. The hook's
    /// returned list is unioned with [`WebRtcConfig::pinned_fingerprints`];
    /// when the union is non-empty, peers whose `a=fingerprint:` doesn't
    /// match are rejected with
    /// [`WebRtcError::FingerprintNotPinned`](crate::errors::WebRtcError::FingerprintNotPinned).
    pub fn set_fingerprint_policy(&self, hook: Arc<dyn FingerprintPolicyHook>) {
        *self.fingerprint_policy.write() = Some(hook);
    }

    /// D2 — clear any previously-registered policy hook. Static
    /// `WebRtcConfig::pinned_fingerprints` still applies.
    pub fn clear_fingerprint_policy(&self) {
        *self.fingerprint_policy.write() = None;
    }

    /// D2 — evaluate the union of static + dynamic pin lists against the
    /// remote SDP's fingerprints. `Ok(())` when allowed (including when no
    /// pinning is in effect); `Err(FingerprintNotPinned)` when the remote
    /// has at least one fingerprint and none match.
    async fn enforce_fingerprint_policy(
        &self,
        conn: &ConnectionId,
        remote_sdp: &str,
        session_hint: Option<&str>,
    ) -> Result<()> {
        let mut allowed: Vec<crate::identity::DtlsFingerprint> =
            self.config.pinned_fingerprints.clone();
        // Drop the read guard before awaiting — parking_lot guards are not Send.
        let hook = self.fingerprint_policy.read().clone();
        if let Some(hook) = hook {
            allowed.extend(hook.allowed_fingerprints(conn, session_hint).await);
        }
        if allowed.is_empty() {
            return Ok(());
        }
        let remote = crate::identity::extract_fingerprints(remote_sdp);
        if remote.is_empty() {
            return Err(WebRtcError::FingerprintNotPinned);
        }
        let normalize =
            |fp: &crate::identity::DtlsFingerprint| (fp.algorithm.to_ascii_lowercase(), fp.value.to_ascii_lowercase());
        let allowed_norm: std::collections::HashSet<_> = allowed.iter().map(normalize).collect();
        if !remote.iter().any(|r| allowed_norm.contains(&normalize(r))) {
            return Err(WebRtcError::FingerprintNotPinned);
        }
        Ok(())
    }

    /// Atomically reserve a session slot. Returns a guard that releases the
    /// slot on Drop unless `commit()` is called first. Race-free under
    /// concurrent originate / apply_remote_offer.
    fn reserve_session_slot(&self) -> Result<SessionSlotGuard> {
        let cap = self.config.max_concurrent_sessions;
        let live = Arc::clone(&self.live_sessions);
        loop {
            let current = live.load(Ordering::Acquire);
            if cap > 0 && current >= cap {
                self.metrics_rejected.fetch_add(1, Ordering::Relaxed);
                return Err(WebRtcError::Adapter(format!(
                    "concurrent session cap reached ({cap})"
                )));
            }
            if live
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(SessionSlotGuard {
                    live: Some(live),
                });
            }
        }
    }

    /// Increment the signaling-errors counter; called by the WHIP/WS handlers
    /// when something rejectable happens.
    pub fn note_signaling_error(&self) {
        self.metrics_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the live-session counter (called when a route is removed).
    fn release_session_slot(&self) {
        // saturating sub so a double-release can't underflow.
        let mut cur = self.live_sessions.load(Ordering::Acquire);
        while cur > 0 {
            match self.live_sessions.compare_exchange(
                cur,
                cur - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(actual) => cur = actual,
            }
        }
    }

    pub fn routes(&self) -> &Arc<DashMap<ConnectionId, Route>> {
        &self.routes
    }

    /// G4 — aggregate [`WebRtcStatsSnapshot`] fields across every live media
    /// stream on every route. Used by the Prometheus exporter and by
    /// dashboards that want a single rollup number per peer-connection.
    ///
    /// Returns a `(total_streams, aggregated_snapshot)` tuple. The snapshot's
    /// `selected_pair` field is taken from the first stream that has one.
    pub fn aggregated_stats(&self) -> (usize, crate::media::WebRtcStatsSnapshot) {
        use crate::media::pump::{CandidatePairStats, OutboundStats};
        let mut total = 0usize;
        let mut agg = crate::media::WebRtcStatsSnapshot::default();
        let mut sample_pair: Option<CandidatePairStats> = None;
        let mut jitter_sum: f32 = 0.0;
        let mut loss_sum: f32 = 0.0;
        let mut mos_sum: f32 = 0.0;
        for entry in self.routes.iter() {
            for stream in entry.value().streams.iter() {
                let snap = stream.value().webrtc_stats_snapshot();
                total += 1;
                agg.packets_received = agg.packets_received.saturating_add(snap.packets_received);
                agg.bytes_received = agg.bytes_received.saturating_add(snap.bytes_received);
                agg.packets_lost = agg.packets_lost.saturating_add(snap.packets_lost);
                agg.frames_dropped = agg.frames_dropped.saturating_add(snap.frames_dropped);
                jitter_sum += snap.jitter_ms;
                loss_sum += snap.packet_loss_pct;
                mos_sum += snap.mos;
                agg.outbound = OutboundStats {
                    packets_sent: agg
                        .outbound
                        .packets_sent
                        .saturating_add(snap.outbound.packets_sent),
                    bytes_sent: agg
                        .outbound
                        .bytes_sent
                        .saturating_add(snap.outbound.bytes_sent),
                    retransmitted_packets: agg
                        .outbound
                        .retransmitted_packets
                        .saturating_add(snap.outbound.retransmitted_packets),
                    retransmitted_bytes: agg
                        .outbound
                        .retransmitted_bytes
                        .saturating_add(snap.outbound.retransmitted_bytes),
                    nack_count: agg
                        .outbound
                        .nack_count
                        .saturating_add(snap.outbound.nack_count),
                    pli_count: agg
                        .outbound
                        .pli_count
                        .saturating_add(snap.outbound.pli_count),
                    fir_count: agg
                        .outbound
                        .fir_count
                        .saturating_add(snap.outbound.fir_count),
                };
                if sample_pair.is_none() {
                    sample_pair = snap.selected_pair;
                }
            }
        }
        if total > 0 {
            agg.jitter_ms = jitter_sum / total as f32;
            agg.packet_loss_pct = loss_sum / total as f32;
            agg.mos = mos_sum / total as f32;
        }
        agg.selected_pair = sample_pair;
        (total, agg)
    }

    /// Single-take event receiver. Returns `Err(AlreadySubscribed)` on second call
    /// instead of panicking. The trait method [`ConnectionAdapter::subscribe_events`]
    /// preserves the original infallible signature by returning a closed receiver
    /// after the first take.
    pub fn try_subscribe_events(&self) -> Result<mpsc::Receiver<AdapterEvent>> {
        match self.events_rx.lock() {
            Ok(mut guard) => guard.take().ok_or(WebRtcError::AlreadySubscribed),
            Err(poisoned) => {
                // Recover from a poisoned mutex (a panic occurred while holding it).
                let mut guard = poisoned.into_inner();
                guard.take().ok_or(WebRtcError::AlreadySubscribed)
            }
        }
    }

    fn try_send(&self, event: AdapterEvent) {
        if self.events_tx.try_send(event).is_err() {
            warn!("WebRtcAdapter event channel full or closed");
        }
    }

    fn build_connection(
        &self,
        conn_id: ConnectionId,
        direction: Direction,
        negotiated: NegotiatedCodecs,
        handle: Arc<WebRtcTransportHandle>,
    ) -> Connection {
        Connection {
            id: conn_id,
            session_id: rvoip_core::ids::SessionId::new(),
            participant_id: rvoip_core::ids::ParticipantId::new(),
            transport: Transport::WebRtc,
            direction,
            state: ConnectionState::Connecting,
            capabilities: self.config.capabilities.clone(),
            negotiated_codecs: negotiated,
            streams: vec![],
            messaging_enabled: true,
            transport_handle: TransportHandle(handle),
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    fn make_transport_handle(
        &self,
        conn_id: ConnectionId,
        cancel: Arc<Notify>,
    ) -> Arc<WebRtcTransportHandle> {
        Arc::new(WebRtcTransportHandle {
            connection_id: conn_id,
            routes: Arc::downgrade(&self.routes),
            cancel,
        })
    }

    /// Create the audio media stream for this route. Mirrors the original
    /// (pre-H1) behavior: wait up to 500ms for the remote track via
    /// `wait_remote_track`, then build the stream with the remote inline (if
    /// arrived) or as send-only (if not — late tracks attach via the
    /// track-attacher spawned in `insert_route`).
    async fn seed_media_stream(&self, route: &Route) -> Result<()> {
        if !route.streams.is_empty() {
            return Ok(());
        }

        let codec = route.negotiated.audio.clone().unwrap_or_else(|| CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48000,
            channels: 2,
            fmtp: None,
        });

        let local = route
            .peer
            .local_audio_track()
            .ok_or_else(|| WebRtcError::Adapter("no local audio track".into()))?;
        let local_ssrc = route
            .peer
            .local_audio_ssrc()
            .ok_or_else(|| WebRtcError::Adapter("no local audio SSRC".into()))?;
        let payload_type = payload_type_for_audio_codec(&codec);

        let remote = route
            .peer
            .wait_remote_track(Duration::from_millis(500))
            .await
            .or(route.peer.try_recv_remote_track().await);

        let stream_id = StreamId::new();
        let has_remote = remote.is_some();
        let media = from_tracks(stream_id.clone(), codec, local, local_ssrc, payload_type, remote);
        if has_remote {
            media.enable_webrtc_stats(
                Arc::clone(route.peer.peer_connection()),
                Arc::clone(&route.cancel),
            );
        }
        route.streams.insert(stream_id, media);
        Ok(())
    }

    fn route(&self, conn: &ConnectionId) -> Result<Route> {
        self.routes
            .get(conn)
            .map(|e| e.value().clone())
            .ok_or(WebRtcError::ConnectionNotFound)
    }

    /// D2 — update the stored remote SDP for an existing route (e.g. after
    /// `apply_remote_answer` lands the offerer's answer). No-op when the
    /// route has already been reaped.
    fn update_remote_sdp(&self, conn: &ConnectionId, sdp: &str) {
        if let Some(mut entry) = self.routes.get_mut(conn) {
            entry.remote_sdp = Some(sdp.to_owned());
        }
    }

    fn insert_route(&self, conn_id: ConnectionId, route: Route) {
        // Track-attacher: wire the answerer's inbound RTP into each
        // `WebRtcMediaStream`'s frames_in pump once a remote track is
        // observed.
        //
        // The attacher *used* to only consume the `remote_track_rx`
        // channel (`try_recv_remote_track`) and `break` after the first
        // hit. That race-loses against any other caller that also reads
        // the channel — notably the test helper
        // `RvoipPeerConnection::prime_remote_track`, which calls
        // `wait_remote_track` (also consumes the channel). When the test
        // won the race the attacher looped forever and the inbound pump
        // was never spawned, so the QUIC bridge test
        // (`webrtc_quic_bridge_e2e::whip_webrtc_bridged_to_real_quic_leg`)
        // would time out at `client_in.recv()`.
        //
        // Fix: fall back to `discover_remote_track` (transceiver scan,
        // non-consuming) when the channel poll returns None. The
        // attacher also keeps looping after the first attach so a second
        // m-line (e.g. D1's DTMF or a future video track) gets its own
        // pump on a later iteration; `attach_remote`'s `compare_exchange`
        // guard makes the call idempotent per stream.
        let routes_track = Arc::clone(&self.routes);
        let conn_track = conn_id.clone();
        let peer_track = route.peer.clone();
        tokio::spawn(async move {
            use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
            use rvoip_core::stream::StreamKind;
            loop {
                if !routes_track.contains_key(&conn_track) {
                    break;
                }
                // 1) Fast path: drain anything sitting in the handler
                //    channel from `on_track` firings.
                while let Some(track) = peer_track.try_recv_remote_track().await {
                    attach_track_to_streams(&routes_track, &conn_track, &track).await;
                }
                // 2) Fallback: even if another consumer drained the
                //    channel, the underlying transceiver still exposes
                //    the receiver's track. Scan and attach. Idempotent
                //    via `WebRtcMediaStream::attach_remote`.
                if let Some(audio) =
                    peer_track.discover_remote_track(RtpCodecKind::Audio).await
                {
                    attach_track_to_streams_matching(
                        &routes_track,
                        &conn_track,
                        &audio,
                        StreamKind::Audio,
                    )
                    .await;
                }
                if let Some(video) =
                    peer_track.discover_remote_track(RtpCodecKind::Video).await
                {
                    attach_track_to_streams_matching(
                        &routes_track,
                        &conn_track,
                        &video,
                        StreamKind::Video,
                    )
                    .await;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });

        let routes_fail = Arc::clone(&self.routes);
        let events_fail = self.events_tx.clone();
        let conn_fail = conn_id.clone();
        let peer_fail = route.peer.clone();
        tokio::spawn(async move {
            peer_fail.wait_failed().await;
            if routes_fail.remove(&conn_fail).is_some() {
                let _ = events_fail
                    .send(AdapterEvent::Failed {
                        connection_id: conn_fail,
                        detail: "peer connection failed".into(),
                    })
                    .await;
            }
        });
        self.routes.insert(conn_id, route);
    }

    // (H1 had two helper functions `spawn_track_attacher` and `spawn_fail_watcher`
    // factored out; reverted in the H4-followup bisect because the inline
    // original better matches webrtc-rs 0.20-alpha's timing expectations.
    // See `insert_route` above.)

    /// Background reaper: every `REAPER_TICK`, walk routes and remove peers that
    /// have been in `Failed` state for at least `ttl_secs` (gives users a window
    /// to introspect via `routes()` before GC).
    async fn run_reaper(
        routes: Arc<DashMap<ConnectionId, Route>>,
        events_tx: mpsc::Sender<AdapterEvent>,
        cancel: Arc<Notify>,
        ttl_secs: u64,
        reaped_counter: Arc<AtomicU64>,
        live_sessions: Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let ttl = Duration::from_secs(ttl_secs);
        loop {
            tokio::select! {
                _ = cancel.notified() => break,
                _ = tokio::time::sleep(REAPER_TICK) => {}
            }

            let mut victims: Vec<ConnectionId> = Vec::new();
            for entry in routes.iter() {
                let failed = *entry.value().failed_at.lock();
                if let Some(at) = failed {
                    if at.elapsed() >= ttl {
                        victims.push(entry.key().clone());
                    }
                }
            }
            for id in victims {
                if let Some((_, route)) = routes.remove(&id) {
                    route.cancel.notify_waiters();
                    let _ = route.peer.close().await;
                    // Mirror release_session_slot inline (we don't have &self here).
                    let mut cur = live_sessions.load(Ordering::Acquire);
                    while cur > 0 {
                        match live_sessions.compare_exchange(
                            cur,
                            cur - 1,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => break,
                            Err(actual) => cur = actual,
                        }
                    }
                    let _ = events_tx
                        .send(AdapterEvent::Ended {
                            connection_id: id,
                            reason: EndReason::Normal,
                        })
                        .await;
                    reaped_counter.fetch_add(1, Ordering::Relaxed);
                    debug!("session reaper removed idle/failed route");
                }
            }
        }
    }

    /// Apply a remote SDP answer to an outbound (offerer) connection.
    pub async fn apply_remote_answer(
        &self,
        conn: ConnectionId,
        answer_sdp: &str,
    ) -> Result<()> {
        // D2 — enforce pinned fingerprints against the answer's `a=fingerprint:`
        // lines before handing it to webrtc-rs. Rejecting here avoids
        // completing the DTLS handshake with an un-pinned peer.
        self.enforce_fingerprint_policy(&conn, answer_sdp, None)
            .await?;
        let route = self.route(&conn)?;
        route.peer.set_remote_answer(answer_sdp).await?;
        // Update the stored remote SDP so subsequent verify_request_signature
        // / remote_dtls_fingerprint calls see the answer's fingerprint.
        self.update_remote_sdp(&conn, answer_sdp);
        Ok(())
    }

    /// Handle an inbound SDP offer — creates answerer PC and emits `InboundConnection`.
    #[instrument(skip(self, offer_sdp), fields(sdp_bytes = offer_sdp.len()))]
    pub async fn apply_remote_offer(&self, offer_sdp: &str) -> Result<ConnectionId> {
        let slot = self.reserve_session_slot()?;
        self.metrics_inbound.fetch_add(1, Ordering::Relaxed);
        let conn_id = ConnectionId::new();
        // D2 — enforce pinned fingerprints against the offer's
        // `a=fingerprint:` lines BEFORE allocating a peer connection, so
        // an un-pinned peer never triggers DTLS negotiation costs.
        self.enforce_fingerprint_policy(&conn_id, offer_sdp, None)
            .await?;
        let peer = RvoipPeerConnection::new(&self.config, PeerRole::Answerer).await?;
        let answer_sdp = peer.accept_offer_and_gather(offer_sdp).await?;

        let negotiated = negotiate_audio(&self.config.capabilities, &self.config.capabilities)?;

        let cancel = Arc::new(Notify::new());
        let route = Route {
            peer: Arc::clone(&peer),
            streams: Arc::new(DashMap::new()),
            local_sdp: Some(answer_sdp),
            remote_sdp: Some(offer_sdp.to_owned()),
            data_channel: Arc::new(DashMap::new()),
            negotiated: negotiated.clone(),
            held: false,
            cancel: Arc::clone(&cancel),
            failed_at: Arc::new(SyncMutex::new(None)),
        };

        // Don't seed media stream here — the track-attacher (spawned in
        // insert_route) buffers any early on_track event and `accept()` /
        // `streams()` will create the stream lazily. Eager seeding before
        // `accept()` was attempted but interacted badly with webrtc-rs
        // 0.20-alpha's negotiation timing.

        self.insert_route(conn_id.clone(), route);
        slot.commit();

        let handle = self.make_transport_handle(conn_id.clone(), cancel);
        let connection =
            self.build_connection(conn_id.clone(), Direction::Inbound, negotiated, handle);
        self.try_send(AdapterEvent::InboundConnection { connection });

        Ok(conn_id)
    }

    pub fn local_sdp(&self, conn: &ConnectionId) -> Result<String> {
        self.route(conn)?
            .local_sdp
            .clone()
            .ok_or_else(|| WebRtcError::Sdp("no local SDP".into()))
    }

    /// Ensure the route has a media stream — idempotent. Called from
    /// `accept()` (after wait_connected) and `streams()`.
    async fn ensure_media_streams(&self, conn: &ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        if route.streams.is_empty() {
            self.seed_media_stream(&route)
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }
        Ok(())
    }

    /// Apply a remote SDP answer to a WHEP/outbound offerer connection and bring it up.
    pub async fn accept_remote_answer(&self, conn: ConnectionId, answer_sdp: &str) -> Result<()> {
        self.apply_remote_answer(conn.clone(), answer_sdp).await?;
        ConnectionAdapter::accept(self, conn)
            .await
            .map_err(|e| WebRtcError::Adapter(format!("{e}")))?;
        Ok(())
    }

    /// WHIP ICE restart: apply a new offer on an inbound (answerer) connection.
    pub async fn apply_ice_restart_offer(
        &self,
        conn: ConnectionId,
        offer_sdp: &str,
    ) -> Result<String> {
        let route = self.route(&conn)?;
        if route.peer.role() != PeerRole::Answerer {
            return Err(WebRtcError::Adapter(
                "WHIP ICE restart requires an inbound (answerer) connection".into(),
            ));
        }
        let answer = route.peer.renegotiate_as_answerer(offer_sdp).await?;
        if let Some(mut route_mut) = self.routes.get_mut(&conn) {
            route_mut.local_sdp = Some(answer.clone());
            route_mut.remote_sdp = Some(offer_sdp.to_owned());
        }
        Ok(answer)
    }

    /// Apply a trickle ICE candidate (JSON `RTCIceCandidateInit` shape) to the
    /// peer identified by `conn`. Returns `ConnectionNotFound` if there is no
    /// such route. Drops `.local` mDNS candidates when
    /// `WebRtcConfig::mdns_candidate_policy` is `Drop` (the default).
    #[instrument(skip(self, candidate), fields(conn = %conn))]
    pub async fn apply_trickle_candidate(
        &self,
        conn: &ConnectionId,
        candidate: webrtc::peer_connection::RTCIceCandidateInit,
    ) -> Result<()> {
        let route = self.route(conn)?;
        if matches!(
            self.config.mdns_candidate_policy,
            crate::config::MdnsCandidatePolicy::Drop
        ) && crate::config::MdnsCandidatePolicy::is_mdns_candidate(&candidate.candidate)
        {
            debug!(
                conn = %conn,
                candidate = %candidate.candidate,
                "dropping mDNS (.local) trickle candidate per policy"
            );
            return Ok(());
        }
        route.peer.add_remote_ice_candidate(candidate).await
    }

    /// Re-create a local SDP after a transceiver direction change (hold/resume).
    /// Stores it on the route as `local_sdp` and returns it. The caller (or
    /// signaling layer) is responsible for pushing the new SDP to the remote.
    async fn renegotiate_after_direction_change(&self, conn: &ConnectionId) -> Result<String> {
        let route = self.route(conn)?;
        let sdp = match route.peer.role() {
            PeerRole::Offerer => route.peer.renegotiate_as_offerer().await?,
            PeerRole::Answerer => {
                let offer = route.remote_sdp.clone().ok_or_else(|| {
                    WebRtcError::Sdp("no remote offer stored to renegotiate against".into())
                })?;
                route.peer.renegotiate_as_answerer(&offer).await?
            }
        };
        if let Some(mut route_mut) = self.routes.get_mut(conn) {
            route_mut.local_sdp = Some(sdp.clone());
        }
        Ok(sdp)
    }

    /// Trigger ICE restart and produce a fresh local SDP. Caller is
    /// responsible for re-signaling the resulting SDP to the remote peer.
    #[instrument(skip(self), fields(conn = %conn))]
    pub async fn restart_ice(&self, conn: &ConnectionId) -> Result<String> {
        let route = self.route(conn)?;
        route.peer.restart_ice().await?;
        let sdp = match route.peer.role() {
            PeerRole::Offerer => route.peer.renegotiate_as_offerer().await?,
            PeerRole::Answerer => {
                let offer = route
                    .remote_sdp
                    .clone()
                    .ok_or_else(|| WebRtcError::Sdp("no remote offer to restart against".into()))?;
                route.peer.renegotiate_as_answerer(&offer).await?
            }
        };
        if let Some(mut route_mut) = self.routes.get_mut(conn) {
            route_mut.local_sdp = Some(sdp.clone());
        }
        Ok(sdp)
    }
}

#[async_trait]
impl ConnectionAdapter for WebRtcAdapter {
    fn transport(&self) -> Transport {
        Transport::WebRtc
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    #[instrument(skip(self, request), fields(session = %request.session_id))]
    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let slot = self
            .reserve_session_slot()
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        self.metrics_outbound.fetch_add(1, Ordering::Relaxed);
        let conn_id = ConnectionId::new();
        let peer = RvoipPeerConnection::new(&self.config, PeerRole::Offerer)
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        // Pre-attach a video track when the caller wants outbound offers to
        // include `m=video`. `create_offer_and_gather` skips its auto-audio
        // path when *any* local track is already present, so we still need
        // an explicit audio attach below to keep symmetry with the default
        // behavior.
        if self.config.originate_include_video {
            peer.add_local_audio_track()
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
            peer.add_local_video_track()
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }

        let offer_sdp = peer
            .create_offer_and_gather()
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let negotiated = negotiate_audio(&request.capabilities, &self.config.capabilities)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let cancel = Arc::new(Notify::new());
        let route = Route {
            peer,
            streams: Arc::new(DashMap::new()),
            local_sdp: Some(offer_sdp),
            remote_sdp: None,
            data_channel: Arc::new(DashMap::new()),
            negotiated: negotiated.clone(),
            held: false,
            cancel: Arc::clone(&cancel),
            failed_at: Arc::new(SyncMutex::new(None)),
        };

        // Same rationale as `apply_remote_offer`: lazy seeding in `accept()`.
        self.insert_route(conn_id.clone(), route);
        slot.commit();

        let handle = self.make_transport_handle(conn_id.clone(), cancel);
        let mut connection =
            self.build_connection(conn_id, Direction::Outbound, negotiated, handle);
        connection.session_id = request.session_id;
        connection.participant_id = request.participant_id;

        Ok(ConnectionHandle { connection })
    }

    #[instrument(skip(self), fields(conn = %conn))]
    async fn accept(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        route
            .peer
            .wait_connected(Duration::from_secs(self.config.gather_timeout_secs + 10))
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        self.ensure_media_streams(&conn).await?;
        self.try_send(AdapterEvent::Connected {
            connection_id: conn,
        });
        Ok(())
    }

    async fn reject(&self, conn: ConnectionId, _reason: RejectReason) -> RvoipResult<()> {
        if let Some((_, route)) = self.routes.remove(&conn) {
            route.cancel.notify_waiters();
            route.peer.close().await.ok();
            self.release_session_slot();
        }
        self.try_send(AdapterEvent::Failed {
            connection_id: conn,
            detail: "rejected".into(),
        });
        Ok(())
    }

    #[instrument(skip(self), fields(conn = %conn, reason = ?reason))]
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        if let Some((_, route)) = self.routes.remove(&conn) {
            route.cancel.notify_waiters();
            route.peer.close().await.ok();
            self.release_session_slot();
            info!(conn = %conn, "ended");
        }
        self.try_send(AdapterEvent::Ended {
            connection_id: conn,
            reason,
        });
        Ok(())
    }

    async fn hold(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        route
            .peer
            .hold_audio()
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        if self.config.hold_renegotiate {
            // Best-effort SDP renegotiation so peers that ignore mute also stop sending.
            self.renegotiate_after_direction_change(&conn)
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }
        if let Some(mut route_mut) = self.routes.get_mut(&conn) {
            route_mut.held = true;
        }
        Ok(())
    }

    async fn resume(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        route
            .peer
            .resume_audio()
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        if self.config.hold_renegotiate {
            self.renegotiate_after_direction_change(&conn)
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }
        if let Some(mut route_mut) = self.routes.get_mut(&conn) {
            route_mut.held = false;
        }
        Ok(())
    }

    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "WebRTC transfer requires SIP REFER or renegotiation to a new peer; deferred in v1",
        ))
    }

    async fn streams(&self, conn: ConnectionId) -> RvoipResult<Vec<Arc<dyn MediaStream>>> {
        self.ensure_media_streams(&conn).await?;
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        Ok(route
            .streams
            .iter()
            .map(|e| Arc::clone(e.value()) as Arc<dyn MediaStream>)
            .collect())
    }

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        crate::media::dtmf::send_dtmf(&route.peer, digits, duration_ms)
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))
    }

    async fn send_message(&self, conn: ConnectionId, message: Message) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let dc = if let Some(entry) = route.data_channel.get(&()) {
            entry.value().clone()
        } else {
            let dc = tokio::time::timeout(
                Duration::from_secs(2),
                route.peer.peer_connection().create_data_channel("rvoip-messages", None),
            )
            .await
            .map_err(|_| RvoipError::Adapter("create_data_channel timed out".into()))?
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
            route.data_channel.insert((), Arc::clone(&dc));
            dc
        };

        let body = String::from_utf8_lossy(&message.body).into_owned();
        tokio::time::timeout(Duration::from_secs(2), dc.send_text(&body))
            .await
            .map_err(|_| RvoipError::Adapter("data channel send timed out".into()))?
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        Ok(())
    }

    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> RvoipResult<NegotiatedCodecs> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let negotiated = negotiate_audio(&capabilities, &self.config.capabilities)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let offer = tokio::time::timeout(
            Duration::from_secs(2),
            route.peer.peer_connection().create_offer(None),
        )
        .await
        .map_err(|_| RvoipError::Adapter("create_offer timed out".into()))?
        .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        tokio::time::timeout(
            Duration::from_secs(2),
            route.peer.peer_connection().set_local_description(offer),
        )
        .await
        .map_err(|_| RvoipError::Adapter("set_local_description timed out".into()))?
        .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        if let Some(desc) = route.peer.peer_connection().local_description().await {
            if let Ok(sdp) = sdp_to_string(&desc) {
                if let Some(mut route_mut) = self.routes.get_mut(&conn) {
                    route_mut.local_sdp = Some(sdp);
                }
            }
        }

        Ok(negotiated)
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        match self.try_subscribe_events() {
            Ok(rx) => rx,
            Err(_) => {
                warn!(
                    "WebRtcAdapter::subscribe_events called more than once; \
                     returning closed receiver. Prefer try_subscribe_events() to detect."
                );
                let (_tx, rx) = mpsc::channel(1);
                rx
            }
        }
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        self.config.capabilities.clone()
    }

    async fn verify_request_signature(
        &self,
        conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> RvoipResult<IdentityAssurance> {
        // D2 — surface the negotiated peer's DTLS fingerprint as the
        // assurance. The variant is key-binding without a real-world
        // identity (see `IdentityAssurance::DtlsFingerprint` docs); higher
        // assurance levels require a credential flow handled by auth-core
        // before this point. Returns `Anonymous` when there's no remote SDP
        // yet (outbound originate before the answer lands) or the route is
        // unknown.
        let fps = self
            .remote_dtls_fingerprint(&conn)
            .unwrap_or_default();
        match fps.into_iter().next() {
            Some(fp) => Ok(IdentityAssurance::DtlsFingerprint {
                algorithm: fp.algorithm,
                value: fp.value,
            }),
            None => Ok(IdentityAssurance::Anonymous),
        }
    }
}

/// Guard returned by [`WebRtcAdapter::reserve_session_slot`]. Drops release
/// the slot; `commit()` promotes it to a permanent occupant (released when
/// the route is removed by `end`/`reject`/reaper).
struct SessionSlotGuard {
    live: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

impl SessionSlotGuard {
    /// Promote this reservation into a held slot — the live counter stays
    /// incremented until the matching route is removed. Caller must ensure
    /// a matching release happens (handled in [`WebRtcAdapter::release_session_slot`]).
    fn commit(mut self) {
        self.live = None; // skip the Drop decrement
    }
}

impl Drop for SessionSlotGuard {
    fn drop(&mut self) {
        if let Some(live) = self.live.take() {
            live.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for WebRtcAdapter {
    fn drop(&mut self) {
        // Stop the reaper.
        self.reaper_cancel.notify_waiters();
        // Cancel each route's background tasks; peer connections will be dropped
        // when their Arc refcount hits zero.
        for entry in self.routes.iter() {
            entry.value().cancel.notify_waiters();
        }
    }
}

/// QUIC-bridge-flake fix — attach `track` to **every** stream in the
/// route. Idempotent via [`WebRtcMediaStream::attach_remote`]'s
/// `compare_exchange` guard.
async fn attach_track_to_streams(
    routes: &Arc<DashMap<ConnectionId, Route>>,
    conn: &ConnectionId,
    track: &Arc<dyn webrtc::media_stream::track_remote::TrackRemote>,
) {
    if let Some(route) = routes.get(conn) {
        for entry in route.streams.iter() {
            entry.value().attach_remote(track.clone());
        }
    }
}

/// QUIC-bridge-flake fix — same as above but only attach to streams of
/// the matching kind, so a future video track doesn't end up wired into
/// the audio inbound pump (and vice versa).
async fn attach_track_to_streams_matching(
    routes: &Arc<DashMap<ConnectionId, Route>>,
    conn: &ConnectionId,
    track: &Arc<dyn webrtc::media_stream::track_remote::TrackRemote>,
    kind: rvoip_core::stream::StreamKind,
) {
    if let Some(route) = routes.get(conn) {
        for entry in route.streams.iter() {
            if entry.value().kind() == kind {
                entry.value().attach_remote(track.clone());
            }
        }
    }
}

/// D4 follow-up — map a negotiated audio `CodecInfo` to the RTP payload
/// type the outbound pump should stamp on each packet. Matches the codec
/// table registered by
/// [`build_media_engine`](crate::peer::builder::build_media_engine).
fn payload_type_for_audio_codec(codec: &CodecInfo) -> u8 {
    let name = codec.name.to_ascii_lowercase();
    if name.contains("opus") {
        crate::media::pump::OPUS_PT_DEFAULT
    } else if name.contains("pcmu") || name.starts_with("g.711") && !name.contains("a-law") {
        0 // PCMU
    } else if name.contains("pcma") || name.contains("a-law") {
        8 // PCMA
    } else {
        // Fall back to Opus PT — the engine only registers a handful of
        // audio codecs and the negotiation path narrows to Opus by default.
        crate::media::pump::OPUS_PT_DEFAULT
    }
}

/// Export SDP from a live peer connection (for WHIP/WHEP responses).
pub async fn export_local_sdp(peer: &Arc<RvoipPeerConnection>) -> Result<String> {
    let desc = peer
        .peer_connection()
        .local_description()
        .await
        .ok_or_else(|| WebRtcError::Sdp("no local description".into()))?;
    sdp_to_string(&desc)
}
