// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! File-based coordinator for multi-relay deployments.
//!
//! This coordinator uses a shared JSON file with file locking to coordinate
//! namespace registration across multiple relay instances. No separate
//! server process is required.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
#[cfg(test)]
use std::sync::{Barrier, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use fs2::FileExt;
use moq_native_ietf::quic::Client;
use moq_transport::coding::{TrackNamespace, TupleField};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use url::Url;

use moq_relay_ietf::{
    Coordinator, CoordinatorError, CoordinatorResult, NamespaceInfo, NamespaceOrigin,
    NamespaceRegistration, NamespaceSubscription, NamespaceUpdate, NamespaceUpdateSender,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileCoordinatorLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for FileCoordinatorLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_bytes: 16 * 1024 * 1024,
        }
    }
}

impl FileCoordinatorLimits {
    fn validate(self) -> Result<Self> {
        anyhow::ensure!(
            self.max_entries > 0,
            "file coordinator entry limit must be positive"
        );
        anyhow::ensure!(
            self.max_bytes > 0,
            "file coordinator byte limit must be positive"
        );
        Ok(self)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("file coordinator capacity exhausted for {resource}")]
struct FileCoordinatorCapacityError {
    resource: &'static str,
}

const COORDINATOR_DATA_VERSION: u8 = 1;
const NAMESPACE_KEY_PREFIX: &str = "v1:";
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const NAMESPACE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_NAMESPACE_UPDATE_QUEUE_CAPACITY: usize = 256;

/// Data stored in the shared file
#[derive(Debug, Serialize, Deserialize)]
struct CoordinatorData {
    #[serde(default)]
    version: u8,
    /// Maps connection scope to namespace map
    namespaces: HashMap<String, HashMap<String, String>>,
}

impl Default for CoordinatorData {
    fn default() -> Self {
        Self {
            version: COORDINATOR_DATA_VERSION,
            namespaces: HashMap::new(),
        }
    }
}

impl CoordinatorData {
    fn scope_key(scope: Option<&str>) -> String {
        match scope {
            Some(scope) => format!("s:{}", hex::encode(scope.as_bytes())),
            None => "u".to_string(),
        }
    }

    fn namespace_key(namespace: &TrackNamespace) -> String {
        let fields = namespace
            .fields
            .iter()
            .map(|field| hex::encode(&field.value))
            .collect::<Vec<_>>()
            .join(".");
        format!("{NAMESPACE_KEY_PREFIX}{fields}")
    }

    fn namespace_from_key(key: &str) -> Result<TrackNamespace> {
        let encoded = key
            .strip_prefix(NAMESPACE_KEY_PREFIX)
            .context("unsupported coordinator namespace key version")?;
        let fields = encoded
            .split('.')
            .map(|field| {
                Ok(TupleField {
                    value: hex::decode(field).context("invalid coordinator namespace key")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        TrackNamespace::try_from(fields).context("invalid coordinator namespace")
    }

    fn migrate_and_validate(&mut self) -> Result<()> {
        match self.version {
            COORDINATOR_DATA_VERSION => {
                for (scope, bucket) in &self.namespaces {
                    if scope != "u" {
                        let encoded = scope
                            .strip_prefix("s:")
                            .context("invalid coordinator scope key")?;
                        hex::decode(encoded).context("invalid coordinator scope key")?;
                    }
                    for key in bucket.keys() {
                        Self::namespace_from_key(key)?;
                    }
                }
            }
            0 => {
                let legacy_scopes = std::mem::take(&mut self.namespaces);
                for (scope, legacy) in legacy_scopes {
                    let scope = if scope.is_empty() {
                        Self::scope_key(None)
                    } else {
                        Self::scope_key(Some(&scope))
                    };
                    let bucket = self.namespaces.entry(scope).or_default();
                    for (key, url) in legacy {
                        let namespace = TrackNamespace::try_from(key.as_str())
                            .context("invalid legacy coordinator namespace")?;
                        let key = Self::namespace_key(&namespace);
                        anyhow::ensure!(
                            bucket.insert(key, url).is_none(),
                            "legacy coordinator namespace migration collision"
                        );
                    }
                }
                self.version = COORDINATOR_DATA_VERSION;
            }
            version => anyhow::bail!("unsupported coordinator data version {version}"),
        }
        Ok(())
    }

    fn entry_count(&self) -> usize {
        self.namespaces
            .values()
            .map(HashMap::len)
            .fold(0usize, usize::saturating_add)
    }
}

struct FileCleanupRequest {
    scope_key: String,
    namespace_key: String,
    _capacity: OwnedSemaphorePermit,
}

struct FileCleanupWorker {
    sender: mpsc::SyncSender<FileCleanupRequest>,
    notify: Arc<tokio::sync::Notify>,
}

impl FileCleanupWorker {
    fn new(file_path: PathBuf, limits: FileCoordinatorLimits) -> Result<Arc<Self>> {
        let (sender, receiver) = mpsc::sync_channel::<FileCleanupRequest>(limits.max_entries);
        let notify = Arc::new(tokio::sync::Notify::new());
        let worker_notify = notify.clone();
        std::thread::Builder::new()
            .name("moq-file-coordinator-cleanup".to_string())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if let Err(error) = unregister_namespace_sync(
                        &file_path,
                        &request.scope_key,
                        &request.namespace_key,
                        limits,
                    ) {
                        tracing::warn!(%error, "file coordinator cleanup failed");
                    }
                    drop(request);
                    worker_notify.notify_one();
                }
            })?;
        Ok(Arc::new(Self { sender, notify }))
    }

    fn enqueue(&self, request: FileCleanupRequest) {
        if let Err(error) = self.sender.try_send(request) {
            metrics::counter!(
                "moq_relay_coordinator_capacity_rejections_total",
                "kind" => "file_cleanup_queue"
            )
            .increment(1);
            tracing::error!(%error, "bounded file coordinator cleanup queue invariant violated");
        }
    }
}

/// Handle that asynchronously unregisters a namespace when dropped.
struct NamespaceUnregisterHandle {
    request: Option<FileCleanupRequest>,
    worker: Arc<FileCleanupWorker>,
}

/// Cancels a bounded namespace watcher when the owning subscription drops.
struct NamespaceSubscriptionHandle {
    cancel: watch::Sender<bool>,
}

impl Drop for NamespaceSubscriptionHandle {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}

impl Drop for NamespaceUnregisterHandle {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            self.worker.enqueue(request);
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
struct RegistrationCommitHook {
    barriers: Arc<Mutex<Option<RegistrationBarriers>>>,
}

#[cfg(test)]
type RegistrationBarriers = (Arc<Barrier>, Arc<Barrier>);

#[cfg(test)]
impl RegistrationCommitHook {
    fn new(committed: Arc<Barrier>, release: Arc<Barrier>) -> Self {
        Self {
            barriers: Arc::new(Mutex::new(Some((committed, release)))),
        }
    }

    fn after_commit(&self) {
        let barriers = self.barriers.lock().unwrap().take();
        if let Some((committed, release)) = barriers {
            committed.wait();
            release.wait();
        }
    }
}

/// Synchronous helper for unregistering namespace (used in Drop)
fn unregister_namespace_sync(
    file_path: &Path,
    scope_key: &str,
    namespace_key: &str,
    limits: FileCoordinatorLimits,
) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(file_path)?;

    file.lock_exclusive()?;

    let mut data = read_data(&file, limits)?;
    tracing::debug!("unregistering namespace from file coordinator");
    if let Some(bucket) = data.namespaces.get_mut(scope_key) {
        bucket.remove(namespace_key);
        if bucket.is_empty() {
            data.namespaces.remove(scope_key);
        }
    }

    write_data(&file, &data, limits)?;
    file.unlock()?;

    Ok(())
}

/// Read coordinator data from file
fn read_data(file: &File, limits: FileCoordinatorLimits) -> Result<CoordinatorData> {
    let mut file = file;
    file.seek(SeekFrom::Start(0))?;

    anyhow::ensure!(
        file.metadata()?.len() <= limits.max_bytes as u64,
        "coordinator file exceeds configured byte limit"
    );

    let read_limit = u64::try_from(limits.max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut contents = Vec::with_capacity(limits.max_bytes.min(64 * 1024));
    file.take(read_limit).read_to_end(&mut contents)?;
    anyhow::ensure!(
        contents.len() <= limits.max_bytes,
        "coordinator file exceeds configured byte limit"
    );

    if contents.is_empty() {
        return Ok(CoordinatorData::default());
    }

    let mut data: CoordinatorData =
        serde_json::from_slice(&contents).context("failed to parse coordinator data")?;
    data.migrate_and_validate()?;
    anyhow::ensure!(
        data.entry_count() <= limits.max_entries,
        "coordinator file exceeds configured entry limit"
    );
    Ok(data)
}

fn matching_namespaces_sync(
    file_path: &Path,
    scope_key: &str,
    prefix: &TrackNamespace,
    limits: FileCoordinatorLimits,
) -> Result<Vec<NamespaceInfo>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(file_path)?;
    file.lock_shared()?;
    let data = read_data(&file, limits)?;
    let mut matches = Vec::new();
    if let Some(bucket) = data.namespaces.get(scope_key) {
        for key in bucket.keys() {
            let namespace = CoordinatorData::namespace_from_key(key)?;
            let is_match = prefix.fields.len() <= namespace.fields.len()
                && prefix
                    .fields
                    .iter()
                    .zip(&namespace.fields)
                    .all(|(expected, actual)| expected == actual);
            if is_match {
                matches.push(NamespaceInfo::new(namespace));
            }
        }
    }
    file.unlock()?;
    matches.sort_by(|left, right| {
        left.namespace
            .to_utf8_path()
            .cmp(&right.namespace.to_utf8_path())
    });
    Ok(matches)
}

struct NamespaceWatcherConfig {
    file_path: PathBuf,
    scope_key: String,
    prefix: TrackNamespace,
    limits: FileCoordinatorLimits,
}

async fn supervise_namespace_updates(
    config: NamespaceWatcherConfig,
    mut known: HashSet<TrackNamespace>,
    updates: NamespaceUpdateSender,
    mut cancel: watch::Receiver<bool>,
    _capacity: OwnedSemaphorePermit,
) {
    let mut interval = tokio::time::interval(NAMESPACE_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    return;
                }
            }
            _ = interval.tick() => {}
        }

        let poll_path = config.file_path.clone();
        let poll_scope = config.scope_key.clone();
        let poll_prefix = config.prefix.clone();
        let limits = config.limits;
        let current = match tokio::task::spawn_blocking(move || {
            matching_namespaces_sync(&poll_path, &poll_scope, &poll_prefix, limits)
        })
        .await
        {
            Ok(Ok(current)) => current,
            Ok(Err(error)) => {
                tracing::warn!(%error, "namespace subscription polling failed closed");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "namespace subscription polling task failed closed");
                return;
            }
        };
        let current: HashSet<_> = current.into_iter().map(|info| info.namespace).collect();

        let mut added: Vec<_> = current.difference(&known).cloned().collect();
        let mut removed: Vec<_> = known.difference(&current).cloned().collect();
        added.sort_by_key(TrackNamespace::to_utf8_path);
        removed.sort_by_key(TrackNamespace::to_utf8_path);

        for namespace in added {
            if updates
                .try_send(NamespaceUpdate::Added(NamespaceInfo::new(namespace)))
                .is_err()
            {
                return;
            }
        }
        for namespace in removed {
            if updates
                .try_send(NamespaceUpdate::Removed(NamespaceInfo::new(namespace)))
                .is_err()
            {
                return;
            }
        }
        known = current;
    }
}

/// Write coordinator data to file
fn write_data(file: &File, data: &CoordinatorData, limits: FileCoordinatorLimits) -> Result<()> {
    anyhow::ensure!(
        data.version == COORDINATOR_DATA_VERSION,
        "coordinator data must be migrated before writing"
    );
    anyhow::ensure!(
        data.entry_count() <= limits.max_entries,
        "coordinator file exceeds configured entry limit"
    );
    let json = serde_json::to_vec_pretty(data)?;
    anyhow::ensure!(
        json.len() <= limits.max_bytes,
        "coordinator file exceeds configured byte limit"
    );

    let mut file = file;
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&json)?;
    file.flush()?;

    Ok(())
}

/// A coordinator that uses a shared file for state storage.
///
/// Multiple relay instances can use the same file to share namespace/track
/// registration data. File locking ensures safe concurrent access.
pub struct FileCoordinator {
    /// Path to the shared coordination file
    file_path: PathBuf,
    /// URL of this relay (used when registering namespaces)
    relay_url: Url,
    limits: FileCoordinatorLimits,
    cleanup_capacity: Arc<Semaphore>,
    subscription_capacity: Arc<Semaphore>,
    cleanup_worker: Arc<FileCleanupWorker>,
    #[cfg(test)]
    registration_commit_hook: Option<RegistrationCommitHook>,
}

impl FileCoordinator {
    pub fn with_limits(
        file_path: impl AsRef<Path>,
        relay_url: Url,
        limits: FileCoordinatorLimits,
    ) -> Result<Self> {
        let file_path = file_path.as_ref().to_path_buf();
        let limits = limits.validate()?;
        Ok(Self {
            cleanup_worker: FileCleanupWorker::new(file_path.clone(), limits)?,
            cleanup_capacity: Arc::new(Semaphore::new(limits.max_entries)),
            subscription_capacity: Arc::new(Semaphore::new(limits.max_entries)),
            file_path,
            relay_url,
            limits,
            #[cfg(test)]
            registration_commit_hook: None,
        })
    }

    #[cfg(test)]
    fn with_registration_commit_hook(mut self, hook: RegistrationCommitHook) -> Self {
        self.registration_commit_hook = Some(hook);
        self
    }
}

#[async_trait]
impl Coordinator for FileCoordinator {
    async fn register_namespace(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<NamespaceRegistration> {
        let cleanup_permit = self
            .cleanup_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| CoordinatorError::CapacityExhausted {
                resource: "file_registration_handles",
            })?;
        let scope_key = CoordinatorData::scope_key(scope);
        let namespace_key = CoordinatorData::namespace_key(namespace);
        let relay_url = self.relay_url.clone();
        let file_path = self.file_path.clone();
        let limits = self.limits;
        let cleanup_worker = self.cleanup_worker.clone();
        #[cfg(test)]
        let registration_commit_hook = self.registration_commit_hook.clone();

        // Move cleanup ownership into the blocking transaction. If this async
        // request is cancelled after the file commit, the detached blocking
        // task's output is dropped and queues bounded cleanup automatically.
        let result = tokio::task::spawn_blocking(move || {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&file_path)?;

            file.lock_exclusive()?;

            let mut data = read_data(&file, limits)?;
            tracing::info!(relay_url = %moq_relay_ietf::redact_url_for_logging(&relay_url), "registering namespace in file coordinator");
            let already_registered = data
                .namespaces
                .get(&scope_key)
                .is_some_and(|bucket| bucket.contains_key(&namespace_key));
            if !already_registered && data.entry_count() >= limits.max_entries {
                return Err(anyhow::Error::new(FileCoordinatorCapacityError {
                    resource: "file_entries",
                }));
            }
            data
                .namespaces
                .entry(scope_key.clone())
                .or_default()
                .insert(namespace_key.clone(), relay_url.to_string());

            let serialized = serde_json::to_vec_pretty(&data)?;
            if serialized.len() > limits.max_bytes {
                return Err(anyhow::Error::new(FileCoordinatorCapacityError {
                    resource: "file_bytes",
                }));
            }
            write_data(&file, &data, limits)?;

            let handle = NamespaceUnregisterHandle {
                request: Some(FileCleanupRequest {
                    scope_key,
                    namespace_key,
                    _capacity: cleanup_permit,
                }),
                worker: cleanup_worker,
            };

            file.unlock()?;

            #[cfg(test)]
            if let Some(hook) = registration_commit_hook {
                hook.after_commit();
            }

            Ok::<_, anyhow::Error>(handle)
        })
        .await?;
        let handle = match result {
            Ok(handle) => handle,
            Err(error) => {
                if let Some(capacity) = error.downcast_ref::<FileCoordinatorCapacityError>() {
                    return Err(CoordinatorError::CapacityExhausted {
                        resource: capacity.resource,
                    });
                }
                return Err(CoordinatorError::Other(error));
            }
        };

        Ok(NamespaceRegistration::new(handle))
    }

    // Explicit request-stream cancellation can call this; ordinary cleanup
    // currently unregisters when the namespace registration handle is dropped.
    async fn unregister_namespace(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<()> {
        let scope_key = CoordinatorData::scope_key(scope);
        let namespace_key = CoordinatorData::namespace_key(namespace);
        let file_path = self.file_path.clone();
        let limits = self.limits;

        tokio::task::spawn_blocking(move || {
            unregister_namespace_sync(&file_path, &scope_key, &namespace_key, limits)
        })
        .await??;

        Ok(())
    }

    async fn lookup(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<(NamespaceOrigin, Option<Client>)> {
        let namespace = namespace.clone();
        let scope_key = CoordinatorData::scope_key(scope);
        let namespace_key = CoordinatorData::namespace_key(&namespace);
        let file_path = self.file_path.clone();
        let limits = self.limits;

        let result = tokio::task::spawn_blocking(
            move || -> Result<Option<(NamespaceOrigin, Option<Client>)>> {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&file_path)?;

                file.lock_shared()?;

                let data = read_data(&file, limits)?;
                tracing::debug!("looking up namespace in file coordinator");

                let Some(bucket) = data.namespaces.get(&scope_key) else {
                    file.unlock()?;
                    return Ok(None);
                };

                // Try exact match first
                if let Some(relay_url) = bucket.get(&namespace_key) {
                    file.unlock()?;
                    let url = Url::parse(relay_url)?;
                    return Ok(Some((NamespaceOrigin::new(namespace, url, None), None)));
                }

                // Try prefix matching (find longest matching prefix)
                let mut best_match: Option<(TrackNamespace, String)> = None;
                for (registered_key, url) in bucket {
                    let registered = CoordinatorData::namespace_from_key(registered_key)?;
                    let is_prefix = registered.fields.len() <= namespace.fields.len()
                        && registered
                            .fields
                            .iter()
                            .zip(&namespace.fields)
                            .all(|(registered, requested)| registered == requested);
                    match &best_match {
                        Some((best, _))
                            if is_prefix && best.fields.len() < registered.fields.len() =>
                        {
                            best_match = Some((registered, url.clone()));
                        }
                        None if is_prefix => {
                            best_match = Some((registered, url.clone()));
                        }
                        _ => {}
                    }
                }

                file.unlock()?;

                if let Some((matched_ns, relay_url)) = best_match {
                    let url = Url::parse(&relay_url)?;
                    return Ok(Some((NamespaceOrigin::new(matched_ns, url, None), None)));
                }

                Ok(None)
            },
        )
        .await??;

        result.ok_or(CoordinatorError::NamespaceNotFound)
    }

    async fn subscribe_namespace(
        &self,
        scope: Option<&str>,
        prefix: &TrackNamespace,
    ) -> CoordinatorResult<NamespaceSubscription> {
        let capacity = self
            .subscription_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| CoordinatorError::CapacityExhausted {
                resource: "file_namespace_subscriptions",
            })?;
        let prefix = prefix.clone();
        let scope_key = CoordinatorData::scope_key(scope);
        let file_path = self.file_path.clone();
        let limits = self.limits;

        let initial_path = file_path.clone();
        let initial_scope = scope_key.clone();
        let initial_prefix = prefix.clone();
        let existing = tokio::task::spawn_blocking(move || {
            matching_namespaces_sync(&initial_path, &initial_scope, &initial_prefix, limits)
        })
        .await??;
        let known = existing.iter().map(|info| info.namespace.clone()).collect();
        let (cancel, cancel_receiver) = watch::channel(false);
        let update_capacity = limits
            .max_entries
            .clamp(1, MAX_NAMESPACE_UPDATE_QUEUE_CAPACITY);
        let (subscription, updates) = NamespaceSubscription::bounded(
            existing,
            NamespaceSubscriptionHandle { cancel },
            update_capacity,
        )
        .map_err(|error| CoordinatorError::Other(error.into()))?;
        tokio::spawn(supervise_namespace_updates(
            NamespaceWatcherConfig {
                file_path,
                scope_key,
                prefix,
                limits,
            },
            known,
            updates,
            cancel_receiver,
            capacity,
        ));

        Ok(subscription)
    }

    async fn shutdown(&self) -> CoordinatorResult<()> {
        let wait = async {
            loop {
                if self.cleanup_capacity.available_permits() == self.limits.max_entries
                    && self.subscription_capacity.available_permits() == self.limits.max_entries
                {
                    return;
                }
                tokio::select! {
                    _ = self.cleanup_worker.notify.notified() => {},
                    _ = tokio::time::sleep(NAMESPACE_POLL_INTERVAL) => {},
                }
            }
        };
        tokio::time::timeout(CLEANUP_TIMEOUT, wait)
            .await
            .context("timed out waiting for file coordinator cleanup")
            .map_err(CoordinatorError::Other)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(fields: &[&[u8]]) -> TrackNamespace {
        TrackNamespace::try_from(
            fields
                .iter()
                .map(|value| TupleField {
                    value: value.to_vec(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn relay_url() -> Url {
        Url::parse("https://relay.example.com").unwrap()
    }

    #[test]
    fn invalid_limits_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let limits = FileCoordinatorLimits {
            max_entries: 0,
            max_bytes: 1,
        };
        assert!(FileCoordinator::with_limits(
            directory.path().join("coordinator.json"),
            relay_url(),
            limits
        )
        .is_err());
    }

    #[tokio::test]
    async fn entry_capacity_is_fail_fast_and_released_by_registration_drop() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = FileCoordinator::with_limits(
            directory.path().join("coordinator.json"),
            relay_url(),
            FileCoordinatorLimits {
                max_entries: 1,
                max_bytes: 4_096,
            },
        )
        .unwrap();
        let first_namespace = TrackNamespace::from_utf8_path("first");
        let second_namespace = TrackNamespace::from_utf8_path("second");

        let registration = coordinator
            .register_namespace(Some("tenant-a"), &first_namespace)
            .await
            .unwrap();
        let error = coordinator
            .register_namespace(Some("tenant-b"), &second_namespace)
            .await
            .err()
            .expect("N+1 file entry must be rejected");
        assert!(matches!(
            error,
            CoordinatorError::CapacityExhausted {
                resource: "file_registration_handles"
            }
        ));

        drop(registration);
        coordinator.shutdown().await.unwrap();
        coordinator
            .register_namespace(Some("tenant-b"), &second_namespace)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn namespace_subscription_snapshot_is_scope_and_prefix_bound() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = FileCoordinator::with_limits(
            directory.path().join("coordinator.json"),
            relay_url(),
            FileCoordinatorLimits::default(),
        )
        .unwrap();
        let matching = TrackNamespace::from_utf8_path("shows/live/clock");
        let sibling = TrackNamespace::from_utf8_path("shows/vod/archive");
        let other_tenant = TrackNamespace::from_utf8_path("shows/live/private");
        let _matching = coordinator
            .register_namespace(Some("tenant-a"), &matching)
            .await
            .unwrap();
        let _sibling = coordinator
            .register_namespace(Some("tenant-a"), &sibling)
            .await
            .unwrap();
        let _other = coordinator
            .register_namespace(Some("tenant-b"), &other_tenant)
            .await
            .unwrap();

        let prefix = TrackNamespace::from_utf8_path("shows/live");
        let snapshot = coordinator
            .subscribe_namespace(Some("tenant-a"), &prefix)
            .await
            .unwrap();
        assert_eq!(snapshot.existing_namespaces.len(), 1);
        assert_eq!(snapshot.existing_namespaces[0].namespace, matching);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn namespace_subscription_streams_file_additions_and_removals() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = FileCoordinator::with_limits(
            directory.path().join("coordinator.json"),
            relay_url(),
            FileCoordinatorLimits::default(),
        )
        .unwrap();
        let prefix = TrackNamespace::from_utf8_path("shows/live");
        let namespace = TrackNamespace::from_utf8_path("shows/live/clock");
        let mut subscription = coordinator
            .subscribe_namespace(Some("tenant-a"), &prefix)
            .await
            .unwrap();
        assert!(subscription.existing_namespaces.is_empty());

        let registration = coordinator
            .register_namespace(Some("tenant-a"), &namespace)
            .await
            .unwrap();
        let added = tokio::time::timeout(Duration::from_secs(2), subscription.next_update())
            .await
            .expect("file watcher did not publish namespace addition")
            .unwrap();
        assert_eq!(
            added,
            NamespaceUpdate::Added(NamespaceInfo::new(namespace.clone()))
        );

        drop(registration);
        let removed = tokio::time::timeout(Duration::from_secs(2), subscription.next_update())
            .await
            .expect("file watcher did not publish namespace removal")
            .unwrap();
        assert_eq!(
            removed,
            NamespaceUpdate::Removed(NamespaceInfo::new(namespace))
        );
        drop(subscription);
        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn namespace_subscription_capacity_is_bounded_and_released() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = FileCoordinator::with_limits(
            directory.path().join("coordinator.json"),
            relay_url(),
            FileCoordinatorLimits {
                max_entries: 1,
                max_bytes: 4_096,
            },
        )
        .unwrap();
        let prefix = TrackNamespace::from_utf8_path("shows/live");
        let first = coordinator
            .subscribe_namespace(Some("tenant-a"), &prefix)
            .await
            .unwrap();
        assert!(matches!(
            coordinator
                .subscribe_namespace(Some("tenant-a"), &prefix)
                .await,
            Err(CoordinatorError::CapacityExhausted {
                resource: "file_namespace_subscriptions"
            })
        ));

        drop(first);
        coordinator.shutdown().await.unwrap();
        let second = coordinator
            .subscribe_namespace(Some("tenant-a"), &prefix)
            .await
            .unwrap();
        drop(second);
        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_registration_after_commit_cleans_route_and_releases_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("coordinator.json");
        let committed = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let coordinator = Arc::new(
            FileCoordinator::with_limits(
                &path,
                relay_url(),
                FileCoordinatorLimits {
                    max_entries: 1,
                    max_bytes: 4_096,
                },
            )
            .unwrap()
            .with_registration_commit_hook(RegistrationCommitHook::new(
                committed.clone(),
                release.clone(),
            )),
        );
        let registered = TrackNamespace::from_utf8_path("cancelled");
        let task_coordinator = coordinator.clone();
        let task_namespace = registered.clone();

        let registration = tokio::spawn(async move {
            task_coordinator
                .register_namespace(Some("tenant-a"), &task_namespace)
                .await
        });

        // The blocking transaction has committed and owns the cleanup guard,
        // but has not yet returned it to the cancelled async caller.
        tokio::task::spawn_blocking(move || committed.wait())
            .await
            .unwrap();
        registration.abort();
        assert!(matches!(
            registration.await,
            Err(error) if error.is_cancelled()
        ));
        tokio::task::spawn_blocking(move || release.wait())
            .await
            .unwrap();

        coordinator.shutdown().await.unwrap();
        assert!(matches!(
            coordinator.lookup(Some("tenant-a"), &registered).await,
            Err(CoordinatorError::NamespaceNotFound)
        ));
        assert_eq!(coordinator.cleanup_capacity.available_permits(), 1);

        // Both persistent-file capacity and the cleanup-handle permit are
        // immediately reusable for the same route after cleanup completes.
        let replacement_registration = coordinator
            .register_namespace(Some("tenant-a"), &registered)
            .await
            .unwrap();
        coordinator
            .lookup(Some("tenant-a"), &registered)
            .await
            .unwrap();
        drop(replacement_registration);
        coordinator.shutdown().await.unwrap();
        assert_eq!(coordinator.cleanup_capacity.available_permits(), 1);
    }

    #[tokio::test]
    async fn byte_capacity_rejects_before_truncating_existing_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("coordinator.json");
        let coordinator = FileCoordinator::with_limits(
            &path,
            relay_url(),
            FileCoordinatorLimits {
                max_entries: 4,
                max_bytes: 128,
            },
        )
        .unwrap();
        let small = TrackNamespace::from_utf8_path("small");
        let small_registration = coordinator.register_namespace(None, &small).await.unwrap();
        let before = std::fs::read(&path).unwrap();
        let oversized = TrackNamespace::from_utf8_path(&"x".repeat(256));

        let error = coordinator
            .register_namespace(None, &oversized)
            .await
            .err()
            .expect("oversized file entry must be rejected");
        assert!(matches!(
            error,
            CoordinatorError::CapacityExhausted {
                resource: "file_bytes"
            }
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        coordinator.lookup(None, &small).await.unwrap();
        drop(small_registration);
        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn oversized_existing_file_is_rejected_without_unbounded_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("coordinator.json");
        std::fs::write(&path, vec![b'x'; 65]).unwrap();
        let coordinator = FileCoordinator::with_limits(
            path,
            relay_url(),
            FileCoordinatorLimits {
                max_entries: 4,
                max_bytes: 64,
            },
        )
        .unwrap();
        let namespace = TrackNamespace::from_utf8_path("track");
        assert!(coordinator.lookup(None, &namespace).await.is_err());
    }

    #[tokio::test]
    async fn binary_tuple_keys_and_prefixes_are_collision_free() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("coordinator.json");
        let limits = FileCoordinatorLimits {
            max_entries: 8,
            max_bytes: 8_192,
        };
        let slash_field = namespace(&[b"a/b"]);
        let split_fields = namespace(&[b"a", b"b"]);
        let binary_prefix = namespace(&[&[0xff, 0x00]]);
        let binary_child = namespace(&[&[0xff, 0x00], b"child"]);
        let first =
            FileCoordinator::with_limits(&path, Url::parse("https://one.example").unwrap(), limits)
                .unwrap();
        let second =
            FileCoordinator::with_limits(&path, Url::parse("https://two.example").unwrap(), limits)
                .unwrap();

        let slash_registration = first
            .register_namespace(Some("/tenant?token=secret"), &slash_field)
            .await
            .unwrap();
        let split_registration = second
            .register_namespace(Some("/tenant?token=secret"), &split_fields)
            .await
            .unwrap();
        let prefix_registration = first
            .register_namespace(Some("/tenant?token=secret"), &binary_prefix)
            .await
            .unwrap();

        assert_eq!(
            first
                .lookup(Some("/tenant?token=secret"), &slash_field)
                .await
                .unwrap()
                .0
                .url(),
            Url::parse("https://one.example").unwrap()
        );
        assert_eq!(
            first
                .lookup(Some("/tenant?token=secret"), &split_fields)
                .await
                .unwrap()
                .0
                .url(),
            Url::parse("https://two.example").unwrap()
        );
        assert_eq!(
            first
                .lookup(Some("/tenant?token=secret"), &binary_child)
                .await
                .unwrap()
                .0
                .namespace(),
            &binary_prefix
        );
        let serialized = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
        assert!(!serialized.contains("tenant"));
        assert!(!serialized.contains("token"));

        drop((slash_registration, split_registration, prefix_registration));
        first.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_utf8_file_migrates_to_versioned_binary_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("coordinator.json");
        std::fs::write(
            &path,
            br#"{"namespaces":{"tenant":{"/alpha/beta":"https://legacy.example/"}}}"#,
        )
        .unwrap();
        let coordinator = FileCoordinator::with_limits(
            &path,
            relay_url(),
            FileCoordinatorLimits {
                max_entries: 4,
                max_bytes: 4_096,
            },
        )
        .unwrap();
        let legacy = namespace(&[b"alpha", b"beta"]);
        assert_eq!(
            coordinator
                .lookup(Some("tenant"), &legacy)
                .await
                .unwrap()
                .0
                .url(),
            Url::parse("https://legacy.example/").unwrap()
        );

        let new_registration = coordinator
            .register_namespace(Some("tenant"), &namespace(&[b"new"]))
            .await
            .unwrap();
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(data["version"], COORDINATOR_DATA_VERSION);
        let serialized = data.to_string();
        assert!(serialized.contains(NAMESPACE_KEY_PREFIX));
        assert!(!serialized.contains("/alpha/beta"));
        assert!(!serialized.contains("\"tenant\""));
        drop(new_registration);
        coordinator.shutdown().await.unwrap();
    }
}
