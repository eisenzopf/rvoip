// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use moq_native_ietf::quic;
use moq_transport::coding::{TrackName, TrackNamespace};
use moq_transport::serve::{Track, TrackReader};
use moq_transport::session::RequestCapacity;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::{
    metrics::GaugeGuard, Coordinator, CoordinatorError, CoordinatorResult, NamespaceSubscription,
};

const DEFAULT_MAX_CONNECTIONS: usize = 128;
const DEFAULT_MAX_TRACKS: usize = 4_096;
const DEFAULT_TRACK_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Retained-state limits for upstream relay connections and track subscriptions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteManagerLimits {
    /// Maximum retained upstream connection slots across all resolved scopes.
    pub max_connections: usize,
    /// Maximum retained upstream track slots across all connections and scopes.
    pub max_tracks: usize,
    /// Time since the last lookup before an unused track is removed from the cache.
    pub track_idle_timeout: Duration,
    /// Time since the last lookup before an unused connection is shut down.
    pub connection_idle_timeout: Duration,
}

impl Default for RemoteManagerLimits {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_tracks: DEFAULT_MAX_TRACKS,
            track_idle_timeout: DEFAULT_TRACK_IDLE_TIMEOUT,
            connection_idle_timeout: DEFAULT_CONNECTION_IDLE_TIMEOUT,
        }
    }
}

impl RemoteManagerLimits {
    fn validate(self) -> Result<Self, RemoteManagerLimitsError> {
        if self.max_connections == 0 {
            return Err(RemoteManagerLimitsError::ZeroConnections);
        }
        if self.max_tracks == 0 {
            return Err(RemoteManagerLimitsError::ZeroTracks);
        }
        if self.max_connections > Semaphore::MAX_PERMITS {
            return Err(RemoteManagerLimitsError::TooManyConnections);
        }
        if self.max_tracks > Semaphore::MAX_PERMITS {
            return Err(RemoteManagerLimitsError::TooManyTracks);
        }
        if self.track_idle_timeout.is_zero() {
            return Err(RemoteManagerLimitsError::ZeroTrackIdleTimeout);
        }
        if self.connection_idle_timeout.is_zero() {
            return Err(RemoteManagerLimitsError::ZeroConnectionIdleTimeout);
        }
        Ok(self)
    }
}

/// Invalid [`RemoteManagerLimits`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RemoteManagerLimitsError {
    #[error("maximum upstream connections must be greater than zero")]
    ZeroConnections,
    #[error("maximum upstream tracks must be greater than zero")]
    ZeroTracks,
    #[error("maximum upstream connections exceeds the semaphore maximum")]
    TooManyConnections,
    #[error("maximum upstream tracks exceeds the semaphore maximum")]
    TooManyTracks,
    #[error("upstream track idle timeout must be greater than zero")]
    ZeroTrackIdleTimeout,
    #[error("upstream connection idle timeout must be greater than zero")]
    ZeroConnectionIdleTimeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteCapacityResource {
    Connection,
    Track,
}

impl std::fmt::Display for RemoteCapacityResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Connection => "connection",
            Self::Track => "track",
        })
    }
}

/// A retryable upstream retained-state admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("upstream {resource} capacity exhausted (limit {limit})")]
pub struct RemoteCapacityError {
    pub resource: RemoteCapacityResource,
    pub limit: usize,
}

/// Aggregate retained-state diagnostics. Scope and media names are intentionally omitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteManagerSnapshot {
    /// Number of connection slots currently addressable through the cache.
    pub cached_connections: usize,
    /// Number of connection capacity leases still retained by cache users or cleanup.
    pub retained_connections: usize,
    /// Number of track capacity leases still retained by cache users or cleanup.
    pub retained_tracks: usize,
    /// Number of session, cleanup, or eviction tasks owned by the manager.
    pub supervised_tasks: usize,
    /// Limits used to produce this snapshot.
    pub limits: RemoteManagerLimits,
}

/// Cache key for upstream relay-to-relay connections.
///
/// The resolved scope is part of the key so two tenants that resolve to the
/// same origin never share an authenticated upstream session.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RemoteCacheKey {
    scope: Option<Arc<str>>,
    url: Url,
    addr: Option<SocketAddr>,
}

impl RemoteCacheKey {
    fn new(scope: Option<&str>, url: Url, addr: Option<SocketAddr>) -> Self {
        Self {
            scope: scope.map(Arc::from),
            url,
            addr,
        }
    }
}

type RemoteSlot = Arc<RemoteSlotEntry>;
type TrackCacheKey = (TrackNamespace, TrackName);
type TrackSlot = Arc<TrackSlotEntry>;

struct RemoteSlotEntry {
    state: Mutex<RemoteSlotState>,
    idle_cancel: CancellationToken,
    _capacity: OwnedSemaphorePermit,
}

struct RemoteSlotState {
    remote: Option<Remote>,
    last_used: Instant,
}

impl RemoteSlotEntry {
    fn new(capacity: OwnedSemaphorePermit) -> Self {
        metrics::gauge!("moq_relay_upstream_retained_entries", "kind" => "connection")
            .increment(1.0);
        Self {
            state: Mutex::new(RemoteSlotState {
                remote: None,
                last_used: Instant::now(),
            }),
            idle_cancel: CancellationToken::new(),
            _capacity: capacity,
        }
    }
}

impl Drop for RemoteSlotEntry {
    fn drop(&mut self) {
        self.idle_cancel.cancel();
        metrics::gauge!("moq_relay_upstream_retained_entries", "kind" => "connection")
            .decrement(1.0);
    }
}

struct TrackSlotEntry {
    state: Mutex<TrackSlotState>,
    idle_cancel: CancellationToken,
    _capacity: OwnedSemaphorePermit,
}

struct TrackSlotState {
    reader: Option<TrackReader>,
    last_used: Instant,
}

impl TrackSlotEntry {
    fn new(capacity: OwnedSemaphorePermit) -> Self {
        metrics::gauge!("moq_relay_upstream_retained_entries", "kind" => "track").increment(1.0);
        Self {
            state: Mutex::new(TrackSlotState {
                reader: None,
                last_used: Instant::now(),
            }),
            idle_cancel: CancellationToken::new(),
            _capacity: capacity,
        }
    }
}

impl Drop for TrackSlotEntry {
    fn drop(&mut self) {
        self.idle_cancel.cancel();
        metrics::gauge!("moq_relay_upstream_retained_entries", "kind" => "track").decrement(1.0);
    }
}

/// Manages connections to remote relays.
///
/// When a subscription request comes in for a namespace that isn't local,
/// RemoteManager uses the coordinator to find which remote relay serves it,
/// establishes a connection if needed, and subscribes to the track.
#[derive(Clone)]
pub struct RemoteManager {
    coordinator: Arc<dyn Coordinator>,
    clients: Vec<quic::Client>,
    remotes: Arc<Mutex<HashMap<RemoteCacheKey, RemoteSlot>>>,
    connection_capacity: Arc<Semaphore>,
    track_capacity: Arc<Semaphore>,
    limits: RemoteManagerLimits,
    tasks: TaskTracker,
    shutdown: CancellationToken,
    request_capacity: RequestCapacity,
}

impl RemoteManager {
    /// Create a new RemoteManager.
    pub fn new(coordinator: Arc<dyn Coordinator>, clients: Vec<quic::Client>) -> Self {
        Self::with_limits(coordinator, clients, RemoteManagerLimits::default())
            .expect("default remote-manager limits are valid")
    }

    /// Create a remote manager with explicit retained-state limits.
    pub fn with_limits(
        coordinator: Arc<dyn Coordinator>,
        clients: Vec<quic::Client>,
        limits: RemoteManagerLimits,
    ) -> Result<Self, RemoteManagerLimitsError> {
        Self::with_limits_and_capacity(coordinator, clients, limits, RequestCapacity::default())
    }

    /// Create a manager sharing transport request/retention capacity with the relay process.
    pub fn with_limits_and_capacity(
        coordinator: Arc<dyn Coordinator>,
        clients: Vec<quic::Client>,
        limits: RemoteManagerLimits,
        request_capacity: RequestCapacity,
    ) -> Result<Self, RemoteManagerLimitsError> {
        let limits = limits.validate()?;
        Ok(Self {
            coordinator,
            clients,
            remotes: Arc::new(Mutex::new(HashMap::new())),
            connection_capacity: Arc::new(Semaphore::new(limits.max_connections)),
            track_capacity: Arc::new(Semaphore::new(limits.max_tracks)),
            limits,
            tasks: TaskTracker::new(),
            shutdown: CancellationToken::new(),
            request_capacity,
        })
    }

    /// Return aggregate retained-state diagnostics without tenant or media labels.
    pub async fn snapshot(&self) -> RemoteManagerSnapshot {
        RemoteManagerSnapshot {
            cached_connections: self.remotes.lock().await.len(),
            retained_connections: self
                .limits
                .max_connections
                .saturating_sub(self.connection_capacity.available_permits()),
            retained_tracks: self
                .limits
                .max_tracks
                .saturating_sub(self.track_capacity.available_permits()),
            supervised_tasks: self.tasks.len(),
            limits: self.limits,
        }
    }

    /// Register namespace-prefix discovery with the shared coordinator and
    /// return the current matching namespace snapshot plus its RAII lease.
    pub async fn subscribe_namespace(
        &self,
        scope: Option<&str>,
        prefix: &TrackNamespace,
    ) -> CoordinatorResult<NamespaceSubscription> {
        self.coordinator.subscribe_namespace(scope, prefix).await
    }

    /// Subscribe to a track from a remote relay.
    ///
    /// `scope` is the resolved scope identity from `Coordinator::resolve_scope()`,
    /// passed through to the coordinator's `lookup()` to scope the search.
    ///
    /// Returns None if the namespace isn't found in any remote relay.
    pub async fn subscribe(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
        track_name: impl Into<TrackName>,
    ) -> anyhow::Result<Option<TrackReader>> {
        let track_name = track_name.into();
        let (origin, client) = match self.coordinator.lookup(scope, namespace).await {
            Ok(result) => result,
            Err(CoordinatorError::NamespaceNotFound) => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        let url = origin.url();
        let cache_key = RemoteCacheKey::new(scope, url.clone(), origin.addr());

        let remote = match self
            .get_or_connect(cache_key.clone(), client.as_ref())
            .await
        {
            Ok(remote) => remote,
            Err(err) => {
                tracing::error!(remote_url = %crate::redact_url_for_logging(&url), error = %err, "failed to connect to remote relay");
                return Err(err);
            }
        };

        match remote.subscribe(namespace.clone(), track_name).await {
            Ok(reader) => Ok(reader),
            Err(err) => {
                tracing::warn!(remote_url = %crate::redact_url_for_logging(&url), error = %err, "remote subscribe failed, removing from cache");
                self.remove_if_same_remote(&cache_key, &remote).await;

                Err(err)
            }
        }
    }

    /// Get an existing remote connection or create a new one.
    async fn get_or_connect(
        &self,
        cache_key: RemoteCacheKey,
        client: Option<&quic::Client>,
    ) -> anyhow::Result<Remote> {
        if self.shutdown.is_cancelled() {
            anyhow::bail!("remote manager is shut down");
        }

        let client = match client {
            Some(client) => client,
            None => self.clients.first().ok_or_else(|| {
                anyhow::anyhow!("no QUIC clients configured for remote connections")
            })?,
        };

        loop {
            // The manager lock only protects the map. The per-key slot lock protects
            // that key's connection state, so unrelated remotes can connect in parallel.
            let slot = self.get_or_insert_remote_slot(cache_key.clone()).await?;

            let mut cached = slot.state.lock().await;
            cached.last_used = Instant::now();

            let is_current_slot = {
                let remotes = self.remotes.lock().await;
                matches!(remotes.get(&cache_key), Some(current) if Arc::ptr_eq(current, &slot))
            };

            if !is_current_slot {
                continue;
            }

            if let Some(remote) = cached.remote.as_ref() {
                if remote.is_connected() {
                    return Ok(remote.clone());
                }

                tracing::info!(remote_url = %crate::redact_url_for_logging(&cache_key.url), "removing dead connection to remote relay");
            };

            if let Some(remote) = cached.remote.take() {
                remote.shutdown().await;
            }

            tracing::info!(remote_url = %crate::redact_url_for_logging(&cache_key.url), "connecting to remote relay");
            let remote = match Remote::connect(
                cache_key.url.clone(),
                cache_key.addr,
                client,
                RemoteConnectContext {
                    remotes: Arc::downgrade(&self.remotes),
                    cache_key: cache_key.clone(),
                    cache_slot: Arc::downgrade(&slot),
                    track_capacity: self.track_capacity.clone(),
                    limits: self.limits,
                    tasks: self.tasks.clone(),
                    manager_shutdown: self.shutdown.clone(),
                    request_capacity: self.request_capacity.clone(),
                },
            )
            .await
            {
                Ok(remote) => remote,
                Err(err) => {
                    drop(cached);
                    remove_empty_remote_slot(&self.remotes, &cache_key, &slot).await;
                    return Err(err);
                }
            };

            cached.remote = Some(remote.clone());
            return Ok(remote);
        }
    }

    async fn get_or_insert_remote_slot(
        &self,
        cache_key: RemoteCacheKey,
    ) -> anyhow::Result<RemoteSlot> {
        {
            let remotes = self.remotes.lock().await;
            if let Some(slot) = remotes.get(&cache_key) {
                return Ok(slot.clone());
            }
        }

        let capacity = match self.connection_capacity.clone().try_acquire_owned() {
            Ok(capacity) => capacity,
            Err(_) => {
                let remotes = self.remotes.lock().await;
                if let Some(slot) = remotes.get(&cache_key) {
                    return Ok(slot.clone());
                }
                metrics::counter!(
                    "moq_relay_upstream_capacity_rejections_total",
                    "kind" => "connection"
                )
                .increment(1);
                return Err(RemoteCapacityError {
                    resource: RemoteCapacityResource::Connection,
                    limit: self.limits.max_connections,
                }
                .into());
            }
        };
        let candidate = Arc::new(RemoteSlotEntry::new(capacity));

        let slot = {
            let mut remotes = self.remotes.lock().await;
            if self.shutdown.is_cancelled() {
                anyhow::bail!("remote manager is shut down");
            }
            if let Some(slot) = remotes.get(&cache_key) {
                return Ok(slot.clone());
            }
            remotes.insert(cache_key.clone(), candidate.clone());
            candidate
        };

        self.spawn_connection_idle_task(cache_key, Arc::downgrade(&slot));
        Ok(slot)
    }

    fn spawn_connection_idle_task(&self, cache_key: RemoteCacheKey, slot: Weak<RemoteSlotEntry>) {
        let remotes = Arc::downgrade(&self.remotes);
        let shutdown = self.shutdown.clone();
        let idle_cancel = slot
            .upgrade()
            .expect("new remote slot is alive")
            .idle_cancel
            .clone();
        let timeout = self.limits.connection_idle_timeout;

        spawn_supervised(&self.tasks, async move {
            let mut delay = timeout;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown.cancelled() => return,
                    _ = idle_cancel.cancelled() => return,
                }

                let Some(slot) = slot.upgrade() else {
                    return;
                };
                let Some(remotes) = remotes.upgrade() else {
                    return;
                };

                let mut cached = slot.state.lock().await;
                let elapsed = Instant::now().saturating_duration_since(cached.last_used);
                if elapsed < timeout {
                    delay = timeout - elapsed;
                    continue;
                }
                if cached.remote.as_ref().is_some_and(Remote::is_active) {
                    continue;
                }

                let removed = {
                    let mut remotes = remotes.lock().await;
                    if matches!(remotes.get(&cache_key), Some(current) if Arc::ptr_eq(current, &slot))
                    {
                        remotes.remove(&cache_key);
                        true
                    } else {
                        false
                    }
                };

                if !removed {
                    return;
                }

                slot.idle_cancel.cancel();
                if let Some(remote) = cached.remote.take() {
                    remote.shutdown().await;
                }
                metrics::counter!(
                    "moq_relay_upstream_idle_evictions_total",
                    "kind" => "connection"
                )
                .increment(1);
                tracing::debug!(
                    remote_url = %crate::redact_url_for_logging(&cache_key.url),
                    "evicted idle upstream connection"
                );
                return;
            }
        });
    }

    async fn remove_if_same_remote(&self, cache_key: &RemoteCacheKey, remote: &Remote) {
        let slot = {
            let remotes = self.remotes.lock().await;
            remotes.get(cache_key).cloned()
        };

        if let Some(slot) = slot {
            let removed = {
                let mut cached = slot.state.lock().await;
                match cached.remote.as_ref() {
                    Some(current) if current.is_same_connection(remote) => cached.remote.take(),
                    _ => None,
                }
            };

            if let Some(remote) = removed {
                remote.shutdown().await;
                remove_empty_remote_slot(&self.remotes, cache_key, &slot).await;
            }
        }
    }

    /// Shutdown all remote connections and await every supervised task.
    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        let remotes = {
            let mut remotes = self.remotes.lock().await;
            remotes.drain().collect::<Vec<_>>()
        };

        for (cache_key, slot) in remotes {
            tracing::info!(remote_url = %crate::redact_url_for_logging(&cache_key.url), "shutting down remote connection");
            slot.idle_cancel.cancel();
            let mut remote = slot.state.lock().await;
            if let Some(remote) = remote.remote.take() {
                remote.shutdown().await;
            }
        }

        self.tasks.close();
        self.tasks.wait().await;
    }
}

impl Drop for RemoteManager {
    fn drop(&mut self) {
        if Arc::strong_count(&self.remotes) == 1 {
            self.shutdown.cancel();
            self.tasks.close();
        }
    }
}

async fn remove_empty_remote_slot(
    remotes: &Arc<Mutex<HashMap<RemoteCacheKey, RemoteSlot>>>,
    cache_key: &RemoteCacheKey,
    slot: &RemoteSlot,
) {
    let cached = slot.state.lock().await;
    if cached.remote.is_some() {
        return;
    }

    let mut remotes = remotes.lock().await;
    if matches!(remotes.get(cache_key), Some(current) if Arc::ptr_eq(current, slot)) {
        remotes.remove(cache_key);
        slot.idle_cancel.cancel();
    }
}

async fn remove_empty_track_slot(
    tracks: &Arc<Mutex<HashMap<TrackCacheKey, TrackSlot>>>,
    key: &TrackCacheKey,
    slot: &TrackSlot,
) {
    let cached = slot.state.lock().await;
    if cached.reader.is_some() {
        return;
    }

    let mut tracks = tracks.lock().await;
    if matches!(tracks.get(key), Some(current) if Arc::ptr_eq(current, slot)) {
        tracks.remove(key);
        slot.idle_cancel.cancel();
    }
}

fn spawn_supervised<F>(tasks: &TaskTracker, task: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tasks.spawn(async move {
        let _task_guard = UpstreamTaskGuard::new();
        task.await;
    });
}

struct UpstreamTaskGuard;

impl UpstreamTaskGuard {
    fn new() -> Self {
        metrics::gauge!("moq_relay_upstream_supervised_tasks").increment(1.0);
        Self
    }
}

impl Drop for UpstreamTaskGuard {
    fn drop(&mut self) {
        metrics::gauge!("moq_relay_upstream_supervised_tasks").decrement(1.0);
    }
}

#[derive(Clone)]
struct RemoteTrackCache {
    entries: Arc<Mutex<HashMap<TrackCacheKey, TrackSlot>>>,
    capacity: Arc<Semaphore>,
    max_tracks: usize,
    idle_timeout: Duration,
    tasks: TaskTracker,
    cancel: CancellationToken,
    url: Url,
}

impl RemoteTrackCache {
    fn new(
        capacity: Arc<Semaphore>,
        max_tracks: usize,
        idle_timeout: Duration,
        tasks: TaskTracker,
        cancel: CancellationToken,
        url: Url,
    ) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            capacity,
            max_tracks,
            idle_timeout,
            tasks,
            cancel,
            url,
        }
    }

    async fn get_or_insert(&self, key: TrackCacheKey) -> anyhow::Result<TrackSlot> {
        {
            let tracks = self.entries.lock().await;
            if let Some(slot) = tracks.get(&key) {
                return Ok(slot.clone());
            }
        }

        let capacity = match self.capacity.clone().try_acquire_owned() {
            Ok(capacity) => capacity,
            Err(_) => {
                let tracks = self.entries.lock().await;
                if let Some(slot) = tracks.get(&key) {
                    return Ok(slot.clone());
                }
                metrics::counter!(
                    "moq_relay_upstream_capacity_rejections_total",
                    "kind" => "track"
                )
                .increment(1);
                return Err(RemoteCapacityError {
                    resource: RemoteCapacityResource::Track,
                    limit: self.max_tracks,
                }
                .into());
            }
        };
        let candidate = Arc::new(TrackSlotEntry::new(capacity));

        let slot = {
            let mut tracks = self.entries.lock().await;
            if self.cancel.is_cancelled() {
                anyhow::bail!("remote connection is closed");
            }
            if let Some(slot) = tracks.get(&key) {
                return Ok(slot.clone());
            }
            tracks.insert(key.clone(), candidate.clone());
            candidate
        };

        self.spawn_idle_task(key, Arc::downgrade(&slot));
        Ok(slot)
    }

    fn spawn_idle_task(&self, key: TrackCacheKey, slot: Weak<TrackSlotEntry>) {
        let tracks = Arc::downgrade(&self.entries);
        let cancel = self.cancel.clone();
        let idle_cancel = slot
            .upgrade()
            .expect("new track slot is alive")
            .idle_cancel
            .clone();
        let timeout = self.idle_timeout;
        let url = self.url.clone();

        spawn_supervised(&self.tasks, async move {
            let mut delay = timeout;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = cancel.cancelled() => return,
                    _ = idle_cancel.cancelled() => return,
                }

                let Some(slot) = slot.upgrade() else {
                    return;
                };
                let Some(tracks) = tracks.upgrade() else {
                    return;
                };

                let mut cached = slot.state.lock().await;
                let elapsed = Instant::now().saturating_duration_since(cached.last_used);
                if elapsed < timeout {
                    delay = timeout - elapsed;
                    continue;
                }
                if cached
                    .reader
                    .as_ref()
                    .is_some_and(|reader| reader.reader_count() > 1)
                {
                    delay = timeout;
                    continue;
                }

                let removed = {
                    let mut tracks = tracks.lock().await;
                    if matches!(tracks.get(&key), Some(current) if Arc::ptr_eq(current, &slot)) {
                        tracks.remove(&key);
                        true
                    } else {
                        false
                    }
                };

                if !removed {
                    return;
                }

                slot.idle_cancel.cancel();
                cached.reader.take();
                metrics::counter!(
                    "moq_relay_upstream_idle_evictions_total",
                    "kind" => "track"
                )
                .increment(1);
                tracing::debug!(
                    remote_url = %crate::redact_url_for_logging(&url),
                    "evicted idle upstream track"
                );
                return;
            }
        });
    }

    async fn clear(&self) {
        let entries = {
            let mut entries = self.entries.lock().await;
            entries.drain().map(|(_, slot)| slot).collect::<Vec<_>>()
        };
        for slot in entries {
            slot.idle_cancel.cancel();
        }
    }
}

/// A connection to a single remote relay with its own QUIC client.
#[derive(Clone)]
struct Remote {
    url: Url,
    subscriber: moq_transport::session::Subscriber,
    /// Track subscriptions keyed by full track name.
    tracks: RemoteTrackCache,
    tasks: TaskTracker,
    active_operations: Arc<AtomicUsize>,
    active_subscriptions: Arc<AtomicUsize>,
    /// Flag indicating if the connection is still alive.
    connected: Arc<AtomicBool>,
    /// Cancellation token for the session task.
    cancel: CancellationToken,
}

struct RemoteConnectContext {
    remotes: Weak<Mutex<HashMap<RemoteCacheKey, RemoteSlot>>>,
    cache_key: RemoteCacheKey,
    cache_slot: Weak<RemoteSlotEntry>,
    track_capacity: Arc<Semaphore>,
    limits: RemoteManagerLimits,
    tasks: TaskTracker,
    manager_shutdown: CancellationToken,
    request_capacity: RequestCapacity,
}

impl Remote {
    /// Connect to a remote relay with a dedicated QUIC client.
    async fn connect(
        url: Url,
        addr: Option<SocketAddr>,
        client: &quic::Client,
        context: RemoteConnectContext,
    ) -> anyhow::Result<Self> {
        let RemoteConnectContext {
            remotes,
            cache_key,
            cache_slot,
            track_capacity,
            limits,
            tasks,
            manager_shutdown,
            request_capacity,
        } = context;
        let (target, policy) = quic::compatibility_target(&url)?;
        let connection = match client.connect_target(&target, policy, addr).await {
            Ok(connection) => connection,
            Err(err) => {
                metrics::counter!("moq_relay_upstream_errors_total", "stage" => "connect")
                    .increment(1);
                return Err(err);
            }
        };

        let (session, subscriber) = match moq_transport::session::Subscriber::connect_with_capacity(
            connection.session,
            connection.negotiated,
            &request_capacity,
        )
        .await
        {
            Ok(session) => session,
            Err(err) => {
                metrics::counter!("moq_relay_upstream_errors_total", "stage" => "session")
                    .increment(1);
                return Err(err.into());
            }
        };

        let connected = Arc::new(AtomicBool::new(true));
        let cancel = CancellationToken::new();
        let upstream_guard = GaugeGuard::new("moq_relay_upstream_connections");

        let session_url = url.clone();
        let session_connected = connected.clone();
        let session_cancel = cancel.clone();

        spawn_supervised(&tasks, async move {
            let _upstream_guard = upstream_guard;
            tokio::select! {
                result = session.run() => {
                    if let Err(err) = result {
                        tracing::warn!(remote_url = %crate::redact_url_for_logging(&session_url), error = %err, "remote session closed");
                    } else {
                        tracing::info!(remote_url = %crate::redact_url_for_logging(&session_url), "remote session closed normally");
                    }
                }
                _ = session_cancel.cancelled() => {
                    tracing::info!(remote_url = %crate::redact_url_for_logging(&session_url), "remote session cancelled");
                }
                _ = manager_shutdown.cancelled() => {
                    tracing::info!(remote_url = %crate::redact_url_for_logging(&session_url), "remote manager dropped or shut down");
                }
            }

            session_connected.store(false, Ordering::Release);
            session_cancel.cancel();

            if let Some(cache_slot) = cache_slot.upgrade() {
                let mut cleared = false;
                let mut cached = cache_slot.state.lock().await;
                if matches!(cached.remote.as_ref(), Some(remote) if Arc::ptr_eq(&remote.connected, &session_connected))
                {
                    cached.remote.take();
                    cleared = true;
                    tracing::info!(remote_url = %crate::redact_url_for_logging(&session_url), "cleared closed remote connection from cache");
                }
                drop(cached);

                if cleared {
                    if let Some(remotes) = remotes.upgrade() {
                        remove_empty_remote_slot(&remotes, &cache_key, &cache_slot).await;
                    }
                }
            }
        });

        let tracks = RemoteTrackCache::new(
            track_capacity,
            limits.max_tracks,
            limits.track_idle_timeout,
            tasks.clone(),
            cancel.clone(),
            url.clone(),
        );

        Ok(Self {
            url,
            subscriber,
            tracks,
            tasks,
            active_operations: Arc::new(AtomicUsize::new(0)),
            active_subscriptions: Arc::new(AtomicUsize::new(0)),
            connected,
            cancel,
        })
    }

    /// Check if the connection is still alive.
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn is_same_connection(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.connected, &other.connected)
    }

    fn is_active(&self) -> bool {
        self.active_operations.load(Ordering::Acquire) != 0
            || self.active_subscriptions.load(Ordering::Acquire) != 0
    }

    /// Shutdown the remote connection.
    async fn shutdown(&self) {
        self.cancel.cancel();
        self.connected.store(false, Ordering::Release);
        self.tracks.clear().await;
    }

    /// Subscribe to a track on this remote relay.
    async fn subscribe(
        &self,
        namespace: TrackNamespace,
        track_name: TrackName,
    ) -> anyhow::Result<Option<TrackReader>> {
        let _operation = AtomicCountGuard::new(self.active_operations.clone());
        let key = (namespace.clone(), track_name.clone());

        loop {
            if !self.is_connected() {
                anyhow::bail!(
                    "remote connection to {} is closed",
                    crate::redact_url_for_logging(&self.url)
                );
            }

            let slot = self.tracks.get_or_insert(key.clone()).await?;

            let mut cached = slot.state.lock().await;
            cached.last_used = Instant::now();

            let is_current_slot = {
                let tracks = self.tracks.entries.lock().await;
                matches!(tracks.get(&key), Some(current) if Arc::ptr_eq(current, &slot))
            };

            if !is_current_slot {
                continue;
            }

            if let Some(reader) = cached.reader.as_ref() {
                if !reader.is_closed() {
                    return Ok(Some(reader.clone()));
                }

                tracing::debug!(remote_url = %crate::redact_url_for_logging(&self.url), namespace = %key.0, track = %key.1, "removing closed remote track from cache");
            }

            cached.reader.take();

            let mut subscriber = self.subscriber.clone();
            let url = self.url.clone();
            let tracks = Arc::downgrade(&self.tracks.entries);
            let cancel = self.cancel.clone();
            let track_cancel = slot.idle_cancel.clone();

            tracing::info!(remote_url = %crate::redact_url_for_logging(&url), namespace = %key.0, track = %key.1, "subscribing to remote track");

            let (writer, reader) = Track::new(namespace.clone(), track_name.clone()).produce();
            let subscribe_result = tokio::select! {
                result = subscriber.subscribe_open(writer) => result,
                _ = cancel.cancelled() => {
                    drop(cached);
                    remove_empty_track_slot(&self.tracks.entries, &key, &slot).await;
                    anyhow::bail!("subscribe cancelled, remote connection to {} is closed", crate::redact_url_for_logging(&self.url));
                }
            };

            let subscribe = match subscribe_result {
                Ok(subscribe) => subscribe,
                Err(err) => {
                    drop(cached);
                    remove_empty_track_slot(&self.tracks.entries, &key, &slot).await;
                    return Err(err.into());
                }
            };

            if !self.is_connected() {
                drop(cached);
                remove_empty_track_slot(&self.tracks.entries, &key, &slot).await;
                anyhow::bail!(
                    "remote connection to {} is closed",
                    crate::redact_url_for_logging(&self.url)
                );
            }

            cached.reader = Some(reader.clone());
            drop(cached);

            let cleanup_key = key.clone();
            let cleanup_info = reader.info.clone();
            let cleanup_slot = slot.clone();
            let active_subscriptions = self.active_subscriptions.clone();
            let subscription = AtomicCountGuard::new(active_subscriptions);
            spawn_supervised(&self.tasks, async move {
                let _subscription = subscription;
                tokio::select! {
                    result = subscribe.closed() => {
                        match result {
                            Ok(()) => {
                                tracing::debug!(remote_url = %crate::redact_url_for_logging(&url), namespace = %cleanup_key.0, track = %cleanup_key.1, "remote track subscription ended");
                            }
                            Err(err) => {
                                tracing::warn!(remote_url = %crate::redact_url_for_logging(&url), namespace = %cleanup_key.0, track = %cleanup_key.1, error = %err, "remote track subscription ended with error");
                            }
                        }
                    }
                    _ = cancel.cancelled() => {
                        tracing::debug!(remote_url = %crate::redact_url_for_logging(&url), namespace = %cleanup_key.0, track = %cleanup_key.1, "remote track subscription cancelled");
                    }
                    _ = track_cancel.cancelled() => {
                        tracing::debug!(remote_url = %crate::redact_url_for_logging(&url), namespace = %cleanup_key.0, track = %cleanup_key.1, "idle remote track subscription cancelled");
                    }
                }

                if let Some(tracks) = tracks.upgrade() {
                    let mut cached = cleanup_slot.state.lock().await;
                    if matches!(cached.reader.as_ref(), Some(current) if Arc::ptr_eq(&current.info, &cleanup_info))
                    {
                        cached.reader.take();
                    }
                    drop(cached);

                    remove_empty_track_slot(&tracks, &cleanup_key, &cleanup_slot).await;
                }
            });

            return Ok(Some(reader));
        }
    }
}

struct AtomicCountGuard {
    count: Arc<AtomicUsize>,
}

impl AtomicCountGuard {
    fn new(count: Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self { count }
    }
}

impl Drop for AtomicCountGuard {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("url", &crate::redact_url_for_logging(&self.url))
            .field("connected", &self.is_connected())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoordinatorResult, NamespaceOrigin, NamespaceRegistration};

    struct NotFoundCoordinator;

    #[async_trait::async_trait]
    impl Coordinator for NotFoundCoordinator {
        async fn register_namespace(
            &self,
            _scope: Option<&str>,
            _namespace: &TrackNamespace,
        ) -> CoordinatorResult<NamespaceRegistration> {
            Ok(NamespaceRegistration::new(()))
        }

        async fn unregister_namespace(
            &self,
            _scope: Option<&str>,
            _namespace: &TrackNamespace,
        ) -> CoordinatorResult<()> {
            Ok(())
        }

        async fn lookup(
            &self,
            _scope: Option<&str>,
            _namespace: &TrackNamespace,
        ) -> CoordinatorResult<(NamespaceOrigin, Option<quic::Client>)> {
            Err(CoordinatorError::NamespaceNotFound)
        }
    }

    fn limits(max_connections: usize, max_tracks: usize) -> RemoteManagerLimits {
        RemoteManagerLimits {
            max_connections,
            max_tracks,
            track_idle_timeout: DEFAULT_TRACK_IDLE_TIMEOUT,
            connection_idle_timeout: DEFAULT_CONNECTION_IDLE_TIMEOUT,
        }
    }

    fn manager(limits: RemoteManagerLimits) -> RemoteManager {
        RemoteManager::with_limits(Arc::new(NotFoundCoordinator), Vec::new(), limits).unwrap()
    }

    fn remote_key(scope: Option<&str>, suffix: usize) -> RemoteCacheKey {
        RemoteCacheKey::new(
            scope,
            Url::parse(&format!("https://relay-{suffix}.example.test/endpoint")).unwrap(),
            None,
        )
    }

    fn track_key(suffix: usize) -> TrackCacheKey {
        (
            TrackNamespace::from_utf8_path(&format!("namespace/{suffix}")),
            TrackName::from(format!("track-{suffix}")),
        )
    }

    fn track_cache(
        max_tracks: usize,
        idle_timeout: Duration,
    ) -> (RemoteTrackCache, Arc<Semaphore>, TaskTracker) {
        let capacity = Arc::new(Semaphore::new(max_tracks));
        let tasks = TaskTracker::new();
        let cache = RemoteTrackCache::new(
            capacity.clone(),
            max_tracks,
            idle_timeout,
            tasks.clone(),
            CancellationToken::new(),
            Url::parse("https://relay.example.test/endpoint").unwrap(),
        );
        (cache, capacity, tasks)
    }

    #[test]
    fn remote_manager_limits_defaults_and_validation_are_stable() {
        assert_eq!(
            RemoteManagerLimits::default(),
            RemoteManagerLimits {
                max_connections: 128,
                max_tracks: 4_096,
                track_idle_timeout: Duration::from_secs(30),
                connection_idle_timeout: Duration::from_secs(60),
            }
        );

        let valid = RemoteManagerLimits::default();
        for (invalid, expected) in [
            (
                RemoteManagerLimits {
                    max_connections: 0,
                    ..valid
                },
                RemoteManagerLimitsError::ZeroConnections,
            ),
            (
                RemoteManagerLimits {
                    max_tracks: 0,
                    ..valid
                },
                RemoteManagerLimitsError::ZeroTracks,
            ),
            (
                RemoteManagerLimits {
                    max_connections: Semaphore::MAX_PERMITS.saturating_add(1),
                    ..valid
                },
                RemoteManagerLimitsError::TooManyConnections,
            ),
            (
                RemoteManagerLimits {
                    max_tracks: Semaphore::MAX_PERMITS.saturating_add(1),
                    ..valid
                },
                RemoteManagerLimitsError::TooManyTracks,
            ),
            (
                RemoteManagerLimits {
                    track_idle_timeout: Duration::ZERO,
                    ..valid
                },
                RemoteManagerLimitsError::ZeroTrackIdleTimeout,
            ),
            (
                RemoteManagerLimits {
                    connection_idle_timeout: Duration::ZERO,
                    ..valid
                },
                RemoteManagerLimitsError::ZeroConnectionIdleTimeout,
            ),
        ] {
            assert_eq!(invalid.validate(), Err(expected));
        }
    }

    #[test]
    fn final_manager_drop_cancels_standalone_tasks() {
        let manager = manager(RemoteManagerLimits::default());
        let shutdown = manager.shutdown.clone();
        let clone = manager.clone();
        drop(manager);
        assert!(!shutdown.is_cancelled());
        drop(clone);
        assert!(shutdown.is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn resolved_scopes_have_isolated_connection_slots() {
        let manager = manager(limits(2, 2));
        let url = Url::parse("https://relay.example.test/endpoint").unwrap();
        let alpha_key = RemoteCacheKey::new(Some("alpha"), url.clone(), None);
        let beta_key = RemoteCacheKey::new(Some("beta"), url.clone(), None);

        let alpha = manager
            .get_or_insert_remote_slot(alpha_key.clone())
            .await
            .unwrap();
        let alpha_again = manager.get_or_insert_remote_slot(alpha_key).await.unwrap();
        let beta = manager.get_or_insert_remote_slot(beta_key).await.unwrap();

        assert!(Arc::ptr_eq(&alpha, &alpha_again));
        assert!(!Arc::ptr_eq(&alpha, &beta));
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.cached_connections, 2);
        assert_eq!(snapshot.retained_connections, 2);
        assert_eq!(snapshot.retained_tracks, 0);

        drop((alpha, alpha_again, beta));
        manager.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn connection_capacity_rejects_n_plus_one_and_releases_exactly() {
        let manager = manager(limits(2, 2));
        let first_key = remote_key(Some("scope"), 1);
        let first = manager
            .get_or_insert_remote_slot(first_key.clone())
            .await
            .unwrap();
        let second = manager
            .get_or_insert_remote_slot(remote_key(Some("scope"), 2))
            .await
            .unwrap();

        let error = manager
            .get_or_insert_remote_slot(remote_key(Some("scope"), 3))
            .await
            .err()
            .expect("N+1 connection must be rejected");
        assert!(error.to_string().contains("capacity exhausted"));
        assert_eq!(
            error.downcast_ref::<RemoteCapacityError>(),
            Some(&RemoteCapacityError {
                resource: RemoteCapacityResource::Connection,
                limit: 2,
            })
        );
        assert_eq!(manager.snapshot().await.retained_connections, 2);

        remove_empty_remote_slot(&manager.remotes, &first_key, &first).await;
        drop(first);
        tokio::task::yield_now().await;
        assert_eq!(manager.snapshot().await.retained_connections, 1);

        let replacement = manager
            .get_or_insert_remote_slot(remote_key(Some("scope"), 3))
            .await
            .unwrap();
        assert_eq!(manager.snapshot().await.retained_connections, 2);

        drop((second, replacement));
        manager.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn track_capacity_rejects_n_plus_one_and_releases_exactly() {
        let (cache, capacity, tasks) = track_cache(2, DEFAULT_TRACK_IDLE_TIMEOUT);
        let first_key = track_key(1);
        let first = cache.get_or_insert(first_key.clone()).await.unwrap();
        let second = cache.get_or_insert(track_key(2)).await.unwrap();

        let error = cache
            .get_or_insert(track_key(3))
            .await
            .err()
            .expect("N+1 track must be rejected");
        assert!(error.to_string().contains("capacity exhausted"));
        assert_eq!(
            error.downcast_ref::<RemoteCapacityError>(),
            Some(&RemoteCapacityError {
                resource: RemoteCapacityResource::Track,
                limit: 2,
            })
        );
        assert_eq!(capacity.available_permits(), 0);

        remove_empty_track_slot(&cache.entries, &first_key, &first).await;
        drop(first);
        tokio::task::yield_now().await;
        assert_eq!(capacity.available_permits(), 1);

        let replacement = cache.get_or_insert(track_key(3)).await.unwrap();
        assert_eq!(capacity.available_permits(), 0);

        drop((second, replacement));
        cache.cancel.cancel();
        cache.clear().await;
        tasks.close();
        tasks.wait().await;
        assert_eq!(capacity.available_permits(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn idle_slots_evict_at_default_deadlines_and_release_capacity() {
        let manager = manager(limits(1, 1));
        let connection = manager
            .get_or_insert_remote_slot(remote_key(Some("scope"), 1))
            .await
            .unwrap();
        drop(connection);
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(59)).await;
        tokio::task::yield_now().await;
        assert_eq!(manager.snapshot().await.cached_connections, 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(manager.snapshot().await.cached_connections, 0);
        assert_eq!(manager.snapshot().await.retained_connections, 0);

        let (cache, capacity, tasks) = track_cache(1, DEFAULT_TRACK_IDLE_TIMEOUT);
        let track = cache.get_or_insert(track_key(1)).await.unwrap();
        drop(track);
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(29)).await;
        tokio::task::yield_now().await;
        assert_eq!(cache.entries.lock().await.len(), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(cache.entries.lock().await.len(), 0);
        assert_eq!(capacity.available_permits(), 1);

        cache.cancel.cancel();
        tasks.close();
        tasks.wait().await;
        manager.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn track_idle_eviction_waits_for_external_readers_then_releases_capacity() {
        let (cache, capacity, tasks) = track_cache(1, DEFAULT_TRACK_IDLE_TIMEOUT);
        let key = track_key(1);
        let slot = cache.get_or_insert(key.clone()).await.unwrap();
        let (writer, external_reader) = Track::new(key.0.clone(), key.1.clone()).produce();
        slot.state.lock().await.reader = Some(external_reader.clone());
        drop(slot);
        tokio::task::yield_now().await;

        tokio::time::advance(DEFAULT_TRACK_IDLE_TIMEOUT).await;
        tokio::task::yield_now().await;
        assert_eq!(cache.entries.lock().await.len(), 1);
        assert_eq!(capacity.available_permits(), 0);

        drop(external_reader);
        tokio::time::advance(DEFAULT_TRACK_IDLE_TIMEOUT).await;
        tokio::task::yield_now().await;
        assert_eq!(cache.entries.lock().await.len(), 0);
        assert_eq!(capacity.available_permits(), 1);

        drop(writer);
        cache.cancel.cancel();
        tasks.close();
        tasks.wait().await;
    }

    #[tokio::test(start_paused = true)]
    async fn connection_and_track_floods_remain_bounded() {
        let manager = manager(limits(8, 16));
        let connection_results = futures::future::join_all((0..256).map(|index| {
            let manager = manager.clone();
            async move {
                manager
                    .get_or_insert_remote_slot(remote_key(Some("scope"), index))
                    .await
            }
        }))
        .await;
        assert_eq!(connection_results.iter().filter(|r| r.is_ok()).count(), 8);
        assert_eq!(
            connection_results.iter().filter(|r| r.is_err()).count(),
            248
        );
        assert_eq!(manager.snapshot().await.cached_connections, 8);
        assert_eq!(manager.snapshot().await.retained_connections, 8);

        let (cache, capacity, tasks) = track_cache(16, DEFAULT_TRACK_IDLE_TIMEOUT);
        let track_results = futures::future::join_all((0..512).map(|index| {
            let cache = cache.clone();
            async move { cache.get_or_insert(track_key(index)).await }
        }))
        .await;
        assert_eq!(track_results.iter().filter(|r| r.is_ok()).count(), 16);
        assert_eq!(track_results.iter().filter(|r| r.is_err()).count(), 496);
        assert_eq!(cache.entries.lock().await.len(), 16);
        assert_eq!(capacity.available_permits(), 0);

        drop(connection_results);
        drop(track_results);
        cache.cancel.cancel();
        cache.clear().await;
        tasks.close();
        tasks.wait().await;
        assert_eq!(capacity.available_permits(), 16);
        manager.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_insert_remove_churn_does_not_retain_slots_or_tasks() {
        let manager = manager(limits(1, 1));
        for index in 0..1_024 {
            let key = remote_key(Some("scope"), index);
            let slot = manager
                .get_or_insert_remote_slot(key.clone())
                .await
                .unwrap();
            remove_empty_remote_slot(&manager.remotes, &key, &slot).await;
            drop(slot);
            if index % 32 == 0 {
                tokio::task::yield_now().await;
            }
        }
        tokio::task::yield_now().await;
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.cached_connections, 0);
        assert_eq!(snapshot.retained_connections, 0);
        assert_eq!(snapshot.supervised_tasks, 0);

        let (cache, capacity, tasks) = track_cache(1, DEFAULT_TRACK_IDLE_TIMEOUT);
        for index in 0..1_024 {
            let key = track_key(index);
            let slot = cache.get_or_insert(key.clone()).await.unwrap();
            remove_empty_track_slot(&cache.entries, &key, &slot).await;
            drop(slot);
            if index % 32 == 0 {
                tokio::task::yield_now().await;
            }
        }
        tokio::task::yield_now().await;
        assert!(cache.entries.lock().await.is_empty());
        assert_eq!(capacity.available_permits(), 1);
        assert_eq!(tasks.len(), 0);

        cache.cancel.cancel();
        tasks.close();
        tasks.wait().await;
        manager.shutdown().await;
    }

    #[test]
    fn remote_diagnostics_never_render_urls_directly() {
        let source = include_str!("remote.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("remote implementation precedes its tests");

        for line in implementation
            .lines()
            .filter(|line| line.contains("remote_url = %"))
        {
            assert!(
                line.contains("redact_url_for_logging"),
                "raw remote URL diagnostic: {line}"
            );
        }

        let raw_debug = [".field(\"url\", &self.url", ".to_string())"].concat();
        assert!(
            !implementation.contains(&raw_debug),
            "Remote Debug must use the bounded URL diagnostic"
        );
        assert!(
            !implementation.contains("\"scope\" =>"),
            "resolved scopes must not become high-cardinality metric labels"
        );
    }
}
