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
use rvoip_core::{DataMessage, DataReliability};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct MsgAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    sent: AtomicUsize,
    data_sent: AtomicUsize,
}

impl MsgAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (tx, rx) = mpsc::channel(16);
        let a = Arc::new(Self {
            inbound: Mutex::new(Some(rx)),
            sent: AtomicUsize::new(0),
            data_sent: AtomicUsize::new(0),
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
    async fn send_data_message(&self, _c: ConnectionId, _m: DataMessage) -> RvResult<()> {
        self.data_sent.fetch_add(1, Ordering::SeqCst);
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

async fn drive_inbound(tx: &mpsc::Sender<AdapterEvent>, connid: ConnectionId) {
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
async fn adapter_message_event_persists_and_emits_message_received() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = MsgAdapter::new();
    orch.register(adapter).unwrap();
    let mut events = orch.subscribe_events();

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

    let conn = ConnectionId::new();
    let participant = ParticipantId::new();
    drive_inbound(&tx, conn.clone()).await;
    orch.route_inbound_connection(
        conn.clone(),
        InboundAction::Accept {
            session_id: sid,
            participant_id: participant.clone(),
        },
    )
    .await
    .unwrap();

    tx.send(AdapterEvent::Message {
        connection_id: conn.clone(),
        text: "I need to talk to Alice".into(),
    })
    .await
    .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(Event::MessageReceived {
                message_id,
                conversation_id,
                ..
            }) = events.recv().await
            {
                if conversation_id == cid {
                    return message_id;
                }
            }
        }
    })
    .await
    .unwrap();

    let page = orch
        .list_messages(cid, MessageFilter::default(), None)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].id, received);
    assert_eq!(
        page.messages[0].body,
        Bytes::from("I need to talk to Alice")
    );
    assert_eq!(page.messages[0].from_participant, participant);
    match &page.messages[0].origin {
        MessageOrigin::Connection(origin_conn) => assert_eq!(origin_conn, &conn),
        other => panic!("expected connection origin, got {other:?}"),
    }
}

#[tokio::test]
async fn adapter_data_message_event_preserves_binary_metadata_without_legacy_projection() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = MsgAdapter::new();
    orch.register(adapter).unwrap();
    let mut events = orch.subscribe_events();

    let conversation_id = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let session_id = orch
        .start_session(conversation_id.clone(), SessionMedium::TextChat, vec![])
        .await
        .unwrap();
    let connection_id = ConnectionId::new();
    drive_inbound(&tx, connection_id.clone()).await;
    orch.route_inbound_connection(
        connection_id.clone(),
        InboundAction::Accept {
            session_id,
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();

    let message = DataMessage {
        label: "bridgefu.context.v1".into(),
        content_type: "application/octet-stream".into(),
        bytes: Bytes::from_static(&[0, 0xff, 7, 42]),
        reliability: DataReliability::ReliableUnordered,
        message_id: MessageId::new(),
    };
    tx.send(AdapterEvent::DataMessage {
        connection_id: connection_id.clone(),
        message: message.clone(),
    })
    .await
    .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(Event::DataMessageReceived {
                connection_id: received_connection,
                message: received_message,
                ..
            }) = events.recv().await
            {
                if received_connection == connection_id {
                    return received_message;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(received, message);

    let page = orch
        .list_messages(conversation_id, MessageFilter::default(), None)
        .await
        .unwrap();
    assert!(
        page.messages.is_empty(),
        "application labels must not be projected into the legacy Message store"
    );
}

#[tokio::test]
async fn reserved_data_message_label_projects_exactly_once_into_legacy_messages() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = MsgAdapter::new();
    orch.register(adapter).unwrap();
    let mut events = orch.subscribe_events();

    let conversation_id = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let session_id = orch
        .start_session(conversation_id.clone(), SessionMedium::TextChat, vec![])
        .await
        .unwrap();
    let connection_id = ConnectionId::new();
    drive_inbound(&tx, connection_id.clone()).await;
    orch.route_inbound_connection(
        connection_id.clone(),
        InboundAction::Accept {
            session_id,
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();

    let message = DataMessage {
        label: "rvoip-chat".into(),
        content_type: "application/problem+json".into(),
        bytes: Bytes::from_static(br#"{"detail":"help"}"#),
        reliability: DataReliability::ReliableOrdered,
        message_id: MessageId::new(),
    };
    tx.send(AdapterEvent::DataMessage {
        connection_id,
        message: message.clone(),
    })
    .await
    .unwrap();

    let mut projected_events = 0;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(Event::MessageReceived {
                message_id,
                conversation_id: received_conversation,
                ..
            }) = events.recv().await
            {
                if received_conversation == conversation_id && message_id == message.message_id {
                    projected_events += 1;
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
        if matches!(
            event,
            Event::MessageReceived {
                ref message_id,
                conversation_id: ref received_conversation,
                ..
            } if message_id == &message.message_id && received_conversation == &conversation_id
        ) {
            projected_events += 1;
        }
    }
    assert_eq!(projected_events, 1);

    let page = orch
        .list_messages(conversation_id, MessageFilter::default(), None)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].id, message.message_id);
    assert_eq!(page.messages[0].body, message.bytes);
    assert_eq!(page.messages[0].content_type, ContentType::Json);
}

#[tokio::test]
async fn untracked_adapter_data_message_is_not_emitted() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = MsgAdapter::new();
    orch.register(adapter).unwrap();
    let mut events = orch.subscribe_events();

    tx.send(AdapterEvent::DataMessage {
        connection_id: ConnectionId::new(),
        message: DataMessage::reliable("bridgefu.context.v1", "text/plain", "ignored"),
    })
    .await
    .unwrap();

    assert!(
        tokio::time::timeout(Duration::from_millis(150), events.recv())
            .await
            .is_err(),
        "an untracked connection must not be able to inject an event"
    );
}

#[tokio::test]
async fn invalid_outbound_data_message_never_reaches_adapter() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx) = MsgAdapter::new();
    let counts = adapter.clone();
    orch.register(adapter).unwrap();
    let connection_id = ConnectionId::new();
    drive_inbound(&tx, connection_id.clone()).await;

    let invalid = DataMessage {
        label: String::new(),
        content_type: "text/plain".into(),
        bytes: Bytes::from_static(b"ignored"),
        reliability: DataReliability::ReliableOrdered,
        message_id: MessageId::new(),
    };
    assert!(orch
        .send_data_message(connection_id, invalid)
        .await
        .is_err());
    assert_eq!(counts.data_sent.load(Ordering::SeqCst), 0);
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
    let p2 = orch.list_messages(cid, filter, Some(cursor)).await.unwrap();
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

#[test]
fn cross_crate_data_message_event_exports_only_aggregate_safe_metadata() {
    let event = Event::DataMessageReceived {
        connection_id: ConnectionId::new(),
        message: DataMessage {
            label: "bridgefu.context.v1".into(),
            content_type: "application/json".into(),
            bytes: Bytes::from_static(b"sensitive-body-must-not-cross-the-bus"),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::from_string("customer-email@example.test"),
        },
        at: Utc::now(),
    };

    let wire = serde_json::to_string(&event.to_cross_crate()).unwrap();
    assert!(wire.contains("body_size"));
    assert!(!wire.contains("bridgefu.context.v1"));
    assert!(!wire.contains("application/json"));
    assert!(!wire.contains("customer-email@example.test"));
    assert!(!wire.contains("sensitive-body-must-not-cross-the-bus"));
}

// Unused-import suppressor for items that come up only in parts of the
// suite we exercise transitively.
#[allow(dead_code)]
fn _unused_imports_keep_alive(_: MuteDirection) {}
