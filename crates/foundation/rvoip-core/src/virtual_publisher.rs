//! MediaGraph-backed logical publishers for transport subscribers.
//!
//! A virtual publisher exposes any Connection's reusable audio source under a
//! canonical `(SessionId, StreamId)` in the Orchestrator publisher registry.
//! It is deliberately transport-neutral: UCTP subscribers use the existing
//! subscription path, while the source may be SIP, WebRTC, Amazon, or another
//! adapter. The source receiver remains owned exactly once by `MediaGraph`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use tokio::task::JoinHandle;

use crate::ids::{ConnectionId, SessionId, StreamId};
use crate::media_graph::{ManagedMediaRoute, MediaGraphRouteStatus};
use crate::orchestrator::Orchestrator;
use crate::subscriptions::{PublisherRegistrationId, PublisherRegistry};
use crate::Result;

/// The per-publisher graph-to-fanout queue. Ten 20 ms audio frames bound the
/// additional buffering to roughly 200 ms while the MediaGraph's own slow-sink
/// policy provides the overload/eviction backstop.
pub const DEFAULT_VIRTUAL_PUBLISHER_QUEUE_CAPACITY: usize = 10;

/// Canonical identity under which a Connection's audio is published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualPublisherDescriptor {
    pub session_id: SessionId,
    pub stream_id: StreamId,
    pub participant: String,
}

impl VirtualPublisherDescriptor {
    pub fn new(session_id: SessionId, stream_id: StreamId, participant: impl Into<String>) -> Self {
        Self {
            session_id,
            stream_id,
            participant: participant.into(),
        }
    }
}

struct VirtualPublisherCleanup {
    registry: Arc<PublisherRegistry>,
    session_id: SessionId,
    stream_id: StreamId,
    registration_id: PublisherRegistrationId,
    active: AtomicBool,
}

impl VirtualPublisherCleanup {
    fn new(
        registry: Arc<PublisherRegistry>,
        descriptor: &VirtualPublisherDescriptor,
        registration_id: PublisherRegistrationId,
    ) -> Self {
        metrics::gauge!("rvoip_virtual_publishers_active").increment(1.0);
        Self {
            registry,
            session_id: descriptor.session_id.clone(),
            stream_id: descriptor.stream_id.clone(),
            registration_id,
            active: AtomicBool::new(true),
        }
    }

    fn is_current(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && self.registry.registration_is_current(
                &self.session_id,
                self.stream_id.as_str(),
                self.registration_id,
            )
    }

    fn unregister(&self) {
        if !self.active.swap(false, Ordering::AcqRel) {
            return;
        }
        self.registry.remove_registration(
            &self.session_id,
            self.stream_id.as_str(),
            self.registration_id,
        );
        metrics::gauge!("rvoip_virtual_publishers_active").decrement(1.0);
    }
}

struct TaskCleanup(Arc<VirtualPublisherCleanup>);

impl Drop for TaskCleanup {
    fn drop(&mut self) {
        self.0.unregister();
    }
}

/// Owning lease for a MediaGraph-backed publisher.
///
/// Explicit [`Self::close`] waits for the pump to stop and the graph route to
/// be removed. Dropping the handle aborts the pump, unregisters only its own
/// generation, and drops the managed graph route. Both paths are idempotent
/// with transport/session teardown.
#[must_use = "dropping the managed publisher immediately unregisters it"]
pub struct ManagedVirtualPublisher {
    source_connection_id: ConnectionId,
    descriptor: VirtualPublisherDescriptor,
    route: Option<ManagedMediaRoute>,
    task: Option<JoinHandle<()>>,
    cleanup: Arc<VirtualPublisherCleanup>,
}

impl ManagedVirtualPublisher {
    pub(crate) fn start(
        orchestrator: Weak<Orchestrator>,
        source_connection_id: ConnectionId,
        descriptor: VirtualPublisherDescriptor,
        route: ManagedMediaRoute,
        mut frames: tokio::sync::mpsc::Receiver<crate::stream::MediaFrame>,
        registry: Arc<PublisherRegistry>,
        registration_id: PublisherRegistrationId,
    ) -> Self {
        let cleanup = Arc::new(VirtualPublisherCleanup::new(
            registry,
            &descriptor,
            registration_id,
        ));
        let task_cleanup = TaskCleanup(Arc::clone(&cleanup));
        let task_session_id = descriptor.session_id.clone();
        let task_stream_id = descriptor.stream_id.clone();
        let task_publisher_id = source_connection_id.clone();
        let task = tokio::spawn(async move {
            let cleanup = task_cleanup;
            while let Some(mut frame) = frames.recv().await {
                // A compatible legacy registration may intentionally replace
                // this row. Stop before publishing under an identity we no
                // longer own, and let the managed graph route observe its
                // receiver closing.
                if !cleanup.0.is_current() {
                    break;
                }
                let Some(orchestrator) = orchestrator.upgrade() else {
                    break;
                };
                frame.stream_id = task_stream_id.clone();
                let delivered = orchestrator
                    .fanout_frame(&task_session_id, &task_publisher_id, &task_stream_id, frame)
                    .await;
                metrics::counter!("rvoip_virtual_publisher_frames_total").increment(1);
                metrics::counter!("rvoip_virtual_publisher_deliveries_total")
                    .increment(delivered as u64);
            }
        });

        Self {
            source_connection_id,
            descriptor,
            route: Some(route),
            task: Some(task),
            cleanup,
        }
    }

    pub fn source_connection_id(&self) -> &ConnectionId {
        &self.source_connection_id
    }

    pub fn descriptor(&self) -> &VirtualPublisherDescriptor {
        &self.descriptor
    }

    pub fn route_status(&self) -> MediaGraphRouteStatus {
        self.route
            .as_ref()
            .expect("managed virtual publisher route is present until close")
            .status()
    }

    /// Stop fanout, unregister the exact publisher generation, and await graph
    /// route removal. Cleanup completes even when the graph already ended.
    pub async fn close(mut self) -> Result<()> {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        self.cleanup.unregister();
        if let Some(route) = self.route.take() {
            // A source disconnect may already have closed the graph. The
            // owning registration and task are gone either way, so a terminal
            // route needs no second acknowledgement.
            if matches!(
                route.state(),
                crate::media_graph::MediaGraphRouteState::Terminal(_)
            ) {
                drop(route);
            } else {
                let _ = route.remove().await?;
            }
        }
        Ok(())
    }
}

impl Drop for ManagedVirtualPublisher {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.cleanup.unregister();
        self.route.take();
    }
}
