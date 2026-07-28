//! Global Event Coordinator for Monolithic and Distributed Deployments
//!
//! Provides a unified event system that replaces individual crate event processors
//! with a single shared event bus, reducing thread count by 50-75%.
//
// The distributed-mode pieces (network_transport publishing, plane routing,
// ChannelForwarder, EventSubscription bookkeeping) are scaffolded but not
// wired to a running deployment yet, so several fields and helpers are
// reachable only on the future code path that consumes them.
#![allow(dead_code)]

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{broadcast, mpsc, Mutex as TokioMutex, Notify, OnceCell, RwLock};
use tracing::{debug, error, info, warn};

use crate::events::system::EventSystem;
use crate::events::types::EventPriority;
use crate::planes::{LayerTaskManager, PlaneConfig, PlaneRouter, PlaneType, TaskPriority};

use crate::events::cross_crate::{
    CrossCrateEvent, EventTypeId, OrchestrationCrossCrateEvent, RvoipCoreCrossCrateEvent,
};

use super::config::{DeploymentConfig, EventCoordinatorConfig};
use super::transport::NetworkTransport;

/// Global singleton instance for monolithic deployments
static GLOBAL_COORDINATOR: OnceCell<Arc<GlobalEventCoordinator>> = OnceCell::const_new();

#[cfg(not(test))]
const OBSERVATIONAL_HANDLER_SHUTDOWN_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(1);
#[cfg(test)]
const OBSERVATIONAL_HANDLER_SHUTDOWN_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(100);
const COORDINATOR_OPEN: u8 = 0;
const COORDINATOR_DRAINING: u8 = 1;
const COORDINATOR_CLOSED: u8 = 2;

/// Get the global coordinator instance for monolithic deployments
///
/// This function returns a reference to the global singleton coordinator.
/// On first call, it initializes the coordinator with monolithic configuration.
/// Subsequent calls return the same instance.
///
/// # Panics
/// Panics if the coordinator fails to initialize (should only happen on first call)
///
/// # Example
/// ```rust,no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::Arc;
/// use rvoip_infra_common::events::coordinator::global_coordinator;
/// use rvoip_infra_common::events::cross_crate::{
///     CrossCrateEvent, RvoipCrossCrateEvent, SessionToDialogEvent,
/// };
///
/// // Get the global instance - initialized on first access
/// let coordinator = global_coordinator().await;
///
/// // Publish an event
/// let event: Arc<dyn CrossCrateEvent> = Arc::new(RvoipCrossCrateEvent::SessionToDialog(
///     SessionToDialogEvent::TerminateSession {
///         session_id: "session-1".into(),
///         reason: "normal clearing".into(),
///     },
/// ));
/// coordinator.publish(event).await?;
/// # Ok(())
/// # }
/// ```
pub async fn global_coordinator() -> &'static Arc<GlobalEventCoordinator> {
    GLOBAL_COORDINATOR
        .get_or_init(|| async {
            info!("Initializing global event coordinator singleton for monolithic deployment");

            // Try to load config from environment
            let config = EventCoordinatorConfig::from_env()
                .unwrap_or_else(|_| EventCoordinatorConfig::monolithic());

            Arc::new(
                GlobalEventCoordinator::new(config)
                    .await
                    .expect("Failed to initialize global event coordinator"),
            )
        })
        .await
}

/// Global event coordinator supporting both monolithic and distributed modes
pub struct GlobalEventCoordinator {
    /// Configuration
    config: EventCoordinatorConfig,

    /// Core event bus (StaticFastPath for monolithic, network-aware for distributed)
    event_bus: Arc<dyn EventBusAdapter>,

    /// Network transport for distributed mode (None for monolithic)
    network_transport: Option<Arc<dyn NetworkTransport>>,

    /// Plane-aware event routing
    plane_router: Arc<PlaneRouter>,

    /// Unified task manager for all event processing
    task_manager: Arc<LayerTaskManager>,

    /// Event type registry for cross-crate event management
    event_registry: Arc<EventTypeRegistry>,

    /// Registered event handlers by type.
    ///
    /// Protocol publications retain the historical synchronous handler path.
    /// Observational publications (notably `session_to_app`) use each
    /// handler's independent bounded FIFO so one hostile observer cannot
    /// delay bus delivery, later observers, or authoritative cleanup.
    handlers: Arc<DashMap<EventTypeId, Vec<Arc<RegisteredEventHandler>>>>,
    next_handler_id: AtomicU64,
    observational_handler_metrics: Arc<ObservationalHandlerMetrics>,
    handler_admission_gate: Arc<RwLock<()>>,
    lifecycle_state: Arc<AtomicU8>,
    lifecycle_closed: Arc<Notify>,
    shutdown_error: Arc<StdMutex<Option<String>>>,
    observational_publish_gates: Arc<DashMap<EventTypeId, Arc<TokioMutex<()>>>>,

    /// Active event subscriptions
    subscriptions: Arc<RwLock<HashMap<EventTypeId, Vec<EventSubscription>>>>,
}

struct RegisteredEventHandler {
    id: u64,
    handler: Arc<dyn CrossCrateEventHandler>,
    observational_sender: StdMutex<Option<mpsc::Sender<ObservationalHandlerEnvelope>>>,
    observational_task: TokioMutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Default)]
struct ObservationalHandlerMetrics {
    enqueued_total: AtomicU64,
    queued_current: AtomicU64,
    queued_max: AtomicU64,
    in_flight_current: AtomicU64,
    in_flight_max: AtomicU64,
    delivered_total: AtomicU64,
    handler_failures: AtomicU64,
    dropped_full: AtomicU64,
    dropped_closed: AtomicU64,
    dropped_shutdown_queued: AtomicU64,
    dropped_shutdown_in_flight: AtomicU64,
    shutdown_aborted_workers: AtomicU64,
}

struct ObservationalHandlerEnvelope {
    event: Arc<dyn CrossCrateEvent>,
    accounting: ObservationalQueueAccounting,
}

struct ObservationalQueueAccounting {
    metrics: Arc<ObservationalHandlerMetrics>,
    queued: bool,
    count_shutdown_drop: bool,
}

impl ObservationalQueueAccounting {
    fn new(metrics: Arc<ObservationalHandlerMetrics>) -> Self {
        metrics.enqueued_total.fetch_add(1, Ordering::Relaxed);
        let queued = metrics.queued_current.fetch_add(1, Ordering::Relaxed) + 1;
        record_observational_max(&metrics.queued_max, queued);
        Self {
            metrics,
            queued: true,
            count_shutdown_drop: true,
        }
    }

    fn begin_delivery(mut self) -> ObservationalInFlightAccounting {
        self.remove_from_queue();
        let in_flight = self
            .metrics
            .in_flight_current
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        record_observational_max(&self.metrics.in_flight_max, in_flight);
        ObservationalInFlightAccounting {
            metrics: Arc::clone(&self.metrics),
            completed: false,
        }
    }

    fn remove_from_queue(&mut self) {
        if self.queued {
            self.metrics.queued_current.fetch_sub(1, Ordering::Relaxed);
            self.queued = false;
        }
    }

    fn rejected(mut self) {
        self.count_shutdown_drop = false;
        self.remove_from_queue();
    }
}

impl Drop for ObservationalQueueAccounting {
    fn drop(&mut self) {
        if self.queued && self.count_shutdown_drop {
            self.metrics
                .dropped_shutdown_queued
                .fetch_add(1, Ordering::Relaxed);
        }
        self.remove_from_queue();
    }
}

struct ObservationalInFlightAccounting {
    metrics: Arc<ObservationalHandlerMetrics>,
    completed: bool,
}

impl ObservationalInFlightAccounting {
    fn delivered(mut self) {
        self.metrics.delivered_total.fetch_add(1, Ordering::Relaxed);
        self.completed = true;
    }

    fn failed(mut self) {
        self.metrics
            .handler_failures
            .fetch_add(1, Ordering::Relaxed);
        self.completed = true;
    }
}

impl Drop for ObservationalInFlightAccounting {
    fn drop(&mut self) {
        if !self.completed {
            self.metrics
                .dropped_shutdown_in_flight
                .fetch_add(1, Ordering::Relaxed);
        }
        self.metrics
            .in_flight_current
            .fetch_sub(1, Ordering::Relaxed);
    }
}

fn record_observational_max(maximum: &AtomicU64, observed: u64) {
    let mut current = maximum.load(Ordering::Relaxed);
    while observed > current {
        match maximum.compare_exchange_weak(current, observed, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DeploymentMode {
    Monolithic,
    Distributed,
}

/// Trait for event bus adapters (monolithic vs distributed)
#[async_trait]
pub trait EventBusAdapter: Send + Sync {
    async fn publish(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()>;
    async fn subscribe(
        &self,
        event_type: EventTypeId,
    ) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>>;
    async fn shutdown(&self) -> Result<()>;

    /// Point-in-time retained queue counts for diagnostic builds.
    /// Implementations that do not retain in-process queues may keep the
    /// default empty value.
    fn diagnostic_snapshot(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

/// Monolithic event bus adapter using broadcast channels
pub struct MonolithicEventBus {
    event_bus: Arc<EventSystem>,
    task_manager: Arc<LayerTaskManager>,
    /// Broadcast channels by event type - lock-free publishing!
    broadcasters: Arc<DashMap<EventTypeId, broadcast::Sender<Arc<dyn CrossCrateEvent>>>>,
    /// Weak handles to compatibility subscriber queues. Weak senders expose
    /// queue depth without keeping a dropped application subscription alive.
    subscriber_queues: Arc<DashMap<EventTypeId, Vec<SubscriberQueueDiagnostic>>>,
    next_subscriber_queue_id: AtomicU64,
    subscriber_dead_weak_slots_pruned: Arc<AtomicU64>,
    /// Channel capacity for broadcast channels
    channel_capacity: usize,
}

struct SubscriberQueueDiagnostic {
    id: u64,
    sender: mpsc::WeakSender<Arc<dyn CrossCrateEvent>>,
}

fn prune_subscriber_queues(
    queues: &DashMap<EventTypeId, Vec<SubscriberQueueDiagnostic>>,
    event_type: EventTypeId,
    remove_id: Option<u64>,
) -> usize {
    let Some(mut event_queues) = queues.get_mut(event_type) else {
        return 0;
    };
    let before = event_queues.len();
    event_queues.retain(|entry| remove_id != Some(entry.id) && entry.sender.upgrade().is_some());
    before.saturating_sub(event_queues.len())
}

#[async_trait]
impl EventBusAdapter for MonolithicEventBus {
    async fn publish(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        let event_type = event.event_type();
        debug!("Publishing cross-crate event: {}", event_type);

        // Get or create broadcast channel for this event type
        let sender = self
            .broadcasters
            .entry(event_type)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.channel_capacity);
                tx
            })
            .clone();

        // Send to all subscribers - completely lock-free!
        // Broadcast automatically handles disconnected receivers
        match sender.send(event) {
            Ok(receiver_count) => {
                debug!(
                    "Event {} sent to {} subscribers",
                    event_type, receiver_count
                );
            }
            Err(_) => {
                // No receivers currently listening, but that's ok
                debug!("No subscribers for event type {}", event_type);
            }
        }

        Ok(())
    }

    async fn subscribe(
        &self,
        event_type: EventTypeId,
    ) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>> {
        // Get or create broadcast channel for this event type
        let sender = self
            .broadcasters
            .entry(event_type)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.channel_capacity);
                tx
            })
            .clone();

        // Subscribe to the broadcast channel
        let mut broadcast_rx = sender.subscribe();

        // Create mpsc channel for API compatibility
        let (mpsc_tx, mpsc_rx) = mpsc::channel(self.channel_capacity);
        let subscriber_queue_id = self
            .next_subscriber_queue_id
            .fetch_add(1, Ordering::Relaxed);
        let pruned = prune_subscriber_queues(&self.subscriber_queues, event_type, None);
        self.subscriber_dead_weak_slots_pruned
            .fetch_add(pruned as u64, Ordering::Relaxed);
        self.subscriber_queues
            .entry(event_type)
            .or_default()
            .push(SubscriberQueueDiagnostic {
                id: subscriber_queue_id,
                sender: mpsc_tx.downgrade(),
            });

        // Spawn a task to bridge broadcast to mpsc
        // This maintains API compatibility while using broadcast internally
        let subscriber_queues = Arc::clone(&self.subscriber_queues);
        let subscriber_dead_weak_slots_pruned = Arc::clone(&self.subscriber_dead_weak_slots_pruned);
        self.task_manager
            .spawn_tracked(
                format!("broadcast-to-mpsc-{event_type}-{subscriber_queue_id}"),
                TaskPriority::Normal,
                async move {
                    debug!(
                        "Starting broadcast->mpsc bridge for event type: {}",
                        event_type
                    );
                    loop {
                        let received = tokio::select! {
                            _ = mpsc_tx.closed() => {
                                debug!(
                                    "Stopping bridge for event type {} - receiver dropped",
                                    event_type
                                );
                                break;
                            }
                            received = broadcast_rx.recv() => received,
                        };
                        match received {
                            Ok(event) => {
                                // Forward to mpsc channel
                                if mpsc_tx.send(event).await.is_err() {
                                    debug!(
                                        "Stopping bridge for event type {} - receiver dropped",
                                        event_type
                                    );
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                warn!(
                                    event_type,
                                    skipped,
                                    "Broadcast subscriber lagged; continuing bridge after skipped events"
                                );
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                debug!(
                                    "Stopping bridge for event type {} - broadcast closed",
                                    event_type
                                );
                                break;
                            }
                        }
                    }
                    let pruned = prune_subscriber_queues(
                        &subscriber_queues,
                        event_type,
                        Some(subscriber_queue_id),
                    );
                    subscriber_dead_weak_slots_pruned
                        .fetch_add(pruned as u64, Ordering::Relaxed);
                    debug!("Bridge task ending for event type: {}", event_type);
                },
            )
            .await?;

        debug!("Subscribed to cross-crate event type: {}", event_type);

        Ok(mpsc_rx)
    }

    async fn shutdown(&self) -> Result<()> {
        use crate::events::api::EventSystem as EventSystemTrait;
        let event_system_result = self
            .event_bus
            .shutdown()
            .await
            .map_err(|e| anyhow::anyhow!("Event system shutdown failed: {}", e));
        // Dropping every retained sender closes each broadcast receiver. The
        // tracked bridge tasks then release their mpsc sender, so an existing
        // application receiver observes closure before this method returns.
        self.broadcasters.clear();
        let task_manager_result = self.task_manager.shutdown_all().await;
        self.subscriber_queues.clear();
        match (event_system_result, task_manager_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(event_error), Ok(())) => Err(event_error),
            (Ok(()), Err(task_error)) => Err(task_error),
            (Err(event_error), Err(task_error)) => Err(anyhow::anyhow!(
                "{event_error:#}; task manager shutdown failed: {task_error:#}"
            )),
        }
    }

    fn diagnostic_snapshot(&self) -> serde_json::Value {
        let mut event_types = serde_json::Map::new();
        let mut retained_broadcast_total = 0_usize;
        let mut queued_subscriber_total = 0_usize;
        let mut subscriber_weak_slots_total = 0_usize;
        for entry in self.broadcasters.iter() {
            let event_type = *entry.key();
            let sender = entry.value();
            let retained = sender.len();
            retained_broadcast_total = retained_broadcast_total.saturating_add(retained);

            let mut subscriber_queue_count = 0_usize;
            let mut subscriber_queued = 0_usize;
            let mut subscriber_queued_max = 0_usize;
            let pruned = prune_subscriber_queues(&self.subscriber_queues, event_type, None);
            self.subscriber_dead_weak_slots_pruned
                .fetch_add(pruned as u64, Ordering::Relaxed);
            let mut subscriber_weak_slots = 0_usize;
            if let Some(queues) = self.subscriber_queues.get(event_type) {
                subscriber_weak_slots = queues.len();
                subscriber_weak_slots_total =
                    subscriber_weak_slots_total.saturating_add(subscriber_weak_slots);
                for diagnostic in queues.iter() {
                    if let Some(queue) = diagnostic.sender.upgrade() {
                        subscriber_queue_count = subscriber_queue_count.saturating_add(1);
                        let queued = queue.max_capacity().saturating_sub(queue.capacity());
                        subscriber_queued = subscriber_queued.saturating_add(queued);
                        subscriber_queued_max = subscriber_queued_max.max(queued);
                    }
                }
            }
            queued_subscriber_total = queued_subscriber_total.saturating_add(subscriber_queued);
            event_types.insert(
                event_type.to_string(),
                serde_json::json!({
                    "broadcast_retained": retained,
                    "broadcast_capacity": self.channel_capacity,
                    "broadcast_receivers": sender.receiver_count(),
                    "subscriber_queue_count": subscriber_queue_count,
                    "subscriber_weak_slots": subscriber_weak_slots,
                    "subscriber_queued": subscriber_queued,
                    "subscriber_queued_max": subscriber_queued_max,
                }),
            );
        }
        serde_json::json!({
            "mode": "monolithic",
            "channel_capacity": self.channel_capacity,
            "broadcast_retained_total": retained_broadcast_total,
            "subscriber_queued_total": queued_subscriber_total,
            "subscriber_weak_slots_total": subscriber_weak_slots_total,
            "subscriber_dead_weak_slots_pruned_total": self
                .subscriber_dead_weak_slots_pruned
                .load(Ordering::Relaxed),
            "event_types": event_types,
        })
    }
}

/// Event handler that forwards to a channel
struct ChannelForwarder {
    tx: mpsc::Sender<Arc<dyn CrossCrateEvent>>,
}

impl ChannelForwarder {
    fn new(tx: mpsc::Sender<Arc<dyn CrossCrateEvent>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl CrossCrateEventHandler for ChannelForwarder {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        if let Err(_) = self.tx.try_send(event) {
            warn!("Event channel full, dropping event");
        }
        Ok(())
    }
}

impl GlobalEventCoordinator {
    /// Create a new coordinator with the given configuration
    pub async fn new(config: EventCoordinatorConfig) -> Result<Self> {
        match &config.deployment {
            DeploymentConfig::Monolithic => Self::new_monolithic(config).await,
            DeploymentConfig::Distributed { .. } => Self::new_distributed(config).await,
        }
    }

    /// Create coordinator for monolithic deployment (single process)
    ///
    /// **Note**: For most monolithic applications, use `global_coordinator()` instead
    /// to get the singleton instance. Only create a new instance if you need
    /// isolated event handling (e.g., for testing or special use cases).
    #[deprecated(
        note = "Use global_coordinator() for singleton access or GlobalEventCoordinator::new() with config"
    )]
    pub async fn monolithic() -> Result<Self> {
        Self::new(EventCoordinatorConfig::monolithic()).await
    }

    /// Create a monolithic coordinator
    async fn new_monolithic(config: EventCoordinatorConfig) -> Result<Self> {
        let event_bus = Arc::new(EventSystem::new_static_fast_path(config.channel_capacity));
        let task_manager = Arc::new(LayerTaskManager::new("global"));

        let monolithic_adapter = Arc::new(MonolithicEventBus {
            event_bus,
            task_manager: task_manager.clone(),
            broadcasters: Arc::new(DashMap::new()),
            subscriber_queues: Arc::new(DashMap::new()),
            next_subscriber_queue_id: AtomicU64::new(1),
            subscriber_dead_weak_slots_pruned: Arc::new(AtomicU64::new(0)),
            channel_capacity: config.channel_capacity,
        });

        Ok(Self {
            config,
            event_bus: monolithic_adapter,
            network_transport: None,
            plane_router: Arc::new(PlaneRouter::new(PlaneConfig::Local)),
            task_manager,
            event_registry: Arc::new(EventTypeRegistry::new()),
            handlers: Arc::new(DashMap::new()),
            next_handler_id: AtomicU64::new(1),
            observational_handler_metrics: Arc::new(ObservationalHandlerMetrics::default()),
            handler_admission_gate: Arc::new(RwLock::new(())),
            lifecycle_state: Arc::new(AtomicU8::new(COORDINATOR_OPEN)),
            lifecycle_closed: Arc::new(Notify::new()),
            shutdown_error: Arc::new(StdMutex::new(None)),
            observational_publish_gates: Arc::new(DashMap::new()),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Create a distributed coordinator (stub)
    async fn new_distributed(config: EventCoordinatorConfig) -> Result<Self> {
        error!("Distributed mode not yet implemented");

        // Extract transport and discovery config
        let (transport_config, discovery_config) = match &config.deployment {
            DeploymentConfig::Distributed {
                transport,
                discovery,
            } => (transport, discovery),
            _ => unreachable!("new_distributed called with non-distributed config"),
        };

        // Log what would be configured
        info!(
            "Would create distributed coordinator with transport: {:?}, discovery: {:?}",
            transport_config, discovery_config
        );

        // For now, return an error
        Err(anyhow::anyhow!(
            "Distributed mode is not yet implemented. \
            Please use monolithic mode or wait for distributed support. \
            Attempted config: transport={:?}, discovery={:?}",
            transport_config,
            discovery_config
        ))
    }

    /// Get the deployment mode
    pub fn deployment_mode(&self) -> &DeploymentConfig {
        &self.config.deployment
    }

    /// Get the service name
    pub fn service_name(&self) -> &str {
        &self.config.service_name
    }

    /// Register an event handler for a specific event type
    pub async fn register_handler<H>(&self, event_type: EventTypeId, handler: H) -> Result<()>
    where
        H: CrossCrateEventHandler + 'static,
    {
        let admission = self.handler_admission_gate.read().await;
        if self.lifecycle_state.load(Ordering::Acquire) != COORDINATOR_OPEN {
            return Err(anyhow::anyhow!("Event coordinator is draining or closed"));
        }
        let handler: Arc<dyn CrossCrateEventHandler> = Arc::new(handler);
        let handler_id = self.next_handler_id.fetch_add(1, Ordering::Relaxed);
        let (observational_sender, mut observational_receiver) =
            mpsc::channel::<ObservationalHandlerEnvelope>(self.config.channel_capacity.max(1));
        let worker_handler = Arc::clone(&handler);
        let worker_event_type = event_type;
        let observational_task = tokio::spawn(async move {
            while let Some(envelope) = observational_receiver.recv().await {
                let in_flight = envelope.accounting.begin_delivery();
                if let Err(error) = worker_handler.handle(envelope.event).await {
                    in_flight.failed();
                    warn!(
                        event_type = worker_event_type,
                        handler_id,
                        error_class = "handler",
                        "Observational event handler failed: {error}"
                    );
                } else {
                    in_flight.delivered();
                }
            }
        });
        let registered = Arc::new(RegisteredEventHandler {
            id: handler_id,
            handler,
            observational_sender: StdMutex::new(Some(observational_sender)),
            observational_task: TokioMutex::new(Some(observational_task)),
        });

        // Add to handlers registry
        self.handlers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(registered);
        drop(admission);

        debug!("Registered handler for event type: {}", event_type);
        Ok(())
    }

    /// Publish an event through the global coordinator
    pub async fn publish(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        let event_type = event.event_type();
        let admission = self.handler_admission_gate.read().await;
        if self.lifecycle_state.load(Ordering::Acquire) != COORDINATOR_OPEN {
            return Err(anyhow::anyhow!("Event coordinator is draining or closed"));
        }

        // For distributed mode, check if we need network transport
        if let DeploymentConfig::Distributed { .. } = &self.config.deployment {
            if let Some(_transport) = &self.network_transport {
                // TODO: Determine target service from event metadata
                warn!(
                    "Distributed event publishing not yet implemented for event: {}",
                    event_type
                );
                return Err(anyhow::anyhow!(
                    "Distributed event publishing not yet implemented"
                ));
            }
        }

        debug!("Publishing event type: {}", event_type);

        // Call registered handlers for this event type
        let handlers = self
            .handlers
            .get(event_type)
            .map(|handlers| handlers.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if !handlers.is_empty() {
            debug!(
                "Found {} handlers for event type: {}",
                handlers.len(),
                event_type
            );
            // The DashMap shard guard was dropped before the first handler
            // await. Shutdown may therefore wait asynchronously on the
            // admission gate without synchronously deadlocking this runtime.
            for handler in &handlers {
                if let Err(e) = handler.handler.handle(event.clone()).await {
                    warn!("Handler failed for event type {}: {}", event_type, e);
                }
            }
        } else {
            debug!("No handlers registered for event type: {}", event_type);
        }

        // TODO: Add plane-aware routing for cross-crate events
        debug!("Routing cross-crate event through planes");

        // Publish through event bus (for subscribers)
        self.event_bus.publish(event).await?;
        drop(admission);

        Ok(())
    }

    /// Deliver a protocol-authoritative cross-crate event and report whether
    /// at least one registered in-process handler acknowledged it. Handler
    /// failures are propagated instead of being downgraded to observational
    /// warnings. The event-bus copy remains lock-free/best-effort and cannot
    /// backpressure the authoritative handler path.
    pub async fn publish_authoritative(&self, event: Arc<dyn CrossCrateEvent>) -> Result<bool> {
        let event_type = event.event_type();
        let _admission = self.handler_admission_gate.read().await;
        if self.lifecycle_state.load(Ordering::Acquire) != COORDINATOR_OPEN {
            return Err(anyhow::anyhow!("Event coordinator is draining or closed"));
        }
        if let DeploymentConfig::Distributed { .. } = &self.config.deployment {
            if self.network_transport.is_some() {
                return Err(anyhow::anyhow!(
                    "Distributed authoritative event publishing is not implemented"
                ));
            }
        }

        let handlers = self
            .handlers
            .get(event_type)
            .map(|handlers| handlers.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let [handler] = handlers.as_slice() else {
            return if handlers.is_empty() {
                Ok(false)
            } else {
                Err(anyhow::anyhow!(
                    "Authoritative event type {event_type} has more than one direct handler"
                ))
            };
        };
        handler.handler.handle(Arc::clone(&event)).await?;
        if let Err(error) = self.event_bus.publish(event).await {
            warn!(
                event_type,
                %error,
                "Observational event-bus copy failed after authoritative delivery"
            );
        }
        Ok(true)
    }

    /// Deliver a protocol-authoritative event to its one direct in-process
    /// handler without publishing the event to the observational bus.
    ///
    /// This boundary is intended for private, capability-bearing control
    /// messages. Callers that also expose a public observation must publish a
    /// separately sanitized event type; the private event is never made
    /// available to subscribers by this method.
    pub async fn dispatch_authoritative_handler(
        &self,
        event: Arc<dyn CrossCrateEvent>,
    ) -> Result<bool> {
        let event_type = event.event_type();
        let _admission = self.handler_admission_gate.read().await;
        if self.lifecycle_state.load(Ordering::Acquire) != COORDINATOR_OPEN {
            return Err(anyhow::anyhow!("Event coordinator is draining or closed"));
        }
        if let DeploymentConfig::Distributed { .. } = &self.config.deployment {
            if self.network_transport.is_some() {
                return Err(anyhow::anyhow!(
                    "Distributed authoritative event dispatch is not implemented"
                ));
            }
        }

        let handlers = self
            .handlers
            .get(event_type)
            .map(|handlers| handlers.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let [handler] = handlers.as_slice() else {
            return if handlers.is_empty() {
                Ok(false)
            } else {
                Err(anyhow::anyhow!(
                    "Authoritative event type {event_type} has more than one direct handler"
                ))
            };
        };
        handler.handler.handle(event).await?;
        Ok(true)
    }

    /// Publish an application-observational event without allowing a handler
    /// to block authoritative bus delivery or the publishing protocol path.
    ///
    /// The bus is delivered first. Registered handlers then receive the same
    /// event through independent bounded FIFO workers. A saturated handler is
    /// diagnosed and skips only its own copy; it cannot make this method fail
    /// after subscribers have already observed the event.
    pub async fn publish_observational(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        let event_type = event.event_type();
        let _admission = self.handler_admission_gate.read().await;
        if self.lifecycle_state.load(Ordering::Acquire) != COORDINATOR_OPEN {
            return Err(anyhow::anyhow!("Event coordinator is draining or closed"));
        }
        if let DeploymentConfig::Distributed { .. } = &self.config.deployment {
            if self.network_transport.is_some() {
                return Err(anyhow::anyhow!(
                    "Distributed observational event publishing is not implemented"
                ));
            }
        }

        let publish_gate = self
            .observational_publish_gates
            .entry(event_type)
            .or_insert_with(|| Arc::new(TokioMutex::new(())))
            .clone();
        let _sequence = publish_gate.lock().await;

        // This is the authoritative public-observation boundary. Do not put a
        // cancellable aggregate handler future in front of it.
        self.event_bus.publish(Arc::clone(&event)).await?;

        let handlers = self
            .handlers
            .get(event_type)
            .map(|handlers| handlers.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if handlers.is_empty() {
            return Ok(());
        }
        for handler in &handlers {
            let accounting =
                ObservationalQueueAccounting::new(Arc::clone(&self.observational_handler_metrics));
            let sender = handler
                .observational_sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let Some(sender) = sender else {
                self.observational_handler_metrics
                    .dropped_closed
                    .fetch_add(1, Ordering::Relaxed);
                accounting.rejected();
                continue;
            };
            let envelope = ObservationalHandlerEnvelope {
                event: Arc::clone(&event),
                accounting,
            };
            match sender.try_send(envelope) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(envelope)) => {
                    self.observational_handler_metrics
                        .dropped_full
                        .fetch_add(1, Ordering::Relaxed);
                    envelope.accounting.rejected();
                    warn!(
                        event_type,
                        handler_id = handler.id,
                        error_class = "bounded-handler-queue-full",
                        "Observational handler lagged; skipped its event copy"
                    );
                }
                Err(mpsc::error::TrySendError::Closed(envelope)) => {
                    self.observational_handler_metrics
                        .dropped_closed
                        .fetch_add(1, Ordering::Relaxed);
                    envelope.accounting.rejected();
                }
            }
        }
        Ok(())
    }

    /// Retained in-process event-bus queues for leak diagnostics.
    pub fn event_bus_diagnostic_snapshot(&self) -> serde_json::Value {
        let metrics = &self.observational_handler_metrics;
        let handler_snapshot = serde_json::json!({
            "registered": self.handlers.iter().map(|entry| entry.value().len()).sum::<usize>(),
            "enqueued_total": metrics.enqueued_total.load(Ordering::Relaxed),
            "queued_current": metrics.queued_current.load(Ordering::Relaxed),
            "queued_max": metrics.queued_max.load(Ordering::Relaxed),
            "in_flight_current": metrics.in_flight_current.load(Ordering::Relaxed),
            "in_flight_max": metrics.in_flight_max.load(Ordering::Relaxed),
            "delivered_total": metrics.delivered_total.load(Ordering::Relaxed),
            "handler_failures": metrics.handler_failures.load(Ordering::Relaxed),
            "dropped_full": metrics.dropped_full.load(Ordering::Relaxed),
            "dropped_closed": metrics.dropped_closed.load(Ordering::Relaxed),
            "dropped_shutdown_queued": metrics.dropped_shutdown_queued.load(Ordering::Relaxed),
            "dropped_shutdown_in_flight": metrics.dropped_shutdown_in_flight.load(Ordering::Relaxed),
            "shutdown_aborted_workers": metrics.shutdown_aborted_workers.load(Ordering::Relaxed),
            "lifecycle_state": self.lifecycle_state.load(Ordering::Acquire),
        });
        let mut snapshot = self.event_bus.diagnostic_snapshot();
        if let Some(snapshot) = snapshot.as_object_mut() {
            snapshot.insert("observational_handlers".to_string(), handler_snapshot);
        }
        snapshot
    }

    /// Subscribe to events of a specific type
    pub async fn subscribe(
        &self,
        event_type: EventTypeId,
    ) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>> {
        debug!("Subscribing to event type: {}", event_type);

        let admission = self.handler_admission_gate.read().await;
        if self.lifecycle_state.load(Ordering::Acquire) != COORDINATOR_OPEN {
            return Err(anyhow::anyhow!("Event coordinator is draining or closed"));
        }

        // Subscribe through event bus
        let receiver = self.event_bus.subscribe(event_type).await?;

        // Track subscription
        let subscription = EventSubscription {
            event_type,
            subscribed_at: std::time::Instant::now(),
        };

        self.subscriptions
            .write()
            .await
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(subscription);
        drop(admission);

        Ok(receiver)
    }

    /// Subscribe with plane filtering
    pub async fn subscribe_with_plane_filter(
        &self,
        event_type: EventTypeId,
        plane_type: PlaneType,
    ) -> Result<mpsc::Receiver<Arc<dyn CrossCrateEvent>>> {
        // For monolithic mode, plane filtering is informational
        // In distributed mode, this would filter by plane
        debug!(
            "Subscribing to event type: {} for plane: {:?}",
            event_type, plane_type
        );

        self.subscribe(event_type).await
    }

    /// Route an event through the plane router
    pub async fn route_event(
        &self,
        source_plane: PlaneType,
        _event: Arc<dyn CrossCrateEvent>,
    ) -> Result<()> {
        debug!("Routing event from plane: {:?}", source_plane);

        // TODO: Add plane-aware event routing
        debug!(
            "Routing event from plane: {:?} to target plane",
            source_plane
        );

        Ok(())
    }

    /// Get statistics about the event coordinator
    pub async fn stats(&self) -> EventCoordinatorStats {
        let handler_count: usize = self.handlers.iter().map(|entry| entry.value().len()).sum();
        let subscription_count: usize = self
            .subscriptions
            .read()
            .await
            .values()
            .map(|v| v.len())
            .sum();
        let task_stats = self.task_manager.stats().await;

        EventCoordinatorStats {
            deployment_config: self.config.deployment.clone(),
            service_name: self.config.service_name.clone(),
            registered_handlers: handler_count,
            active_subscriptions: subscription_count,
            active_tasks: task_stats.active_tasks,
            total_events_processed: 0, // TODO: Add metrics
        }
    }

    /// Graceful shutdown
    pub async fn shutdown(&self) -> Result<()> {
        info!("Starting global event coordinator shutdown");

        // Admission and lifecycle transition are one atomic boundary. Every
        // publisher/registrar that entered while OPEN finishes before this
        // write lock is acquired; all later operations observe DRAINING.
        let admission = self.handler_admission_gate.write().await;
        if self
            .lifecycle_state
            .compare_exchange(
                COORDINATOR_OPEN,
                COORDINATOR_DRAINING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            let registered_handlers: Vec<_> = self
                .handlers
                .iter()
                .flat_map(|entry| entry.value().clone())
                .collect();
            self.handlers.clear();
            self.observational_publish_gates.clear();

            let handler_metrics = Arc::clone(&self.observational_handler_metrics);
            let event_bus = Arc::clone(&self.event_bus);
            let task_manager = Arc::clone(&self.task_manager);
            let subscriptions = Arc::clone(&self.subscriptions);
            let lifecycle_state = Arc::clone(&self.lifecycle_state);
            let lifecycle_closed = Arc::clone(&self.lifecycle_closed);
            let shutdown_error = Arc::clone(&self.shutdown_error);

            // The complete shutdown is owned by this supervisor, not by the
            // caller. Cancelling the first caller therefore cannot make a
            // second caller return before handler work and shared resources
            // have reached the same terminal state.
            tokio::spawn(async move {
                for handler in &registered_handlers {
                    handler
                        .observational_sender
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                }

                let mut tasks = Vec::with_capacity(registered_handlers.len());
                for handler in &registered_handlers {
                    if let Some(task) = handler.observational_task.lock().await.take() {
                        tasks.push(task);
                    }
                }

                let drain = async {
                    for task in &mut tasks {
                        let _ = task.await;
                    }
                };
                if tokio::time::timeout(OBSERVATIONAL_HANDLER_SHUTDOWN_TIMEOUT, drain)
                    .await
                    .is_err()
                {
                    let unfinished: Vec<_> = tasks
                        .iter()
                        .enumerate()
                        .filter_map(|(index, task)| (!task.is_finished()).then_some(index))
                        .collect();
                    handler_metrics
                        .shutdown_aborted_workers
                        .fetch_add(unfinished.len() as u64, Ordering::Relaxed);
                    for &index in &unfinished {
                        tasks[index].abort();
                    }
                    for index in unfinished {
                        let _ = (&mut tasks[index]).await;
                    }
                }

                let mut errors = Vec::new();
                if let Err(error) = event_bus.shutdown().await {
                    errors.push(format!("event bus shutdown failed: {error:#}"));
                }
                if let Err(error) = task_manager.shutdown_all().await {
                    errors.push(format!("task manager shutdown failed: {error:#}"));
                }
                subscriptions.write().await.clear();

                let error = (!errors.is_empty()).then(|| errors.join("; "));
                *shutdown_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = error;
                lifecycle_state.store(COORDINATOR_CLOSED, Ordering::Release);
                lifecycle_closed.notify_waiters();
                info!("Global event coordinator shutdown complete");
            });
        }
        drop(admission);

        // Register the waiter before inspecting CLOSED so notify_waiters
        // cannot be lost between the state check and the await.
        loop {
            let closed = self.lifecycle_closed.notified();
            tokio::pin!(closed);
            closed.as_mut().enable();
            if self.lifecycle_state.load(Ordering::Acquire) == COORDINATOR_CLOSED {
                break;
            }
            closed.await;
        }

        let error = self
            .shutdown_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match error {
            Some(error) => Err(anyhow::anyhow!(error)),
            None => Ok(()),
        }
    }
}

/// Event type registry for managing cross-crate event types
pub struct EventTypeRegistry {
    types: DashMap<EventTypeId, EventTypeInfo>,
}

impl EventTypeRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            types: DashMap::new(),
        };

        // Register built-in cross-crate event types
        registry.register_builtin_types();

        registry
    }

    /// Register a new event type
    pub fn register_event_type(&self, event_type: EventTypeId, info: EventTypeInfo) {
        self.types.insert(event_type, info);
        debug!("Registered event type: {}", event_type);
    }

    /// Get event type information
    pub fn get_type_info(&self, event_type: EventTypeId) -> Option<EventTypeInfo> {
        self.types
            .get(event_type)
            .map(|entry| entry.value().clone())
    }

    /// Register built-in cross-crate event types
    fn register_builtin_types(&mut self) {
        // Register core cross-crate event types
        self.register_event_type(
            "session_to_dialog",
            EventTypeInfo {
                event_type: "session_to_dialog",
                source_plane: PlaneType::Signaling,
                target_plane: PlaneType::Signaling,
                priority: EventPriority::High,
                description: "Events from session-core to dialog-core".to_string(),
            },
        );

        self.register_event_type(
            "dialog_to_session",
            EventTypeInfo {
                event_type: "dialog_to_session",
                source_plane: PlaneType::Signaling,
                target_plane: PlaneType::Signaling,
                priority: EventPriority::High,
                description: "Events from dialog-core to session-core".to_string(),
            },
        );

        self.register_event_type(
            "session_to_media",
            EventTypeInfo {
                event_type: "session_to_media",
                source_plane: PlaneType::Signaling,
                target_plane: PlaneType::Media,
                priority: EventPriority::High,
                description: "Events from session-core to media-core".to_string(),
            },
        );

        self.register_event_type(
            "media_to_session",
            EventTypeInfo {
                event_type: "media_to_session",
                source_plane: PlaneType::Media,
                target_plane: PlaneType::Signaling,
                priority: EventPriority::Normal,
                description: "Events from media-core to session-core".to_string(),
            },
        );

        // Orchestration-plane events: one entry per fine-grained variant so
        // the coordinator allocates a separate broadcast channel per variant.
        for &event_type in OrchestrationCrossCrateEvent::ALL_EVENT_TYPES {
            self.register_event_type(
                event_type,
                EventTypeInfo {
                    event_type,
                    source_plane: PlaneType::Signaling,
                    target_plane: PlaneType::Signaling,
                    priority: EventPriority::Normal,
                    description: format!("Orchestration-plane event: {event_type}"),
                },
            );
        }

        // rvoip-core spine events: same per-variant pattern as orchestration.
        for &event_type in RvoipCoreCrossCrateEvent::ALL_EVENT_TYPES {
            self.register_event_type(
                event_type,
                EventTypeInfo {
                    event_type,
                    source_plane: PlaneType::Signaling,
                    target_plane: PlaneType::Signaling,
                    priority: EventPriority::Normal,
                    description: format!("rvoip-core spine event: {event_type}"),
                },
            );
        }
    }
}

/// Information about an event type
#[derive(Debug, Clone)]
pub struct EventTypeInfo {
    pub event_type: EventTypeId,
    pub source_plane: PlaneType,
    pub target_plane: PlaneType,
    pub priority: EventPriority,
    pub description: String,
}

/// Event subscription tracking
#[derive(Debug, Clone)]
struct EventSubscription {
    event_type: EventTypeId,
    subscribed_at: std::time::Instant,
}

/// Statistics about the event coordinator
#[derive(Debug, Clone)]
pub struct EventCoordinatorStats {
    pub deployment_config: DeploymentConfig,
    pub service_name: String,
    pub registered_handlers: usize,
    pub active_subscriptions: usize,
    pub active_tasks: usize,
    pub total_events_processed: u64,
}

/// Trait for cross-crate event handlers
#[async_trait]
pub trait CrossCrateEventHandler: Send + Sync {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Semaphore;

    #[derive(Debug)]
    struct TestLagEvent {
        id: u64,
    }

    impl CrossCrateEvent for TestLagEvent {
        fn event_type(&self) -> EventTypeId {
            "test_lag"
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

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    struct BlockingHandler {
        observed: mpsc::UnboundedSender<u64>,
        release: Arc<Semaphore>,
    }

    #[async_trait]
    impl CrossCrateEventHandler for BlockingHandler {
        async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
            let id = event
                .as_any()
                .downcast_ref::<TestLagEvent>()
                .expect("test event")
                .id;
            self.observed
                .send(id)
                .map_err(|_| anyhow::anyhow!("blocking observer closed"))?;
            self.release
                .acquire()
                .await
                .map_err(|_| anyhow::anyhow!("release semaphore closed"))?
                .forget();
            Ok(())
        }
    }

    struct RecordingHandler {
        observed: mpsc::UnboundedSender<u64>,
    }

    #[async_trait]
    impl CrossCrateEventHandler for RecordingHandler {
        async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
            let id = event
                .as_any()
                .downcast_ref::<TestLagEvent>()
                .expect("test event")
                .id;
            self.observed
                .send(id)
                .map_err(|_| anyhow::anyhow!("recording observer closed"))?;
            Ok(())
        }
    }

    async fn receive_test_id(receiver: &mut mpsc::Receiver<Arc<dyn CrossCrateEvent>>) -> u64 {
        tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("bus observation timed out")
            .expect("bus observation channel closed")
            .as_any()
            .downcast_ref::<TestLagEvent>()
            .expect("test event")
            .id
    }

    async fn receive_handler_id(receiver: &mut mpsc::UnboundedReceiver<u64>) -> u64 {
        tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("handler observation timed out")
            .expect("handler observation channel closed")
    }

    #[tokio::test]
    async fn authoritative_publish_requires_one_successful_direct_handler() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        assert!(!coordinator
            .publish_authoritative(Arc::new(TestLagEvent { id: 1 }))
            .await
            .unwrap());

        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
        coordinator
            .register_handler(
                "test_lag",
                RecordingHandler {
                    observed: observed_tx,
                },
            )
            .await
            .unwrap();
        assert!(coordinator
            .publish_authoritative(Arc::new(TestLagEvent { id: 2 }))
            .await
            .unwrap());
        assert_eq!(receive_handler_id(&mut observed_rx).await, 2);

        let (second_tx, _second_rx) = mpsc::unbounded_channel();
        coordinator
            .register_handler(
                "test_lag",
                RecordingHandler {
                    observed: second_tx,
                },
            )
            .await
            .unwrap();
        assert!(coordinator
            .publish_authoritative(Arc::new(TestLagEvent { id: 3 }))
            .await
            .is_err());
        assert!(matches!(
            observed_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn authoritative_handler_only_dispatch_never_reaches_bus_subscribers() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut bus = coordinator.subscribe("test_lag").await.unwrap();
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
        coordinator
            .register_handler(
                "test_lag",
                RecordingHandler {
                    observed: observed_tx,
                },
            )
            .await
            .unwrap();

        assert!(coordinator
            .dispatch_authoritative_handler(Arc::new(TestLagEvent { id: 41 }))
            .await
            .unwrap());
        assert_eq!(receive_handler_id(&mut observed_rx).await, 41);
        assert!(matches!(
            bus.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn broadcast_bridge_survives_lagged_and_delivers_later_events() {
        let bus = MonolithicEventBus {
            event_bus: Arc::new(EventSystem::new_static_fast_path(2)),
            task_manager: Arc::new(LayerTaskManager::new("test-lag")),
            broadcasters: Arc::new(DashMap::new()),
            subscriber_queues: Arc::new(DashMap::new()),
            next_subscriber_queue_id: AtomicU64::new(1),
            subscriber_dead_weak_slots_pruned: Arc::new(AtomicU64::new(0)),
            channel_capacity: 2,
        };

        let mut rx = bus.subscribe("test_lag").await.unwrap();
        for id in 0..20 {
            bus.publish(Arc::new(TestLagEvent { id })).await.unwrap();
        }

        let mut saw_late_event = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(event)) => {
                    if event
                        .as_any()
                        .downcast_ref::<TestLagEvent>()
                        .is_some_and(|event| event.id >= 18)
                    {
                        saw_late_event = true;
                        break;
                    }
                }
                _ => break,
            }
        }

        assert!(
            saw_late_event,
            "bridge should continue after RecvError::Lagged and deliver retained later events"
        );
    }

    #[tokio::test]
    async fn dropped_subscriber_prunes_its_weak_diagnostic_slot() {
        let bus = MonolithicEventBus {
            event_bus: Arc::new(EventSystem::new_static_fast_path(2)),
            task_manager: Arc::new(LayerTaskManager::new("test-prune")),
            broadcasters: Arc::new(DashMap::new()),
            subscriber_queues: Arc::new(DashMap::new()),
            next_subscriber_queue_id: AtomicU64::new(1),
            subscriber_dead_weak_slots_pruned: Arc::new(AtomicU64::new(0)),
            channel_capacity: 2,
        };

        let rx = bus.subscribe("test_lag").await.unwrap();
        assert_eq!(bus.diagnostic_snapshot()["subscriber_weak_slots_total"], 1);
        drop(rx);

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let snapshot = bus.diagnostic_snapshot();
                if snapshot["subscriber_weak_slots_total"] == 0
                    && snapshot["subscriber_dead_weak_slots_pruned_total"] == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("bridge removed its weak diagnostic slot");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn observational_publish_is_bus_first_and_isolates_lagging_handlers() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(2),
            )
            .await
            .unwrap(),
        );
        let mut bus = coordinator.subscribe("test_lag").await.unwrap();
        let (blocking_tx, mut blocking_rx) = mpsc::unbounded_channel();
        let (recording_tx, mut recording_rx) = mpsc::unbounded_channel();
        let release = Arc::new(Semaphore::new(0));
        coordinator
            .register_handler(
                "test_lag",
                BlockingHandler {
                    observed: blocking_tx,
                    release: Arc::clone(&release),
                },
            )
            .await
            .unwrap();
        coordinator
            .register_handler(
                "test_lag",
                RecordingHandler {
                    observed: recording_tx,
                },
            )
            .await
            .unwrap();

        coordinator
            .publish_observational(Arc::new(TestLagEvent { id: 1 }))
            .await
            .unwrap();
        assert_eq!(receive_test_id(&mut bus).await, 1);
        assert_eq!(receive_handler_id(&mut blocking_rx).await, 1);
        assert_eq!(receive_handler_id(&mut recording_rx).await, 1);

        // Handler A remains in flight. Its queue accepts ids 2 and 3, then
        // reports a local lag for id 4. Handler B and the authoritative bus
        // still receive every event exactly once and in order.
        for id in 2..=4 {
            coordinator
                .publish_observational(Arc::new(TestLagEvent { id }))
                .await
                .unwrap();
            assert_eq!(receive_handler_id(&mut recording_rx).await, id);
        }
        assert_eq!(
            vec![
                receive_test_id(&mut bus).await,
                receive_test_id(&mut bus).await,
                receive_test_id(&mut bus).await,
            ],
            vec![2, 3, 4]
        );
        let saturated = coordinator.event_bus_diagnostic_snapshot();
        assert_eq!(saturated["observational_handlers"]["dropped_full"], 1);
        assert_eq!(saturated["observational_handlers"]["in_flight_current"], 1);
        assert_eq!(saturated["observational_handlers"]["queued_current"], 2);

        release.add_permits(3);
        assert_eq!(receive_handler_id(&mut blocking_rx).await, 2);
        assert_eq!(receive_handler_id(&mut blocking_rx).await, 3);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let converged = coordinator.event_bus_diagnostic_snapshot();
                if converged["observational_handlers"]["queued_current"] == 0
                    && converged["observational_handlers"]["in_flight_current"] == 0
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("handler queues converged");
        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn observational_handler_shutdown_survives_caller_cancellation() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(2),
            )
            .await
            .unwrap(),
        );
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
        coordinator
            .register_handler(
                "test_lag",
                BlockingHandler {
                    observed: observed_tx,
                    release: Arc::new(Semaphore::new(0)),
                },
            )
            .await
            .unwrap();
        coordinator
            .publish_observational(Arc::new(TestLagEvent { id: 1 }))
            .await
            .unwrap();
        assert_eq!(receive_handler_id(&mut observed_rx).await, 1);

        // Hold the worker-handle mutex so the owned supervisor remains in
        // DRAINING deterministically even on a heavily loaded test runner.
        let registered = coordinator
            .handlers
            .get("test_lag")
            .expect("registered handler")[0]
            .clone();
        let task_handle = registered.observational_task.lock().await;

        let shutdown_coordinator = Arc::clone(&coordinator);
        let first_shutdown = tokio::spawn(async move { shutdown_coordinator.shutdown().await });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while coordinator.lifecycle_state.load(Ordering::Acquire) == COORDINATOR_OPEN {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown entered the shared draining state");
        assert_eq!(
            coordinator.lifecycle_state.load(Ordering::Acquire),
            COORDINATOR_DRAINING,
            "hostile handler keeps the shared supervisor in draining"
        );

        let (late_tx, _late_rx) = mpsc::unbounded_channel();
        let late_registration = coordinator
            .register_handler("test_lag", RecordingHandler { observed: late_tx })
            .await;
        assert!(
            late_registration.is_err(),
            "registration must be rejected after draining starts"
        );

        first_shutdown.abort();
        let _ = first_shutdown.await;
        drop(task_handle);

        tokio::time::timeout(std::time::Duration::from_secs(1), coordinator.shutdown())
            .await
            .expect("second caller waited for the shared shutdown supervisor")
            .unwrap();

        let snapshot = coordinator.event_bus_diagnostic_snapshot();
        let handlers = &snapshot["observational_handlers"];
        assert_eq!(handlers["lifecycle_state"], COORDINATOR_CLOSED);
        assert_eq!(handlers["queued_current"], 0);
        assert_eq!(handlers["in_flight_current"], 0);
        assert_eq!(handlers["shutdown_aborted_workers"], 1);
        assert_eq!(handlers["dropped_shutdown_in_flight"], 1);

        let value = |name: &str| handlers[name].as_u64().expect("u64 diagnostic");
        assert_eq!(
            value("enqueued_total"),
            value("delivered_total")
                + value("handler_failures")
                + value("dropped_full")
                + value("dropped_closed")
                + value("dropped_shutdown_queued")
                + value("dropped_shutdown_in_flight"),
            "every accepted observational copy reaches one terminal outcome"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_observational_publish_has_one_bus_and_handler_order() {
        const EVENT_COUNT: u64 = 64;
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(128),
            )
            .await
            .unwrap(),
        );
        let mut bus = coordinator.subscribe("test_lag").await.unwrap();
        let (first_tx, mut first_rx) = mpsc::unbounded_channel();
        let (second_tx, mut second_rx) = mpsc::unbounded_channel();
        coordinator
            .register_handler("test_lag", RecordingHandler { observed: first_tx })
            .await
            .unwrap();
        coordinator
            .register_handler(
                "test_lag",
                RecordingHandler {
                    observed: second_tx,
                },
            )
            .await
            .unwrap();

        let mut publishers = tokio::task::JoinSet::new();
        for id in 0..EVENT_COUNT {
            let coordinator = Arc::clone(&coordinator);
            publishers.spawn(async move {
                coordinator
                    .publish_observational(Arc::new(TestLagEvent { id }))
                    .await
            });
        }
        while let Some(result) = publishers.join_next().await {
            result.expect("publisher task").expect("publish succeeds");
        }

        let mut bus_order = Vec::with_capacity(EVENT_COUNT as usize);
        let mut first_order = Vec::with_capacity(EVENT_COUNT as usize);
        let mut second_order = Vec::with_capacity(EVENT_COUNT as usize);
        for _ in 0..EVENT_COUNT {
            bus_order.push(receive_test_id(&mut bus).await);
            first_order.push(receive_handler_id(&mut first_rx).await);
            second_order.push(receive_handler_id(&mut second_rx).await);
        }
        assert_eq!(first_order, bus_order);
        assert_eq!(second_order, bus_order);

        let snapshot = coordinator.event_bus_diagnostic_snapshot();
        assert_eq!(snapshot["observational_handlers"]["dropped_full"], 0);
        assert_eq!(snapshot["observational_handlers"]["dropped_closed"], 0);
        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn synchronous_publish_releases_before_shutdown_without_map_guard_deadlock() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(4),
            )
            .await
            .unwrap(),
        );
        let (blocking_tx, mut blocking_rx) = mpsc::unbounded_channel();
        let (recording_tx, mut recording_rx) = mpsc::unbounded_channel();
        let release = Arc::new(Semaphore::new(0));
        coordinator
            .register_handler(
                "test_lag",
                BlockingHandler {
                    observed: blocking_tx,
                    release: Arc::clone(&release),
                },
            )
            .await
            .unwrap();
        coordinator
            .register_handler(
                "test_lag",
                RecordingHandler {
                    observed: recording_tx,
                },
            )
            .await
            .unwrap();

        let publish_coordinator = Arc::clone(&coordinator);
        let publish = tokio::spawn(async move {
            publish_coordinator
                .publish(Arc::new(TestLagEvent { id: 7 }))
                .await
        });
        assert_eq!(receive_handler_id(&mut blocking_rx).await, 7);
        assert!(matches!(
            recording_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let shutdown_coordinator = Arc::clone(&coordinator);
        let shutdown = tokio::spawn(async move { shutdown_coordinator.shutdown().await });
        release.add_permits(1);

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            publish
                .await
                .expect("publish task")
                .expect("publish succeeds");
            assert_eq!(receive_handler_id(&mut recording_rx).await, 7);
            shutdown
                .await
                .expect("shutdown task")
                .expect("shutdown succeeds");
        })
        .await
        .expect("publish and shutdown do not deadlock on the handler registry");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_rejects_new_subscriptions_and_closes_existing_bridges() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(4),
            )
            .await
            .unwrap(),
        );
        let mut existing = coordinator.subscribe("test_lag").await.unwrap();
        coordinator
            .publish_observational(Arc::new(TestLagEvent { id: 9 }))
            .await
            .unwrap();
        assert_eq!(receive_test_id(&mut existing).await, 9);

        let shutdown_coordinator = Arc::clone(&coordinator);
        let shutdown = tokio::spawn(async move { shutdown_coordinator.shutdown().await });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while coordinator.lifecycle_state.load(Ordering::Acquire) == COORDINATOR_OPEN {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown left the open state");

        assert!(
            coordinator.subscribe("test_lag").await.is_err(),
            "a subscription cannot enter after the shutdown boundary"
        );
        shutdown
            .await
            .expect("shutdown task")
            .expect("shutdown succeeds");

        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), existing.recv())
                .await
                .expect("existing receiver observes closure")
                .is_none()
        );
        let snapshot = coordinator.event_bus_diagnostic_snapshot();
        assert_eq!(snapshot["broadcast_retained_total"], 0);
        assert_eq!(snapshot["subscriber_queued_total"], 0);
        assert_eq!(snapshot["subscriber_weak_slots_total"], 0);
        let stats = coordinator.stats().await;
        assert_eq!(stats.active_subscriptions, 0);
        assert_eq!(stats.active_tasks, 0);
    }

    #[tokio::test]
    async fn test_monolithic_coordinator_creation() {
        let coordinator = crate::events::global_coordinator().await;

        assert!(matches!(
            coordinator.deployment_mode(),
            DeploymentConfig::Monolithic
        ));

        let stats = coordinator.stats().await;
        assert_eq!(stats.registered_handlers, 0);
        assert_eq!(stats.active_subscriptions, 0);

        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_event_type_registry() {
        let registry = EventTypeRegistry::new();

        let info = registry.get_type_info("session_to_dialog").unwrap();
        assert_eq!(info.event_type, "session_to_dialog");
        assert_eq!(info.source_plane, PlaneType::Signaling);
        assert_eq!(info.target_plane, PlaneType::Signaling);
    }
}
