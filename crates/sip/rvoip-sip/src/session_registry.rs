//! Session Registry for single session mapping in rvoip-sip
//!
//! This module provides simple mappings for the single session constraint.
//! Since only one session can exist at a time, the mappings are much simpler.
//!
//! Storage uses `arc_swap::ArcSwapOption` rather than `tokio::sync::RwLock`:
//! each field holds at most one optional value, never a collection, so the
//! single-writer-many-readers RwLock model adds overhead with no benefit.
//! ArcSwap reads are wait-free atomic loads; writes are a single
//! compare-and-swap. See `crates/sip/rvoip-sip/docs/PROFILING.md` Scenario 8
//! ("SessionRegistry contention") for the side-by-side benchmark this
//! choice is grounded in.

use arc_swap::ArcSwapOption;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::types::{DialogId, IncomingCallInfo, MediaSessionId, SessionId};

/// Registry for single session mappings.
///
/// The `async fn` signatures are preserved across the RwLock → ArcSwap
/// migration so existing callers (which uniformly `.await` every method)
/// stay source-compatible. The await points are no-ops — each method
/// resolves synchronously — but the cost of an immediately-ready future
/// is negligible compared to the per-call `RwLock::read().await` that
/// these methods used to perform.
#[derive(Clone)]
pub struct SessionRegistry {
    /// Current session ID (if any).
    current_session: Arc<ArcSwapOption<SessionId>>,
    /// Current dialog ID (if any).
    current_dialog: Arc<ArcSwapOption<DialogId>>,
    /// Current media session ID (if any).
    current_media: Arc<ArcSwapOption<MediaSessionId>>,
    /// Temporary storage for pending incoming call.
    pending_incoming_call: Arc<ArcSwapOption<IncomingCallInfo>>,
    /// SIP_API_DESIGN_2 Phase A: parsed inbound INVITE request, retained
    /// while the call is in `Ringing` so `IncomingCall::raw_request()` can
    /// surface it.
    pending_incoming_request: Arc<ArcSwapOption<rvoip_sip_core::Request>>,
    /// Transport context for the pending inbound INVITE.
    pending_incoming_transport: Arc<ArcSwapOption<crate::auth::SipTransportSecurityContext>>,

    /// Diagnostic lifecycle counters for map/remove balance.
    dialog_mapped_total: Arc<AtomicU64>,
    media_mapped_total: Arc<AtomicU64>,
    removed_total: Arc<AtomicU64>,
    remove_missing_total: Arc<AtomicU64>,
}

impl SessionRegistry {
    /// Create a new session registry.
    pub fn new() -> Self {
        Self {
            current_session: Arc::new(ArcSwapOption::empty()),
            current_dialog: Arc::new(ArcSwapOption::empty()),
            current_media: Arc::new(ArcSwapOption::empty()),
            pending_incoming_call: Arc::new(ArcSwapOption::empty()),
            pending_incoming_request: Arc::new(ArcSwapOption::empty()),
            pending_incoming_transport: Arc::new(ArcSwapOption::empty()),
            dialog_mapped_total: Arc::new(AtomicU64::new(0)),
            media_mapped_total: Arc::new(AtomicU64::new(0)),
            removed_total: Arc::new(AtomicU64::new(0)),
            remove_missing_total: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Map a dialog ID to a session ID (single session version).
    pub async fn map_dialog(&self, session_id: SessionId, dialog_id: DialogId) {
        let session_id = Arc::new(session_id);
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        let previous_session = self.current_session.swap(Some(session_id.clone()));
        #[cfg(not(feature = "perf-infra-memory-diagnostics"))]
        let _previous_session = self.current_session.swap(Some(session_id.clone()));
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        self.record_session_mapping_for_memory_diagnostics(
            previous_session.as_deref(),
            &session_id,
        );
        self.current_dialog.store(Some(Arc::new(dialog_id)));
        self.dialog_mapped_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Map a media session ID to a session ID (single session version).
    pub async fn map_media(&self, session_id: SessionId, media_id: MediaSessionId) {
        let session_id = Arc::new(session_id);
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        let previous_session = self.current_session.swap(Some(session_id.clone()));
        #[cfg(not(feature = "perf-infra-memory-diagnostics"))]
        let _previous_session = self.current_session.swap(Some(session_id.clone()));
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        self.record_session_mapping_for_memory_diagnostics(
            previous_session.as_deref(),
            &session_id,
        );
        self.current_media.store(Some(Arc::new(media_id)));
        self.media_mapped_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Get session ID by dialog ID (single session version).
    pub async fn get_session_by_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        if self.current_dialog.load().as_deref() == Some(dialog_id) {
            self.current_session.load().as_deref().cloned()
        } else {
            None
        }
    }

    /// Get session ID by media session ID (single session version).
    pub async fn get_session_by_media(&self, media_id: &MediaSessionId) -> Option<SessionId> {
        if self.current_media.load().as_deref() == Some(media_id) {
            self.current_session.load().as_deref().cloned()
        } else {
            None
        }
    }

    /// Get dialog ID by session ID (single session version).
    pub async fn get_dialog_by_session(&self, session_id: &SessionId) -> Option<DialogId> {
        if self.current_session.load().as_deref() == Some(session_id) {
            self.current_dialog.load().as_deref().cloned()
        } else {
            None
        }
    }

    /// Get media session ID by session ID (single session version).
    pub async fn get_media_by_session(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        if self.current_session.load().as_deref() == Some(session_id) {
            self.current_media.load().as_deref().cloned()
        } else {
            None
        }
    }

    /// Remove all mappings for a session (single session version).
    pub async fn remove_session(&self, session_id: &SessionId) {
        loop {
            let current = self.current_session.load();
            if current.as_deref() != Some(session_id) {
                self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
                return;
            }

            let previous = self.current_session.compare_and_swap(&*current, None);
            if Self::option_arc_ptr_eq(&current, &previous) {
                self.current_dialog.store(None);
                self.current_media.store(None);
                self.removed_total.fetch_add(1, Ordering::Relaxed);
                #[cfg(feature = "perf-infra-memory-diagnostics")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "sip.session_registry.current_session",
                    std::mem::size_of::<SessionId>(),
                );
                return;
            }
        }
    }

    /// Feature-gated lifecycle counters for perf leak investigations.
    #[cfg(feature = "perf-tests")]
    pub(crate) fn perf_lifecycle_counts(&self) -> serde_json::Value {
        serde_json::json!({
            "dialog_mapped_total": self.dialog_mapped_total.load(Ordering::Relaxed),
            "media_mapped_total": self.media_mapped_total.load(Ordering::Relaxed),
            "removed_total": self.removed_total.load(Ordering::Relaxed),
            "remove_missing_total": self.remove_missing_total.load(Ordering::Relaxed),
        })
    }

    /// Check if a session exists in the registry (single session version).
    pub async fn contains_session(&self, session_id: &SessionId) -> bool {
        self.current_session.load().as_deref() == Some(session_id)
    }

    /// Get the total number of sessions in the registry (0 or 1).
    pub async fn session_count(&self) -> usize {
        usize::from(self.current_session.load().is_some())
    }

    /// Clear all mappings (single session version).
    pub async fn clear(&self) {
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        let previous_session = self.current_session.swap(None);
        #[cfg(not(feature = "perf-infra-memory-diagnostics"))]
        let _previous_session = self.current_session.swap(None);
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        if previous_session.is_some() {
            rvoip_infra_common::memory_diagnostics::record_dropped(
                "sip.session_registry.current_session",
                std::mem::size_of::<SessionId>(),
            );
        }
        self.current_dialog.store(None);
        self.current_media.store(None);
        self.pending_incoming_call.store(None);
        self.pending_incoming_request.store(None);
        self.pending_incoming_transport.store(None);
    }

    #[cfg(feature = "perf-infra-memory-diagnostics")]
    fn record_session_mapping_for_memory_diagnostics(
        &self,
        previous: Option<&SessionId>,
        session_id: &SessionId,
    ) {
        if previous == Some(session_id) {
            return;
        }
        if previous.is_some() {
            rvoip_infra_common::memory_diagnostics::record_dropped(
                "sip.session_registry.current_session",
                std::mem::size_of::<SessionId>(),
            );
        }
        rvoip_infra_common::memory_diagnostics::record_created(
            "sip.session_registry.current_session",
            std::mem::size_of::<SessionId>(),
        );
    }

    fn option_arc_ptr_eq<T>(
        left: &Option<Arc<T>>,
        right: &arc_swap::Guard<Option<Arc<T>>>,
    ) -> bool {
        match (left.as_ref(), right.as_ref()) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }

    /// Store pending incoming call info (single session version).
    pub async fn store_pending_incoming_call(
        &self,
        _session_id: SessionId,
        info: IncomingCallInfo,
    ) {
        self.pending_incoming_call.store(Some(Arc::new(info)));
    }

    /// Get and remove pending incoming call info (single session version).
    pub async fn take_pending_incoming_call(
        &self,
        _session_id: &SessionId,
    ) -> Option<IncomingCallInfo> {
        self.pending_incoming_call
            .swap(None)
            .map(|arc| (*arc).clone())
    }

    /// SIP_API_DESIGN_2 Phase A: store the parsed inbound INVITE so
    /// `IncomingCall::raw_request()` can surface it. The companion
    /// take/peek accessors are used by the four API surfaces when
    /// constructing the user-facing `IncomingCall`.
    pub async fn store_pending_incoming_request(&self, request: Arc<rvoip_sip_core::Request>) {
        self.pending_incoming_request.store(Some(request));
    }

    /// Store the transport context for the pending inbound INVITE.
    pub async fn store_pending_incoming_transport(
        &self,
        transport: crate::auth::SipTransportSecurityContext,
    ) {
        self.pending_incoming_transport
            .store(Some(Arc::new(transport)));
    }

    /// Peek at the parsed inbound INVITE without consuming it. Used
    /// when multiple surfaces (StreamPeer, CallbackPeer event stream,
    /// Endpoint) may build their own `IncomingCall` view of the same
    /// inbound call.
    pub async fn peek_pending_incoming_request(&self) -> Option<Arc<rvoip_sip_core::Request>> {
        self.pending_incoming_request.load_full()
    }

    /// Peek at the transport context for the pending inbound INVITE.
    pub async fn peek_pending_incoming_transport(
        &self,
    ) -> Option<Arc<crate::auth::SipTransportSecurityContext>> {
        self.pending_incoming_transport.load_full()
    }

    /// Consume the parsed inbound INVITE once an
    /// `IncomingCall::accept()` / `reject()` / `defer()` resolves the
    /// call. Idempotent — repeated calls return `None`.
    pub async fn take_pending_incoming_request(&self) -> Option<Arc<rvoip_sip_core::Request>> {
        self.pending_incoming_request.swap(None)
    }

    /// Consume the pending inbound INVITE transport context.
    pub async fn take_pending_incoming_transport(
        &self,
    ) -> Option<Arc<crate::auth::SipTransportSecurityContext>> {
        self.pending_incoming_transport.swap(None)
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dialog_mapping() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();

        registry
            .map_dialog(session_id.clone(), dialog_id.clone())
            .await;

        assert_eq!(
            registry.get_session_by_dialog(&dialog_id).await,
            Some(session_id.clone())
        );
        assert_eq!(
            registry.get_dialog_by_session(&session_id).await,
            Some(dialog_id)
        );
    }

    #[tokio::test]
    async fn test_media_mapping() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();
        let media_id = MediaSessionId::new_v4();

        registry
            .map_media(session_id.clone(), media_id.clone())
            .await;

        assert_eq!(
            registry.get_session_by_media(&media_id).await,
            Some(session_id.clone())
        );
        assert_eq!(
            registry.get_media_by_session(&session_id).await,
            Some(media_id)
        );
    }

    #[tokio::test]
    async fn test_remove_session() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();
        let media_id = MediaSessionId::new_v4();

        registry
            .map_dialog(session_id.clone(), dialog_id.clone())
            .await;
        registry
            .map_media(session_id.clone(), media_id.clone())
            .await;

        assert!(registry.contains_session(&session_id).await);

        registry.remove_session(&session_id).await;

        assert!(!registry.contains_session(&session_id).await);
        assert_eq!(registry.get_session_by_dialog(&dialog_id).await, None);
        assert_eq!(registry.get_session_by_media(&media_id).await, None);
    }

    #[tokio::test]
    async fn test_session_count() {
        let registry = SessionRegistry::new();

        assert_eq!(registry.session_count().await, 0);

        let session1 = SessionId::new();
        let session2 = SessionId::new();

        registry.map_dialog(session1.clone(), DialogId::new()).await;
        registry.map_dialog(session2.clone(), DialogId::new()).await;

        // Single-session registry: second map_dialog overwrites the first,
        // so count is 1, not 2.
        assert_eq!(registry.session_count().await, 1);
    }

    #[tokio::test]
    async fn test_clear() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();

        registry
            .map_dialog(session_id.clone(), DialogId::new())
            .await;
        registry
            .map_media(session_id.clone(), MediaSessionId::new_v4())
            .await;

        registry.clear().await;

        assert_eq!(registry.session_count().await, 0);
        assert!(!registry.contains_session(&session_id).await);
    }
}
