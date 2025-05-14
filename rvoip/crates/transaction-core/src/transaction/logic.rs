use std::sync::Arc;
use std::time::Duration; // Required for timer configurations
use tokio::task::JoinHandle;
use tokio::sync::mpsc;

// Assuming these are accessible. Adjust paths if necessary.
use rvoip_sip_core::Message;
use crate::error::Result;
use crate::transaction::{TransactionState, TransactionKind, InternalTransactionCommand};
use crate::timer::TimerSettings;

/// Trait defining the specific logic for a type of SIP transaction.
///
/// Implementors of this trait provide the state-specific handling for messages,
/// timers, and state transitions, while a generic runner handles the common event loop.
///
/// - `D` is the transaction-specific data structure (e.g., `ClientTransactionData`, `ServerTransactionData`).
/// - `TH` is a struct holding the `JoinHandle`s for the specific timers used by this transaction type.
#[async_trait::async_trait]
pub trait TransactionLogic<D, TH>
where
    D: Send + Sync + 'static, // Shared transaction data
    TH: Default + Send + Sync + 'static, // Holds timer JoinHandles
{
    /// Returns the kind of this transaction (e.g., InviteClient, NonInviteServer).
    /// Used for validating state transitions.
    fn kind(&self) -> TransactionKind;

    /// Returns the initial state for this transaction type when it's first created.
    fn initial_state(&self) -> TransactionState;

    /// Provides access to the timer configuration (T1, T2, etc.) for this transaction.
    /// Typically sourced from the shared transaction `data` structure.
    fn timer_settings<'a>(data: &'a Arc<D>) -> &'a TimerSettings;

    /// Processes an incoming SIP message based on the transaction's current state.
    ///
    /// This method encapsulates the core logic of how a transaction reacts to different
    /// SIP messages (e.g., a provisional response, a final response, a retransmitted request).
    ///
    /// # Arguments
    /// * `data`: The shared data associated with this transaction.
    /// * `message`: The SIP message received from the transport layer.
    /// * `current_state`: The current `TransactionState` of this transaction.
    ///
    /// # Returns
    /// * `Ok(Some(new_state))`: If processing the message leads to a state transition.
    /// * `Ok(None)`: If the message is processed but no state change occurs.
    /// * `Err(_)`: If an error occurs during message processing.
    async fn process_message(
        &self,
        data: &Arc<D>,
        message: Message,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>>;

    /// Handles a timer event that has fired for this transaction.
    ///
    /// This method defines what actions to take when a specific timer expires,
    /// such as retransmitting a message or timing out the transaction.
    ///
    /// # Arguments
    /// * `data`: The shared data associated with this transaction.
    /// * `timer_name`: A string identifying the timer that fired (e.g., "A", "F", "Timeout_K").
    /// * `current_state`: The current `TransactionState` of this transaction.
    /// * `timer_handles`: Mutable access to the struct holding specific timer `JoinHandle`s.
    ///                  The implementor should clear the handle for the timer that just fired.
    ///
    /// # Returns
    /// * `Ok(Some(new_state))`: If handling the timer event leads to a state transition.
    /// * `Ok(None)`: If the timer event is handled (e.g., retransmission and timer restart)
    ///               without an immediate state change.
    /// * `Err(_)`: If an error occurs.
    async fn handle_timer(
        &self,
        data: &Arc<D>,
        timer_name: &str, // e.g., "E", "F", "K" for ClientNonInvite
        current_state: TransactionState,
        timer_handles: &mut TH,
    ) -> Result<Option<TransactionState>>;

    /// Called by the generic event loop when the transaction enters a new state.
    ///
    /// This method is responsible for starting any timers that are required for the `new_state`.
    /// It will use the provided `command_tx` to spawn timer tasks that, upon completion,
    /// send an `InternalTransactionCommand::Timer` back to the generic event loop.
    ///
    /// # Arguments
    /// * `data`: The shared data associated with this transaction.
    /// * `new_state`: The `TransactionState` being entered.
    /// * `previous_state`: The `TransactionState` being exited.
    /// * `timer_handles`: Mutable access to the struct holding specific timer `JoinHandle`s.
    ///                  The implementor should store new `JoinHandle`s here.
    /// * `command_tx`: An `mpsc::Sender` for `InternalTransactionCommand`. Timer tasks spawned
    ///                 by this method should use this sender to notify the event loop when they fire.
    ///
    /// # Returns
    /// * `Ok(())`: If timers are successfully started or if no timers are needed for this state.
    /// * `Err(_)`: If an error occurs during timer setup.
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
    /// This method should iterate through the `timer_handles` and abort any active tasks.
    /// It's called by the generic event loop before transitioning to a new state that
    /// might start a different set of timers, or when the transaction is terminating.
    ///
    /// # Arguments
    /// * `timer_handles`: Mutable reference to the transaction's timer handles.
    fn cancel_all_specific_timers(&self, timer_handles: &mut TH);
} 