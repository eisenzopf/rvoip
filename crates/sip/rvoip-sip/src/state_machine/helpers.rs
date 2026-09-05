//! Helper methods for common state machine operations
//!
//! These methods provide convenience functions that can't be done through
//! simple message passing. They handle:
//! - Session creation and initialization
//! - State queries and session info
//! - Subscription management
//! - Complex operations that need multiple coordinated steps

use super::{executor::ResponseStateInput, StateMachine};
use crate::{
    errors::{Result, SessionError},
    session_registry::SessionRegistryHandle,
    state_table::types::{EventType, Role},
    types::{CallState, SessionId, SessionInfo},
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

type SessionSubscriber = Box<dyn Fn(SessionEvent) + Send + Sync>;
type SessionSubscribers = Arc<RwLock<HashMap<SessionId, Vec<SessionSubscriber>>>>;

/// Extended state machine with helper methods
pub struct StateMachineHelpers {
    /// Core state machine
    pub state_machine: Arc<StateMachine>,

    /// Event subscribers
    subscribers: SessionSubscribers,
}

/// Events for subscribers
#[derive(Clone)]
pub enum SessionEvent {
    StateChanged { from: CallState, to: CallState },
    CallEstablished,
    CallTerminated { reason: String },
    MediaReady,
    IncomingCall { from: String },
}

impl std::fmt::Debug for SessionEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StateChanged { from, to } => formatter
                .debug_struct("StateChanged")
                .field("from", from)
                .field("to", to)
                .finish(),
            Self::CallEstablished => formatter.write_str("CallEstablished"),
            Self::CallTerminated { reason } => formatter
                .debug_struct("CallTerminated")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::MediaReady => formatter.write_str("MediaReady"),
            Self::IncomingCall { from } => formatter
                .debug_struct("IncomingCall")
                .field("from_bytes", &from.len())
                .finish(),
        }
    }
}

impl StateMachineHelpers {
    pub fn new(state_machine: Arc<StateMachine>) -> Self {
        Self {
            state_machine,
            subscribers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ========== Session Creation ==========
    // These can't be done through message passing alone

    /// Create and initialize a new session
    pub async fn create_session(
        &self,
        session_id: SessionId,
        from: String,
        to: String,
        role: Role,
    ) -> Result<()> {
        // Publish the exact lifetime only after its initial addressing fields
        // are complete; no create-then-replace snapshot is exposed.
        self.state_machine
            .store
            .create_session_initialized(
                session_id.clone(),
                role,
                true, // with history
                |session| {
                    session.local_uri = Some(from.clone());
                    session.remote_uri = Some(to.clone());
                },
            )
            .await?;

        Ok(())
    }

    // ========== Convenience Methods ==========
    // High-level operations that coordinate multiple events.
    //
    // The basic `make_call*` family was removed when the SIP_API_DESIGN_2
    // deprecation cycle completed; the canonical outbound INVITE entry
    // point is [`OutboundCallBuilder`](crate::api::send::OutboundCallBuilder)
    // (`coord.invite(from, to).with_credentials(...)?.send().await`),
    // which does its own session setup + state plumbing without going
    // through `helpers.make_call*`. `make_call_inner` is retained as
    // shared kitchen logic for the transfer-leg path below.

    /// Spawn an outbound leg that will carry RFC 3515 §2.4.5 progress
    /// NOTIFYs back to `transferor_session_id` as its dialog advances.
    ///
    /// Critical invariant: `transferor_session_id` is written to the new
    /// leg's `SessionState` *before* the `MakeCall` event enters the
    /// state machine. That ordering closes the race where
    /// `Dialog180Ringing` (or a fast `Dialog200OK` on loopback) could
    /// fire between this helper returning and the caller setting the
    /// linkage. The shared `SendTransferNotify*` actions intentionally have
    /// no transfer projection for an ordinary call, but fail closed when any
    /// transfer marker is present without its exact transferor lifetime.
    ///
    /// The b2bua wrapper crate will call this as its primary
    /// REFER-forwarding entry point.
    pub async fn make_transfer_leg(
        &self,
        from: &str,
        to: &str,
        transferor_session_id: &SessionId,
    ) -> Result<SessionId> {
        self.make_call_inner(
            from,
            to,
            None,
            Some(transferor_session_id.clone()),
            None,
            Vec::new(),
        )
        .await
    }

    /// Lower-level primitive: retroactively link an existing leg to a
    /// transferor session. Callers must accept the race — any dialog
    /// event that fires before this call has no transfer projection because
    /// the leg is still an ordinary call at that instant. Prefer
    /// [`make_transfer_leg`](Self::make_transfer_leg) for freshly-created legs.
    pub async fn set_transferor_session(
        &self,
        leg_session_id: &SessionId,
        transferor_session_id: &SessionId,
    ) -> Result<()> {
        self.state_machine
            .set_transferor_session(leg_session_id, transferor_session_id.clone())
            .await?;
        Ok(())
    }

    async fn make_call_inner(
        &self,
        from: &str,
        to: &str,
        credentials: Option<crate::types::Credentials>,
        transferor_session_id: Option<SessionId>,
        pai_uri: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<SessionId> {
        let transferor_lifecycle_handle = match transferor_session_id.as_ref() {
            Some(transferor) => Some(
                self.state_machine
                    .store
                    .lifecycle_handle(transferor)
                    .ok_or_else(|| SessionError::SessionNotFound(transferor.to_string()))?,
            ),
            None => None,
        };
        let session_id = SessionId::new();

        self.create_session(
            session_id.clone(),
            from.to_string(),
            to.to_string(),
            Role::UAC,
        )
        .await?;

        self.state_machine
            .process_event_with_outbound_session_input(
                &session_id,
                EventType::MakeCall {
                    target: to.to_string(),
                },
                super::executor::OutboundSessionStateInput::new(
                    credentials,
                    None,
                    pai_uri,
                    transferor_session_id,
                    transferor_lifecycle_handle,
                    extra_headers,
                ),
            )
            .await?;

        Ok(session_id)
    }

    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.state_machine
            .process_event(session_id, EventType::AcceptCall)
            .await?;
        Ok(())
    }

    /// Exact-lifetime counterpart of [`Self::accept_call`].
    pub(crate) async fn accept_call_exact(&self, handle: &SessionRegistryHandle) -> Result<()> {
        self.state_machine
            .process_event_exact(handle, EventType::AcceptCall)
            .await?;
        Ok(())
    }

    /// Accept an incoming call with a caller-supplied SDP answer, bypassing
    /// local negotiation. Intended for b2bua scenarios where the answer comes
    /// from the outbound leg's 200 OK. Applies the SDP and
    /// `sdp_negotiated = true` only after acquiring the exact session lane, so
    /// the `GenerateLocalSDP`/`NegotiateSDPAsUAS` actions become no-ops without
    /// exposing a pre-transition store write.
    pub async fn accept_call_with_sdp(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        self.state_machine
            .process_event_with_local_sdp(session_id, EventType::AcceptCall, sdp)
            .await?;
        Ok(())
    }

    /// Exact-lifetime counterpart of [`Self::accept_call_with_sdp`].
    pub(crate) async fn accept_call_with_sdp_exact(
        &self,
        handle: &SessionRegistryHandle,
        sdp: String,
    ) -> Result<()> {
        self.state_machine
            .process_event_with_response_input_exact(
                handle,
                EventType::AcceptCall,
                ResponseStateInput::accept(Some(sdp), Vec::new()),
            )
            .await?;
        Ok(())
    }

    /// Accept with one application-authored response envelope captured for an
    /// exact incoming-call lifetime.
    pub(crate) async fn accept_call_with_response(
        &self,
        session_id: &SessionId,
        handle: &SessionRegistryHandle,
        sdp: Option<String>,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        if handle.session_id() != session_id {
            return Err(SessionError::InvalidInput(
                "response lifecycle handle does not match its session".to_string(),
            ));
        }
        self.state_machine
            .process_event_with_response_input_exact(
                handle,
                EventType::AcceptCall,
                ResponseStateInput::accept(sdp, extras),
            )
            .await?;
        Ok(())
    }

    /// Send a reliable 183 Session Progress with SDP (RFC 3262 early media).
    /// If `sdp` is `Some(_)`, the caller's SDP is sent verbatim. If `None`,
    /// the SDP answer is negotiated from the stored remote offer.
    pub async fn send_early_media(
        &self,
        session_id: &SessionId,
        sdp: Option<String>,
    ) -> Result<()> {
        self.state_machine
            .process_event(session_id, EventType::SendEarlyMedia { sdp })
            .await?;
        Ok(())
    }

    /// Exact-lifetime counterpart of [`Self::send_early_media`].
    pub(crate) async fn send_early_media_exact(
        &self,
        handle: &SessionRegistryHandle,
        sdp: Option<String>,
    ) -> Result<()> {
        self.state_machine
            .process_event_exact(handle, EventType::SendEarlyMedia { sdp })
            .await?;
        Ok(())
    }

    /// Send an application-authored provisional response through the retained
    /// YAML early-media transition. The status override lets the public
    /// builder request an arbitrary 1xx without adding a public event variant.
    pub(crate) async fn send_provisional_with_response(
        &self,
        session_id: &SessionId,
        handle: &SessionRegistryHandle,
        status: u16,
        sdp: Option<String>,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        if handle.session_id() != session_id {
            return Err(SessionError::InvalidInput(
                "response lifecycle handle does not match its session".to_string(),
            ));
        }
        self.state_machine
            .process_event_with_response_input_exact(
                handle,
                EventType::SendEarlyMedia { sdp },
                ResponseStateInput::provisional(status, extras),
            )
            .await?;
        Ok(())
    }

    /// Reject an incoming call with a specific SIP status code and reason phrase.
    pub async fn reject_call(
        &self,
        session_id: &SessionId,
        status: u16,
        reason: &str,
    ) -> Result<()> {
        self.state_machine
            .process_event(
                session_id,
                EventType::RejectCall {
                    status,
                    reason: reason.to_string(),
                },
            )
            .await?;
        Ok(())
    }

    /// Exact-lifetime counterpart of [`Self::reject_call`].
    pub(crate) async fn reject_call_exact(
        &self,
        handle: &SessionRegistryHandle,
        status: u16,
        reason: &str,
    ) -> Result<()> {
        self.state_machine
            .process_event_exact(
                handle,
                EventType::RejectCall {
                    status,
                    reason: reason.to_string(),
                },
            )
            .await?;
        Ok(())
    }

    /// Exact-lifetime rejection with application-authored response headers.
    pub(crate) async fn reject_call_with_extras_exact(
        &self,
        handle: &SessionRegistryHandle,
        status: u16,
        reason: &str,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        self.state_machine
            .process_event_with_response_input_exact(
                handle,
                EventType::RejectCall {
                    status,
                    reason: reason.to_string(),
                },
                ResponseStateInput::headers(extras),
            )
            .await?;
        Ok(())
    }

    /// Redirect an incoming call (send a 3xx response with `Contact:` headers
    /// per RFC 3261 §8.1.3.4 / §21.3). Valid from `Ringing` and `EarlyMedia`
    /// on the UAS role. `status` should be 300-399; `contacts` must be
    /// non-empty.
    pub async fn redirect_call(
        &self,
        session_id: &SessionId,
        status: u16,
        contacts: Vec<String>,
    ) -> Result<()> {
        self.state_machine
            .process_event(session_id, EventType::RedirectCall { status, contacts })
            .await?;
        Ok(())
    }

    /// Exact-lifetime counterpart of [`Self::redirect_call`].
    pub(crate) async fn redirect_call_exact(
        &self,
        handle: &SessionRegistryHandle,
        status: u16,
        contacts: Vec<String>,
    ) -> Result<()> {
        self.state_machine
            .process_event_exact(handle, EventType::RedirectCall { status, contacts })
            .await?;
        Ok(())
    }

    /// Redirect with application-authored headers through the same exact lane
    /// and YAML transition as the redirect lifecycle decision.
    pub(crate) async fn redirect_call_with_extras_exact(
        &self,
        handle: &SessionRegistryHandle,
        status: u16,
        contacts: Vec<String>,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        self.state_machine
            .process_event_with_response_input_exact(
                handle,
                EventType::RedirectCall { status, contacts },
                ResponseStateInput::headers(extras),
            )
            .await?;
        Ok(())
    }

    /// Hangup a call
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        // Skip the state-machine dispatch if the session is already gone —
        // a natural call-ended cleanup path may have won the race. Returning
        // a typed `SessionNotFound` here lets fire-and-forget callers
        // recognize it via `SessionError::is_session_gone()` while avoiding
        // the general-purpose ERROR log in executor::process_event.
        let handle = self
            .state_machine
            .store
            .lifecycle_handle(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;
        self.hangup_exact(&handle).await
    }

    /// Hang up only the exact session lifetime captured by the caller.
    pub(crate) async fn hangup_exact(&self, handle: &SessionRegistryHandle) -> Result<()> {
        self.state_machine
            .process_event_exact(handle, EventType::HangupCall)
            .await?;
        Ok(())
    }

    /// Create a conference from an active call
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.state_machine
            .process_event(
                session_id,
                EventType::CreateConference {
                    name: name.to_string(),
                },
            )
            .await?;
        Ok(())
    }

    /// Add a participant to a conference
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.state_machine
            .process_event(
                host_session_id,
                EventType::AddParticipant {
                    session_id: participant_session_id.to_string(),
                },
            )
            .await?;
        Ok(())
    }

    // ========== Query Methods ==========
    // These need access to internal state

    /// Get session information
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        let session = self
            .state_machine
            .store
            .with_session(session_id, Clone::clone)?;
        Ok(session_info_from_state(session))
    }

    /// Inspect one captured session lifetime without raw-ID re-resolution.
    pub(crate) async fn get_session_info_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<SessionInfo> {
        let session = self.state_machine.store.get_session_exact(handle).await?;
        Ok(session_info_from_state(session))
    }

    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        // Query the session store directly to get ALL sessions, including
        // those created by auto-transfer which bypass helpers.create_session()
        let sessions = self.state_machine.store.get_all_sessions().await;

        sessions.into_iter().map(session_info_from_state).collect()
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    #[cfg(feature = "perf-tests")]
    pub async fn perf_diagnostic_counts(&self) -> serde_json::Value {
        let active_sessions = self.state_machine.store.get_all_sessions().await.len();
        let subscribers = self.subscribers.read().await.len();
        serde_json::json!({
            "active_sessions": active_sessions,
            "subscriber_sessions": subscribers,
        })
    }

    /// Get current state of a session
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        Ok(self
            .state_machine
            .store
            .with_session(session_id, |session| session.call_state)?)
    }

    /// Read call state only from the captured exact lifetime.
    pub(crate) async fn get_state_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<CallState> {
        Ok(self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)?
            .call_state)
    }

    /// Return the codec negotiated for one exact live session.
    ///
    /// Media adapters use this retained session-state value instead of
    /// guessing PCMU before SDP negotiation has completed. A session whose
    /// SDP was supplied by the application without an anchored media
    /// negotiation cannot back a [`SipMediaStream`](crate::media_stream::SipMediaStream),
    /// so fail that case instead of leaving stream binding pending forever.
    pub(crate) async fn negotiated_media_config(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<(crate::session_store::state::NegotiatedConfig, u8)>> {
        let (negotiated_config, payload_type, sdp_negotiated) = self
            .state_machine
            .store
            .with_session(session_id, |session| {
                (
                    session.negotiated_config.clone(),
                    session.negotiated_payload_type(),
                    session.sdp_negotiated,
                )
            })?;
        match negotiated_config.zip(payload_type) {
            Some(config) => Ok(Some(config)),
            None if sdp_negotiated => Err(crate::errors::SessionError::MediaError(
                "SDP was supplied without an anchored negotiated media configuration".to_string(),
            )),
            None => Ok(None),
        }
    }

    pub(crate) async fn negotiated_media_config_exact(
        &self,
        handle: &crate::session_registry::SessionRegistryHandle,
    ) -> Result<Option<(crate::session_store::state::NegotiatedConfig, u8)>> {
        let snapshot = self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)?;
        let session = snapshot.state();
        match session
            .negotiated_config
            .clone()
            .zip(session.negotiated_payload_type())
        {
            Some(config) => Ok(Some(config)),
            None if session.sdp_negotiated => Err(crate::errors::SessionError::MediaError(
                "SDP was supplied without an anchored negotiated media configuration".to_string(),
            )),
            None => Ok(None),
        }
    }

    /// Check if a session is in conference
    pub async fn is_in_conference(&self, session_id: &SessionId) -> Result<bool> {
        // Conference functionality is handled via bridging
        // Check if session has a conference_mixer_id or is bridged
        let _ = session_id;
        Ok(false)
    }

    // ========== Subscription Management ==========
    // Can't be done through message passing

    /// Subscribe to events for a session
    pub async fn subscribe<F>(&self, session_id: SessionId, callback: F)
    where
        F: Fn(SessionEvent) + Send + Sync + 'static,
    {
        self.subscribers
            .write()
            .await
            .entry(session_id)
            .or_insert_with(Vec::new)
            .push(Box::new(callback));
    }

    /// Unsubscribe from a session
    pub async fn unsubscribe(&self, session_id: &SessionId) {
        self.subscribers.write().await.remove(session_id);
    }

    // ========== Internal Helpers ==========

    /// Notify subscribers of an event
    pub(crate) async fn notify_subscribers(&self, session_id: &SessionId, event: SessionEvent) {
        if let Some(callbacks) = self.subscribers.read().await.get(session_id) {
            for callback in callbacks {
                callback(event.clone());
            }
        }
    }

    /// Clean up terminated session
    pub(crate) async fn cleanup_session(&self, session_id: &SessionId) {
        self.subscribers.write().await.remove(session_id);
    }
}

fn session_info_from_state(session: crate::session_store::SessionState) -> SessionInfo {
    let start_time = std::time::SystemTime::now()
        .checked_sub(session.session_duration())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    SessionInfo {
        session_id: session.session_id,
        from: session.local_uri.unwrap_or_default(),
        to: session.remote_uri.unwrap_or_default(),
        state: session.call_state,
        start_time,
        media_active: session.media_session_id.is_some(),
    }
}

// ========== Things that CAN'T be done through message passing ==========
//
// 1. Session Creation - Need to allocate storage, set initial state
// 2. State Queries - Need direct access to session store
// 3. Listing Sessions - Need to enumerate all active sessions
// 4. Subscriptions - Need to maintain callback registry
// 5. Complex Coordinated Operations - Like creating a conference which needs
//    to track multiple sessions together
// 6. Resource Cleanup - Need to clean up multiple data structures
// 7. Session History - Need to access and query transition history
// 8. Performance Metrics - Need to collect timing data across components
//
// Everything else (call control, media operations, etc.) is done through
// the state machine by sending events and executing actions.

#[cfg(test)]
mod single_session_view_tests {
    use super::*;

    #[test]
    fn session_info_is_projected_from_the_canonical_session_state() {
        let session_id = SessionId::new();
        let mut session = crate::session_store::SessionState::new(session_id.clone(), Role::UAC);
        session.local_uri = Some("sip:alice@example.test".to_string());
        session.remote_uri = Some("sip:bob@example.test".to_string());
        session.call_state = CallState::Active;
        session.media_session_id = Some(crate::types::MediaSessionId::new("media-exact"));

        let info = session_info_from_state(session);

        assert_eq!(info.session_id, session_id);
        assert_eq!(info.from, "sip:alice@example.test");
        assert_eq!(info.to, "sip:bob@example.test");
        assert_eq!(info.state, CallState::Active);
        assert!(info.media_active);
    }

    #[test]
    fn helpers_have_no_second_mutable_active_session_view() {
        let production = include_str!("helpers.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production helper source");
        assert!(!production.contains("active_sessions:"));
        assert!(!production.contains("self.active_sessions"));
        assert!(production.contains("with_session(session_id, Clone::clone)"));
    }
}
