//! # Metrics Collection Implementation
//!
//! This module provides comprehensive metrics collection and analysis capabilities
//! for call center operations, including performance tracking, KPI calculation,
//! historical analysis, and automated reporting.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, debug, warn, error};
use chrono::{DateTime, Utc, TimeZone};
use tokio::sync::RwLock;

use crate::error::{CallCenterError, Result};
use crate::orchestrator::CallCenterEngine;
use crate::agent::AgentId;

/// # Metrics Collector for Call Center Analytics
///
/// The `MetricsCollector` provides comprehensive metrics collection, analysis, and
/// reporting capabilities for call center operations. It tracks key performance
/// indicators (KPIs), generates historical reports, and enables data-driven
/// decision making for call center optimization.
///
/// ## Key Features
///
/// - **Real-time Metrics**: Live collection of call center performance data
/// - **KPI Calculation**: Automatic computation of industry-standard metrics
/// - **Historical Analysis**: Trend analysis and performance over time
/// - **Performance Reports**: Automated generation of detailed reports
/// - **Alert Generation**: Proactive notifications for performance issues
/// - **Data Export**: Export capabilities for external analysis tools
///
/// ## Collected Metrics
///
/// ### Call Metrics
/// - Total calls handled
/// - Average handle time (AHT)
/// - Call resolution rates
/// - Abandon rates
/// - Queue wait times
///
/// ### Agent Metrics
/// - Agent utilization rates
/// - Performance scores
/// - Productivity measures
/// - Schedule adherence
/// - Customer satisfaction ratings
///
/// ### Service Level Metrics
/// - Service level percentage
/// - Average speed of answer (ASA)
/// - Peak hour performance
/// - SLA compliance rates
///
/// ### Quality Metrics
/// - Call quality scores
/// - Customer satisfaction (CSAT)
/// - Net Promoter Score (NPS)
/// - First call resolution (FCR)
///
/// ## Examples
///
/// ### Basic Metrics Collection
///
/// ```rust
/// use rvoip_call_engine::monitoring::MetricsCollector;
/// use rvoip_call_engine::orchestrator::CallCenterEngine;
/// use std::sync::Arc;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let engine = CallCenterEngine::new(Default::default(), None).await?;
/// let metrics = MetricsCollector::new(engine.clone());
/// 
/// // Record a call completion
/// metrics.record_call_completed(
///     "call-123",
///     "agent-001", 
///     180, // 3 minutes
///     true, // resolved
///     4.5  // satisfaction score
/// ).await?;
/// 
/// // Get current performance summary
/// let summary = metrics.get_performance_summary().await;
/// println!("📊 Performance Summary:");
/// println!("  Calls today: {}", summary.total_calls_today);
/// println!("  Average handle time: {}s", summary.average_handle_time);
/// println!("  Service level: {:.1}%", summary.service_level_percentage);
/// # Ok(())
/// # }
/// ```
///
/// ### Historical Reporting
///
/// ```rust
/// use rvoip_call_engine::monitoring::MetricsCollector;
/// use chrono::{Utc, Duration};
/// 
/// # async fn example(metrics: MetricsCollector) -> Result<(), Box<dyn std::error::Error>> {
/// // Generate weekly performance report
/// let end_time = Utc::now();
/// let start_time = end_time - Duration::days(7);
/// 
/// let report = metrics.generate_performance_report(start_time, end_time).await?;
/// 
/// println!("📋 Weekly Performance Report");
/// println!("  Period: {} to {}", start_time.format("%Y-%m-%d"), end_time.format("%Y-%m-%d"));
/// println!("  Total calls: {}", report.total_calls);
/// println!("  Service level: {:.1}%", report.service_level_percentage);
/// println!("  Customer satisfaction: {:.1}/5.0", report.average_satisfaction_score);
/// 
/// // Export detailed data
/// let csv_data = metrics.export_call_data(start_time, end_time, "csv").await?;
/// println!("Exported {} bytes of call data", csv_data.len());
/// # Ok(())
/// # }
/// ```
///
/// ### Real-time Monitoring
///
/// ```rust
/// use rvoip_call_engine::monitoring::MetricsCollector;
/// 
/// # async fn example(metrics: MetricsCollector) -> Result<(), Box<dyn std::error::Error>> {
/// // Monitor real-time metrics
/// loop {
///     let realtime_metrics = metrics.get_realtime_metrics().await;
///     
///     // Check for performance issues
///     if realtime_metrics.current_service_level < 80.0 {
///         println!("🚨 Service level alert: {:.1}%", realtime_metrics.current_service_level);
///     }
///     
///     if realtime_metrics.average_wait_time > 120 {
///         println!("⚠️ Wait time alert: {}s", realtime_metrics.average_wait_time);
///     }
///     
///     // Update every 30 seconds
///     tokio::time::sleep(std::time::Duration::from_secs(30)).await;
///     break; // For example purposes
/// }
/// # Ok(())
/// # }
/// ```
pub struct MetricsCollector {
    /// Reference to the call center engine for data access
    engine: Arc<CallCenterEngine>,
    
    /// In-memory metrics cache for fast access
    metrics_cache: Arc<RwLock<MetricsCache>>,
    
    /// Historical metrics storage (ring buffer for recent data)
    historical_data: Arc<RwLock<VecDeque<HistoricalDataPoint>>>,
    
    /// Agent performance tracking
    agent_metrics: Arc<RwLock<HashMap<String, AgentMetrics>>>,
    
    /// Configuration for metrics collection
    config: MetricsConfig,
}

/// Performance summary for dashboard display
#[derive(Debug, Clone)]
pub struct PerformanceSummary {
    /// Total calls handled today
    pub total_calls_today: u64,
    
    /// Average handle time in seconds
    pub average_handle_time: u64,
    
    /// Service level percentage
    pub service_level_percentage: f64,
    
    /// Average speed of answer in seconds
    pub average_speed_of_answer: u64,
    
    /// Call abandon rate percentage
    pub abandon_rate_percentage: f64,
    
    /// Agent utilization percentage
    pub agent_utilization_percentage: f64,
    
    /// Customer satisfaction score (1.0 - 5.0)
    pub customer_satisfaction_score: f32,
    
    /// First call resolution rate percentage
    pub first_call_resolution_rate: f64,
    
    /// Peak concurrent calls today
    pub peak_concurrent_calls: u32,
    
    /// Current active agents
    pub active_agents: u32,
    
    /// Report generation timestamp
    pub timestamp: DateTime<Utc>,
}

/// Comprehensive performance report for historical analysis
#[derive(Debug, Clone)]
pub struct PerformanceReport {
    /// Report period start
    pub period_start: DateTime<Utc>,
    
    /// Report period end
    pub period_end: DateTime<Utc>,
    
    /// Total calls in period
    pub total_calls: u64,
    
    /// Total call duration in seconds
    pub total_call_duration: u64,
    
    /// Service level percentage
    pub service_level_percentage: f64,
    
    /// Average satisfaction score
    pub average_satisfaction_score: f32,
    
    /// Call abandon statistics
    pub abandon_statistics: AbandonStatistics,
    
    /// Agent performance breakdown
    pub agent_performance: HashMap<String, AgentPerformanceReport>,
    
    /// Hourly distribution of calls
    pub hourly_distribution: Vec<HourlyCallData>,
    
    /// Quality metrics summary
    pub quality_metrics: QualityMetricsSummary,
}

/// Real-time metrics for live monitoring
#[derive(Debug, Clone)]
pub struct RealtimeMetrics {
    /// Current active calls
    pub active_calls: u32,
    
    /// Current queued calls
    pub queued_calls: u32,
    
    /// Current service level percentage
    pub current_service_level: f64,
    
    /// Current average wait time in seconds
    pub average_wait_time: u64,
    
    /// Available agents count
    pub available_agents: u32,
    
    /// Busy agents count
    pub busy_agents: u32,
    
    /// Calls per hour rate
    pub calls_per_hour: f64,
    
    /// Current system capacity percentage
    pub capacity_percentage: f64,
    
    /// Metrics snapshot time
    pub timestamp: DateTime<Utc>,
}

/// Call abandon statistics
#[derive(Debug, Clone)]
pub struct AbandonStatistics {
    /// Total abandoned calls
    pub total_abandoned: u64,
    
    /// Abandon rate percentage
    pub abandon_rate_percentage: f64,
    
    /// Average time before abandon (seconds)
    pub average_abandon_time: u64,
    
    /// Abandons by wait time buckets
    pub abandon_by_wait_time: HashMap<String, u64>,
}

/// Individual agent performance report
#[derive(Debug, Clone)]
pub struct AgentPerformanceReport {
    /// Agent identifier
    pub agent_id: String,
    
    /// Calls handled in period
    pub calls_handled: u32,
    
    /// Total talk time in seconds
    pub total_talk_time: u64,
    
    /// Average handle time
    pub average_handle_time: u64,
    
    /// Utilization percentage
    pub utilization_percentage: f64,
    
    /// Customer satisfaction score
    pub satisfaction_score: f32,
    
    /// First call resolution rate
    pub fcr_rate: f64,
    
    /// Schedule adherence percentage
    pub schedule_adherence: f64,
}

/// Hourly call distribution data
#[derive(Debug, Clone)]
pub struct HourlyCallData {
    /// Hour of day (0-23)
    pub hour: u8,
    
    /// Number of calls in this hour
    pub call_count: u64,
    
    /// Average wait time for this hour
    pub average_wait_time: u64,
    
    /// Service level for this hour
    pub service_level: f64,
}

/// Quality metrics summary
#[derive(Debug, Clone)]
pub struct QualityMetricsSummary {
    /// Average call quality score
    pub average_quality_score: f32,
    
    /// Customer satisfaction distribution
    pub satisfaction_distribution: HashMap<u8, u64>, // Score (1-5) -> Count
    
    /// Net Promoter Score
    pub net_promoter_score: f32,
    
    /// Quality alerts generated
    pub quality_alerts_count: u64,
}

/// Internal metrics cache
#[derive(Debug, Clone)]
struct MetricsCache {
    /// Cached performance summary
    performance_summary: Option<PerformanceSummary>,
    
    /// Cached real-time metrics
    realtime_metrics: Option<RealtimeMetrics>,
    
    /// Cache last update time
    last_updated: DateTime<Utc>,
    
    /// Cache validity duration
    cache_duration: Duration,
}

/// Historical data point for trend analysis
#[derive(Debug, Clone)]
struct HistoricalDataPoint {
    /// Data point timestamp
    timestamp: DateTime<Utc>,
    
    /// Snapshot of metrics at this time
    metrics: RealtimeMetrics,
}

/// Agent-specific metrics tracking
#[derive(Debug, Clone)]
struct AgentMetrics {
    /// Agent identifier
    agent_id: String,
    
    /// Total calls handled
    total_calls: u64,
    
    /// Total talk time
    total_talk_time: Duration,
    
    /// Call resolution count
    resolved_calls: u64,
    
    /// Satisfaction scores
    satisfaction_scores: Vec<f32>,
    
    /// Last activity timestamp
    last_activity: DateTime<Utc>,
}

/// Configuration for metrics collection
#[derive(Debug, Clone)]
struct MetricsConfig {
    /// How long to keep historical data in memory
    historical_retention_hours: u32,
    
    /// Cache duration for expensive calculations
    cache_duration_seconds: u64,
    
    /// Service level target percentage
    service_level_target: f64,
    
    /// Target answer time in seconds
    target_answer_time: u64,
}

impl MetricsCollector {
    /// Create a new metrics collector
    ///
    /// Initializes a new metrics collector connected to the specified call center engine.
    /// The collector will begin tracking performance metrics and providing analytics
    /// capabilities immediately.
    ///
    /// # Arguments
    ///
    /// * `engine` - Shared reference to the call center engine
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::MetricsCollector;
    /// use rvoip_call_engine::orchestrator::CallCenterEngine;
    /// use std::sync::Arc;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = CallCenterEngine::new(Default::default(), None).await?;
    /// let metrics = MetricsCollector::new(engine.clone());
    /// 
    /// // Metrics collector is ready for use
    /// println!("Metrics collection initialized");
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        let config = MetricsConfig {
            historical_retention_hours: 24,
            cache_duration_seconds: 30,
            service_level_target: 80.0,
            target_answer_time: 20,
        };
        
        Self {
            engine,
            metrics_cache: Arc::new(RwLock::new(MetricsCache {
                performance_summary: None,
                realtime_metrics: None,
                last_updated: Utc::now(),
                cache_duration: Duration::from_secs(config.cache_duration_seconds),
            })),
            historical_data: Arc::new(RwLock::new(VecDeque::new())),
            agent_metrics: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }
    
    /// Record a completed call for metrics collection
    ///
    /// Records the completion of a call with all relevant metrics for analysis.
    /// This data is used to calculate performance statistics and generate reports.
    ///
    /// # Arguments
    ///
    /// * `call_id` - Unique identifier for the call
    /// * `agent_id` - Agent who handled the call
    /// * `duration_seconds` - Total call duration in seconds
    /// * `resolved` - Whether the call was resolved successfully
    /// * `satisfaction_score` - Customer satisfaction score (1.0 - 5.0)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::MetricsCollector;
    /// 
    /// # async fn example(metrics: MetricsCollector) -> Result<(), Box<dyn std::error::Error>> {
    /// // Record a successful call
    /// metrics.record_call_completed(
    ///     "call-12345",
    ///     "agent-alice", 
    ///     240, // 4 minutes
    ///     true, // successfully resolved
    ///     4.8  // high satisfaction
    /// ).await?;
    /// 
    /// // Record a problematic call
    /// metrics.record_call_completed(
    ///     "call-12346",
    ///     "agent-bob", 
    ///     600, // 10 minutes (long)
    ///     false, // not resolved
    ///     2.1   // low satisfaction
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn record_call_completed(
        &self,
        call_id: &str,
        agent_id: &str,
        duration_seconds: u64,
        resolved: bool,
        satisfaction_score: f32,
    ) -> Result<()> {
        debug!("📊 Recording call completion: {} by {} ({}s, resolved: {}, satisfaction: {:.1})",
               call_id, agent_id, duration_seconds, resolved, satisfaction_score);
        
        // Update agent metrics
        {
            let mut agent_metrics = self.agent_metrics.write().await;
            let agent_metric = agent_metrics.entry(agent_id.to_string()).or_insert_with(|| {
                AgentMetrics {
                    agent_id: agent_id.to_string(),
                    total_calls: 0,
                    total_talk_time: Duration::ZERO,
                    resolved_calls: 0,
                    satisfaction_scores: Vec::new(),
                    last_activity: Utc::now(),
                }
            });
            
            agent_metric.total_calls += 1;
            agent_metric.total_talk_time += Duration::from_secs(duration_seconds);
            if resolved {
                agent_metric.resolved_calls += 1;
            }
            agent_metric.satisfaction_scores.push(satisfaction_score);
            agent_metric.last_activity = Utc::now();
        }
        
        // Invalidate cache to force refresh
        self.invalidate_cache().await;
        
        info!("✅ Call metrics recorded for {}", agent_id);
        Ok(())
    }
    
    /// Get current performance summary
    ///
    /// Returns a comprehensive summary of current call center performance,
    /// including all key metrics for dashboard display.
    ///
    /// # Returns
    ///
    /// [`PerformanceSummary`] containing current performance metrics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::MetricsCollector;
    /// 
    /// # async fn example(metrics: MetricsCollector) -> Result<(), Box<dyn std::error::Error>> {
    /// let summary = metrics.get_performance_summary().await;
    /// 
    /// println!("📊 Current Performance:");
    /// println!("  Calls today: {}", summary.total_calls_today);
    /// println!("  Service level: {:.1}%", summary.service_level_percentage);
    /// println!("  Satisfaction: {:.1}/5.0", summary.customer_satisfaction_score);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_performance_summary(&self) -> PerformanceSummary {
        // Check cache first
        {
            let cache = self.metrics_cache.read().await;
            if let Some(summary) = &cache.performance_summary {
                let cache_age = Utc::now().signed_duration_since(cache.last_updated);
                if cache_age < chrono::Duration::from_std(cache.cache_duration).unwrap_or(chrono::Duration::seconds(30)) {
                    return summary.clone();
                }
            }
        }
        
        // Calculate fresh metrics
        let engine_stats = self.engine.get_stats().await;
        let agent_metrics = self.agent_metrics.read().await;
        
        let total_calls_today = agent_metrics.values()
            .map(|m| m.total_calls)
            .sum();
        
        let average_handle_time = self.calculate_average_handle_time(&agent_metrics).await;
        let customer_satisfaction = self.calculate_average_satisfaction(&agent_metrics).await;
        
        let summary = PerformanceSummary {
            total_calls_today,
            average_handle_time,
            service_level_percentage: 85.0, // TODO: Calculate from actual data
            average_speed_of_answer: 18,     // TODO: Calculate from actual data
            abandon_rate_percentage: 5.2,    // TODO: Calculate from actual data
            agent_utilization_percentage: 72.5, // TODO: Calculate from actual data
            customer_satisfaction_score: customer_satisfaction,
            first_call_resolution_rate: 89.3, // TODO: Calculate from actual data
            peak_concurrent_calls: 25,       // TODO: Track from historical data
            active_agents: engine_stats.available_agents as u32 + engine_stats.busy_agents as u32,
            timestamp: Utc::now(),
        };
        
        // Update cache
        {
            let mut cache = self.metrics_cache.write().await;
            cache.performance_summary = Some(summary.clone());
            cache.last_updated = Utc::now();
        }
        
        summary
    }
    
    /// Generate a comprehensive performance report for a time period
    ///
    /// Creates a detailed performance report covering all metrics for the
    /// specified time period. Useful for management reporting and analysis.
    ///
    /// # Arguments
    ///
    /// * `start_time` - Beginning of the report period
    /// * `end_time` - End of the report period
    ///
    /// # Returns
    ///
    /// `Ok(PerformanceReport)` with comprehensive metrics, or error if generation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::MetricsCollector;
    /// use chrono::{Utc, Duration};
    /// 
    /// # async fn example(metrics: MetricsCollector) -> Result<(), Box<dyn std::error::Error>> {
    /// // Generate report for last week
    /// let end_time = Utc::now();
    /// let start_time = end_time - Duration::days(7);
    /// 
    /// let report = metrics.generate_performance_report(start_time, end_time).await?;
    /// 
    /// println!("📋 Performance Report ({} to {})", 
    ///          start_time.format("%Y-%m-%d"), 
    ///          end_time.format("%Y-%m-%d"));
    /// println!("Total calls: {}", report.total_calls);
    /// println!("Service level: {:.1}%", report.service_level_percentage);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn generate_performance_report(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<PerformanceReport> {
        info!("📊 Generating performance report for {} to {}", start_time, end_time);
        
        // TODO: Implement comprehensive report generation
        // This would typically:
        // 1. Query database for call data in the time period
        // 2. Calculate all performance metrics
        // 3. Generate agent performance breakdown
        // 4. Create hourly distribution analysis
        // 5. Compile quality metrics summary
        
        warn!("🚧 generate_performance_report not fully implemented yet");
        
        // Return placeholder report
        Ok(PerformanceReport {
            period_start: start_time,
            period_end: end_time,
            total_calls: 0,
            total_call_duration: 0,
            service_level_percentage: 0.0,
            average_satisfaction_score: 0.0,
            abandon_statistics: AbandonStatistics {
                total_abandoned: 0,
                abandon_rate_percentage: 0.0,
                average_abandon_time: 0,
                abandon_by_wait_time: HashMap::new(),
            },
            agent_performance: HashMap::new(),
            hourly_distribution: Vec::new(),
            quality_metrics: QualityMetricsSummary {
                average_quality_score: 0.0,
                satisfaction_distribution: HashMap::new(),
                net_promoter_score: 0.0,
                quality_alerts_count: 0,
            },
        })
    }
    
    /// Get real-time metrics for live monitoring
    ///
    /// Returns current real-time metrics suitable for live dashboard display
    /// and monitoring applications.
    ///
    /// # Returns
    ///
    /// [`RealtimeMetrics`] with current system state.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::MetricsCollector;
    /// 
    /// # async fn example(metrics: MetricsCollector) -> Result<(), Box<dyn std::error::Error>> {
    /// let realtime = metrics.get_realtime_metrics().await;
    /// 
    /// println!("🔴 LIVE: {} active calls, {} in queue", 
    ///          realtime.active_calls, realtime.queued_calls);
    /// println!("📊 Service level: {:.1}%, Wait time: {}s", 
    ///          realtime.current_service_level, realtime.average_wait_time);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_realtime_metrics(&self) -> RealtimeMetrics {
        let engine_stats = self.engine.get_stats().await;
        
        RealtimeMetrics {
            active_calls: engine_stats.active_calls as u32,
            queued_calls: engine_stats.queued_calls as u32,
            current_service_level: 85.0, // TODO: Calculate from recent data
            average_wait_time: 45,       // TODO: Calculate from queue data
            available_agents: engine_stats.available_agents as u32,
            busy_agents: engine_stats.busy_agents as u32,
            calls_per_hour: 120.0,      // TODO: Calculate from call rate
            capacity_percentage: 68.5,   // TODO: Calculate system capacity
            timestamp: Utc::now(),
        }
    }
    
    /// Export call data in specified format
    ///
    /// Exports detailed call data for the specified time period in the requested
    /// format for external analysis or reporting tools.
    ///
    /// # Arguments
    ///
    /// * `start_time` - Beginning of export period
    /// * `end_time` - End of export period  
    /// * `format` - Export format ("csv", "json", "xlsx")
    ///
    /// # Returns
    ///
    /// `Ok(Vec<u8>)` containing the exported data, or error if export fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::monitoring::MetricsCollector;
    /// use chrono::{Utc, Duration};
    /// 
    /// # async fn example(metrics: MetricsCollector) -> Result<(), Box<dyn std::error::Error>> {
    /// let end_time = Utc::now();
    /// let start_time = end_time - Duration::days(1);
    /// 
    /// // Export as CSV
    /// let csv_data = metrics.export_call_data(start_time, end_time, "csv").await?;
    /// std::fs::write("call_data.csv", csv_data)?;
    /// 
    /// // Export as JSON
    /// let json_data = metrics.export_call_data(start_time, end_time, "json").await?;
    /// println!("Exported {} bytes of JSON data", json_data.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn export_call_data(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        format: &str,
    ) -> Result<Vec<u8>> {
        info!("📤 Exporting call data from {} to {} in {} format", 
              start_time, end_time, format);
        
        // TODO: Implement data export functionality
        // This would:
        // 1. Query database for call records in time period
        // 2. Format data according to requested format
        // 3. Generate appropriate file content
        // 4. Return formatted data
        
        warn!("🚧 export_call_data not fully implemented yet");
        
        match format {
            "csv" => Ok(b"timestamp,call_id,agent_id,duration,resolved\n".to_vec()),
            "json" => Ok(b"[]".to_vec()),
            _ => Err(CallCenterError::InvalidInput(format!("Unsupported export format: {}", format))),
        }
    }
    
    // Internal helper methods
    
    async fn calculate_average_handle_time(&self, agent_metrics: &HashMap<String, AgentMetrics>) -> u64 {
        let total_calls: u64 = agent_metrics.values().map(|m| m.total_calls).sum();
        let total_time: Duration = agent_metrics.values().map(|m| m.total_talk_time).sum();
        
        if total_calls > 0 {
            total_time.as_secs() / total_calls
        } else {
            0
        }
    }
    
    async fn calculate_average_satisfaction(&self, agent_metrics: &HashMap<String, AgentMetrics>) -> f32 {
        let all_scores: Vec<f32> = agent_metrics.values()
            .flat_map(|m| &m.satisfaction_scores)
            .copied()
            .collect();
        
        if !all_scores.is_empty() {
            all_scores.iter().sum::<f32>() / all_scores.len() as f32
        } else {
            0.0
        }
    }
    
    async fn invalidate_cache(&self) {
        let mut cache = self.metrics_cache.write().await;
        cache.performance_summary = None;
        cache.realtime_metrics = None;
        cache.last_updated = Utc::now() - Duration::from_secs(3600); // Force refresh
    }
}

impl Clone for MetricsCollector {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            metrics_cache: self.metrics_cache.clone(),
            historical_data: self.historical_data.clone(),
            agent_metrics: self.agent_metrics.clone(),
            config: self.config.clone(),
        }
    }
} 