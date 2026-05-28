//! `UctpCoordinator` — per-peer driver that routes inbound envelopes to
//! per-Session / per-Connection machines and emits coordinator events.
//!
//! See `UCTP_IMPLEMENTATION_PLAN.md` §3.5 for the full design (shutdown
//! choreography, backpressure policy, observability spans).

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, debug_span, info, info_span, instrument, warn, Instrument};

use crate::substrate::correlation::Pending;

use rvoip_auth_core::BearerValidator;
use rvoip_core::capability::{
    negotiate_streams, CapabilityDescriptor, CodecInfo, NegotiationOutcome, StreamOffer,
};
use rvoip_core::identity::IdentityAssurance;
use crate::envelope::UctpEnvelope;
use crate::errors::{Result, UctpError};
use crate::ids::{ConnectionId, EnvelopeId, SessionId};
use crate::payloads;
use crate::types::MessageType;

use super::connection::{ConnectionInput, ConnectionMachine};
use super::events::UctpSessionEvent;
use super::session::{SessionInput, SessionMachine};
use super::subscription::{rejecting_handler, SubscriptionHandler, SubscriptionOutcome};

/// Substrate transport name used for the `transport` metric/span label.
/// One coordinator instance per peer connection; the adapter that owns
/// the substrate sets this at construction.
pub type TransportLabel = &'static str;

/// Channel capacities per design doc §3.5 / §4.4.
pub const ENVELOPE_CHANNEL_CAP: usize = 256;

/// Default soft timeout for outbound signaling sends. If `out_tx.send`
/// is pending for longer than this, the writer is treated as wedged
/// and the coordinator triggers its shutdown choreography (design doc
/// §3.5). Production deployments on slow/embedded hosts may want to
/// raise this — pass a [`UctpCoordinatorCaps`] with a different
/// `signaling_send_timeout` to [`UctpCoordinator::start_full_with_caps`].
pub const SIGNALING_SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Default per-peer Session cap. A coordinator that has more than this
/// many `Inviting`/`Active`/`Ending` sessions refuses further
/// `session.invite` envelopes with `error 429 too-many-sessions`. v0.x
/// D1 — protects the coordinator's `sessions` `DashMap` from
/// unauthenticated- or runaway-peer flooding. 32 is generous for
/// realistic call-center / mesh-conferencing workloads; deployments
/// with extreme N-party rooms can override via
/// [`UctpCoordinatorCaps`].
pub const MAX_SESSIONS_PER_PEER: usize = 32;

/// Per-peer resource caps for a [`UctpCoordinator`] instance. Exposed
/// via [`UctpCoordinator::start_full_with_caps`] for adapters that
/// want non-default tuning. Both fields have safe defaults from
/// [`SIGNALING_SEND_TIMEOUT`] / [`MAX_SESSIONS_PER_PEER`], so callers
/// that don't care can keep using the existing [`start`] /
/// [`start_full`] entry points.
#[derive(Clone, Debug)]
pub struct UctpCoordinatorCaps {
    /// Soft timeout for outbound signaling sends. See [`SIGNALING_SEND_TIMEOUT`].
    pub signaling_send_timeout: Duration,
    /// Maximum Sessions per peer. Excess invites get `error 429`. See
    /// [`MAX_SESSIONS_PER_PEER`].
    pub max_sessions_per_peer: usize,
    /// P7 — envelope-id replay protection window. `Some(ttl)` enables
    /// the [`rvoip_core::signing::ReplayCache`] gate on dispatch;
    /// `None` disables it (legacy / dev). Default off — production
    /// deployments enable it explicitly via
    /// `UctpCoordinatorCaps::with_replay_protection`.
    pub replay_protection: Option<Duration>,
}

impl Default for UctpCoordinatorCaps {
    fn default() -> Self {
        Self {
            signaling_send_timeout: SIGNALING_SEND_TIMEOUT,
            max_sessions_per_peer: MAX_SESSIONS_PER_PEER,
            replay_protection: None,
        }
    }
}

impl UctpCoordinatorCaps {
    /// P7 — enable envelope replay protection with the given TTL.
    /// CONVERSATION_PROTOCOL.md §5.5 recommends 5 minutes.
    pub fn with_replay_protection(mut self, ttl: Duration) -> Self {
        self.replay_protection = Some(ttl);
        self
    }
}

/// Per-peer auth state tracked on the coordinator. Every envelope other
/// than `auth.hello` / `auth.response` (and unknown types, which are
/// silently dropped) requires `Authenticated`; `Unauthenticated` peers
/// get `error 401 auth/unauthenticated`.
///
/// With the v0 `bearer_stub` validator this is mostly cosmetic — any
/// non-empty token authenticates. The gating exists so the day real
/// DPoP / JWT / AAuth validators land in `auth-core`, the wire flow
/// already refuses session/connection envelopes from peers that haven't
/// completed the `auth.hello → auth.response → auth.session` handshake.
/// See plan §7 / G1.
#[derive(Clone, Debug)]
enum PeerAuthState {
    Unauthenticated,
    Authenticated {
        identity_id: String,
        participant_id: String,
        assurance: IdentityAssurance,
    },
}

pub struct UctpCoordinator {
    transport: TransportLabel,
    sessions: Arc<DashMap<SessionId, Mutex<SessionMachine>>>,
    connections: Arc<DashMap<ConnectionId, Mutex<ConnectionMachine>>>,
    /// Wall-clock start of each session.invite handler — used by the
    /// handshake-duration histogram (design doc §3.9).
    handshake_started: Arc<DashMap<SessionId, Instant>>,
    out_tx: mpsc::Sender<UctpEnvelope>,
    events_tx: mpsc::Sender<UctpSessionEvent>,
    cancel: CancellationToken,
    bearer: Arc<dyn BearerValidator>,
    /// Local capability descriptor used to run `negotiate_streams` against
    /// incoming `connection.offer` payloads. Spec §11.2 488 fires when the
    /// offer's `streams_offered[*].codec_preferences` and this descriptor's
    /// `audio_codecs`/`video_codecs` have no overlap.
    local_descriptor: Arc<CapabilityDescriptor>,
    /// Outstanding correlated-response waiters. Drained on shutdown so
    /// any awaiting `Pending::wait_for` resolves with `SubstrateError::Closed`
    /// rather than hanging until its TTL.
    pending: Arc<Pending>,
    /// Multi-party subscription handler (v0.x MP2). Adapters that want
    /// real routing inject an [`OrchestratorSubscriptionHandler`]
    /// (or similar) at construction. Default [`RejectingHandler`]
    /// preserves the legacy v0 503 reject.
    subscription_handler: Arc<dyn SubscriptionHandler>,
    /// Per-peer auth state (plan §7 G1). Transitions
    /// `Unauthenticated → Authenticated { .. }` in [`handle_auth_response`]
    /// on a successful bearer validation; consulted by every non-auth
    /// envelope dispatch to refuse traffic from un-authed peers with
    /// `error 401 auth/unauthenticated`.
    peer_auth: Arc<Mutex<PeerAuthState>>,
    /// Per-coordinator resource caps. Set at construction via
    /// [`Self::start_full_with_caps`]; defaults from
    /// [`UctpCoordinatorCaps::default`] for the legacy entry points.
    /// Plan D1 / D2.
    caps: UctpCoordinatorCaps,
    /// P7 — envelope-id replay cache. `Some` when
    /// `caps.replay_protection` was set at construction. Inbound
    /// `dispatch_inner` rejects duplicate `env.id` within the cache
    /// TTL with an `error 401 auth/replay` envelope.
    replay_cache: Option<Arc<rvoip_core::signing::ReplayCache>>,
    /// Optional AAuth validator (gap plan §5.1). When `Some`, an
    /// `auth.response` envelope with `method == "aauth"` is routed
    /// here instead of the standard `bearer` validator. When `None`,
    /// `aauth` requests are rejected with `401 auth/aauth-not-configured`.
    aauth: Option<Arc<rvoip_auth_core::AAuthValidator>>,
    /// Optional RFC 9421 inline-signature verifier (gap plan §5.2 v1
    /// punch list). When `Some`, the dispatch gate runs every signed
    /// envelope through this verifier and rejects on failure; if
    /// `sig_policy.requires(env.msg_type)` and `env.signature` is
    /// `None`, the envelope is rejected with `401-1 signature-required`.
    sig_verifier: Option<Arc<rvoip_auth_core::sig9421::Sig9421Verifier>>,
    /// Per-deployment policy for which `MessageType`s mandate a
    /// signature when `sig_verifier` is wired. Defaults to empty
    /// (opportunistic verification only).
    sig_policy: Option<super::Sig9421Policy>,
}

/// Default permissive v0 descriptor — supports the codecs every v0 demo
/// path exercises (opus 48 kHz mono and PCMU 8 kHz mono). Adapters that
/// want stricter negotiation should call [`UctpCoordinator::start_with_descriptor`]
/// with their own descriptor.
pub fn default_v0_descriptor() -> CapabilityDescriptor {
    CapabilityDescriptor {
        audio_codecs: vec![
            CodecInfo {
                name: "opus".into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            },
            CodecInfo {
                name: "g.711-mu".into(),
                clock_rate_hz: 8_000,
                channels: 1,
                fmtp: None,
            },
        ],
        ..Default::default()
    }
}

impl UctpCoordinator {
    /// Spawn the coordinator driver task.
    ///
    /// - `in_rx` — inbound envelopes from the substrate reader.
    /// - `out_tx` — outbound envelopes to the substrate writer.
    /// - `events_tx` — coordinator events to the adapter.
    /// - `bearer` — bearer validator (use [`rvoip_auth_core::bearer_stub`] in v0).
    /// - `transport` — `"quic"` or `"webtransport"` (used as a metrics
    ///   label so per-transport regressions are visible).
    pub fn start(
        transport: TransportLabel,
        in_rx: mpsc::Receiver<UctpEnvelope>,
        out_tx: mpsc::Sender<UctpEnvelope>,
        events_tx: mpsc::Sender<UctpSessionEvent>,
        bearer: Arc<dyn BearerValidator>,
    ) -> Arc<Self> {
        Self::start_with_descriptor(
            transport,
            in_rx,
            out_tx,
            events_tx,
            bearer,
            Arc::new(default_v0_descriptor()),
        )
    }

    /// Spawn the coordinator driver task with an explicit local
    /// capability descriptor.
    ///
    /// The descriptor is consulted by `handle_connection_offer` to run the
    /// CONVERSATION_PROTOCOL.md §8.1 negotiation; on disjoint codec sets
    /// the coordinator emits `error 488 incompatible-capabilities` and
    /// does not create a Connection machine (plan §3.5).
    pub fn start_with_descriptor(
        transport: TransportLabel,
        in_rx: mpsc::Receiver<UctpEnvelope>,
        out_tx: mpsc::Sender<UctpEnvelope>,
        events_tx: mpsc::Sender<UctpSessionEvent>,
        bearer: Arc<dyn BearerValidator>,
        local_descriptor: Arc<CapabilityDescriptor>,
    ) -> Arc<Self> {
        Self::start_full(
            transport,
            in_rx,
            out_tx,
            events_tx,
            bearer,
            local_descriptor,
            rejecting_handler(),
        )
    }

    /// Spawn the coordinator with full configuration including a
    /// [`SubscriptionHandler`] for multi-party routing (v0.x MP2).
    ///
    /// Adapters that want to honor `stream.subscribe` envelopes inject
    /// an orchestrator-backed handler here; otherwise the
    /// [`RejectingHandler`] keeps the legacy 503 reject.
    pub fn start_full(
        transport: TransportLabel,
        in_rx: mpsc::Receiver<UctpEnvelope>,
        out_tx: mpsc::Sender<UctpEnvelope>,
        events_tx: mpsc::Sender<UctpSessionEvent>,
        bearer: Arc<dyn BearerValidator>,
        local_descriptor: Arc<CapabilityDescriptor>,
        subscription_handler: Arc<dyn SubscriptionHandler>,
    ) -> Arc<Self> {
        Self::start_full_with_caps(
            transport,
            in_rx,
            out_tx,
            events_tx,
            bearer,
            local_descriptor,
            subscription_handler,
            UctpCoordinatorCaps::default(),
        )
    }

    /// Spawn the coordinator with full configuration plus per-peer
    /// resource caps (plan D1 / D2). Adapters that want non-default
    /// tuning — slower hosts that need a longer signaling send
    /// timeout, or N-party rooms that exceed the default session
    /// cap — go through this entry point. Other callers should use
    /// [`Self::start_full`] which delegates here with defaults.
    pub fn start_full_with_caps(
        transport: TransportLabel,
        in_rx: mpsc::Receiver<UctpEnvelope>,
        out_tx: mpsc::Sender<UctpEnvelope>,
        events_tx: mpsc::Sender<UctpSessionEvent>,
        bearer: Arc<dyn BearerValidator>,
        local_descriptor: Arc<CapabilityDescriptor>,
        subscription_handler: Arc<dyn SubscriptionHandler>,
        caps: UctpCoordinatorCaps,
    ) -> Arc<Self> {
        let cancel = CancellationToken::new();
        let coord = Arc::new(Self {
            transport,
            sessions: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            handshake_started: Arc::new(DashMap::new()),
            out_tx,
            events_tx,
            cancel,
            bearer,
            local_descriptor,
            pending: Arc::new(Pending::new()),
            subscription_handler,
            peer_auth: Arc::new(Mutex::new(PeerAuthState::Unauthenticated)),
            replay_cache: caps
                .replay_protection
                .map(|ttl| Arc::new(rvoip_core::signing::ReplayCache::new(ttl))),
            caps,
            aauth: None,
            sig_verifier: None,
            sig_policy: None,
        });

        let driver = Arc::clone(&coord);
        tokio::spawn(async move {
            driver.run(in_rx).await;
        });

        coord
    }

    /// Shared `Pending` map for envelope-id correlation. Substrate code
    /// that needs to await a typed response (DPoP step-up, message.history,
    /// etc.) registers with this map; the driver's inbound path delivers
    /// matched responses via `Pending::deliver`. Drained on `shutdown()`.
    pub fn pending(&self) -> Arc<Pending> {
        Arc::clone(&self.pending)
    }

    /// Variant of [`Self::start_full_with_caps`] that also wires an
    /// AAuth validator (gap plan §5.1). With this set, an inbound
    /// `auth.response` envelope with `method == "aauth"` is routed
    /// to the AAuth validator (`subject_token = payload.credential`,
    /// `actor_token = payload.actor_token`). Other methods still go
    /// through the standard `bearer` validator.
    #[allow(clippy::too_many_arguments)]
    pub fn start_full_with_aauth(
        transport: TransportLabel,
        in_rx: mpsc::Receiver<UctpEnvelope>,
        out_tx: mpsc::Sender<UctpEnvelope>,
        events_tx: mpsc::Sender<UctpSessionEvent>,
        bearer: Arc<dyn BearerValidator>,
        aauth: Arc<rvoip_auth_core::AAuthValidator>,
        local_descriptor: Arc<CapabilityDescriptor>,
        subscription_handler: Arc<dyn SubscriptionHandler>,
        caps: UctpCoordinatorCaps,
    ) -> Arc<Self> {
        let cancel = CancellationToken::new();
        let coord = Arc::new(Self {
            transport,
            sessions: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            handshake_started: Arc::new(DashMap::new()),
            out_tx,
            events_tx,
            cancel,
            bearer,
            local_descriptor,
            pending: Arc::new(Pending::new()),
            subscription_handler,
            peer_auth: Arc::new(Mutex::new(PeerAuthState::Unauthenticated)),
            replay_cache: caps
                .replay_protection
                .map(|ttl| Arc::new(rvoip_core::signing::ReplayCache::new(ttl))),
            caps,
            aauth: Some(aauth),
            sig_verifier: None,
            sig_policy: None,
        });
        let driver = Arc::clone(&coord);
        tokio::spawn(async move {
            driver.run(in_rx).await;
        });
        coord
    }

    /// Gap plan §5.2 v1 punch list — variant of
    /// [`Self::start_full_with_caps`] that also wires an RFC 9421
    /// inline-signature verifier plus a policy describing which
    /// envelope types require a signature. Envelopes that carry a
    /// `signature` field are always verified; envelopes without one
    /// pass through unless `policy.requires(env.msg_type)` is true,
    /// in which case the dispatch gate emits
    /// `error 401-1 signature-required`.
    ///
    /// Deployments that want signature enforcement opt in via this
    /// entry point. Default constructors leave both fields `None`,
    /// preserving the pre-v1 behavior.
    #[allow(clippy::too_many_arguments)]
    pub fn start_full_with_sig9421(
        transport: TransportLabel,
        in_rx: mpsc::Receiver<UctpEnvelope>,
        out_tx: mpsc::Sender<UctpEnvelope>,
        events_tx: mpsc::Sender<UctpSessionEvent>,
        bearer: Arc<dyn BearerValidator>,
        sig_verifier: Arc<rvoip_auth_core::sig9421::Sig9421Verifier>,
        sig_policy: super::Sig9421Policy,
        local_descriptor: Arc<CapabilityDescriptor>,
        subscription_handler: Arc<dyn SubscriptionHandler>,
        caps: UctpCoordinatorCaps,
    ) -> Arc<Self> {
        let cancel = CancellationToken::new();
        let coord = Arc::new(Self {
            transport,
            sessions: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            handshake_started: Arc::new(DashMap::new()),
            out_tx,
            events_tx,
            cancel,
            bearer,
            local_descriptor,
            pending: Arc::new(Pending::new()),
            subscription_handler,
            peer_auth: Arc::new(Mutex::new(PeerAuthState::Unauthenticated)),
            replay_cache: caps
                .replay_protection
                .map(|ttl| Arc::new(rvoip_core::signing::ReplayCache::new(ttl))),
            caps,
            aauth: None,
            sig_verifier: Some(sig_verifier),
            sig_policy: Some(sig_policy),
        });
        let driver = Arc::clone(&coord);
        tokio::spawn(async move {
            driver.run(in_rx).await;
        });
        coord
    }

    /// Trigger shutdown and run the §3.5 choreography:
    ///
    /// 1. Cancel the driver token (stops envelope routing).
    /// 2. Synthesize `session.end` for every Active/Inviting/Ending
    ///    Session and emit `UctpSessionEvent::SessionEnded` so the
    ///    adapter / orchestrator sees clean terminal events in flight.
    /// 3. Drain `Pending`, dropping every `oneshot::Sender` so awaiting
    ///    `wait_for` futures resolve with `SubstrateError::Closed`.
    ///
    /// The substrate layer is expected to close the underlying transport
    /// (QUIC `ApplicationClose`, WT session close) **after** this returns
    /// — see the design doc §3.5 layer-3 step.
    pub async fn shutdown(&self) {
        info!(transport = %self.transport, "uctp.coordinator: shutdown requested");
        self.cancel.cancel();

        // Snapshot the active SessionIds so we don't iterate the DashMap
        // while we mutate it (and so the synthesized envelopes can be
        // emitted without holding any locks).
        let active_sids: Vec<SessionId> = self
            .sessions
            .iter()
            .filter_map(|entry| {
                let state = entry.value().lock().state();
                match state {
                    super::session::UctpSessionState::Inviting
                    | super::session::UctpSessionState::Active
                    | super::session::UctpSessionState::Ending => Some(entry.key().clone()),
                    super::session::UctpSessionState::Ended => None,
                }
            })
            .collect();

        for sid in active_sids {
            let payload = payloads::session::SessionEnd {
                by: "local".into(),
                reason_code: 0,
                reason: "shutdown".into(),
            };
            if let Ok(value) = serde_json::to_value(payload) {
                let env = UctpEnvelope::new(MessageType::SessionEnd, value)
                    .with_sid(sid.to_string());
                // Use the timeout-wrapped path: if the substrate writer
                // is wedged we still want to return rather than hang.
                let _ = self.send_out(env).await;
            }
            // Emit the local UctpSessionEvent so adapters/orchestrator
            // see the terminal event even if the wire envelope didn't
            // make it out.
            let _ = self
                .events_tx
                .send(UctpSessionEvent::SessionEnded {
                    sid,
                    reason: "shutdown".into(),
                })
                .await;
        }

        // Drain in-flight correlated-response waiters. Each dropped
        // oneshot::Sender surfaces to its awaiter as SubstrateError::Closed
        // (via Pending::wait_for's timeout-or-recv-err arm).
        self.pending.close();
    }

    fn metric(&self, name: &'static str, direction: &'static str, msg_type: &str) {
        metrics::counter!(
            name,
            "direction" => direction,
            "type" => msg_type.to_string(),
            "transport" => self.transport
        )
        .increment(1);
    }

    fn refresh_gauges(&self) {
        metrics::gauge!(
            "uctp_sessions_active",
            "transport" => self.transport
        )
        .set(self.sessions.len() as f64);
        metrics::gauge!(
            "uctp_connections_active",
            "transport" => self.transport
        )
        .set(self.connections.len() as f64);

        // Count connections in Negotiating state (small lock dance, but
        // connections.len() is bounded in normal usage).
        let negotiating = self
            .connections
            .iter()
            .filter(|entry| {
                entry.value().lock().state()
                    == super::connection::UctpConnectionState::Negotiating
            })
            .count();
        metrics::gauge!(
            "uctp_connections_negotiating",
            "transport" => self.transport
        )
        .set(negotiating as f64);

        // Plan §3.9 — leak detector for request/response correlation.
        // Active once `Pending` is exercised (renegotiate-media etc.).
        metrics::gauge!(
            "uctp_substrate_pending_outstanding",
            "transport" => self.transport
        )
        .set(self.pending.len() as f64);
    }

    #[instrument(name = "uctp.coordinator.driver", skip(self, in_rx), fields(transport = %self.transport))]
    async fn run(self: Arc<Self>, mut in_rx: mpsc::Receiver<UctpEnvelope>) {
        loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => {
                    debug!("uctp.coordinator: driver exiting on cancel");
                    return;
                }
                next = in_rx.recv() => {
                    match next {
                        Some(env) => {
                            if let Err(e) = self.dispatch(env).await {
                                warn!(error = %e, "uctp.coordinator: dispatch failed");
                            }
                        }
                        None => {
                            debug!("uctp.coordinator: in_rx closed; exiting");
                            return;
                        }
                    }
                }
            }
        }
    }

    async fn dispatch(&self, env: UctpEnvelope) -> Result<()> {
        self.metric("uctp_envelopes_total", "in", env.msg_type.as_wire_str());
        let span = info_span!(
            "uctp.envelope.in",
            r#type = %env.msg_type,
            id = %env.id,
            transport = %self.transport,
        );
        self.dispatch_inner(env).instrument(span).await
    }

    async fn dispatch_inner(&self, env: UctpEnvelope) -> Result<()> {
        // §3.1 / §11.2 — version gate. This server only speaks v=1; any
        // envelope with a different `v` gets `505 version-not-supported`
        // (the payload includes the set of `v` values we do speak so
        // the peer can downgrade). Pre-v0.x silently dropped these.
        if env.v != 1 {
            warn!(
                transport = %self.transport,
                got_v = env.v,
                envelope = %env.msg_type,
                "uctp.coordinator: rejecting envelope with unsupported protocol version"
            );
            return self
                .emit_version_not_supported(&env)
                .await
                .or_else(|_| Ok(()));
        }
        // Gap plan §4.2 v1 punch list — reply-correlator gate.
        //
        // Adapter code that needs to await a typed response (e.g.
        // `renegotiate_media` waiting for the peer's `connection.update`
        // reply) registers an envelope id on `self.pending` and awaits.
        // If this inbound envelope's `in_reply_to` matches a registered
        // waiter, hand the envelope to the waiter and return; otherwise
        // fall through to the normal handler dispatch path (the
        // `in_reply_to` field is fine on a peer-initiated envelope —
        // see e.g. `connection.update` requests, which we route below).
        //
        // The deliver gate sits BEFORE the signature gate. Replies that
        // carry an inline signature get delivered unverified — that's
        // intentional in v0: `wait_for` callers are local adapter code
        // that already trusts the upstream pipeline, and the signature
        // verifier's primary job is gating peer-initiated envelopes
        // (auth handshake, session.invite, etc.). A future hardening
        // pass can swap the ordering if signed replies become load-
        // bearing.
        let env = if env.in_reply_to.is_some() {
            match self.pending.deliver(env) {
                Ok(()) => return Ok(()),
                Err(env) => env, // no waiter matched — keep dispatching
            }
        } else {
            env
        };
        // P7 — envelope replay protection (CONVERSATION_PROTOCOL §5.5).
        // Runs AFTER the in-reply-to delivery gate so legitimate
        // retransmits of correlated replies still reach their waiters
        // (the waiter's oneshot semantics de-dup naturally); for
        // peer-initiated envelopes, replay-rejects on duplicate `id`
        // within the TTL window. Disabled by default — production
        // deployments enable via `UctpCoordinatorCaps::with_replay_protection`.
        if let Some(cache) = &self.replay_cache {
            if cache.check_and_record(&env.id).is_err() {
                warn!(
                    transport = %self.transport,
                    envelope = %env.msg_type,
                    id = %env.id,
                    "uctp.coordinator: rejecting replayed envelope"
                );
                self.metric(
                    "uctp_envelopes_replay_rejected_total",
                    "in",
                    env.msg_type.as_wire_str(),
                );
                return Ok(());
            }
        }
        // Gap plan §5.2 v1 punch list — RFC 9421 signature gate.
        // Opt-in: only runs when the coordinator was built via
        // `start_full_with_sig9421`. The gate sits after the version
        // check (so we can still answer 505 on a mismatched v) but
        // before any handler dispatch. Auth envelopes are NOT exempt
        // here — deployments that exempt them do so via a policy that
        // omits AuthHello/AuthResponse from the required set, which
        // is the default.
        if let Some(verifier) = self.sig_verifier.as_ref() {
            let required = self
                .sig_policy
                .as_ref()
                .map(|p| p.requires(&env.msg_type))
                .unwrap_or(false);
            match &env.signature {
                Some(_) => {
                    // The verifier expects the envelope as a JSON value
                    // (canonicalization runs over the same shape).
                    let env_value = match serde_json::to_value(&env) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(error = %e, "uctp.coordinator: sig9421 serialize failed");
                            return self
                                .emit_error(env.id.clone(), 401, "auth", "invalid-signature")
                                .await
                                .or_else(|_| Ok(()));
                        }
                    };
                    if let Err(e) = verifier.verify(&env_value).await {
                        let reason = match e {
                            rvoip_auth_core::sig9421::Sig9421Error::ReplayDetected(_) => {
                                "replay-detected"
                            }
                            rvoip_auth_core::sig9421::Sig9421Error::StaleTimestamp(_) => {
                                "stale-timestamp"
                            }
                            rvoip_auth_core::sig9421::Sig9421Error::UnknownKeyid(_) => {
                                "unknown-keyid"
                            }
                            rvoip_auth_core::sig9421::Sig9421Error::UnsupportedAlgorithm(_) => {
                                "unsupported-algorithm"
                            }
                            rvoip_auth_core::sig9421::Sig9421Error::MalformedSignature(_)
                            | rvoip_auth_core::sig9421::Sig9421Error::MalformedEnvelope
                            | rvoip_auth_core::sig9421::Sig9421Error::MissingEnvelopeId
                            | rvoip_auth_core::sig9421::Sig9421Error::InvalidEnvelopeTimestamp => {
                                "malformed-signature"
                            }
                            _ => "invalid-signature",
                        };
                        warn!(
                            transport = %self.transport,
                            envelope = %env.msg_type,
                            env_id = %env.id,
                            %reason,
                            "uctp.coordinator: sig9421 verify rejected envelope"
                        );
                        return self
                            .emit_error(env.id.clone(), 401, "auth", reason)
                            .await
                            .or_else(|_| Ok(()));
                    }
                }
                None if required => {
                    warn!(
                        transport = %self.transport,
                        envelope = %env.msg_type,
                        env_id = %env.id,
                        "uctp.coordinator: required signature missing"
                    );
                    return self
                        .emit_error(env.id.clone(), 401, "auth", "signature-required")
                        .await
                        .or_else(|_| Ok(()));
                }
                None => {} // unsigned + not required → continue.
            }
        }
        match env.msg_type.clone() {
            // Auth envelopes are the one class that runs pre-auth — the
            // four-envelope handshake from CONVERSATION_PROTOCOL.md §5.1
            // (`auth.hello → auth.challenge → auth.response → auth.session`)
            // is exactly how peers establish auth state.
            MessageType::AuthHello => self.handle_auth_hello(env).await,
            MessageType::AuthResponse => self.handle_auth_response(env).await,
            // §3.2 of the spec: silently ignore unknown types — applies
            // regardless of auth so a forward-compat extension envelope
            // sent before auth completes can't be misread as a 401 trigger.
            MessageType::Unknown(_) => Ok(()),
            // Everything else: refuse from un-authed peers (plan §7 G1).
            other => {
                if !self.require_authenticated(&env).await? {
                    return Ok(());
                }
                match other {
                    MessageType::SessionInvite => self.handle_session_invite(env).await,
                    MessageType::SessionAccept => self.handle_session_accept(env).await,
                    MessageType::SessionCancel => self.handle_session_cancel(env).await,
                    MessageType::SessionEnd | MessageType::ConnectionEnd => {
                        self.handle_end(env).await
                    }
                    MessageType::ConnectionOffer => self.handle_connection_offer(env).await,
                    MessageType::ConnectionAnswer => self.handle_connection_answer(env).await,
                    MessageType::ConnectionReady => self.handle_connection_ready(env).await,
                    MessageType::ConnectionUpdate => self.handle_connection_update(env).await,
                    MessageType::StreamSubscribe => self.handle_stream_subscribe(env).await,
                    MessageType::StreamUnsubscribe => self.handle_stream_unsubscribe(env).await,
                    MessageType::DtmfSend => self.handle_dtmf_send(env).await,
                    MessageType::ConnectionQuality => self.handle_connection_quality(env).await,
                    MessageType::AuthRefresh => self.handle_auth_refresh(env).await,
                    MessageType::IdentityStepUpResponse => {
                        self.handle_step_up_response(env).await
                    }
                    // `identity.step-up-request` is server→client per
                    // CONVERSATION_PROTOCOL.md §5.8 — silently drop on
                    // the inbound dispatcher (the server does not
                    // expect to receive its own request shape).
                    MessageType::IdentityStepUpRequest => Ok(()),
                    _ => Ok(()),
                }
            }
        }
    }

    /// P12.6 — handle an inbound `identity.step-up-response` envelope:
    /// emit [`UctpSessionEvent::StepUpResponse`] so the substrate
    /// adapter can forward it to the orchestrator as
    /// `AdapterEvent::StepUpResponse`. The credential verification +
    /// `IdentityAssuranceChanged` emission happen on the orchestrator
    /// side via [`rvoip_core::Orchestrator::complete_step_up`].
    async fn handle_step_up_response(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::control::IdentityStepUpResponse =
            match serde_json::from_value(env.payload.clone()) {
                Ok(p) => p,
                Err(_) => {
                    return self
                        .emit_error(env.id.clone(), 400, "protocol", "malformed-payload")
                        .await
                        .or_else(|_| Ok(()));
                }
            };
        let connid = env
            .connid
            .as_ref()
            .map(|c| crate::ids::ConnectionId::from_string(c.clone()));
        self.emit_event(UctpSessionEvent::StepUpResponse {
            connid,
            method: payload.method,
            credential: payload.credential,
        })
        .await
    }

    /// P12.6 — build and send an `identity.step-up-request` envelope
    /// to the peer, asking them to re-auth at `required` assurance.
    /// Public so substrate adapters (rvoip-quic, rvoip-webtransport,
    /// rvoip-websocket) can wire `ConnectionAdapter::send_step_up_request`
    /// straight through to the coordinator owning the connection.
    pub async fn send_step_up_request(
        &self,
        connid: Option<String>,
        required: &str,
        allowed_methods: Vec<String>,
        reason: Option<String>,
    ) -> Result<()> {
        let payload = payloads::control::IdentityStepUpRequest {
            required: required.into(),
            allowed_methods,
            reason,
        };
        let mut env = UctpEnvelope::new(
            MessageType::IdentityStepUpRequest,
            serde_json::to_value(payload)?,
        );
        if let Some(c) = connid {
            env = env.with_connid(c);
        }
        self.send_out(env).await
    }

    /// Returns `true` if the peer has completed the auth handshake.
    /// Otherwise emits an `error 401 auth/unauthenticated` envelope
    /// (correlated to the offending envelope's id and carrying its sid
    /// / connid for caller diagnostics) and returns `false` so the
    /// caller can short-circuit the handler.
    async fn require_authenticated(&self, env: &UctpEnvelope) -> Result<bool> {
        let is_authed = matches!(
            &*self.peer_auth.lock(),
            PeerAuthState::Authenticated { .. }
        );
        if is_authed {
            return Ok(true);
        }
        warn!(
            transport = %self.transport,
            envelope = %env.msg_type,
            "uctp.coordinator: refusing envelope from un-authed peer"
        );
        self.emit_error_full(
            env.id.clone(),
            401,
            "auth",
            "unauthenticated",
            env.sid.clone(),
            env.connid.clone(),
        )
        .await?;
        Ok(false)
    }

    async fn send_out(&self, env: UctpEnvelope) -> Result<()> {
        let span = debug_span!(
            "uctp.envelope.out",
            r#type = %env.msg_type,
            id = %env.id,
            in_reply_to = ?env.in_reply_to,
            transport = %self.transport,
        );
        self.send_out_inner(env).instrument(span).await
    }

    async fn send_out_inner(&self, env: UctpEnvelope) -> Result<()> {
        let msg_type_label = env.msg_type.as_wire_str().to_string();
        self.metric("uctp_envelopes_total", "out", &msg_type_label);
        // Backpressure (§3.5): signaling channel never drops; await
        // with a soft timeout. If the substrate writer is wedged for
        // longer than `caps.signaling_send_timeout`, treat the
        // connection as unrecoverable: log, trigger shutdown, and
        // surface Closed. (Plan D2 — was a hard-coded const; now
        // configurable per-coordinator via [`UctpCoordinatorCaps`].)
        match tokio::time::timeout(
            self.caps.signaling_send_timeout,
            self.out_tx.send(env),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(UctpError::Closed),
            Err(_) => {
                warn!(
                    transport = %self.transport,
                    "uctp.coordinator: signaling send timed out — triggering shutdown"
                );
                metrics::counter!(
                    "uctp_envelope_errors_total",
                    "code" => "503".to_string(),
                    "transport" => self.transport
                )
                .increment(1);
                self.cancel.cancel();
                Err(UctpError::Closed)
            }
        }
    }

    async fn emit_event(&self, event: UctpSessionEvent) -> Result<()> {
        let _ = self.events_tx.send(event).await;
        Ok(())
    }

    async fn emit_error(
        &self,
        in_reply_to: String,
        code: u16,
        category: &str,
        reason: &str,
    ) -> Result<()> {
        self.emit_error_full(in_reply_to, code, category, reason, None, None)
            .await
    }

    async fn emit_error_full(
        &self,
        in_reply_to: String,
        code: u16,
        category: &str,
        reason: &str,
        sid: Option<String>,
        connid: Option<String>,
    ) -> Result<()> {
        let payload = payloads::control::Error {
            code,
            category: category.into(),
            reason: reason.into(),
            details: serde_json::Value::Null,
        };
        metrics::counter!(
            "uctp_envelope_errors_total",
            "code" => code.to_string(),
            "transport" => self.transport
        )
        .increment(1);
        let mut env = UctpEnvelope::new(MessageType::Error, serde_json::to_value(payload)?)
            .with_in_reply_to(in_reply_to);
        if let Some(s) = sid {
            env = env.with_sid(s);
        }
        if let Some(c) = connid {
            env = env.with_connid(c);
        }
        self.send_out(env).await
    }

    /// Emit `error 505 version-not-supported` for an envelope whose
    /// `v` field is outside the set this server understands. The reply
    /// includes `details.supported = [1]` so the peer can downgrade.
    /// See CONVERSATION_PROTOCOL.md §11.2.
    async fn emit_version_not_supported(&self, env: &UctpEnvelope) -> Result<()> {
        let payload = payloads::control::Error {
            code: 505,
            category: "protocol".into(),
            reason: "version-not-supported".into(),
            details: serde_json::json!({ "supported": [1u8] }),
        };
        metrics::counter!(
            "uctp_envelope_errors_total",
            "code" => "505".to_string(),
            "transport" => self.transport
        )
        .increment(1);
        let mut reply = UctpEnvelope::new(MessageType::Error, serde_json::to_value(payload)?)
            .with_in_reply_to(env.id.clone());
        if let Some(s) = env.sid.clone() {
            reply = reply.with_sid(s);
        }
        if let Some(c) = env.connid.clone() {
            reply = reply.with_connid(c);
        }
        self.send_out(reply).await
    }

    /// Emit `error 404 not-found` for an envelope addressed to an unknown
    /// session or connection id, per plan §3.5. Returns `Ok(())` so
    /// callers can `return self.not_found(...).await` in a single line.
    async fn not_found(
        &self,
        env: &UctpEnvelope,
        kind: &'static str,
    ) -> Result<()> {
        self.emit_error_full(
            env.id.clone(),
            404,
            "not-found",
            kind,
            env.sid.clone(),
            env.connid.clone(),
        )
        .await
    }

    // --- Handlers ---

    async fn handle_auth_hello(&self, env: UctpEnvelope) -> Result<()> {
        let _payload: payloads::auth::AuthHello = env.decode_payload()?;
        let challenge = payloads::auth::AuthChallenge {
            nonce: EnvelopeId::new().to_string(),
            accepted_methods: vec!["bearer".into()],
            server_capabilities: serde_json::Value::Object(Default::default()),
        };
        let reply = UctpEnvelope::new(MessageType::AuthChallenge, serde_json::to_value(challenge)?)
            .with_in_reply_to(env.id);
        self.send_out(reply).await
    }

    async fn handle_auth_response(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::auth::AuthResponse = env.decode_payload()?;
        // Gap plan §5.1 — AAuth routing. When the peer signals
        // `method = "aauth"` and the coordinator was constructed with
        // an AAuth validator, run the dual-token check; the bearer
        // path stays for every other method (oauth2-dpop, passkey,
        // plain bearer, ...).
        let validation = if payload.method == "aauth" {
            let aauth_span = info_span!(
                "uctp.auth.aauth",
                transport = %self.transport,
            );
            match self.aauth.as_ref() {
                Some(validator) => {
                    let subject = payload.credential.clone();
                    let actor = payload.actor_token.clone().unwrap_or_default();
                    validator
                        .validate_aauth(&subject, &actor)
                        .instrument(aauth_span)
                        .await
                }
                None => {
                    warn!(
                        transport = %self.transport,
                        "auth.aauth: rejected — no AAuthValidator configured"
                    );
                    return self
                        .emit_error(env.id, 401, "auth", "aauth-not-configured")
                        .await;
                }
            }
        } else {
            let bearer_span = info_span!(
                "uctp.auth.bearer",
                method = %payload.method,
                transport = %self.transport,
            );
            self.bearer
                .validate(&payload.credential)
                .instrument(bearer_span)
                .await
        };
        match validation {
            Ok(assurance) => {
                let session = payloads::auth::AuthSession {
                    identity_id: format!("id_{}", uuid::Uuid::new_v4().simple()),
                    participant_id: format!("part_{}", uuid::Uuid::new_v4().simple()),
                    session_token: format!("tok_{}", uuid::Uuid::new_v4().simple()),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                    assurance: assurance_label(&assurance).into(),
                    reachability: Vec::new(),
                };
                // Flip the per-peer auth gate (plan §7 G1). Subsequent
                // session/connection/stream envelopes from this peer now
                // pass `require_authenticated`.
                *self.peer_auth.lock() = PeerAuthState::Authenticated {
                    identity_id: session.identity_id.clone(),
                    participant_id: session.participant_id.clone(),
                    assurance: assurance.clone(),
                };
                let reply = UctpEnvelope::new(
                    MessageType::AuthSession,
                    serde_json::to_value(&session)?,
                )
                .with_in_reply_to(env.id);
                self.send_out(reply).await?;
                self.emit_event(UctpSessionEvent::Authenticated {
                    identity_id: session.identity_id,
                    participant_id: session.participant_id,
                    assurance,
                })
                .await
            }
            Err(e) => {
                warn!(error = %e, "auth.bearer: validation failed");
                self.emit_error(env.id, 401, "auth", "bearer-validation-failed")
                    .await
            }
        }
    }

    async fn handle_session_invite(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::session::SessionInvite = env.decode_payload()?;
        let sid_str = env
            .sid
            .clone()
            .ok_or(UctpError::MissingField("sid"))?;
        let sid = SessionId::from_string(sid_str.clone());

        let span = info_span!(
            "uctp.session.invite",
            sid = %sid,
            from = %payload.from,
            transport = %self.transport,
        );

        // D1: per-peer Session cap. A peer that floods invites can
        // balloon the `sessions` DashMap; refuse new sessions over the
        // configured cap with `error 429 too-many-sessions`.
        // Idempotency: an invite for an *existing* sid (retransmit, or
        // mid-flight cross-traffic) is still accepted so we don't
        // break the §7.2 lifecycle on a duplicate.
        if !self.sessions.contains_key(&sid)
            && self.sessions.len() >= self.caps.max_sessions_per_peer
        {
            warn!(
                transport = %self.transport,
                limit = self.caps.max_sessions_per_peer,
                "uctp.coordinator: refusing session.invite — peer over session cap"
            );
            return self
                .emit_error_full(
                    env.id.clone(),
                    429,
                    "rate-limit",
                    "too-many-sessions",
                    Some(sid_str),
                    None,
                )
                .await;
        }

        self.sessions
            .entry(sid.clone())
            .or_insert_with(|| Mutex::new(SessionMachine::new_inviting()));
        self.handshake_started
            .entry(sid.clone())
            .or_insert_with(Instant::now);
        self.refresh_gauges();

        self.emit_event(UctpSessionEvent::InboundInvite {
            sid,
            from: payload.from,
            to: payload.to,
            medium: payload.medium,
        })
        .instrument(span)
        .await
    }

    async fn handle_session_accept(&self, env: UctpEnvelope) -> Result<()> {
        let sid_str = env.sid.clone().ok_or(UctpError::MissingField("sid"))?;
        let sid = SessionId::from_string(sid_str);
        let known = self.sessions.contains_key(&sid);
        if !known {
            return self.not_found(&env, "unknown-sid").await;
        }
        let applied = {
            let machine = self.sessions.get(&sid).expect("contains_key just true");
            let mut m = machine.lock();
            m.apply(SessionInput::AcceptReceived).is_ok()
        };
        if applied {
            // Handshake duration histogram (§3.9).
            if let Some((_, started)) = self.handshake_started.remove(&sid) {
                metrics::histogram!(
                    "uctp_handshake_duration_seconds",
                    "transport" => self.transport,
                    "outcome" => "ok"
                )
                .record(started.elapsed().as_secs_f64());
            }
            self.emit_event(UctpSessionEvent::SessionConnected { sid }).await?;
        }
        Ok(())
    }

    async fn handle_session_cancel(&self, env: UctpEnvelope) -> Result<()> {
        let sid_str = env.sid.clone().ok_or(UctpError::MissingField("sid"))?;
        let sid = SessionId::from_string(sid_str);
        let known = self.sessions.contains_key(&sid);
        if !known {
            return self.not_found(&env, "unknown-sid").await;
        }
        let applied = {
            let machine = self.sessions.get(&sid).expect("contains_key just true");
            let mut m = machine.lock();
            m.apply(SessionInput::CancelReceived).is_ok()
        };
        if applied {
            self.emit_event(UctpSessionEvent::SessionEnded {
                sid,
                reason: "cancelled".into(),
            })
            .await?;
        }
        Ok(())
    }

    async fn handle_connection_offer(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::connection::ConnectionOffer = env.decode_payload()?;
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let connid = ConnectionId::from_string(connid_str.clone());

        // §8.1 capability negotiation: walk the offer's streams against
        // the local descriptor. Spec §11.2 488 fires when no stream's
        // codec_preferences overlap with what we support. We borrow
        // String -> &str for the StreamOffer<'a> view that
        // negotiate_streams expects.
        let preferences: Vec<Vec<String>> = payload
            .streams_offered
            .iter()
            .map(|s| s.codec_preferences.clone())
            .collect();
        let offers: Vec<StreamOffer<'_>> = payload
            .streams_offered
            .iter()
            .zip(preferences.iter())
            .map(|(s, prefs)| StreamOffer {
                id: s.id.as_str(),
                kind: s.kind.as_str(),
                direction: s.direction.as_str(),
                codec_preferences: prefs.as_slice(),
            })
            .collect();

        match negotiate_streams(offers, &self.local_descriptor) {
            NegotiationOutcome::NotAcceptable488 => {
                metrics::counter!(
                    "uctp_capability_negotiations_total",
                    "outcome" => "488",
                    "transport" => self.transport
                )
                .increment(1);
                // Spec §11.2: emit error 488 in_reply_to the offer. The
                // connection machine is NOT created — the negotiation
                // failed before any state would have been useful.
                return self
                    .emit_error_full(
                        env.id.clone(),
                        488,
                        "capability",
                        "incompatible-capabilities",
                        env.sid.clone(),
                        Some(connid_str),
                    )
                    .await;
            }
            NegotiationOutcome::Ok(negotiated) => {
                // C5: open the per-Connection lifetime span here so
                // every subsequent envelope on this connid nests under
                // it. The span lives on the ConnectionMachine and
                // closes when the machine is dropped from
                // `connections` at end-of-call.
                let lifetime_span = info_span!(
                    "uctp.connection.lifetime",
                    connid = %connid,
                    sid = ?env.sid,
                    transport = %self.transport,
                );
                let _lifetime_enter = lifetime_span.clone().entered();

                // Enter the negotiate span synchronously — the rest of
                // this handler does no awaits, so `.entered()` is safe.
                let _span = info_span!(
                    "uctp.connection.negotiate",
                    connid = %connid,
                    transport = %self.transport,
                )
                .entered();
                // Build the AcceptedStream set: zip the
                // `negotiated` results back against the offer to
                // preserve kind/direction. All streams in one offer
                // share the same publisher (`by_participant`), which
                // we stamp onto each AcceptedStream so the
                // subscription handler can resolve `from_participant`
                // queries against any of them.
                let publisher_participant = payload.by_participant.clone();
                let accepted: Vec<super::connection::AcceptedStream> = negotiated
                    .into_iter()
                    .map(|n| super::connection::AcceptedStream {
                        strm_id: n.stream_id,
                        kind: n.kind,
                        direction: n.direction,
                        chosen_codec: n.chosen_codec,
                        participant: publisher_participant.clone(),
                    })
                    .collect();
                let machine_ref = self.connections.entry(connid).or_insert_with(|| {
                    Mutex::new(ConnectionMachine::new_negotiating_with_span(
                        lifetime_span.clone(),
                    ))
                });
                machine_ref.lock().set_pending_streams(accepted);
                drop(machine_ref);
                self.refresh_gauges();
                metrics::counter!(
                    "uctp_capability_negotiations_total",
                    "outcome" => "ok",
                    "transport" => self.transport
                )
                .increment(1);
                Ok(())
            }
        }
    }

    async fn handle_connection_answer(&self, env: UctpEnvelope) -> Result<()> {
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let connid = ConnectionId::from_string(connid_str);
        if !self.connections.contains_key(&connid) {
            return self.not_found(&env, "unknown-connid").await;
        }
        // C5: do the state transition inside `lifetime.in_scope` so
        // this handler's tracing nests under the per-Connection span.
        // No `.entered()` guard here — `in_scope` confines the span to
        // a sync closure that holds the MutexGuard, which is safe
        // because the closure doesn't await.
        let machine = self.connections.get(&connid).expect("just checked");
        let lifetime = machine.lock().lifetime_span();
        lifetime.in_scope(|| {
            let mut m = machine.lock();
            let _ = m.apply(ConnectionInput::AnswerReceived);
        });
        Ok(())
    }

    /// Handle `connection.update` (CONVERSATION_PROTOCOL.md §7.4).
    ///
    /// Supported actions (gap plan §4.2):
    /// - `"renegotiate-media"` — runs the §8.1 negotiation algorithm
    ///   against the requested `codec_preferences` and the local
    ///   capability descriptor. On overlap, replies with a
    ///   `connection.update` envelope carrying the chosen codec under
    ///   the same action label. On no overlap, replies with `error 488
    ///   not-acceptable`.
    /// - `"hold"`, `"resume"`, `"mute"`, `"unmute"` — currently emit
    ///   `ack`. The actual stream-direction change is a future hop.
    /// - Unknown actions — emit `ack` for forward-compat (peers may
    ///   send v1 actions we don't yet recognize).
    ///
    /// Pre-§4.2 the coordinator silently dropped `connection.update`
    /// envelopes; the adapter-level `renegotiate_media` (still a
    /// `NotImplemented` stub at the moment per §4.2 carryover) is the
    /// driver-side counterpart of this handler.
    async fn handle_connection_update(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::connection::ConnectionUpdate = env.decode_payload()?;
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let connid = ConnectionId::from_string(connid_str.clone());
        if !self.connections.contains_key(&connid) {
            return self.not_found(&env, "unknown-connid").await;
        }

        match payload.action.as_str() {
            "renegotiate-media" => {
                let prefs = payload.codec_preferences.clone();
                let stream_ids: Vec<String> = if payload.streams.is_empty() {
                    vec!["strm_audio".into()]
                } else {
                    payload.streams.clone()
                };
                let offers: Vec<StreamOffer<'_>> = stream_ids
                    .iter()
                    .map(|sid| StreamOffer {
                        id: sid.as_str(),
                        kind: "audio",
                        direction: "send-recv",
                        codec_preferences: prefs.as_slice(),
                    })
                    .collect();

                match negotiate_streams(offers, &self.local_descriptor) {
                    NegotiationOutcome::NotAcceptable488 => {
                        metrics::counter!(
                            "uctp_capability_renegotiations_total",
                            "outcome" => "488",
                            "transport" => self.transport
                        )
                        .increment(1);
                        return self
                            .emit_error_full(
                                env.id.clone(),
                                488,
                                "capability",
                                "incompatible-capabilities",
                                env.sid.clone(),
                                Some(connid_str),
                            )
                            .await;
                    }
                    NegotiationOutcome::Ok(results) => {
                        metrics::counter!(
                            "uctp_capability_renegotiations_total",
                            "outcome" => "ok",
                            "transport" => self.transport
                        )
                        .increment(1);
                        // Echo the chosen codec(s) back. The body is a
                        // `connection.update` reply with the same action
                        // label plus the chosen codec under
                        // `codec_preferences` (a single-element list —
                        // the negotiation collapsed the preference
                        // ordering to one). Peers that initiated the
                        // renegotiation use this to drive their
                        // adapter-side codec swap.
                        let chosen: Vec<String> = results
                            .into_iter()
                            .filter_map(|r| r.chosen_codec)
                            .collect();
                        let reply_payload = payloads::connection::ConnectionUpdate {
                            action: "renegotiate-media".into(),
                            streams: stream_ids,
                            codec_preferences: chosen,
                            details: serde_json::Value::Null,
                        };
                        let mut reply = UctpEnvelope::new(
                            MessageType::ConnectionUpdate,
                            serde_json::to_value(reply_payload)?,
                        )
                        .with_in_reply_to(env.id.clone())
                        .with_connid(connid_str);
                        if let Some(s) = env.sid.clone() {
                            reply = reply.with_sid(s);
                        }
                        return self.send_out(reply).await;
                    }
                }
            }
            // Forward-compat: ack any other (or unknown) action.
            _ => {
                let ack = UctpEnvelope::new(MessageType::Ack, serde_json::Value::Null)
                    .with_in_reply_to(env.id.clone());
                let ack = if let Some(s) = env.sid.clone() {
                    ack.with_sid(s)
                } else {
                    ack
                };
                let ack = ack.with_connid(connid_str);
                return self.send_out(ack).await;
            }
        }
    }

    async fn handle_connection_ready(&self, env: UctpEnvelope) -> Result<()> {
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let connid = ConnectionId::from_string(connid_str.clone());
        let sid_str = env.sid.clone().ok_or(UctpError::MissingField("sid"))?;
        let sid = SessionId::from_string(sid_str.clone());
        if !self.connections.contains_key(&connid) {
            return self.not_found(&env, "unknown-connid").await;
        }
        // Apply the state transitions and drain pending streams while
        // holding the per-machine lock. Drop the lock before any
        // outbound send so we don't hold it across awaits.
        // C5: capture the lifetime span at the same time so the
        // outbound `stream.opened` emissions and the publisher
        // registration calls below nest under it.
        let (pending, lifetime_span) = {
            let machine = self.connections.get(&connid).expect("just checked");
            let mut m = machine.lock();
            let _ = m.apply(ConnectionInput::ReadyReceived);
            // Allocate stream_local_ids for the streams that survived
            // negotiation. The first call returns the set; subsequent
            // calls (duplicate connection.ready) return empty.
            (m.take_pending_streams()?, m.lifetime_span())
        };
        // C5: wrap the async tail under the per-Connection lifetime
        // span via `.instrument`. A sync `.entered()` guard isn't
        // Send-safe across the `send_out(...).await` in the loop
        // below, so the future-based approach is correct here.
        async move {
            if let Some(machine) = self.sessions.get(&sid) {
                let mut m = machine.lock();
                // Idempotent: ConnectionReady on an already-Active session
                // is a no-op.
                let _ = m.apply(SessionInput::ConnectionReady);
            }

            // Emit `stream.opened` per allocated stream and register
            // the publisher in whatever subscription handler is
            // configured. CONVERSATION_PROTOCOL.md §7.4: server
            // announces the stream_local_id here.
            for (stream, local_id) in pending {
                let stream_info = payloads::stream::StreamInfo {
                    strm_id: stream.strm_id.clone(),
                    kind: stream.kind.clone(),
                    codec: stream
                        .chosen_codec
                        .as_ref()
                        .map(|c| serde_json::json!({ "name": c }))
                        .unwrap_or(serde_json::Value::Null),
                    direction: stream.direction.clone(),
                    stream_local_id: local_id,
                    opened_at: chrono::Utc::now(),
                };
                let opened_env = UctpEnvelope::new(
                    MessageType::StreamOpened,
                    serde_json::to_value(payloads::stream::StreamOpened {
                        stream: stream_info,
                    })?,
                )
                .with_sid(sid_str.clone())
                .with_connid(connid_str.clone());
                self.send_out(opened_env).await?;
                let codec = stream.chosen_codec.as_ref().map(|name| {
                    rvoip_core::capability::CodecInfo::from_name_with_defaults(name)
                });
                self.subscription_handler.register_publisher(
                    super::subscription::PublisherInfo {
                        sid: &sid,
                        strm_id: &stream.strm_id,
                        connection: &connid,
                        participant: &stream.participant,
                        kind: &stream.kind,
                        codec,
                    },
                );
            }

            self.emit_event(UctpSessionEvent::ConnectionConnected {
                sid,
                connid,
            })
            .await
        }
        .instrument(lifetime_span)
        .await
    }

    async fn handle_stream_subscribe(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::stream::StreamSubscribe = env.decode_payload()?;
        let sid_str = env.sid.clone().ok_or(UctpError::MissingField("sid"))?;
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let sid = SessionId::from_string(sid_str);
        let subscriber = ConnectionId::from_string(connid_str);

        match self
            .subscription_handler
            .subscribe(&sid, &subscriber, &payload)
        {
            SubscriptionOutcome::Ok => {
                let ack = UctpEnvelope::new(
                    MessageType::Ack,
                    serde_json::to_value(payloads::control::Ack::default())?,
                )
                .with_in_reply_to(env.id)
                .with_sid(sid.to_string())
                .with_connid(subscriber.to_string());
                self.send_out(ack).await
            }
            SubscriptionOutcome::Reject { code, reason } => {
                self.emit_error_full(
                    env.id.clone(),
                    code,
                    if code == 404 { "not-found" } else { "transient" },
                    &reason,
                    Some(sid.to_string()),
                    Some(subscriber.to_string()),
                )
                .await
            }
        }
    }

    async fn handle_stream_unsubscribe(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::stream::StreamUnsubscribe = env.decode_payload()?;
        let sid_str = env.sid.clone().ok_or(UctpError::MissingField("sid"))?;
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let sid = SessionId::from_string(sid_str);
        let subscriber = ConnectionId::from_string(connid_str);

        match self
            .subscription_handler
            .unsubscribe(&sid, &subscriber, &payload)
        {
            SubscriptionOutcome::Ok => {
                let ack = UctpEnvelope::new(
                    MessageType::Ack,
                    serde_json::to_value(payloads::control::Ack::default())?,
                )
                .with_in_reply_to(env.id)
                .with_sid(sid.to_string())
                .with_connid(subscriber.to_string());
                self.send_out(ack).await
            }
            SubscriptionOutcome::Reject { code, reason } => {
                self.emit_error_full(
                    env.id.clone(),
                    code,
                    if code == 404 { "not-found" } else { "transient" },
                    &reason,
                    Some(sid.to_string()),
                    Some(subscriber.to_string()),
                )
                .await
            }
        }
    }

    async fn handle_end(&self, env: UctpEnvelope) -> Result<()> {
        // Treat session.end and connection.end as a single category for v0:
        // both wind down the matching machine and emit Ended events.
        let connid = env.connid.clone().map(ConnectionId::from_string);
        let sid = env.sid.clone().map(SessionId::from_string);

        let connid_known = connid
            .as_ref()
            .map(|c| self.connections.contains_key(c))
            .unwrap_or(false);
        let sid_known = sid
            .as_ref()
            .map(|s| self.sessions.contains_key(s))
            .unwrap_or(false);

        // 404 only when the envelope addresses ids that are *all* unknown.
        // A session.end with sid only must 404 if the sid is unknown; a
        // connection.end with connid only must 404 if the connid is
        // unknown; an envelope carrying both must 404 only if neither
        // exists (otherwise we still want the partial teardown).
        if !connid_known && !sid_known {
            return self.not_found(&env, "unknown-session-or-connection").await;
        }

        if let Some(ref connid) = connid {
            if let Some(machine) = self.connections.get(connid) {
                let mut m = machine.lock();
                let _ = m.apply(ConnectionInput::EndReceived);
            }
        }
        if let Some(ref sid) = sid {
            if let Some(machine) = self.sessions.get(sid) {
                let mut m = machine.lock();
                // EndReceived on Active → Ending; LastConnectionEnded → Ended.
                let _ = m.apply(SessionInput::EndReceived);
                let _ = m.apply(SessionInput::LastConnectionEnded);
            }
        }

        if let (Some(connid), Some(sid)) = (connid.clone(), sid.clone()) {
            self.emit_event(UctpSessionEvent::ConnectionEnded {
                sid,
                connid,
                reason: "peer-ended".into(),
            })
            .await?;
        }
        if let Some(sid) = sid {
            self.handshake_started.remove(&sid);
            self.refresh_gauges();
            self.emit_event(UctpSessionEvent::SessionEnded {
                sid,
                reason: "peer-ended".into(),
            })
            .await?;
        }

        Ok(())
    }

    /// Handle inbound `dtmf.send` (peer pressed digits) per
    /// CONVERSATION_PROTOCOL.md §7.5. Emits `UctpSessionEvent::Dtmf`
    /// so the adapter can translate to `AdapterEvent::Dtmf` and the
    /// orchestrator surfaces it on the event bus as
    /// `Event::DtmfReceived`. Plan C2.
    ///
    /// An envelope addressed to an unknown connid produces `error 404
    /// not-found/unknown-connid` to match the rest of the connection-
    /// scoped handlers.
    async fn handle_dtmf_send(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::control::DtmfSend = env.decode_payload()?;
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let connid = ConnectionId::from_string(connid_str);
        if !self.connections.contains_key(&connid) {
            return self.not_found(&env, "unknown-connid").await;
        }
        self.emit_event(UctpSessionEvent::Dtmf {
            connid,
            digits: payload.digits,
            duration_ms: payload.duration_ms,
            method: payload.method,
        })
        .await
    }

    /// Handle inbound `auth.refresh` — plan D4. Validates the new
    /// credential, updates `PeerAuthState` on success, and replies
    /// with a fresh `auth.session` envelope. On validation failure
    /// the existing session is preserved (the peer can retry); a
    /// `401 auth/refresh-failed` error is returned but the gate
    /// stays open for envelopes still validating under the prior
    /// token, so a momentary refresh hiccup doesn't drop the call.
    async fn handle_auth_refresh(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::auth::AuthRefresh = env.decode_payload()?;
        let bearer_span = info_span!(
            "uctp.auth.refresh",
            method = %payload.method,
            transport = %self.transport,
        );
        let validation = self
            .bearer
            .validate(&payload.credential)
            .instrument(bearer_span)
            .await;
        match validation {
            Ok(assurance) => {
                // Reuse identity_id / participant_id from the prior
                // auth state if present. The wire spec treats refresh
                // as continuity (same logical session); reissuing
                // brand-new ids would force re-binding on consumers.
                let (identity_id, participant_id) = match &*self.peer_auth.lock() {
                    PeerAuthState::Authenticated {
                        identity_id,
                        participant_id,
                        ..
                    } => (identity_id.clone(), participant_id.clone()),
                    PeerAuthState::Unauthenticated => {
                        // Refresh without prior auth — synthesize fresh
                        // ids. This shouldn't be reachable post-A1
                        // because the gate refuses non-auth envelopes
                        // from un-authed peers, but be defensive.
                        (
                            format!("id_{}", uuid::Uuid::new_v4().simple()),
                            format!("part_{}", uuid::Uuid::new_v4().simple()),
                        )
                    }
                };
                let session = payloads::auth::AuthSession {
                    identity_id: identity_id.clone(),
                    participant_id: participant_id.clone(),
                    session_token: format!("tok_{}", uuid::Uuid::new_v4().simple()),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                    assurance: assurance_label(&assurance).into(),
                    reachability: Vec::new(),
                };
                // Update the auth gate with the refreshed assurance.
                *self.peer_auth.lock() = PeerAuthState::Authenticated {
                    identity_id: identity_id.clone(),
                    participant_id: participant_id.clone(),
                    assurance: assurance.clone(),
                };
                let reply = UctpEnvelope::new(
                    MessageType::AuthSession,
                    serde_json::to_value(&session)?,
                )
                .with_in_reply_to(env.id);
                self.send_out(reply).await?;
                self.emit_event(UctpSessionEvent::Authenticated {
                    identity_id,
                    participant_id,
                    assurance,
                })
                .await
            }
            Err(e) => {
                warn!(error = %e, "auth.refresh: validation failed; existing session preserved");
                // 401 with a distinct reason so the peer can
                // distinguish a failed refresh from a failed initial
                // auth. Existing PeerAuthState is intentionally left
                // alone — the peer keeps using its current token
                // until it actually expires.
                self.emit_error(env.id, 401, "auth", "refresh-failed").await
            }
        }
    }

    /// Handle inbound `connection.quality` per CONVERSATION_PROTOCOL.md
    /// §10.3. The envelope carries a snapshot per Stream; this emits
    /// one `UctpSessionEvent::Quality` per Stream so adapters can
    /// translate to `AdapterEvent::Quality` and the orchestrator
    /// publishes `Event::MediaQuality`. Plan C2.
    ///
    /// `loss_pct` is preserved verbatim; `mos` is wrapped in `Some`
    /// (the wire payload's `mos` is `f32`, but the rvoip-core
    /// `QualitySnapshot::mos` is `Option<f32>` so consumers can
    /// distinguish "no MOS reported" from "MOS == 0.0").
    async fn handle_connection_quality(&self, env: UctpEnvelope) -> Result<()> {
        let payload: payloads::connection::ConnectionQuality = env.decode_payload()?;
        let connid_str = env.connid.clone().ok_or(UctpError::MissingField("connid"))?;
        let connid = ConnectionId::from_string(connid_str);
        if !self.connections.contains_key(&connid) {
            return self.not_found(&env, "unknown-connid").await;
        }
        for stream in payload.streams {
            let snapshot = rvoip_core::stream::QualitySnapshot {
                jitter_ms: stream.jitter_ms as f32,
                packet_loss_pct: stream.loss_pct,
                mos: Some(stream.mos),
            };
            self.emit_event(UctpSessionEvent::Quality {
                connid: connid.clone(),
                strm_id: stream.strm_id,
                snapshot,
                rtt_ms: stream.rtt_ms,
                bitrate_bps: stream.bitrate_bps,
            })
            .await?;
        }
        Ok(())
    }
}

/// Map a typed `IdentityAssurance` to the wire-format string used in
/// `auth.session.payload.assurance`.
fn assurance_label(a: &rvoip_core::identity::IdentityAssurance) -> &'static str {
    use rvoip_core::identity::IdentityAssurance::*;
    match a {
        Anonymous => "anonymous",
        Pseudonymous { .. } => "pseudonymous",
        Identified { .. } => "identified",
        TaskScoped { .. } => "task-scoped",
        UserAuthorized { .. } => "user-authorized",
        // D2 — DTLS fingerprint binding is key-binding without a
        // real-world identity, so on the wire it maps to "pseudonymous"
        // (matches `AssuranceLevel::from_core`).
        DtlsFingerprint { .. } => "pseudonymous",
    }
}

