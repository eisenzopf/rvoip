use std::sync::atomic::{AtomicU8, Ordering};
use crate::error::{Error, Result};
use crate::transaction::TransactionKind;

/// Represents the state of a SIP transaction, aligned with the state machines
/// defined in RFC 3261 (Section 17).
///
/// The state determines how a transaction reacts to incoming messages (requests or responses)
/// and timers. Different transaction kinds (Client/Server, INVITE/Non-INVITE)
/// follow different state machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionState {
    /// The initial state of a transaction before any significant event (like sending a request
    /// for client transactions, or receiving one for server transactions) has occurred.
    /// Not explicitly named in RFC 3261 state diagrams but represents the state before
    /// the transaction officially starts its lifecycle (e.g., before moving to `Calling` or `Trying`).
    Initial,    // Before any action

    /// **Client INVITE & Non-INVITE Transactions / Server Non-INVITE Transactions:**
    /// The transaction has sent a request (Client) or received a request and sent a provisional response (Server Non-INVITE)
    /// and is waiting for a final response or further provisional responses.
    /// Specifically:
    /// - Client INVITE: After sending INVITE, before 1xx/final. (RFC uses "Calling")
    /// - Client Non-INVITE: After sending request, before 1xx/final. (RFC uses "Trying")
    /// - Server Non-INVITE: After receiving request, before sending final response. (RFC uses "Trying" before sending provisional, "Proceeding" after)
    /// This state consolidates these initial active phases.
    /// 
    /// Server INVITE transactions typically start in `Proceeding` after receiving an INVITE if they choose to respond.
    Trying,     // Non-INVITE specific: Request sent, waiting 1xx/final

    /// **Client INVITE & Non-INVITE Transactions / Server INVITE & Non-INVITE Transactions:**
    /// A provisional response (1xx) has been sent (Server) or received (Client).
    /// The transaction is awaiting a final response (Client) or further actions that lead to a final response (Server).
    /// - Client (INVITE/Non-INVITE): After receiving a 1xx, awaiting final response.
    /// - Server (INVITE/Non-INVITE): After sending a 1xx, before sending final response.
    Proceeding, // 1xx received, waiting final

    /// **Client INVITE Transactions Only:**
    /// The initial INVITE request has been sent, and the transaction is awaiting any response (provisional or final).
    /// This is specific to client INVITE transactions as per RFC 3261, Section 17.1.1.
    /// If a provisional response is received, it transitions to `Proceeding`.
    /// If a final response is received, it transitions to `Completed`.
    Calling,    // INVITE specific: Request sent, waiting 1xx/final

    /// **All Transaction Types:**
    /// The transaction has received a final response (Client) or sent a final response (Server).
    /// - Client INVITE/Non-INVITE: Received a final response (2xx-6xx).
    ///   - If 2xx to INVITE, TU must send ACK. Transaction handles ACK retransmissions if it were to send one (it doesn't for 2xx).
    ///   - If non-2xx to INVITE, transaction absorbs ACKs.
    ///   - For Non-INVITE, no ACK processing here.
    /// - Server INVITE: Sent a non-2xx final response. Awaits ACK from client. (Timer H starts)
    /// - Server INVITE: Sent a 2xx final response. Waits for TU to signal ACK, or for Timer G to handle 2xx retransmissions.
    ///   (RFC 3261 indicates the server INVITE transaction itself does not retransmit 2xx, but higher layers might rely on its state/timers)
    ///   *Correction*: Server INVITE sends 2xx and transitions to Terminated after Timer G, unless an ACK is received, then it transitions to Confirmed (then Terminated).
    ///   *RFC17.2.1*: If a 2xx response is sent, state is Terminated. This seems to be a simplification in the RFC diagram for the core transaction logic.
    ///   This `Completed` state for Server INVITE (especially for 2xx) might be more about TU interaction point.
    ///   Let's align with simplified view: for server INVITE 2xx, it moves to Terminated. For non-2xx, it's `Completed` awaiting ACK.
    /// - Server Non-INVITE: Sent a final response. (Timer J starts for UDP to absorb retransmitted requests).
    /// The transaction remains in this state for a period to absorb retransmissions or ACKs.
    Completed,  // Final response sent/received, waiting for termination timer/ACK

    /// **Server INVITE Transactions Only:**
    /// A 2xx final response was sent, and an ACK has been received from the client.
    /// The transaction will shortly transition to `Terminated`.
    /// (RFC 3261, Section 17.2.1)
    Confirmed,  // ACK received (Server INVITE only)

    /// **All Transaction Types:**
    /// The transaction has finished all its processing, timers have expired, and it should be destroyed.
    /// No further messages will be processed by this transaction instance.
    Terminated, // Transaction finished
}

impl TransactionState {
    /// Checks if the transaction state is `Terminated`.
    ///
    /// # Returns
    /// `true` if the state is `Terminated`, `false` otherwise.
    pub fn is_terminated(&self) -> bool {
        *self == TransactionState::Terminated
    }
}

/// Numeric representation of transaction states for atomic operations
/// This internal enum allows `TransactionState` to be stored and manipulated
/// as a `u8` in `AtomicTransactionState`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StateValue {
    Initial = 0,
    Calling = 1,
    Trying = 2,
    Proceeding = 3,
    Completed = 4,
    Confirmed = 5,
    Terminated = 6,
}

impl From<TransactionState> for StateValue {
    fn from(state: TransactionState) -> Self {
        match state {
            TransactionState::Initial => StateValue::Initial,
            TransactionState::Calling => StateValue::Calling,
            TransactionState::Trying => StateValue::Trying,
            TransactionState::Proceeding => StateValue::Proceeding,
            TransactionState::Completed => StateValue::Completed,
            TransactionState::Confirmed => StateValue::Confirmed,
            TransactionState::Terminated => StateValue::Terminated,
        }
    }
}

impl From<StateValue> for TransactionState {
    fn from(value: StateValue) -> Self {
        match value {
            StateValue::Initial => TransactionState::Initial,
            StateValue::Calling => TransactionState::Calling,
            StateValue::Trying => TransactionState::Trying,
            StateValue::Proceeding => TransactionState::Proceeding,
            StateValue::Completed => TransactionState::Completed,
            StateValue::Confirmed => TransactionState::Confirmed,
            StateValue::Terminated => TransactionState::Terminated,
        }
    }
}

impl From<u8> for StateValue {
    fn from(value: u8) -> Self {
        match value {
            0 => StateValue::Initial,
            1 => StateValue::Calling,
            2 => StateValue::Trying,
            3 => StateValue::Proceeding,
            4 => StateValue::Completed,
            5 => StateValue::Confirmed,
            6 => StateValue::Terminated,
            _ => StateValue::Terminated, // Default to terminated for unknown values
        }
    }
}

/// Provides thread-safe, atomic management of a `TransactionState`.
///
/// This struct wraps a `TransactionState` (internally represented as a `u8`)
/// in an `AtomicU8`, allowing for atomic reads and writes, which is crucial
/// when a transaction's state might be accessed or modified by multiple tasks
/// concurrently (e.g., a task processing an incoming message and a timer task).
#[derive(Debug)]
pub struct AtomicTransactionState {
    value: AtomicU8,
}

impl AtomicTransactionState {
    /// Creates a new `AtomicTransactionState` initialized to the given `state`.
    pub fn new(state: TransactionState) -> Self {
        Self {
            value: AtomicU8::new(StateValue::from(state) as u8),
        }
    }

    /// Atomically loads and returns the current `TransactionState`.
    /// Uses `Ordering::Acquire` to ensure that all previous writes by other threads
    /// that released this atomic variable are visible in the current thread.
    pub fn get(&self) -> TransactionState {
        let value = self.value.load(Ordering::Acquire);
        TransactionState::from(StateValue::from(value))
    }

    /// Atomically sets the current state to `new_state` and returns the previous state.
    /// Uses `Ordering::AcqRel` (Acquire-Release) to ensure that this operation acts as
    /// both an acquire operation (for the read part) and a release operation (for the write part),
    /// synchronizing memory with other threads.
    pub fn set(&self, new_state: TransactionState) -> TransactionState {
        let prev_value = self.value.swap(StateValue::from(new_state) as u8, Ordering::AcqRel);
        TransactionState::from(StateValue::from(prev_value))
    }

    /// Atomically transitions the state from `current_state` to `new_state` if the
    /// current state matches `current_state`.
    ///
    /// This is a compare-and-swap (CAS) operation.
    ///
    /// # Behavior
    /// - If the actual current state is equal to the `current_state` parameter, it's updated to `new_state`,
    ///   and `Ok(true)` is returned.
    /// - If the actual current state is already `new_state`, `Ok(true)` is returned (idempotent success).
    /// - If `new_state` is `TransactionState::Terminated`, the transition occurs unconditionally
    ///   (i.e., an attempt to terminate will always store `Terminated`), and `Ok(true)` is returned.
    /// - Otherwise (actual current state is neither `current_state` nor `new_state`, and `new_state` is not `Terminated`),
    ///   the state is not changed, and `Ok(false)` is returned.
    ///
    /// # Returns
    /// - `Ok(true)`: The state was successfully transitioned (or was already `new_state`, or `new_state` was `Terminated`).
    /// - `Ok(false)`: The state was not transitioned because the actual current state did not match `current_state`
    ///                (and it wasn't an unconditional termination or already the target state).
    /// Note: This method previously returned `Result<bool>`, but `Error` was not constructible here. 
    /// The RFC validation is separate. This method focuses on the atomic CAS logic.
    pub fn transition_if(&self, current_state: TransactionState, new_state: TransactionState) -> bool {
        let current_value = StateValue::from(current_state) as u8;
        let new_value = StateValue::from(new_state) as u8;
        
        // Attempt to transition using compare_exchange.
        match self.value.compare_exchange(
            current_value,
            new_value,
            Ordering::AcqRel, // Strongest ordering for CAS success.
            Ordering::Acquire, // Weaker ordering for CAS failure is fine, only reading.
        ) {
            Ok(_) => true, // Successfully transitioned from current_value to new_value.
            Err(actual_loaded_value) => {
                // CAS failed. Check why.
                if actual_loaded_value == new_value {
                    // The state was already what we wanted it to be.
                    true
                } else if new_state == TransactionState::Terminated {
                    // If the target is Terminated, force it.
                    // This ensures transactions can always be moved to Terminated.
                    self.value.store(new_value, Ordering::Release);
                    true
                } else {
                    // The current state was not as expected, and we are not forcing termination,
                    // nor was it already the new_state.
                    false
                }
            }
        }
    }

    /// Validates if a transition from `current_state` to `new_state` is valid
    /// for the given transaction kind according to the RFC 3261 state machine rules.
    ///
    /// Returns `Ok(())` if the transition is valid, or `Err(String)` with an error message if invalid.
    pub fn validate_transition(
        tx_kind: TransactionKind,
        current_state: TransactionState,
        new_state: TransactionState,
    ) -> std::result::Result<(), String> {
        if current_state == new_state {
            // Always allow transitioning to the same state (no-op)
            return Ok(());
        }

        // Always allow transitions to Terminated from any state for all transaction kinds
        if new_state == TransactionState::Terminated {
            return Ok(());
        }
        
        match tx_kind {
            TransactionKind::InviteClient => {
                match current_state {
                    TransactionState::Initial => {
                        // Initial state can transition to Calling or Terminated
                        if new_state == TransactionState::Calling {
                            return Ok(());
                        }
                    },
                    TransactionState::Calling => {
                        // Calling can transition to Proceeding, Completed, or Terminated
                        match new_state {
                            TransactionState::Proceeding | 
                            TransactionState::Completed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Proceeding => {
                        // Proceeding can transition to Completed or Terminated
                        match new_state {
                            TransactionState::Completed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Completed => {
                        // Completed can only transition to Terminated
                        // Handled above
                    },
                    TransactionState::Terminated => {
                        // Terminated is a final state, cannot transition further
                        return Err("Cannot transition from Terminated state".to_string());
                    },
                    // States that don't apply to this transaction kind
                    _ => {},
                }
            },
            TransactionKind::NonInviteClient => {
                match current_state {
                    TransactionState::Initial => {
                        // Initial state can transition to Trying or Terminated
                        if new_state == TransactionState::Trying {
                            return Ok(());
                        }
                    },
                    TransactionState::Trying => {
                        // Trying can transition to Proceeding, Completed, or Terminated
                        match new_state {
                            TransactionState::Proceeding | 
                            TransactionState::Completed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Proceeding => {
                        // Proceeding can transition to Completed or Terminated
                        match new_state {
                            TransactionState::Completed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Completed => {
                        // Completed can only transition to Terminated
                        // Handled above
                    },
                    TransactionState::Terminated => {
                        // Terminated is a final state, cannot transition further
                        return Err("Cannot transition from Terminated state".to_string());
                    },
                    // States that don't apply to this transaction kind
                    _ => {},
                }
            },
            TransactionKind::InviteServer => {
                match current_state {
                    TransactionState::Initial => {
                        // Initial state can transition to Proceeding, Completed, or Terminated
                        match new_state {
                            TransactionState::Proceeding |
                            TransactionState::Completed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Proceeding => {
                        // Proceeding can transition to Completed, Confirmed, or Terminated
                        match new_state {
                            TransactionState::Completed | 
                            TransactionState::Confirmed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Completed => {
                        // Completed can transition to Confirmed or Terminated
                        match new_state {
                            TransactionState::Confirmed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Confirmed => {
                        // Confirmed can only transition to Terminated
                        // Handled above
                    },
                    TransactionState::Terminated => {
                        // Terminated is a final state, cannot transition further
                        return Err("Cannot transition from Terminated state".to_string());
                    },
                    // States that don't apply to this transaction kind
                    _ => {},
                }
            },
            TransactionKind::NonInviteServer => {
                match current_state {
                    TransactionState::Initial => {
                        // Initial state can transition to Trying, Proceeding, or Terminated
                        match new_state {
                            TransactionState::Trying | 
                            TransactionState::Proceeding => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Trying => {
                        // Trying can transition to Proceeding, Completed, or Terminated
                        match new_state {
                            TransactionState::Proceeding | 
                            TransactionState::Completed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Proceeding => {
                        // Proceeding can transition to Completed or Terminated
                        match new_state {
                            TransactionState::Completed => return Ok(()),
                            _ => {},
                        }
                    },
                    TransactionState::Completed => {
                        // Completed can only transition to Terminated
                        // Handled above
                    },
                    TransactionState::Terminated => {
                        // Terminated is a final state, cannot transition further
                        return Err("Cannot transition from Terminated state".to_string());
                    },
                    // States that don't apply to this transaction kind
                    _ => {},
                }
            },
        }
        
        Err(format!(
            "Invalid transition for {:?}: {:?} -> {:?}", 
            tx_kind, current_state, new_state
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_state_is_terminated() {
        assert!(TransactionState::Terminated.is_terminated());
        assert!(!TransactionState::Initial.is_terminated());
        assert!(!TransactionState::Calling.is_terminated());
        assert!(!TransactionState::Trying.is_terminated());
        assert!(!TransactionState::Proceeding.is_terminated());
        assert!(!TransactionState::Completed.is_terminated());
        assert!(!TransactionState::Confirmed.is_terminated());
    }

    #[test]
    fn state_value_from_transaction_state() {
        assert_eq!(StateValue::from(TransactionState::Initial), StateValue::Initial);
        assert_eq!(StateValue::from(TransactionState::Calling), StateValue::Calling);
        assert_eq!(StateValue::from(TransactionState::Trying), StateValue::Trying);
        assert_eq!(StateValue::from(TransactionState::Proceeding), StateValue::Proceeding);
        assert_eq!(StateValue::from(TransactionState::Completed), StateValue::Completed);
        assert_eq!(StateValue::from(TransactionState::Confirmed), StateValue::Confirmed);
        assert_eq!(StateValue::from(TransactionState::Terminated), StateValue::Terminated);
    }

    #[test]
    fn transaction_state_from_state_value() {
        assert_eq!(TransactionState::from(StateValue::Initial), TransactionState::Initial);
        assert_eq!(TransactionState::from(StateValue::Calling), TransactionState::Calling);
        assert_eq!(TransactionState::from(StateValue::Trying), TransactionState::Trying);
        assert_eq!(TransactionState::from(StateValue::Proceeding), TransactionState::Proceeding);
        assert_eq!(TransactionState::from(StateValue::Completed), TransactionState::Completed);
        assert_eq!(TransactionState::from(StateValue::Confirmed), TransactionState::Confirmed);
        assert_eq!(TransactionState::from(StateValue::Terminated), TransactionState::Terminated);
    }

    #[test]
    fn state_value_from_u8() {
        assert_eq!(StateValue::from(0u8), StateValue::Initial);
        assert_eq!(StateValue::from(1u8), StateValue::Calling);
        assert_eq!(StateValue::from(2u8), StateValue::Trying);
        assert_eq!(StateValue::from(3u8), StateValue::Proceeding);
        assert_eq!(StateValue::from(4u8), StateValue::Completed);
        assert_eq!(StateValue::from(5u8), StateValue::Confirmed);
        assert_eq!(StateValue::from(6u8), StateValue::Terminated);
        assert_eq!(StateValue::from(7u8), StateValue::Terminated); // Test default for unknown
        assert_eq!(StateValue::from(255u8), StateValue::Terminated); // Test default for unknown
    }

    #[test]
    fn atomic_transaction_state_new_and_get() {
        let atomic_state = AtomicTransactionState::new(TransactionState::Calling);
        assert_eq!(atomic_state.get(), TransactionState::Calling);

        let atomic_state_terminated = AtomicTransactionState::new(TransactionState::Terminated);
        assert_eq!(atomic_state_terminated.get(), TransactionState::Terminated);
    }

    #[test]
    fn atomic_transaction_state_set() {
        let atomic_state = AtomicTransactionState::new(TransactionState::Initial);
        let prev_state = atomic_state.set(TransactionState::Proceeding);
        assert_eq!(prev_state, TransactionState::Initial);
        assert_eq!(atomic_state.get(), TransactionState::Proceeding);

        let prev_state_2 = atomic_state.set(TransactionState::Completed);
        assert_eq!(prev_state_2, TransactionState::Proceeding);
        assert_eq!(atomic_state.get(), TransactionState::Completed);
    }

    #[test]
    fn atomic_transaction_state_transition_if_success() {
        let atomic_state = AtomicTransactionState::new(TransactionState::Trying);
        // Successful transition: current state matches expected
        assert!(atomic_state.transition_if(TransactionState::Trying, TransactionState::Proceeding));
        assert_eq!(atomic_state.get(), TransactionState::Proceeding);
    }

    #[test]
    fn atomic_transaction_state_transition_if_already_new_state() {
        let atomic_state = AtomicTransactionState::new(TransactionState::Completed);
        // Current state is already new_state
        assert!(atomic_state.transition_if(TransactionState::Proceeding, TransactionState::Completed));
        assert_eq!(atomic_state.get(), TransactionState::Completed);
    }

    #[test]
    fn atomic_transaction_state_transition_if_fail_current_mismatch() {
        let atomic_state = AtomicTransactionState::new(TransactionState::Calling);
        // Failed transition: current state (Calling) does not match expected (Trying)
        assert!(!atomic_state.transition_if(TransactionState::Trying, TransactionState::Proceeding));
        assert_eq!(atomic_state.get(), TransactionState::Calling); // State should not change
    }

    #[test]
    fn atomic_transaction_state_transition_if_unconditional_terminate() {
        let atomic_state = AtomicTransactionState::new(TransactionState::Calling);
        // Transition to Terminated should succeed even if current_state param doesn't match actual
        assert!(atomic_state.transition_if(TransactionState::Initial, TransactionState::Terminated));
        assert_eq!(atomic_state.get(), TransactionState::Terminated);

        let atomic_state_2 = AtomicTransactionState::new(TransactionState::Completed);
        assert!(atomic_state_2.transition_if(TransactionState::Completed, TransactionState::Terminated));
        assert_eq!(atomic_state_2.get(), TransactionState::Terminated);
    }
    
    // --- Tests for validate_transition --- 

    // Helper macro for terser validation tests
    macro_rules! assert_valid_transition {
        ($kind:expr, $from:expr, $to:expr) => {
            assert!(AtomicTransactionState::validate_transition($kind, $from, $to).is_ok(),
                    "Expected valid transition for {:?} from {:?} to {:?}", $kind, $from, $to);
        };
    }

    macro_rules! assert_invalid_transition {
        ($kind:expr, $from:expr, $to:expr) => {
            assert!(AtomicTransactionState::validate_transition($kind, $from, $to).is_err(),
                    "Expected invalid transition for {:?} from {:?} to {:?}", $kind, $from, $to);
        };
    }

    #[test]
    fn validate_invite_client_transitions() {
        use TransactionState::*; // Initial, Calling, Proceeding, Completed, Terminated
        let kind = TransactionKind::InviteClient;

        // Valid transitions
        assert_valid_transition!(kind, Initial, Calling);
        assert_valid_transition!(kind, Calling, Proceeding);
        assert_valid_transition!(kind, Calling, Completed);
        assert_valid_transition!(kind, Proceeding, Completed);
        assert_valid_transition!(kind, Completed, Terminated);

        // Always valid: to Terminated
        assert_valid_transition!(kind, Initial, Terminated);
        assert_valid_transition!(kind, Calling, Terminated);
        assert_valid_transition!(kind, Proceeding, Terminated);
        
        // Valid: same state
        assert_valid_transition!(kind, Calling, Calling);
        assert_valid_transition!(kind, Proceeding, Proceeding);

        // Invalid transitions
        assert_invalid_transition!(kind, Initial, Proceeding);
        assert_invalid_transition!(kind, Initial, Completed);
        assert_invalid_transition!(kind, Calling, Trying); // Trying is not for InviteClient
        assert_invalid_transition!(kind, Proceeding, Calling);
        assert_invalid_transition!(kind, Completed, Calling);
        assert_invalid_transition!(kind, Terminated, Calling); // Cannot leave Terminated (except to Terminated)
    }

    #[test]
    fn validate_non_invite_client_transitions() {
        use TransactionState::*; // Initial, Trying, Proceeding, Completed, Terminated
        let kind = TransactionKind::NonInviteClient;

        assert_valid_transition!(kind, Initial, Trying);
        assert_valid_transition!(kind, Trying, Proceeding);
        assert_valid_transition!(kind, Trying, Completed);
        assert_valid_transition!(kind, Proceeding, Completed);
        assert_valid_transition!(kind, Completed, Terminated);
        
        assert_valid_transition!(kind, Initial, Terminated); // Always valid
        assert_valid_transition!(kind, Trying, Trying); // Same state

        assert_invalid_transition!(kind, Initial, Calling); // Calling is for InviteClient
        assert_invalid_transition!(kind, Initial, Completed);
        assert_invalid_transition!(kind, Trying, Initial);
    }

    #[test]
    fn validate_invite_server_transitions() {
        use TransactionState::*; // Initial, Proceeding, Completed, Confirmed, Terminated
        let kind = TransactionKind::InviteServer;
        
        // Valid based on RFC + typical flows (e.g. server sends 1xx first)
        assert_valid_transition!(kind, Initial, Proceeding);
        assert_valid_transition!(kind, Initial, Completed); // e.g. send 4xx immediately
        assert_valid_transition!(kind, Proceeding, Completed);
        assert_valid_transition!(kind, Proceeding, Terminated); // e.g. send 2xx
        assert_valid_transition!(kind, Completed, Confirmed);   // Sent non-2xx, got ACK
        assert_valid_transition!(kind, Completed, Terminated);  // Timer H for non-2xx, or after 2xx+Timer G
        assert_valid_transition!(kind, Confirmed, Terminated);  // Timer I after ACK for non-2xx

        assert_valid_transition!(kind, Initial, Terminated); // Always valid
        assert_valid_transition!(kind, Proceeding, Proceeding); // Same state

        assert_invalid_transition!(kind, Initial, Calling);
        assert_invalid_transition!(kind, Initial, Trying);
        assert_invalid_transition!(kind, Proceeding, Initial);
        assert_invalid_transition!(kind, Confirmed, Proceeding);
    }

    #[test]
    fn validate_non_invite_server_transitions() {
        use TransactionState::*; // Initial, Trying, Proceeding, Completed, Terminated
        let kind = TransactionKind::NonInviteServer;

        assert_valid_transition!(kind, Initial, Trying);
        assert_valid_transition!(kind, Trying, Proceeding);
        assert_valid_transition!(kind, Trying, Completed);
        assert_valid_transition!(kind, Proceeding, Completed);
        assert_valid_transition!(kind, Completed, Terminated);
        
        assert_valid_transition!(kind, Initial, Terminated); // Always valid
        assert_valid_transition!(kind, Trying, Trying); // Same state

        assert_invalid_transition!(kind, Initial, Calling);
        assert_invalid_transition!(kind, Initial, Completed);
        assert_invalid_transition!(kind, Completed, Trying);
    }
} 