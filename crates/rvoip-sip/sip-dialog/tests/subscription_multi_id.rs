//! RFC 6665 §4.5.2 — multi-subscription dialog disambiguation.
//!
//! Verifies that two subscriptions sharing a single dialog (same
//! Call-ID, From / To tags) but distinguished by the
//! `Event: pkg;id=<sid>` parameter do not clobber each other in the
//! `SubscriptionManager`'s shared `dialog_lookup`, and that inbound
//! NOTIFYs route to the matching subscription based on their own
//! `Event` header's `id` parameter.
//!
//! Closes SIP_API_DESIGN_2 Phase 6 deep plumbing for §10 verification
//! item #19 (`notify_subscription_id_routing`).

use std::net::SocketAddr;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;

use rvoip_sip_dialog::{
    dialog::{Dialog, DialogId},
    events::DialogEvent,
    subscription::SubscriptionManager,
};

use rvoip_sip_core::builder::headers::event::EventBuilderExt;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::event::EventType;
use rvoip_sip_core::{Method, Request};

const CALL_ID: &str = "shared-call@multi.example";
const FROM_TAG: &str = "from-tag-abc";

fn subscribe_with_event_id(id: &str) -> Request {
    SimpleRequestBuilder::new(Method::Subscribe, "sip:notifier@example.com")
        .unwrap()
        .from("Bob", "sip:subscriber@example.com", Some(FROM_TAG))
        .to("Alice", "sip:notifier@example.com", None)
        .call_id(CALL_ID)
        .cseq(1)
        .via("192.168.1.100:5060", "UDP", Some(&format!("branch-{}", id)))
        .event_id(EventType::Token("presence".to_string()), id)
        .expires(3600)
        .build()
}

#[tokio::test]
async fn two_subscriptions_with_different_event_ids_coexist_in_dialog_lookup() {
    let (event_tx, _event_rx) = mpsc::channel(100);
    let dialogs: Arc<DashMap<DialogId, Dialog>> = Arc::new(DashMap::new());
    let dialog_lookup: Arc<DashMap<String, DialogId>> = Arc::new(DashMap::new());
    let manager = SubscriptionManager::new(dialogs.clone(), dialog_lookup.clone(), event_tx);

    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    let local: SocketAddr = "192.168.1.200:5060".parse().unwrap();

    // First SUBSCRIBE with id=presence-1.
    let (resp1, dialog_id_1) = manager
        .handle_subscribe(subscribe_with_event_id("presence-1"), source, local)
        .await
        .expect("first SUBSCRIBE");
    assert_eq!(resp1.status_code(), 200);
    let dialog_id_1 = dialog_id_1.expect("first SUBSCRIBE must create a dialog");

    // Second SUBSCRIBE with same Call-ID/From-tag, different event id.
    let (resp2, dialog_id_2) = manager
        .handle_subscribe(subscribe_with_event_id("presence-2"), source, local)
        .await
        .expect("second SUBSCRIBE");
    assert_eq!(resp2.status_code(), 200);
    let dialog_id_2 = dialog_id_2.expect("second SUBSCRIBE must create a dialog");

    assert_ne!(
        dialog_id_1, dialog_id_2,
        "distinct event ids must produce distinct dialog IDs"
    );

    // Both dialog_lookup entries must survive — pre-fix, the second
    // insert clobbered the first because the lookup key was the
    // 3-tuple (call_id, local_tag, remote_tag) and was identical.
    assert_eq!(
        dialog_lookup.len(),
        2,
        "both subscriptions must be findable in dialog_lookup; got {} entry/entries",
        dialog_lookup.len()
    );

    // Both dialogs must coexist in the dialog store.
    assert!(dialogs.get(&dialog_id_1).is_some());
    assert!(dialogs.get(&dialog_id_2).is_some());
}

/// Inbound NOTIFY (subscriber-side path) routes to the correct
/// dialog based on the Event header's `id` parameter, even when two
/// subscriptions share a Call-ID / tag pair.
///
/// The test pre-populates `dialog_lookup` with the subscriber-side
/// keys directly (matching the `(call_id, to_tag, from_tag, event_id)`
/// lookup format `handle_notify` uses) and verifies the manager
/// finds the right entry. This exercises the lookup-key plumbing
/// independent of the asymmetric tag-ordering between SUBSCRIBE (UAS)
/// dialog creation and NOTIFY (UAC) dialog lookup — that asymmetry is
/// a pre-existing concern of the manager's two-role design and is
/// out of scope for the Phase 6 plumbing fix.
#[tokio::test]
async fn inbound_notify_disambiguates_by_event_id_when_call_id_and_tags_match() {
    let (event_tx, mut event_rx) = mpsc::channel(100);
    let dialogs: Arc<DashMap<DialogId, Dialog>> = Arc::new(DashMap::new());
    let dialog_lookup: Arc<DashMap<String, DialogId>> = Arc::new(DashMap::new());
    let manager = SubscriptionManager::new(dialogs.clone(), dialog_lookup.clone(), event_tx);

    // Two subscriber-side dialogs that differ only in `event_id`. We
    // populate the lookup map and the dialog store directly, mimicking
    // the UAC-side handler that would have set them up after sending
    // SUBSCRIBE and receiving 200 OK.
    let to_tag = "local-uac-tag";
    let from_tag = "remote-notifier-tag";

    fn make_dialog(call_id: &str, local_tag: &str, remote_tag: &str, event_id: &str) -> Dialog {
        use rvoip_sip_core::types::Uri;
        let local_uri: Uri = "sip:subscriber@example.com".parse().unwrap();
        let remote_uri: Uri = "sip:notifier@example.com".parse().unwrap();
        let mut d = Dialog::new(
            call_id.to_string(),
            local_uri,
            remote_uri,
            Some(local_tag.to_string()),
            Some(remote_tag.to_string()),
            true, // initiator
        );
        d.event_package = Some("presence".to_string());
        d.event_id = Some(event_id.to_string());
        d
    }

    let id_a = DialogId::new();
    let mut dlg_a = make_dialog(CALL_ID, to_tag, from_tag, "presence-1");
    dlg_a.id = id_a.clone();
    dialogs.insert(id_a.clone(), dlg_a);
    dialog_lookup.insert(
        format!("{}:{}:{}:{}", CALL_ID, to_tag, from_tag, "presence-1"),
        id_a.clone(),
    );

    let id_b = DialogId::new();
    let mut dlg_b = make_dialog(CALL_ID, to_tag, from_tag, "presence-2");
    dlg_b.id = id_b.clone();
    dialogs.insert(id_b.clone(), dlg_b);
    dialog_lookup.insert(
        format!("{}:{}:{}:{}", CALL_ID, to_tag, from_tag, "presence-2"),
        id_b.clone(),
    );

    let source: SocketAddr = "192.168.1.100:5060".parse().unwrap();

    // NOTIFY for id=presence-1 — must route to dialog A.
    fn notify_for_event_id(to_tag: &str, from_tag: &str, event_id: &str) -> Request {
        SimpleRequestBuilder::new(Method::Notify, "sip:subscriber@example.com")
            .unwrap()
            .from("Alice", "sip:notifier@example.com", Some(from_tag))
            .to("Bob", "sip:subscriber@example.com", Some(to_tag))
            .call_id(CALL_ID)
            .cseq(1)
            .via("192.168.1.200:5060", "UDP", Some("branch-notify"))
            .event_id(EventType::Token("presence".to_string()), event_id)
            .subscription_state("active")
            .build()
    }

    let _ = manager
        .handle_notify(notify_for_event_id(to_tag, from_tag, "presence-1"), source)
        .await
        .expect("notify presence-1");

    let observed = tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv())
        .await
        .expect("NotifyReceived for presence-1")
        .expect("recv");
    match observed {
        DialogEvent::NotifyReceived { dialog_id, .. } => {
            assert_eq!(
                dialog_id, id_a,
                "NOTIFY for presence-1 must route to dialog A, not dialog B"
            );
        }
        other => panic!("expected NotifyReceived; got {other:?}"),
    }

    let _ = manager
        .handle_notify(notify_for_event_id(to_tag, from_tag, "presence-2"), source)
        .await
        .expect("notify presence-2");

    let observed = tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv())
        .await
        .expect("NotifyReceived for presence-2")
        .expect("recv");
    match observed {
        DialogEvent::NotifyReceived { dialog_id, .. } => {
            assert_eq!(
                dialog_id, id_b,
                "NOTIFY for presence-2 must route to dialog B, not dialog A"
            );
        }
        other => panic!("expected NotifyReceived; got {other:?}"),
    }
}
