//! # Call Monitoring and Analytics Module
//!
//! This module provides comprehensive real-time monitoring, metrics collection,
//! analytics, and supervisor oversight capabilities for the call center. It enables
//! supervisors and administrators to monitor call center performance, agent
//! productivity, and customer satisfaction in real-time.
//!
//! ## Architecture
//!
//! The monitoring system follows an event-driven architecture:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │              Call Center Operations                         │
//! │  (Agents, Calls, Queues, Routing)                         │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │ Events & Metrics
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                  Event Collector                            │
//! │  - Call events (start, end, transfer, hold)               │
//! │  - Agent events (login, status change, performance)       │
//! │  - Queue events (enqueue, dequeue, overflow)              │
//! │  - System events (errors, warnings, capacity)             │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//!           ┌───────────────┼───────────────┐
//!           │               │               │
//! ┌─────────▼─────────┐ ┌─────▼─────┐ ┌─────▼──────┐
//! │ Metrics           │ │Supervisor │ │ Analytics  │
//! │ Collector         │ │ Monitor   │ │ Engine     │
//! │                   │ │           │ │            │
//! │ • Real-time KPIs  │ │ • Live    │ │ • Reports  │
//! │ • Performance     │ │   Dashboard│ │ • Trends   │
//! │ • Alerting        │ │ • Alerts  │ │ • ML Insights│
//! └───────────────────┘ └───────────┘ └────────────┘
//! ```
//!
//! ## Core Features
//!
//! ### **Real-Time Monitoring**
//! - Live call center dashboard
//! - Agent status and performance tracking
//! - Queue length and wait time monitoring
//! - System health and capacity alerts
//!
//! ### **Performance Metrics**
//! - Service level agreements (SLA) tracking
//! - Average handling time (AHT) analysis
//! - First call resolution (FCR) rates
//! - Customer satisfaction scores
//!
//! ### **Supervisor Tools**
//! - Agent coaching and intervention
//! - Call barge-in and whisper capabilities
//! - Performance reports and analytics
//! - Real-time queue management
//!
//! ### **Analytics and Reporting**
//! - Historical performance trends
//! - Predictive analytics for staffing
//! - Custom dashboard creation
//! - Automated report generation
//!
//! ## Quick Start
//!
//! ### Basic Monitoring Setup
//!
//! ```rust
//! use rvoip_call_engine::monitoring::{SupervisorMonitor, MetricsCollector, CallCenterEvents};
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize monitoring components
//! let supervisor_monitor = SupervisorMonitor::new();
//! let metrics_collector = MetricsCollector::new();
//! let event_system = CallCenterEvents::new();
//! 
//! println!("Monitoring system initialized");
//! # Ok(())
//! # }
//! ```
//!
//! ### Real-Time Dashboard Example
//!
//! ```rust
//! use rvoip_call_engine::monitoring::SupervisorMonitor;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let monitor = SupervisorMonitor::new();
//! 
//! // Example monitoring loop for supervisor dashboard
//! loop {
//!     // Get current call center status
//!     // let stats = monitor.get_realtime_stats().await?;
//!     
//!     // Display key metrics
//!     // println!("📊 Call Center Dashboard");
//!     // println!("   Active Calls: {}", stats.active_calls);
//!     // println!("   Available Agents: {}", stats.available_agents);
//!     // println!("   Queue Length: {}", stats.total_queued);
//!     // println!("   Service Level: {:.1}%", stats.service_level);
//!     
//!     // Check for alerts
//!     // if let Some(alerts) = monitor.check_alerts().await? {
//!     //     for alert in alerts {
//!     //         println!("🚨 Alert: {}", alert.message);
//!     //     }
//!     // }
//!     
//!     // Update every 5 seconds
//!     tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
//!     break; // For example purposes
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Metrics Collection Example
//!
//! ```rust
//! use rvoip_call_engine::monitoring::MetricsCollector;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let metrics = MetricsCollector::new();
//! 
//! // Collect various metrics
//! // metrics.record_call_start("call-123", "agent-001").await?;
//! // metrics.record_call_end("call-123", 180, "resolved").await?; // 3 minutes
//! // metrics.record_agent_status_change("agent-001", "available", "busy").await?;
//! // metrics.record_queue_event("support", "enqueue", 1).await?;
//! 
//! // Generate performance report
//! // let report = metrics.generate_performance_report(
//! //     chrono::Utc::now() - chrono::Duration::hours(24), // Last 24 hours
//! //     chrono::Utc::now()
//! // ).await?;
//! 
//! println!("Metrics collection configured");
//! # Ok(())
//! # }
//! ```
//!
//! ## Key Performance Indicators (KPIs)
//!
//! The monitoring system tracks essential call center KPIs:
//!
//! ### **Service Level Metrics**
//! - **Service Level**: Percentage of calls answered within target time
//! - **Average Speed of Answer (ASA)**: Average time to answer calls
//! - **Abandon Rate**: Percentage of calls that hang up before being answered
//! - **Queue Time**: Average time callers wait in queue
//!
//! ### **Agent Performance Metrics**
//! - **Average Handling Time (AHT)**: Average time per call including talk and wrap-up
//! - **First Call Resolution (FCR)**: Percentage of issues resolved on first contact
//! - **Utilization Rate**: Percentage of time agents spend on productive activities
//! - **Adherence**: How well agents follow their scheduled activities
//!
//! ### **Quality Metrics**
//! - **Customer Satisfaction (CSAT)**: Customer satisfaction scores
//! - **Net Promoter Score (NPS)**: Customer loyalty measurement
//! - **Call Quality Score**: Based on call monitoring and evaluation
//! - **Escalation Rate**: Percentage of calls escalated to supervisors
//!
//! ### **Operational Metrics**
//! - **Occupancy Rate**: Percentage of time agents are handling calls
//! - **Shrinkage**: Time lost to breaks, training, meetings, etc.
//! - **Staff Requirements**: Predicted staffing needs based on forecasts
//! - **Cost Per Call**: Total operational cost divided by call volume
//!
//! ## Event System
//!
//! The monitoring system uses an event-driven architecture to track all call center activities:
//!
//! ### **Call Events**
//! ```rust
//! # use chrono::{DateTime, Utc};
//! # enum CallEvent {
//! #     CallStarted { session_id: String, agent_id: String, timestamp: DateTime<Utc> },
//! #     CallEnded { session_id: String, duration_seconds: u32, resolution: String },
//! #     CallTransferred { session_id: String, from_agent: String, to_agent: String },
//! #     CallHeld { session_id: String, hold_duration: u32 },
//! # }
//! ```
//!
//! ### **Agent Events**  
//! ```rust
//! # use chrono::{DateTime, Utc};
//! # enum AgentEvent {
//! #     AgentLogin { agent_id: String, timestamp: DateTime<Utc> },
//! #     AgentLogout { agent_id: String, timestamp: DateTime<Utc> },
//! #     StatusChange { agent_id: String, old_status: String, new_status: String },
//! #     PerformanceUpdate { agent_id: String, metric: String, value: f64 },
//! # }
//! ```
//!
//! ### **Queue Events**
//! ```rust
//! # use chrono::{DateTime, Utc};
//! # enum QueueEvent {
//! #     CallEnqueued { queue_id: String, session_id: String, priority: u8 },
//! #     CallDequeued { queue_id: String, session_id: String, wait_time: u32 },
//! #     QueueOverflow { queue_id: String, overflow_to: String },
//! #     QueueThresholdReached { queue_id: String, current_size: usize, threshold: usize },
//! # }
//! ```
//!
//! ## Alerting System
//!
//! The monitoring system provides intelligent alerting for various conditions:
//!
//! ### **Service Level Alerts**
//! - Service level falls below target (e.g., < 80%)
//! - Average speed of answer exceeds threshold
//! - Abandon rate increases beyond acceptable limits
//! - Queue wait times exceed SLA requirements
//!
//! ### **Agent Performance Alerts**
//! - Agent utilization drops below minimum threshold
//! - Average handling time significantly increases
//! - Agent goes offline unexpectedly
//! - Performance metrics decline below standards
//!
//! ### **System Health Alerts**
//! - System capacity approaching limits
//! - Database connectivity issues
//! - High error rates in call processing
//! - Integration failures with external systems
//!
//! ## Dashboard Features
//!
//! The supervisor dashboard provides comprehensive visibility:
//!
//! ### **Real-Time Overview**
//! - Current call center status at a glance
//! - Active call count and queue lengths
//! - Agent availability and status
//! - Service level performance indicators
//!
//! ### **Agent Management**
//! - Individual agent performance tracking
//! - Real-time status monitoring
//! - Coaching alerts and intervention triggers
//! - Historical performance comparisons
//!
//! ### **Queue Management**
//! - Queue length and wait time monitoring
//! - Overflow management and routing
//! - Priority queue performance
//! - Historical queue performance trends
//!
//! ### **Historical Analytics**
//! - Performance trends over time
//! - Comparative analysis across periods
//! - Forecasting and capacity planning
//! - Custom report generation
//!
//! ## Integration Capabilities
//!
//! The monitoring system integrates with:
//!
//! - **Workforce Management (WFM) Systems**: For scheduling and forecasting
//! - **Customer Relationship Management (CRM)**: For customer context
//! - **Business Intelligence (BI) Tools**: For advanced analytics
//! - **Quality Management Systems**: For call recording and evaluation
//! - **External Dashboards**: For executive reporting
//!
//! ## Production Considerations
//!
//! ### **Performance**
//! - Event processing optimized for high throughput
//! - Real-time metrics with minimal latency
//! - Efficient data storage and retrieval
//! - Scalable architecture for growing call volumes
//!
//! ### **Reliability**
//! - Event durability and guaranteed delivery
//! - Monitoring system health checks
//! - Graceful degradation under load
//! - Backup and disaster recovery procedures
//!
//! ### **Security**
//! - Agent activity monitoring and audit trails
//! - Secure access to sensitive performance data
//! - Compliance with privacy regulations
//! - Role-based access control for dashboard features
//!
//! ## Modules
//!
//! - [`supervisor`]: Supervisor monitoring and intervention tools
//! - [`metrics`]: Performance metrics collection and analysis
//! - [`events`]: Event system for real-time monitoring

pub mod supervisor;
pub mod metrics;
pub mod events;

pub use supervisor::SupervisorMonitor;
pub use metrics::MetricsCollector;
pub use events::CallCenterEvents; 