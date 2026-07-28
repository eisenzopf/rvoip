//! Two-coordinator established-call scaffolding.
//!
//! `boot_callback_receiver` boots a `CallbackPeer<AutoAccept>` so
//! inbound INVITEs are answered automatically. `boot_unified_caller`
//! boots a plain `UnifiedCoordinator` for the alice side. `establish_call`
//! wires both together: alice INVITEs bob, the test waits for
//! `CallAnswered`, and an [`EstablishedCall`] is returned with both
//! event receivers ready for mid-dialog wire-trace assertions.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallbackPeer, ShutdownHandle};
use rvoip_sip::api::events::{CallId, Event};
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

use super::handlers::AutoAccept;
use super::traces::{receiver_config, wait_for_inbound_method};

/// Bob side of an established call: a `CallbackPeer` driven by
/// [`AutoAccept`] running in a background task.
pub struct CallbackReceiver {
    pub coord: Arc<UnifiedCoordinator>,
    pub task: tokio::task::JoinHandle<()>,
    pub shutdown: ShutdownHandle,
}

impl CallbackReceiver {
    /// Stops the background callback peer and joins its task.
    pub async fn shutdown(self) {
        self.shutdown.shutdown();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.task).await;
    }
}

/// Boots a `CallbackPeer` whose handler auto-accepts every inbound
/// INVITE. The returned `coord` is the underlying `UnifiedCoordinator`
/// (use `coord.events()` to subscribe to bob's wire-trace stream).
pub async fn boot_callback_receiver(port: u16, name: &str) -> CallbackReceiver {
    boot_callback_receiver_with_handler(AutoAccept, port, name).await
}

pub async fn boot_callback_receiver_with_handler<H: CallHandler>(
    handler: H,
    port: u16,
    name: &str,
) -> CallbackReceiver {
    let bob = CallbackPeer::new(handler, receiver_config(name, port))
        .await
        .expect("callback peer");
    let coord = bob.coordinator().clone();
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    CallbackReceiver {
        coord,
        task,
        shutdown,
    }
}

/// Boots a plain `UnifiedCoordinator` configured with sip_trace enabled.
/// The standard alice side of a §10 two-coordinator test.
pub async fn boot_unified_caller(port: u16, name: &str) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(receiver_config(name, port))
        .await
        .expect("unified caller coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;
    coord
}

/// Boots a plain `UnifiedCoordinator` with a caller-supplied [`Config`]
/// (e.g. with `auto_emit_extra_headers` populated). Sleeps briefly so
/// the transport is bound before the caller dispatches a request.
pub async fn boot_unified_caller_with_config(cfg: Config) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("unified caller coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;
    coord
}

/// Polls `events` for a `CallAnswered` matching `target_call_id`,
/// dropping any other events along the way.
pub async fn wait_for_call_answered(
    events: &mut EventReceiver,
    target_call_id: &CallId,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return false,
            Ok(Some(Event::CallAnswered { call_id, .. })) if &call_id == target_call_id => {
                return true;
            }
            Ok(Some(_)) => continue,
        }
    }
}

/// Established INVITE → 200 OK → ACK call: alice originated, bob answered.
/// Mid-dialog requests can be driven from `alice` against `call_id`; the
/// receivers stay live so wire-trace assertions can inspect either leg.
pub struct EstablishedCall {
    pub alice: Arc<UnifiedCoordinator>,
    pub bob: CallbackReceiver,
    pub call_id: CallId,
    pub alice_events: EventReceiver,
    pub bob_events: EventReceiver,
    pub alice_port: u16,
    pub bob_port: u16,
}

impl EstablishedCall {
    /// Sends a BYE (without extras) and shuts down bob's background task.
    /// The standard `Drop`-like teardown for §10 tests.
    pub async fn teardown(self) {
        let _ = self.alice.bye(&self.call_id).send().await;
        // Allow the BYE to settle before shutting down bob.
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.bob.shutdown().await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Boots alice + bob, INVITEs from alice to bob's port, waits for
/// `CallAnswered`, drains bob's inbound INVITE trace so a subsequent
/// in-dialog assertion doesn't false-match it, and returns the live
/// [`EstablishedCall`] handle.
///
/// `port_a` and `port_b` MUST be distinct and free.
pub async fn establish_call(port_a: u16, port_b: u16) -> EstablishedCall {
    establish_call_with_handler(AutoAccept, port_a, port_b).await
}

pub async fn establish_call_with_handler<H: CallHandler>(
    handler: H,
    port_a: u16,
    port_b: u16,
) -> EstablishedCall {
    let bob = boot_callback_receiver_with_handler(handler, port_b, "bob").await;
    let mut bob_events = bob.coord.events().await.expect("bob events");

    let alice = boot_unified_caller(port_a, "alice").await;
    let mut alice_events = alice.events().await.expect("alice events");

    let target = format!("sip:bob@127.0.0.1:{}", port_b);
    let call_id = alice
        .invite(Some(format!("sip:alice@127.0.0.1:{}", port_a)), target)
        .send()
        .await
        .expect("invite().send()");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "alice did not see CallAnswered"
    );
    // Drain bob's INVITE trace so subsequent in-dialog assertions
    // (BYE/INFO/REFER/NOTIFY/UPDATE) don't false-match against it.
    let _ = wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(2)).await;

    EstablishedCall {
        alice,
        bob,
        call_id,
        alice_events,
        bob_events,
        alice_port: port_a,
        bob_port: port_b,
    }
}
