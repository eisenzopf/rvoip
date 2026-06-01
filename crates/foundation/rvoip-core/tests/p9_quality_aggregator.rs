//! P9 — Quality aggregator collects MediaQuality samples and
//! `SessionEnded` reports the average.

use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{MediaStream, QualitySnapshot};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct StubAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl StubAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (tx, rx) = mpsc::channel(16);
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

#[tokio::test]
async fn session_ended_carries_aggregated_quality_report() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = StubAdapter::new();
    orch.register(adapter).unwrap();

    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .unwrap();
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
        connid.clone(),
        InboundAction::Accept {
            session_id: sid.clone(),
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();

    // Push three quality samples through the adapter event loop.
    for (j, l) in [(10.0_f32, 1.0_f32), (20.0, 2.0), (30.0, 3.0)] {
        tx.send(AdapterEvent::Quality {
            connection_id: connid.clone(),
            snapshot: QualitySnapshot {
                jitter_ms: j,
                packet_loss_pct: l,
                mos: Some(4.0),
            },
        })
        .await
        .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(80)).await;

    let mut events = orch.subscribe_events();
    orch.end_session(sid.clone(), EndReason::Normal)
        .await
        .unwrap();

    let mut report = None;
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
            Ok(Ok(Event::SessionEnded { report: r, .. })) => {
                report = r;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    let report = report.expect("SessionEnded must carry an aggregated report when samples landed");
    assert!((report.jitter_ms - 20.0).abs() < 0.001, "avg jitter == 20ms");
    assert!((report.packet_loss_pct - 2.0).abs() < 0.001);
    assert_eq!(report.mos, Some(4.0));
}
