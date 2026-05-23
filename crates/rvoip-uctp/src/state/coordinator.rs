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

/// Soft timeout for outbound signaling sends. If `out_tx.send` is pending
/// for longer than this, the writer is treated as wedged and the
/// coordinator triggers its shutdown choreography (design doc §3.5).
pub const SIGNALING_SEND_TIMEOUT: Duration = Duration::from_secs(5);

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

    // §3.9 metrics deferred to v0.x:
    //
    // - `uctp_substrate_pending_outstanding` (gauge): requires
    //   `substrate::correlation::Pending` to be wired into the coordinator,
    //   which only matters once request/response correlation is exercised
    //   (DPoP step-up, message.history, etc.). v0 envelope flows don't
    //   await responses through Pending, so the integration is structural
    //   pre-work not yet needed.
    //
    // - `uctp.connection.lifetime` span: needs per-connection span storage
    //   that survives across discrete handler calls (offer → ready → end).
    //   The current coordinator dispatches handlers individually with no
    //   carry-over context, so adding the span requires restructuring to
    //   carry a `tracing::Span` (or `EnteredSpan`) on the ConnectionMachine.
    //   Track separately when that refactor lands.

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
        match env.msg_type.clone() {
            MessageType::AuthHello => self.handle_auth_hello(env).await,
            MessageType::AuthResponse => self.handle_auth_response(env).await,
            MessageType::SessionInvite => self.handle_session_invite(env).await,
            MessageType::SessionAccept => self.handle_session_accept(env).await,
            MessageType::SessionCancel => self.handle_session_cancel(env).await,
            MessageType::SessionEnd | MessageType::ConnectionEnd => self.handle_end(env).await,
            MessageType::ConnectionOffer => self.handle_connection_offer(env).await,
            MessageType::ConnectionAnswer => self.handle_connection_answer(env).await,
            MessageType::ConnectionReady => self.handle_connection_ready(env).await,
            MessageType::StreamSubscribe => self.handle_stream_subscribe(env).await,
            MessageType::StreamUnsubscribe => self.handle_stream_unsubscribe(env).await,
            MessageType::Unknown(_) => {
                // §3.2 of the spec: silently ignore unknown types.
                Ok(())
            }
            _ => Ok(()),
        }
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
        // longer than SIGNALING_SEND_TIMEOUT, treat the connection as
        // unrecoverable: log, trigger shutdown, and surface Closed.
        match tokio::time::timeout(SIGNALING_SEND_TIMEOUT, self.out_tx.send(env)).await {
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
        let bearer_span = info_span!(
            "uctp.auth.bearer",
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
                let session = payloads::auth::AuthSession {
                    identity_id: format!("id_{}", uuid::Uuid::new_v4().simple()),
                    participant_id: format!("part_{}", uuid::Uuid::new_v4().simple()),
                    session_token: format!("tok_{}", uuid::Uuid::new_v4().simple()),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                    assurance: assurance_label(&assurance).into(),
                    reachability: Vec::new(),
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
        let sid = SessionId::from_string(sid_str);

        let span = info_span!(
            "uctp.session.invite",
            sid = %sid,
            from = %payload.from,
            transport = %self.transport,
        );

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
                let machine_ref = self
                    .connections
                    .entry(connid)
                    .or_insert_with(|| Mutex::new(ConnectionMachine::new_negotiating()));
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
        let machine = self.connections.get(&connid).expect("just checked");
        let mut m = machine.lock();
        let _ = m.apply(ConnectionInput::AnswerReceived);
        Ok(())
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
        let pending = {
            let machine = self.connections.get(&connid).expect("just checked");
            let mut m = machine.lock();
            let _ = m.apply(ConnectionInput::ReadyReceived);
            // Allocate stream_local_ids for the streams that survived
            // negotiation. The first call returns the set; subsequent
            // calls (duplicate connection.ready) return empty.
            m.take_pending_streams()?
        };
        if let Some(machine) = self.sessions.get(&sid) {
            let mut m = machine.lock();
            // Idempotent: ConnectionReady on an already-Active session is a no-op.
            let _ = m.apply(SessionInput::ConnectionReady);
        }

        // Emit `stream.opened` per allocated stream and register the
        // publisher in whatever subscription handler is configured.
        // CONVERSATION_PROTOCOL.md §7.4: server announces the
        // stream_local_id here.
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
            self.subscription_handler.register_publisher(
                super::subscription::PublisherInfo {
                    sid: &sid,
                    strm_id: &stream.strm_id,
                    connection: &connid,
                    participant: &stream.participant,
                    kind: &stream.kind,
                },
            );
        }

        self.emit_event(UctpSessionEvent::ConnectionConnected {
            sid,
            connid,
        })
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
    }
}

