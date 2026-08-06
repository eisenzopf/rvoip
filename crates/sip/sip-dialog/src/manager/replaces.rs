//! RFC 3891 dialog matching for the `Replaces` header
//!
//! This is the receiving half of attended transfer. The transferee sends an
//! INVITE carrying `Replaces:`, and this module answers the only question the
//! UAS has to settle before anything else happens: which of our dialogs does
//! that header name, and what is its state?
//!
//! ## The perspective flip
//!
//! RFC 3891 §3 puts it directly:
//!
//! > The UAS matches the to-tag and from-tag parameters as if they were tags
//! > present in an incoming request. In other words, the to-tag parameter is
//! > compared to the local tag, and the from-tag parameter is compared to the
//! > remote tag.
//!
//! So `to-tag` is *our* tag and `from-tag` is the peer's. The lookup key is
//! therefore `call-id:to-tag:from-tag`, never the reverse. Getting this
//! backwards builds a perfectly well formed key that matches nothing, and the
//! transfer then fails as "no such dialog" rather than as an obvious defect,
//! which is why `Replaces::as_local_remote_tags` exists to carry the order.
//!
//! ## Which index holds what
//!
//! `store_dialog` files a dialog under `dialog_lookup` as soon as both tags
//! are known, so that map holds confirmed dialogs *and* early dialogs we
//! initiated as UAC (we have our From tag from the start, and the peer's tag
//! arrives with the 18x). `early_dialog_lookup` holds the other case, an early
//! dialog we did not initiate, keyed on the remote tag alone.
//!
//! That asymmetry is harmless here. §3 answers 481 for an early dialog we did
//! not initiate, and 481 is also the answer when nothing matches, so both
//! roads lead to the same response.

use rvoip_sip_core::types::replaces::Replaces;
use rvoip_sip_core::{HeaderName, Method, Request, StatusCode};
use tracing::{debug, info, warn};

use crate::dialog::{DialogId, DialogState};
use crate::manager::utils::DialogUtils;
use crate::manager::DialogManager;
use crate::transaction::TransactionKey;

/// What a `Replaces` header resolved to, in the vocabulary of RFC 3891 §3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplacesMatch {
    /// A confirmed dialog. Accept the new INVITE, then BYE this one.
    Confirmed(DialogId),
    /// An early dialog this UA initiated. Accept the new INVITE, then CANCEL
    /// this one.
    EarlyLocallyInitiated(DialogId),
    /// An early dialog this UA did not initiate. §3 rejects with 481 and
    /// leaves the dialog untouched.
    EarlyRemotelyInitiated,
    /// A dialog that matched but has already terminated. §3 asks for 603
    /// rather than 481, to avoid alerting for a call that is already gone.
    Terminated,
    /// Nothing matched, or more than one dialog did.
    NoMatch,
}

impl ReplacesMatch {
    /// The status code §3 requires when this match cannot be replaced, or
    /// `None` when the replacement should go ahead.
    ///
    /// `early_only` is the flag from the header. §3 only consults it for a
    /// confirmed match, where it means the transferor wanted to replace a
    /// ringing call and is no longer interested now that the call has been
    /// answered by someone else.
    pub(crate) fn rejection_status(&self, early_only: bool) -> Option<StatusCode> {
        match self {
            ReplacesMatch::Confirmed(_) if early_only => Some(StatusCode::BusyHere),
            ReplacesMatch::Confirmed(_) | ReplacesMatch::EarlyLocallyInitiated(_) => None,
            ReplacesMatch::EarlyRemotelyInitiated | ReplacesMatch::NoMatch => {
                Some(StatusCode::CallOrTransactionDoesNotExist)
            }
            ReplacesMatch::Terminated => Some(StatusCode::Decline),
        }
    }

    /// The dialog to shut down once the new INVITE has been accepted.
    pub(crate) fn dialog_to_replace(&self) -> Option<&DialogId> {
        match self {
            ReplacesMatch::Confirmed(id) | ReplacesMatch::EarlyLocallyInitiated(id) => Some(id),
            _ => None,
        }
    }
}

/// What the initial-INVITE path should do about the `Replaces` header on a
/// request, before it creates any dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplacesDisposition {
    /// No `Replaces` header. An ordinary INVITE.
    Absent,
    /// Reject the INVITE with this status and leave every dialog untouched.
    Reject(StatusCode),
    /// Go ahead, and shut this dialog down once the new one is confirmed.
    Replace(DialogId),
}

impl DialogManager {
    /// Decide what an inbound INVITE's `Replaces` header means, per RFC 3891 §3.
    ///
    /// Runs before any dialog is created, so a rejection costs nothing and
    /// leaves no state behind.
    ///
    /// A malformed value is distinguishable from an absent one because the
    /// message parser leaves an unparseable `Replaces` in place as a raw
    /// header while a valid one becomes the typed variant. §3 requires 400 for
    /// the malformed case, which is why the two are not collapsed.
    pub(crate) fn evaluate_replaces(&self, request: &Request) -> ReplacesDisposition {
        let header_count = request
            .all_headers()
            .iter()
            .filter(|header| header.name() == HeaderName::Replaces)
            .count();

        match header_count {
            0 => return ReplacesDisposition::Absent,
            1 => {}
            // "If more than one Replaces header field is present in an INVITE
            // [...] the UAS MUST reject the request with a 400 Bad Request."
            _ => {
                debug!(
                    "INVITE carries {} Replaces headers, rejecting with 400",
                    header_count
                );
                return ReplacesDisposition::Reject(StatusCode::BadRequest);
            }
        }

        let Some(replaces) = request.typed_header::<Replaces>() else {
            debug!("INVITE carries an unparseable Replaces header, rejecting with 400");
            return ReplacesDisposition::Reject(StatusCode::BadRequest);
        };

        let matched = self.resolve_replaces(replaces);
        match matched.rejection_status(replaces.early_only) {
            Some(status) => {
                debug!(
                    "Replaces resolved to {:?}, rejecting INVITE with {}",
                    matched, status
                );
                ReplacesDisposition::Reject(status)
            }
            None => match matched.dialog_to_replace() {
                Some(dialog_id) => ReplacesDisposition::Replace(dialog_id.clone()),
                // Unreachable by construction: every variant without a
                // rejection status carries a dialog. Fail closed rather than
                // panicking on a future variant.
                None => ReplacesDisposition::Reject(StatusCode::CallOrTransactionDoesNotExist),
            },
        }
    }

    /// Resolve a `Replaces` header against our dialogs, per RFC 3891 §3.
    pub(crate) fn resolve_replaces(&self, replaces: &Replaces) -> ReplacesMatch {
        let (local_tag, remote_tag) = replaces.as_local_remote_tags();
        let key = DialogUtils::create_lookup_key(&replaces.call_id, local_tag, remote_tag);

        let Some(dialog_id) = self
            .dialog_lookup
            .get(&key)
            .map(|entry| entry.value().clone())
        else {
            debug!(
                "Replaces matched no dialog for Call-ID {}",
                replaces.call_id
            );
            return ReplacesMatch::NoMatch;
        };

        // The index can outlive the dialog it points at. Treat a dangling
        // entry as no match rather than trusting the key.
        let Ok(dialog) = self.get_dialog(&dialog_id) else {
            return ReplacesMatch::NoMatch;
        };

        match dialog.state {
            DialogState::Confirmed => ReplacesMatch::Confirmed(dialog_id),
            DialogState::Early | DialogState::Initial => {
                if dialog.is_initiator {
                    ReplacesMatch::EarlyLocallyInitiated(dialog_id)
                } else {
                    ReplacesMatch::EarlyRemotelyInitiated
                }
            }
            // A dialog in recovery is still a live call from the user's point
            // of view, so it is replaceable like any confirmed one.
            DialogState::Recovering => ReplacesMatch::Confirmed(dialog_id),
            DialogState::Terminated => ReplacesMatch::Terminated,
        }
    }

    /// Record that `new_dialog_id` is to displace `replaced_dialog_id` once it
    /// is confirmed.
    pub(crate) fn register_pending_replacement(
        &self,
        new_dialog_id: &DialogId,
        replaced_dialog_id: &DialogId,
    ) {
        self.replaces_pending
            .insert(new_dialog_id.clone(), replaced_dialog_id.clone());
        debug!(
            "Dialog {} will replace dialog {} once confirmed (RFC 3891)",
            new_dialog_id, replaced_dialog_id
        );
    }

    /// Drop a pending replacement without acting on it.
    ///
    /// Called when the new INVITE ends in anything other than a 2xx. RFC 3891
    /// §3: "If the UA cannot accept the new INVITE [...] the UA MUST return an
    /// appropriate error response and MUST leave the matched dialog
    /// unchanged."
    pub(crate) fn discard_pending_replacement(&self, new_dialog_id: &DialogId) {
        if let Some((_, replaced)) = self.replaces_pending.remove(new_dialog_id) {
            debug!(
                "Dialog {} was not accepted, leaving replaced dialog {} unchanged",
                new_dialog_id, replaced
            );
        }
    }

    /// The client INVITE transaction of an early dialog we initiated, which is
    /// what a CANCEL has to target.
    fn cancellable_invite_transaction(&self, dialog_id: &DialogId) -> Option<TransactionKey> {
        self.dialog_invite_transactions
            .get(dialog_id)
            .and_then(|entry| {
                entry
                    .value()
                    .iter()
                    .rev()
                    .find(|key| !key.is_server())
                    .cloned()
            })
    }

    /// Shut down the dialog that `new_dialog_id` replaced, now that the new
    /// dialog has been confirmed.
    ///
    /// The ordering here is the whole point of RFC 3891 §3: "it accepts the
    /// new INVITE by sending a 200-class response, and shuts down the replaced
    /// dialog by sending a BYE". Doing it the other way round would leave the
    /// user with no call at all if the new INVITE then failed.
    ///
    /// An early dialog we initiated is torn down with CANCEL instead, since it
    /// has no 2xx to BYE.
    pub(crate) async fn complete_pending_replacement(&self, new_dialog_id: &DialogId) {
        let Some((_, replaced_dialog_id)) = self.replaces_pending.remove(new_dialog_id) else {
            return;
        };

        let state = match self.get_dialog(&replaced_dialog_id) {
            Ok(dialog) => dialog.state,
            Err(_) => {
                debug!(
                    "Replaced dialog {} is already gone, nothing to shut down",
                    replaced_dialog_id
                );
                return;
            }
        };

        let outcome = match state {
            // An early dialog has no 2xx to BYE, so §3 tears it down with a
            // CANCEL of its own INVITE. `dialog_invite_transactions` is the
            // sanctioned reverse index for exactly this.
            DialogState::Early | DialogState::Initial => {
                match self.cancellable_invite_transaction(&replaced_dialog_id) {
                    Some(invite_tx_id) => self
                        .cancel_invite_transaction_with_dialog(&invite_tx_id)
                        .await
                        .map(|_| "CANCEL")
                        .map_err(|error| error.to_string()),
                    None => Err(format!(
                        "no client INVITE transaction remains for early dialog {}",
                        replaced_dialog_id
                    )),
                }
            }
            DialogState::Terminated => {
                debug!(
                    "Replaced dialog {} terminated on its own before the replacement completed",
                    replaced_dialog_id
                );
                return;
            }
            _ => self
                .send_request(&replaced_dialog_id, Method::Bye, None)
                .await
                .map(|_| "BYE")
                .map_err(|error| error.to_string()),
        };

        match outcome {
            Ok(method) => info!(
                "Dialog {} replaced dialog {}, sent {} (RFC 3891 §3)",
                new_dialog_id, replaced_dialog_id, method
            ),
            // The new call is already up at this point. Failing to tear the
            // old one down is worth shouting about, but it must not unwind the
            // call the user is now talking on.
            Err(error) => warn!(
                "Dialog {} replaced dialog {} but shutting the old one down failed: {}",
                new_dialog_id, replaced_dialog_id, error
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::Dialog;
    use crate::transaction::TransactionManager;
    use async_trait::async_trait;
    use rvoip_sip_core::{parse_message, Message, Uri};
    use rvoip_sip_transport::error::Result as TransportResult;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    /// Transport that records everything written, so a test can assert that a
    /// BYE actually reached the wire rather than that a function returned Ok.
    #[derive(Debug)]
    struct RecordingTransport {
        addr: SocketAddr,
        sent: Mutex<Vec<Message>>,
    }

    impl RecordingTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                addr: SocketAddr::from_str("127.0.0.1:5060").unwrap(),
                sent: Mutex::new(Vec::new()),
            })
        }

        fn methods_sent(&self) -> Vec<Method> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .filter_map(|message| match message {
                    Message::Request(request) => Some(request.method()),
                    Message::Response(_) => None,
                })
                .collect()
        }
    }

    #[async_trait]
    impl Transport for RecordingTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }

        async fn send_message(
            &self,
            message: Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            self.sent.lock().unwrap().push(message);
            Ok(())
        }

        async fn close(&self) -> TransportResult<()> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    async fn make_manager_with(transport: Arc<RecordingTransport>) -> DialogManager {
        let (_tx, transport_rx) = mpsc::channel::<TransportEvent>(16);
        let (transaction_manager, _events_rx) =
            TransactionManager::new(transport, transport_rx, Some(16))
                .await
                .expect("build TransactionManager");
        DialogManager::new(
            Arc::new(transaction_manager),
            SocketAddr::from_str("127.0.0.1:5060").unwrap(),
        )
        .await
        .expect("build DialogManager")
    }

    async fn make_manager() -> DialogManager {
        make_manager_with(RecordingTransport::new()).await
    }

    /// Charlie's view of the consultation call with Bob: Charlie's own tag is
    /// the local one, Bob's is remote.
    async fn store_dialog_in_state(
        manager: &DialogManager,
        call_id: &str,
        local_tag: &str,
        remote_tag: &str,
        state: DialogState,
        is_initiator: bool,
    ) -> DialogId {
        let mut dialog = Dialog::new(
            call_id.to_string(),
            Uri::from_str("sip:charlie@example.test").unwrap(),
            Uri::from_str("sip:bob@127.0.0.1:5062").unwrap(),
            Some(local_tag.to_string()),
            Some(remote_tag.to_string()),
            is_initiator,
        );
        dialog.state = state;
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");
        dialog_id
    }

    /// The INVITE Alice sends to Charlie in step 5 of an attended transfer.
    fn invite_with_replaces_headers(values: &[&str]) -> Request {
        let mut raw = String::from(
            "INVITE sip:charlie@example.test SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.168.0.10:5060;branch=z9hG4bK-replaces\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@example.test>;tag=alice-tag\r\n\
             To: <sip:charlie@example.test>\r\n\
             Call-ID: alice-to-charlie@example.test\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:alice@192.168.0.10:5060>\r\n",
        );
        for value in values {
            raw.push_str(&format!("Replaces: {}\r\n", value));
        }
        raw.push_str("Content-Length: 0\r\n\r\n");

        match parse_message(raw.as_bytes()).expect("parse INVITE") {
            Message::Request(request) => request,
            Message::Response(_) => panic!("expected a request"),
        }
    }

    #[tokio::test]
    async fn an_invite_without_replaces_is_left_alone() {
        let manager = make_manager().await;
        let request = invite_with_replaces_headers(&[]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Absent
        );
    }

    /// RFC 3891 §3: "If more than one Replaces header field is present in an
    /// INVITE [...] the UAS MUST reject the request with a 400 Bad Request."
    #[tokio::test]
    async fn more_than_one_replaces_header_is_a_400() {
        let manager = make_manager().await;
        let request = invite_with_replaces_headers(&[
            "cid-one;to-tag=t1;from-tag=f1",
            "cid-two;to-tag=t2;from-tag=f2",
        ]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Reject(StatusCode::BadRequest)
        );
    }

    /// §6.1 requires exactly one to-tag and one from-tag, so a value missing
    /// either cannot name a dialog and is malformed rather than unmatched.
    #[tokio::test]
    async fn a_malformed_replaces_value_is_a_400_not_a_481() {
        let manager = make_manager().await;
        for malformed in ["cid;to-tag=t1", "cid;from-tag=f1", "cid"] {
            let request = invite_with_replaces_headers(&[malformed]);
            assert_eq!(
                manager.evaluate_replaces(&request),
                ReplacesDisposition::Reject(StatusCode::BadRequest),
                "{:?} is malformed, and a malformed header is not the same as an unmatched one",
                malformed
            );
        }
    }

    /// §3: "If no match is found, the UAS rejects the INVITE and returns a 481
    /// Call/Transaction Does Not Exist response."
    #[tokio::test]
    async fn no_matching_dialog_is_a_481() {
        let manager = make_manager().await;
        let request = invite_with_replaces_headers(&["nobody-here;to-tag=t1;from-tag=f1"]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Reject(StatusCode::CallOrTransactionDoesNotExist)
        );
    }

    #[tokio::test]
    async fn a_confirmed_dialog_is_matched_and_scheduled_for_replacement() {
        let manager = make_manager().await;
        let dialog_id = store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;

        let request =
            invite_with_replaces_headers(&["consult-call;to-tag=charlie-tag;from-tag=bob-tag"]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Replace(dialog_id)
        );
    }

    /// The single highest value test here. §3 compares the to-tag to the local
    /// tag and the from-tag to the remote one. Swapping them yields a well
    /// formed key that matches nothing, so the failure looks like a missing
    /// dialog rather than like a bug, and survives review.
    #[tokio::test]
    async fn tags_supplied_in_the_wrong_direction_do_not_match() {
        let manager = make_manager().await;
        store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;

        let reversed =
            invite_with_replaces_headers(&["consult-call;to-tag=bob-tag;from-tag=charlie-tag"]);
        assert_eq!(
            manager.evaluate_replaces(&reversed),
            ReplacesDisposition::Reject(StatusCode::CallOrTransactionDoesNotExist),
            "to-tag is the receiver's local tag, never the sender's"
        );
    }

    /// §3: "If the Replaces header field matches a dialog which has already
    /// terminated, the UA SHOULD decline the request with a 603 Declined
    /// response." Not 481, and the difference matters: 603 stops the phone
    /// from ringing for a call that is already gone.
    #[tokio::test]
    async fn a_terminated_dialog_is_declined_with_603_not_481() {
        let manager = make_manager().await;
        store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Terminated,
            false,
        )
        .await;

        let request =
            invite_with_replaces_headers(&["consult-call;to-tag=charlie-tag;from-tag=bob-tag"]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Reject(StatusCode::Decline)
        );
    }

    /// §3: "it checks for the presence of the 'early-only' flag [...] If the
    /// flag is present, the UA rejects the request with a 486 Busy response."
    #[tokio::test]
    async fn early_only_against_a_confirmed_dialog_is_a_486() {
        let manager = make_manager().await;
        store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;

        let request = invite_with_replaces_headers(&[
            "consult-call;to-tag=charlie-tag;from-tag=bob-tag;early-only",
        ]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Reject(StatusCode::BusyHere)
        );
    }

    /// §3: "If the Replaces header field matches an early dialog that was not
    /// initiated by this UA, it returns a 481 [...] and leaves the matched
    /// dialog unchanged."
    #[tokio::test]
    async fn an_early_dialog_we_did_not_initiate_is_a_481() {
        let manager = make_manager().await;
        store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Early,
            false,
        )
        .await;

        let request =
            invite_with_replaces_headers(&["consult-call;to-tag=charlie-tag;from-tag=bob-tag"]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Reject(StatusCode::CallOrTransactionDoesNotExist)
        );
    }

    /// A Call-ID carrying a port is the case the old `split(':')` parse cut in
    /// the wrong place, producing a value that matched no dialog.
    #[tokio::test]
    async fn a_call_id_with_a_port_still_matches() {
        let manager = make_manager().await;
        let dialog_id = store_dialog_in_state(
            &manager,
            "consult@192.168.0.1:5060",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;

        let request = invite_with_replaces_headers(&[
            "consult@192.168.0.1:5060;to-tag=charlie-tag;from-tag=bob-tag",
        ]);
        assert_eq!(
            manager.evaluate_replaces(&request),
            ReplacesDisposition::Replace(dialog_id)
        );
    }

    /// The ordering §3 requires, stated as state rather than as timing: while
    /// the new INVITE is merely pending, the replaced dialog is untouched.
    #[tokio::test]
    async fn a_pending_replacement_does_not_touch_the_replaced_dialog() {
        let transport = RecordingTransport::new();
        let manager = make_manager_with(transport.clone()).await;
        let replaced = store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;
        let new_dialog = DialogId::new();

        manager.register_pending_replacement(&new_dialog, &replaced);

        assert_eq!(
            manager.get_dialog(&replaced).expect("dialog present").state,
            DialogState::Confirmed,
            "registering a replacement must not disturb the dialog being replaced"
        );
        assert!(
            transport.methods_sent().is_empty(),
            "nothing may reach the wire before the new INVITE is accepted"
        );
    }

    /// §3: "If the UA cannot accept the new INVITE [...] the UA MUST return an
    /// appropriate error response and MUST leave the matched dialog
    /// unchanged." A rejected call must never produce the BYE.
    #[tokio::test]
    async fn a_rejected_invite_leaves_the_replaced_dialog_alone() {
        let transport = RecordingTransport::new();
        let manager = make_manager_with(transport.clone()).await;
        let replaced = store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;
        let new_dialog = DialogId::new();

        manager.register_pending_replacement(&new_dialog, &replaced);
        manager.discard_pending_replacement(&new_dialog);
        // Completing after a discard must be inert, not a delayed BYE.
        manager.complete_pending_replacement(&new_dialog).await;

        assert_eq!(
            manager.get_dialog(&replaced).expect("dialog present").state,
            DialogState::Confirmed
        );
        assert!(
            !transport.methods_sent().contains(&Method::Bye),
            "a rejected replacement must never BYE the dialog it did not replace"
        );
    }

    /// The other half of the ordering: once the new dialog is confirmed, the
    /// replaced one is shut down with a BYE that actually reaches the wire.
    #[tokio::test]
    async fn accepting_the_new_invite_byes_the_replaced_dialog() {
        let transport = RecordingTransport::new();
        let manager = make_manager_with(transport.clone()).await;
        let replaced = store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;
        let new_dialog = DialogId::new();

        manager.register_pending_replacement(&new_dialog, &replaced);
        manager.complete_pending_replacement(&new_dialog).await;

        assert!(
            transport.methods_sent().contains(&Method::Bye),
            "RFC 3891 §3 shuts the replaced dialog down with a BYE, got {:?}",
            transport.methods_sent()
        );
    }

    /// An early dialog we initiated has no 2xx to BYE, so RFC 3891 §3 tears it
    /// down with a CANCEL of its own INVITE instead.
    ///
    /// Worth a real transaction rather than a stub: picking the wrong verb
    /// here would leave the consultation INVITE outstanding, and the failure
    /// would only show up as a call that never stops ringing.
    #[tokio::test]
    async fn accepting_the_new_invite_cancels_an_early_dialog_we_initiated() {
        use rvoip_sip_core::builder::SimpleRequestBuilder;

        let transport = RecordingTransport::new();
        let manager = make_manager_with(transport.clone()).await;
        let replaced = store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Early,
            true,
        )
        .await;

        // A real client INVITE transaction, because that is what a CANCEL has
        // to target.
        let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@127.0.0.1:5062")
            .expect("builder")
            .from("charlie", "sip:charlie@example.test", Some("charlie-tag"))
            .to("bob", "sip:bob@127.0.0.1:5062", None)
            .call_id("consult-call")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-consult"))
            .max_forwards(70)
            .contact("sip:charlie@127.0.0.1:5060", None)
            .build();
        let invite_tx_id = manager
            .transaction_manager()
            .create_client_transaction(invite, "127.0.0.1:5062".parse().unwrap())
            .await
            .expect("create client INVITE transaction");
        manager.link_transaction_to_dialog_indexed(&invite_tx_id, &replaced);

        let new_dialog = DialogId::new();
        manager.register_pending_replacement(&new_dialog, &replaced);
        manager.complete_pending_replacement(&new_dialog).await;

        let sent = transport.methods_sent();
        assert!(
            sent.contains(&Method::Cancel),
            "RFC 3891 §3 shuts an early dialog down with a CANCEL, got {:?}",
            sent
        );
        assert!(
            !sent.contains(&Method::Bye),
            "an early dialog has no 2xx to BYE, got {:?}",
            sent
        );
        assert!(
            manager.replaces_pending.get(&new_dialog).is_none(),
            "the pending replacement must be consumed"
        );
    }

    /// A replacement is consumed once. A later response on the same dialog,
    /// a re-INVITE for instance, must not resurrect it and BYE a second time.
    #[tokio::test]
    async fn a_replacement_fires_at_most_once() {
        let transport = RecordingTransport::new();
        let manager = make_manager_with(transport.clone()).await;
        let replaced = store_dialog_in_state(
            &manager,
            "consult-call",
            "charlie-tag",
            "bob-tag",
            DialogState::Confirmed,
            false,
        )
        .await;
        let new_dialog = DialogId::new();

        manager.register_pending_replacement(&new_dialog, &replaced);
        manager.complete_pending_replacement(&new_dialog).await;
        let after_first = transport.methods_sent();
        manager.complete_pending_replacement(&new_dialog).await;

        assert_eq!(
            transport.methods_sent(),
            after_first,
            "the second completion must be a no-op"
        );
    }
}
