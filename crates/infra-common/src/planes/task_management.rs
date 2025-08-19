//! Task lifecycle management for federated planes
//!
//! Provides tracked task spawning with cancellation support to prevent
//! the shutdown hangs that have been plaguing the system.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use anyhow::Result;
use tracing::{debug, warn, error};

/// Task priority for scheduling
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Critical,  // Must complete even during shutdown
    High,      // Important tasks
    Normal,    // Regular tasks
    Low,       // Can be cancelled early during shutdown
}

/// Handle to a tracked task
#[derive(Debug)]
pub struct TaskHandle {
    id: usize,
    name: String,
    handle: JoinHandle<()>,
    priority: TaskPriority,
    started_at: Instant,
}

impl TaskHandle {
    /// Create a new task handle
    fn new(id: usize, name: String, handle: JoinHandle<()>, priority: TaskPriority) -> Self {
        Self {
            id,
            name,
            handle,
            priority,
            started_at: Instant::now(),
        }
    }
    
    /// Get task ID
    pub fn id(&self) -> usize {
        self.id
    }
    
    /// Get task name
    pub fn name(&self) -> &str {
        &self.name
    }
    
    /// Check if task is finished
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
    
    /// Abort the task
    pub fn abort(&self) {
        self.handle.abort();
    }
    
    /// Get task runtime
    pub fn runtime(&self) -> Duration {
        self.started_at.elapsed()
    }
}

/// Layer task manager for tracking and managing async tasks
/// 
/// This is the solution to the shutdown hanging problem - all tasks
/// must be tracked and cancellable.
pub struct LayerTaskManager {
    /// Unique task ID counter
    next_task_id: AtomicUsize,
    
    /// All tracked task handles
    tasks: Arc<Mutex<Vec<TaskHandle>>>,
    
    /// Cancellation token for graceful shutdown
    cancel_token: CancellationToken,
    
    /// Number of active tasks
    active_count: AtomicUsize,
    
    /// Layer name for logging
    layer_name: String,
    
    /// Maximum tasks allowed
    max_tasks: usize,
    
    /// Shutdown timeout
    shutdown_timeout: Duration,
}

impl LayerTaskManager {
    /// Create a new task manager for a layer
    pub fn new(layer_name: impl Into<String>) -> Self {
        Self {
            next_task_id: AtomicUsize::new(0),
            tasks: Arc::new(Mutex::new(Vec::new())),
            cancel_token: CancellationToken::new(),
            active_count: AtomicUsize::new(0),
            layer_name: layer_name.into(),
            max_tasks: 1000,
            shutdown_timeout: Duration::from_secs(5),
        }
    }
    
    /// Create with custom configuration
    pub fn with_config(
        layer_name: impl Into<String>,
        max_tasks: usize,
        shutdown_timeout: Duration,
    ) -> Self {
        Self {
            next_task_id: AtomicUsize::new(0),
            tasks: Arc::new(Mutex::new(Vec::new())),
            cancel_token: CancellationToken::new(),
            active_count: AtomicUsize::new(0),
            layer_name: layer_name.into(),
            max_tasks,
            shutdown_timeout,
        }
    }
    
    /// Spawn a tracked task
    pub async fn spawn_tracked<F>(
        &self,
        name: impl Into<String>,
        priority: TaskPriority,
        future: F,
    ) -> Result<usize>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task_name = name.into();
        let task_id = self.next_task_id.fetch_add(1, Ordering::SeqCst);
        
        // Check if we've hit the task limit
        let active = self.active_count.load(Ordering::Relaxed);
        if active >= self.max_tasks {
            anyhow::bail!(
                "Task limit reached for {}: {} active tasks",
                self.layer_name,
                active
            );
        }
        
        // Wrap the future with cancellation and tracking
        let cancel_token = self.cancel_token.clone();
        let layer_name = self.layer_name.clone();
        let task_name_clone = task_name.clone();
        
        // Clone the atomic counter to move into the future
        let active_count_clone = Arc::new(AtomicUsize::new(0));
        
        let wrapped_future = async move {
            active_count_clone.fetch_add(1, Ordering::Relaxed);
            debug!(
                "Task started: {} [{}] in layer {}",
                task_name_clone, task_id, layer_name
            );
            
            tokio::select! {
                _ = future => {
                    debug!(
                        "Task completed: {} [{}] in layer {}",
                        task_name_clone, task_id, layer_name
                    );
                }
                _ = cancel_token.cancelled() => {
                    debug!(
                        "Task cancelled: {} [{}] in layer {}",
                        task_name_clone, task_id, layer_name
                    );
                }
            }
            
            active_count_clone.fetch_sub(1, Ordering::Relaxed);
        };
        
        // Update the main counter
        self.active_count.fetch_add(1, Ordering::Relaxed);
        
        let handle = tokio::spawn(wrapped_future);
        
        // Store the task handle
        let task_handle = TaskHandle::new(task_id, task_name, handle, priority);
        self.tasks.lock().await.push(task_handle);
        
        Ok(task_id)
    }
    
    /// Spawn a task with timeout
    pub async fn spawn_with_timeout<F>(
        &self,
        name: impl Into<String>,
        priority: TaskPriority,
        timeout: Duration,
        future: F,
    ) -> Result<usize>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task_name = name.into();
        let task_name_clone = task_name.clone();
        let timeout_future = async move {
            match tokio::time::timeout(timeout, future).await {
                Ok(()) => {}
                Err(_) => {
                    warn!("Task {} timed out after {:?}", task_name_clone, timeout);
                }
            }
        };
        
        self.spawn_tracked(task_name, priority, timeout_future).await
    }
    
    /// Get number of active tasks
    pub fn active_task_count(&self) -> usize {
        self.active_count.load(Ordering::Relaxed)
    }
    
    /// Cancel all tasks gracefully
    pub fn cancel_all(&self) {
        debug!(
            "Cancelling all tasks in layer {}",
            self.layer_name
        );
        self.cancel_token.cancel();
    }
    
    /// Shutdown all tasks with timeout
    pub async fn shutdown_all(&self) -> Result<()> {
        let start = Instant::now();
        
        debug!(
            "Starting shutdown for layer {} with {} active tasks",
            self.layer_name,
            self.active_task_count()
        );
        
        // First, cancel all tasks gracefully
        self.cancel_all();
        
        // Wait for graceful shutdown with timeout
        let shutdown_result = tokio::time::timeout(
            self.shutdown_timeout,
            self.wait_for_completion(),
        ).await;
        
        match shutdown_result {
            Ok(()) => {
                debug!(
                    "Layer {} shutdown completed gracefully in {:?}",
                    self.layer_name,
                    start.elapsed()
                );
            }
            Err(_) => {
                warn!(
                    "Layer {} shutdown timed out after {:?}, forcing abort",
                    self.layer_name,
                    self.shutdown_timeout
                );
                
                // Force abort remaining tasks
                self.abort_all().await;
            }
        }
        
        // Clean up task handles
        self.tasks.lock().await.clear();
        
        Ok(())
    }
    
    /// Wait for all tasks to complete
    async fn wait_for_completion(&self) {
        while self.active_count.load(Ordering::Relaxed) > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    
    /// Force abort all tasks
    async fn abort_all(&self) {
        let mut tasks = self.tasks.lock().await;
        
        // Abort tasks in reverse priority order (low priority first)
        tasks.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        for task in tasks.iter() {
            if !task.is_finished() {
                warn!(
                    "Force aborting task: {} [{}] after {:?}",
                    task.name,
                    task.id,
                    task.runtime()
                );
                task.abort();
            }
        }
        
        // Give tasks a moment to actually abort
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    /// Clean up finished tasks
    pub async fn cleanup_finished(&self) {
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|task| !task.is_finished());
    }
    
    /// Get task statistics
    pub async fn stats(&self) -> TaskStats {
        let tasks = self.tasks.lock().await;
        
        let total_tasks = tasks.len();
        let active_tasks = self.active_count.load(Ordering::Relaxed);
        let finished_tasks = tasks.iter().filter(|t| t.is_finished()).count();
        
        let task_breakdown = tasks
            .iter()
            .fold(TaskBreakdown::default(), |mut acc, task| {
                match task.priority {
                    TaskPriority::Critical => acc.critical += 1,
                    TaskPriority::High => acc.high += 1,
                    TaskPriority::Normal => acc.normal += 1,
                    TaskPriority::Low => acc.low += 1,
                }
                acc
            });
        
        TaskStats {
            layer_name: self.layer_name.clone(),
            total_tasks,
            active_tasks,
            finished_tasks,
            task_breakdown,
        }
    }
}

/// Task statistics
#[derive(Debug, Clone)]
pub struct TaskStats {
    pub layer_name: String,
    pub total_tasks: usize,
    pub active_tasks: usize,
    pub finished_tasks: usize,
    pub task_breakdown: TaskBreakdown,
}

/// Task breakdown by priority
#[derive(Debug, Clone, Default)]
pub struct TaskBreakdown {
    pub critical: usize,
    pub high: usize,
    pub normal: usize,
    pub low: usize,
}

/// Global task registry for system-wide task tracking
pub struct GlobalTaskRegistry {
    managers: Arc<Mutex<Vec<Arc<LayerTaskManager>>>>,
}

impl GlobalTaskRegistry {
    /// Create a new global registry
    pub fn new() -> Self {
        Self {
            managers: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    /// Register a task manager
    pub async fn register(&self, manager: Arc<LayerTaskManager>) {
        self.managers.lock().await.push(manager);
    }
    
    /// Shutdown all registered managers
    pub async fn shutdown_all(&self, timeout: Duration) -> Result<()> {
        let managers = self.managers.lock().await;
        
        let shutdown_futures: Vec<_> = managers
            .iter()
            .map(|manager| manager.shutdown_all())
            .collect();
        
        // Shutdown all managers in parallel
        let results = tokio::time::timeout(
            timeout,
            futures::future::join_all(shutdown_futures),
        ).await;
        
        match results {
            Ok(results) => {
                for result in results {
                    if let Err(e) = result {
                        error!("Task manager shutdown error: {}", e);
                    }
                }
                Ok(())
            }
            Err(_) => {
                error!("Global shutdown timed out after {:?}", timeout);
                anyhow::bail!("Global task shutdown timeout")
            }
        }
    }
    
    /// Get global statistics
    pub async fn global_stats(&self) -> Vec<TaskStats> {
        let managers = self.managers.lock().await;
        let mut stats = Vec::new();
        
        for manager in managers.iter() {
            stats.push(manager.stats().await);
        }
        
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_task_spawning_and_tracking() {
        let manager = LayerTaskManager::new("test");
        
        let task_id = manager
            .spawn_tracked("test_task", TaskPriority::Normal, async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            })
            .await
            .unwrap();
        
        assert_eq!(task_id, 0);
        assert_eq!(manager.active_task_count(), 1);
        
        // Wait for task to complete
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(manager.active_task_count(), 0);
    }
    
    #[tokio::test]
    async fn test_task_cancellation() {
        let manager = LayerTaskManager::new("test");
        
        // Spawn a long-running task
        let _task_id = manager
            .spawn_tracked("long_task", TaskPriority::Normal, async {
                tokio::time::sleep(Duration::from_secs(10)).await;
            })
            .await
            .unwrap();
        
        assert_eq!(manager.active_task_count(), 1);
        
        // Cancel all tasks
        manager.cancel_all();
        
        // Wait a bit for cancellation to take effect
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(manager.active_task_count(), 0);
    }
    
    #[tokio::test]
    async fn test_shutdown_with_timeout() {
        let manager = LayerTaskManager::with_config(
            "test",
            100,
            Duration::from_millis(100),
        );
        
        // Spawn tasks that won't respond to cancellation
        for i in 0..3 {
            let _task_id = manager
                .spawn_tracked(
                    format!("stubborn_task_{}", i),
                    TaskPriority::Low,
                    async {
                        // This task ignores cancellation
                        loop {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    },
                )
                .await
                .unwrap();
        }
        
        assert_eq!(manager.active_task_count(), 3);
        
        // Shutdown should timeout and force abort
        manager.shutdown_all().await.unwrap();
        
        // All tasks should be aborted
        assert_eq!(manager.active_task_count(), 0);
    }
}