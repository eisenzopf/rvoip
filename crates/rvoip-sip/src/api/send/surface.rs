//! SIP_API_DESIGN_2 §4 — generic [`Surface`] / [`SurfaceBuilder`].
//!
//! Today's surfaces (`UnifiedCoordinator`, `Endpoint`, `StreamPeer`,
//! `CallbackPeer`) each expose the same 14 builder entry points. To
//! avoid `4 × 14 = 56` hand-written wrappers, a single generic adapter
//! does the surface-specific glue: pre-filling `from` from the
//! surface's local URI, resolving bare extensions on `Endpoint`, and
//! wrapping the resulting `SessionId` into a `SessionHandle` on the
//! peer surfaces. New builders extend automatically — adding `PUBLISH`
//! later is one new entry point on each surface plus one new builder
//! type.

use std::sync::Arc;

use crate::api::handle::{CallId, SessionHandle};
use crate::api::headers::SipRequestOptions;
use crate::api::unified::UnifiedCoordinator;

/// Per-surface customization knobs consumed by [`SurfaceBuilder`].
pub trait Surface: Send + Sync + 'static {
    /// What `.send()` returns on this surface. `UnifiedCoordinator`
    /// returns the bare [`CallId`]; the other three return a
    /// [`SessionHandle`] that wraps both the id and a coordinator
    /// reference.
    type SessionRef: Send + Sync;

    /// Convert a coordinator-level `CallId` into the surface's typed
    /// session ref.
    fn into_session_ref(&self, id: CallId) -> Self::SessionRef;

    /// Pre-populate `from` from the surface's local URI when the
    /// caller passes `None`. `Endpoint` additionally runs
    /// [`resolve_target`](Self::resolve_target) on bare extensions
    /// (`"1002"` → `"sip:1002@registrar"`).
    fn resolve_from(&self, from: Option<String>) -> String;

    /// Resolve a target string. Defaults to the identity transform;
    /// `Endpoint` overrides for extension-style targets.
    fn resolve_target(&self, target: &str) -> String {
        target.to_string()
    }
}

/// Generic wrapper that forwards every [`SipRequestOptions`] method to
/// the inner builder while keeping a handle to the surface so the
/// surface can post-process the result of `.send()` (e.g. wrap into a
/// [`SessionHandle`]).
pub struct SurfaceBuilder<B, S>
where
    B: SipRequestOptions,
    S: Surface,
{
    pub(crate) inner: B,
    pub(crate) surface: Arc<S>,
}

impl<B, S> SurfaceBuilder<B, S>
where
    B: SipRequestOptions,
    S: Surface,
{
    /// Construct a fresh `SurfaceBuilder` around the given inner.
    pub fn new(inner: B, surface: Arc<S>) -> Self {
        Self { inner, surface }
    }

    /// Map the inner builder, preserving the surface reference.
    pub fn map<F, B2>(self, f: F) -> SurfaceBuilder<B2, S>
    where
        F: FnOnce(B) -> B2,
        B2: SipRequestOptions,
    {
        SurfaceBuilder {
            inner: f(self.inner),
            surface: self.surface,
        }
    }

    /// Consume the wrapper and yield (inner, surface) for any builder
    /// that needs to drive its own `.send()` flow.
    pub fn into_parts(self) -> (B, Arc<S>) {
        (self.inner, self.surface)
    }

    /// Inspect the inner builder by reference.
    pub fn inner(&self) -> &B {
        &self.inner
    }

    /// Inspect the surface by reference.
    pub fn surface(&self) -> &S {
        &self.surface
    }
}

/// `SurfaceBuilder` itself implements `SipRequestOptions` by
/// forwarding to the inner. Phase C uses this so generic carry-through
/// code (`with_headers_from`, `strip_header`, etc.) works uniformly
/// across direct-coordinator and surfaced builders.
impl<B, S> SipRequestOptions for SurfaceBuilder<B, S>
where
    B: SipRequestOptions,
    S: Surface,
{
    fn method(&self) -> rvoip_sip_core::types::Method {
        self.inner.method()
    }

    fn header_state_mut(&mut self) -> &mut crate::api::headers::BuilderHeaderState {
        self.inner.header_state_mut()
    }

    fn header_state(&self) -> &crate::api::headers::BuilderHeaderState {
        self.inner.header_state()
    }
}

// Built-in Surface impl for `UnifiedCoordinator` — the bare-builder
// path. Returns `CallId` as the session ref.
impl Surface for UnifiedCoordinator {
    type SessionRef = CallId;

    fn into_session_ref(&self, id: CallId) -> CallId {
        id
    }

    fn resolve_from(&self, from: Option<String>) -> String {
        from.unwrap_or_else(|| self.config_local_uri())
    }
}

/// Helper for surfaces that wrap their `SessionRef` into a
/// [`SessionHandle`]. Used by `Endpoint` / `StreamPeer` /
/// `CallbackPeer` adapters.
pub fn wrap_handle(coord: &Arc<UnifiedCoordinator>, id: CallId) -> SessionHandle {
    SessionHandle::new(id, coord.clone())
}
