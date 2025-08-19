//! Task lifecycle management and spawn elimination
//!
//! This module provides TrackedTaskManager to eliminate untracked async task
//! proliferation and enable clean shutdown with proper task lifecycle management.

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::collections::HashMap;
use std::future::Future;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tokio::task::JoinHandle;
use uuid::Uuid;
use crate::errors::{Result, SessionError};

/// Handle for a tracked task that provides cancellation and monitoring
#[derive(Debug)]
pub struct TaskHandle {
    /// Unique identifier for this task
    pub id: Uuid,
    
    /// Handle to the underlying tokio task
    handle: JoinHandle<()>,
    
    /// Cancellation token for graceful shutdown
    cancel_token: CancellationToken,
    
    /// Task metadata
    metadata: TaskMetadata,
}

/// Metadata about a tracked task
#[derive(Debug, Clone)]
pub struct TaskMetadata {
    /// Human-readable name for the task
    pub name: String,
    
    /// Component that owns this task
    pub component: String,
    
    /// Task priority level
    pub priority: TaskPriority,
    
    /// When the task was created
    pub created_at: std::time::Instant,
}

/// Priority levels for task management
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskPriority {
    /// Low priority background tasks
    Low,
    /// Normal priority tasks
    Normal,
    /// High priority tasks (cleanup, shutdown)
    High,
    /// Critical system tasks (event loops, core services)
    Critical,
}

impl TaskHandle {
    /// Create a new task handle
    pub fn new(
        handle: JoinHandle<()>,
        cancel_token: CancellationToken,
        metadata: TaskMetadata,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            handle,
            cancel_token,
            metadata,
        }
    }
    
    /// Cancel the task gracefully
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
    
    /// Abort the task immediately
    pub fn abort(&self) {
        self.handle.abort();
    }
    
    /// Check if the task is finished
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
    
    /// Wait for the task to complete
    pub async fn join(self) -> Result<()> {
        match self.handle.await {
            Ok(()) => Ok(()),
            Err(e) if e.is_cancelled() => {
                tracing::debug!("Task {} was cancelled", self.metadata.name);
                Ok(())
            },
            Err(e) => Err(SessionError::internal(&format!("Task {} failed: {}", self.metadata.name, e))),
        }
    }
}

/// Statistics about active tasks
#[derive(Debug, Clone)]
pub struct TaskStats {
    /// Total number of active tasks
    pub total_active: usize,
    
    /// Tasks by priority level
    pub by_priority: HashMap<TaskPriority, usize>,
    
    /// Tasks by component
    pub by_component: HashMap<String, usize>,
    
    /// Average task age in seconds
    pub avg_age_seconds: f64,
}

/// Comprehensive task lifecycle manager with cancellation support
pub struct TrackedTaskManager {
    /// All active task handles
    tasks: Arc<Mutex<HashMap<Uuid, TaskHandle>>>,
    
    /// Global cancellation token for shutdown
    global_cancel_token: CancellationToken,
    
    /// Count of active tasks for quick access
    active_count: Arc<AtomicUsize>,
    
    /// Channel for task completion notifications
    completion_tx: mpsc::UnboundedSender<Uuid>,
    completion_rx: Arc<Mutex<mpsc::UnboundedReceiver<Uuid>>>,
    
    /// Component identifier for this manager
    component_name: String,
}

impl TrackedTaskManager {
    /// Create a new task manager for a component
    pub fn new(component_name: String) -> Self {
        let (completion_tx, completion_rx) = mpsc::unbounded_channel();
        
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            global_cancel_token: CancellationToken::new(),
            active_count: Arc::new(AtomicUsize::new(0)),
            completion_tx,
            completion_rx: Arc::new(Mutex::new(completion_rx)),
            component_name,
        }
    }
    
    /// Spawn a tracked task with automatic lifecycle management
    pub fn spawn_tracked<F>(&self, future: F, name: String, priority: TaskPriority) -> TaskHandle
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task_cancel_token = self.global_cancel_token.child_token();
        let task_cancel_token_clone = task_cancel_token.clone();
        let completion_tx = self.completion_tx.clone();
        let active_count = self.active_count.clone();
        let tasks = self.tasks.clone();
        
        let metadata = TaskMetadata {
            name: name.clone(),
            component: self.component_name.clone(),
            priority,
            created_at: std::time::Instant::now(),
        };
        
        let task_id = Uuid::new_v4();
        
        // Wrap the future with cancellation and cleanup
        let wrapped_future = async move {
            active_count.fetch_add(1, Ordering::Relaxed);
            
            tokio::select! {
                _ = future => {
                    tracing::debug!("Task '{}' completed normally", name);
                },
                _ = task_cancel_token_clone.cancelled() => {
                    tracing::debug!("Task '{}' was cancelled during shutdown", name);
                }
            }
            
            // Clean up
            active_count.fetch_sub(1, Ordering::Relaxed);
            let _ = completion_tx.send(task_id);
            
            // Remove from tracked tasks
            let mut tasks_guard = tasks.lock().await;
            tasks_guard.remove(&task_id);
        };
        
        let handle = tokio::spawn(wrapped_future);
        
        let task_handle = TaskHandle::new(handle, task_cancel_token, metadata);
        
        // Store the task handle
        tokio::spawn({
            let tasks = self.tasks.clone();
            let task_handle_clone = TaskHandle::new(
                tokio::spawn(async {}), // Dummy handle for storage
                task_handle.cancel_token.clone(),
                task_handle.metadata.clone(),
            );
            async move {
                let mut tasks_guard = tasks.lock().await;
                tasks_guard.insert(task_id, task_handle_clone);
            }
        });
        
        task_handle
    }
    
    /// Spawn a critical system task (like event loops)
    pub fn spawn_critical<F>(&self, future: F, name: String) -> TaskHandle
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.spawn_tracked(future, name, TaskPriority::Critical)
    }
    
    /// Spawn a high priority task (like cleanup operations)
    pub fn spawn_high_priority<F>(&self, future: F, name: String) -> TaskHandle
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.spawn_tracked(future, name, TaskPriority::High)
    }
    
    /// Spawn a normal priority task
    pub fn spawn_normal<F>(&self, future: F, name: String) -> TaskHandle
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.spawn_tracked(future, name, TaskPriority::Normal)
    }
    
    /// Get current task statistics
    pub async fn get_stats(&self) -> TaskStats {
        let tasks_guard = self.tasks.lock().await;
        let tasks: Vec<_> = tasks_guard.values().collect();
        
        let total_active = tasks.len();
        let mut by_priority = HashMap::new();
        let mut by_component = HashMap::new();
        let mut total_age_seconds = 0.0;
        
        for task in &tasks {
            // Count by priority
            *by_priority.entry(task.metadata.priority).or_insert(0) += 1;
            
            // Count by component
            *by_component.entry(task.metadata.component.clone()).or_insert(0) += 1;
            
            // Calculate age
            let age = task.metadata.created_at.elapsed().as_secs_f64();
            total_age_seconds += age;
        }
        
        let avg_age_seconds = if total_active > 0 {
            total_age_seconds / total_active as f64
        } else {
            0.0
        };
        
        TaskStats {
            total_active,
            by_priority,
            by_component,
            avg_age_seconds,
        }
    }
    
    /// Cancel all tasks gracefully with timeout
    pub async fn shutdown_gracefully(&self, timeout: std::time::Duration) -> Result<()> {
        tracing::info!("🛑 Starting graceful shutdown of {} tasks...", self.active_count.load(Ordering::Relaxed));
        
        // Signal all tasks to cancel
        self.global_cancel_token.cancel();
        
        // Wait for tasks to finish gracefully
        let start_time = std::time::Instant::now();
        
        while self.active_count.load(Ordering::Relaxed) > 0 && start_time.elapsed() < timeout {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        
        let remaining = self.active_count.load(Ordering::Relaxed);
        
        if remaining > 0 {
            tracing::warn!("⚠️ {} tasks did not finish gracefully, aborting...", remaining);
            
            // Force abort remaining tasks
            let tasks_guard = self.tasks.lock().await;
            for task in tasks_guard.values() {
                if !task.is_finished() {
                    task.abort();
                }
            }
            
            // Wait a bit more for aborts to take effect
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        
        tracing::info!("✅ Task manager shutdown complete");
        Ok(())
    }
    
    /// Force abort all tasks immediately
    pub async fn abort_all(&self) {
        tracing::warn!("🚨 Force aborting all tasks");
        
        let tasks_guard = self.tasks.lock().await;
        for task in tasks_guard.values() {
            task.abort();
        }
        
        self.active_count.store(0, Ordering::Relaxed);
    }
    
    /// Get the number of active tasks
    pub fn active_task_count(&self) -> usize {
        self.active_count.load(Ordering::Relaxed)
    }
    
    /// Check if all tasks have finished
    pub fn all_tasks_finished(&self) -> bool {
        self.active_task_count() == 0
    }
}

impl Drop for TrackedTaskManager {
    fn drop(&mut self) {
        let count = self.active_task_count();
        if count > 0 {
            tracing::warn!("TrackedTaskManager dropped with {} active tasks", count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    
    #[tokio::test]
    async fn test_task_manager_basic() {
        let manager = TrackedTaskManager::new("test".to_string());
        
        let completed = Arc::new(AtomicBool::new(false));
        let completed_clone = completed.clone();
        
        let handle = manager.spawn_normal(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            completed_clone.store(true, Ordering::Relaxed);
        }, "test_task".to_string());
        
        // Wait for task to complete
        handle.join().await.unwrap();
        
        assert!(completed.load(Ordering::Relaxed));
        assert_eq!(manager.active_task_count(), 0);
    }
    
    #[tokio::test]
    async fn test_task_manager_cancellation() {
        let manager = TrackedTaskManager::new("test".to_string());
        
        let started = Arc::new(AtomicBool::new(false));
        let started_clone = started.clone();
        
        let handle = manager.spawn_normal(async move {
            started_clone.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(10)).await; // Long running task
        }, "long_task".to_string());
        
        // Wait for task to start
        while !started.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        
        // Cancel the task
        handle.cancel();
        
        // Task should complete quickly due to cancellation
        handle.join().await.unwrap();
        
        assert_eq!(manager.active_task_count(), 0);
    }
    
    #[tokio::test]
    async fn test_graceful_shutdown() {
        let manager = TrackedTaskManager::new("test".to_string());
        
        // Spawn multiple tasks
        for i in 0..5 {
            manager.spawn_normal(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }, format!("task_{}", i));
        }
        
        assert!(manager.active_task_count() > 0);
        
        // Graceful shutdown should work
        manager.shutdown_gracefully(Duration::from_secs(1)).await.unwrap();
        
        assert_eq!(manager.active_task_count(), 0);
    }
}