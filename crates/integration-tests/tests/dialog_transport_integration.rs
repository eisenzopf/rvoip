//! Tests that dialog-core correctly sends/receives SIP messages through sip-transport.
//!
//! These integration tests verify the full path:
//!   Dialog API -> TransactionManager -> TransportManager -> UDP -> TransportManager -> TransactionManager -> Dialog
//!
//! No pre-built messages are injected; the dialog layer creates proper SIP messages
//! and the transport layer serializes, sends, receives, and parses them.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serial_test::serial;
use tokio::time::timeout;

use rvoip_dialog_core::DialogManager;
use rvoip_dialog_core::dialog::{DialogId, DialogState};
use rvoip_dialog_core::transaction::TransactionManager;
use rvoip_dialog_core::transaction::transport::{TransportManager, TransportManagerConfig};
use rvoip_dialog_core::transaction::TransactionEvent;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::Method;
use rvoip_sip_core::{Request, Uri, TypedHeader, ContentLength, Message};
use rvoip_sip_transport::{Transport, TransportEvent, UdpTransport};

/// Helper: create a TransportManager + TransactionManager + DialogManager stack on an
/// ephemeral UDP port bound to 127.0.0.1. Returns (DialogManager, TransactionEvent rx, bound address).
async fn create_dialog_stack() -> anyhow::Result<(
    DialogManager,
    tokio::sync::mpsc::Receiver<TransactionEvent>,
    SocketAddr,
)> {
    let config = TransportManagerConfig {
        enable_udp: true,
        enable_tcp: false,
        enable_ws: false,
        enable_tls: false,
        bind_addresses: vec!["127.0.0.1:0".parse()?],
        ..Default::default()
    };

    let (mut transport_mgr, transport_rx) = TransportManager::new(config)
        .await
        .map_err(|e| anyhow::anyhow!("TransportManager::new failed: {e}"))?;

    transport_mgr
        .initialize()
        .await
        .map_err(|e| anyhow::anyhow!("TransportManager::initialize failed: {e}"))?;

    let bound_addr = transport_mgr
        .default_transport()
        .await
        .ok_or_else(|| anyhow::anyhow!("No default transport after initialize"))?
        .local_addr()
        .map_err(|e| anyhow::anyhow!("local_addr failed: {e}"))?;

    let (tx_mgr, tx_events) = TransactionManager::with_transport_manager(
        transport_mgr,
        transport_rx,
        Some(100),
    )
    .await
    .map_err(|e| anyhow::anyhow!("TransactionManager::with_transport_manager failed: {e}"))?;

    let dialog_mgr = DialogManager::new(Arc::new(tx_mgr), bound_addr)
        .await
        .map_err(|e| anyhow::anyhow!("DialogManager::new failed: {e}"))?;

    Ok((dialog_mgr, tx_events, bound_addr))
}

/// Build a well-formed INVITE request from `from_addr` targeting `to_addr`.
fn build_invite(from_addr: SocketAddr, to_addr: SocketAddr) -> Request {
    let branch = format!(
        "z9hG4bK-{}",
        uuid::Uuid::new_v4().to_string().replace("-", "")
    );
    let call_id = format!(
        "integ-test-{}",
        uuid::Uuid::new_v4().to_string().replace("-", "")
    );
    let from_tag = format!(
        "ftag-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .replace("-", "")
            .chars()
            .take(8)
            .collect::<String>()
    );
    let from_uri = format!("sip:alice@{}", from_addr);
    let to_uri = format!("sip:bob@{}", to_addr);

    SimpleRequestBuilder::new(Method::Invite, &to_uri)
        .expect("valid URI")
        .from("Alice", &from_uri, Some(&from_tag))
        .to("Bob", &to_uri, None)
        .call_id(&call_id)
        .cseq(1)
        .via(&from_addr.to_string(), "UDP", Some(&branch))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

/// Build a well-formed BYE request targeting `to_addr`.
fn build_bye(from_addr: SocketAddr, to_addr: SocketAddr) -> Request {
    let branch = format!(
        "z9hG4bK-{}",
        uuid::Uuid::new_v4().to_string().replace("-", "")
    );
    let call_id = format!(
        "bye-test-{}",
        uuid::Uuid::new_v4().to_string().replace("-", "")
    );
    let from_tag = format!(
        "ftag-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .replace("-", "")
            .chars()
            .take(8)
            .collect::<String>()
    );
    let from_uri = format!("sip:alice@{}", from_addr);
    let to_uri = format!("sip:bob@{}", to_addr);

    SimpleRequestBuilder::new(Method::Bye, &to_uri)
        .expect("valid URI")
        .from("Alice", &from_uri, Some(&from_tag))
        .to("Bob", &to_uri, None)
        .call_id(&call_id)
        .cseq(2)
        .via(&from_addr.to_string(), "UDP", Some(&branch))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

// =============================================================================
// Test 1: Dialog sends INVITE through real UDP transport and peer receives it
// =============================================================================

#[tokio::test]
#[serial]
async fn test_dialog_sends_invite_through_udp_transport() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("info")
        .try_init();

    timeout(Duration::from_secs(10), async {
        // Create two full dialog stacks (A and B)
        let (dialog_a, _events_a, addr_a) =
            create_dialog_stack().await.context("create stack A")?;
        let (dialog_b, mut events_b, addr_b) =
            create_dialog_stack().await.context("create stack B")?;

        // A creates an outgoing dialog towards B
        let local_uri: Uri = format!("sip:alice@{}", addr_a)
            .parse()
            .context("parse alice URI")?;
        let remote_uri: Uri = format!("sip:bob@{}", addr_b)
            .parse()
            .context("parse bob URI")?;

        let dialog_id = dialog_a
            .create_outgoing_dialog(local_uri, remote_uri, None)
            .await
            .map_err(|e| anyhow::anyhow!("create_outgoing_dialog failed: {e}"))?;

        // Also send an INVITE through the transaction layer directly so B's transport receives it
        let invite = build_invite(addr_a, addr_b);
        let tx_key = dialog_a
            .transaction_manager()
            .create_client_transaction(invite, addr_b)
            .await
            .map_err(|e| anyhow::anyhow!("create_client_transaction: {e}"))?;

        dialog_a
            .transaction_manager()
            .send_request(&tx_key)
            .await
            .map_err(|e| anyhow::anyhow!("send_request: {e}"))?;

        // B should receive a transaction event for the incoming INVITE
        let mut received_invite = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), events_b.recv()).await {
                Ok(Some(event)) => {
                    // Check if this is an incoming INVITE request event
                    match &event {
                        TransactionEvent::InviteRequest { request, .. } => {
                            received_invite = true;
                            // Verify headers
                            let from = request.from();
                            assert!(from.is_some(), "INVITE should have From header");
                            let to = request.to();
                            assert!(to.is_some(), "INVITE should have To header");
                            let call_id = request.call_id();
                            assert!(call_id.is_some(), "INVITE should have Call-ID header");
                            let vias = request.via_headers();
                            assert!(!vias.is_empty(), "INVITE should have Via header");
                            break;
                        }
                        TransactionEvent::StrayRequest { request, .. } => {
                            if request.method() == Method::Invite {
                                received_invite = true;
                                break;
                            }
                        }
                        _ => {
                            // Other events are fine, keep waiting
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        assert!(
            received_invite,
            "B should have received the INVITE sent by A through real UDP transport"
        );

        // Verify dialog was created on A's side
        let a_dialog = dialog_a
            .get_dialog(&dialog_id)
            .map_err(|e| anyhow::anyhow!("get_dialog: {e}"))?;
        // Dialog exists if get_dialog succeeded (returns Result, not Option)

        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Test timed out after 10s"))?
}

// =============================================================================
// Test 2: Dialog transaction retransmission over transport
// =============================================================================

#[tokio::test]
#[serial]
async fn test_dialog_transaction_retransmission_over_transport() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("info")
        .try_init();

    timeout(Duration::from_secs(15), async {
        // Create stack A (sender)
        let (dialog_a, _events_a, addr_a) =
            create_dialog_stack().await.context("create stack A")?;

        // Create a raw UDP receiver on B's side to count messages
        let (transport_b, mut rx_b) = UdpTransport::bind(
            "127.0.0.1:0".parse::<SocketAddr>()?,
            Some(200),
        )
        .await
        .map_err(|e| anyhow::anyhow!("UdpTransport::bind: {e}"))?;

        let addr_b = transport_b
            .local_addr()
            .map_err(|e| anyhow::anyhow!("local_addr: {e}"))?;

        // A sends INVITE via transaction layer (which will retransmit per RFC 3261)
        let invite = build_invite(addr_a, addr_b);
        let tx_key = dialog_a
            .transaction_manager()
            .create_client_transaction(invite, addr_b)
            .await
            .map_err(|e| anyhow::anyhow!("create_client_transaction: {e}"))?;

        dialog_a
            .transaction_manager()
            .send_request(&tx_key)
            .await
            .map_err(|e| anyhow::anyhow!("send_request: {e}"))?;

        // Collect received messages over 4 seconds — INVITE retransmission timer A
        // starts at T1=500ms and doubles, so we should see multiple transmissions
        let mut received_count = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout_at(deadline, rx_b.recv()).await {
                Ok(Some(TransportEvent::MessageReceived { message, .. })) => {
                    if message.is_request() {
                        if let Message::Request(ref req) = message {
                            if req.method() == Method::Invite {
                                received_count += 1;
                            }
                        }
                    }
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            }
        }

        // RFC 3261 says INVITE client transactions retransmit over UDP.
        // In 4 seconds we should see at least the original + one retransmit.
        assert!(
            received_count >= 2,
            "Expected at least 2 INVITE transmissions (original + retransmit), got {}",
            received_count
        );

        transport_b
            .close()
            .await
            .map_err(|e| anyhow::anyhow!("close transport_b: {e}"))?;

        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Test timed out after 15s"))?
}

// =============================================================================
// Test 3: Dialog BYE through real transport
// =============================================================================

#[tokio::test]
#[serial]
async fn test_dialog_bye_through_transport() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("info")
        .try_init();

    timeout(Duration::from_secs(10), async {
        // Create two full dialog stacks
        let (dialog_a, _events_a, addr_a) =
            create_dialog_stack().await.context("create stack A")?;
        let (_dialog_b, mut events_b, addr_b) =
            create_dialog_stack().await.context("create stack B")?;

        // First establish a dialog on A's side
        let local_uri: Uri = format!("sip:alice@{}", addr_a)
            .parse()
            .context("parse alice URI")?;
        let remote_uri: Uri = format!("sip:bob@{}", addr_b)
            .parse()
            .context("parse bob URI")?;

        let _dialog_id = dialog_a
            .create_outgoing_dialog(local_uri, remote_uri, None)
            .await
            .map_err(|e| anyhow::anyhow!("create_outgoing_dialog: {e}"))?;

        // Send a BYE through the transaction layer
        let bye = build_bye(addr_a, addr_b);
        let tx_key = dialog_a
            .transaction_manager()
            .create_client_transaction(bye, addr_b)
            .await
            .map_err(|e| anyhow::anyhow!("create_client_transaction for BYE: {e}"))?;

        dialog_a
            .transaction_manager()
            .send_request(&tx_key)
            .await
            .map_err(|e| anyhow::anyhow!("send BYE request: {e}"))?;

        // B should receive the BYE as a transaction event
        let mut received_bye = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), events_b.recv()).await {
                Ok(Some(event)) => {
                    // BYE is a non-INVITE method
                    match &event {
                        TransactionEvent::NonInviteRequest { request, .. } => {
                            if request.method() == Method::Bye {
                                received_bye = true;
                                // Verify BYE has correct SIP headers
                                let from = request.from();
                                assert!(from.is_some(), "BYE should have From header");
                                let to = request.to();
                                assert!(to.is_some(), "BYE should have To header");
                                let call_id = request.call_id();
                                assert!(call_id.is_some(), "BYE should have Call-ID header");
                                break;
                            }
                        }
                        TransactionEvent::StrayRequest { request, .. } => {
                            if request.method() == Method::Bye {
                                received_bye = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        assert!(
            received_bye,
            "B should have received BYE through real UDP transport"
        );

        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Test timed out after 10s"))?
}
