//! Cross-crate event wrapper for rvoip-sip user-facing (API) events.
//!
//! Session API events are published to the `"session_to_app"` channel on the
//! [`GlobalEventCoordinator`](rvoip_infra_common::events::coordinator::GlobalEventCoordinator).
//! Any peer type (StreamPeer, CallbackPeer, or a custom implementation using
//! `UnifiedCoordinator::subscribe_events()`) receives them by subscribing to
//! that channel.
//!
//! The `MonolithicEventBus` inside the coordinator uses a lock-free broadcast channel
//! internally, so multiple subscribers each get an independent, low-latency delivery.

use rvoip_infra_common::events::cross_crate::CrossCrateEvent;
use rvoip_infra_common::events::types::EventPriority;
use rvoip_infra_common::planes::PlaneType;
use std::any::Any;
use std::sync::Arc;

use crate::session_registry::SessionRegistryHandle;

/// Event type identifier for session API events on the global coordinator.
///
/// Subscribe with:
/// ```rust,ignore
/// let mut rx = global_coordinator.subscribe(SESSION_TO_APP_CHANNEL).await?;
/// ```
pub const SESSION_TO_APP_CHANNEL: &str = "session_to_app";

/// Wraps an rvoip-sip [`Event`] for publishing through the
/// [`GlobalEventCoordinator`](rvoip_infra_common::events::coordinator::GlobalEventCoordinator).
///
/// [`Event`]: crate::api::events::Event
#[derive(Debug)]
pub struct SessionApiCrossCrateEvent {
    /// The user-facing session event.
    pub event: crate::api::events::Event,
}

impl SessionApiCrossCrateEvent {
    pub fn new(event: crate::api::events::Event) -> Arc<Self> {
        Arc::new(Self { event })
    }
}

impl CrossCrateEvent for SessionApiCrossCrateEvent {
    fn event_type(&self) -> &'static str {
        SESSION_TO_APP_CHANNEL
    }

    fn source_plane(&self) -> PlaneType {
        PlaneType::Signaling
    }

    fn target_plane(&self) -> PlaneType {
        PlaneType::Signaling
    }

    fn priority(&self) -> EventPriority {
        EventPriority::Normal
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Private, single-owner delivery for application events.
///
/// This value is sent only through the coordinator-owned bounded control
/// channel. It deliberately does not implement [`CrossCrateEvent`], so exact
/// registry authority cannot be published on the observational event bus.
#[derive(Debug)]
pub(crate) struct SessionControlEvent {
    pub(crate) event: crate::api::events::Event,
    pub(crate) lifecycle_handle: Option<SessionRegistryHandle>,
}

impl SessionControlEvent {
    pub(crate) fn new(
        event: crate::api::events::Event,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        Self {
            event,
            lifecycle_handle,
        }
    }
}

/// Return the public, observation-only copy of an application event.
///
/// Response transactions, exact obligations, coordinator references, and
/// lifecycle authority stay exclusively on [`SessionControlEvent`]. Public
/// event variants and fields remain unchanged for API compatibility.
pub(crate) fn sanitize_session_api_observation(
    event: &crate::api::events::Event,
) -> crate::api::events::Event {
    let mut observation = event.clone();
    match &mut observation {
        crate::api::events::Event::ReferReceived {
            request: Some(request),
            ..
        }
        | crate::api::events::Event::NotifyReceived {
            request: Some(request),
            ..
        } => request.clear_response_capability(),
        crate::api::events::Event::InfoReceived { request, .. }
        | crate::api::events::Event::MessageReceived { request, .. }
        | crate::api::events::Event::OptionsReceived { request, .. }
        | crate::api::events::Event::UpdateReceived { request, .. } => {
            request.clear_response_capability()
        }
        crate::api::events::Event::IncomingRegister { register } => {
            register.mark_control_observation()
        }
        _ => {}
    }
    observation
}
