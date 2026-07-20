//! Exact client-transaction completion state.
//!
//! The public transaction event stream is observational. Protocol code that
//! needs an exact answer (BYE, MESSAGE, authentication retries, and session
//! timers) waits on this cell instead. The cell is owned by the transaction
//! while active and retained by the manager after runner removal, closing both
//! response-before-wait and response-versus-removal races without allocating a
//! global event subscription.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytes::Bytes;
use rvoip_sip_core::prelude::{Message, Response};
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::HeaderName;
use rvoip_sip_core::Uri;
use tokio::sync::Notify;

use crate::transaction::error::{Error, Result};
use crate::transaction::TransactionState;

/// A terminal client-transaction failure that did not produce a final SIP
/// response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientTransactionFailure {
    /// The RFC transaction response timer expired.
    Timeout,
    /// The selected transport failed.
    Transport,
    /// Transaction processing failed internally.
    Internal,
    /// The transaction was explicitly cancelled by its owner.
    Cancelled,
    /// The transaction terminated without a more specific result.
    Terminated,
}

/// The authoritative terminal outcome of a client transaction.
#[derive(Clone)]
pub enum ClientTransactionOutcome {
    /// A final SIP response (status code 200 or greater).
    FinalResponse(Response),
    /// A terminal failure that did not carry a final SIP response.
    Failure(ClientTransactionFailure),
}

impl std::fmt::Debug for ClientTransactionOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FinalResponse(response) => formatter
                .debug_struct("FinalResponse")
                .field("status", &response.status().as_u16())
                .finish(),
            Self::Failure(failure) => formatter.debug_tuple("Failure").field(failure).finish(),
        }
    }
}

#[derive(Clone)]
enum StoredResponse {
    Parsed(Arc<Response>),
    Wire(Bytes),
}

struct CompletionSnapshot {
    revision: u64,
    state: TransactionState,
    visited_states: u8,
    last_response: Option<StoredResponse>,
    /// Exact Request-URI for a 401/407 response. This is populated only for
    /// authentication challenges, so ordinary Timer K tombstones keep their
    /// compact shape while a challenge can still be retried after the live
    /// transaction runner has retired.
    auth_challenge_request_uri: Option<Arc<Uri>>,
    terminal_failure: Option<ClientTransactionFailure>,
}

/// Immutable terminal representation installed in the manager after the
/// transaction runner is removed.
///
/// Existing waiters retain the live [`ClientTransactionCompletion`] Arc, while
/// lookups that race with or follow removal resolve this record.  Keeping the
/// terminal representation separate lets retirement release the cell's
/// `Mutex`, `Notify`, and allocation without weakening exact-response waits.
#[derive(Clone)]
pub(crate) struct RetainedClientTransactionCompletion {
    revision: u64,
    state: TransactionState,
    visited_states: u8,
    last_response: Option<Bytes>,
    auth_challenge_request_uri: Option<Arc<Uri>>,
    terminal_failure: Option<ClientTransactionFailure>,
    expires_at: Instant,
    version: u64,
    _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
}

impl RetainedClientTransactionCompletion {
    pub(crate) fn with_admission_owner(
        mut self,
        owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
    ) -> Self {
        self._admission_owner = owner;
        self
    }
    pub(crate) fn deadline(&self) -> (Instant, u64) {
        (self.expires_at, self.version)
    }

    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        self.expires_at <= now
    }

    pub(crate) fn reached_state(&self, state: TransactionState) -> bool {
        // Retention preserves the authoritative completion revision together
        // with the visited-state mask. Reading it here documents that this is
        // a versioned terminal snapshot rather than a reconstruction from the
        // final state alone.
        let _revision = self.revision;
        self.visited_states & transaction_state_bit(state) != 0
    }

    pub(crate) fn diagnostics(&self) -> CompletionCellDiagnostics {
        CompletionCellDiagnostics {
            compact: 1,
            parsed_responses: 0,
            wire_responses: usize::from(self.last_response.is_some()),
            wire_response_bytes: self.last_response.as_ref().map_or(0, Bytes::len),
        }
    }

    pub(crate) fn wire_response(&self) -> Option<&Bytes> {
        self.last_response.as_ref()
    }

    pub(crate) fn auth_challenge_request_uri(&self) -> Option<Uri> {
        self.auth_challenge_request_uri.as_deref().cloned()
    }

    #[cfg(test)]
    pub(crate) fn set_deadline(&mut self, expires_at: Instant, version: u64) {
        self.expires_at = expires_at;
        self.version = version;
    }

    pub(crate) fn last_response(&self) -> Result<Option<Response>> {
        decode_response(self.last_response.clone().map(StoredResponse::Wire))
    }

    pub(crate) fn outcome(&self) -> Result<Option<ClientTransactionOutcome>> {
        outcome_from_snapshot(
            self.last_response.clone().map(StoredResponse::Wire),
            self.terminal_failure,
            self.state,
        )
    }
}

/// Manager-owned active-or-retained exact completion authority.
#[derive(Clone)]
pub(crate) enum ClientTransactionCompletionEntry {
    Active(Arc<ClientTransactionCompletion>),
    Retained(RetainedClientTransactionCompletion),
}

impl ClientTransactionCompletionEntry {
    pub(crate) fn retained_deadline(&self) -> Option<(Instant, u64)> {
        match self {
            Self::Active(_) => None,
            Self::Retained(completion) => Some(completion.deadline()),
        }
    }

    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        match self {
            Self::Active(_) => false,
            Self::Retained(completion) => completion.is_expired(now),
        }
    }

    pub(crate) fn diagnostics(&self) -> CompletionCellDiagnostics {
        match self {
            Self::Active(completion) => completion.diagnostics(),
            Self::Retained(completion) => completion.diagnostics(),
        }
    }

    pub(crate) fn auth_challenge_request_uri(&self) -> Option<Uri> {
        match self {
            Self::Active(completion) => completion.auth_challenge_request_uri(),
            Self::Retained(completion) => completion.auth_challenge_request_uri(),
        }
    }

    pub(crate) fn last_response(&self) -> Result<Option<Response>> {
        match self {
            Self::Active(completion) => completion.last_response(),
            Self::Retained(completion) => completion.last_response(),
        }
    }

    pub(crate) async fn wait_for_state(
        &self,
        target_state: TransactionState,
        timeout: std::time::Duration,
    ) -> bool {
        match self {
            Self::Active(completion) => completion.wait_for_state(target_state, timeout).await,
            Self::Retained(completion) => completion.reached_state(target_state),
        }
    }

    pub(crate) async fn wait_for_outcome(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<ClientTransactionOutcome>> {
        match self {
            Self::Active(completion) => completion.wait_for_outcome(timeout).await,
            Self::Retained(completion) => completion.outcome(),
        }
    }
}

/// Aggregate-safe diagnostic counts for exact completion cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CompletionCellDiagnostics {
    pub(crate) compact: usize,
    pub(crate) parsed_responses: usize,
    pub(crate) wire_responses: usize,
    /// Serialized response payload retained by compact completion records.
    ///
    /// This deliberately excludes container/node overhead. It is paired with
    /// the manager's table-capacity and inline-size diagnostics so memory
    /// profiles can distinguish real SIP payload retention from index cost.
    pub(crate) wire_response_bytes: usize,
}

/// Per-client-transaction exact result authority.
pub(crate) struct ClientTransactionCompletion {
    snapshot: Mutex<CompletionSnapshot>,
    notify: Notify,
}

impl ClientTransactionCompletion {
    pub(crate) fn new(initial_state: TransactionState) -> Self {
        Self {
            snapshot: Mutex::new(CompletionSnapshot {
                revision: 0,
                state: initial_state,
                visited_states: transaction_state_bit(initial_state),
                last_response: None,
                auth_challenge_request_uri: None,
                terminal_failure: None,
            }),
            notify: Notify::new(),
        }
    }

    /// Store a response before any corresponding public event is published.
    #[cfg(test)]
    pub(crate) fn record_response(&self, response: Response) {
        self.record_response_with_request_uri(response, None);
    }

    /// Store a response together with the exact URI from the request that
    /// crossed the wire. The URI is retained only for 401/407 challenges;
    /// authentication retry must not reconstruct it from mutable dialog
    /// state after the non-INVITE runner has compacted into Timer K state.
    pub(crate) fn record_response_for_request(&self, response: Response, request_uri: &Uri) {
        self.record_response_with_request_uri(response, Some(request_uri));
    }

    fn record_response_with_request_uri(&self, response: Response, request_uri: Option<&Uri>) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Terminal completion is first-writer-wins. A late final response
        // after an exact timeout/transport failure remains observable on the
        // public event stream, but cannot rewrite the result already returned
        // to protocol waiters.
        if snapshot.terminal_failure.is_some() || has_final_response(&snapshot.last_response) {
            return;
        }
        if response_has_auth_challenge(&response) {
            snapshot.auth_challenge_request_uri =
                request_uri.map(|request_uri| Arc::new(request_uri.clone()));
        }
        snapshot.last_response = Some(StoredResponse::Parsed(Arc::new(response)));
        snapshot.revision = snapshot.revision.wrapping_add(1);
        drop(snapshot);
        self.notify.notify_waiters();
    }

    /// Store a state transition before its public `StateChanged` event.
    pub(crate) fn record_state(&self, state: TransactionState) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if snapshot.state == state {
            return;
        }
        snapshot.state = state;
        snapshot.visited_states |= transaction_state_bit(state);
        snapshot.revision = snapshot.revision.wrapping_add(1);
        if state == TransactionState::Terminated
            && snapshot.terminal_failure.is_none()
            && !has_final_response(&snapshot.last_response)
        {
            snapshot.terminal_failure = Some(ClientTransactionFailure::Terminated);
        }
        drop(snapshot);
        self.notify.notify_waiters();
    }

    /// Store a typed terminal failure before its public timeout/error event.
    pub(crate) fn record_failure(&self, failure: ClientTransactionFailure) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if has_final_response(&snapshot.last_response) || snapshot.terminal_failure.is_some() {
            return;
        }
        snapshot.terminal_failure = Some(failure);
        snapshot.revision = snapshot.revision.wrapping_add(1);
        drop(snapshot);
        self.notify.notify_waiters();
    }

    /// Synchronously claim explicit owner cancellation before the manager
    /// retires the transaction runner.
    ///
    /// Sending the runner's `Terminate` command is intentionally nonblocking,
    /// so retirement cannot wait for the runner to update this cell. Keep the
    /// failure and terminal state in one critical section to ensure existing
    /// waiters and the immutable post-retirement snapshot observe one exact
    /// result. A final response or more-specific terminal failure that won the
    /// race remains authoritative.
    pub(crate) fn record_forced_termination(&self) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = false;
        if snapshot.terminal_failure.is_none() && !has_final_response(&snapshot.last_response) {
            snapshot.terminal_failure = Some(ClientTransactionFailure::Cancelled);
            changed = true;
        }
        if snapshot.state != TransactionState::Terminated {
            snapshot.state = TransactionState::Terminated;
            snapshot.visited_states |= transaction_state_bit(TransactionState::Terminated);
            changed = true;
        }
        if changed {
            snapshot.revision = snapshot.revision.wrapping_add(1);
        }
        drop(snapshot);
        if changed {
            self.notify.notify_waiters();
        }
    }

    /// Snapshot the exact terminal result into the immutable representation
    /// used after manager removal. The cell-local lock makes replacement with
    /// the shared wire image atomic with respect to response/state updates.
    pub(crate) fn retained(
        &self,
        expires_at: Instant,
        version: u64,
    ) -> RetainedClientTransactionCompletion {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored = snapshot.last_response.take();
        let wire = match stored {
            Some(StoredResponse::Parsed(response)) => {
                // Retirement owns the ordinary final-response Arc. Moving it
                // into the serializer avoids cloning the complete parsed
                // header tree on the high-CPS path. A concurrent compatibility
                // reader may briefly share the Arc; only that uncommon race
                // falls back to an exact deep clone.
                let response =
                    Arc::try_unwrap(response).unwrap_or_else(|shared| shared.as_ref().clone());
                Some(Bytes::from(Message::Response(response).to_bytes()))
            }
            Some(StoredResponse::Wire(wire)) => Some(wire),
            None => None,
        };
        // Existing waiters may retain this live cell after the manager swaps
        // its map value. Share the immutable wire allocation with them rather
        // than keeping a second parsed or serialized response.
        snapshot.last_response = wire.clone().map(StoredResponse::Wire);
        let retained = RetainedClientTransactionCompletion {
            revision: snapshot.revision,
            state: snapshot.state,
            visited_states: snapshot.visited_states,
            last_response: wire,
            auth_challenge_request_uri: snapshot.auth_challenge_request_uri.clone(),
            terminal_failure: snapshot.terminal_failure,
            expires_at,
            version,
            _admission_owner: None,
        };
        drop(snapshot);
        self.notify.notify_waiters();
        retained
    }

    /// Compact the terminal response together with an already serialized
    /// prefix into one immutable backing allocation.
    ///
    /// Retired INVITE state passes the original request wire image as the
    /// prefix. The returned prefix slice and the response stored in both the
    /// immutable record and any pre-existing live waiter then share a single
    /// allocation for the complete 90-second late-2xx horizon.
    pub(crate) fn retained_with_shared_prefix(
        &self,
        mut prefix: Vec<u8>,
        expires_at: Instant,
        version: u64,
    ) -> (Bytes, RetainedClientTransactionCompletion) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored = snapshot.last_response.take();
        let response_wire = match stored {
            Some(StoredResponse::Parsed(response)) => {
                let response =
                    Arc::try_unwrap(response).unwrap_or_else(|shared| shared.as_ref().clone());
                Some(Message::Response(response).to_bytes())
            }
            Some(StoredResponse::Wire(wire)) => Some(wire.to_vec()),
            None => None,
        };

        let prefix_len = prefix.len();
        if let Some(response_wire) = response_wire.as_ref() {
            prefix.reserve(response_wire.len());
            prefix.extend_from_slice(response_wire);
        }
        let shared = Bytes::from(prefix);
        let prefix = shared.slice(..prefix_len);
        let response = response_wire
            .as_ref()
            .map(|wire| shared.slice(prefix_len..prefix_len + wire.len()));

        snapshot.last_response = response.clone().map(StoredResponse::Wire);
        let retained = RetainedClientTransactionCompletion {
            revision: snapshot.revision,
            state: snapshot.state,
            visited_states: snapshot.visited_states,
            last_response: response,
            auth_challenge_request_uri: snapshot.auth_challenge_request_uri.clone(),
            terminal_failure: snapshot.terminal_failure,
            expires_at,
            version,
            _admission_owner: None,
        };
        drop(snapshot);
        self.notify.notify_waiters();
        (prefix, retained)
    }

    pub(crate) fn diagnostics(&self) -> CompletionCellDiagnostics {
        let snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &snapshot.last_response {
            Some(StoredResponse::Parsed(_)) => CompletionCellDiagnostics {
                parsed_responses: 1,
                ..CompletionCellDiagnostics::default()
            },
            Some(StoredResponse::Wire(wire)) => CompletionCellDiagnostics {
                compact: 1,
                wire_responses: 1,
                wire_response_bytes: wire.len(),
                ..CompletionCellDiagnostics::default()
            },
            None => CompletionCellDiagnostics::default(),
        }
    }

    pub(crate) fn last_response(&self) -> Result<Option<Response>> {
        let stored = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_response
            .clone();
        decode_response(stored)
    }

    pub(crate) fn auth_challenge_request_uri(&self) -> Option<Uri> {
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .auth_challenge_request_uri
            .as_deref()
            .cloned()
    }

    pub(crate) fn has_auth_challenge_request_uri(&self) -> bool {
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .auth_challenge_request_uri
            .is_some()
    }

    pub(crate) async fn wait_for_state(
        &self,
        target_state: TransactionState,
        timeout: std::time::Duration,
    ) -> bool {
        let wait = async {
            let mut observed_revision = None;
            loop {
                let notified = self.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                let (revision, state, reached) = {
                    let snapshot = self
                        .snapshot
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    (
                        snapshot.revision,
                        snapshot.state,
                        snapshot.visited_states & transaction_state_bit(target_state) != 0,
                    )
                };
                if reached {
                    return true;
                }
                if state == TransactionState::Terminated {
                    return false;
                }
                // A transition that raced between waiter construction and the
                // enabled Notify is already reflected in the visited-state
                // mask. Re-snapshot once rather than sleeping on an old
                // revision; an unchanged revision can safely await.
                if observed_revision.replace(revision) != Some(revision) {
                    continue;
                }
                notified.await;
            }
        };
        tokio::time::timeout(timeout, wait).await.unwrap_or(false)
    }

    pub(crate) async fn wait_for_outcome(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<ClientTransactionOutcome>> {
        let wait = async {
            loop {
                let notified = self.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                if let Some(outcome) = self.outcome()? {
                    return Ok(Some(outcome));
                }
                notified.await;
            }
        };
        match tokio::time::timeout(timeout, wait).await {
            Ok(outcome) => outcome,
            Err(_) => Ok(None),
        }
    }

    fn outcome(&self) -> Result<Option<ClientTransactionOutcome>> {
        let (stored, failure, state) = {
            let snapshot = self
                .snapshot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                snapshot.last_response.clone(),
                snapshot.terminal_failure,
                snapshot.state,
            )
        };
        outcome_from_snapshot(stored, failure, state)
    }
}

fn transaction_state_bit(state: TransactionState) -> u8 {
    1 << match state {
        TransactionState::Initial => 0,
        TransactionState::Calling => 1,
        TransactionState::Trying => 2,
        TransactionState::Proceeding => 3,
        TransactionState::Completed => 4,
        TransactionState::Confirmed => 5,
        TransactionState::Terminated => 6,
    }
}

fn outcome_from_snapshot(
    stored: Option<StoredResponse>,
    failure: Option<ClientTransactionFailure>,
    state: TransactionState,
) -> Result<Option<ClientTransactionOutcome>> {
    if let Some(response) = decode_response(stored)? {
        if response.status().as_u16() >= 200 {
            return Ok(Some(ClientTransactionOutcome::FinalResponse(response)));
        }
    }
    if let Some(failure) = failure {
        return Ok(Some(ClientTransactionOutcome::Failure(failure)));
    }
    if state == TransactionState::Terminated {
        return Ok(Some(ClientTransactionOutcome::Failure(
            ClientTransactionFailure::Terminated,
        )));
    }
    Ok(None)
}

fn has_final_response(response: &Option<StoredResponse>) -> bool {
    match response {
        Some(StoredResponse::Parsed(response)) => response.status().as_u16() >= 200,
        // A response is compacted only after retirement. If the cell has a
        // wire response at that point it is authoritative regardless of a
        // subsequent generic Terminated transition.
        Some(StoredResponse::Wire(_)) => true,
        None => false,
    }
}

fn response_has_auth_challenge(response: &Response) -> bool {
    let header = match response.status().as_u16() {
        401 => HeaderName::WwwAuthenticate,
        407 => HeaderName::ProxyAuthenticate,
        _ => return false,
    };
    response.raw_header_value(&header).is_some()
}

fn decode_response(stored: Option<StoredResponse>) -> Result<Option<Response>> {
    match stored {
        Some(StoredResponse::Parsed(response)) => Ok(Some(response.as_ref().clone())),
        Some(StoredResponse::Wire(wire)) => match rvoip_sip_core::parse_message(wire.as_ref())? {
            Message::Response(response) => Ok(Some(response)),
            Message::Request(_) => Err(Error::Other(
                "client completion response wire image parsed as a request".into(),
            )),
        },
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::auth::WwwAuthenticate;
    use rvoip_sip_core::StatusCode;
    use rvoip_sip_core::TypedHeader;

    #[tokio::test]
    async fn response_before_wait_is_exact_and_compactable() {
        let cell = ClientTransactionCompletion::new(TransactionState::Trying);
        let response = Response::new(StatusCode::Ok)
            .with_reason("Exact terminal response")
            .with_body(Bytes::from_static(b"terminal-body"));
        cell.record_response(response.clone());
        cell.record_state(TransactionState::Completed);

        let outcome = cell
            .wait_for_outcome(std::time::Duration::from_millis(10))
            .await
            .expect("valid response")
            .expect("terminal outcome");
        assert!(matches!(
            outcome,
            ClientTransactionOutcome::FinalResponse(_)
        ));

        let expires_at = Instant::now() + std::time::Duration::from_secs(90);
        let retained = cell.retained(expires_at, 17);
        assert_eq!(
            retained.diagnostics(),
            CompletionCellDiagnostics {
                compact: 1,
                parsed_responses: 0,
                wire_responses: 1,
                wire_response_bytes: retained
                    .last_response
                    .as_ref()
                    .expect("retained wire response")
                    .len(),
            }
        );
        assert_eq!(retained.deadline(), (expires_at, 17));
        assert_eq!(retained.last_response().unwrap(), Some(response.clone()));
        assert!(matches!(
            retained.outcome().unwrap(),
            Some(ClientTransactionOutcome::FinalResponse(exact)) if exact == response
        ));
        let cell_wire = {
            let snapshot = cell
                .snapshot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match snapshot.last_response.as_ref() {
                Some(StoredResponse::Wire(wire)) => wire.clone(),
                Some(StoredResponse::Parsed(_)) | None => panic!("cell was not compacted"),
            }
        };
        assert_eq!(
            cell.diagnostics(),
            CompletionCellDiagnostics {
                compact: 1,
                parsed_responses: 0,
                wire_responses: 1,
                wire_response_bytes: cell_wire.len(),
            },
            "the live race handle must share the compact wire representation"
        );
        assert_eq!(
            cell_wire.as_ptr(),
            retained
                .last_response
                .as_ref()
                .expect("retained wire response")
                .as_ptr(),
            "cell and retained record must share one wire allocation"
        );
    }

    #[test]
    fn auth_challenge_uri_is_pointer_sized_and_survives_compaction() {
        assert_eq!(
            std::mem::size_of::<Option<Arc<Uri>>>(),
            std::mem::size_of::<usize>(),
            "ordinary completion records must pay only one inline pointer for rare auth state"
        );

        let request_uri = "sip:transfer-target@example.com:5061;transport=udp"
            .parse::<Uri>()
            .expect("valid exact request URI");
        let completion = Arc::new(ClientTransactionCompletion::new(TransactionState::Trying));
        assert_eq!(completion.auth_challenge_request_uri(), None);

        let mut challenge = Response::new(StatusCode::Unauthorized).with_reason("Authenticate");
        challenge
            .headers
            .push(TypedHeader::WwwAuthenticate(WwwAuthenticate::new(
                "example", "nonce",
            )));
        completion.record_response_for_request(challenge, &request_uri);
        assert_eq!(
            completion.auth_challenge_request_uri().as_ref(),
            Some(&request_uri),
            "the active exact-completion cell must own the wire Request-URI"
        );

        let retained = completion.retained(Instant::now() + std::time::Duration::from_secs(5), 41);
        assert_eq!(
            retained.auth_challenge_request_uri().as_ref(),
            Some(&request_uri),
            "the compact successor must close the runner-removal race"
        );
        assert_eq!(
            ClientTransactionCompletionEntry::Retained(retained)
                .auth_challenge_request_uri()
                .as_ref(),
            Some(&request_uri),
            "manager lookups must resolve auth state through retained completion"
        );

        let ordinary = ClientTransactionCompletion::new(TransactionState::Trying);
        ordinary.record_response_for_request(Response::new(StatusCode::Ok), &request_uri);
        let ordinary_retained =
            ordinary.retained(Instant::now() + std::time::Duration::from_secs(5), 42);
        assert_eq!(
            ordinary_retained.auth_challenge_request_uri(),
            None,
            "non-challenge completions must not allocate or retain a URI"
        );
    }

    #[tokio::test]
    async fn terminal_failure_wakes_waiter_without_an_event_subscription() {
        let cell = Arc::new(ClientTransactionCompletion::new(TransactionState::Trying));
        let waiter = {
            let cell = cell.clone();
            tokio::spawn(async move {
                cell.wait_for_outcome(std::time::Duration::from_secs(1))
                    .await
            })
        };
        tokio::task::yield_now().await;
        cell.record_failure(ClientTransactionFailure::Transport);
        cell.record_state(TransactionState::Terminated);

        assert!(matches!(
            waiter.await.unwrap().unwrap(),
            Some(ClientTransactionOutcome::Failure(
                ClientTransactionFailure::Transport
            ))
        ));
    }

    #[tokio::test]
    async fn state_wait_observes_a_reached_state_after_a_later_transition() {
        let cell = ClientTransactionCompletion::new(TransactionState::Trying);
        cell.record_state(TransactionState::Proceeding);
        cell.record_state(TransactionState::Completed);
        cell.record_state(TransactionState::Terminated);

        assert!(
            cell.wait_for_state(
                TransactionState::Completed,
                std::time::Duration::from_millis(10)
            )
            .await,
            "the exact completion cell must retain reached-state evidence"
        );
        let revision = cell
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revision;
        assert_eq!(revision, 3, "each state transition advances the revision");

        let retained = cell.retained(Instant::now() + std::time::Duration::from_secs(90), 29);
        assert!(retained.reached_state(TransactionState::Completed));
        assert_eq!(retained.revision, revision);
    }

    #[test]
    fn shared_prefix_compaction_uses_one_backing_allocation() {
        let cell = ClientTransactionCompletion::new(TransactionState::Completed);
        let response = Response::new(StatusCode::BusyHere)
            .with_reason("Retained response")
            .with_body(Bytes::from_static(b"response-body"));
        cell.record_response(response.clone());
        let request_wire =
            b"INVITE sip:bob@example.com SIP/2.0\r\nContent-Length: 0\r\n\r\n".to_vec();
        let request_len = request_wire.len();

        let (request, retained) = cell.retained_with_shared_prefix(
            request_wire.clone(),
            Instant::now() + std::time::Duration::from_secs(90),
            23,
        );
        let response_wire = retained
            .last_response
            .as_ref()
            .expect("retained response wire");
        assert_eq!(request.as_ref(), request_wire.as_slice());
        assert_eq!(
            response_wire.as_ptr(),
            request.as_ptr().wrapping_add(request_len),
            "request and response must be adjacent slices of one allocation"
        );
        assert_eq!(retained.last_response().unwrap(), Some(response));
        assert_eq!(
            cell.last_response().unwrap(),
            retained.last_response().unwrap()
        );
    }

    #[tokio::test]
    async fn first_terminal_outcome_wins_in_both_orders() {
        let failure_first = ClientTransactionCompletion::new(TransactionState::Trying);
        failure_first.record_failure(ClientTransactionFailure::Timeout);
        failure_first.record_response(Response::new(StatusCode::Ok));
        assert!(matches!(
            failure_first
                .wait_for_outcome(std::time::Duration::from_millis(10))
                .await
                .unwrap(),
            Some(ClientTransactionOutcome::Failure(
                ClientTransactionFailure::Timeout
            ))
        ));

        let response_first = ClientTransactionCompletion::new(TransactionState::Trying);
        response_first.record_response(Response::new(StatusCode::Ok));
        response_first.record_failure(ClientTransactionFailure::Transport);
        assert!(matches!(
            response_first
                .wait_for_outcome(std::time::Duration::from_millis(10))
                .await
                .unwrap(),
            Some(ClientTransactionOutcome::FinalResponse(response))
                if response.status().as_u16() == 200
        ));
    }

    #[test]
    fn late_provisional_cannot_replace_first_final_response() {
        let cell = ClientTransactionCompletion::new(TransactionState::Trying);
        cell.record_response(Response::new(StatusCode::Ok).with_reason("first final"));
        cell.record_response(Response::new(StatusCode::Ringing).with_reason("late provisional"));
        assert!(matches!(
            cell.outcome().unwrap(),
            Some(ClientTransactionOutcome::FinalResponse(response))
                if response.status().as_u16() == 200 && response.reason_phrase() == "first final"
        ));
    }

    #[test]
    fn conflicting_late_final_cannot_replace_first_final_response() {
        let cell = ClientTransactionCompletion::new(TransactionState::Calling);
        cell.record_response(Response::new(StatusCode::Ok).with_reason("winning fork"));
        cell.record_response(Response::new(StatusCode::ServerInternalError).with_reason("late"));
        cell.record_response(Response::new(StatusCode::Accepted).with_reason("late fork"));
        assert!(matches!(
            cell.outcome().unwrap(),
            Some(ClientTransactionOutcome::FinalResponse(response))
                if response.status().as_u16() == 200 && response.reason_phrase() == "winning fork"
        ));
    }

    #[test]
    fn terminal_failure_cannot_be_rewritten_by_late_success() {
        let cell = ClientTransactionCompletion::new(TransactionState::Calling);
        cell.record_response(Response::new(StatusCode::Ringing));
        cell.record_failure(ClientTransactionFailure::Timeout);
        cell.record_response(Response::new(StatusCode::Ok));
        assert!(matches!(
            cell.outcome().unwrap(),
            Some(ClientTransactionOutcome::Failure(
                ClientTransactionFailure::Timeout
            ))
        ));
    }
}
