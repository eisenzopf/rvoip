/*!
Logging and Metrics

This module provides standardized logging and metrics collection
for the RVOIP stack. It includes:

- Logging setup and configuration
- Contextual logging with additional metadata
- Metrics collection and reporting
*/

pub mod context;
pub mod metrics;
pub mod setup;

pub use context::{with_context, LogContext};
pub use metrics::{Metric, MetricType, MetricsCollector};
pub use setup::{setup_logging, LoggingConfig};
