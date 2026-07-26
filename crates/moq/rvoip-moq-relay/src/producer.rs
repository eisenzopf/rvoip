// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashSet;

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use moq_transport::{
    coding::{TrackNamespace, TrackNamespacePrefix},
    serve::{FullTrackName, ServeError, TrackReader, TrackRequestError, TracksReader},
    session::{Publisher, SessionError, Subscribed, SubscribedNamespace, TrackStatusRequested},
};

use crate::{
    metrics::{GaugeGuard, TimingGuard},
    Locals, NamespaceUpdate, RelayCapacity, RelayCapacityLease, RelayIdentity, RelayResource,
    RemoteCapacityError, RemoteManager,
};

/// Producer of tracks to a remote Subscriber
#[derive(Clone)]
pub struct Producer {
    publisher: Publisher,
    locals: Locals,
    remotes: RemoteManager,
    /// The resolved scope identity for this session, if any.
    /// Produced by `Coordinator::resolve_scope()` from the connection path.
    /// Passed to locals/remotes to isolate namespace lookups.
    identity: RelayIdentity,
    capacity: RelayCapacity,
}

impl Producer {
    /// Compatibility constructor with an isolated operator capacity pool.
    /// Production embedders should use [`Self::new_admitted`] and share one
    /// [`RelayCapacity`] across every producer and consumer.
    pub fn new(
        publisher: Publisher,
        locals: Locals,
        remotes: RemoteManager,
        scope: Option<String>,
    ) -> Self {
        Self::new_admitted(
            publisher,
            locals,
            remotes,
            RelayIdentity::operator(scope),
            RelayCapacity::default(),
        )
    }

    /// Construct a producer with authenticated identity and process-shared capacity.
    pub fn new_admitted(
        publisher: Publisher,
        locals: Locals,
        remotes: RemoteManager,
        identity: RelayIdentity,
        capacity: RelayCapacity,
    ) -> Self {
        Self {
            publisher,
            locals,
            remotes,
            identity,
            capacity,
        }
    }

    pub fn identity(&self) -> &RelayIdentity {
        &self.identity
    }

    /// Send PUBLISH_NAMESPACE for a set of tracks to the remote peer.
    pub async fn publish_namespace(&mut self, tracks: TracksReader) -> Result<(), SessionError> {
        self.publisher.publish_namespace(tracks).await
    }

    /// Send PUBLISH for one exact track.
    pub async fn publish(&mut self, track: TrackReader) -> Result<(), SessionError> {
        self.publisher.publish(track).await
    }

    /// Run the producer to serve subscribe requests.
    pub async fn run(self) -> Result<(), SessionError> {
        let mut tasks: FuturesUnordered<futures::future::BoxFuture<'static, ()>> =
            FuturesUnordered::new();

        loop {
            let mut publisher_subscribed = self.publisher.clone();
            let mut publisher_subscribed_namespace = self.publisher.clone();
            let mut publisher_track_status = self.publisher.clone();
            let mut publisher_fetch = self.publisher.clone();

            tokio::select! {
                // Handle a new subscribe request
                Some(subscribed) = publisher_subscribed.subscribed() => {
                    metrics::counter!("moq_relay_subscribers_total").increment(1);

                    let capacity_lease = match self.capacity.try_acquire(
                        &self.identity,
                        RelayResource::Subscribe,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            metrics::counter!("moq_relay_request_overload_total", "resource" => "subscribe").increment(1);
                            tracing::warn!(%error, "rejecting SUBSCRIBE at relay capacity");
                            let _ = subscribed.close(ServeError::Closed(
                                moq_transport::message::RequestErrorCode::ExcessiveLoad as u64,
                            ));
                            continue;
                        }
                    };

                    let this = self.clone();

                    // Spawn a new task to handle the subscribe
                    tasks.push(async move {
                        let info = subscribed.clone();
                        let namespace = info.track_namespace.to_utf8_path();
                        let track_name = info.track_name.clone();
                        tracing::info!(namespace = %namespace, track = %track_name, "serving subscribe: {:?}", info);

                        // Serve the subscribe request
                        if let Err(err) = this.serve_subscribe(subscribed, capacity_lease).await {
                            if Self::is_expected_serve_shutdown(&err) {
                                tracing::debug!(namespace = %namespace, track = %track_name, subscribe_info = ?info, error = %err, "stopped serving subscribe");
                            } else {
                                tracing::warn!(namespace = %namespace, track = %track_name, subscribe_info = ?info, error = %err, "failed serving subscribe");
                            }
                        }
                    }.boxed())
                },
                Some(subscribed_namespace) = publisher_subscribed_namespace.subscribed_namespace() => {
                    let capacity_lease = match self.capacity.try_acquire(
                        &self.identity,
                        RelayResource::Subscribe,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            metrics::counter!("moq_relay_request_overload_total", "resource" => "subscribe_namespace").increment(1);
                            tracing::warn!(%error, "rejecting SUBSCRIBE_NAMESPACE at relay capacity");
                            subscribed_namespace.close(ServeError::Closed(
                                moq_transport::message::RequestErrorCode::ExcessiveLoad as u64,
                            ));
                            continue;
                        }
                    };
                    let this = self.clone();
                    tasks.push(async move {
                        let prefix = subscribed_namespace.info.prefix.to_utf8_path();
                        if let Err(error) = this
                            .serve_subscribe_namespace(subscribed_namespace, capacity_lease)
                            .await
                        {
                            tracing::warn!(%prefix, %error, "failed serving SUBSCRIBE_NAMESPACE");
                        }
                    }.boxed())
                },
                // Handle a new track_status request
                Some(track_status_requested) = publisher_track_status.track_status_requested() => {
                    let capacity_lease = match self.capacity.try_acquire(
                        &self.identity,
                        RelayResource::TrackStatus,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            metrics::counter!("moq_relay_request_overload_total", "resource" => "track_status").increment(1);
                            tracing::warn!(%error, "rejecting TRACK_STATUS at relay capacity");
                            let mut request = track_status_requested;
                            request.respond_error_with_retry(
                                moq_transport::message::RequestErrorCode::ExcessiveLoad as u64,
                                1_001,
                                "relay capacity exhausted",
                            )?;
                            continue;
                        }
                    };
                    let this = self.clone();

                    // Spawn a new task to handle the track_status request
                    tasks.push(async move {
                        let info = track_status_requested.request_msg.clone();
                        let namespace = info.track_namespace.to_utf8_path();
                        let track_name = info.track_name.clone();
                        tracing::info!(namespace = %namespace, track = %track_name, "serving track_status: {:?}", info);

                        // Serve the track_status request
                        if let Err(err) = this.serve_track_status(track_status_requested, capacity_lease).await {
                            tracing::warn!(namespace = %namespace, track = %track_name, error = %err, "failed serving track_status: {:?}, error: {}", info, err)
                        }
                    }.boxed())
                },
                Some(mut fetch_requested) = publisher_fetch.fetch_requested() => {
                    let capacity_lease = match self.capacity.try_acquire(
                        &self.identity,
                        RelayResource::Fetch,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            metrics::counter!("moq_relay_request_overload_total", "resource" => "fetch").increment(1);
                            tracing::warn!(%error, "rejecting FETCH at relay capacity");
                            fetch_requested.reject_with_retry(
                                moq_transport::message::RequestErrorCode::ExcessiveLoad,
                                1_001,
                                "relay capacity exhausted",
                            )?;
                            continue;
                        }
                    };
                    tasks.push(async move {
                        let _capacity_lease = capacity_lease;
                        let request_id = fetch_requested.id();
                        let joining_request_id = fetch_requested.joining_request_id();
                        if let Err(error) = fetch_requested.serve().await {
                            tracing::warn!(request_id, ?joining_request_id, %error, "failed serving FETCH");
                        }
                    }.boxed())
                },
                _= tasks.next(), if !tasks.is_empty() => {},
                else => return Ok(()),
            };
        }
    }

    /// Serve namespace discovery from the coordinator's scope-bound snapshot
    /// and long-lived bounded update stream.
    async fn serve_subscribe_namespace(
        self,
        mut request: SubscribedNamespace,
        _capacity_lease: RelayCapacityLease,
    ) -> Result<(), anyhow::Error> {
        let prefix = TrackNamespace {
            fields: request.info.prefix.fields.clone(),
        };
        let mut subscription = match self
            .remotes
            .subscribe_namespace(self.identity.scope(), &prefix)
            .await
        {
            Ok(subscription) => subscription,
            Err(error) => {
                let serve_error = ServeError::internal_ctx(format!(
                    "namespace coordinator subscription failed: {error}"
                ));
                request.close(serve_error);
                return Err(error.into());
            }
        };

        request.ok()?;
        let mut active_namespaces = HashSet::with_capacity(subscription.existing_namespaces.len());
        for namespace in &subscription.existing_namespaces {
            let suffix = Self::namespace_suffix(&request.info.prefix, &namespace.namespace)?;
            Self::record_namespace_added(&mut active_namespaces, &namespace.namespace)?;
            request.namespace(suffix)?;
        }

        let _subscription_guard = GaugeGuard::new("moq_relay_active_namespace_subscriptions");
        loop {
            let update = tokio::select! {
                _ = request.closed() => break,
                update = subscription.next_update() => update,
            };

            let update = match update {
                Ok(update) => update,
                Err(error) => {
                    metrics::counter!(
                        "moq_relay_namespace_subscription_failures_total",
                        "kind" => "coordinator_update"
                    )
                    .increment(1);
                    return Err(anyhow::anyhow!(
                        "namespace coordinator update stream failed: {error}"
                    ));
                }
            };

            match update {
                NamespaceUpdate::Added(namespace) => {
                    let suffix =
                        Self::namespace_suffix(&request.info.prefix, &namespace.namespace)?;
                    Self::record_namespace_added(&mut active_namespaces, &namespace.namespace)?;
                    request.namespace(suffix)?;
                }
                NamespaceUpdate::Removed(namespace) => {
                    let suffix =
                        Self::namespace_suffix(&request.info.prefix, &namespace.namespace)?;
                    Self::record_namespace_removed(&mut active_namespaces, &namespace.namespace)?;
                    request.namespace_done(suffix)?;
                }
            }
        }
        drop(subscription);
        Ok(())
    }

    fn namespace_suffix(
        prefix: &TrackNamespacePrefix,
        namespace: &TrackNamespace,
    ) -> Result<TrackNamespacePrefix, ServeError> {
        let matches = prefix.fields.len() <= namespace.fields.len()
            && prefix
                .fields
                .iter()
                .zip(&namespace.fields)
                .all(|(expected, actual)| expected == actual);
        if !matches {
            return Err(ServeError::internal_ctx(
                "coordinator returned a namespace outside the subscribed prefix",
            ));
        }
        Ok(TrackNamespacePrefix {
            fields: namespace.fields[prefix.fields.len()..].to_vec(),
        })
    }

    fn record_namespace_added(
        active: &mut HashSet<TrackNamespace>,
        namespace: &TrackNamespace,
    ) -> Result<(), ServeError> {
        if active.insert(namespace.clone()) {
            Ok(())
        } else {
            Err(ServeError::internal_ctx(
                "coordinator announced an already-active namespace",
            ))
        }
    }

    fn record_namespace_removed(
        active: &mut HashSet<TrackNamespace>,
        namespace: &TrackNamespace,
    ) -> Result<(), ServeError> {
        if active.remove(namespace) {
            Ok(())
        } else {
            Err(ServeError::internal_ctx(
                "coordinator withdrew a namespace before announcing it",
            ))
        }
    }

    /// Serve a subscribe request.
    async fn serve_subscribe(
        self,
        subscribed: Subscribed,
        _capacity_lease: RelayCapacityLease,
    ) -> Result<(), anyhow::Error> {
        // Track subscribe latency from request to track resolution (records on drop)
        let mut timing_guard =
            TimingGuard::with_label("moq_relay_subscribe_latency_seconds", "source", "not_found");
        // Track active subscriptions - decrements when this function returns
        let _sub_guard = GaugeGuard::new("moq_relay_active_subscriptions");

        let namespace = subscribed.track_namespace.clone();
        let track_name = subscribed.track_name.clone();

        let full_name = FullTrackName {
            namespace: namespace.clone(),
            name: track_name.clone(),
        };
        if let Some(track) = self
            .locals
            .retrieve_track(self.identity.scope(), &full_name)
        {
            let ns = namespace.to_utf8_path();
            tracing::info!(namespace = %ns, track = %track_name, source = "local_publish", "serving subscribe from exact PUBLISH track");
            timing_guard.set_label("source", "local_publish");
            let _track_guard = GaugeGuard::new("moq_relay_active_tracks");
            return Ok(subscribed.serve(track).await?);
        }

        // Check local tracks first, and serve from local if possible
        if let Some(mut local) = self.locals.retrieve(self.identity.scope(), &namespace) {
            // Pass the full requested namespace, not the announced prefix
            match local.try_subscribe(namespace.clone(), &track_name) {
                Ok(track) => {
                    let ns = namespace.to_utf8_path();
                    tracing::info!(namespace = %ns, track = %track_name, source = "local", "serving subscribe from local: {:?}", track.info);
                    // Update label to indicate local source, timing recorded on drop
                    timing_guard.set_label("source", "local");
                    // Track active tracks - decrements when serve completes
                    let _track_guard = GaugeGuard::new("moq_relay_active_tracks");
                    return Ok(subscribed.serve(track).await?);
                }
                Err(TrackRequestError::CapacityExhausted) => {
                    metrics::counter!("moq_relay_request_overload_total", "resource" => "namespace_track_cache").increment(1);
                    let error = ServeError::Closed(
                        moq_transport::message::RequestErrorCode::ExcessiveLoad as u64,
                    );
                    subscribed.close(error.clone())?;
                    return Err(error.into());
                }
                Err(TrackRequestError::Closed) => {}
            }
        }

        // Check remote tracks second, and serve from remote if possible
        match self
            .remotes
            .subscribe(self.identity.scope(), &namespace, &track_name)
            .await
        {
            Ok(track) => {
                if let Some(track) = track {
                    let ns = namespace.to_utf8_path();
                    tracing::info!(namespace = %ns, track = %track_name, source = "remote", "serving subscribe from remote: {:?}", track.info);
                    // Update label to indicate remote source, timing recorded on drop
                    timing_guard.set_label("source", "remote");
                    // Track active tracks - decrements when serve completes
                    let _track_guard = GaugeGuard::new("moq_relay_active_tracks");
                    return Ok(subscribed.serve(track).await?);
                }
            }
            Err(e) => {
                // Route error = infrastructure failure (couldn't reach coordinator/upstream)
                // This is different from "not found" - we don't know if the track exists
                let ns = namespace.to_utf8_path();
                tracing::error!(namespace = %ns, track = %track_name, error = %e, "failed to route to remote: {}", e);
                timing_guard.set_label("source", "route_error");
                metrics::counter!("moq_relay_subscribe_route_errors_total").increment(1);

                // Return an internal error rather than "not found" since we couldn't check
                // TODO: Consider returning a more specific error to the subscriber
                let err = Self::route_serve_error(&e, &namespace);
                subscribed.close(err.clone())?;
                return Err(err.into());
            }
        }

        // Track not found - we checked all sources and the track doesn't exist
        // timing_guard label already set to "not_found", will record on drop
        metrics::counter!("moq_relay_subscribe_not_found_total").increment(1);

        let err = ServeError::not_found_ctx(format!(
            "track '{}/{}' not found in local or remote tracks",
            namespace, track_name
        ));
        subscribed.close(err.clone())?;
        Err(err.into())
    }

    fn is_expected_serve_shutdown(err: &anyhow::Error) -> bool {
        matches!(
            err.downcast_ref::<SessionError>(),
            Some(SessionError::Serve(ServeError::Cancel | ServeError::Done))
        ) || matches!(
            err.downcast_ref::<ServeError>(),
            Some(ServeError::Cancel | ServeError::Done)
        )
    }

    fn route_serve_error(
        error: &anyhow::Error,
        namespace: &moq_transport::coding::TrackNamespace,
    ) -> ServeError {
        if error.downcast_ref::<RemoteCapacityError>().is_some() {
            ServeError::Closed(moq_transport::message::RequestErrorCode::ExcessiveLoad as u64)
        } else {
            ServeError::internal_ctx(format!(
                "route error for namespace '{}': {}",
                namespace, error
            ))
        }
    }

    /// Serve a track_status request.
    async fn serve_track_status(
        self,
        mut track_status_requested: TrackStatusRequested,
        _capacity_lease: RelayCapacityLease,
    ) -> Result<(), anyhow::Error> {
        let full_name = FullTrackName {
            namespace: track_status_requested.request_msg.track_namespace.clone(),
            name: track_status_requested.request_msg.track_name.clone(),
        };
        if let Some(track) = self
            .locals
            .retrieve_track(self.identity.scope(), &full_name)
        {
            return Ok(track_status_requested.respond_ok(&track)?);
        }

        // Check local tracks first, and serve from local if possible
        if let Some(mut local_tracks) = self.locals.retrieve(
            self.identity.scope(),
            &track_status_requested.request_msg.track_namespace,
        ) {
            if let Some(track) = local_tracks.get_track_reader(
                &track_status_requested.request_msg.track_namespace,
                &track_status_requested.request_msg.track_name,
            ) {
                let namespace = track_status_requested
                    .request_msg
                    .track_namespace
                    .to_utf8_path();
                let track_name = &track_status_requested.request_msg.track_name;
                tracing::info!(namespace = %namespace, track = %track_name, source = "local", "serving track_status from local: {:?}", track.info);
                return Ok(track_status_requested.respond_ok(&track)?);
            }
        }

        // TODO - forward track status to remotes?
        // Check remote tracks second, and serve from remote if possible
        /*
        if let Some(remotes) = &self.remotes {
            // Try to route to a remote for this namespace
            if let Some(remote) = remotes.route(&subscribe.track_namespace).await? {
                if let Some(track) =
                    remote.subscribe(subscribe.track_namespace.clone(), subscribe.track_name.clone())?
                {
                    tracing::info!("serving from remote: {:?} {:?}", remote.info, track.info);

                    // NOTE: Depends on drop(track) being called afterwards
                    return Ok(subscribe.serve(track.reader).await?);
                }
            }
        }*/

        track_status_requested.respond_error(
            moq_transport::message::RequestErrorCode::DoesNotExist as u64,
            "track not found",
        )?;

        Err(ServeError::not_found_ctx(format!(
            "track '{}/{}' not found for track_status",
            track_status_requested.request_msg.track_namespace,
            track_status_requested.request_msg.track_name
        ))
        .into())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use moq_transport::{
        coding::{TrackNamespace, TrackNamespacePrefix},
        message::RequestErrorCode,
        serve::ServeError,
        session::SessionError,
    };

    use super::Producer;
    use crate::{RemoteCapacityError, RemoteCapacityResource};

    #[test]
    fn expected_serve_shutdown_accepts_wrapped_session_errors() {
        assert!(Producer::is_expected_serve_shutdown(&anyhow::Error::new(
            SessionError::Serve(ServeError::Cancel)
        )));
        assert!(Producer::is_expected_serve_shutdown(&anyhow::Error::new(
            SessionError::Serve(ServeError::Done)
        )));
        assert!(!Producer::is_expected_serve_shutdown(&anyhow::Error::new(
            SessionError::Serve(ServeError::NotFound)
        )));
    }

    #[test]
    fn expected_serve_shutdown_accepts_direct_serve_errors() {
        assert!(Producer::is_expected_serve_shutdown(&anyhow::Error::new(
            ServeError::Cancel
        )));
        assert!(Producer::is_expected_serve_shutdown(&anyhow::Error::new(
            ServeError::Done
        )));
        assert!(!Producer::is_expected_serve_shutdown(&anyhow::Error::new(
            ServeError::NotFound
        )));
    }

    #[test]
    fn upstream_capacity_is_a_retryable_request_overload() {
        let error = anyhow::Error::new(RemoteCapacityError {
            resource: RemoteCapacityResource::Track,
            limit: 1,
        });
        assert_eq!(
            Producer::route_serve_error(
                &error,
                &TrackNamespace::from_utf8_path("tenant/namespace")
            ),
            ServeError::Closed(RequestErrorCode::ExcessiveLoad as u64)
        );
    }

    #[test]
    fn namespace_suffix_preserves_tuple_boundaries() {
        let prefix = TrackNamespacePrefix::from_utf8_path("tenant/live");
        let namespace = TrackNamespace::from_utf8_path("tenant/live/clock");
        let suffix = Producer::namespace_suffix(&prefix, &namespace).unwrap();
        assert_eq!(suffix.to_utf8_path(), "/clock");
    }

    #[test]
    fn namespace_suffix_rejects_out_of_prefix_results() {
        let prefix = TrackNamespacePrefix::from_utf8_path("tenant/live");
        let namespace = TrackNamespace::from_utf8_path("other/live/clock");
        assert!(Producer::namespace_suffix(&prefix, &namespace).is_err());
    }

    #[test]
    fn namespace_update_state_rejects_done_before_namespace() {
        let mut active = HashSet::new();
        let namespace = TrackNamespace::from_utf8_path("tenant/live/clock");
        assert!(Producer::record_namespace_removed(&mut active, &namespace).is_err());
    }

    #[test]
    fn namespace_update_state_rejects_duplicate_namespace() {
        let mut active = HashSet::new();
        let namespace = TrackNamespace::from_utf8_path("tenant/live/clock");
        Producer::record_namespace_added(&mut active, &namespace).unwrap();
        assert!(Producer::record_namespace_added(&mut active, &namespace).is_err());
    }

    #[test]
    fn namespace_update_state_allows_withdrawal_and_reannouncement() {
        let mut active = HashSet::new();
        let namespace = TrackNamespace::from_utf8_path("tenant/live/clock");
        Producer::record_namespace_added(&mut active, &namespace).unwrap();
        Producer::record_namespace_removed(&mut active, &namespace).unwrap();
        Producer::record_namespace_added(&mut active, &namespace).unwrap();
        assert!(active.contains(&namespace));
    }
}
