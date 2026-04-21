//! Helper methods for common state machine operations
//! 
//! These methods provide convenience functions that can't be done through
//! simple message passing. They handle:
//! - Session creation and initialization
//! - State queries and session info
//! - Subscription management
//! - Complex operations that need multiple coordinated steps

use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use crate::{
    types::{SessionId, SessionInfo, CallState},
    state_table::types::{Role, EventType},
    errors::{Result, SessionError},
};
use super::StateMachine;

/// Extended state machine with helper methods
pub struct StateMachineHelpers {
    /// Core state machine
    pub state_machine: Arc<StateMachine>,
    
    /// Active session tracking (for queries)
    active_sessions: Arc<RwLock<HashMap<SessionId, SessionInfo>>>,
    
    /// Event subscribers
    subscribers: Arc<RwLock<HashMap<SessionId, Vec<Box<dyn Fn(SessionEvent) + Send + Sync>>>>>,
}

/// Events for subscribers
#[derive(Debug, Clone)]
pub enum SessionEvent {
    StateChanged { from: CallState, to: CallState },
    CallEstablished,
    CallTerminated { reason: String },
    MediaReady,
    IncomingCall { from: String },
}

impl StateMachineHelpers {
    pub fn new(state_machine: Arc<StateMachine>) -> Self {
        Self {
            state_machine,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
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
        // Create session in the store
        let session = self.state_machine.store.create_session(
            session_id.clone(),
            role,
            true, // with history
        ).await?;
        
        // Set initial data
        let mut session = session;
        session.local_uri = Some(from.clone());
        session.remote_uri = Some(to.clone());
        
        // Store it
        self.state_machine.store.update_session(session).await?;
        
        // Track in active sessions
        let info = SessionInfo {
            session_id: session_id.clone(),
            from,
            to,
            state: CallState::Idle,
            start_time: std::time::SystemTime::now(),
            media_active: false,
        };
        self.active_sessions.write().await.insert(session_id, info);
        
        Ok(())
    }
    
    // ========== Convenience Methods ==========
    // High-level operations that coordinate multiple events
    
    /// Make an outgoing call (creates session + sends MakeCall event)
    pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
        self.make_call_with_credentials(from, to, None).await
    }

    /// Make an outgoing call with explicit digest-auth credentials attached
    /// to the session. The credentials land on `SessionState.credentials`
    /// before the INVITE goes on the wire, so if the server challenges with
    /// 401/407 the state machine can compute a response without a round-trip
    /// back to the caller. `None` = no auth retry on 401/407 for this call.
    pub async fn make_call_with_credentials(
        &self,
        from: &str,
        to: &str,
        credentials: Option<crate::types::Credentials>,
    ) -> Result<SessionId> {
        self.make_call_inner(from, to, credentials, None, None).await
    }

    /// Make an outgoing call attaching a `P-Asserted-Identity` URI
    /// (RFC 3325 §9.1) to the very first INVITE. The URI lands on
    /// `SessionState.pai_uri` *before* `MakeCall` is dispatched, so the
    /// `SendINVITE` action picks it up and routes through dialog-core's
    /// `make_call_with_extra_headers_for_session` path. `None` for `pai`
    /// is equivalent to plain [`make_call`](Self::make_call).
    ///
    /// Per-call override of [`Config::pai_uri`].
    pub async fn make_call_with_pai(
        &self,
        from: &str,
        to: &str,
        pai: Option<String>,
    ) -> Result<SessionId> {
        self.make_call_inner(from, to, None, None, pai).await
    }

    /// Combined entry point used by [`UnifiedCoordinator::make_call`] /
    /// `make_call_with_auth` to apply both the digest credentials and the
    /// `P-Asserted-Identity` from `Config` in a single dispatch. Either or
    /// both of `credentials` / `pai` may be `None`.
    pub async fn make_call_with_credentials_and_pai(
        &self,
        from: &str,
        to: &str,
        credentials: Option<crate::types::Credentials>,
        pai: Option<String>,
    ) -> Result<SessionId> {
        self.make_call_inner(from, to, credentials, None, pai).await
    }

    /// Spawn an outbound leg that will carry RFC 3515 §2.4.5 progress
    /// NOTIFYs back to `transferor_session_id` as its dialog advances.
    ///
    /// Critical invariant: `transferor_session_id` is written to the new
    /// leg's `SessionState` *before* the `MakeCall` event enters the
    /// state machine. That ordering closes the race where
    /// `Dialog180Ringing` (or a fast `Dialog200OK` on loopback) could
    /// fire between `make_call` returning and the caller setting the
    /// linkage — the `SendTransferNotify*` actions no-op when linkage is
    /// absent, so early progress NOTIFYs would otherwise be lost.
    ///
    /// The b2bua wrapper crate will call this as its primary
    /// REFER-forwarding entry point.
    pub async fn make_transfer_leg(
        &self,
        from: &str,
        to: &str,
        transferor_session_id: &SessionId,
    ) -> Result<SessionId> {
        self.make_call_inner(from, to, None, Some(transferor_session_id.clone()), None).await
    }

    /// Lower-level primitive: retroactively link an existing leg to a
    /// transferor session. Callers must accept the race — any dialog
    /// event that fires before this call is silently dropped by the
    /// no-op-on-`None` `SendTransferNotify*` actions. Prefer
    /// [`make_transfer_leg`] for freshly-created legs.
    pub async fn set_transferor_session(
        &self,
        leg_session_id: &SessionId,
        transferor_session_id: &SessionId,
    ) -> Result<()> {
        let mut session = self.state_machine.store.get_session(leg_session_id).await?;
        session.transferor_session_id = Some(transferor_session_id.clone());
        session.is_transfer_call = true;
        self.state_machine.store.update_session(session).await?;
        Ok(())
    }

    async fn make_call_inner(
        &self,
        from: &str,
        to: &str,
        credentials: Option<crate::types::Credentials>,
        transferor_session_id: Option<SessionId>,
        pai_uri: Option<String>,
    ) -> Result<SessionId> {
        let session_id = SessionId::new();

        self.create_session(
            session_id.clone(),
            from.to_string(),
            to.to_string(),
            Role::UAC,
        ).await?;

        // Fold any caller-supplied state (credentials, transfer linkage, PAI)
        // into `SessionState` *before* the `MakeCall` event enters the
        // state machine — otherwise a fast loopback `Dialog180Ringing`
        // arriving mid-dispatch can beat the update and the state
        // machine sees stale state.
        if credentials.is_some() || transferor_session_id.is_some() || pai_uri.is_some() {
            let mut session = self.state_machine.store.get_session(&session_id).await?;
            if let Some(creds) = credentials {
                session.credentials = Some(creds);
            }
            if let Some(referor) = transferor_session_id {
                session.transferor_session_id = Some(referor);
                session.is_transfer_call = true;
            }
            if let Some(pai) = pai_uri {
                session.pai_uri = Some(pai);
            }
            self.state_machine.store.update_session(session).await?;
        }

        self.state_machine.process_event(
            &session_id,
            EventType::MakeCall { target: to.to_string() },
        ).await?;

        Ok(session_id)
    }
    
    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::AcceptCall,
        ).await?;
        Ok(())
    }

    /// Accept an incoming call with a caller-supplied SDP answer, bypassing
    /// local negotiation. Intended for b2bua scenarios where the answer comes
    /// from the outbound leg's 200 OK. Writes the SDP into `session.local_sdp`
    /// and flips `sdp_negotiated = true` before dispatching `AcceptCall`, so
    /// the `GenerateLocalSDP`/`NegotiateSDPAsUAS` actions become no-ops.
    pub async fn accept_call_with_sdp(
        &self,
        session_id: &SessionId,
        sdp: String,
    ) -> Result<()> {
        let mut session = self.state_machine.store.get_session(session_id).await?;
        session.local_sdp = Some(sdp);
        session.sdp_negotiated = true;
        self.state_machine.store.update_session(session).await?;

        self.state_machine.process_event(
            session_id,
            EventType::AcceptCall,
        ).await?;
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
        self.state_machine.process_event(
            session_id,
            EventType::SendEarlyMedia { sdp },
        ).await?;
        Ok(())
    }
    
    /// Reject an incoming call with a specific SIP status code and reason phrase.
    pub async fn reject_call(
        &self,
        session_id: &SessionId,
        status: u16,
        reason: &str,
    ) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::RejectCall { status, reason: reason.to_string() },
        ).await?;
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
        self.state_machine.process_event(
            session_id,
            EventType::RedirectCall { status, contacts },
        ).await?;
        Ok(())
    }
    
    /// Hangup a call
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        // Skip the state-machine dispatch if the session is already gone —
        // a natural call-ended cleanup path may have won the race. Returning
        // a typed `SessionNotFound` here lets fire-and-forget callers
        // recognize it via `SessionError::is_session_gone()` while avoiding
        // the general-purpose ERROR log in executor::process_event.
        if self.state_machine.store.get_session(session_id).await.is_err() {
            return Err(SessionError::SessionNotFound(session_id.to_string()));
        }
        self.state_machine.process_event(
            session_id,
            EventType::HangupCall,
        ).await?;
        Ok(())
    }
    
    /// Create a conference from an active call
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::CreateConference { name: name.to_string() },
        ).await?;
        Ok(())
    }
    
    /// Add a participant to a conference
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.state_machine.process_event(
            host_session_id,
            EventType::AddParticipant {
                session_id: participant_session_id.to_string()
            },
        ).await?;
        Ok(())
    }

    // ========== Query Methods ==========
    // These need access to internal state
    
    /// Get session information
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        self.active_sessions.read().await
            .get(session_id)
            .cloned()
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))
    }
    
    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        // Query the session store directly to get ALL sessions, including
        // those created by auto-transfer which bypass helpers.create_session()
        let sessions = self.state_machine.store.get_all_sessions().await;

        sessions.into_iter().map(|s| SessionInfo {
            session_id: s.session_id.clone(),
            from: s.local_uri.unwrap_or_default(),
            to: s.remote_uri.unwrap_or_default(),
            state: s.call_state,
            start_time: std::time::SystemTime::now(), // Approximation - SessionState uses Instant
            media_active: s.media_session_id.is_some(),
        }).collect()
    }
    
    /// Get current state of a session
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        let session = self.state_machine.store.get_session(session_id).await?;
        Ok(session.call_state)
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
        self.subscribers.write().await
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
    #[allow(dead_code)]
    pub(crate) async fn notify_subscribers(&self, session_id: &SessionId, event: SessionEvent) {
        if let Some(callbacks) = self.subscribers.read().await.get(session_id) {
            for callback in callbacks {
                callback(event.clone());
            }
        }
    }
    
    /// Clean up terminated session
    #[allow(dead_code)]
    pub(crate) async fn cleanup_session(&self, session_id: &SessionId) {
        self.active_sessions.write().await.remove(session_id);
        self.subscribers.write().await.remove(session_id);
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
