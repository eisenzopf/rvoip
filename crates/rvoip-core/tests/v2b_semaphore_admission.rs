//! V2.B — per-tenant `Semaphore` admission acceptance.
//!
//! Proves the v1 DashMap-bucket-locked check+increment is fully
//! replaced by a tokio `Semaphore`: under N concurrent calls and a
//! quota of M < N, exactly M succeed and N − M get
//! `AdmissionRejected`. Then stop_recording releases the permit and
//! a new start_recording succeeds.

use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::commands::{InboundAction, RecordingTarget};
use rvoip_core::config::{Config, TenantQuotas};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::MediaStream;
use rvoip_harness::VecRecordingSink;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct StubAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl StubAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (tx, rx) = mpsc::channel(64);
        (
            Arc::new(Self {
                inbound: Mutex::new(Some(rx)),
            }),
            tx,
        )
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for StubAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    async fn originate(&self, _: OriginateRequest) -> RvResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented("orig"))
    }
    async fn accept(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn reject(&self, _: ConnectionId, _: RejectReason) -> RvResult<()> {
        Ok(())
    }
    async fn end(&self, _: ConnectionId, _: EndReason) -> RvResult<()> {
        Ok(())
    }
    async fn hold(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn resume(&self, _: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> RvResult<()> {
        Ok(())
    }
    async fn streams(&self, _: ConnectionId) -> RvResult<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![])
    }
    async fn send_message(&self, _: ConnectionId, _: Message) -> RvResult<()> {
        Ok(())
    }
    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> RvResult<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> RvResult<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.inbound.lock().unwrap().take().unwrap()
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> RvResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

/// Stand up a Session under `tenant` with one bound Connection, so
/// start_recording can target it.
async fn make_session_with_connection(
    orch: &Arc<Orchestrator>,
    tx: &mpsc::Sender<AdapterEvent>,
    tenant: &TenantId,
) -> SessionId {
    let cid = orch
        .open_conversation(
            tenant.clone(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open_conversation");
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .expect("start_session");
    let connid = ConnectionId::new();
    tx.send(AdapterEvent::InboundConnection {
        connection: Connection {
            id: connid.clone(),
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            transport: Transport::Sip,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        },
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.route_inbound_connection(
        connid,
        InboundAction::Accept {
            session_id: sid.clone(),
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .expect("accept");
    sid
}

#[tokio::test]
async fn semaphore_admission_under_concurrent_load_respects_quota() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = StubAdapter::new();
    orch.register(adapter).unwrap();

    let tenant = TenantId::new();
    const QUOTA: usize = 5;
    const CONCURRENT: usize = 20;

    orch.set_tenant_quotas(
        tenant.clone(),
        TenantQuotas {
            max_concurrent_recordings: Some(QUOTA),
            ..Default::default()
        },
    )
    .expect("set quota");

    let sid = make_session_with_connection(&orch, &tx, &tenant).await;
    let sink = Arc::new(VecRecordingSink::new("memory:rec/v2b"));
    orch.register_recording_sink("v2b-sink", sink);

    // Fire CONCURRENT start_recording calls at the same time.
    let mut joinset = tokio::task::JoinSet::new();
    for _ in 0..CONCURRENT {
        let orch_c = Arc::clone(&orch);
        let sid_c = sid.clone();
        joinset.spawn(async move {
            orch_c
                .start_recording(RecordingTarget::Session(sid_c), "v2b-sink")
                .await
        });
    }

    let mut accepted = 0;
    let mut rejected = 0;
    let mut accepted_ids = Vec::new();
    while let Some(r) = joinset.join_next().await {
        match r.expect("join") {
            Ok(id) => {
                accepted += 1;
                accepted_ids.push(id);
            }
            Err(RvoipError::AdmissionRejected(_)) => rejected += 1,
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
    assert_eq!(accepted, QUOTA, "exactly QUOTA admits succeed");
    assert_eq!(rejected, CONCURRENT - QUOTA, "rest are rejected");

    // Now stop one and verify a new start succeeds.
    let stopped = accepted_ids.pop().unwrap();
    orch.stop_recording(stopped).await.expect("stop");
    // The permit drops with the handle — a fresh start should fit.
    let _new = orch
        .start_recording(RecordingTarget::Session(sid), "v2b-sink")
        .await
        .expect("new start after release");
}

#[tokio::test]
async fn shrinking_quota_with_held_permits_is_rejected() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = StubAdapter::new();
    orch.register(adapter).unwrap();

    let tenant = TenantId::new();
    orch.set_tenant_quotas(
        tenant.clone(),
        TenantQuotas {
            max_concurrent_recordings: Some(10),
            ..Default::default()
        },
    )
    .unwrap();

    let sid = make_session_with_connection(&orch, &tx, &tenant).await;
    let sink = Arc::new(VecRecordingSink::new("memory:rec/v2b-shrink"));
    orch.register_recording_sink("v2b-shrink", sink);

    // Acquire 3 permits, leaving 7 available.
    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(
            orch.start_recording(RecordingTarget::Session(sid.clone()), "v2b-shrink")
                .await
                .unwrap(),
        );
    }

    // Shrink to 5: only 5 available now (we want to drop to 5 total
    // permits; available = total - issued = 5 - 3 = 2). Current
    // available is 7. Requested available (5) < current (7) → reject.
    let err = orch
        .set_tenant_quotas(
            tenant.clone(),
            TenantQuotas {
                max_concurrent_recordings: Some(5),
                ..Default::default()
            },
        )
        .expect_err("shrink with live permits must reject");
    assert!(matches!(err, RvoipError::InvalidState(_)));

    // Resize-up still works.
    orch.set_tenant_quotas(
        tenant,
        TenantQuotas {
            max_concurrent_recordings: Some(20),
            ..Default::default()
        },
    )
    .expect("resize-up succeeds");

    // Drain to keep test clean.
    for id in held {
        orch.stop_recording(id).await.unwrap();
    }
}
