//! `Config::options_keepalive_targets`: periodic out-of-dialog OPTIONS pings.
//!
//! An in-process raw-UDP peer answers OPTIONS with 200 OK while "up" and
//! drops them while "down". The test asserts that pings arrive on the
//! configured interval carrying `Config::contact_uri` as Contact, that
//! `Event::PeerReachabilityChanged` reports up -> down -> up exactly on the
//! transitions, and that pings stop after shutdown.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};

use rvoip_sip::{Config, Event, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const CONTACT: &str = "sip:sbc.example.test:5061;transport=tls";

struct MockPeer {
    answering: AtomicBool,
    pings: AtomicUsize,
    contacts: Mutex<Vec<Option<String>>>,
}

async fn run_mock_peer(sock: Arc<UdpSocket>, peer: Arc<MockPeer>) {
    let mut buf = vec![0u8; 8192];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else {
            return;
        };
        let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
            continue;
        };
        if request.method() != Method::Options {
            continue;
        }
        peer.pings.fetch_add(1, Ordering::SeqCst);
        peer.contacts
            .lock()
            .unwrap()
            .push(request.raw_header_value(&HeaderName::Contact));
        if peer.answering.load(Ordering::SeqCst) {
            let response = create_response(&request, StatusCode::Ok);
            let _ = sock
                .send_to(&Message::Response(response).to_bytes(), from)
                .await;
        }
    }
}

async fn next_reachability(peer: &mut StreamPeer) -> (String, bool, Option<u16>) {
    loop {
        match peer.next_event().await {
            Some(Event::PeerReachabilityChanged {
                target,
                reachable,
                status_code,
            }) => return (target, reachable, status_code),
            Some(_) => {}
            None => panic!("event stream closed before a reachability event"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn options_keepalive_reports_reachability_transitions_and_stops_on_shutdown() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("mock bind"));
    let target = format!("sip:127.0.0.1:{}", sock.local_addr().unwrap().port());
    let mock = Arc::new(MockPeer {
        answering: AtomicBool::new(true),
        pings: AtomicUsize::new(0),
        contacts: Mutex::new(Vec::new()),
    });
    let mock_task = tokio::spawn(run_mock_peer(sock, Arc::clone(&mock)));

    let mut config = Config::local("sbc", 0);
    config.media_port_start = 41200;
    config.media_port_end = 41300;
    config.contact_uri = Some(CONTACT.to_string());
    config.options_keepalive_targets = vec![target.clone()];
    config.options_keepalive_interval_secs = 1;
    let mut peer = StreamPeer::with_config(config).await.expect("peer");

    let up = timeout(Duration::from_secs(5), next_reachability(&mut peer))
        .await
        .expect("first reachability event");
    assert_eq!(up, (target.clone(), true, Some(200)));

    // Repeated 200s publish nothing new; the next event is the outage.
    sleep(Duration::from_millis(1500)).await;
    mock.answering.store(false, Ordering::SeqCst);
    let down = timeout(Duration::from_secs(8), next_reachability(&mut peer))
        .await
        .expect("unreachable event");
    assert_eq!(down, (target.clone(), false, None));

    mock.answering.store(true, Ordering::SeqCst);
    let back = timeout(Duration::from_secs(8), next_reachability(&mut peer))
        .await
        .expect("recovery event");
    assert_eq!(back, (target.clone(), true, Some(200)));

    let pings = mock.pings.load(Ordering::SeqCst);
    assert!(pings >= 4, "expected periodic pings, saw {pings}");
    {
        let contacts = mock.contacts.lock().unwrap();
        assert!(
            contacts
                .iter()
                .all(|contact| contact.as_deref().is_some_and(|c| c.contains(CONTACT))),
            "every ping must carry Config::contact_uri: {contacts:?}"
        );
    }

    peer.shutdown().await.expect("shutdown");
    sleep(Duration::from_millis(500)).await;
    let after_shutdown = mock.pings.load(Ordering::SeqCst);
    sleep(Duration::from_millis(2500)).await;
    assert_eq!(
        mock.pings.load(Ordering::SeqCst),
        after_shutdown,
        "pings must stop after shutdown"
    );
    mock_task.abort();
}
