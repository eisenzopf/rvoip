// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::serve::{
    RetentionBudget, RetentionBudgetLimits, RetentionBudgetPool, RetentionBudgetPoolStats,
    RetentionBudgetStats, RetentionLimits,
};

/// A logical MoQT request family with independently reservable capacity.
///
/// Each class is independently bounded so one request family cannot consume
/// capacity reserved for another.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestClass {
    PublishNamespace,
    Subscribe,
    Publish,
    TrackStatus,
    Fetch,
}

impl RequestClass {
    pub const ALL: [Self; 5] = [
        Self::PublishNamespace,
        Self::Subscribe,
        Self::Publish,
        Self::TrackStatus,
        Self::Fetch,
    ];
}

/// Whether the local endpoint initiated a logical request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestDirection {
    Inbound,
    Outbound,
}

impl RequestDirection {
    pub const ALL: [Self; 2] = [Self::Inbound, Self::Outbound];
}

/// Limits for one request direction at one ownership scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestLimitSet {
    pub total: usize,
    pub publish_namespace: usize,
    pub subscribe: usize,
    pub publish: usize,
    pub track_status: usize,
    pub fetch: usize,
}

impl RequestLimitSet {
    fn class(self, class: RequestClass) -> usize {
        match class {
            RequestClass::PublishNamespace => self.publish_namespace,
            RequestClass::Subscribe => self.subscribe,
            RequestClass::Publish => self.publish,
            RequestClass::TrackStatus => self.track_status,
            RequestClass::Fetch => self.fetch,
        }
    }

    fn validate(self, scope: &'static str) -> Result<(), RequestCapacityError> {
        validate_semaphore_limit(scope, "total", self.total)?;
        for class in RequestClass::ALL {
            let field = match class {
                RequestClass::PublishNamespace => "publish_namespace",
                RequestClass::Subscribe => "subscribe",
                RequestClass::Publish => "publish",
                RequestClass::TrackStatus => "track_status",
                RequestClass::Fetch => "fetch",
            };
            validate_semaphore_limit(scope, field, self.class(class))?;
        }
        Ok(())
    }
}

fn validate_semaphore_limit(
    scope: &'static str,
    field: &'static str,
    value: usize,
) -> Result<(), RequestCapacityError> {
    if value == 0 {
        return Err(RequestCapacityError::ZeroLimit { scope, field });
    }
    if value > Semaphore::MAX_PERMITS {
        return Err(RequestCapacityError::LimitTooLarge {
            scope,
            field,
            value,
            maximum: Semaphore::MAX_PERMITS,
        });
    }
    Ok(())
}

/// Logical request and outbound queue limits owned by the transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestLimits {
    pub session_inbound: RequestLimitSet,
    pub session_outbound: RequestLimitSet,
    pub process_inbound: RequestLimitSet,
    pub process_outbound: RequestLimitSet,
    pub max_outbound_tasks: usize,
    pub max_outbound_messages: usize,
    pub max_response_commands: usize,
    pub max_reverse_updates: usize,
    pub retention: RetentionBudgetLimits,
    pub retention_track: RetentionLimits,
}

impl RequestLimits {
    pub fn validate(&self) -> Result<(), RequestCapacityError> {
        self.session_inbound.validate("session_inbound")?;
        self.session_outbound.validate("session_outbound")?;
        self.process_inbound.validate("process_inbound")?;
        self.process_outbound.validate("process_outbound")?;
        for (field, value) in [
            ("max_outbound_tasks", self.max_outbound_tasks),
            ("max_outbound_messages", self.max_outbound_messages),
            ("max_response_commands", self.max_response_commands),
            ("max_reverse_updates", self.max_reverse_updates),
        ] {
            if value == 0 {
                return Err(RequestCapacityError::ZeroLimit {
                    scope: "transport",
                    field,
                });
            }
        }
        RetentionBudgetPool::new(self.retention).map_err(|_| {
            RequestCapacityError::InvalidRetentionLimits {
                session_bytes: self.retention.max_session_bytes,
                process_bytes: self.retention.max_process_bytes,
            }
        })?;
        self.retention_track
            .validate()
            .map_err(|_| RequestCapacityError::InvalidRetentionTrackLimits)?;
        Ok(())
    }
}

impl Default for RequestLimits {
    fn default() -> Self {
        let session = RequestLimitSet {
            total: 128,
            publish_namespace: 16,
            subscribe: 128,
            publish: 128,
            track_status: 16,
            fetch: 32,
        };
        let process = RequestLimitSet {
            total: 4096,
            publish_namespace: 1024,
            subscribe: 4096,
            publish: 4096,
            track_status: 1024,
            fetch: 2048,
        };
        Self {
            session_inbound: session,
            session_outbound: session,
            process_inbound: process,
            process_outbound: process,
            max_outbound_tasks: 128,
            max_outbound_messages: 512,
            max_response_commands: 32,
            max_reverse_updates: 64,
            retention: RetentionBudgetLimits::default(),
            retention_track: RetentionLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RequestCapacityError {
    #[error("request capacity limit {scope}.{field} must be greater than zero")]
    ZeroLimit {
        scope: &'static str,
        field: &'static str,
    },
    #[error("request capacity limit {scope}.{field}={value} exceeds semaphore maximum {maximum}")]
    LimitTooLarge {
        scope: &'static str,
        field: &'static str,
        value: usize,
        maximum: usize,
    },
    #[error("{scope:?} {direction:?} {class:?} request capacity exhausted")]
    Exhausted {
        scope: RequestCapacityScope,
        direction: RequestDirection,
        class: RequestClass,
    },
    #[error("retention byte limits are invalid: session={session_bytes}, process={process_bytes}")]
    InvalidRetentionLimits {
        session_bytes: usize,
        process_bytes: usize,
    },
    #[error("per-track retention limits are invalid")]
    InvalidRetentionTrackLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestCapacityScope {
    SessionTotal,
    SessionClass,
    ProcessTotal,
    ProcessClass,
}

#[derive(Debug)]
struct ClassPools {
    publish_namespace: Arc<Semaphore>,
    subscribe: Arc<Semaphore>,
    publish: Arc<Semaphore>,
    track_status: Arc<Semaphore>,
    fetch: Arc<Semaphore>,
}

impl ClassPools {
    fn new(limits: RequestLimitSet) -> Self {
        Self {
            publish_namespace: Arc::new(Semaphore::new(limits.publish_namespace)),
            subscribe: Arc::new(Semaphore::new(limits.subscribe)),
            publish: Arc::new(Semaphore::new(limits.publish)),
            track_status: Arc::new(Semaphore::new(limits.track_status)),
            fetch: Arc::new(Semaphore::new(limits.fetch)),
        }
    }

    fn get(&self, class: RequestClass) -> Arc<Semaphore> {
        match class {
            RequestClass::PublishNamespace => self.publish_namespace.clone(),
            RequestClass::Subscribe => self.subscribe.clone(),
            RequestClass::Publish => self.publish.clone(),
            RequestClass::TrackStatus => self.track_status.clone(),
            RequestClass::Fetch => self.fetch.clone(),
        }
    }
}

#[derive(Debug)]
struct DirectionPools {
    total: Arc<Semaphore>,
    classes: ClassPools,
}

impl DirectionPools {
    fn new(limits: RequestLimitSet) -> Self {
        Self {
            total: Arc::new(Semaphore::new(limits.total)),
            classes: ClassPools::new(limits),
        }
    }
}

#[derive(Debug)]
struct CapacityPools {
    inbound: DirectionPools,
    outbound: DirectionPools,
}

impl CapacityPools {
    fn new(inbound: RequestLimitSet, outbound: RequestLimitSet) -> Self {
        Self {
            inbound: DirectionPools::new(inbound),
            outbound: DirectionPools::new(outbound),
        }
    }

    fn get(&self, direction: RequestDirection) -> &DirectionPools {
        match direction {
            RequestDirection::Inbound => &self.inbound,
            RequestDirection::Outbound => &self.outbound,
        }
    }
}

/// Process-owned request capacity. Create one of these per process and derive
/// one [`SessionRequestCapacity`] for every accepted connection.
#[derive(Clone, Debug)]
pub struct RequestCapacity {
    process: Arc<CapacityPools>,
    retention: RetentionBudgetPool,
    limits: Arc<RequestLimits>,
}

impl RequestCapacity {
    pub fn new(limits: RequestLimits) -> Result<Self, RequestCapacityError> {
        limits.validate()?;
        Ok(Self {
            process: Arc::new(CapacityPools::new(
                limits.process_inbound,
                limits.process_outbound,
            )),
            retention: RetentionBudgetPool::new(limits.retention).map_err(|_| {
                RequestCapacityError::InvalidRetentionLimits {
                    session_bytes: limits.retention.max_session_bytes,
                    process_bytes: limits.retention.max_process_bytes,
                }
            })?,
            limits: Arc::new(limits),
        })
    }

    pub fn session(&self) -> SessionRequestCapacity {
        SessionRequestCapacity {
            session: Arc::new(CapacityPools::new(
                self.limits.session_inbound,
                self.limits.session_outbound,
            )),
            process: self.process.clone(),
            retention: self.retention.session(),
            limits: self.limits.clone(),
        }
    }

    pub fn limits(&self) -> &RequestLimits {
        &self.limits
    }

    /// Return the process-wide retained-byte gauge for diagnostics and metrics.
    pub fn retention_stats(&self) -> RetentionBudgetPoolStats {
        self.retention.stats()
    }
}

impl Default for RequestCapacity {
    fn default() -> Self {
        Self::new(RequestLimits::default()).expect("default request limits are valid")
    }
}

/// Per-session view over both session-local and process-wide request pools.
#[derive(Clone, Debug)]
pub struct SessionRequestCapacity {
    session: Arc<CapacityPools>,
    process: Arc<CapacityPools>,
    retention: RetentionBudget,
    limits: Arc<RequestLimits>,
}

impl SessionRequestCapacity {
    /// Acquire all four permits in a fixed order without waiting. Any partial
    /// acquisition is rolled back before an error is returned.
    pub fn try_acquire(
        &self,
        direction: RequestDirection,
        class: RequestClass,
    ) -> Result<RequestLease, RequestCapacityError> {
        let session = self.session.get(direction);
        let process = self.process.get(direction);

        let session_total = Self::permit(
            session.total.clone(),
            RequestCapacityScope::SessionTotal,
            direction,
            class,
        )?;
        let session_class = Self::permit(
            session.classes.get(class),
            RequestCapacityScope::SessionClass,
            direction,
            class,
        )?;
        let process_total = Self::permit(
            process.total.clone(),
            RequestCapacityScope::ProcessTotal,
            direction,
            class,
        )?;
        let process_class = Self::permit(
            process.classes.get(class),
            RequestCapacityScope::ProcessClass,
            direction,
            class,
        )?;

        Ok(RequestLease {
            direction,
            class,
            permits: std::sync::Mutex::new(Some(RequestPermits {
                _session_total: session_total,
                _session_class: session_class,
                _process_total: process_total,
                _process_class: process_class,
            })),
        })
    }

    fn permit(
        semaphore: Arc<Semaphore>,
        scope: RequestCapacityScope,
        direction: RequestDirection,
        class: RequestClass,
    ) -> Result<OwnedSemaphorePermit, RequestCapacityError> {
        semaphore
            .try_acquire_owned()
            .map_err(|_| RequestCapacityError::Exhausted {
                scope,
                direction,
                class,
            })
    }

    pub fn limits(&self) -> &RequestLimits {
        &self.limits
    }

    pub fn retention_budget(&self) -> RetentionBudget {
        self.retention.clone()
    }

    pub fn retention_track_limits(&self) -> RetentionLimits {
        self.limits.retention_track
    }

    /// Return session and process retained-byte gauges for diagnostics.
    pub fn retention_stats(&self) -> RetentionBudgetStats {
        self.retention.stats()
    }
}

/// RAII ownership of one logical request slot. Dropping the final owner
/// atomically returns its session and process permits.
#[derive(Debug)]
pub struct RequestLease {
    direction: RequestDirection,
    class: RequestClass,
    permits: std::sync::Mutex<Option<RequestPermits>>,
}

#[derive(Debug)]
struct RequestPermits {
    _session_total: OwnedSemaphorePermit,
    _session_class: OwnedSemaphorePermit,
    _process_total: OwnedSemaphorePermit,
    _process_class: OwnedSemaphorePermit,
}

impl RequestLease {
    pub fn direction(&self) -> RequestDirection {
        self.direction
    }

    pub fn class(&self) -> RequestClass {
        self.class
    }

    /// Return capacity before all observer handles are dropped. This is used
    /// by transport lifecycle guards when a peer closes a request stream while
    /// an application still retains a now-closed handle.
    pub(crate) fn release(&self) {
        self.permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }

    #[cfg(test)]
    fn is_released(&self) -> bool {
        self.permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
    }
}

#[cfg(test)]
pub(super) fn test_request_lease(
    direction: RequestDirection,
    class: RequestClass,
) -> Arc<RequestLease> {
    Arc::new(
        RequestCapacity::default()
            .session()
            .try_acquire(direction, class)
            .unwrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(session_total: usize, process_total: usize, class: usize) -> RequestLimits {
        let session = RequestLimitSet {
            total: session_total,
            publish_namespace: class,
            subscribe: class,
            publish: class,
            track_status: class,
            fetch: class,
        };
        let process = RequestLimitSet {
            total: process_total,
            ..session
        };
        RequestLimits {
            session_inbound: session,
            session_outbound: session,
            process_inbound: process,
            process_outbound: process,
            max_outbound_tasks: 2,
            max_outbound_messages: 2,
            max_response_commands: 2,
            max_reverse_updates: 2,
            retention: RetentionBudgetLimits::default(),
            retention_track: RetentionLimits::default(),
        }
    }

    #[test]
    fn rejects_zero_limits() {
        let mut limits = limits(1, 1, 1);
        limits.session_inbound.fetch = 0;
        assert!(matches!(
            RequestCapacity::new(limits),
            Err(RequestCapacityError::ZeroLimit {
                scope: "session_inbound",
                field: "fetch"
            })
        ));
    }

    #[test]
    fn rejects_every_request_limit_above_tokios_semaphore_maximum() {
        for field in [
            "total",
            "publish_namespace",
            "subscribe",
            "publish",
            "track_status",
            "fetch",
        ] {
            let mut configured = limits(1, 1, 1);
            let limits = &mut configured.session_inbound;
            match field {
                "total" => limits.total = Semaphore::MAX_PERMITS + 1,
                "publish_namespace" => limits.publish_namespace = Semaphore::MAX_PERMITS + 1,
                "subscribe" => limits.subscribe = Semaphore::MAX_PERMITS + 1,
                "publish" => limits.publish = Semaphore::MAX_PERMITS + 1,
                "track_status" => limits.track_status = Semaphore::MAX_PERMITS + 1,
                "fetch" => limits.fetch = Semaphore::MAX_PERMITS + 1,
                _ => unreachable!(),
            }
            assert!(matches!(
                RequestCapacity::new(configured),
                Err(RequestCapacityError::LimitTooLarge {
                    scope: "session_inbound",
                    field: rejected,
                    value,
                    maximum,
                }) if rejected == field
                    && value == Semaphore::MAX_PERMITS + 1
                    && maximum == Semaphore::MAX_PERMITS
            ));
        }
    }

    #[test]
    fn validates_and_threads_retention_limits_into_each_session() {
        let mut invalid = limits(1, 1, 1);
        invalid.retention.max_session_bytes = 0;
        assert!(matches!(
            RequestCapacity::new(invalid),
            Err(RequestCapacityError::InvalidRetentionLimits { .. })
        ));

        let mut configured = limits(1, 2, 2);
        configured.retention = RetentionBudgetLimits {
            max_session_bytes: 64,
            max_process_bytes: 128,
        };
        configured.retention_track.max_object_bytes = 32;
        let capacity = RequestCapacity::new(configured).unwrap();
        let session = capacity.session();
        assert_eq!(capacity.retention_stats().process_bytes, 0);
        assert_eq!(capacity.retention_stats().max_process_bytes, 128);
        assert_eq!(session.retention_budget().limits().max_session_bytes, 64);
        assert_eq!(session.retention_budget().limits().max_process_bytes, 128);
        assert_eq!(session.retention_track_limits().max_object_bytes, 32);
        assert_eq!(session.retention_stats().session_bytes, 0);
        assert_eq!(session.retention_stats().process_bytes, 0);
    }

    #[test]
    fn every_class_and_direction_is_fail_fast_and_reusable() {
        for direction in RequestDirection::ALL {
            for class in RequestClass::ALL {
                let capacity = RequestCapacity::new(limits(2, 2, 1)).unwrap();
                let session = capacity.session();
                let lease = session.try_acquire(direction, class).unwrap();
                assert!(matches!(
                    session.try_acquire(direction, class),
                    Err(RequestCapacityError::Exhausted {
                        scope: RequestCapacityScope::SessionClass,
                        ..
                    })
                ));
                drop(lease);
                assert!(session.try_acquire(direction, class).is_ok());
            }
        }
    }

    #[test]
    fn session_total_is_shared_across_classes_but_not_directions() {
        let capacity = RequestCapacity::new(limits(1, 4, 4)).unwrap();
        let session = capacity.session();
        let inbound = session
            .try_acquire(RequestDirection::Inbound, RequestClass::Subscribe)
            .unwrap();
        assert!(matches!(
            session.try_acquire(RequestDirection::Inbound, RequestClass::Publish),
            Err(RequestCapacityError::Exhausted {
                scope: RequestCapacityScope::SessionTotal,
                ..
            })
        ));
        assert!(session
            .try_acquire(RequestDirection::Outbound, RequestClass::Publish)
            .is_ok());
        drop(inbound);
    }

    #[test]
    fn process_total_is_shared_and_partial_session_acquisition_rolls_back() {
        let capacity = RequestCapacity::new(limits(1, 1, 2)).unwrap();
        let first = capacity.session();
        let second = capacity.session();
        let lease = first
            .try_acquire(RequestDirection::Inbound, RequestClass::Subscribe)
            .unwrap();

        assert!(matches!(
            second.try_acquire(RequestDirection::Inbound, RequestClass::Publish),
            Err(RequestCapacityError::Exhausted {
                scope: RequestCapacityScope::ProcessTotal,
                ..
            })
        ));
        // The failed process acquisition must not strand second's session slot.
        assert!(second
            .try_acquire(RequestDirection::Outbound, RequestClass::Publish)
            .is_ok());

        drop(lease);
        assert!(second
            .try_acquire(RequestDirection::Inbound, RequestClass::Publish)
            .is_ok());
    }

    #[test]
    fn panic_unwind_releases_every_permit() {
        let capacity = RequestCapacity::new(limits(1, 1, 1)).unwrap();
        let session = capacity.session();
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let session = session.clone();
            move || {
                let _lease = session
                    .try_acquire(RequestDirection::Inbound, RequestClass::Fetch)
                    .unwrap();
                panic!("exercise RAII rollback");
            }
        }));
        assert!(unwind.is_err());
        assert!(session
            .try_acquire(RequestDirection::Inbound, RequestClass::Fetch)
            .is_ok());
    }

    #[test]
    fn explicit_release_is_idempotent_across_shared_observers() {
        let capacity = RequestCapacity::new(limits(1, 1, 1)).unwrap();
        let session = capacity.session();
        let lease = Arc::new(
            session
                .try_acquire(RequestDirection::Inbound, RequestClass::Subscribe)
                .unwrap(),
        );
        let observer = lease.clone();
        lease.release();
        observer.release();
        assert!(lease.is_released());
        assert!(session
            .try_acquire(RequestDirection::Inbound, RequestClass::Subscribe)
            .is_ok());
    }
}
