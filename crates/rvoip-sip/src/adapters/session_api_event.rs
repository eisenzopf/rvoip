//! Cross-crate event wrapper for session-core user-facing (API) events.
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

/// Event type identifier for session API events on the global coordinator.
///
/// Subscribe with:
/// ```rust,ignore
/// let mut rx = global_coordinator.subscribe(SESSION_TO_APP_CHANNEL).await?;
/// ```
pub const SESSION_TO_APP_CHANNEL: &str = "session_to_app";

/// Wraps a session-core [`Event`] for publishing through the
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
