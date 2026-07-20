use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::trace;

use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::TransportRoute;

use crate::transaction::timer::{TimerManager, TimerSettings, TimerType};
use crate::transaction::{InternalTransactionCommand, TransactionKey, TransactionState};

/// Whether an RFC 3261 retransmission-absorption timer is required for the
/// selected route.
///
/// Timers J and K are zero on reliable transports. A route created by older
/// callers may not carry an explicit transport kind, so use the transport
/// implementation's default in that case.
pub(crate) fn uses_unreliable_transport(
    route: &TransportRoute,
    default_transport: TransportType,
) -> bool {
    route.transport_type.unwrap_or(default_transport) == TransportType::Udp
}

/// Helper module for transaction-specific timer operations using the core timer infrastructure.
/// This provides a simpler interface for transaction implementations to start/stop timers.

/// Starts a timer for a transaction using the timer manager
///
/// # Arguments
/// * `timer_manager` - Manager to handle timer execution
/// * `tx_id` - Transaction ID (for logging and identification)
/// * `timer_name` - The name of the timer for events (e.g., "E", "F", "K")
/// * `timer_type` - Type of timer (e.g., TimerType::E, TimerType::F)
/// * `interval` - Duration for the timer
/// * `cmd_tx` - Channel to send commands when the timer fires
///
/// # Returns
/// A compatibility `JoinHandle` that completes with the deadline and whose
/// `abort` operation cancels it. The shared timer worker owns the actual sleep.
pub async fn start_transaction_timer(
    timer_manager: &TimerManager,
    tx_id: &TransactionKey,
    timer_name: &str,
    timer_type: TimerType,
    interval: Duration,
    cmd_tx: mpsc::Sender<InternalTransactionCommand>,
) -> Result<JoinHandle<()>, crate::transaction::error::Error> {
    // Register the transaction if not already done
    timer_manager
        .register_transaction(tx_id.clone(), cmd_tx)
        .await;

    // Start the timer with the manager
    let handle = timer_manager
        .start_timer(tx_id.clone(), timer_type, interval)
        .await?;

    trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), timer_class=%crate::transaction::safe_diagnostics::SafeTimerName::new(timer_name), timer_len=timer_name.len(), interval=?interval, "Started transaction timer");
    Ok(handle)
}

/// Core-path variant of [`start_transaction_timer`] that uses the shared
/// deadline worker directly and allocates no compatibility proxy task.
pub(crate) async fn start_transaction_timer_managed(
    timer_manager: &TimerManager,
    tx_id: &TransactionKey,
    timer_name: &str,
    timer_type: TimerType,
    interval: Duration,
    cmd_tx: mpsc::Sender<InternalTransactionCommand>,
) -> Result<crate::transaction::timer::manager::ManagedTimerHandle, crate::transaction::error::Error>
{
    let handle = timer_manager
        .start_timer_managed(tx_id.clone(), timer_type, interval, cmd_tx)
        .await?;
    trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), timer_class=%crate::transaction::safe_diagnostics::SafeTimerName::new(timer_name), timer_len=timer_name.len(), interval=?interval, "Started managed transaction timer");
    Ok(handle)
}

/// Starts a timer with a transition to a specified state when it expires
///
/// # Arguments
/// * `timer_manager` - Manager to handle timer execution
/// * `tx_id` - Transaction ID (for logging and identification)
/// * `timer_name` - Name of the timer for events (e.g., "K", "D")
/// * `timer_type` - Type of timer (e.g., TimerType::K, TimerType::D)
/// * `interval` - Duration for the timer
/// * `cmd_tx` - Channel to send commands when the timer fires
/// * `target_state` - State to transition to when the timer fires
///
/// # Returns
/// A compatibility `JoinHandle` for completion and cancellation. The shared
/// timer worker owns the actual deadline.
pub async fn start_timer_with_transition(
    timer_manager: &TimerManager,
    tx_id: &TransactionKey,
    timer_name: &str,
    timer_type: TimerType,
    interval: Duration,
    cmd_tx: mpsc::Sender<InternalTransactionCommand>,
    target_state: TransactionState,
) -> Result<JoinHandle<()>, crate::transaction::error::Error> {
    timer_manager
        .register_transaction(tx_id.clone(), cmd_tx)
        .await;
    let handle = timer_manager
        .start_timer_with_transition(
            tx_id.clone(),
            timer_name.to_string(),
            timer_type,
            interval,
            target_state,
        )
        .await?;
    trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), timer_class=%crate::transaction::safe_diagnostics::SafeTimerName::new(timer_name), timer_len=timer_name.len(), interval=?interval, target_state=?target_state, "Started timer with transition");
    Ok(handle)
}

/// Core-path transition timer using the shared queue without a proxy task.
pub(crate) async fn start_timer_with_transition_managed(
    timer_manager: &TimerManager,
    tx_id: &TransactionKey,
    timer_name: &str,
    timer_type: TimerType,
    interval: Duration,
    cmd_tx: mpsc::Sender<InternalTransactionCommand>,
    target_state: TransactionState,
) -> Result<crate::transaction::timer::manager::ManagedTimerHandle, crate::transaction::error::Error>
{
    let handle = timer_manager
        .start_timer_managed_with_transition(
            tx_id.clone(),
            timer_name.to_string(),
            timer_type,
            interval,
            target_state,
            cmd_tx,
        )
        .await?;
    trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), timer_class=%crate::transaction::safe_diagnostics::SafeTimerName::new(timer_name), timer_len=timer_name.len(), interval=?interval, target_state=?target_state, "Started managed timer with transition");
    Ok(handle)
}

/// Unregisters a transaction from the timer manager
///
/// # Arguments
/// * `timer_manager` - Manager to handle timer unregistration
/// * `tx_id` - Transaction ID to unregister
pub async fn unregister_transaction(timer_manager: &TimerManager, tx_id: &TransactionKey) {
    timer_manager.unregister_transaction(tx_id).await;
    trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Unregistered transaction from timer manager");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(transport_type: Option<TransportType>) -> TransportRoute {
        let mut route = TransportRoute::new("127.0.0.1:5060".parse().unwrap());
        route.transport_type = transport_type;
        route
    }

    #[test]
    fn retransmission_absorption_is_udp_only() {
        assert!(uses_unreliable_transport(
            &route(Some(TransportType::Udp)),
            TransportType::Tcp,
        ));

        for transport in [
            TransportType::Tcp,
            TransportType::Tls,
            TransportType::Ws,
            TransportType::Wss,
        ] {
            assert!(!uses_unreliable_transport(
                &route(Some(transport)),
                TransportType::Udp,
            ));
        }
    }

    #[test]
    fn retransmission_absorption_uses_transport_default_when_route_is_unspecified() {
        assert!(uses_unreliable_transport(&route(None), TransportType::Udp,));
        assert!(!uses_unreliable_transport(&route(None), TransportType::Tls,));
    }
}

/// Helper that creates a proper backoff interval for retransmission timers
///
/// # Arguments
/// * `current_interval` - Current timer interval
/// * `settings` - Timer settings containing T1 and T2 values
///
/// # Returns
/// The next interval to use for retransmission (doubles until reaching T2)
pub fn calculate_backoff_interval(
    current_interval: Duration,
    settings: &TimerSettings,
) -> Duration {
    std::cmp::min(current_interval * 2, settings.t2)
}
