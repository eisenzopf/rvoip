use std::{fmt, net::SocketAddr, sync::Arc, time::Duration};
use std::future::Future;
use std::pin::Pin;

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use crate::error::{Error, Result};

pub mod state;
pub mod key;
pub mod event;
pub mod logic;
pub mod runner;

pub use state::*;
pub use key::*;
pub use event::*;

/// Defines the core traits, types, and machinery for SIP transactions.
///
/// This module provides the fundamental building blocks for implementing and managing
/// SIP transactions as specified in RFC 3261. It includes:
///
/// - [`TransactionKey`]: For uniquely identifying transactions.
/// - [`TransactionState`]: For representing the various states a transaction can be in.
/// - [`TransactionEvent`]: For communicating significant occurrences from the transaction layer
///   to a Transaction User (TU).
/// - [`TransactionKind`]: An enum to differentiate between client/server and INVITE/non-INVITE transactions.
/// - [`Transaction`]: An object-safe trait defining common synchronous operations on transactions.
/// - [`TransactionAsync`]: A trait for asynchronous operations, extending `Transaction`.
/// - [`InternalTransactionCommand`]: Commands used for internal control flow within a transaction's lifecycle.
/// - [`TimerConfig`]: Configuration for various transaction-related timers (T1, T2, T4, etc.).
///
/// The design separates synchronous state inspection (`Transaction`) from asynchronous state
/// modification and event processing (`TransactionAsync`), facilitating easier management and
/// interaction with transaction objects.
/// Distinguishes between the four fundamental types of SIP transactions based on the
/// request method (INVITE or other) and the role of the local SIP element (Client or Server).
///
/// This classification is crucial as each kind follows a distinct state machine as
/// defined in RFC 3261, Section 17.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionKind {
    /// A client transaction initiated by sending an INVITE request.
    /// Follows the state machine in RFC 3261, Section 17.1.1.
    InviteClient,
    /// A client transaction initiated by sending a non-INVITE request (e.g., REGISTER, OPTIONS, BYE).
    /// Follows the state machine in RFC 3261, Section 17.1.2.
    NonInviteClient,
    /// A server transaction initiated by receiving an INVITE request.
    /// Follows the state machine in RFC 3261, Section 17.2.1.
    InviteServer,
    /// A server transaction initiated by receiving a non-INVITE request.
    /// Follows the state machine in RFC 3261, Section 17.2.2.
    NonInviteServer,
}

/// Represents commands that can be sent to a transaction's internal processing logic,
/// typically by the `TransactionManager` or the transaction itself (e.g., for timer events).
///
/// These commands drive the transaction's state machine and interactions.
#[derive(Debug, Clone)] // Clone is useful if commands need to be resent or duplicated.
pub enum InternalTransactionCommand {
    /// Instructs the transaction to transition to a specified `TransactionState`.
    /// This should be used carefully, respecting the valid state transitions for the transaction kind.
    TransitionTo(TransactionState),
    /// Delivers an incoming SIP `Message` (Request or Response) to the transaction for processing.
    /// The transaction will determine how this message affects its state based on its kind and current state.
    ProcessMessage(Message),
    /// Signals that a specific transaction timer has fired.
    /// The `String` typically identifies the timer (e.g., "Timer_A", "Timer_F").
    Timer(String),
    /// Notifies the transaction of a transport-level error that occurred while trying to send a message
    /// associated with this transaction. This usually leads to the transaction terminating.
    TransportError,
    /// Instructs the transaction to terminate immediately, cleaning up its resources.
    /// This might be used for forceful shutdown or after a critical error.
    Terminate,
}

/// A common, object-safe trait providing synchronous access to core properties of a SIP transaction.
///
/// This trait allows different transaction types (e.g., `ClientInviteTransaction`, `ServerNonInviteTransaction`)
/// to be handled uniformly when only their basic, synchronously accessible information is needed.
/// It avoids `async` methods to maintain object safety (i.e., `dyn Transaction`).
/// For asynchronous operations, see the [`TransactionAsync`] trait.
pub trait Transaction: Send + Sync + fmt::Debug {
    /// Returns the unique key identifying this transaction.
    /// The key is typically derived from the branch parameter of the Via header, the method, and directionality.
    fn id(&self) -> &TransactionKey;
    
    /// Returns the [`TransactionKind`] of this transaction (e.g., InviteClient, InviteServer).
    fn kind(&self) -> TransactionKind;
    
    /// Returns the current [`TransactionState`] of this transaction (e.g., Trying, Proceeding, Completed).
    fn state(&self) -> TransactionState;
    
    /// Returns the network [`SocketAddr`] of the remote party involved in this transaction.
    /// For client transactions, this is the destination address. For server transactions, it's the source address.
    fn remote_addr(&self) -> SocketAddr;
    
    /// Checks if the given SIP `Message` (request or response) matches this transaction
    /// according to the rules in RFC 3261, Section 17.1.3 (client) and 17.2.3 (server).
    /// This involves comparing Via branch, CSeq method and number, From/To tags, Call-ID, etc.
    fn matches(&self, message: &Message) -> bool;
    
    /// Provides a way to downcast this transaction object to its concrete type if needed.
    /// This is a standard Rust pattern for trait objects.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// An extension trait for [`Transaction`] that adds asynchronous operations necessary for
/// driving the transaction's lifecycle and interacting with its state machine.
///
/// Implementors of this trait handle the core logic of processing SIP messages, timer events,
/// and internal commands, typically involving asynchronous operations like network I/O or
/// interaction with shared resources.
pub trait TransactionAsync: Transaction {
    /// Asynchronously processes a significant event relevant to the transaction.
    ///
    /// This method is a general entry point for various event types, differentiated by `event_type`.
    /// It might be called by the `TransactionManager` upon receiving a SIP message that matches
    /// this transaction, or when a timer associated with this transaction fires.
    ///
    /// # Arguments
    /// * `event_type`: A string slice identifying the type of event (e.g., "sip_message", "timer_A").
    /// * `message`: An optional SIP `Message` associated with the event. This is `Some` for
    ///              events like incoming requests/responses and `None` for timer events.
    ///
    /// # Returns
    /// A pinned, boxed future that resolves to `Ok(())` on successful processing, or an `Error`
    /// if processing fails. The future must be `Send` to allow it to be spawned on a runtime.
    fn process_event<'a>(
        &'a self, 
        event_type: &'a str, // Consider an enum for event_type for better type safety
        message: Option<Message> // Message is owned, implies it might be consumed or stored.
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    /// Sends an [`InternalTransactionCommand`] to this transaction for asynchronous processing.
    ///
    /// This allows the `TransactionManager` or other components to influence the transaction's
    /// behavior or state (e.g., forcing a state change, initiating termination).
    ///
    /// # Arguments
    /// * `cmd`: The command to be processed by the transaction.
    ///
    /// # Returns
    /// A pinned, boxed future that resolves to `Ok(())` if the command was accepted for processing,
    /// or an `Error` otherwise.
    fn send_command<'a>(
        &'a self,
        cmd: InternalTransactionCommand // Command is owned.
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    /// Asynchronously retrieves the original SIP request that initiated this transaction.
    ///
    /// For client transactions, this is the request sent by the local UA.
    /// For server transactions, this is the request received from the remote UA.
    ///
    /// # Returns
    /// A pinned, boxed future that resolves to `Some(Request)` if the original request is available,
    /// or `None` otherwise (e.g., if the transaction state is such that it's no longer stored).
    fn original_request<'a>(
        &'a self
    ) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + 'a>>;

    /// Asynchronously retrieves the last SIP response that was either sent (for server transactions)
    /// or received (for client transactions) by this transaction.
    ///
    /// # Returns
    /// A pinned, boxed future that resolves to `Some(Response)` if a last response is available,
    /// or `None` otherwise.
    fn last_response<'a>(
        &'a self
    ) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + 'a>>;
}

/// Configuration for standard SIP transaction timer durations, primarily based on RFC 3261.
///
/// These timers govern retransmission intervals, transaction timeouts, and other time-sensitive
/// aspects of transaction behavior.
#[derive(Debug, Clone, Copy)] // Added Copy as it's small and POD-like
pub struct TimerConfig {
    /// **T1: Round-Trip Time (RTT) Estimate (RFC 3261, Section 17.1.1.2)**
    /// An estimate of the RTT between the client and server. It defaults to 500ms if not otherwise known.
    /// This is the initial retransmission interval for INVITE requests and non-INVITE requests over UDP.
    /// For INVITE client transactions, Timer A starts at T1 and doubles for each retransmission up to T2.
    /// For non-INVITE client transactions, Timer F starts at T1 and doubles for retransmissions up to T2.
    pub t1: Duration,

    /// **T2: Maximum Retransmission Interval for Non-INVITE Requests and Responses (RFC 3261, Section 17.1.2.2)**
    /// The maximum interval, in milliseconds, for retransmitting non-INVITE requests and INVITE responses.
    /// It defaults to 4 seconds. Timer F (non-INVITE client) and Timer H (INVITE server, non-2xx response)
    /// retransmit messages at intervals that cap at T2.
    pub t2: Duration,

    /// **T4: Network Maximum Segment Lifetime (MSL) (RFC 3261, Section 17.1.2.2)**
    /// The maximum duration a message will stay in the network. It defaults to 5 seconds.
    /// For non-INVITE transactions, Timer J (server) and Timer K (client) run for T4 after entering `Completed` state
    /// for UDP to ensure old retransmissions are absorbed.
    /// For INVITE server transactions using UDP, Timer G (for 2xx retransmissions by TU) runs for T4 to wait for ACK.
    /// *Note*: This field is named `t4`, but its usage might also cover Timer D for INVITE client (default 32s for UDP).
    /// A more comprehensive `TimerSettings` struct in the `timer` module might be preferred for all specific timers.
    /// This `TimerConfig` seems to be a more general, basic set.
    pub t4: Duration,
    // Consider adding other timers like Timer_B (INVITE client timeout, 64*T1) explicitly if this struct is primary.
    // Or renaming this struct to be more specific if it's only for T1, T2, T4.
}

impl Default for TimerConfig {
    /// Provides default values for `TimerConfig` as recommended by RFC 3261:
    /// - `t1`: 500 milliseconds
    /// - `t2`: 4 seconds
    /// - `t4`: 5 seconds
    fn default() -> Self {
        Self {
            t1: Duration::from_millis(500), // RFC3261 default for T1
            t2: Duration::from_secs(4),     // RFC3261 default for T2
            t4: Duration::from_secs(5),     // RFC3261 default for T4
        }
    }
}

/// Creates a minimal, empty SIP INVITE request for placeholder or default usage.
/// This is `pub(crate)` and intended for internal use within the `transaction-core` crate,
/// for example, when a transaction needs a `Request` object before one is available,
/// or for default initialization in tests or certain internal states.
///
/// The created request uses `Method::Register` and a dummy URI `sip:example.com`.
/// This might need to be `Method::Invite` or more generic if its use implies an INVITE.
/// Currently uses `Method::Register`. Let's assume it's generic enough or update if specific to INVITE.
pub(crate) fn create_empty_request() -> Request {
    let uri = Uri::sip("example.com"); // Creates sip:example.com
    // Corrected: Request::new in sip-core only takes method and uri. 
    // Version, headers, body are set via builder or other means if needed.
    Request::new(Method::Register, uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_kind_creation() {
        let _invite_client = TransactionKind::InviteClient;
        let _non_invite_client = TransactionKind::NonInviteClient;
        let _invite_server = TransactionKind::InviteServer;
        let _non_invite_server = TransactionKind::NonInviteServer;
        // Simply ensuring they can be constructed and used (e.g. in matches) is enough.
    }

    #[test]
    fn internal_transaction_command_creation() {
        let _cmd1 = InternalTransactionCommand::TransitionTo(TransactionState::Completed);
        let _cmd2 = InternalTransactionCommand::ProcessMessage(Message::Request(create_empty_request()));
        let _cmd3 = InternalTransactionCommand::Timer("Timer_A".to_string());
        let _cmd4 = InternalTransactionCommand::TransportError;
        let _cmd5 = InternalTransactionCommand::Terminate;
    }

    #[test]
    fn timer_config_default() {
        let config = TimerConfig::default();
        assert_eq!(config.t1, Duration::from_millis(500));
        assert_eq!(config.t2, Duration::from_secs(4));
        assert_eq!(config.t4, Duration::from_secs(5));
    }

    #[test]
    fn timer_config_clonable_and_copyable() {
        let config1 = TimerConfig::default();
        let config2 = config1; // Test Copy
        let config3 = config1.clone(); // Test Clone

        assert_eq!(config1.t1, config2.t1);
        assert_eq!(config1.t1, config3.t1);
    }

    #[test]
    fn create_empty_request_works() {
        let req = create_empty_request();
        assert_eq!(req.method(), Method::Register); // As per current implementation
        assert_eq!(req.uri().to_string(), "sip:example.com");
    }
} 