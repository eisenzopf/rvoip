//! [`IncomingReinvite`] — the application-controlled re-INVITE handle.
//!
//! Only exists when [`crate::api::unified::ReinvitePolicy::ApplicationControlled`]
//! is configured. See that type's doc comment for exactly which
//! re-INVITE/UPDATE shapes go through it (short version: only a re-INVITE
//! that carries SDP; a bodyless re-INVITE and every UPDATE are always
//! automatic).

use std::sync::Arc;

use crate::api::handle::CallId;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::session_registry::SessionRegistryHandle;
use crate::session_store::state::PendingIncomingReinvite;

/// A re-INVITE carrying SDP, held for an application decision.
///
/// The call keeps running on its previously negotiated media the entire
/// time this is outstanding; nothing about the existing dialog changes
/// until [`Self::accept_with_answer`] or [`Self::reject`] resolves it.
/// Resolving twice, or resolving after the underlying session ended,
/// returns a deterministic `Err` rather than silently doing nothing or
/// sending a second response for the same transaction.
pub struct IncomingReinvite {
    call_id: CallId,
    sdp: String,
    coordinator: Arc<UnifiedCoordinator>,
    lifecycle_handle: SessionRegistryHandle,
}

impl IncomingReinvite {
    pub(crate) fn new(
        call_id: CallId,
        sdp: String,
        coordinator: Arc<UnifiedCoordinator>,
        lifecycle_handle: SessionRegistryHandle,
    ) -> Self {
        Self {
            call_id,
            sdp,
            coordinator,
            lifecycle_handle,
        }
    }

    /// Look up the still-pending decision for `call_id`, after receiving
    /// [`crate::api::events::Event::IncomingReinvite`] carrying that
    /// `call_id`. Returns `None` if the application already resolved it
    /// (or another `IncomingReinvite` instance for the same event beat
    /// this one to it) or if the session has since ended.
    pub fn for_call(coordinator: Arc<UnifiedCoordinator>, call_id: CallId) -> Option<Self> {
        let handle = coordinator
            .helpers
            .state_machine
            .store
            .lifecycle_handle(&call_id)?;
        let sdp = coordinator
            .helpers
            .state_machine
            .store
            .get_session_snapshot_exact(&handle)
            .ok()?
            .pending_incoming_reinvite
            .as_ref()?
            .offered_sdp
            .clone();
        Some(Self::new(call_id, sdp, coordinator, handle))
    }

    /// The dialog this re-INVITE arrived on.
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// The peer's offer.
    pub fn sdp(&self) -> &str {
        &self.sdp
    }

    /// Accept, answering with `answer_sdp` exactly as supplied.
    ///
    /// This mirrors [`crate::api::unified::UnifiedCoordinator::accept_call_with_sdp`]'s
    /// contract: the state machine trusts the application's answer rather
    /// than negotiating one itself, so `answer_sdp` must already be a
    /// valid RFC 3264 answer to [`Self::sdp`]. This is the right shape for
    /// a B2BUA that derives the answer from another leg rather than from
    /// this process's own media stack.
    pub async fn accept_with_answer(self, answer_sdp: String) -> Result<()> {
        let pending = self.take_pending()?;
        let dialog_adapter = Arc::clone(self.coordinator.dialog_adapter());
        crate::state_machine::actions::send_exact_sip_response_on_fresh_task(
            dialog_adapter,
            self.lifecycle_handle.session_id().clone(),
            pending.transaction_id,
            200,
            Some(answer_sdp.clone()),
            None,
        )
        .await
        .map_err(|error| SessionError::DialogError(error.to_string()))?;

        self.coordinator
            .helpers
            .state_machine
            .store
            .update_session_exact_with(&self.lifecycle_handle, None, |session| {
                session.local_sdp = Some(answer_sdp);
                session.remote_sdp = Some(pending.offered_sdp.clone());
                session.pending_remote_offer = None;
                session.sdp_negotiated = true;
            })
            .map_err(|error| SessionError::InvalidTransition(error.to_string()))?;
        Ok(())
    }

    /// Reject. The call keeps using its previously negotiated media; per
    /// RFC 3264, a rejected offer/answer exchange never tears down the
    /// dialog.
    ///
    /// `status` must be 3xx-6xx. `reason` is carried as an RFC 3261
    /// Warning header (399) rather than the status-line reason phrase,
    /// since the exact-response primitive this uses always sends the
    /// standard phrase for `status`.
    pub async fn reject(self, status: u16, reason: &str) -> Result<()> {
        if !(300..=699).contains(&status) {
            return Err(SessionError::InvalidInput(
                "IncomingReinvite::reject status must be 3xx-6xx".to_string(),
            ));
        }
        let pending = self.take_pending()?;
        let dialog_adapter = Arc::clone(self.coordinator.dialog_adapter());
        let extra_headers = (!reason.is_empty()).then(|| {
            vec![rvoip_sip_core::types::TypedHeader::Warning(vec![
                rvoip_sip_core::types::warning::Warning::new(
                    399,
                    rvoip_sip_core::types::uri::Uri::sip("rvoip"),
                    reason.to_string(),
                ),
            ])]
        });
        crate::state_machine::actions::send_exact_sip_response_on_fresh_task(
            dialog_adapter,
            self.lifecycle_handle.session_id().clone(),
            pending.transaction_id,
            status,
            None,
            extra_headers,
        )
        .await
        .map_err(|error| SessionError::DialogError(error.to_string()))?;
        // The failed offer is dropped without touching remote_sdp/local_sdp/
        // negotiated_config: RFC 3264 atomicity, same invariant the
        // automatic path (NegotiateSDPAsUAS) keeps.
        let _ = self.coordinator.helpers.state_machine.store.update_session_exact_with(
            &self.lifecycle_handle,
            None,
            |session| {
                session.pending_remote_offer = None;
            },
        );
        Ok(())
    }

    /// Takes the pending decision, so a second `accept_with_answer`/
    /// `reject` call (whether on this instance reused via a bug, or on a
    /// second `IncomingReinvite` built from the same event) gets a
    /// deterministic error instead of double-responding on the same
    /// transaction.
    fn take_pending(&self) -> Result<PendingIncomingReinvite> {
        self.coordinator
            .helpers
            .state_machine
            .store
            .update_session_exact_with(&self.lifecycle_handle, None, |session| {
                session.pending_incoming_reinvite.take()
            })
            .map_err(|error| SessionError::InvalidTransition(error.to_string()))?
            .ok_or_else(|| {
                SessionError::InvalidTransition(
                    "IncomingReinvite already resolved or the session ended".to_string(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::Config;
    use crate::state_machine::actions::exact_response_dispatch_test_hook::{self, Step};
    use crate::state_table::types::{Role, SessionId};

    async fn coordinator(name: &str) -> Arc<UnifiedCoordinator> {
        UnifiedCoordinator::new(Config::local(name, 0))
            .await
            .expect("create test coordinator")
    }

    fn fake_transaction(branch: &str) -> rvoip_sip_dialog::transaction::TransactionKey {
        rvoip_sip_dialog::transaction::TransactionKey::new(
            branch.to_string(),
            rvoip_sip_core::Method::Invite,
            true,
        )
    }

    async fn session_with_pending_reinvite(
        coord: &Arc<UnifiedCoordinator>,
        session_id: &SessionId,
        transaction: rvoip_sip_dialog::transaction::TransactionKey,
        offered_sdp: &str,
    ) {
        let offered_sdp = offered_sdp.to_string();
        coord
            .helpers
            .state_machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAS, false, move |session| {
                session.call_state = crate::types::CallState::Active;
                session.local_sdp = Some("v=0\r\na=x-committed-local\r\n".to_string());
                session.remote_sdp = Some("v=0\r\na=x-committed-remote\r\n".to_string());
                session.sdp_negotiated = true;
                session.pending_incoming_reinvite = Some(PendingIncomingReinvite {
                    transaction_id: transaction,
                    offered_sdp,
                });
            })
            .await
            .expect("create session with pending re-INVITE");
    }

    #[tokio::test]
    async fn for_call_returns_none_without_a_pending_reinvite() {
        let coord = coordinator("incoming-reinvite-none").await;
        let session_id = SessionId("incoming-reinvite-none-session".to_string());
        coord
            .helpers
            .state_machine
            .store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create plain session");

        assert!(IncomingReinvite::for_call(Arc::clone(&coord), session_id.clone()).is_none());

        coord
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown coordinator");
    }

    #[tokio::test]
    async fn accept_with_answer_sends_200_and_commits_the_supplied_sdp() {
        let coord = coordinator("incoming-reinvite-accept").await;
        let session_id = SessionId("incoming-reinvite-accept-session".to_string());
        let transaction = fake_transaction("z9hG4bK-incoming-reinvite-accept");
        session_with_pending_reinvite(
            &coord,
            &session_id,
            transaction.clone(),
            "v=0\r\na=x-offer\r\n",
        )
        .await;

        let script =
            exact_response_dispatch_test_hook::install(&session_id, vec![Step::Written]);

        let reinvite = IncomingReinvite::for_call(Arc::clone(&coord), session_id.clone())
            .expect("pending re-INVITE is visible");
        assert_eq!(reinvite.sdp(), "v=0\r\na=x-offer\r\n");
        reinvite
            .accept_with_answer("v=0\r\na=x-answer\r\n".to_string())
            .await
            .expect("accept_with_answer sends 200 OK");

        assert_eq!(script.dispatches(), vec![(transaction, 200)]);

        let handle = coord
            .helpers
            .state_machine
            .store
            .lifecycle_handle(&session_id)
            .expect("session still present");
        let snapshot = coord
            .helpers
            .state_machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read post-accept snapshot");
        assert_eq!(snapshot.local_sdp.as_deref(), Some("v=0\r\na=x-answer\r\n"));
        assert_eq!(
            snapshot.remote_sdp.as_deref(),
            Some("v=0\r\na=x-offer\r\n")
        );
        assert!(snapshot.sdp_negotiated);
        assert!(snapshot.pending_incoming_reinvite.is_none());

        // Double-resolution is a deterministic error, not a second 200 OK.
        assert!(IncomingReinvite::for_call(Arc::clone(&coord), session_id.clone()).is_none());

        exact_response_dispatch_test_hook::remove(&session_id);
        coord
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown coordinator");
    }

    #[tokio::test]
    async fn reject_sends_the_given_status_and_preserves_committed_sdp() {
        let coord = coordinator("incoming-reinvite-reject").await;
        let session_id = SessionId("incoming-reinvite-reject-session".to_string());
        let transaction = fake_transaction("z9hG4bK-incoming-reinvite-reject");
        session_with_pending_reinvite(
            &coord,
            &session_id,
            transaction.clone(),
            "v=0\r\na=x-offer\r\n",
        )
        .await;

        let script =
            exact_response_dispatch_test_hook::install(&session_id, vec![Step::Written]);

        let reinvite = IncomingReinvite::for_call(Arc::clone(&coord), session_id.clone())
            .expect("pending re-INVITE is visible");
        reinvite
            .reject(488, "codec not supported")
            .await
            .expect("reject sends the given status");

        assert_eq!(script.dispatches(), vec![(transaction, 488)]);

        let handle = coord
            .helpers
            .state_machine
            .store
            .lifecycle_handle(&session_id)
            .expect("session still present");
        let snapshot = coord
            .helpers
            .state_machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read post-reject snapshot");
        // RFC 3264: a rejected offer/answer exchange leaves the call on its
        // previously committed media untouched.
        assert_eq!(
            snapshot.local_sdp.as_deref(),
            Some("v=0\r\na=x-committed-local\r\n")
        );
        assert_eq!(
            snapshot.remote_sdp.as_deref(),
            Some("v=0\r\na=x-committed-remote\r\n")
        );
        assert!(snapshot.pending_incoming_reinvite.is_none());

        exact_response_dispatch_test_hook::remove(&session_id);
        coord
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown coordinator");
    }

    #[tokio::test]
    async fn reject_rejects_an_out_of_range_status_without_touching_the_wire() {
        let coord = coordinator("incoming-reinvite-reject-bad-status").await;
        let session_id = SessionId("incoming-reinvite-reject-bad-status-session".to_string());
        let transaction = fake_transaction("z9hG4bK-incoming-reinvite-reject-bad-status");
        session_with_pending_reinvite(&coord, &session_id, transaction, "v=0\r\na=x-offer\r\n")
            .await;

        // No script installed: if `reject` tried to dispatch anything, the
        // real (non-test) transport path would run and this would fail
        // for an unrelated reason instead of the validation error we want.
        let reinvite = IncomingReinvite::for_call(Arc::clone(&coord), session_id.clone())
            .expect("pending re-INVITE is visible");
        let error = reinvite
            .reject(199, "not a final response")
            .await
            .expect_err("199 is not a valid final-response status");
        assert!(matches!(error, SessionError::InvalidInput(_)));

        // The pending decision is still there, untouched by the failed call.
        let handle = coord
            .helpers
            .state_machine
            .store
            .lifecycle_handle(&session_id)
            .expect("session still present");
        let snapshot = coord
            .helpers
            .state_machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read snapshot");
        assert!(snapshot.pending_incoming_reinvite.is_some());

        coord
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown coordinator");
    }
}
