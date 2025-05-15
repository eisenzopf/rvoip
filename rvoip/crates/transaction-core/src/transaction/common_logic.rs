use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use crate::error::{Error, Result};
use crate::transaction::{
    TransactionState, TransactionKey, TransactionEvent, 
    InternalTransactionCommand, TransactionKind
};

/// Send a transaction state changed event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `previous_state` - The previous state
/// * `new_state` - The new state
/// * `events_tx` - The events channel to send on
pub async fn send_state_changed_event(
    tx_id: &TransactionKey,
    previous_state: TransactionState,
    new_state: TransactionState,
    events_tx: &mpsc::Sender<TransactionEvent>,
) {
    debug!(id=%tx_id, "State transition: {:?} -> {:?}", previous_state, new_state);
    let _ = events_tx.send(TransactionEvent::StateChanged {
        transaction_id: tx_id.clone(),
        previous_state,
        new_state,
    }).await;
}

/// Send a transaction terminated event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `events_tx` - The events channel to send on
pub async fn send_transaction_terminated_event(
    tx_id: &TransactionKey,
    events_tx: &mpsc::Sender<TransactionEvent>,
) {
    debug!(id=%tx_id, "Transaction terminated");
    let _ = events_tx.send(TransactionEvent::TransactionTerminated {
        transaction_id: tx_id.clone(),
    }).await;
}

/// Send a timer triggered event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `timer_name` - The name of the timer that triggered
/// * `events_tx` - The events channel to send on
pub async fn send_timer_triggered_event(
    tx_id: &TransactionKey,
    timer_name: &str,
    events_tx: &mpsc::Sender<TransactionEvent>,
) {
    trace!(id=%tx_id, timer=%timer_name, "Timer triggered event");
    let _ = events_tx.send(TransactionEvent::TimerTriggered {
        transaction_id: tx_id.clone(),
        timer: timer_name.to_string(),
    }).await;
}

/// Send a transaction timeout event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `events_tx` - The events channel to send on
pub async fn send_transaction_timeout_event(
    tx_id: &TransactionKey,
    events_tx: &mpsc::Sender<TransactionEvent>,
) {
    debug!(id=%tx_id, "Transaction timed out");
    let _ = events_tx.send(TransactionEvent::TransactionTimeout {
        transaction_id: tx_id.clone(),
    }).await;
}

/// Send a provisional response event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `response` - The provisional response
/// * `events_tx` - The events channel to send on
pub async fn send_provisional_response_event(
    tx_id: &TransactionKey,
    response: Response,
    events_tx: &mpsc::Sender<TransactionEvent>,
) {
    trace!(id=%tx_id, status=%response.status(), "Sending provisional response event");
    let _ = events_tx.send(TransactionEvent::ProvisionalResponse {
        transaction_id: tx_id.clone(),
        response,
    }).await;
}

/// Send a success response event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `response` - The success response
/// * `events_tx` - The events channel to send on
/// * `remote_addr` - The remote socket address
pub async fn send_success_response_event(
    tx_id: &TransactionKey,
    response: Response,
    events_tx: &mpsc::Sender<TransactionEvent>,
    remote_addr: SocketAddr,
) {
    debug!(id=%tx_id, status=%response.status(), "Sending success response event");
    let _ = events_tx.send(TransactionEvent::SuccessResponse {
        transaction_id: tx_id.clone(),
        response: response.clone(),
        need_ack: tx_id.method() == &Method::Invite, // Need ACK if this is an INVITE transaction
        source: remote_addr,
    }).await;
}

/// Send a failure response event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `response` - The failure response
/// * `events_tx` - The events channel to send on
pub async fn send_failure_response_event(
    tx_id: &TransactionKey,
    response: Response,
    events_tx: &mpsc::Sender<TransactionEvent>,
) {
    debug!(id=%tx_id, status=%response.status(), "Sending failure response event");
    let _ = events_tx.send(TransactionEvent::FailureResponse {
        transaction_id: tx_id.clone(),
        response,
    }).await;
}

/// Send a transport error event
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `events_tx` - The events channel to send on
pub async fn send_transport_error_event(
    tx_id: &TransactionKey,
    events_tx: &mpsc::Sender<TransactionEvent>,
) {
    debug!(id=%tx_id, "Sending transport error event");
    let _ = events_tx.send(TransactionEvent::TransportError {
        transaction_id: tx_id.clone(),
    }).await;
}

/// Handle response based on its status code and the current transaction state.
/// Returns the new state to transition to if needed.
/// 
/// # Arguments
/// * `tx_id` - The transaction ID
/// * `response` - The SIP response
/// * `current_state` - The current transaction state
/// * `events_tx` - The events channel to send events on
/// * `is_invite` - Whether this is for an INVITE transaction
/// * `remote_addr` - The remote socket address
/// 
/// # Returns
/// * `Some(TransactionState)` if a state transition is needed
/// * `None` if no state transition is needed
pub async fn handle_response_by_status(
    tx_id: &TransactionKey,
    response: Response,
    current_state: TransactionState,
    events_tx: &mpsc::Sender<TransactionEvent>,
    is_invite: bool,
    remote_addr: SocketAddr,
) -> Option<TransactionState> {
    let status = response.status();
    let is_provisional = status.is_provisional();
    let is_success = status.is_success();
    
    match current_state {
        TransactionState::Trying | TransactionState::Calling => {
            if is_provisional {
                // 1xx responses
                send_provisional_response_event(tx_id, response, events_tx).await;
                
                // INVITE transactions go to Proceeding
                // Non-INVITE transactions also go to Proceeding
                Some(TransactionState::Proceeding)
            } else if is_success {
                // 2xx responses
                send_success_response_event(tx_id, response, events_tx, remote_addr).await;
                
                // For INVITE, success responses go straight to Terminated
                // For non-INVITE, they go to Completed first
                if is_invite {
                    Some(TransactionState::Terminated)
                } else {
                    Some(TransactionState::Completed)
                }
            } else {
                // 3xx-6xx responses
                send_failure_response_event(tx_id, response, events_tx).await;
                
                // Both transaction types go to Completed
                Some(TransactionState::Completed)
            }
        },
        TransactionState::Proceeding => {
            if is_provisional {
                // Additional 1xx responses
                send_provisional_response_event(tx_id, response, events_tx).await;
                None // Stay in Proceeding
            } else if is_success {
                // 2xx responses
                send_success_response_event(tx_id, response, events_tx, remote_addr).await;
                
                // For INVITE, success responses go straight to Terminated
                // For non-INVITE, they go to Completed first
                if is_invite {
                    Some(TransactionState::Terminated)
                } else {
                    Some(TransactionState::Completed)
                }
            } else {
                // 3xx-6xx responses
                send_failure_response_event(tx_id, response, events_tx).await;
                
                // Both transaction types go to Completed
                Some(TransactionState::Completed)
            }
        },
        TransactionState::Completed => {
            // In Completed state, any response is a retransmission
            // For INVITE client transactions, we might need to resend ACK
            // For non-INVITE, we just ignore it
            trace!(id=%tx_id, status=%status, "Received response in Completed state");
            None // Stay in Completed
        },
        _ => {
            // Other states like Initial, Terminated
            trace!(id=%tx_id, state=?current_state, status=%status, "Received response in unexpected state");
            None
        }
    }
}

async fn handle_success_response_for_client_non_invite_transaction(
    tx_id: &TransactionKey,
    response: Response,
    events_tx: &mpsc::Sender<TransactionEvent>,
    remote_addr: SocketAddr,
) {
    // Success response
    // Deliver the response to the transaction user
    debug!(id=%tx_id, "Delivering success response to transaction user");
    send_success_response_event(tx_id, response, events_tx, remote_addr).await;
}

async fn handle_success_response_for_client_invite_transaction(
    tx_id: &TransactionKey,
    state: TransactionState,
    response: Response,
    events_tx: &mpsc::Sender<TransactionEvent>,
    remote_addr: SocketAddr,
) {
    // In the Calling or Proceeding state, move to the Terminated state
    // and deliver the response to the transaction user
    send_success_response_event(tx_id, response, events_tx, remote_addr).await;
}

pub async fn handle_response_for_client_transaction(
    tx_id: &TransactionKey,
    state: TransactionState,
    kind: TransactionKind,
    response: Response,
    events_tx: &mpsc::Sender<TransactionEvent>,
    transport: &Arc<dyn Transport>,
    remote_addr: SocketAddr,
) -> Result<TransactionState> {
    // Get the status code
    let status_code = response.status();
    
    // Handle based on the transaction kind and state
    match kind {
        TransactionKind::InviteClient => {
            match state {
                TransactionState::Calling | TransactionState::Proceeding => {
                    if status_code.is_provisional() {
                        // 1xx response
                        // Move to the Proceeding state 
                        // and deliver the response to the transaction user
                        debug!(id=%tx_id, "Received 1xx response in {:?} state", state);
                        let _ = events_tx.send(TransactionEvent::ProvisionalResponse {
                            transaction_id: tx_id.clone(),
                            response,
                        }).await;
                        
                        return Ok(TransactionState::Proceeding);
                    } else if status_code.is_success() {
                        // 2xx response
                        // Move to the Terminated state 
                        // and deliver the response to the transaction user
                        debug!(id=%tx_id, "Received 2xx response in {:?} state", state);
                        handle_success_response_for_client_invite_transaction(
                            tx_id,
                            state,
                            response,
                            events_tx,
                            remote_addr
                        ).await;
                        
                        return Ok(TransactionState::Terminated);
                    } else {
                        // 3xx-6xx response
                        // Move to the Completed state 
                        // and deliver the response to the transaction user
                        debug!(id=%tx_id, "Received failure response in {:?} state", state);
                        let _ = events_tx.send(TransactionEvent::FailureResponse {
                            transaction_id: tx_id.clone(),
                            response,
                        }).await;
                        
                        return Ok(TransactionState::Completed);
                    }
                },
                _ => {
                    // Ignore responses in other states
                    return Ok(state);
                }
            }
        },
        TransactionKind::NonInviteClient => {
            match state {
                TransactionState::Trying | TransactionState::Proceeding => {
                    if status_code.is_provisional() {
                        // 1xx response
                        // Move to the Proceeding state 
                        // and deliver the response to the transaction user
                        debug!(id=%tx_id, "Received 1xx response in {:?} state", state);
                        let _ = events_tx.send(TransactionEvent::ProvisionalResponse {
                            transaction_id: tx_id.clone(),
                            response,
                        }).await;
                        
                        return Ok(TransactionState::Proceeding);
                    } else {
                        // Final response
                        // Move to the Completed state 
                        // and deliver the response to the transaction user
                        debug!(id=%tx_id, "Received final response in {:?} state", state);
                        if status_code.is_success() {
                            handle_success_response_for_client_non_invite_transaction(
                                tx_id,
                                response,
                                events_tx,
                                remote_addr
                            ).await;
                        } else {
                            let _ = events_tx.send(TransactionEvent::FailureResponse {
                                transaction_id: tx_id.clone(),
                                response,
                            }).await;
                        }
                        
                        return Ok(TransactionState::Completed);
                    }
                },
                _ => {
                    // Ignore responses in other states
                    return Ok(state);
                }
            }
        },
        _ => {
            // Unexpected transaction kind
            error!(id=%tx_id, "Unexpected transaction kind {:?} in handle_response_for_client_transaction", kind);
            return Err(Error::Other(format!("Unexpected transaction kind {:?}", kind)));
        }
    }
} 