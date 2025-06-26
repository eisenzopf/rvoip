use std::collections::{HashMap, VecDeque, HashSet};
use tracing::{info, debug, warn};

use rvoip_session_core::SessionId;

use crate::error::{CallCenterError, Result};

/// Call queue manager
pub struct QueueManager {
    /// Active queues
    queues: HashMap<String, CallQueue>,
    /// Calls currently being assigned (to prevent re-queuing)
    calls_being_assigned: HashSet<SessionId>,
}

/// Individual call queue
#[derive(Debug)]
pub struct CallQueue {
    pub id: String,
    pub name: String,
    pub calls: VecDeque<QueuedCall>,
    pub max_size: usize,
    pub max_wait_time_seconds: u64,
}

/// Information about a queued call
#[derive(Debug, Clone)]
pub struct QueuedCall {
    pub session_id: SessionId,
    pub caller_id: String,
    pub priority: u8,
    pub queued_at: chrono::DateTime<chrono::Utc>,
    pub estimated_wait_time: Option<u64>,
    pub retry_count: u8,  // Number of times this call has been retried
}

/// Track when calls were marked as being assigned
#[derive(Debug)]
struct AssignmentTracker {
    session_id: SessionId,
    marked_at: std::time::Instant,
}

impl QueueManager {
    /// Create a new queue manager
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
            calls_being_assigned: HashSet::new(),
        }
    }
    
    /// Get all queue IDs
    pub fn get_queue_ids(&self) -> Vec<String> {
        self.queues.keys().cloned().collect()
    }
    
    /// Create a new queue
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
    pub fn is_call_queued(&self, queue_id: &str, session_id: &SessionId) -> bool {
        if let Some(queue) = self.queues.get(queue_id) {
            queue.calls.iter().any(|call| &call.session_id == session_id)
        } else {
            false
        }
    }
    
    /// Check if a call is being assigned
    pub fn is_call_being_assigned(&self, session_id: &SessionId) -> bool {
        self.calls_being_assigned.contains(session_id)
    }
    
    /// Enqueue a call
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
    pub fn mark_as_assigned(&mut self, session_id: &SessionId) {
        info!("🔒 Marking call {} as being assigned", session_id);
        self.calls_being_assigned.insert(session_id.clone());
    }
    
    /// Mark a call as no longer being assigned (on failure)
    pub fn mark_as_not_assigned(&mut self, session_id: &SessionId) {
        info!("🔓 Marking call {} as no longer being assigned", session_id);
        self.calls_being_assigned.remove(session_id);
    }
    
    /// Dequeue the next call for an agent
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