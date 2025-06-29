//! # Queue Manager Implementation
//!
//! This module provides the core implementation of the call queue management system,
//! including the main QueueManager, individual CallQueue instances, and related
//! data structures for call queuing operations.

use std::collections::{HashMap, VecDeque, HashSet};
use tracing::{info, debug, warn};

use rvoip_session_core::SessionId;

use crate::error::{CallCenterError, Result};

/// # Call Queue Manager
///
/// The central coordinator for all call queuing operations in the call center.
/// Manages multiple named queues, coordinates call assignment, and provides
/// comprehensive queue statistics and monitoring.
///
/// ## Key Features
///
/// - **Multi-Queue Management**: Handles multiple named queues simultaneously
/// - **Priority-Based Queuing**: Automatic priority ordering within queues
/// - **Assignment Tracking**: Prevents race conditions during call assignment
/// - **Comprehensive Statistics**: Real-time queue performance metrics
/// - **Expiration Management**: Automatic cleanup of expired calls
///
/// ## Thread Safety
///
/// `QueueManager` is designed for single-threaded use within the call center engine.
/// For multi-threaded access, wrap in appropriate synchronization primitives.
///
/// ## Examples
///
/// ### Basic Usage
///
/// ```rust
/// use rvoip_call_engine::queue::{QueueManager, QueuedCall};
/// use rvoip_session_core::SessionId;
/// 
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut queue_manager = QueueManager::new();
/// 
/// // Create department queues
/// queue_manager.create_queue("support".to_string(), "Technical Support".to_string(), 50)?;
/// queue_manager.create_queue("sales".to_string(), "Sales Team".to_string(), 30)?;
/// 
/// // Queue a high-priority call
/// let urgent_call = QueuedCall {
///     session_id: SessionId::new(),
///     caller_id: "+1-555-0123".to_string(),
///     priority: 1,
///     queued_at: chrono::Utc::now(),
///     estimated_wait_time: Some(30),
///     retry_count: 0,
/// };
/// 
/// queue_manager.enqueue_call("support", urgent_call)?;
/// 
/// // Get next call for an agent
/// if let Some(call) = queue_manager.dequeue_for_agent("support")? {
///     println!("Assigning call to agent: {}", call.session_id);
/// }
/// # Ok(())
/// # }
/// ```
pub struct QueueManager {
    /// Active queues indexed by queue ID
    queues: HashMap<String, CallQueue>,
    /// Calls currently being assigned (to prevent re-queuing)
    calls_being_assigned: HashSet<SessionId>,
}

/// # Individual Call Queue
///
/// Represents a single call queue with its own configuration, capacity limits,
/// and call collection. Each queue maintains calls in priority order and
/// enforces queue-specific policies.
///
/// ## Queue Behavior
///
/// - Calls are automatically ordered by priority (lower number = higher priority)
/// - Queue capacity is enforced to prevent system overload
/// - Wait time limits trigger automatic call expiration
/// - Statistics are calculated in real-time
///
/// ## Examples
///
/// ```rust
/// use rvoip_call_engine::queue::CallQueue;
/// 
/// # fn example() {
/// let queue = CallQueue {
///     id: "support".to_string(),
///     name: "Technical Support".to_string(),
///     calls: std::collections::VecDeque::new(),
///     max_size: 50,
///     max_wait_time_seconds: 300, // 5 minutes
/// };
/// 
/// println!("Queue: {} (capacity: {})", queue.name, queue.max_size);
/// # }
/// ```
#[derive(Debug)]
pub struct CallQueue {
    /// Unique identifier for the queue
    pub id: String,
    /// Human-readable queue name
    pub name: String,
    /// Ordered collection of queued calls (priority-sorted)
    pub calls: VecDeque<QueuedCall>,
    /// Maximum number of calls allowed in this queue
    pub max_size: usize,
    /// Maximum wait time in seconds before call expiration
    pub max_wait_time_seconds: u64,
}

/// # Queued Call Information
///
/// Contains all information about a call waiting in a queue, including
/// priority, timing, and retry information for comprehensive call tracking.
///
/// ## Priority System
///
/// The priority field uses a numeric system where:
/// - **0**: Emergency/VIP calls (highest priority)
/// - **1-2**: High priority calls
/// - **3-5**: Normal priority calls
/// - **6-8**: Low priority calls
/// - **9+**: Lowest priority calls
///
/// ## Examples
///
/// ### Creating Different Priority Calls
///
/// ```rust
/// use rvoip_call_engine::queue::QueuedCall;
/// use rvoip_session_core::SessionId;
/// 
/// # fn example() {
/// // VIP customer call
/// let vip_call = QueuedCall {
///     session_id: SessionId::new(),
///     caller_id: "+1-555-VIP1".to_string(),
///     priority: 0,  // Highest priority
///     queued_at: chrono::Utc::now(),
///     estimated_wait_time: Some(15), // Expedited service
///     retry_count: 0,
/// };
/// 
/// // Normal support call
/// let normal_call = QueuedCall {
///     session_id: SessionId::new(),
///     caller_id: "+1-555-1234".to_string(),
///     priority: 5,  // Normal priority
///     queued_at: chrono::Utc::now(),
///     estimated_wait_time: Some(120), // 2 minutes estimated
///     retry_count: 0,
/// };
/// 
/// // Escalated call (retry)
/// let escalated_call = QueuedCall {
///     session_id: SessionId::new(),
///     caller_id: "+1-555-5678".to_string(),
///     priority: 2,  // Elevated due to retry
///     queued_at: chrono::Utc::now(),
///     estimated_wait_time: Some(60),
///     retry_count: 1, // This is a retry
/// };
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueuedCall {
    /// Unique session identifier for this call
    pub session_id: SessionId,
    /// Caller identification (phone number, customer ID, etc.)
    pub caller_id: String,
    /// Call priority (0 = highest, 255 = lowest)
    pub priority: u8,
    /// Timestamp when call was added to queue
    pub queued_at: chrono::DateTime<chrono::Utc>,
    /// Estimated wait time in seconds (if available)
    pub estimated_wait_time: Option<u64>,
    /// Number of times this call has been retried
    pub retry_count: u8,
}

/// Track when calls were marked as being assigned
#[derive(Debug)]
struct AssignmentTracker {
    session_id: SessionId,
    marked_at: std::time::Instant,
}

impl QueueManager {
    /// Create a new queue manager
    ///
    /// Initializes an empty queue manager ready to handle call queuing operations.
    /// No queues are created by default - use [`create_queue`] to add queues.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueueManager;
    /// 
    /// let queue_manager = QueueManager::new();
    /// assert_eq!(queue_manager.total_queued_calls(), 0);
    /// ```
    ///
    /// [`create_queue`]: QueueManager::create_queue
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
            calls_being_assigned: HashSet::new(),
        }
    }
    
    /// Get all queue IDs
    ///
    /// Returns a list of all queue identifiers currently managed by this
    /// queue manager. Useful for iterating over all queues or administrative
    /// operations.
    ///
    /// # Returns
    ///
    /// Vector of queue ID strings.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueueManager;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut queue_manager = QueueManager::new();
    /// queue_manager.create_queue("support".to_string(), "Support".to_string(), 50)?;
    /// queue_manager.create_queue("sales".to_string(), "Sales".to_string(), 30)?;
    /// 
    /// let queue_ids = queue_manager.get_queue_ids();
    /// assert_eq!(queue_ids.len(), 2);
    /// assert!(queue_ids.contains(&"support".to_string()));
    /// assert!(queue_ids.contains(&"sales".to_string()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_queue_ids(&self) -> Vec<String> {
        self.queues.keys().cloned().collect()
    }
    
    /// Create a new queue
    ///
    /// Adds a new named queue to the queue manager with the specified
    /// configuration. Each queue has its own capacity limits and policies.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Unique identifier for the queue
    /// * `name` - Human-readable name for the queue
    /// * `max_size` - Maximum number of calls allowed in this queue
    ///
    /// # Returns
    ///
    /// `Ok(())` if queue was created successfully, error if queue already exists.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueueManager;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut queue_manager = QueueManager::new();
    /// 
    /// // Create queues for different departments
    /// queue_manager.create_queue("support".to_string(), "Technical Support".to_string(), 50)?;
    /// queue_manager.create_queue("sales".to_string(), "Sales Team".to_string(), 30)?;
    /// queue_manager.create_queue("billing".to_string(), "Billing Department".to_string(), 20)?;
    /// 
    /// // Verify queues were created
    /// assert_eq!(queue_manager.get_queue_ids().len(), 3);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Production Notes
    ///
    /// - Choose appropriate `max_size` values based on expected call volume
    /// - Queue IDs should follow a consistent naming convention
    /// - Consider using department codes or service types as queue IDs
    pub fn create_queue(&mut self, queue_id: String, name: String, max_size: usize) -> Result<()> {
        info!("📋 Creating queue: {} ({})", name, queue_id);
        
        let queue = CallQueue {
            id: queue_id.clone(),
            name,
            calls: VecDeque::new(),
            max_size,
            max_wait_time_seconds: 3600, // 60 minutes for testing - effectively no timeout
        };
        
        self.queues.insert(queue_id, queue);
        Ok(())
    }
    
    /// Check if a call is already in the queue
    ///
    /// Determines whether a specific call (identified by session ID) is
    /// already present in the specified queue. Useful for preventing
    /// duplicate queueing operations.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Queue to check
    /// * `session_id` - Session ID of the call to check
    ///
    /// # Returns
    ///
    /// `true` if the call is already queued, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueueManager, QueuedCall};
    /// use rvoip_session_core::SessionId;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let mut queue_manager = QueueManager::new();
    /// # queue_manager.create_queue("support".to_string(), "Support".to_string(), 50)?;
    /// 
    /// let session_id = SessionId::new();
    /// 
    /// // Initially not queued
    /// assert!(!queue_manager.is_call_queued("support", &session_id));
    /// 
    /// // Add call to queue
    /// let call = QueuedCall {
    ///     session_id: session_id.clone(),
    ///     caller_id: "+1-555-1234".to_string(),
    ///     priority: 5,
    ///     queued_at: chrono::Utc::now(),
    ///     estimated_wait_time: Some(120),
    ///     retry_count: 0,
    /// };
    /// queue_manager.enqueue_call("support", call)?;
    /// 
    /// // Now it's queued
    /// assert!(queue_manager.is_call_queued("support", &session_id));
    /// # Ok(())
    /// # }
    /// ```
    pub fn is_call_queued(&self, queue_id: &str, session_id: &SessionId) -> bool {
        if let Some(queue) = self.queues.get(queue_id) {
            queue.calls.iter().any(|call| &call.session_id == session_id)
        } else {
            false
        }
    }
    
    /// Check if a call is being assigned
    ///
    /// Determines whether a call is currently in the process of being
    /// assigned to an agent. This prevents race conditions where a call
    /// might be assigned to multiple agents simultaneously.
    ///
    /// # Arguments
    ///
    /// * `session_id` - Session ID of the call to check
    ///
    /// # Returns
    ///
    /// `true` if the call is being assigned, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueueManager;
    /// use rvoip_session_core::SessionId;
    /// 
    /// # fn example() {
    /// let mut queue_manager = QueueManager::new();
    /// let session_id = SessionId::new();
    /// 
    /// // Initially not being assigned
    /// assert!(!queue_manager.is_call_being_assigned(&session_id));
    /// 
    /// // Mark as being assigned
    /// queue_manager.mark_as_assigned(&session_id);
    /// assert!(queue_manager.is_call_being_assigned(&session_id));
    /// 
    /// // Mark as no longer being assigned
    /// queue_manager.mark_as_not_assigned(&session_id);
    /// assert!(!queue_manager.is_call_being_assigned(&session_id));
    /// # }
    /// ```
    pub fn is_call_being_assigned(&self, session_id: &SessionId) -> bool {
        self.calls_being_assigned.contains(session_id)
    }
    
    /// Enqueue a call
    ///
    /// Adds a call to the specified queue with automatic priority ordering.
    /// Higher priority calls (lower priority numbers) are placed ahead of
    /// lower priority calls in the queue.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Target queue for the call
    /// * `call` - Call information including priority and metadata
    ///
    /// # Returns
    ///
    /// Returns the position in the queue where the call was inserted (0-based),
    /// or an error if the operation failed.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueueManager, QueuedCall};
    /// use rvoip_session_core::SessionId;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let mut queue_manager = QueueManager::new();
    /// # queue_manager.create_queue("support".to_string(), "Support".to_string(), 50)?;
    /// 
    /// // Add a normal priority call
    /// let normal_call = QueuedCall {
    ///     session_id: SessionId::new(),
    ///     caller_id: "+1-555-1234".to_string(),
    ///     priority: 5,
    ///     queued_at: chrono::Utc::now(),
    ///     estimated_wait_time: Some(120),
    ///     retry_count: 0,
    /// };
    /// let position1 = queue_manager.enqueue_call("support", normal_call)?;
    /// assert_eq!(position1, 0); // First call, position 0
    /// 
    /// // Add a high priority call (should jump to front)
    /// let urgent_call = QueuedCall {
    ///     session_id: SessionId::new(),
    ///     caller_id: "+1-555-VIP1".to_string(),
    ///     priority: 1, // Higher priority
    ///     queued_at: chrono::Utc::now(),
    ///     estimated_wait_time: Some(30),
    ///     retry_count: 0,
    /// };
    /// let position2 = queue_manager.enqueue_call("support", urgent_call)?;
    /// assert_eq!(position2, 0); // Jumped to front due to higher priority
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - `CallCenterError::Queue` - Queue is full or other queue-related error
    /// - `CallCenterError::NotFound` - Specified queue doesn't exist
    ///
    /// # Behavior
    ///
    /// - Prevents duplicate queueing of the same call
    /// - Prevents queueing calls that are currently being assigned
    /// - Enforces queue capacity limits
    /// - Maintains priority ordering automatically
    pub fn enqueue_call(&mut self, queue_id: &str, call: QueuedCall) -> Result<usize> {
        // Check for duplicates
        if self.is_call_queued(queue_id, &call.session_id) {
            warn!("📞 Call {} already in queue {}, not re-queuing", call.session_id, queue_id);
            return Ok(0);
        }
        
        // Check if call is being assigned
        if self.is_call_being_assigned(&call.session_id) {
            warn!("📞 Call {} is being assigned, not re-queuing", call.session_id);
            return Ok(0);
        }
        
        info!("📞 Enqueuing call {} to queue {} (priority: {}, retry: {})", 
              call.session_id, queue_id, call.priority, call.retry_count);
        
        if let Some(queue) = self.queues.get_mut(queue_id) {
            if queue.calls.len() >= queue.max_size {
                return Err(CallCenterError::queue("Queue is full"));
            }
            
            // Insert based on priority (higher priority = lower number = front of queue)
            let insert_position = queue.calls.iter()
                .position(|existing| existing.priority > call.priority)
                .unwrap_or(queue.calls.len());
            
            queue.calls.insert(insert_position, call);
            
            info!("📊 Queue {} size: {} calls", queue_id, queue.calls.len());
            Ok(insert_position)
        } else {
            Err(CallCenterError::not_found(format!("Queue not found: {}", queue_id)))
        }
    }
    
    /// Mark a call as being assigned (to prevent duplicate processing)
    ///
    /// Marks a call as currently being assigned to an agent to prevent
    /// race conditions where the same call might be processed by multiple
    /// threads or assigned to multiple agents.
    ///
    /// # Arguments
    ///
    /// * `session_id` - Session ID of the call being assigned
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueueManager;
    /// use rvoip_session_core::SessionId;
    /// 
    /// # fn example() {
    /// let mut queue_manager = QueueManager::new();
    /// let session_id = SessionId::new();
    /// 
    /// // Mark call as being assigned
    /// queue_manager.mark_as_assigned(&session_id);
    /// 
    /// // Verify it's marked
    /// assert!(queue_manager.is_call_being_assigned(&session_id));
    /// # }
    /// ```
    ///
    /// # Usage Pattern
    ///
    /// This method should be called when a call is dequeued and assignment
    /// begins. If assignment succeeds, the call remains dequeued. If assignment
    /// fails, call [`mark_as_not_assigned`] to allow re-queuing.
    ///
    /// [`mark_as_not_assigned`]: QueueManager::mark_as_not_assigned
    pub fn mark_as_assigned(&mut self, session_id: &SessionId) {
        info!("🔒 Marking call {} as being assigned", session_id);
        self.calls_being_assigned.insert(session_id.clone());
    }
    
    /// Mark a call as no longer being assigned (on failure)
    ///
    /// Removes the assignment lock on a call, allowing it to be processed
    /// again. This should be called when call assignment fails and the
    /// call should be made available for re-queuing or re-assignment.
    ///
    /// # Arguments
    ///
    /// * `session_id` - Session ID of the call to unlock
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::QueueManager;
    /// use rvoip_session_core::SessionId;
    /// 
    /// # fn example() {
    /// let mut queue_manager = QueueManager::new();
    /// let session_id = SessionId::new();
    /// 
    /// // Mark as assigned, then unmark due to failure
    /// queue_manager.mark_as_assigned(&session_id);
    /// queue_manager.mark_as_not_assigned(&session_id);
    /// 
    /// // Verify it's no longer marked
    /// assert!(!queue_manager.is_call_being_assigned(&session_id));
    /// # }
    /// ```
    ///
    /// # Usage Pattern
    ///
    /// ```rust,ignore
    /// // Dequeue call (automatically marked as assigned)
    /// if let Some(call) = queue_manager.dequeue_for_agent("support")? {
    ///     // Attempt to assign to agent
    ///     match assign_to_agent(&call).await {
    ///         Ok(_) => {
    ///             // Success - call remains dequeued and assigned
    ///             info!("Call assigned successfully");
    ///         }
    ///         Err(e) => {
    ///             // Failure - allow re-queuing
    ///             queue_manager.mark_as_not_assigned(&call.session_id);
    ///             warn!("Assignment failed: {}", e);
    ///         }
    ///     }
    /// }
    /// ```
    pub fn mark_as_not_assigned(&mut self, session_id: &SessionId) {
        info!("🔓 Marking call {} as no longer being assigned", session_id);
        self.calls_being_assigned.remove(session_id);
    }
    
    /// Dequeue the next call for an agent
    ///
    /// Removes and returns the highest priority call from the specified queue
    /// that is not currently being assigned. The returned call is automatically
    /// marked as being assigned to prevent duplicate processing.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Queue to dequeue from
    ///
    /// # Returns
    ///
    /// - `Ok(Some(QueuedCall))` - Call was successfully dequeued
    /// - `Ok(None)` - No calls available in the queue
    /// - `Err(...)` - Queue doesn't exist or other error
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::queue::{QueueManager, QueuedCall};
    /// use rvoip_session_core::SessionId;
    /// 
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let mut queue_manager = QueueManager::new();
    /// # queue_manager.create_queue("support".to_string(), "Support".to_string(), 50)?;
    /// 
    /// // Add some calls to the queue
    /// let call1 = QueuedCall {
    ///     session_id: SessionId::new(),
    ///     caller_id: "+1-555-1234".to_string(),
    ///     priority: 5, // Normal priority
    ///     queued_at: chrono::Utc::now(),
    ///     estimated_wait_time: Some(120),
    ///     retry_count: 0,
    /// };
    /// queue_manager.enqueue_call("support", call1)?;
    /// 
    /// let urgent_call = QueuedCall {
    ///     session_id: SessionId::new(),
    ///     caller_id: "+1-555-VIP1".to_string(),
    ///     priority: 1, // High priority
    ///     queued_at: chrono::Utc::now(),
    ///     estimated_wait_time: Some(30),
    ///     retry_count: 0,
    /// };
    /// queue_manager.enqueue_call("support", urgent_call)?;
    /// 
    /// // Dequeue - should get the high priority call first
    /// if let Some(call) = queue_manager.dequeue_for_agent("support")? {
    ///     assert_eq!(call.priority, 1); // High priority call came first
    ///     assert_eq!(call.caller_id, "+1-555-VIP1");
    ///     
    ///     // Call is now marked as being assigned
    ///     assert!(queue_manager.is_call_being_assigned(&call.session_id));
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Behavior
    ///
    /// - Returns highest priority call (lowest priority number)
    /// - Skips calls that are already being assigned
    /// - Automatically marks returned call as being assigned
    /// - Maintains queue statistics and logging
    pub fn dequeue_for_agent(&mut self, queue_id: &str) -> Result<Option<QueuedCall>> {
        if let Some(queue) = self.queues.get_mut(queue_id) {
            // Find the first call that isn't being assigned
            let mut index_to_remove = None;
            
            for (index, call) in queue.calls.iter().enumerate() {
                if !self.calls_being_assigned.contains(&call.session_id) {
                    index_to_remove = Some(index);
                    break;
                }
            }
            
            if let Some(index) = index_to_remove {
                let call = queue.calls.remove(index);
                if let Some(call) = call {
                    info!("📤 Dequeued call {} from queue {} (remaining: {})", 
                          call.session_id, queue_id, queue.calls.len());
                    // Mark as being assigned to prevent re-queuing during assignment
                    self.mark_as_assigned(&call.session_id);
                    return Ok(Some(call));
                }
            }
            
            Ok(None)
        } else {
            Err(CallCenterError::not_found(format!("Queue not found: {}", queue_id)))
        }
    }
    
    /// Remove expired calls from all queues
    pub fn remove_expired_calls(&mut self) -> Vec<SessionId> {
        let mut expired_calls = Vec::new();
        let now = chrono::Utc::now();
        
        for (queue_id, queue) in &mut self.queues {
            queue.calls.retain(|call| {
                let wait_time = now.signed_duration_since(call.queued_at).num_seconds();
                if wait_time > queue.max_wait_time_seconds as i64 {
                    warn!("⏰ Removing expired call {} from queue {} (waited {} seconds)", 
                          call.session_id, queue_id, wait_time);
                    expired_calls.push(call.session_id.clone());
                    false
                } else {
                    true
                }
            });
        }
        
        expired_calls
    }
    
    /// Get total number of queued calls across all queues
    pub fn total_queued_calls(&self) -> usize {
        self.queues.values().map(|q| q.calls.len()).sum()
    }
    
    /// Get queue statistics
    pub fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats> {
        if let Some(queue) = self.queues.get(queue_id) {
            let total_calls = queue.calls.len();
            let now = chrono::Utc::now();
            
            let (average_wait_time, longest_wait_time) = if total_calls > 0 {
                let wait_times: Vec<i64> = queue.calls.iter()
                    .map(|call| now.signed_duration_since(call.queued_at).num_seconds())
                    .collect();
                
                let total_wait: i64 = wait_times.iter().sum();
                let average = total_wait / total_calls as i64;
                let longest = wait_times.iter().max().cloned().unwrap_or(0);
                
                (average, longest)
            } else {
                (0, 0)
            };
            
            Ok(QueueStats {
                queue_id: queue_id.to_string(),
                total_calls,
                average_wait_time_seconds: average_wait_time as u64,
                longest_wait_time_seconds: longest_wait_time as u64,
            })
        } else {
            Err(CallCenterError::not_found(format!("Queue not found: {}", queue_id)))
        }
    }
    
    /// Clean up calls that have been stuck in "being assigned" state
    pub fn cleanup_stuck_assignments(&mut self, timeout_seconds: u64) -> Vec<SessionId> {
        // For now, we'll clear all assignments older than timeout
        // In a real implementation, we'd track timestamps
        let stuck_calls: Vec<SessionId> = self.calls_being_assigned.iter().cloned().collect();
        
        if !stuck_calls.is_empty() {
            warn!("🧹 Clearing {} stuck 'being assigned' calls", stuck_calls.len());
            self.calls_being_assigned.clear();
        }
        
        stuck_calls
    }
    
    /// Force remove a call from being assigned state
    pub fn force_unmark_assigned(&mut self, session_id: &SessionId) -> bool {
        if self.calls_being_assigned.remove(session_id) {
            warn!("🔓 Force unmarked call {} from being assigned", session_id);
            true
        } else {
            false
        }
    }
}

impl Default for QueueManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Queue statistics
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub queue_id: String,
    pub total_calls: usize,
    pub average_wait_time_seconds: u64,
    pub longest_wait_time_seconds: u64,
} 