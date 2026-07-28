// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A broadcast is a collection of tracks, split into two handles: [Writer] and [Reader].
//!
//! The [Writer] can create tracks, either manually or on request.
//! It receives all requests by a [Reader] for a tracks that don't exist.
//! The simplest implementation is to close every unknown track with [ServeError::NotFound].
//!
//! A [Reader] can request tracks by name.
//! If the track already exists, it will be returned.
//! If the track doesn't exist, it will be sent to [Unknown] to be handled.
//! A [Reader] can be cloned to create multiple subscriptions.
//!
//! The broadcast is automatically closed with [ServeError::Done] when [Writer] is dropped, or all [Reader]s are dropped.
use std::{collections::HashMap, ops::Deref, sync::Arc};

use super::{ServeError, Track, TrackReader, TrackWriter};
use crate::coding::{TrackName, TrackNamespace};
use crate::watch::{Queue, State};

/// Full track identifier: namespace + track name
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct FullTrackName {
    pub namespace: TrackNamespace,
    pub name: TrackName,
}

/// Static information about a broadcast.
#[derive(Debug)]
pub struct Tracks {
    pub namespace: TrackNamespace,
}

impl Tracks {
    pub fn new(namespace: TrackNamespace) -> Self {
        Self { namespace }
    }

    pub fn produce(self) -> (TracksWriter, TracksRequest, TracksReader) {
        self.produce_with_limits(TracksLimits::default())
            .expect("default track limits are valid")
    }

    pub fn produce_with_limits(
        self,
        limits: TracksLimits,
    ) -> Result<(TracksWriter, TracksRequest, TracksReader), TracksLimitsError> {
        limits.validate()?;
        let info = Arc::new(self);
        let state = State::new(TracksState {
            tracks: HashMap::new(),
            max_cached_tracks: limits.max_cached_tracks,
        })
        .split();
        let queue = Queue::bounded(limits.max_pending_requests).split();

        let writer = TracksWriter::new(state.0.clone(), info.clone());
        let request = TracksRequest::new(state.0, queue.0, info.clone());
        let reader = TracksReader::new(state.1, queue.1, info);

        Ok((writer, request, reader))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracksLimits {
    pub max_cached_tracks: usize,
    pub max_pending_requests: usize,
}

impl Default for TracksLimits {
    fn default() -> Self {
        Self {
            max_cached_tracks: 4_096,
            max_pending_requests: 1_024,
        }
    }
}

impl TracksLimits {
    pub fn validate(self) -> Result<(), TracksLimitsError> {
        if self.max_cached_tracks == 0 {
            return Err(TracksLimitsError::ZeroLimit("max_cached_tracks"));
        }
        if self.max_pending_requests == 0 {
            return Err(TracksLimitsError::ZeroLimit("max_pending_requests"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TracksLimitsError {
    #[error("track limit {0} must be greater than zero")]
    ZeroLimit(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TrackRequestError {
    #[error("track request state is closed")]
    Closed,
    #[error("track request capacity exhausted")]
    CapacityExhausted,
}

pub struct TracksState {
    tracks: HashMap<FullTrackName, TrackReader>,
    max_cached_tracks: usize,
}

impl TracksState {
    fn prune_closed(&mut self) {
        self.tracks.retain(|_, reader| !reader.is_closed());
    }

    fn has_capacity_for(&self, name: &FullTrackName) -> bool {
        self.tracks.contains_key(name) || self.tracks.len() < self.max_cached_tracks
    }
}

/// Publish new tracks for a broadcast by name.
pub struct TracksWriter {
    state: State<TracksState>,
    pub info: Arc<Tracks>,
}

impl TracksWriter {
    fn new(state: State<TracksState>, info: Arc<Tracks>) -> Self {
        Self { state, info }
    }

    /// Create a new track with the given name, inserting it into the broadcast.
    /// The track will use this writer's namespace.
    /// None is returned if all [TracksReader]s have been dropped.
    pub fn create(&mut self, track: impl Into<TrackName>) -> Option<TrackWriter> {
        let track = track.into();
        let full_name = FullTrackName {
            namespace: self.namespace.clone(),
            name: track.clone(),
        };
        let mut state = self.state.lock_mut()?;
        state.prune_closed();
        if !state.has_capacity_for(&full_name) {
            tracing::debug!(
                target: "moq_transport::tracks",
                "track cache capacity exhausted while publishing"
            );
            return None;
        }
        let (writer, reader) = Track {
            namespace: self.namespace.clone(),
            name: track,
        }
        .produce();

        // NOTE: We overwrite the track if it already exists.
        state.tracks.insert(full_name, reader);

        Some(writer)
    }

    /// Remove a track from the broadcast by full name.
    pub fn remove(
        &mut self,
        namespace: &TrackNamespace,
        track_name: impl Into<TrackName>,
    ) -> Option<TrackReader> {
        let full_name = FullTrackName {
            namespace: namespace.clone(),
            name: track_name.into(),
        };
        self.state.lock_mut()?.tracks.remove(&full_name)
    }
}

impl Deref for TracksWriter {
    type Target = Tracks;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

pub struct TracksRequest {
    #[allow(dead_code)] // Avoid dropping the write side
    state: State<TracksState>,
    incoming: Option<Queue<TrackWriter>>,
    pub info: Arc<Tracks>,
}

impl TracksRequest {
    fn new(state: State<TracksState>, incoming: Queue<TrackWriter>, info: Arc<Tracks>) -> Self {
        Self {
            state,
            incoming: Some(incoming),
            info,
        }
    }

    /// Wait for a request to create a new track.
    /// None is returned if all [TracksReader]s have been dropped.
    pub async fn next(&mut self) -> Option<TrackWriter> {
        self.incoming.as_mut()?.pop().await
    }
}

impl Deref for TracksRequest {
    type Target = Tracks;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

impl Drop for TracksRequest {
    fn drop(&mut self) {
        // Close any tracks still in the Queue
        let pending_tracks = self.incoming.take().unwrap().close();
        if !pending_tracks.is_empty() {
            tracing::debug!(
                target: "moq_transport::tracks",
                namespace = %self.info.namespace.to_utf8_path(),
                count = pending_tracks.len(),
                "TracksRequest dropped with pending track requests"
            );
        }
        for track in pending_tracks {
            let _ = track.close(ServeError::not_found_ctx(
                "tracks request dropped before track handled",
            ));
        }
    }
}

/// Subscribe to a broadcast by requesting tracks.
///
/// This can be cloned to create handles.
#[derive(Clone)]
pub struct TracksReader {
    state: State<TracksState>,
    queue: Queue<TrackWriter>,
    pub info: Arc<Tracks>,
}

impl TracksReader {
    fn new(state: State<TracksState>, queue: Queue<TrackWriter>, info: Arc<Tracks>) -> Self {
        Self { state, queue, info }
    }

    /// Wait until all producer-side handles for this namespace are gone.
    ///
    /// Track writers may finish before the namespace itself. This barrier is
    /// used by PUBLISH_NAMESPACE to drain active track-serving tasks before
    /// sending FIN on the owning request stream.
    pub async fn closed(&self) {
        loop {
            let modified = self.state.lock().modified();
            let Some(modified) = modified else {
                return;
            };
            modified.await;
        }
    }

    /// Get a track from the broadcast by full name, if it exists and is still alive.
    /// Returns None if the track doesn't exist or has been closed.
    pub fn get_track_reader(
        &mut self,
        namespace: &TrackNamespace,
        track_name: impl Into<TrackName>,
    ) -> Option<TrackReader> {
        let track_name = track_name.into();
        let state = self.state.lock();
        let full_name = FullTrackName {
            namespace: namespace.clone(),
            name: track_name.clone(),
        };

        if let Some(track_reader) = state.tracks.get(&full_name) {
            if !track_reader.is_closed() {
                return Some(track_reader.clone());
            }
        }
        state.into_mut()?.tracks.remove(&full_name);
        None
    }

    /// Get or request a track from the broadcast by full name.
    /// The namespace parameter should be the full requested namespace, not just the announced prefix.
    /// None is returned if [TracksWriter] or [TracksRequest] cannot fufill the request.
    pub fn subscribe(
        &mut self,
        namespace: TrackNamespace,
        track_name: impl Into<TrackName>,
    ) -> Option<TrackReader> {
        self.try_subscribe(namespace, track_name).ok()
    }

    /// Get or request a track while preserving overload versus closed-state failures.
    pub fn try_subscribe(
        &mut self,
        namespace: TrackNamespace,
        track_name: impl Into<TrackName>,
    ) -> Result<TrackReader, TrackRequestError> {
        let track_name = track_name.into();
        let state = self.state.lock();
        let full_name = FullTrackName {
            namespace: namespace.clone(),
            name: track_name.clone(),
        };

        // Check if we have a cached track that is still alive
        if let Some(track_reader) = state.tracks.get(&full_name) {
            if !track_reader.is_closed() {
                // Track is still active, return the cached reader
                tracing::debug!(
                    target: "moq_transport::tracks",
                    namespace = %namespace.to_utf8_path(),
                    track = %track_name,
                    "track cache hit (active)"
                );
                return Ok(track_reader.clone());
            }
            // Track is closed/stale, fall through to create a new one
            tracing::debug!(
                target: "moq_transport::tracks",
                namespace = %namespace.to_utf8_path(),
                track = %track_name,
                "track cache hit but stale, will evict and re-request"
            );
        }

        let mut state = state.into_mut().ok_or(TrackRequestError::Closed)?;

        state.prune_closed();
        if !state.has_capacity_for(&full_name) {
            tracing::debug!(
                target: "moq_transport::tracks",
                "track cache capacity exhausted while subscribing"
            );
            return Err(TrackRequestError::CapacityExhausted);
        }
        // Use the full requested namespace, not self.namespace
        let track_writer_reader = Track {
            namespace: namespace.clone(),
            name: track_name.clone(),
        }
        .produce();

        if self.queue.push(track_writer_reader.0).is_err() {
            tracing::debug!(
                target: "moq_transport::tracks",
                namespace = %namespace.to_utf8_path(),
                track = %track_name,
                "track request queue closed"
            );
            return Err(
                if self
                    .queue
                    .capacity()
                    .is_some_and(|capacity| self.queue.len() >= capacity)
                {
                    TrackRequestError::CapacityExhausted
                } else {
                    TrackRequestError::Closed
                },
            );
        }

        // We requested the track successfully so we can deduplicate it by full name.
        state
            .tracks
            .insert(full_name, track_writer_reader.1.clone());

        tracing::debug!(
            target: "moq_transport::tracks",
            namespace = %namespace.to_utf8_path(),
            track = %track_name,
            "track cache miss, requested from upstream"
        );

        Ok(track_writer_reader.1)
    }

    /// Aggregate retained-state diagnostics for this namespace.
    pub fn cached_tracks(&self) -> usize {
        self.state.lock().tracks.len()
    }

    pub fn pending_requests(&self) -> usize {
        self.queue.len()
    }
}

impl Deref for TracksReader {
    type Target = Tracks;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn namespace_close_barrier_waits_for_both_producer_handles() {
        let namespace = TrackNamespace::from_utf8_path("test/namespace");
        let (writer, requests, reader) = Tracks::new(namespace).produce();
        let barrier = tokio::spawn({
            let reader = reader.clone();
            async move { reader.closed().await }
        });

        tokio::task::yield_now().await;
        assert!(!barrier.is_finished());
        drop(writer);
        tokio::task::yield_now().await;
        assert!(!barrier.is_finished());
        drop(requests);
        tokio::time::timeout(std::time::Duration::from_millis(100), barrier)
            .await
            .expect("namespace barrier did not observe producer completion")
            .unwrap();
    }

    #[test]
    fn explicit_limits_reject_zero() {
        let namespace = TrackNamespace::from_utf8_path("limited");
        assert!(matches!(
            Tracks::new(namespace.clone()).produce_with_limits(TracksLimits {
                max_cached_tracks: 0,
                max_pending_requests: 1,
            }),
            Err(TracksLimitsError::ZeroLimit("max_cached_tracks"))
        ));
        assert!(Tracks::new(namespace)
            .produce_with_limits(TracksLimits {
                max_cached_tracks: 1,
                max_pending_requests: 0,
            })
            .is_err());
    }

    #[test]
    fn writer_cache_rejects_n_plus_one_and_reuses_closed_capacity() {
        let namespace = TrackNamespace::from_utf8_path("limited");
        let (mut writer, _requests, reader) = Tracks::new(namespace)
            .produce_with_limits(TracksLimits {
                max_cached_tracks: 2,
                max_pending_requests: 2,
            })
            .unwrap();
        let first = writer.create("first").unwrap();
        let _second = writer.create("second").unwrap();
        assert!(writer.create("third").is_none());
        assert_eq!(reader.cached_tracks(), 2);

        drop(first);
        assert!(writer.create("third").is_some());
        assert_eq!(reader.cached_tracks(), 2);
    }

    #[tokio::test]
    async fn pending_request_queue_rejects_n_plus_one_and_reuses_capacity() {
        let namespace = TrackNamespace::from_utf8_path("limited");
        let (_writer, mut requests, mut reader) = Tracks::new(namespace.clone())
            .produce_with_limits(TracksLimits {
                max_cached_tracks: 2,
                max_pending_requests: 1,
            })
            .unwrap();
        let _first_reader = reader.subscribe(namespace.clone(), "first").unwrap();
        assert!(matches!(
            reader.try_subscribe(namespace.clone(), "second"),
            Err(TrackRequestError::CapacityExhausted)
        ));
        assert_eq!(reader.pending_requests(), 1);

        let _first_writer = requests.next().await.unwrap();
        assert_eq!(reader.pending_requests(), 0);
        assert!(reader.subscribe(namespace, "second").is_some());
    }

    #[test]
    fn request_flood_stays_bounded_and_namespaces_are_isolated() {
        let limits = TracksLimits {
            max_cached_tracks: 4,
            max_pending_requests: 2,
        };
        let namespace_a = TrackNamespace::from_utf8_path("tenant-a");
        let namespace_b = TrackNamespace::from_utf8_path("tenant-b");
        let (_writer_a, _requests_a, mut reader_a) = Tracks::new(namespace_a.clone())
            .produce_with_limits(limits)
            .unwrap();
        let (_writer_b, _requests_b, mut reader_b) = Tracks::new(namespace_b.clone())
            .produce_with_limits(limits)
            .unwrap();

        for index in 0..1_000 {
            let _ = reader_a.subscribe(namespace_a.clone(), format!("track-{index}"));
        }
        assert!(reader_a.cached_tracks() <= limits.max_cached_tracks);
        assert!(reader_a.pending_requests() <= limits.max_pending_requests);
        assert!(reader_b.subscribe(namespace_b, "independent").is_some());
    }

    /// Regression test for the stale track caching bug.
    ///
    /// Scenario:
    /// 1. Subscriber requests a track via subscribe()
    /// 2. Publisher receives TrackWriter, closes it with an error (simulating failure)
    /// 3. Subscriber requests the same track again
    /// 4. Publisher should receive a new TrackWriter (previously didn't due to stale cache)
    ///
    /// This test verifies the fix for an issue seen in production where a track became
    /// "stale" after a connection timeout, and subsequent subscribers never received
    /// data because the publisher was never notified of new subscriptions.
    #[tokio::test]
    async fn test_stale_track_cache_bug() {
        let namespace = TrackNamespace::from_utf8_path("test/namespace");
        let track_name = "test-track";

        // Create the Tracks producer (simulates what the relay does)
        let (_writer, mut request, mut reader) = Tracks::new(namespace.clone()).produce();

        // First subscription: subscriber requests the track
        let track_reader_1 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("first subscribe should succeed");

        // Publisher receives the request and gets a TrackWriter
        let track_writer_1 = request
            .next()
            .await
            .expect("publisher should receive first track request");

        assert_eq!(track_writer_1.name, TrackName::from(track_name));

        // Publisher closes the track with an error (simulates connection failure)
        track_writer_1
            .close(ServeError::Cancel)
            .expect("close should succeed");

        // Verify the first track reader is now closed
        // (This is what makes subsequent reads fail immediately)
        let closed_result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            track_reader_1.closed(),
        )
        .await;
        assert!(
            closed_result.is_ok(),
            "track_reader_1 should be closed after writer closes"
        );

        // Second subscription: subscriber requests the SAME track again
        let track_reader_2 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("second subscribe should succeed");

        // With the fix, the stale cached TrackReader is detected and evicted,
        // so the publisher receives a new TrackWriter for the second subscription.
        let maybe_track_writer_2 =
            tokio::time::timeout(std::time::Duration::from_millis(100), request.next()).await;

        // Publisher should receive a new TrackWriter (stale cache entry was evicted)
        assert!(
            maybe_track_writer_2.is_ok(),
            "Publisher should receive a new track request after the first one was closed"
        );

        let track_writer_2 = maybe_track_writer_2
            .unwrap()
            .expect("publisher should receive second track request");

        assert_eq!(track_writer_2.name, TrackName::from(track_name));

        // Verify that track_reader_2 is NOT already closed
        // (It should be a fresh, working track)
        let closed_result_2 = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            track_reader_2.closed(),
        )
        .await;
        assert!(
            closed_result_2.is_err(),
            "track_reader_2 should NOT be immediately closed - it should be a fresh track"
        );
    }

    /// Test that normal track caching works correctly when tracks are still alive.
    ///
    /// Multiple subscribers to the same track should share the same TrackReader
    /// (deduplication), and the publisher should only receive one request.
    #[tokio::test]
    async fn test_track_deduplication_while_alive() {
        let namespace = TrackNamespace::from_utf8_path("test/namespace");
        let track_name = "test-track";

        let (_writer, mut request, mut reader) = Tracks::new(namespace.clone()).produce();

        // First subscription
        let track_reader_1 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("first subscribe should succeed");

        // Publisher receives request
        let _track_writer = request
            .next()
            .await
            .expect("publisher should receive track request");

        // Second subscription to the SAME track (while it's still alive)
        let track_reader_2 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("second subscribe should succeed");

        // Publisher should NOT receive another request (track is cached and alive)
        let maybe_second_request =
            tokio::time::timeout(std::time::Duration::from_millis(100), request.next()).await;

        assert!(
            maybe_second_request.is_err(),
            "Publisher should NOT receive a second request - track is cached and alive"
        );

        // Both readers should refer to the same track
        assert_eq!(track_reader_1.name, track_reader_2.name);
        assert_eq!(track_reader_1.namespace, track_reader_2.namespace);
    }

    /// Test that a track is NOT considered stale after the writer transitions to
    /// subgroups mode. This is the core regression: TrackWriter::subgroups()
    /// consumes self, dropping the Track-level State, but the SubgroupsWriter
    /// is still alive — so is_closed() must return false.
    #[tokio::test]
    async fn test_track_not_stale_after_subgroups_transition() {
        let namespace = TrackNamespace::from_utf8_path("test/namespace");
        let track_name = "test-track";

        let (_writer, mut request, mut reader) = Tracks::new(namespace.clone()).produce();

        let _track_reader_1 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("first subscribe should succeed");

        let track_writer = request
            .next()
            .await
            .expect("publisher should receive track request");

        let _subgroups_writer = track_writer
            .subgroups()
            .expect("subgroups transition should succeed");

        let _track_reader_2 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("second subscribe should succeed");

        let maybe_second_request =
            tokio::time::timeout(std::time::Duration::from_millis(100), request.next()).await;

        assert!(
            maybe_second_request.is_err(),
            "publisher should NOT get a second request while SubgroupsWriter is alive"
        );
    }

    /// Test that a track IS considered stale after the SubgroupsWriter is dropped.
    /// This preserves the RT-458 eviction behavior for dead publishers.
    #[tokio::test]
    async fn test_track_stale_after_subgroups_writer_dropped() {
        let namespace = TrackNamespace::from_utf8_path("test/namespace");
        let track_name = "test-track";

        let (_writer, mut request, mut reader) = Tracks::new(namespace.clone()).produce();

        let _track_reader_1 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("first subscribe should succeed");

        let track_writer = request
            .next()
            .await
            .expect("publisher should receive track request");

        let subgroups_writer = track_writer
            .subgroups()
            .expect("subgroups transition should succeed");
        drop(subgroups_writer);

        let _track_reader_2 = reader
            .subscribe(namespace.clone(), track_name)
            .expect("second subscribe should succeed");

        let maybe_second_request =
            tokio::time::timeout(std::time::Duration::from_millis(100), request.next()).await;

        assert!(
            maybe_second_request.is_ok(),
            "publisher should get a new request after SubgroupsWriter is dropped"
        );

        let _second_request = maybe_second_request
            .unwrap()
            .expect("publisher should receive second track request");
    }
}
