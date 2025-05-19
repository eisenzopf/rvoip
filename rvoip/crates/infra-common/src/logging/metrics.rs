use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Types of metrics that can be collected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricType {
    /// Counter that increments over time
    Counter,
    /// Gauge that can go up and down
    Gauge,
    /// Histogram for distribution of values
    Histogram,
    /// Summary of observations
    Summary,
    /// Timer for measuring durations
    Timer,
}

impl fmt::Display for MetricType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricType::Counter => write!(f, "counter"),
            MetricType::Gauge => write!(f, "gauge"),
            MetricType::Histogram => write!(f, "histogram"),
            MetricType::Summary => write!(f, "summary"),
            MetricType::Timer => write!(f, "timer"),
        }
    }
}

/// A single metric with metadata
#[derive(Debug, Clone)]
pub struct Metric {
    /// Name of the metric
    pub name: String,
    /// Type of metric
    pub metric_type: MetricType,
    /// Component that owns this metric
    pub component: String,
    /// Description of what the metric measures
    pub description: Option<String>,
    /// Labels/tags for the metric
    pub labels: HashMap<String, String>,
    /// Current value of the metric
    pub value: f64,
}

impl Metric {
    /// Create a new counter metric
    pub fn counter<S: Into<String>, T: Into<String>>(name: S, component: T) -> Self {
        Metric {
            name: name.into(),
            metric_type: MetricType::Counter,
            component: component.into(),
            description: None,
            labels: HashMap::new(),
            value: 0.0,
        }
    }
    
    /// Create a new gauge metric
    pub fn gauge<S: Into<String>, T: Into<String>>(name: S, component: T) -> Self {
        Metric {
            name: name.into(),
            metric_type: MetricType::Gauge,
            component: component.into(),
            description: None,
            labels: HashMap::new(),
            value: 0.0,
        }
    }
    
    /// Create a new timer metric
    pub fn timer<S: Into<String>, T: Into<String>>(name: S, component: T) -> Self {
        Metric {
            name: name.into(),
            metric_type: MetricType::Timer,
            component: component.into(),
            description: None,
            labels: HashMap::new(),
            value: 0.0,
        }
    }
    
    /// Add a description to the metric
    pub fn with_description<S: Into<String>>(mut self, description: S) -> Self {
        self.description = Some(description.into());
        self
    }
    
    /// Add a label to the metric
    pub fn with_label<S: Into<String>, T: Into<String>>(mut self, key: S, value: T) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }
    
    /// Add multiple labels to the metric
    pub fn with_labels<S: Into<String>, T: Into<String>>(mut self, labels: Vec<(S, T)>) -> Self {
        for (key, value) in labels {
            self.labels.insert(key.into(), value.into());
        }
        self
    }
    
    /// Set the value of the metric
    pub fn with_value(mut self, value: f64) -> Self {
        self.value = value;
        self
    }
}

/// Collector for gathering and reporting metrics
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    metrics: Arc<RwLock<HashMap<String, Metric>>>,
    timers: Arc<RwLock<HashMap<String, Instant>>>,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        MetricsCollector {
            metrics: Arc::new(RwLock::new(HashMap::new())),
            timers: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Register a metric
    pub fn register(&self, metric: Metric) {
        let mut metrics = self.metrics.write().unwrap();
        metrics.insert(metric.name.clone(), metric);
    }
    
    /// Increment a counter
    pub fn increment(&self, name: &str, amount: f64) {
        let mut metrics = self.metrics.write().unwrap();
        if let Some(metric) = metrics.get_mut(name) {
            if metric.metric_type == MetricType::Counter {
                metric.value += amount;
            }
        }
    }
    
    /// Set a gauge value
    pub fn set_gauge(&self, name: &str, value: f64) {
        let mut metrics = self.metrics.write().unwrap();
        if let Some(metric) = metrics.get_mut(name) {
            if metric.metric_type == MetricType::Gauge {
                metric.value = value;
            }
        }
    }
    
    /// Start a timer
    pub fn start_timer(&self, name: &str) {
        let mut timers = self.timers.write().unwrap();
        timers.insert(name.to_string(), Instant::now());
    }
    
    /// Stop a timer and record the duration
    pub fn stop_timer(&self, name: &str) -> Option<Duration> {
        let mut timers = self.timers.write().unwrap();
        let start = timers.remove(name)?;
        let duration = start.elapsed();
        
        let mut metrics = self.metrics.write().unwrap();
        if let Some(metric) = metrics.get_mut(name) {
            if metric.metric_type == MetricType::Timer {
                metric.value = duration.as_secs_f64();
            }
        }
        
        Some(duration)
    }
    
    /// Get a snapshot of all metrics
    pub fn snapshot(&self) -> HashMap<String, Metric> {
        self.metrics.read().unwrap().clone()
    }
    
    /// Get a specific metric
    pub fn get(&self, name: &str) -> Option<Metric> {
        self.metrics.read().unwrap().get(name).cloned()
    }
    
    /// Clear all metrics
    pub fn clear(&self) {
        self.metrics.write().unwrap().clear();
        self.timers.write().unwrap().clear();
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Guard for automatically stopping a timer when dropped
pub struct TimerGuard<'a> {
    collector: &'a MetricsCollector,
    name: String,
}

impl<'a> TimerGuard<'a> {
    /// Create a new timer guard
    pub fn new(collector: &'a MetricsCollector, name: &str) -> Self {
        collector.start_timer(name);
        TimerGuard {
            collector,
            name: name.to_string(),
        }
    }
    
    /// Stop the timer early and get the duration
    pub fn stop(self) -> Option<Duration> {
        self.collector.stop_timer(&self.name)
    }
}

impl<'a> Drop for TimerGuard<'a> {
    fn drop(&mut self) {
        let _ = self.collector.stop_timer(&self.name);
    }
} 