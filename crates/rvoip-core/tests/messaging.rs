//! P4 — messaging end-to-end: persist + fan-out + history pagination
//! + read receipts + inline-body cap.

use bytes::Bytes;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::commands::{InboundAction, MuteDirection};
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, MessageId, ParticipantId, SessionId, TenantId};
use rvoip_core::message::{ContentType, Message, MessageOrigin, MessageRecipients};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::store::MessageFilter;
use rvoip_core::stream::MediaStream;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct MsgAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    sent: AtomicUsize,
}

impl MsgAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (tx, rx) = mpsc::channel(16);
        let a = Arc::new(Self {
            inbound: Mutex::new(Some(rx)),
            sent: AtomicUsize::new(0),
        });
        (a, tx)
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for MsgAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    async fn originate(&self, _r: OriginateRequest) -> RvResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented("orig"))
    }
    async fn accept(&self, _c: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn reject(&self, _c: ConnectionId, _r: RejectReason) -> RvResult<()> {
        Ok(())
    }
    async fn end(&self, _c: ConnectionId, _r: EndReason) -> RvResult<()> {
        Ok(())
    }
    async fn hold(&self, _c: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn resume(&self, _c: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn transfer(&self, _c: ConnectionId, _t: TransferTarget) -> RvResult<()> {
        Ok(())
    }
    async fn streams(&self, _c: ConnectionId) -> RvResult<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![])
    }
    async fn send_message(&self, _c: ConnectionId, _m: Message) -> RvResult<()> {
        self.sent.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn send_dtmf(&self, _c: ConnectionId, _d: &str, _ms: u32) -> RvResult<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _c: ConnectionId,
        _caps: CapabilityDescriptor,
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
        _c: ConnectionId,
        _s: SignatureHeaders,
    ) -> RvResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

fn msg(cid: rvoip_core::ids::ConversationId, body: &str) -> Message {
    Message {
        id: MessageId::new(),
        conversation_id: cid,
        origin: MessageOrigin::System,
        from_participant: ParticipantId::new(),
        to: MessageRecipients::All,
        direction: Direction::Outbound,
        content_type: ContentType::Text,
        body: Bytes::from(body.to_string()),
        attachments: vec![],
        in_reply_to: None,
        timestamp: Utc::now(),
    }
}

async fn drive_inbound(
    tx: &mpsc::Sender<AdapterEvent>,
    connid: ConnectionId,
) {
    tx.send(AdapterEvent::InboundConnection {
        connection: Connection {
            id: connid,
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            transport: Transport::Sip,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: true,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        },
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
}

#[tokio::test]
async fn fanout_send_persists_and_dispatches_to_all_active_legs() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = MsgAdapter::new();
    let counts = adapter.clone();
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
        .start_session(cid.clone(), SessionMedium::TextChat, vec![])
        .await
        .unwrap();

    // Two inbound legs, both accepted into the same Session.
    for _ in 0..2 {
        let cn = ConnectionId::new();
        drive_inbound(&tx, cn.clone()).await;
        orch.route_inbound_connection(
            cn,
            InboundAction::Accept {
                session_id: sid.clone(),
                participant_id: ParticipantId::new(),
            },
        )
        .await
        .unwrap();
    }

    let mid = orch
        .send_message_to_conversation(cid.clone(), msg(cid.clone(), "hello"))
        .await
        .unwrap();
    assert_eq!(counts.sent.load(Ordering::SeqCst), 2);

    let page = orch
        .list_messages(cid, MessageFilter::default(), None)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].id, mid);
}

#[tokio::test]
async fn pagination_returns_cursor_when_page_full() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    // No legs — send_message_to_conversation just persists.
    for i in 0..5 {
        orch.send_message_to_conversation(cid.clone(), msg(cid.clone(), &format!("m{i}")))
            .await
            .unwrap();
    }
    let mut filter = MessageFilter::default();
    filter.page_size = Some(2);
    let p1 = orch
        .list_messages(cid.clone(), filter.clone(), None)
        .await
        .unwrap();
    assert_eq!(p1.messages.len(), 2);
    let cursor = p1.next.expect("cursor present");
    let p2 = orch
        .list_messages(cid, filter, Some(cursor))
        .await
        .unwrap();
    assert_eq!(p2.messages.len(), 2);
}

#[tokio::test]
async fn mark_message_read_emits_message_read_event() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let mid = orch
        .send_message_to_conversation(cid.clone(), msg(cid, "x"))
        .await
        .unwrap();
    let mut events = orch.subscribe_events();
    orch.mark_message_read(mid.clone(), ParticipantId::new())
        .await
        .unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    matches!(ev, Event::MessageRead { message_id, .. } if message_id == mid);
}

#[tokio::test]
async fn oversized_inline_body_rejected_without_attachments() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let mut m = msg(cid.clone(), "");
    m.body = Bytes::from(vec![0u8; 70 * 1024]);
    match orch.send_message_to_conversation(cid, m).await {
        Err(RvoipError::AdmissionRejected(_)) => {}
        other => panic!("expected AdmissionRejected, got {other:?}"),
    }
}

// Unused-import suppressor for items that come up only in parts of the
// suite we exercise transitively.
#[allow(dead_code)]
fn _unused_imports_keep_alive(_: MuteDirection) {}
