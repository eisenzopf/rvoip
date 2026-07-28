//! Opt-in, real-localhost qualification for retained initial-INVITE planning.
//!
//! This deliberately runs longer than the production 90-second retained-plan
//! horizon at the release configuration of 100 active plans and 10 call
//! attempts per second. Every INVITE traverses the normal coordinator,
//! dialog, transaction, UDP transport, parser, and final-response path. The
//! raw localhost UAS returns 486 so calls terminate without allocating a
//! remote media peer while their authenticated late-response state remains
//! retained.

#![cfg(feature = "perf-tests")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_sip::{Config, UnifiedCoordinator};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::{Message, Method, StatusCode};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::task::JoinSet;
use tokio::time::{interval, sleep, timeout, MissedTickBehavior};

const ACTIVE_PLAN_CAPACITY: usize = 100;
const ATTEMPTS_PER_SECOND: usize = 10;
const QUALIFICATION_SECONDS: usize = 92;
const ATTEMPT_COUNT: usize = ATTEMPTS_PER_SECOND * QUALIFICATION_SECONDS;

async fn unused_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("reserve localhost UDP port")
        .local_addr()
        .expect("reserved localhost address")
        .port()
}

fn metric(snapshot: &serde_json::Value, pointer: &str) -> u64 {
    snapshot
        .pointer(pointer)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

async fn wait_for_metric(
    coordinator: &Arc<UnifiedCoordinator>,
    pointer: &str,
    expected: u64,
) -> serde_json::Value {
    timeout(Duration::from_secs(15), async {
        loop {
            let snapshot = coordinator.perf_diagnostic_snapshot().await;
            if metric(&snapshot, pointer) == expected {
                return snapshot;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {pointer}={expected}"))
}

#[ignore = "release qualification: runs a real 100-capacity/10-CPS localhost stack for >90s"]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn real_stack_capacity_100_at_10_cps_crosses_retention_horizon_and_drains_zero() {
    let uas_socket = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind qualification UAS"),
    );
    let uas_port = uas_socket
        .local_addr()
        .expect("qualification UAS address")
        .port();
    let invite_count = Arc::new(AtomicUsize::new(0));
    let ack_count = Arc::new(AtomicUsize::new(0));
    let uas_task = {
        let socket = Arc::clone(&uas_socket);
        let invites = Arc::clone(&invite_count);
        let acks = Arc::clone(&ack_count);
        tokio::spawn(async move {
            let mut buffer = vec![0_u8; 16 * 1024];
            loop {
                let (length, source) = socket
                    .recv_from(&mut buffer)
                    .await
                    .expect("qualification UAS receive");
                let Ok(Message::Request(request)) = parse_message(&buffer[..length]) else {
                    continue;
                };
                match request.method() {
                    Method::Invite => {
                        invites.fetch_add(1, Ordering::AcqRel);
                        let response = create_response(&request, StatusCode::BusyHere);
                        socket
                            .send_to(&Message::Response(response).to_bytes(), source)
                            .await
                            .expect("qualification UAS response");
                    }
                    Method::Ack => {
                        acks.fetch_add(1, Ordering::AcqRel);
                    }
                    _ => {}
                }
            }
        })
    };

    let uac_port = unused_udp_port().await;
    let coordinator = UnifiedCoordinator::new(
        Config::local("retention-qualification", uac_port)
            .with_server_capacity(ACTIVE_PLAN_CAPACITY)
            .with_signaling_only_media(9),
    )
    .await
    .expect("boot qualification UAC");
    sleep(Duration::from_millis(150)).await;

    let target = format!("sip:busy@127.0.0.1:{uas_port}");
    let from = format!("sip:caller@127.0.0.1:{uac_port}");
    let started = Instant::now();
    let mut cadence = interval(Duration::from_millis(1_000 / ATTEMPTS_PER_SECOND as u64));
    cadence.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut sends = JoinSet::new();
    for _ in 0..ATTEMPT_COUNT {
        cadence.tick().await;
        let coordinator = Arc::clone(&coordinator);
        let target = target.clone();
        let from = from.clone();
        sends.spawn(async move {
            timeout(
                Duration::from_secs(5),
                coordinator.invite(Some(from), target).send(),
            )
            .await
        });
    }

    let scheduled_elapsed = started.elapsed();
    let mut dispatch_successes = 0usize;
    let mut dispatch_failures = std::collections::BTreeMap::<String, usize>::new();
    while let Some(result) = sends.join_next().await {
        match result {
            Ok(Ok(Ok(_))) => dispatch_successes += 1,
            Ok(Ok(Err(error))) => {
                *dispatch_failures.entry(error.to_string()).or_default() += 1;
            }
            Ok(Err(_)) => {
                *dispatch_failures
                    .entry("dispatch timeout".to_string())
                    .or_default() += 1;
            }
            Err(error) => {
                *dispatch_failures
                    .entry(format!("dispatch task failed: {error}"))
                    .or_default() += 1;
            }
        }
    }

    assert!(
        scheduled_elapsed > Duration::from_secs(90),
        "the release qualification must cross the 90-second retention horizon"
    );
    let achieved_cps = ATTEMPT_COUNT as f64 / scheduled_elapsed.as_secs_f64();
    assert!(
        (9.5..=10.2).contains(&achieved_cps),
        "configured 10-CPS cadence achieved only {achieved_cps:.3} CPS"
    );
    assert_eq!(
        dispatch_successes, ATTEMPT_COUNT,
        "all scheduled calls must pass active-plan admission; failures={dispatch_failures:?}"
    );
    if !dispatch_failures.is_empty() {
        eprintln!(
            "qualification admission failure snapshot: {}",
            coordinator.perf_diagnostic_snapshot().await
        );
    }
    assert!(dispatch_failures.is_empty());

    timeout(Duration::from_secs(15), async {
        while invite_count.load(Ordering::Acquire) != ATTEMPT_COUNT
            || ack_count.load(Ordering::Acquire) != ATTEMPT_COUNT
        {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("all INVITE/486/ACK exchanges completed on localhost");

    let retained = wait_for_metric(
        &coordinator,
        "/dialog_manager/active_invite_failover_by_dialog",
        0,
    )
    .await;
    assert_eq!(
        retained.pointer("/config/server_call_capacity"),
        Some(&serde_json::json!(ACTIVE_PLAN_CAPACITY))
    );
    let retained_plans = metric(&retained, "/dialog_manager/invite_failover_plans");
    assert!(
        retained_plans > ACTIVE_PLAN_CAPACITY as u64,
        "the sample must observe retained history beyond active capacity, not a false early zero"
    );
    assert_eq!(
        metric(
            &retained,
            "/dialog_manager/invite_failover_plan_reservations"
        ),
        retained_plans
    );
    assert_eq!(
        metric(&retained, "/dialog_manager/invite_failover_plans_by_dialog"),
        retained_plans
    );
    assert_eq!(
        metric(
            &retained,
            "/dialog_manager/invite_failover_attempt_reservations"
        ),
        metric(&retained, "/dialog_manager/invite_failover_attempts")
    );
    assert_eq!(
        metric(
            &retained,
            "/dialog_manager/invite_failover_attempts_by_dialog"
        ),
        metric(&retained, "/dialog_manager/invite_failover_attempts")
    );
    assert!(
        metric(
            &retained,
            "/transaction_manager/retired_client_transactions"
        ) > 0,
        "the sample must include real retired client transaction routes"
    );
    eprintln!(
        "{}",
        serde_json::json!({
            "qualification": "sip-invite-retention-capacity",
            "active_plan_capacity": ACTIVE_PLAN_CAPACITY,
            "configured_cps": ATTEMPTS_PER_SECOND,
            "attempts": ATTEMPT_COUNT,
            "elapsed_seconds": scheduled_elapsed.as_secs_f64(),
            "achieved_cps": achieved_cps,
            "invite_packets": invite_count.load(Ordering::Acquire),
            "ack_packets": ack_count.load(Ordering::Acquire),
            "retained_plans": retained_plans,
            "retained_attempts": metric(
                &retained,
                "/dialog_manager/invite_failover_attempts"
            ),
            "retired_client_transactions": metric(
                &retained,
                "/transaction_manager/retired_client_transactions"
            ),
        })
    );

    coordinator
        .shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("qualification UAC drains");
    let drained = coordinator.perf_diagnostic_snapshot().await;
    for pointer in [
        "/dialog_manager/invite_failover_plans",
        "/dialog_manager/active_invite_failover_by_dialog",
        "/dialog_manager/invite_failover_plans_by_dialog",
        "/dialog_manager/invite_failover_attempts",
        "/dialog_manager/invite_failover_attempts_by_dialog",
        "/dialog_manager/invite_failover_plan_reservations",
        "/dialog_manager/invite_failover_attempt_reservations",
        "/transaction_manager/retired_client_transactions",
        "/transaction_manager/transaction_destinations",
    ] {
        assert_eq!(metric(&drained, pointer), 0, "final snapshot {pointer}");
    }

    uas_task.abort();
}
