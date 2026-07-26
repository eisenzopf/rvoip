// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! API-based coordinator for multi-relay deployments.
//!
//! This coordinator uses the moq-api HTTP server as a centralized registry
//! to coordinate namespace registration across multiple relay instances.
//! It provides:
//!
//! - HTTP-based namespace lookups via moq-api
//! - Automatic TTL refresh to maintain registrations
//! - High availability when using the moq-api server

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use async_trait::async_trait;
use moq_api::{Client, Origin};
use moq_native_ietf::quic;
use moq_transport::coding::TrackNamespace;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use url::Url;

use moq_relay_ietf::{
    Coordinator, CoordinatorError, CoordinatorResult, NamespaceOrigin, NamespaceRegistration,
};

/// Default TTL for namespace registrations (in seconds)
/// moq-api server uses 600 seconds (10 minutes) TTL
const DEFAULT_REGISTRATION_TTL_SECS: u64 = 600;
const DEFAULT_MAX_BACKGROUND_TASKS: usize = 4_096;
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_CLEANUP_TIMEOUT_MS: u64 = 2_000;

/// Configuration for the API coordinator
#[derive(Debug, Clone)]
pub struct ApiCoordinatorConfig {
    /// URL of the moq-api server (e.g., "http://localhost:8080")
    pub api_url: Url,
    /// URL of this relay (advertised to other relays)
    pub relay_url: Url,
    /// TTL for namespace registrations in seconds
    pub registration_ttl_secs: u64,
    /// Interval for refreshing registrations (should be less than TTL)
    pub refresh_interval_secs: u64,
    /// Maximum retained namespace refresh/cleanup tasks.
    pub max_background_tasks: usize,
    /// Maximum time for one registry API request.
    pub request_timeout: Duration,
    /// Maximum time to wait for each API unregister and coordinator shutdown.
    pub cleanup_timeout: Duration,
}

impl ApiCoordinatorConfig {
    /// Create a new configuration with default TTL values
    pub fn new(api_url: Url, relay_url: Url) -> Self {
        Self {
            api_url,
            relay_url,
            registration_ttl_secs: DEFAULT_REGISTRATION_TTL_SECS,
            // Refresh at half the TTL to ensure we don't expire
            refresh_interval_secs: DEFAULT_REGISTRATION_TTL_SECS / 2,
            max_background_tasks: DEFAULT_MAX_BACKGROUND_TASKS,
            request_timeout: Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            cleanup_timeout: Duration::from_millis(DEFAULT_CLEANUP_TIMEOUT_MS),
        }
    }

    /// Set custom TTL for registrations
    pub fn with_ttl(mut self, ttl_secs: u64) -> Self {
        self.registration_ttl_secs = ttl_secs;
        self.refresh_interval_secs = ttl_secs / 2;
        self
    }

    pub fn with_background_task_limit(mut self, limit: usize) -> Self {
        self.max_background_tasks = limit;
        self
    }

    pub fn with_cleanup_timeout(mut self, timeout: Duration) -> Self {
        self.cleanup_timeout = timeout;
        self
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.registration_ttl_secs >= 2,
            "API registration TTL must be at least two seconds"
        );
        anyhow::ensure!(
            self.refresh_interval_secs > 0,
            "API refresh interval must be positive"
        );
        anyhow::ensure!(
            self.refresh_interval_secs < self.registration_ttl_secs,
            "API refresh interval must be less than its TTL"
        );
        anyhow::ensure!(
            self.max_background_tasks > 0,
            "API background task limit must be positive"
        );
        anyhow::ensure!(
            self.max_background_tasks <= Semaphore::MAX_PERMITS,
            "API background task limit exceeds the semaphore maximum"
        );
        anyhow::ensure!(
            !self.request_timeout.is_zero(),
            "API request timeout must be positive"
        );
        anyhow::ensure!(
            !self.cleanup_timeout.is_zero(),
            "API cleanup timeout must be positive"
        );
        Ok(())
    }
}

/// Handle that unregisters a namespace when dropped and manages TTL refresh
struct NamespaceUnregisterHandle {
    /// Channel to signal the supervised refresh/cleanup task to stop.
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for NamespaceUnregisterHandle {
    fn drop(&mut self) {
        // Signal the refresh task to stop
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Async helper for unregistering a namespace
async fn unregister_namespace_async(client: &Client, namespace_key: &str) -> Result<()> {
    tracing::debug!("unregistering namespace from API");

    client
        .delete_origin(namespace_key)
        .await
        .context("failed to delete namespace from API")?;

    Ok(())
}

/// A coordinator that uses moq-api for state storage.
///
/// Multiple relay instances can connect to the same moq-api server to
/// coordinate namespace registration and discovery. Features:
///
/// - HTTP-based registration and lookup
/// - TTL-based automatic expiration of stale registrations
/// - Background refresh tasks to maintain registrations
///
/// # Scope handling
///
/// Registry keys encode the scope and namespace into a single collision-free
/// string. Namespace tuple fields are hex-encoded to handle arbitrary bytes
/// (MoQT namespaces are tuples of byte arrays, not strings). See
/// [`ApiCoordinator::registry_key()`] for format details.
pub struct ApiCoordinator {
    /// moq-api client
    client: Client,
    /// Configuration
    config: ApiCoordinatorConfig,
    task_capacity: Arc<Semaphore>,
    tasks: TaskTracker,
    shutdown: CancellationToken,
}

impl ApiCoordinator {
    /// Build the moq-api registry key for a namespace, scoped if applicable.
    ///
    /// The key unambiguously encodes `(scope, namespace)` into a single string
    /// that can be used as an opaque key in the moq-api HTTP registry.
    ///
    /// ## Format
    ///
    /// Scope presence, scope bytes, tuple count, and every tuple-field length
    /// are encoded unambiguously and SHA-256 hashed into one bounded URL-safe key.
    ///
    /// ## Why this is collision-free
    ///
    /// Length-prefixing preserves arbitrary bytes and field boundaries, while
    /// hashing prevents query-bearing scope identities from reaching HTTP URLs.
    fn registry_key(scope: Option<&str>, namespace: &TrackNamespace) -> String {
        let mut encoded = Vec::new();
        match scope {
            Some(scope) => {
                encoded.push(1);
                encoded.extend_from_slice(&(scope.len() as u64).to_be_bytes());
                encoded.extend_from_slice(scope.as_bytes());
            }
            None => encoded.push(0),
        }
        encoded.extend_from_slice(&(namespace.fields.len() as u32).to_be_bytes());
        for field in &namespace.fields {
            encoded.extend_from_slice(&(field.value.len() as u64).to_be_bytes());
            encoded.extend_from_slice(&field.value);
        }
        let digest = ring::digest::digest(&ring::digest::SHA256, &encoded);
        format!("v1-{}", hex::encode(digest.as_ref()))
    }

    /// Create a new API-based coordinator.
    ///
    /// # Arguments
    /// * `config` - Configuration for the API coordinator
    ///
    /// # Returns
    /// A new `ApiCoordinator` instance
    pub fn new(config: ApiCoordinatorConfig) -> Result<Self> {
        config.validate()?;
        let client = Client::new(config.api_url.clone());
        let task_capacity = Arc::new(Semaphore::new(config.max_background_tasks));

        Ok(Self {
            client,
            config,
            task_capacity,
            tasks: TaskTracker::new(),
            shutdown: CancellationToken::new(),
        })
    }

    /// Start a background task to refresh namespace registration
    fn start_refresh_task(
        &self,
        namespace_key: String,
        mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
        _permit: OwnedSemaphorePermit,
    ) {
        let client = self.client.clone();
        let relay_url = self.config.relay_url.clone();
        let refresh_interval = Duration::from_secs(self.config.refresh_interval_secs);
        let cleanup_timeout = self.config.cleanup_timeout;
        let request_timeout = self.config.request_timeout;
        let shutdown = self.shutdown.clone();
        self.tasks.spawn(async move {
            let _permit = _permit;
            let _task_guard = moq_relay_ietf::metrics::GaugeGuard::new(
                "moq_relay_coordinator_background_tasks",
            );
            let mut interval = tokio::time::interval(refresh_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let origin = Origin { url: relay_url.clone() };

                        match tokio::time::timeout(request_timeout, client.patch_origin(&namespace_key, origin)).await {
                            Ok(Ok(())) => {
                                tracing::trace!("refreshed namespace registration");
                            }
                            Ok(Err(err)) => {
                                tracing::warn!(error = %err, "failed to refresh namespace registration");
                            }
                            Err(_) => tracing::warn!("namespace registration refresh timed out"),
                        }
                    }
                    _ = &mut shutdown_rx => {
                        tracing::debug!("namespace refresh task shutting down");
                        break;
                    }
                    _ = shutdown.cancelled() => break,
                }
            }

            if let Err(error) = tokio::time::timeout(
                cleanup_timeout,
                unregister_namespace_async(&client, &namespace_key),
            )
            .await
            .context("namespace API unregister timed out")
            .and_then(|result| result)
            {
                tracing::warn!(error = %error, "failed to unregister namespace during supervised cleanup");
            }
        });
    }

    fn try_task_permit(&self) -> CoordinatorResult<OwnedSemaphorePermit> {
        if self.shutdown.is_cancelled() {
            return Err(CoordinatorError::Other(anyhow::anyhow!(
                "API coordinator is shutting down"
            )));
        }
        self.task_capacity.clone().try_acquire_owned().map_err(|_| {
            metrics::counter!(
                "moq_relay_coordinator_capacity_rejections_total",
                "kind" => "api_task"
            )
            .increment(1);
            CoordinatorError::CapacityExhausted {
                resource: "api_background_task",
            }
        })
    }

    #[cfg(test)]
    fn snapshot(&self) -> ApiCoordinatorSnapshot {
        ApiCoordinatorSnapshot {
            active_background_tasks: self
                .config
                .max_background_tasks
                .saturating_sub(self.task_capacity.available_permits()),
            supervised_tasks: self.tasks.len(),
            max_background_tasks: self.config.max_background_tasks,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ApiCoordinatorSnapshot {
    active_background_tasks: usize,
    supervised_tasks: usize,
    max_background_tasks: usize,
}

#[async_trait]
impl Coordinator for ApiCoordinator {
    async fn register_namespace(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<NamespaceRegistration> {
        let namespace_str = Self::registry_key(scope, namespace);
        let origin = Origin {
            url: self.config.relay_url.clone(),
        };
        let task_permit = self.try_task_permit()?;

        tracing::info!(
            scoped = scope.is_some(),
            namespace_fields = namespace.fields.len(),
            relay_url = %moq_relay_ietf::redact_url_for_logging(&self.config.relay_url),
            "registering namespace in API"
        );

        // Register the namespace with the API
        tokio::time::timeout(
            self.config.request_timeout,
            self.client.set_origin(&namespace_str, origin),
        )
        .await
        .context("namespace API registration timed out")
        .map_err(CoordinatorError::Other)?
        .context("failed to register namespace in API")
        .map_err(CoordinatorError::Other)?;

        // Create shutdown channel for the refresh task
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        // Start background refresh task
        self.start_refresh_task(namespace_str.clone(), shutdown_rx, task_permit);

        let handle = NamespaceUnregisterHandle {
            shutdown_tx: Some(shutdown_tx),
        };

        Ok(NamespaceRegistration::new(handle))
    }

    async fn unregister_namespace(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<()> {
        let namespace_str = Self::registry_key(scope, namespace);
        tracing::info!(
            scoped = scope.is_some(),
            namespace_fields = namespace.fields.len(),
            "unregistering namespace from API"
        );

        tokio::time::timeout(
            self.config.request_timeout,
            self.client.delete_origin(&namespace_str),
        )
        .await
        .context("namespace API unregister timed out")
        .map_err(CoordinatorError::Other)?
        .context("failed to unregister namespace from API")
        .map_err(CoordinatorError::Other)?;

        Ok(())
    }

    async fn lookup(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<(NamespaceOrigin, Option<quic::Client>)> {
        let namespace_str = Self::registry_key(scope, namespace);
        tracing::debug!(
            scoped = scope.is_some(),
            namespace_fields = namespace.fields.len(),
            "looking up namespace in API"
        );

        // Query the API for the namespace
        let result = tokio::time::timeout(
            self.config.request_timeout,
            self.client.get_origin(&namespace_str),
        )
        .await
        .context("namespace API lookup timed out")
        .map_err(CoordinatorError::Other)?
        .context("failed to lookup namespace in API")
        .map_err(CoordinatorError::Other)?;

        match result {
            Some(origin) => {
                tracing::debug!(origin_url = %moq_relay_ietf::redact_url_for_logging(&origin.url), "found namespace");
                Ok((
                    NamespaceOrigin::new(namespace.clone(), origin.url, None),
                    None,
                ))
            }
            None => {
                tracing::debug!("namespace not found");
                Err(CoordinatorError::NamespaceNotFound)
            }
        }
    }

    async fn shutdown(&self) -> CoordinatorResult<()> {
        tracing::info!("shutting down API coordinator");
        self.shutdown.cancel();
        self.tasks.close();
        let shutdown_timeout = self
            .config
            .request_timeout
            .saturating_add(self.config.cleanup_timeout);
        tokio::time::timeout(shutdown_timeout, self.tasks.wait())
            .await
            .context("timed out waiting for API coordinator tasks")
            .map_err(CoordinatorError::Other)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moq_transport::coding::TupleField;

    #[test]
    fn registry_keys_are_scope_safe_and_tuple_collision_free() {
        let one_field = TrackNamespace::try_from(vec![TupleField {
            value: b"a/b".to_vec(),
        }])
        .unwrap();
        let two_fields = TrackNamespace::try_from(vec![
            TupleField {
                value: b"a".to_vec(),
            },
            TupleField {
                value: b"b".to_vec(),
            },
        ])
        .unwrap();
        let scope = "/tenant?token=secret";
        let one = ApiCoordinator::registry_key(Some(scope), &one_field);
        let two = ApiCoordinator::registry_key(Some(scope), &two_fields);
        assert_ne!(one, two);
        assert!(!one.contains("tenant"));
        assert!(!one.contains("token"));
        assert!(!one.contains('?'));
        assert_ne!(one, ApiCoordinator::registry_key(None, &one_field));
    }

    #[test]
    fn test_config_new() {
        let api_url = Url::parse("http://localhost:8080").unwrap();
        let relay_url = Url::parse("https://relay.example.com").unwrap();

        let config = ApiCoordinatorConfig::new(api_url.clone(), relay_url.clone());

        assert_eq!(config.api_url, api_url);
        assert_eq!(config.relay_url, relay_url);
        assert_eq!(config.registration_ttl_secs, DEFAULT_REGISTRATION_TTL_SECS);
        assert_eq!(
            config.refresh_interval_secs,
            DEFAULT_REGISTRATION_TTL_SECS / 2
        );
        assert_eq!(config.max_background_tasks, DEFAULT_MAX_BACKGROUND_TASKS);
        assert_eq!(
            config.request_timeout,
            Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS)
        );
        assert_eq!(
            config.cleanup_timeout,
            Duration::from_millis(DEFAULT_CLEANUP_TIMEOUT_MS)
        );
    }

    #[test]
    fn test_config_with_ttl() {
        let api_url = Url::parse("http://localhost:8080").unwrap();
        let relay_url = Url::parse("https://relay.example.com").unwrap();

        let config = ApiCoordinatorConfig::new(api_url, relay_url).with_ttl(120);

        assert_eq!(config.registration_ttl_secs, 120);
        assert_eq!(config.refresh_interval_secs, 60);
    }

    #[test]
    fn invalid_limits_are_rejected() {
        let api_url = Url::parse("http://localhost:8080").unwrap();
        let relay_url = Url::parse("https://relay.example.com").unwrap();
        let config = ApiCoordinatorConfig::new(api_url, relay_url).with_background_task_limit(0);
        assert!(ApiCoordinator::new(config).is_err());
        let config = ApiCoordinatorConfig::new(
            Url::parse("http://localhost:8080").unwrap(),
            Url::parse("https://relay.example.com").unwrap(),
        )
        .with_background_task_limit(Semaphore::MAX_PERMITS.saturating_add(1));
        assert!(ApiCoordinator::new(config).is_err());
        let config = ApiCoordinatorConfig::new(
            Url::parse("http://localhost:8080").unwrap(),
            Url::parse("https://relay.example.com").unwrap(),
        )
        .with_request_timeout(Duration::ZERO);
        assert!(ApiCoordinator::new(config).is_err());
    }

    #[test]
    fn background_task_permits_are_fail_fast_and_reusable() {
        let api_url = Url::parse("http://localhost:8080").unwrap();
        let relay_url = Url::parse("https://relay.example.com").unwrap();
        let coordinator = ApiCoordinator::new(
            ApiCoordinatorConfig::new(api_url, relay_url).with_background_task_limit(1),
        )
        .unwrap();
        let permit = coordinator.try_task_permit().unwrap();
        assert_eq!(coordinator.snapshot().active_background_tasks, 1);
        assert!(coordinator.try_task_permit().is_err());
        drop(permit);
        assert!(coordinator.try_task_permit().is_ok());
    }
}
