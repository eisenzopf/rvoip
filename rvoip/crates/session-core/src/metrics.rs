//! Monitoring and metrics for operational observability
//!
//! This module provides metrics collection and operational monitoring 
//! capabilities for the session-core library in production environments.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Instant, Duration, SystemTime};
use std::collections::HashMap;
use tokio::sync::Mutex;

/// Metrics collector for session core
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    /// Total active sessions
    active_sessions: Arc<AtomicUsize>,
    
    /// Total active dialogs
    active_dialogs: Arc<AtomicUsize>,
    
    /// Total completed sessions
    completed_sessions: Arc<AtomicUsize>,
    
    /// Total failed sessions
    failed_sessions: Arc<AtomicUsize>,
    
    /// Detailed metrics storage
    detailed_metrics: Arc<Mutex<DetailedMetrics>>,
}

/// Detailed metrics for deeper analysis
#[derive(Debug)]
struct DetailedMetrics {
    /// Operation durations (in microseconds)
    op_durations: HashMap<String, Vec<u64>>,
    
    /// Error counts by category
    errors_by_category: HashMap<String, usize>,
    
    /// Session durations (in seconds)
    session_durations: Vec<u64>,
    
    /// Start time for this metrics collection period
    start_time: std::time::SystemTime,
}

impl Default for DetailedMetrics {
    fn default() -> Self {
        Self {
            op_durations: HashMap::new(),
            errors_by_category: HashMap::new(),
            session_durations: Vec::new(),
            start_time: SystemTime::now(),
        }
    }
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        Self {
            active_sessions: Arc::new(AtomicUsize::new(0)),
            active_dialogs: Arc::new(AtomicUsize::new(0)),
            completed_sessions: Arc::new(AtomicUsize::new(0)),
            failed_sessions: Arc::new(AtomicUsize::new(0)),
            detailed_metrics: Arc::new(Mutex::new(DetailedMetrics {
                start_time: SystemTime::now(),
                ..Default::default()
            })),
        }
    }
    
    /// Record a new session
    pub fn record_new_session(&self) {
        self.active_sessions.fetch_add(1, Ordering::SeqCst);
    }
    
    /// Record a completed session
    pub fn record_completed_session(&self, duration_secs: u64) {
        self.active_sessions.fetch_sub(1, Ordering::SeqCst);
        self.completed_sessions.fetch_add(1, Ordering::SeqCst);
        
        // Record the session duration
        tokio::spawn({
            let detailed_metrics = self.detailed_metrics.clone();
            async move {
                let mut metrics = detailed_metrics.lock().await;
                metrics.session_durations.push(duration_secs);
            }
        });
    }
    
    /// Record a failed session
    pub fn record_failed_session(&self, error_category: String) {
        self.active_sessions.fetch_sub(1, Ordering::SeqCst);
        self.failed_sessions.fetch_add(1, Ordering::SeqCst);
        
        // Record the error category
        tokio::spawn({
            let detailed_metrics = self.detailed_metrics.clone();
            async move {
                let mut metrics = detailed_metrics.lock().await;
                *metrics.errors_by_category.entry(error_category).or_insert(0) += 1;
            }
        });
    }
    
    /// Record a new dialog
    pub fn record_new_dialog(&self) {
        self.active_dialogs.fetch_add(1, Ordering::SeqCst);
    }
    
    /// Record a terminated dialog
    pub fn record_terminated_dialog(&self) {
        self.active_dialogs.fetch_sub(1, Ordering::SeqCst);
    }
    
    /// Record an operation duration
    pub fn record_operation(&self, operation: &str, duration_us: u64) {
        tokio::spawn({
            let detailed_metrics = self.detailed_metrics.clone();
            let operation = operation.to_string();
            async move {
                let mut metrics = detailed_metrics.lock().await;
                metrics.op_durations.entry(operation).or_default().push(duration_us);
            }
        });
    }
    
    /// Get current metrics as a formatted string
    pub async fn get_metrics_report(&self) -> String {
        let active_sessions = self.active_sessions.load(Ordering::SeqCst);
        let active_dialogs = self.active_dialogs.load(Ordering::SeqCst);
        let completed_sessions = self.completed_sessions.load(Ordering::SeqCst);
        let failed_sessions = self.failed_sessions.load(Ordering::SeqCst);
        
        let detailed = self.detailed_metrics.lock().await;
        let uptime = detailed.start_time.elapsed();
        
        let mut report = format!(
            "RVOIP Session Core Metrics\n\
             ---------------------------\n\
             Uptime: {:?}\n\
             Active Sessions: {}\n\
             Active Dialogs: {}\n\
             Completed Sessions: {}\n\
             Failed Sessions: {}\n\n",
            uptime, active_sessions, active_dialogs, completed_sessions, failed_sessions
        );
        
        // Add operation statistics if available
        if !detailed.op_durations.is_empty() {
            report.push_str("Operation Performance:\n");
            for (op, durations) in &detailed.op_durations {
                if !durations.is_empty() {
                    let avg = durations.iter().sum::<u64>() as f64 / durations.len() as f64;
                    report.push_str(&format!("  {}: {:.2} µs average ({} samples)\n", 
                                          op, avg, durations.len()));
                }
            }
            report.push('\n');
        }
        
        // Add error statistics if available
        if !detailed.errors_by_category.is_empty() {
            report.push_str("Errors by Category:\n");
            for (category, count) in &detailed.errors_by_category {
                report.push_str(&format!("  {}: {}\n", category, count));
            }
        }
        
        report
    }
    
    /// Reset all metrics
    pub async fn reset(&self) {
        self.active_sessions.store(0, Ordering::SeqCst);
        self.active_dialogs.store(0, Ordering::SeqCst);
        self.completed_sessions.store(0, Ordering::SeqCst);
        self.failed_sessions.store(0, Ordering::SeqCst);
        
        let mut detailed = self.detailed_metrics.lock().await;
        *detailed = DetailedMetrics {
            start_time: SystemTime::now(),
            ..Default::default()
        };
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
} 