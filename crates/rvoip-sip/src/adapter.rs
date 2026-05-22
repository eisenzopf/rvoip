//! `SipAdapter` — the [`rvoip_core::ConnectionAdapter`] implementation that
//! plugs the proven [`crate::api::UnifiedCoordinator`] surface into
//! [`rvoip_core::Orchestrator`].
//!
//! Per CARVE_PLAN §2 layering rule: every method here ultimately calls into
//! [`crate::api::UnifiedCoordinator`] (the sole sanctioned path to
//! [`rvoip_sip_dialog`] / [`rvoip_media_core`] from this crate). No new
//! state machine, no parallel SIP plumbing — just translation between the
//! [`rvoip_core`] vocabulary and the [`UnifiedCoordinator`] API.

use crate::api::events::Event as ApiEvent;
use crate::api::unified::{Config as ApiConfig, UnifiedCoordinator};
use crate::SessionId;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as CoreResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId as CoreSessionId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// SIP-protocol adapter. Wraps an [`UnifiedCoordinator`]; every
/// `ConnectionAdapter` method dispatches to it.
pub struct SipAdapter {
    coordinator: Arc<UnifiedCoordinator>,
    /// rvoip-core ConnectionId → SIP api SessionId.
    by_connection: Arc<DashMap<ConnectionId, SessionId>>,
    /// SIP api SessionId → rvoip-core ConnectionId. Used by the event
    /// translator task to map outgoing api::Event → AdapterEvent.
    by_session: Arc<DashMap<SessionId, ConnectionId>>,
    out_tx: mpsc::Sender<AdapterEvent>,
    /// Single-take receiver for [`ConnectionAdapter::subscribe_events`].
    out_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl SipAdapter {
    /// Construct from a fully-configured [`UnifiedCoordinator`]. Spawns the
    /// background event-translation task; the returned `Arc<SipAdapter>` is
    /// what gets registered with [`rvoip_core::Orchestrator::register`].
    pub async fn new(coordinator: Arc<UnifiedCoordinator>) -> crate::errors::Result<Arc<Self>> {
        let (out_tx, out_rx) = mpsc::channel(256);
        let adapter = Arc::new(Self {
            coordinator: Arc::clone(&coordinator),
            by_connection: Arc::new(DashMap::new()),
            by_session: Arc::new(DashMap::new()),
            out_tx: out_tx.clone(),
            out_rx: StdMutex::new(Some(out_rx)),
        });

        // Subscribe to the coordinator's typed event stream and spawn the
        // translator task. EventReceiver yields api::Event values; we map
        // each into AdapterEvent and forward.
        let mut events = coordinator.events().await?;
        let me = Arc::clone(&adapter);
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                me.translate_api_event(event);
            }
            debug!("SipAdapter event translator stream ended");
        });

        Ok(adapter)
    }

    /// Convenience: build a coordinator from `Config` and wrap it.
    pub async fn from_config(config: ApiConfig) -> crate::errors::Result<Arc<Self>> {
        let coordinator = UnifiedCoordinator::new(config).await?;
        Self::new(coordinator).await
    }

    /// Borrow the underlying coordinator (for code that needs both surfaces
    /// during the carve transition — e.g. server::*  helpers).
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }

    fn ensure_mapped(&self, session_id: SessionId) -> ConnectionId {
        if let Some(entry) = self.by_session.get(&session_id) {
            return entry.value().clone();
        }
        let conn_id = ConnectionId::new();
        self.by_session.insert(session_id.clone(), conn_id.clone());
        self.by_connection.insert(conn_id.clone(), session_id);
        conn_id
    }

    fn forget(&self, session_id: &SessionId) {
        if let Some((_, conn_id)) = self.by_session.remove(session_id) {
            self.by_connection.remove(&conn_id);
        }
    }

    fn lookup_session(&self, conn: &ConnectionId) -> CoreResult<SessionId> {
        self.by_connection
            .get(conn)
            .map(|e| e.value().clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))
    }

    fn build_connection(&self, conn_id: ConnectionId, direction: Direction) -> Connection {
        Connection {
            id: conn_id,
            session_id: CoreSessionId::new(),
            participant_id: ParticipantId::new(),
            transport: Transport::Sip,
            direction,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    fn translate_api_event(&self, event: ApiEvent) {
        match event {
            ApiEvent::IncomingCall { call_id, .. } => {
                let conn_id = self.ensure_mapped(call_id);
                let connection = self.build_connection(conn_id, Direction::Inbound);
                self.try_send(AdapterEvent::InboundConnection { connection });
            }
            ApiEvent::CallAnswered { call_id, .. } => {
                let conn_id = self.ensure_mapped(call_id);
                self.try_send(AdapterEvent::Connected {
                    connection_id: conn_id,
                });
            }
            ApiEvent::CallProgress {
                call_id,
                status_code,
                reason,
                ..
            } => {
                let _conn_id = self.ensure_mapped(call_id);
                self.try_send(AdapterEvent::Native {
                    kind: "sip.call_progress",
                    detail: format!("{} {}", status_code, reason),
                });
            }
            ApiEvent::CallEnded { call_id, reason } => {
                let conn_id = self.ensure_mapped(call_id.clone());
                self.forget(&call_id);
                self.try_send(AdapterEvent::Ended {
                    connection_id: conn_id,
                    reason: EndReason::Failed { detail: reason },
                });
            }
            ApiEvent::CallFailed {
                call_id,
                status_code,
                reason,
            } => {
                let conn_id = self.ensure_mapped(call_id.clone());
                self.forget(&call_id);
                self.try_send(AdapterEvent::Failed {
                    connection_id: conn_id,
                    detail: format!("{} {}", status_code, reason),
                });
            }
            ApiEvent::CallCancelled { call_id } => {
                let conn_id = self.ensure_mapped(call_id.clone());
                self.forget(&call_id);
                self.try_send(AdapterEvent::Ended {
                    connection_id: conn_id,
                    reason: EndReason::Cancelled,
                });
            }
            other => {
                self.try_send(AdapterEvent::Native {
                    kind: "sip.api_event",
                    detail: format!("{:?}", other),
                });
            }
        }
    }

    fn try_send(&self, event: AdapterEvent) {
        if let Err(e) = self.out_tx.try_send(event) {
            warn!(
                ?e,
                "SipAdapter event channel full or closed; dropping event"
            );
        }
    }

    fn map_session_err(err: crate::errors::SessionError) -> RvoipError {
        RvoipError::Adapter(format!("session-core: {}", err))
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for SipAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    async fn originate(&self, request: OriginateRequest) -> CoreResult<ConnectionHandle> {
        // The OriginateRequest's `target` is the SIP URI to dial; without an
        // explicit `from` we synthesize a local AOR. Step-7 keeps this simple;
        // step 9 wires real auth/PAI when orchestration-core flows through.
        let from = "sip:anonymous@invalid";
        let session_id = self
            .coordinator
            .invite(Some(from.to_string()), request.target.clone())
            .send()
            .await
            .map_err(Self::map_session_err)?;
        let conn_id = self.ensure_mapped(session_id);
        let mut connection = self.build_connection(conn_id, Direction::Outbound);
        // Carry the caller-supplied vocabulary IDs through so the consumer's
        // session/participant stay coherent.
        connection.session_id = request.session_id;
        connection.participant_id = request.participant_id;
        connection.capabilities = request.capabilities;
        Ok(ConnectionHandle { connection })
    }

    async fn accept(&self, conn: ConnectionId) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .accept_call(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        let (status, phrase) = match reason {
            RejectReason::Busy => (486, "Busy Here"),
            RejectReason::Decline => (603, "Decline"),
            RejectReason::NotFound => (404, "Not Found"),
            RejectReason::Forbidden => (403, "Forbidden"),
            RejectReason::NotAcceptable => (488, "Not Acceptable Here"),
            RejectReason::ServerError => (500, "Server Internal Error"),
            RejectReason::Custom { code, ref phrase } => (code, phrase.as_str()),
        };
        self.coordinator
            .reject(&session_id)
            .with_status(status)
            .with_reason(phrase)
            .send()
            .await
            .map_err(Self::map_session_err)
    }

    async fn end(&self, conn: ConnectionId, _reason: EndReason) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .hangup(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn hold(&self, conn: ConnectionId) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .hold(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn resume(&self, conn: ConnectionId) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .resume(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn transfer(&self, conn: ConnectionId, target: TransferTarget) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        let refer_to = match target {
            TransferTarget::Uri(uri) => uri,
            TransferTarget::Connection(_) | TransferTarget::Session(_) => {
                return Err(RvoipError::NotImplemented(
                    "SipAdapter::transfer — Connection/Session targets need attended-transfer wiring (server::transfer in step 8)",
                ));
            }
        };
        self.coordinator
            .refer(&session_id, refer_to)
            .send()
            .await
            .map_err(Self::map_session_err)
    }

    async fn streams(&self, _conn: ConnectionId) -> CoreResult<Vec<Arc<dyn MediaStream>>> {
        // Wrapping SIP RTP streams as `dyn MediaStream` lands in step 8+
        // alongside `RtpMediaStream`. Step-7 returns an empty set so the
        // trait is satisfied; consumers that don't need direct stream access
        // (the common case for B2BUA / gateway flows) work today.
        Ok(vec![])
    }

    async fn send_message(&self, _conn: ConnectionId, _message: Message) -> CoreResult<()> {
        // SIP MESSAGE wiring lives in api::UnifiedCoordinator::send_message
        // (Step 8 hooks it up once the rvoip-core Message → SIP MESSAGE body
        //  shape is decided).
        Err(RvoipError::NotImplemented(
            "SipAdapter::send_message — SIP MESSAGE wiring lands in step 8",
        ))
    }

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        _duration_ms: u32,
    ) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        // api::send_dtmf takes one digit per call; loop the string.
        for ch in digits.chars() {
            self.coordinator
                .send_dtmf(&session_id, ch)
                .await
                .map_err(Self::map_session_err)?;
        }
        Ok(())
    }

    async fn renegotiate_media(
        &self,
        _conn: ConnectionId,
        _capabilities: CapabilityDescriptor,
    ) -> CoreResult<NegotiatedCodecs> {
        Err(RvoipError::NotImplemented(
            "SipAdapter::renegotiate_media — re-INVITE wiring lands in step 8",
        ))
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.out_rx
            .lock()
            .unwrap()
            .take()
            .expect("SipAdapter::subscribe_events already consumed")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        // Step-7 returns the empty descriptor. Real codec/feature discovery
        // happens by inspecting the negotiated session in step 8+.
        CapabilityDescriptor::default()
    }

    async fn verify_request_signature(
        &self,
        _conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> CoreResult<IdentityAssurance> {
        // Per INTERFACE_DESIGN §6: SIP/WebRTC interop adapters return
        // Anonymous unless the peer presents an HTTP-mediated AAuth/OAuth
        // surface. For v1 SIP we always return Anonymous.
        Ok(IdentityAssurance::Anonymous)
    }
}
