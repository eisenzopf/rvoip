//! Retained-object invariants for resilience and perf-gate tests.
//!
//! These helpers intentionally mirror the retained-object definition used by
//! the soak harness. Short-lived terminal lifecycle evidence is allowed, but
//! per-call owners, transaction runners, cleanup work, and media/dialog maps
//! must converge to zero after a terminal drain.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::unified::UnifiedCoordinator;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WatchdogCounters {
    pub armed: u64,
    pub disarmed: u64,
    pub fired: u64,
    pub transition_failed: u64,
    pub release_completed: u64,
    pub release_failed: u64,
}

impl WatchdogCounters {
    pub fn delta_from(self, before: Self) -> Self {
        Self {
            armed: self.armed.saturating_sub(before.armed),
            disarmed: self.disarmed.saturating_sub(before.disarmed),
            fired: self.fired.saturating_sub(before.fired),
            transition_failed: self
                .transition_failed
                .saturating_sub(before.transition_failed),
            release_completed: self
                .release_completed
                .saturating_sub(before.release_completed),
            release_failed: self.release_failed.saturating_sub(before.release_failed),
        }
    }
}

pub async fn watchdog_counters(coord: &Arc<UnifiedCoordinator>) -> WatchdogCounters {
    watchdog_counters_from_snapshot(&coord.perf_diagnostic_snapshot().await)
}

pub fn watchdog_counters_from_snapshot(snapshot: &Value) -> WatchdogCounters {
    WatchdogCounters {
        armed: metric(snapshot, "/cleanup/setup_teardown_watchdog/armed"),
        disarmed: metric(snapshot, "/cleanup/setup_teardown_watchdog/disarmed"),
        fired: metric(snapshot, "/cleanup/setup_teardown_watchdog/fired"),
        transition_failed: metric(
            snapshot,
            "/cleanup/setup_teardown_watchdog/transition_failed",
        ),
        release_completed: metric(
            snapshot,
            "/cleanup/setup_teardown_watchdog/release_completed",
        ),
        release_failed: metric(snapshot, "/cleanup/setup_teardown_watchdog/release_failed"),
    }
}

pub fn assert_no_watchdog_fallback(before: WatchdogCounters, after: WatchdogCounters) {
    let delta = after.delta_from(before);
    assert_eq!(
        delta.fired, 0,
        "setup/teardown watchdog fired during a normal resilient flow: {:?}",
        delta
    );
    assert_eq!(
        delta.transition_failed, 0,
        "setup/teardown watchdog transition failed during a normal resilient flow: {:?}",
        delta
    );
    assert_eq!(
        delta.release_failed, 0,
        "setup/teardown watchdog release failed during a normal resilient flow: {:?}",
        delta
    );
}

pub async fn assert_single_endpoint_released(
    label: &str,
    coord: &Arc<UnifiedCoordinator>,
    timeout: Duration,
) -> Value {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let snapshot = coord.perf_diagnostic_snapshot().await;
        let retained =
            endpoint_retained_total(&snapshot) + endpoint_global_retained_total(&snapshot);
        let sample = json!({
            "label": label,
            "retained_total": retained,
            "endpoint": endpoint_summary(&snapshot),
        });
        if retained == 0 {
            return sample;
        }

        if tokio::time::Instant::now() >= deadline {
            panic!(
                "{label}: endpoint retained objects after drain:\n{}",
                serde_json::to_string_pretty(&sample).unwrap()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn assert_pair_released(
    label: &str,
    alice: &Arc<UnifiedCoordinator>,
    bob: &Arc<UnifiedCoordinator>,
    timeout: Duration,
) -> Value {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let alice_snapshot = alice.perf_diagnostic_snapshot().await;
        let bob_snapshot = bob.perf_diagnostic_snapshot().await;
        let retained = endpoint_retained_total(&alice_snapshot)
            + endpoint_retained_total(&bob_snapshot)
            + endpoint_global_retained_total(&alice_snapshot);
        let sample = json!({
            "label": label,
            "retained_total": retained,
            "alice": endpoint_summary(&alice_snapshot),
            "bob": endpoint_summary(&bob_snapshot),
        });
        if retained == 0 {
            return sample;
        }

        if tokio::time::Instant::now() >= deadline {
            panic!(
                "{label}: endpoint pair retained objects after drain:\n{}",
                serde_json::to_string_pretty(&sample).unwrap()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn endpoint_summary(snapshot: &Value) -> Value {
    json!({
        "session_store": snapshot["session_store"].clone(),
        "session_registry": snapshot["session_registry"].clone(),
        "lifecycle": snapshot["lifecycle"].clone(),
        "state_machine_helpers": snapshot["state_machine_helpers"].clone(),
        "transaction_manager": snapshot["transaction_manager"].clone(),
        "dialog_manager": snapshot["dialog_manager"].clone(),
        "dialog_adapter": snapshot["dialog_adapter"].clone(),
        "media_adapter": snapshot["media_adapter"].clone(),
        "sip_dialog_diagnostics": snapshot["sip_dialog_diagnostics"].clone(),
        "cleanup": snapshot["cleanup"].clone(),
    })
}

fn endpoint_retained_total(snapshot: &Value) -> u64 {
    const POINTERS: &[&str] = &[
        "/session_store/total",
        "/session_registry/sessions",
        "/state_machine_helpers/active_sessions",
        "/state_machine_helpers/subscriber_sessions",
        "/dialog_adapter/session_to_dialog",
        "/dialog_adapter/dialog_to_session",
        "/dialog_adapter/callid_to_session",
        "/dialog_adapter/outgoing_invite_tx",
        "/dialog_adapter/registration_refresh_tasks",
        "/lifecycle/expired_terminal_entries",
        "/transaction_manager/total",
        "/transaction_manager/terminated_transactions",
        "/transaction_manager/server_invite_dialog_index",
        "/transaction_manager/server_invite_dialog_keys_by_tx",
        "/transaction_manager/invite_2xx_response_cache",
        "/transaction_manager/invite_2xx_response_due_queue",
        "/transaction_manager/transaction_destinations",
        "/transaction_manager/subscriber_to_transactions",
        "/transaction_manager/transaction_to_subscribers",
        "/transaction_manager/pending_inbound_bytes",
        "/transaction_manager/pending_inbound_timing",
        "/dialog_manager/dialogs",
        "/dialog_manager/dialog_lookup",
        "/dialog_manager/early_dialog_lookup",
        "/dialog_manager/terminated_bye_lookup",
        "/dialog_manager/terminated_bye_deadlines",
        "/dialog_manager/transaction_to_dialog",
        "/dialog_manager/transaction_dialog_route_hash",
        "/dialog_manager/dialog_invite_transactions",
        "/dialog_manager/dialog_server_transactions",
        "/dialog_manager/pending_response_transaction_by_dialog",
        "/dialog_manager/session_to_dialog",
        "/dialog_manager/dialog_to_session",
        "/dialog_manager/reliable_provisional_tasks",
        "/dialog_manager/session_refresh_tasks",
        "/dialog_manager/outbound_flows",
        "/dialog_manager/outbound_flow_tasks",
        "/dialog_manager/flow_by_destination",
        "/dialog_manager/flow_by_aor",
        "/media_adapter/session_to_dialog",
        "/media_adapter/dialog_to_session",
        "/media_adapter/media_sessions",
        "/media_adapter/audio_receivers",
        "/media_adapter/pending_srtp_offerers",
        "/media_adapter/negotiated_srtp",
        "/media_adapter/audio_mixers",
        "/media_adapter/controller/sessions",
        "/media_adapter/controller/rtp_sessions",
        "/media_adapter/controller/session_to_media",
        "/media_adapter/controller/media_to_session",
        "/media_adapter/controller/audio_frame_callbacks",
        "/media_adapter/controller/dtmf_callbacks",
        "/media_adapter/controller/bridge_partners",
        "/media_adapter/controller/cn_gate_state",
        "/media_adapter/controller/advanced_processors",
        "/media_adapter/controller/media_directions",
        "/cleanup/active_total",
    ];

    POINTERS
        .iter()
        .map(|pointer| metric(snapshot, pointer))
        .sum()
}

fn endpoint_global_retained_total(snapshot: &Value) -> u64 {
    const POINTERS: &[&str] = &[
        "/sip_dialog_diagnostics/transaction_runner/active",
        "/sip_dialog_diagnostics/transaction_cleanup/in_flight",
    ];

    POINTERS
        .iter()
        .map(|pointer| metric(snapshot, pointer))
        .sum()
}

fn metric(snapshot: &Value, pointer: &str) -> u64 {
    snapshot
        .pointer(pointer)
        .and_then(Value::as_u64)
        .unwrap_or(0)
}
