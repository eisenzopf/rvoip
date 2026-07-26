// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use anyhow::Context;
use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use moq_transport::{
    message::RequestErrorCode,
    serve::{self, Tracks, TracksLimits},
    session::{PublishReceived, PublishedNamespace, SessionError, Subscriber},
};

use crate::{
    metrics::GaugeGuard, Coordinator, Locals, Producer, RelayCapacity, RelayCapacityLease,
    RelayIdentity, RelayResource,
};

/// Consumer of tracks from a remote Publisher
#[derive(Clone)]
pub struct Consumer {
    subscriber: Subscriber,
    locals: Locals,
    coordinator: Arc<dyn Coordinator>,
    forward: Option<Producer>, // Forward all announcements to this subscriber
    /// The resolved scope identity for this session, if any.
    /// Produced by `Coordinator::resolve_scope()` from the connection path.
    /// Passed to coordinator register/lookup calls to isolate namespaces.
    identity: RelayIdentity,
    capacity: RelayCapacity,
    tracks_limits: TracksLimits,
}

impl Consumer {
    /// Compatibility constructor with an isolated operator capacity pool.
    /// Production embedders should use [`Self::new_admitted`] and share one
    /// [`RelayCapacity`] across every producer and consumer.
    pub fn new(
        subscriber: Subscriber,
        locals: Locals,
        coordinator: Arc<dyn Coordinator>,
        forward: Option<Producer>,
        scope: Option<String>,
    ) -> Self {
        let identity = RelayIdentity::operator(scope);
        Self::new_admitted(
            subscriber,
            locals,
            coordinator,
            forward,
            identity,
            RelayCapacity::default(),
            TracksLimits::default(),
        )
    }

    /// Construct a consumer with authenticated identity and process-shared capacity.
    pub fn new_admitted(
        subscriber: Subscriber,
        locals: Locals,
        coordinator: Arc<dyn Coordinator>,
        forward: Option<Producer>,
        identity: RelayIdentity,
        capacity: RelayCapacity,
        tracks_limits: TracksLimits,
    ) -> Self {
        Self {
            subscriber,
            locals,
            coordinator,
            forward,
            identity,
            capacity,
            tracks_limits,
        }
    }

    pub fn identity(&self) -> &RelayIdentity {
        &self.identity
    }

    /// Run the consumer to handle inbound namespace and exact-track publishes.
    pub async fn run(self) -> Result<(), SessionError> {
        let mut tasks: FuturesUnordered<futures::future::BoxFuture<'static, ()>> =
            FuturesUnordered::new();
        let mut namespace_subscriber = self.subscriber.clone();
        let mut publish_subscriber = self.subscriber.clone();

        loop {
            tokio::select! {
                Some(published_ns) = namespace_subscriber.published_namespace() => {
                    metrics::counter!("moq_relay_publishers_total").increment(1);

                    let capacity_lease = match self.capacity.try_acquire(
                        &self.identity,
                        RelayResource::PublishNamespace,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            metrics::counter!("moq_relay_request_overload_total", "resource" => "publish_namespace").increment(1);
                            tracing::warn!(%error, "rejecting PUBLISH_NAMESPACE at relay capacity");
                            let _ = published_ns.close(serve::ServeError::Closed(
                                RequestErrorCode::ExcessiveLoad as u64,
                            ));
                            continue;
                        }
                    };

                    let this = self.clone();

                    tasks.push(async move {
                        let info = published_ns.clone();
                        let namespace = info.namespace.to_utf8_path();
                        tracing::info!(
                            namespace = %namespace,
                            "serving PUBLISH_NAMESPACE: {:?}", info
                        );

                        if let Err(err) = this.serve(published_ns, capacity_lease).await {
                            tracing::warn!(
                                namespace = %namespace,
                                error = %err,
                                "failed serving PUBLISH_NAMESPACE: {:?}", info
                            );
                        }
                    }.boxed());
                },
                Some(publish) = publish_subscriber.publish_received() => {
                    metrics::counter!("moq_relay_published_tracks_total").increment(1);
                    let capacity_lease = match self.capacity.try_acquire(
                        &self.identity,
                        RelayResource::PublishTrack,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            metrics::counter!("moq_relay_request_overload_total", "resource" => "publish_track").increment(1);
                            tracing::warn!(%error, "rejecting PUBLISH at relay capacity");
                            publish.close(serve::ServeError::Closed(
                                RequestErrorCode::ExcessiveLoad as u64,
                            ));
                            continue;
                        }
                    };
                    let this = self.clone();
                    tasks.push(async move {
                        let namespace = publish.namespace().to_utf8_path();
                        let track = publish.name().clone();
                        if let Err(err) = this.serve_track(publish, capacity_lease).await {
                            tracing::warn!(namespace = %namespace, track = %track, error = %err, "failed serving PUBLISH");
                        }
                    }.boxed());
                },
                _ = tasks.next(), if !tasks.is_empty() => {},
                else => return Ok(()),
            };
        }
    }

    /// Serve an inbound PUBLISH_NAMESPACE.
    async fn serve(
        mut self,
        mut published_ns: PublishedNamespace,
        _capacity_lease: RelayCapacityLease,
    ) -> Result<(), anyhow::Error> {
        // Track active publishers - decrements when this function returns.
        let _publisher_guard = GaugeGuard::new("moq_relay_active_publishers");

        let mut tasks = FuturesUnordered::new();

        let (_, mut request, reader) =
            Tracks::new(published_ns.namespace.clone()).produce_with_limits(self.tracks_limits)?;

        let ns = reader.namespace.to_utf8_path();

        // Register the namespace locally so downstream subscribers can be served.
        tracing::debug!(namespace = %ns, "registering namespace in locals");
        let _register = match self
            .locals
            .register(self.identity.scope(), reader.clone())
            .await
        {
            Ok(reg) => reg,
            Err(err) => {
                metrics::counter!("moq_relay_announce_errors_total", "phase" => "local_register")
                    .increment(1);
                return Err(err);
            }
        };
        tracing::debug!(namespace = %ns, "namespace registered in locals");

        // Register namespace with the coordinator so other relay nodes can route to us.
        tracing::debug!(namespace = %ns, "registering namespace with coordinator");
        let _namespace_registration = match self
            .coordinator
            .register_namespace(self.identity.scope(), &reader.namespace)
            .await
        {
            Ok(reg) => reg,
            Err(crate::CoordinatorError::CapacityExhausted { resource }) => {
                metrics::counter!("moq_relay_request_overload_total", "resource" => "coordinator_namespace").increment(1);
                tracing::warn!(resource, "coordinator namespace capacity exhausted");
                let overload = serve::ServeError::Closed(RequestErrorCode::ExcessiveLoad as u64);
                published_ns.close(overload.clone())?;
                return Err(overload.into());
            }
            Err(err) => {
                metrics::counter!("moq_relay_announce_errors_total", "phase" => "coordinator_register")
                    .increment(1);
                return Err(err.into());
            }
        };
        tracing::debug!(namespace = %ns, "namespace registered with coordinator");

        // Accept the PUBLISH_NAMESPACE with REQUEST_OK.
        if let Err(err) = published_ns.ok() {
            metrics::counter!("moq_relay_announce_errors_total", "phase" => "send_ok").increment(1);
            return Err(err.into());
        }
        tracing::debug!(namespace = %ns, "sent REQUEST_OK for PUBLISH_NAMESPACE");
        metrics::counter!("moq_relay_announce_ok_total").increment(1);

        // Forward the namespace upstream, if configured.
        if let Some(mut forward) = self.forward {
            tasks.push(
                async move {
                    let namespace = reader.namespace.to_utf8_path();
                    tracing::info!(
                        namespace = %namespace,
                        "forwarding PUBLISH_NAMESPACE: {:?}", reader.info
                    );
                    forward
                        .publish_namespace(reader)
                        .await
                        .context("failed forwarding PUBLISH_NAMESPACE")
                }
                .boxed(),
            );
        }

        loop {
            tokio::select! {
                res = published_ns.closed() => {
                    let ns = published_ns.namespace.to_utf8_path();
                    res?;
                    tracing::info!(namespace = %ns, "PUBLISH_NAMESPACE closed");
                    return Ok(());
                },
                Some(track) = request.next() => {
                    let track_capacity_lease = match self.capacity.try_acquire(
                        &self.identity,
                        RelayResource::PublishTrack,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            metrics::counter!("moq_relay_request_overload_total", "resource" => "namespace_publish_track").increment(1);
                            tracing::warn!(%error, "rejecting namespace track at relay capacity");
                            let _ = track.close(serve::ServeError::Closed(
                                RequestErrorCode::ExcessiveLoad as u64,
                            ));
                            continue;
                        }
                    };
                    let mut subscriber = self.subscriber.clone();

                    tasks.push(async move {
                        let _track_capacity_lease = track_capacity_lease;
                        let info = track.clone();
                        let namespace = info.namespace.to_utf8_path();
                        let track_name = info.name.clone();
                        tracing::info!(
                            namespace = %namespace,
                            track = %track_name,
                            "forwarding subscribe: {:?}", info
                        );

                        if let Err(err) = subscriber.subscribe(track).await {
                            tracing::warn!(
                                namespace = %namespace,
                                track = %track_name,
                                error = %err,
                                "failed forwarding subscribe: {:?}", info
                            )
                        }

                        Ok(())
                    }.boxed());
                },
                res = tasks.next(), if !tasks.is_empty() => res.unwrap()?,
                else => return Ok(()),
            }
        }
    }

    async fn serve_track(
        mut self,
        mut publish: PublishReceived,
        _capacity_lease: RelayCapacityLease,
    ) -> Result<(), anyhow::Error> {
        let namespace = publish.namespace().clone();
        let track_name = publish.name().clone();
        let reader = publish.take_reader()?;
        let _local_registration = match self
            .locals
            .register_track(self.identity.scope(), reader.clone())
            .await
        {
            Ok(registration) => registration,
            Err(err) => {
                publish.close(serve::ServeError::Duplicate);
                return Err(err);
            }
        };

        let track_name_string = track_name.to_string();
        let _coordinator_registration = match self
            .coordinator
            .register_track(self.identity.scope(), &namespace, &track_name_string)
            .await
        {
            Ok(registration) => registration,
            Err(crate::CoordinatorError::CapacityExhausted { resource }) => {
                metrics::counter!("moq_relay_request_overload_total", "resource" => "coordinator_track").increment(1);
                tracing::warn!(resource, "coordinator track capacity exhausted");
                let overload = serve::ServeError::Closed(RequestErrorCode::ExcessiveLoad as u64);
                publish.close(overload.clone());
                return Err(overload.into());
            }
            Err(err) => {
                publish.close(serve::ServeError::Closed(
                    RequestErrorCode::InternalError as u64,
                ));
                return Err(err.into());
            }
        };

        publish.accept(true)?;
        let mut forward_task = self.forward.map(|mut forward| {
            tokio::spawn(async move {
                if let Err(err) = forward.publish(reader).await {
                    tracing::warn!(error = %err, "failed forwarding exact-track PUBLISH");
                }
            })
        });

        let result = publish.closed().await;
        if let Some(task) = forward_task.as_mut() {
            if tokio::time::timeout(std::time::Duration::from_secs(1), &mut *task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
        result?;
        Ok(())
    }
}
