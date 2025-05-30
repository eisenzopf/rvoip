use std::collections::{HashMap, VecDeque};
use tracing::{info, debug};

use rvoip_session_core::SessionId;

use crate::error::{CallCenterError, Result};

/// Call queue manager
pub struct QueueManager {
    /// Active queues
    queues: HashMap<String, CallQueue>,
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
}

impl QueueManager {
    /// Create a new queue manager
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
        }
    }
    
    /// Create a new queue
    pub fn create_queue(&mut self, queue_id: String, name: String, max_size: usize) -> Result<()> {
        info!("📋 Creating queue: {} ({})", name, queue_id);
        
        let queue = CallQueue {
            id: queue_id.clone(),
            name,
            calls: VecDeque::new(),
            max_size,
            max_wait_time_seconds: 600, // 10 minutes default
        };
        
        self.queues.insert(queue_id, queue);
        Ok(())
    }
    
    /// Enqueue a call
    pub fn enqueue_call(&mut self, queue_id: &str, call: QueuedCall) -> Result<usize> {
        info!("📞 Enqueuing call {} to queue {}", call.session_id, queue_id);
        
        if let Some(queue) = self.queues.get_mut(queue_id) {
            if queue.calls.len() >= queue.max_size {
                return Err(CallCenterError::queue("Queue is full"));
            }
            
            // Insert based on priority (higher priority = lower number = front of queue)
            let insert_position = queue.calls.iter()
                .position(|existing| existing.priority > call.priority)
                .unwrap_or(queue.calls.len());
            
            queue.calls.insert(insert_position, call);
            
            debug!("📊 Queue {} now has {} calls", queue_id, queue.calls.len());
            Ok(insert_position)
        } else {
            Err(CallCenterError::not_found(format!("Queue not found: {}", queue_id)))
        }
    }
    
    /// Dequeue the next call for an agent
    pub fn dequeue_for_agent(&mut self, queue_id: &str) -> Result<Option<QueuedCall>> {
        if let Some(queue) = self.queues.get_mut(queue_id) {
            let call = queue.calls.pop_front();
            
            if let Some(ref call) = call {
                info!("📤 Dequeued call {} from queue {}", call.session_id, queue_id);
            }
            
            Ok(call)
        } else {
            Err(CallCenterError::not_found(format!("Queue not found: {}", queue_id)))
        }
    }
    
    /// Get queue statistics
    pub fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats> {
        if let Some(queue) = self.queues.get(queue_id) {
            let total_calls = queue.calls.len();
            let average_wait_time = if total_calls > 0 {
                let total_wait: i64 = queue.calls.iter()
                    .map(|call| chrono::Utc::now().signed_duration_since(call.queued_at).num_seconds())
                    .sum();
                total_wait / total_calls as i64
            } else {
                0
            };
            
            Ok(QueueStats {
                queue_id: queue_id.to_string(),
                total_calls,
                average_wait_time_seconds: average_wait_time as u64,
                longest_wait_time_seconds: 0, // TODO: Calculate
            })
        } else {
            Err(CallCenterError::not_found(format!("Queue not found: {}", queue_id)))
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