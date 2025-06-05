//! Session Priority and Scheduling Management
//! 
//! This module provides sophisticated priority management for session coordination:
//! 
//! - Session priority classification and management
//! - Resource allocation based on priority levels
//! - Scheduling policies for session processing
//! - Quality of Service (QoS) enforcement
//! - Priority-based conflict resolution

use std::collections::{HashMap, BinaryHeap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, Instant};
use std::cmp::Ordering;
use dashmap::DashMap;
use tokio::sync::{RwLock, Notify};
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorContext};

/// Session priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SessionPriority {
    /// Emergency calls (911, etc.)
    Emergency = 100,
    
    /// Critical business calls
    Critical = 90,
    
    /// High priority calls
    High = 70,
    
    /// Normal priority calls
    Normal = 50,
    
    /// Low priority calls
    Low = 30,
    
    /// Background/maintenance calls
    Background = 10,
}

impl std::fmt::Display for SessionPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionPriority::Emergency => write!(f, "Emergency"),
            SessionPriority::Critical => write!(f, "Critical"),
            SessionPriority::High => write!(f, "High"),
            SessionPriority::Normal => write!(f, "Normal"),
            SessionPriority::Low => write!(f, "Low"),
            SessionPriority::Background => write!(f, "Background"),
        }
    }
}

impl Default for SessionPriority {
    fn default() -> Self {
        SessionPriority::Normal
    }
}

/// Priority class for grouping sessions
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PriorityClass {
    /// Real-time communications
    RealTime,
    
    /// Interactive communications
    Interactive,
    
    /// Bulk data transfer
    Bulk,
    
    /// Best effort
    BestEffort,
    
    /// Custom priority class
    Custom(String),
}

impl std::fmt::Display for PriorityClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PriorityClass::RealTime => write!(f, "RealTime"),
            PriorityClass::Interactive => write!(f, "Interactive"),
            PriorityClass::Bulk => write!(f, "Bulk"),
            PriorityClass::BestEffort => write!(f, "BestEffort"),
            PriorityClass::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

/// Scheduling policies for session processing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulingPolicy {
    /// First-In-First-Out
    FIFO,
    
    /// Priority-based scheduling
    Priority,
    
    /// Weighted Fair Queuing
    WFQ,
    
    /// Round Robin
    RoundRobin,
    
    /// Shortest Job First
    SJF,
    
    /// Earliest Deadline First
    EDF,
    
    /// Custom scheduling policy
    Custom(String),
}

impl std::fmt::Display for SchedulingPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulingPolicy::FIFO => write!(f, "FIFO"),
            SchedulingPolicy::Priority => write!(f, "Priority"),
            SchedulingPolicy::WFQ => write!(f, "WFQ"),
            SchedulingPolicy::RoundRobin => write!(f, "RoundRobin"),
            SchedulingPolicy::SJF => write!(f, "SJF"),
            SchedulingPolicy::EDF => write!(f, "EDF"),
            SchedulingPolicy::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

/// Resource limits for priority classes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum sessions in this priority class
    pub max_sessions: Option<usize>,
    
    /// Maximum bandwidth allocation (bytes/sec)
    pub max_bandwidth: Option<u64>,
    
    /// Maximum CPU percentage
    pub max_cpu_percent: Option<f32>,
    
    /// Maximum memory allocation (bytes)
    pub max_memory: Option<u64>,
    
    /// Minimum guaranteed resources
    pub guaranteed_resources: Option<GuaranteedResources>,
}

/// Guaranteed resource allocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuaranteedResources {
    /// Minimum bandwidth (bytes/sec)
    pub min_bandwidth: u64,
    
    /// Minimum CPU percentage
    pub min_cpu_percent: f32,
    
    /// Minimum memory (bytes)
    pub min_memory: u64,
}

/// Priority configuration for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPriorityInfo {
    /// Session ID
    pub session_id: SessionId,
    
    /// Priority level
    pub priority: SessionPriority,
    
    /// Priority class
    pub priority_class: PriorityClass,
    
    /// When priority was assigned
    pub assigned_at: SystemTime,
    
    /// Priority expiration (if any)
    pub expires_at: Option<SystemTime>,
    
    /// Resource allocation
    pub resource_allocation: Option<ResourceAllocation>,
    
    /// Custom priority metadata
    pub metadata: HashMap<String, String>,
}

/// Resource allocation for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAllocation {
    /// Allocated bandwidth (bytes/sec)
    pub bandwidth: u64,
    
    /// Allocated CPU percentage
    pub cpu_percent: f32,
    
    /// Allocated memory (bytes)
    pub memory: u64,
    
    /// Quality of Service level
    pub qos_level: QoSLevel,
}

/// Quality of Service levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QoSLevel {
    /// Best effort
    BestEffort,
    
    /// Assured forwarding
    AssuredForwarding,
    
    /// Expedited forwarding
    ExpeditedForwarding,
    
    /// Voice service
    Voice,
    
    /// Video service
    Video,
}

/// Scheduled task for session processing
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    /// Task ID
    pub task_id: String,
    
    /// Session ID
    pub session_id: SessionId,
    
    /// Priority of the task
    pub priority: SessionPriority,
    
    /// When the task was scheduled
    pub scheduled_at: Instant,
    
    /// Task deadline (if any)
    pub deadline: Option<Instant>,
    
    /// Estimated processing time
    pub estimated_duration: Option<Duration>,
    
    /// Task metadata
    pub metadata: HashMap<String, String>,
}

impl Eq for ScheduledTask {}

impl PartialEq for ScheduledTask {
    fn eq(&self, other: &Self) -> bool {
        self.task_id == other.task_id
    }
}

impl Ord for ScheduledTask {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority tasks should come first in the heap
        other.priority.cmp(&self.priority)
            .then_with(|| self.scheduled_at.cmp(&other.scheduled_at))
    }
}

impl PartialOrd for ScheduledTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Priority manager configuration
#[derive(Debug, Clone)]
pub struct PriorityManagerConfig {
    /// Default scheduling policy
    pub default_scheduling_policy: SchedulingPolicy,
    
    /// Whether to enforce resource limits
    pub enforce_resource_limits: bool,
    
    /// Whether to track detailed metrics
    pub track_metrics: bool,
    
    /// Priority reassessment interval
    pub reassessment_interval: Duration,
    
    /// Maximum queue size per priority
    pub max_queue_size: usize,
    
    /// Enable priority preemption
    pub enable_preemption: bool,
}

impl Default for PriorityManagerConfig {
    fn default() -> Self {
        Self {
            default_scheduling_policy: SchedulingPolicy::Priority,
            enforce_resource_limits: true,
            track_metrics: true,
            reassessment_interval: Duration::from_secs(60),
            max_queue_size: 1000,
            enable_preemption: false,
        }
    }
}

/// Priority management metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PriorityMetrics {
    /// Sessions by priority level
    pub sessions_by_priority: HashMap<String, u64>,
    
    /// Sessions by priority class
    pub sessions_by_class: HashMap<String, u64>,
    
    /// Average wait time by priority
    pub average_wait_time: HashMap<String, Duration>,
    
    /// Total preemptions
    pub total_preemptions: u64,
    
    /// Resource utilization by class
    pub resource_utilization: HashMap<String, f64>,
    
    /// Queue lengths by priority
    pub queue_lengths: HashMap<String, usize>,
    
    /// Priority changes
    pub priority_changes: u64,
}

/// Session priority manager
pub struct SessionPriorityManager {
    /// Priority information by session
    session_priorities: Arc<DashMap<SessionId, SessionPriorityInfo>>,
    
    /// Resource limits by priority class
    resource_limits: Arc<DashMap<PriorityClass, ResourceLimits>>,
    
    /// Scheduling queues by priority
    priority_queues: Arc<DashMap<SessionPriority, VecDeque<ScheduledTask>>>,
    
    /// Global task queue (priority heap)
    global_queue: Arc<RwLock<BinaryHeap<ScheduledTask>>>,
    
    /// Current resource usage by class
    resource_usage: Arc<DashMap<PriorityClass, ResourceUsage>>,
    
    /// Priority metrics
    metrics: Arc<RwLock<PriorityMetrics>>,
    
    /// Configuration
    config: PriorityManagerConfig,
    
    /// Notification for new tasks
    task_notify: Arc<Notify>,
}

/// Current resource usage tracking
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    /// Current session count
    pub session_count: usize,
    
    /// Current bandwidth usage (bytes/sec)
    pub bandwidth_usage: u64,
    
    /// Current CPU usage percentage
    pub cpu_usage: f32,
    
    /// Current memory usage (bytes)
    pub memory_usage: u64,
}

impl SessionPriorityManager {
    /// Create a new priority manager
    pub fn new(config: PriorityManagerConfig) -> Self {
        Self {
            session_priorities: Arc::new(DashMap::new()),
            resource_limits: Arc::new(DashMap::new()),
            priority_queues: Arc::new(DashMap::new()),
            global_queue: Arc::new(RwLock::new(BinaryHeap::new())),
            resource_usage: Arc::new(DashMap::new()),
            metrics: Arc::new(RwLock::new(PriorityMetrics::default())),
            config,
            task_notify: Arc::new(Notify::new()),
        }
    }
    
    /// Set priority for a session
    pub async fn set_session_priority(
        &self,
        session_id: SessionId,
        priority: SessionPriority,
        priority_class: PriorityClass,
        expires_at: Option<SystemTime>,
    ) -> Result<(), Error> {
        let priority_info = SessionPriorityInfo {
            session_id,
            priority,
            priority_class: priority_class.clone(),
            assigned_at: SystemTime::now(),
            expires_at,
            resource_allocation: None,
            metadata: HashMap::new(),
        };
        
        // Check if priority change is allowed
        if let Some(existing) = self.session_priorities.get(&session_id) {
            if existing.priority > priority && !self.config.enable_preemption {
                return Err(Error::InternalError(
                    format!("Cannot lower priority from {} to {} without preemption enabled", 
                        existing.priority, priority),
                    ErrorContext::default().with_message("Priority downgrade not allowed")
                ));
            }
        }
        
        // Allocate resources based on priority class
        let resource_allocation = self.allocate_resources(&priority_class, &priority).await?;
        
        let mut info = priority_info;
        info.resource_allocation = Some(resource_allocation);
        
        self.session_priorities.insert(session_id, info);
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.priority_changes += 1;
            *metrics.sessions_by_priority.entry(priority.to_string()).or_insert(0) += 1;
            *metrics.sessions_by_class.entry(priority_class.to_string()).or_insert(0) += 1;
        }
        
        info!("✅ Set priority {} (class: {}) for session {}", priority, priority_class, session_id);
        
        Ok(())
    }
    
    /// Schedule a task for a session
    pub async fn schedule_task(
        &self,
        task_id: String,
        session_id: SessionId,
        deadline: Option<Instant>,
        estimated_duration: Option<Duration>,
        metadata: HashMap<String, String>,
    ) -> Result<(), Error> {
        // Get session priority
        let priority = if let Some(priority_info) = self.session_priorities.get(&session_id) {
            priority_info.priority
        } else {
            SessionPriority::Normal // Default priority
        };
        
        let task = ScheduledTask {
            task_id: task_id.clone(),
            session_id,
            priority,
            scheduled_at: Instant::now(),
            deadline,
            estimated_duration,
            metadata,
        };
        
        // Add to appropriate queue based on scheduling policy
        match self.config.default_scheduling_policy {
            SchedulingPolicy::Priority => {
                let mut global_queue = self.global_queue.write().await;
                global_queue.push(task);
            },
            SchedulingPolicy::FIFO => {
                self.priority_queues
                    .entry(priority)
                    .or_insert_with(VecDeque::new)
                    .push_back(task);
            },
            _ => {
                // For other policies, use global queue for now
                let mut global_queue = self.global_queue.write().await;
                global_queue.push(task);
            }
        }
        
        // Update queue metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            *metrics.queue_lengths.entry(priority.to_string()).or_insert(0) += 1;
        }
        
        // Notify waiting processors
        self.task_notify.notify_one();
        
        debug!("📅 Scheduled task {} for session {} with priority {}", task_id, session_id, priority);
        
        Ok(())
    }
    
    /// Get the next task to process
    pub async fn get_next_task(&self) -> Option<ScheduledTask> {
        match self.config.default_scheduling_policy {
            SchedulingPolicy::Priority => {
                let mut global_queue = self.global_queue.write().await;
                global_queue.pop()
            },
            SchedulingPolicy::FIFO => {
                // Process highest priority queue first
                for priority in [SessionPriority::Emergency, SessionPriority::Critical, 
                               SessionPriority::High, SessionPriority::Normal, 
                               SessionPriority::Low, SessionPriority::Background] {
                    if let Some(mut queue) = self.priority_queues.get_mut(&priority) {
                        if let Some(task) = queue.pop_front() {
                            return Some(task);
                        }
                    }
                }
                None
            },
            _ => {
                // For other policies, use priority queue for now
                let mut global_queue = self.global_queue.write().await;
                global_queue.pop()
            }
        }
    }
    
    /// Wait for next task
    pub async fn wait_for_task(&self) -> Option<ScheduledTask> {
        loop {
            if let Some(task) = self.get_next_task().await {
                return Some(task);
            }
            
            // Wait for notification of new tasks
            self.task_notify.notified().await;
        }
    }
    
    /// Set resource limits for a priority class
    pub async fn set_resource_limits(
        &self,
        priority_class: PriorityClass,
        limits: ResourceLimits,
    ) -> Result<(), Error> {
        self.resource_limits.insert(priority_class.clone(), limits);
        
        info!("⚙️ Set resource limits for priority class {}", priority_class);
        Ok(())
    }
    
    /// Allocate resources for a session
    async fn allocate_resources(
        &self,
        priority_class: &PriorityClass,
        priority: &SessionPriority,
    ) -> Result<ResourceAllocation, Error> {
        // Get current usage
        let mut current_usage = self.resource_usage.entry(priority_class.clone())
            .or_insert_with(ResourceUsage::default);
        
        // Get limits for this class
        let limits = self.resource_limits.get(priority_class);
        
        // Calculate allocation based on priority and available resources
        let bandwidth = match priority {
            SessionPriority::Emergency => 10_000_000, // 10 Mbps
            SessionPriority::Critical => 5_000_000,   // 5 Mbps
            SessionPriority::High => 2_000_000,       // 2 Mbps
            SessionPriority::Normal => 1_000_000,     // 1 Mbps
            SessionPriority::Low => 500_000,          // 500 Kbps
            SessionPriority::Background => 100_000,   // 100 Kbps
        };
        
        let cpu_percent = match priority {
            SessionPriority::Emergency => 20.0,
            SessionPriority::Critical => 15.0,
            SessionPriority::High => 10.0,
            SessionPriority::Normal => 5.0,
            SessionPriority::Low => 2.0,
            SessionPriority::Background => 1.0,
        };
        
        let memory = match priority {
            SessionPriority::Emergency => 100_000_000, // 100 MB
            SessionPriority::Critical => 50_000_000,   // 50 MB
            SessionPriority::High => 25_000_000,       // 25 MB
            SessionPriority::Normal => 10_000_000,     // 10 MB
            SessionPriority::Low => 5_000_000,         // 5 MB
            SessionPriority::Background => 1_000_000,  // 1 MB
        };
        
        // Check limits if enforced
        if self.config.enforce_resource_limits {
            if let Some(ref limits) = limits {
                if let Some(max_sessions) = limits.max_sessions {
                    if current_usage.session_count >= max_sessions {
                        return Err(Error::InternalError(
                            format!("Session limit {} exceeded for priority class {}", 
                                max_sessions, priority_class),
                            ErrorContext::default().with_message("Resource limit exceeded")
                        ));
                    }
                }
                
                if let Some(max_bandwidth) = limits.max_bandwidth {
                    if current_usage.bandwidth_usage + bandwidth > max_bandwidth {
                        return Err(Error::InternalError(
                            format!("Bandwidth limit {} exceeded for priority class {}", 
                                max_bandwidth, priority_class),
                            ErrorContext::default().with_message("Bandwidth limit exceeded")
                        ));
                    }
                }
            }
        }
        
        // Update usage tracking
        current_usage.session_count += 1;
        current_usage.bandwidth_usage += bandwidth;
        current_usage.cpu_usage += cpu_percent;
        current_usage.memory_usage += memory;
        
        let qos_level = match priority {
            SessionPriority::Emergency | SessionPriority::Critical => QoSLevel::ExpeditedForwarding,
            SessionPriority::High => QoSLevel::AssuredForwarding,
            SessionPriority::Normal => QoSLevel::Voice,
            SessionPriority::Low | SessionPriority::Background => QoSLevel::BestEffort,
        };
        
        Ok(ResourceAllocation {
            bandwidth,
            cpu_percent,
            memory,
            qos_level,
        })
    }
    
    /// Release resources for a session
    pub async fn release_session_resources(&self, session_id: SessionId) -> Result<(), Error> {
        if let Some((_, priority_info)) = self.session_priorities.remove(&session_id) {
            if let Some(allocation) = priority_info.resource_allocation {
                // Update usage tracking
                if let Some(mut usage) = self.resource_usage.get_mut(&priority_info.priority_class) {
                    usage.session_count = usage.session_count.saturating_sub(1);
                    usage.bandwidth_usage = usage.bandwidth_usage.saturating_sub(allocation.bandwidth);
                    usage.cpu_usage = (usage.cpu_usage - allocation.cpu_percent).max(0.0);
                    usage.memory_usage = usage.memory_usage.saturating_sub(allocation.memory);
                }
                
                info!("🗑️ Released resources for session {} (priority: {})", 
                    session_id, priority_info.priority);
            }
        }
        
        Ok(())
    }
    
    /// Get session priority information
    pub async fn get_session_priority(&self, session_id: SessionId) -> Option<SessionPriorityInfo> {
        self.session_priorities.get(&session_id).map(|info| info.value().clone())
    }
    
    /// Check for expired priorities and clean them up
    pub async fn cleanup_expired_priorities(&self) -> Result<usize, Error> {
        let now = SystemTime::now();
        let mut cleanup_count = 0;
        let mut to_remove = Vec::new();
        
        for entry in self.session_priorities.iter() {
            if let Some(expires_at) = entry.value().expires_at {
                if now >= expires_at {
                    to_remove.push(entry.key().clone());
                }
            }
        }
        
        for session_id in to_remove {
            self.release_session_resources(session_id).await?;
            cleanup_count += 1;
        }
        
        if cleanup_count > 0 {
            info!("🧹 Cleaned up {} expired priority assignments", cleanup_count);
        }
        
        Ok(cleanup_count)
    }
    
    /// Get priority metrics
    pub async fn get_metrics(&self) -> PriorityMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Get resource usage for a priority class
    pub async fn get_resource_usage(&self, priority_class: &PriorityClass) -> Option<ResourceUsage> {
        self.resource_usage.get(priority_class).map(|usage| usage.clone())
    }
    
    /// Get queue status
    pub async fn get_queue_status(&self) -> HashMap<String, usize> {
        let mut status = HashMap::new();
        
        match self.config.default_scheduling_policy {
            SchedulingPolicy::Priority => {
                let global_queue = self.global_queue.read().await;
                status.insert("global".to_string(), global_queue.len());
            },
            SchedulingPolicy::FIFO => {
                for entry in self.priority_queues.iter() {
                    status.insert(entry.key().to_string(), entry.value().len());
                }
            },
            _ => {
                let global_queue = self.global_queue.read().await;
                status.insert("global".to_string(), global_queue.len());
            }
        }
        
        status
    }
} 