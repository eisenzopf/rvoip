// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{AdmissionDecision, AdmissionPrincipal, AuthenticationMethod};

/// Long-lived relay resources with independent hierarchical capacity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RelayResource {
    PublishNamespace,
    PublishTrack,
    Subscribe,
    TrackStatus,
    Fetch,
}

impl RelayResource {
    pub const ALL: [Self; 5] = [
        Self::PublishNamespace,
        Self::PublishTrack,
        Self::Subscribe,
        Self::TrackStatus,
        Self::Fetch,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::PublishNamespace => "publish_namespace",
            Self::PublishTrack => "publish_track",
            Self::Subscribe => "subscribe",
            Self::TrackStatus => "track_status",
            Self::Fetch => "fetch",
        }
    }
}

/// Limits at one level of the process → principal → scope hierarchy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelayCapacityLimitSet {
    pub total: usize,
    pub publish_namespaces: usize,
    pub publish_tracks: usize,
    pub subscribes: usize,
    pub track_statuses: usize,
    pub fetches: usize,
}

impl RelayCapacityLimitSet {
    fn resource(self, resource: RelayResource) -> usize {
        match resource {
            RelayResource::PublishNamespace => self.publish_namespaces,
            RelayResource::PublishTrack => self.publish_tracks,
            RelayResource::Subscribe => self.subscribes,
            RelayResource::TrackStatus => self.track_statuses,
            RelayResource::Fetch => self.fetches,
        }
    }

    fn validate(self, level: RelayCapacityLevel) -> Result<(), RelayCapacityError> {
        if self.total == 0 {
            return Err(RelayCapacityError::ZeroLimit {
                level,
                field: "total",
            });
        }
        for resource in RelayResource::ALL {
            if self.resource(resource) == 0 {
                return Err(RelayCapacityError::ZeroLimit {
                    level,
                    field: resource.label(),
                });
            }
        }
        for (field, value) in [
            ("total", self.total),
            ("publish_namespace", self.publish_namespaces),
            ("publish_track", self.publish_tracks),
            ("subscribe", self.subscribes),
            ("track_status", self.track_statuses),
            ("fetch", self.fetches),
        ] {
            if value > Semaphore::MAX_PERMITS {
                return Err(RelayCapacityError::LimitTooLarge { level, field });
            }
        }
        Ok(())
    }
}

/// Configurable hierarchical relay limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayCapacityLimits {
    pub process: RelayCapacityLimitSet,
    pub per_principal: RelayCapacityLimitSet,
    pub per_scope: RelayCapacityLimitSet,
}

impl RelayCapacityLimits {
    pub fn validate(&self) -> Result<(), RelayCapacityError> {
        self.process.validate(RelayCapacityLevel::Process)?;
        self.per_principal.validate(RelayCapacityLevel::Principal)?;
        self.per_scope.validate(RelayCapacityLevel::Scope)?;
        Ok(())
    }
}

impl Default for RelayCapacityLimits {
    fn default() -> Self {
        Self {
            process: RelayCapacityLimitSet {
                total: 20_000,
                publish_namespaces: 4_096,
                publish_tracks: 8_192,
                subscribes: 10_000,
                track_statuses: 2_048,
                fetches: 4_096,
            },
            per_principal: RelayCapacityLimitSet {
                total: 2_048,
                publish_namespaces: 256,
                publish_tracks: 1_024,
                subscribes: 1_024,
                track_statuses: 256,
                fetches: 512,
            },
            per_scope: RelayCapacityLimitSet {
                total: 8_192,
                publish_namespaces: 2_048,
                publish_tracks: 4_096,
                subscribes: 4_096,
                track_statuses: 1_024,
                fetches: 2_048,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayCapacityLevel {
    Process,
    Principal,
    Scope,
}

impl RelayCapacityLevel {
    const fn label(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Principal => "principal",
            Self::Scope => "scope",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RelayCapacityError {
    #[error("relay capacity limit {level:?}.{field} must be greater than zero")]
    ZeroLimit {
        level: RelayCapacityLevel,
        field: &'static str,
    },
    #[error("relay capacity limit {level:?}.{field} exceeds the semaphore maximum")]
    LimitTooLarge {
        level: RelayCapacityLevel,
        field: &'static str,
    },
    #[error("{level:?} capacity exhausted for {resource:?}")]
    Exhausted {
        level: RelayCapacityLevel,
        resource: RelayResource,
    },
}

/// Authenticated identity retained by media/request handlers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayIdentity {
    principal: AdmissionPrincipal,
    resolved_scope: Option<String>,
}

impl RelayIdentity {
    pub fn admitted(decision: &AdmissionDecision, resolved_scope: Option<String>) -> Self {
        Self {
            principal: decision.principal.clone(),
            resolved_scope,
        }
    }

    pub fn new(principal: AdmissionPrincipal, resolved_scope: Option<String>) -> Self {
        Self {
            principal,
            resolved_scope,
        }
    }

    pub fn operator(resolved_scope: Option<String>) -> Self {
        Self::new(
            AdmissionPrincipal::new("relay-operator", AuthenticationMethod::Development)
                .expect("static relay operator principal is valid"),
            resolved_scope,
        )
    }

    pub fn principal(&self) -> &AdmissionPrincipal {
        &self.principal
    }

    pub fn scope(&self) -> Option<&str> {
        self.resolved_scope.as_deref()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PrincipalKey {
    method: AuthenticationMethod,
    subject: String,
}

impl From<&AdmissionPrincipal> for PrincipalKey {
    fn from(principal: &AdmissionPrincipal) -> Self {
        Self {
            method: principal.method,
            subject: principal.subject().to_string(),
        }
    }
}

#[derive(Debug)]
struct ResourcePools {
    publish_namespace: Arc<Semaphore>,
    publish_track: Arc<Semaphore>,
    subscribe: Arc<Semaphore>,
    track_status: Arc<Semaphore>,
    fetch: Arc<Semaphore>,
}

impl ResourcePools {
    fn new(limits: RelayCapacityLimitSet) -> Self {
        Self {
            publish_namespace: Arc::new(Semaphore::new(limits.publish_namespaces)),
            publish_track: Arc::new(Semaphore::new(limits.publish_tracks)),
            subscribe: Arc::new(Semaphore::new(limits.subscribes)),
            track_status: Arc::new(Semaphore::new(limits.track_statuses)),
            fetch: Arc::new(Semaphore::new(limits.fetches)),
        }
    }

    fn get(&self, resource: RelayResource) -> Arc<Semaphore> {
        match resource {
            RelayResource::PublishNamespace => self.publish_namespace.clone(),
            RelayResource::PublishTrack => self.publish_track.clone(),
            RelayResource::Subscribe => self.subscribe.clone(),
            RelayResource::TrackStatus => self.track_status.clone(),
            RelayResource::Fetch => self.fetch.clone(),
        }
    }
}

#[derive(Debug)]
struct CapacityPool {
    total: Arc<Semaphore>,
    resources: ResourcePools,
    active: AtomicUsize,
}

impl CapacityPool {
    fn new(limits: RelayCapacityLimitSet) -> Self {
        Self {
            total: Arc::new(Semaphore::new(limits.total)),
            resources: ResourcePools::new(limits),
            active: AtomicUsize::new(0),
        }
    }
}

#[derive(Default, Debug)]
struct CapacityState {
    principals: HashMap<PrincipalKey, Arc<CapacityPool>>,
    scopes: HashMap<String, Arc<CapacityPool>>,
}

#[derive(Debug)]
struct RelayCapacityInner {
    process: Arc<CapacityPool>,
    state: Mutex<CapacityState>,
    limits: RelayCapacityLimits,
}

/// Shared hierarchical capacity manager for one relay process.
#[derive(Clone, Debug)]
pub struct RelayCapacity(Arc<RelayCapacityInner>);

impl RelayCapacity {
    pub fn new(limits: RelayCapacityLimits) -> Result<Self, RelayCapacityError> {
        limits.validate()?;
        Ok(Self(Arc::new(RelayCapacityInner {
            process: Arc::new(CapacityPool::new(limits.process)),
            state: Mutex::new(CapacityState::default()),
            limits,
        })))
    }

    pub fn limits(&self) -> &RelayCapacityLimits {
        &self.0.limits
    }

    /// Acquire process, principal, and scope total/class permits without
    /// waiting. Dynamic identity buckets are created and removed under one
    /// mutex so an empty-bucket race cannot bypass a configured limit.
    pub fn try_acquire(
        &self,
        identity: &RelayIdentity,
        resource: RelayResource,
    ) -> Result<RelayCapacityLease, RelayCapacityError> {
        let (process_total, process_resource) = match (|| {
            let total = permit(
                self.0.process.total.clone(),
                RelayCapacityLevel::Process,
                resource,
            )?;
            let resource_permit = permit(
                self.0.process.resources.get(resource),
                RelayCapacityLevel::Process,
                resource,
            )?;
            Ok::<_, RelayCapacityError>((total, resource_permit))
        })() {
            Ok(permits) => permits,
            Err(error) => {
                record_rejection(error);
                return Err(error);
            }
        };

        let principal_key = PrincipalKey::from(identity.principal());
        let scope_key = identity.scope().unwrap_or("").to_string();
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let principal = state
            .principals
            .entry(principal_key.clone())
            .or_insert_with(|| Arc::new(CapacityPool::new(self.0.limits.per_principal)))
            .clone();
        let scope = state
            .scopes
            .entry(scope_key.clone())
            .or_insert_with(|| Arc::new(CapacityPool::new(self.0.limits.per_scope)))
            .clone();

        let acquired = (|| {
            let principal_total = permit(
                principal.total.clone(),
                RelayCapacityLevel::Principal,
                resource,
            )?;
            let principal_resource = permit(
                principal.resources.get(resource),
                RelayCapacityLevel::Principal,
                resource,
            )?;
            let scope_total = permit(scope.total.clone(), RelayCapacityLevel::Scope, resource)?;
            let scope_resource = permit(
                scope.resources.get(resource),
                RelayCapacityLevel::Scope,
                resource,
            )?;
            Ok::<_, RelayCapacityError>((
                principal_total,
                principal_resource,
                scope_total,
                scope_resource,
            ))
        })();

        let (principal_total, principal_resource, scope_total, scope_resource) = match acquired {
            Ok(permits) => permits,
            Err(error) => {
                remove_empty_bucket(&mut state.principals, &principal_key, &principal);
                remove_empty_bucket(&mut state.scopes, &scope_key, &scope);
                record_rejection(error);
                return Err(error);
            }
        };
        principal.active.fetch_add(1, Ordering::Relaxed);
        scope.active.fetch_add(1, Ordering::Relaxed);
        self.0.process.active.fetch_add(1, Ordering::Relaxed);
        drop(state);

        for level in [
            RelayCapacityLevel::Process,
            RelayCapacityLevel::Principal,
            RelayCapacityLevel::Scope,
        ] {
            metrics::gauge!(
                "moq_relay_capacity_active",
                "level" => level.label(),
                "resource" => resource.label()
            )
            .increment(1.0);
        }

        Ok(RelayCapacityLease {
            capacity: self.clone(),
            principal_key,
            scope_key,
            principal,
            scope,
            resource,
            _process_total: process_total,
            _process_resource: process_resource,
            _principal_total: principal_total,
            _principal_resource: principal_resource,
            _scope_total: scope_total,
            _scope_resource: scope_resource,
        })
    }

    pub fn snapshot(&self) -> RelayCapacitySnapshot {
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        RelayCapacitySnapshot {
            active: self.0.process.active.load(Ordering::Relaxed),
            principal_buckets: state.principals.len(),
            scope_buckets: state.scopes.len(),
        }
    }
}

impl Default for RelayCapacity {
    fn default() -> Self {
        Self::new(RelayCapacityLimits::default()).expect("default relay capacity limits are valid")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelayCapacitySnapshot {
    pub active: usize,
    pub principal_buckets: usize,
    pub scope_buckets: usize,
}

#[must_use = "relay capacity is released when the lease is dropped"]
pub struct RelayCapacityLease {
    capacity: RelayCapacity,
    principal_key: PrincipalKey,
    scope_key: String,
    principal: Arc<CapacityPool>,
    scope: Arc<CapacityPool>,
    resource: RelayResource,
    _process_total: OwnedSemaphorePermit,
    _process_resource: OwnedSemaphorePermit,
    _principal_total: OwnedSemaphorePermit,
    _principal_resource: OwnedSemaphorePermit,
    _scope_total: OwnedSemaphorePermit,
    _scope_resource: OwnedSemaphorePermit,
}

impl Drop for RelayCapacityLease {
    fn drop(&mut self) {
        self.capacity
            .0
            .process
            .active
            .fetch_sub(1, Ordering::Relaxed);
        self.principal.active.fetch_sub(1, Ordering::Relaxed);
        self.scope.active.fetch_sub(1, Ordering::Relaxed);

        for level in [
            RelayCapacityLevel::Process,
            RelayCapacityLevel::Principal,
            RelayCapacityLevel::Scope,
        ] {
            metrics::gauge!(
                "moq_relay_capacity_active",
                "level" => level.label(),
                "resource" => self.resource.label()
            )
            .decrement(1.0);
        }

        let mut state = self
            .capacity
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        remove_empty_bucket(&mut state.principals, &self.principal_key, &self.principal);
        remove_empty_bucket(&mut state.scopes, &self.scope_key, &self.scope);
    }
}

fn permit(
    semaphore: Arc<Semaphore>,
    level: RelayCapacityLevel,
    resource: RelayResource,
) -> Result<OwnedSemaphorePermit, RelayCapacityError> {
    semaphore
        .try_acquire_owned()
        .map_err(|_| RelayCapacityError::Exhausted { level, resource })
}

fn remove_empty_bucket<K: Eq + std::hash::Hash>(
    buckets: &mut HashMap<K, Arc<CapacityPool>>,
    key: &K,
    bucket: &Arc<CapacityPool>,
) {
    if bucket.active.load(Ordering::Relaxed) == 0
        && buckets
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, bucket))
    {
        buckets.remove(key);
    }
}

fn record_rejection(error: RelayCapacityError) {
    if let RelayCapacityError::Exhausted { level, resource } = error {
        metrics::counter!(
            "moq_relay_capacity_rejections_total",
            "level" => level.label(),
            "resource" => resource.label()
        )
        .increment(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdmissionClaims, AuthenticationMethod};

    fn set(total: usize, class: usize) -> RelayCapacityLimitSet {
        RelayCapacityLimitSet {
            total,
            publish_namespaces: class,
            publish_tracks: class,
            subscribes: class,
            track_statuses: class,
            fetches: class,
        }
    }

    fn capacity(
        process: RelayCapacityLimitSet,
        principal: RelayCapacityLimitSet,
        scope: RelayCapacityLimitSet,
    ) -> RelayCapacity {
        RelayCapacity::new(RelayCapacityLimits {
            process,
            per_principal: principal,
            per_scope: scope,
        })
        .unwrap()
    }

    fn identity(subject: &str, scope: &str) -> RelayIdentity {
        let decision = AdmissionDecision::new(
            AdmissionPrincipal::new(subject, AuthenticationMethod::MutualTls).unwrap(),
            AdmissionClaims {
                scope: Some(scope.to_string()),
                publish: true,
                subscribe: false,
                expires_at_unix_seconds: None,
                token_id: None,
            },
        )
        .unwrap();
        RelayIdentity::admitted(&decision, Some(scope.to_string()))
    }

    #[test]
    fn rejects_zero_limits() {
        let mut limits = RelayCapacityLimits::default();
        limits.per_scope.track_statuses = 0;
        assert!(matches!(
            RelayCapacity::new(limits),
            Err(RelayCapacityError::ZeroLimit {
                level: RelayCapacityLevel::Scope,
                field: "track_status"
            })
        ));
    }

    #[test]
    fn rejects_limits_above_the_semaphore_maximum() {
        let mut limits = RelayCapacityLimits::default();
        limits.process.fetches = Semaphore::MAX_PERMITS.saturating_add(1);
        assert!(matches!(
            RelayCapacity::new(limits),
            Err(RelayCapacityError::LimitTooLarge {
                level: RelayCapacityLevel::Process,
                field: "fetch"
            })
        ));
    }

    #[test]
    fn every_resource_is_fail_fast_and_reusable_at_every_level() {
        for resource in RelayResource::ALL {
            for (process, principal, scope, expected) in [
                (set(8, 1), set(8, 8), set(8, 8), RelayCapacityLevel::Process),
                (
                    set(8, 8),
                    set(8, 1),
                    set(8, 8),
                    RelayCapacityLevel::Principal,
                ),
                (set(8, 8), set(8, 8), set(8, 1), RelayCapacityLevel::Scope),
            ] {
                let capacity = capacity(process, principal, scope);
                let identity = identity("alice", "scope-a");
                let lease = capacity.try_acquire(&identity, resource).unwrap();
                assert!(matches!(
                    capacity.try_acquire(&identity, resource),
                    Err(RelayCapacityError::Exhausted { level, .. }) if level == expected
                ));
                drop(lease);
                assert!(capacity.try_acquire(&identity, resource).is_ok());
            }
        }
    }

    #[test]
    fn mixed_resource_total_caps_apply_at_every_level() {
        for (process, principal, scope, expected) in [
            (set(1, 8), set(8, 8), set(8, 8), RelayCapacityLevel::Process),
            (
                set(8, 8),
                set(1, 8),
                set(8, 8),
                RelayCapacityLevel::Principal,
            ),
            (set(8, 8), set(8, 8), set(1, 8), RelayCapacityLevel::Scope),
        ] {
            let capacity = capacity(process, principal, scope);
            let identity = identity("alice", "scope-a");
            let lease = capacity
                .try_acquire(&identity, RelayResource::PublishNamespace)
                .unwrap();
            assert!(matches!(
                capacity.try_acquire(&identity, RelayResource::Fetch),
                Err(RelayCapacityError::Exhausted { level, .. }) if level == expected
            ));
            drop(lease);
            assert!(capacity
                .try_acquire(&identity, RelayResource::Fetch)
                .is_ok());
        }
    }

    #[test]
    fn principal_fairness_and_scope_isolation_are_independent() {
        let capacity = capacity(set(8, 8), set(1, 1), set(2, 2));
        let alice = identity("alice", "scope-a");
        let bob = identity("bob", "scope-a");
        let carol = identity("carol", "scope-b");
        let _alice = capacity
            .try_acquire(&alice, RelayResource::Subscribe)
            .unwrap();
        assert!(matches!(
            capacity.try_acquire(&alice, RelayResource::Subscribe),
            Err(RelayCapacityError::Exhausted {
                level: RelayCapacityLevel::Principal,
                ..
            })
        ));
        let _bob = capacity
            .try_acquire(&bob, RelayResource::Subscribe)
            .unwrap();
        assert!(capacity
            .try_acquire(&carol, RelayResource::Subscribe)
            .is_ok());
    }

    #[test]
    fn shared_scope_cap_applies_across_principals() {
        let capacity = capacity(set(8, 8), set(8, 8), set(1, 1));
        let alice = identity("alice", "scope-a");
        let bob = identity("bob", "scope-a");
        let _lease = capacity
            .try_acquire(&alice, RelayResource::PublishTrack)
            .unwrap();
        assert!(matches!(
            capacity.try_acquire(&bob, RelayResource::PublishTrack),
            Err(RelayCapacityError::Exhausted {
                level: RelayCapacityLevel::Scope,
                ..
            })
        ));
    }

    #[test]
    fn process_total_rolls_back_dynamic_bucket_acquisition() {
        let capacity = capacity(set(1, 8), set(8, 8), set(8, 8));
        let alice = identity("alice", "scope-a");
        let bob = identity("bob", "scope-b");
        let lease = capacity
            .try_acquire(&alice, RelayResource::PublishNamespace)
            .unwrap();
        assert!(matches!(
            capacity.try_acquire(&bob, RelayResource::Subscribe),
            Err(RelayCapacityError::Exhausted {
                level: RelayCapacityLevel::Process,
                ..
            })
        ));
        assert_eq!(capacity.snapshot().principal_buckets, 1);
        drop(lease);
        assert!(capacity.try_acquire(&bob, RelayResource::Subscribe).is_ok());
    }

    #[test]
    fn empty_identity_buckets_are_removed_after_release_and_failure() {
        let capacity = capacity(set(4, 4), set(1, 1), set(1, 1));
        let alice = identity("alice", "scope-a");
        let bob = identity("bob", "scope-a");
        let lease = capacity
            .try_acquire(&alice, RelayResource::TrackStatus)
            .unwrap();
        assert!(capacity
            .try_acquire(&bob, RelayResource::TrackStatus)
            .is_err());
        assert_eq!(capacity.snapshot().principal_buckets, 1);
        assert_eq!(capacity.snapshot().scope_buckets, 1);
        drop(lease);
        assert_eq!(
            capacity.snapshot(),
            RelayCapacitySnapshot {
                active: 0,
                principal_buckets: 0,
                scope_buckets: 0,
            }
        );
    }

    #[test]
    fn panic_unwind_releases_hierarchy() {
        let capacity = capacity(set(1, 1), set(1, 1), set(1, 1));
        let alice = identity("alice", "scope-a");
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let capacity = capacity.clone();
            let alice = alice.clone();
            move || {
                let _lease = capacity
                    .try_acquire(&alice, RelayResource::PublishTrack)
                    .unwrap();
                panic!("exercise relay capacity lease");
            }
        }));
        assert!(unwind.is_err());
        assert!(capacity
            .try_acquire(&alice, RelayResource::PublishTrack)
            .is_ok());
    }
}
