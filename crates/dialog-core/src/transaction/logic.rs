/// # Transaction Logic
///
/// This module defines the core trait that powers the different transaction state machines
/// required by RFC 3261 Section 17. It provides a flexible, reusable architecture for 
/// implementing the distinct behavior of INVITE and non-INVITE client and server transactions.
///
/// ## RFC 3261 Context
///
/// RFC 3261 defines four different transaction types, each with its own state machine:
///
/// 1. **INVITE Client Transaction** (Section 17.1.1)
/// 2. **Non-INVITE Client Transaction** (Section 17.1.2)  
/// 3. **INVITE Server Transaction** (Section 17.2.1)
/// 4. **Non-INVITE Server Transaction** (Section 17.2.2)
///
/// While each follows different rules for state transitions, retransmissions, and timeouts,
/// they share a common pattern: receiving messages, processing them based on the current state,
/// potentially changing state, and initiating appropriate timers.
///
/// ## Implementation Architecture
///
/// This module implements a trait-based design pattern where:
///
/// 1. The `TransactionLogic` trait defines the interface that all transaction types must implement
/// 2. Each concrete transaction type (e.g., `ClientInviteTransaction`) implements this trait
/// 3. A generic transaction runner in `runner.rs` uses this trait to drive the state machine
///
/// This architecture separates the transaction-specific behavior (how each type processes messages
/// and timers) from the common event loop machinery, reducing code duplication and making the
/// code more maintainable.

use std::sync::Arc;
use std::time::Duration; // Required for timer configurations
use tokio::task::JoinHandle;
use tokio::sync::mpsc;

// Assuming these are accessible. Adjust paths if necessary.
use rvoip_sip_core::Message;
use crate::transaction::error::Result;
use crate::transaction::{TransactionState, TransactionKind, InternalTransactionCommand};
use crate::transaction::timer::TimerSettings;

/// Core trait defining the state machine logic for a SIP transaction type.
///
/// Implementors of this trait provide the transaction-specific behavior for handling
/// messages, timers, and state transitions according to the rules in RFC 3261 Section 17.
/// The generic transaction runner uses this trait to operate the transaction's state machine.
///
/// This trait follows the Strategy Pattern, allowing different transaction types to provide
/// their own implementations of how to process messages, handle timers, and react to state changes,
/// while reusing the common machinery of the event loop.
///
/// # Type Parameters
///
/// - `D`: The transaction-specific data structure (e.g., `ClientTransactionData`, `ServerTransactionData`)
///   that contains the shared state and communication channels.
/// - `TH`: A struct holding `JoinHandle`s for the specific timers used by this transaction type.
///   For example, INVITE client transactions need handles for Timers A, B, and D, while
///   non-INVITE clients need handles for Timers E, F, and K.
#[async_trait::async_trait]
pub trait TransactionLogic<D, TH>
where
    D: Send + Sync + 'static, // Shared transaction data
    TH: Default + Send + Sync + 'static, // Holds timer JoinHandles
{
    /// Returns the kind of this transaction implementation.
    ///
    /// This method identifies which of the four transaction types from RFC 3261 this
    /// implementation represents: INVITE client, non-INVITE client, INVITE server,
    /// or non-INVITE server. The transaction runner uses this to validate state transitions.
    ///
    /// # Returns
    ///
    /// The transaction kind (e.g., `TransactionKind::InviteClient`).
    fn kind(&self) -> TransactionKind;

    /// Returns the initial state for this transaction type.
    ///
    /// Per RFC 3261, different transaction types start in different states:
    /// - INVITE client: Calling (after sending INVITE)
    /// - Non-INVITE client: Trying (after sending request)
    /// - INVITE server: Proceeding (after receiving INVITE and sending 100 Trying)
    /// - Non-INVITE server: Trying or Proceeding (depends on immediate response)
    ///
    /// # Returns
    ///
    /// The appropriate `TransactionState` for the initial state of this transaction type.
    fn initial_state(&self) -> TransactionState;

    /// Provides access to the timer configuration for this transaction.
    ///
    /// RFC 3261 defines several timers with specific durations that control retransmission
    /// intervals, transaction timeouts, and other time-sensitive behavior. This method
    /// retrieves the configuration containing these values.
    ///
    /// # Arguments
    ///
    /// * `data`: A reference to the transaction's shared data.
    ///
    /// # Returns
    ///
    /// A reference to the `TimerSettings` for this transaction.
    fn timer_settings<'a>(data: &'a Arc<D>) -> &'a TimerSettings;

    /// Process an incoming message for this transaction type.
    ///
    /// This method is called when a message (request or response) is received
    /// that matches this transaction. The implementation should handle the message
    /// according to the transaction's state machine and return any state transition.
    ///
    /// For client transactions, this is typically used to handle responses.
    /// For server transactions, this is typically used to handle request retransmissions.
    ///
    /// # Arguments
    ///
    /// * `data`: Shared transaction data
    /// * `message`: The incoming SIP message to process
    /// * `current_state`: The current transaction state
    /// * `timer_handles`: Mutable reference to the transaction's timer handles
    ///
    /// # Returns
    ///
    /// A Result containing an optional new state to transition to.
    /// Return `Ok(Some(state))` to trigger a state transition,
    /// or `Ok(None)` to remain in the current state.
    async fn process_message(
        &self,
        data: &Arc<D>,
        message: Message,
        current_state: TransactionState,
        timer_handles: &mut TH,
    ) -> Result<Option<TransactionState>>;

    /// Handles a timer expiration event based on the transaction's current state.
    ///
    /// RFC 3261 defines various timers for different transaction types:
    /// - INVITE client: Timers A, B, D
    /// - Non-INVITE client: Timers E, F, K
    /// - INVITE server: Timers G, H, I
    /// - Non-INVITE server: Timer J
    ///
    /// This method implements the appropriate behavior when these timers expire,
    /// such as retransmitting messages, terminating the transaction, or moving to a new state.
    ///
    /// # Arguments
    ///
    /// * `data`: The shared data associated with this transaction.
    /// * `timer_name`: A string identifying the specific timer (e.g., "A", "F").
    /// * `current_state`: The current `TransactionState`.
    /// * `timer_handles`: Mutable access to the struct holding timer `JoinHandle`s.
    ///                  The implementation should clear the handle for the timer that fired.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(new_state))`: If the timer event requires a state transition.
    /// * `Ok(None)`: If the timer is handled without needing a state change.
    /// * `Err(_)`: If an error occurs handling the timer.
    async fn handle_timer(
        &self,
        data: &Arc<D>,
        timer_name: &str, // e.g., "E", "F", "K" for ClientNonInvite
        current_state: TransactionState,
        timer_handles: &mut TH,
    ) -> Result<Option<TransactionState>>;

    /// Performs actions needed when the transaction enters a new state.
    ///
    /// This method is called by the transaction runner whenever a state transition occurs.
    /// It's responsible for starting state-specific timers as defined in RFC 3261:
    ///
    /// - INVITE client:
    ///   - Calling state: Start Timers A and B
    ///   - Completed state: Start Timer D
    ///
    /// - Non-INVITE client:
    ///   - Trying state: Start Timers E and F
    ///   - Completed state: Start Timer K
    ///
    /// - INVITE server:
    ///   - Completed state: Start Timers G and H
    ///   - Confirmed state: Start Timer I
    ///
    /// - Non-INVITE server:
    ///   - Completed state: Start Timer J
    ///
    /// # Arguments
    ///
    /// * `data`: The shared transaction data.
    /// * `new_state`: The state being entered.
    /// * `previous_state`: The state being exited.
    /// * `timer_handles`: Mutable access to the timer handles collection where new timer
    ///                  task handles should be stored.
    /// * `command_tx`: Channel for sending commands back to the transaction's event loop,
    ///                used by timer tasks when they complete.
    ///
    /// # Returns
    ///
    /// * `Ok(())`: If the state entry actions were performed successfully.
    /// * `Err(_)`: If an error occurred.
    async fn on_enter_state(
        &self,
        data: &Arc<D>,
        new_state: TransactionState,
        previous_state: TransactionState,
        timer_handles: &mut TH,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<()>;

    /// Cancels all active timers specific to this transaction type.
    ///
    /// This method is called before a state transition to clean up any timers that
    /// should not continue into the new state. It's also called during transaction
    /// termination to ensure all timer tasks are properly aborted.
    ///
    /// For example, when an INVITE client transaction moves from Calling to Proceeding,
    /// Timer A should be canceled as it's no longer needed after receiving a provisional response.
    ///
    /// # Arguments
    ///
    /// * `timer_handles`: Mutable reference to the transaction's timer handles.
    ///                  The implementation should abort and remove all active timers.
    fn cancel_all_specific_timers(&self, timer_handles: &mut TH);
    
    /// Cancels the automatic 100 Trying timer (Timer 100) for INVITE server transactions.
    ///
    /// This method is called when the TU sends any provisional response, making the
    /// automatic 100 Trying response unnecessary per RFC 3261 Section 17.2.1.
    ///
    /// For non-INVITE transactions, this method should be a no-op.
    ///
    /// # Arguments
    ///
    /// * `timer_handles`: Mutable reference to the transaction's timer handles.
    ///
    /// # Returns
    ///
    /// A Result indicating success or failure of the timer cancellation.
    async fn handle_cancel_timer_100(&self, timer_handles: &mut TH) -> Result<()> {
        // Default implementation is a no-op for non-INVITE server transactions
        Ok(())
    }
} 