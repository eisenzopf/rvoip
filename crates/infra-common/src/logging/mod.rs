/*!
Logging and Metrics

This module provides standardized logging and metrics collection
for the RVOIP stack. It includes:

- Logging setup and configuration
- Contextual logging with additional metadata
- Metrics collection and reporting
*/

pub mod setup;
pub mod context;
pub mod metrics;

pub use setup::{setup_logging, LoggingConfig};
pub use context::{LogContext, with_context};
pub use metrics::{Metric, MetricType, MetricsCollector}; 