/*!
Lifecycle Management

This module provides a standardized component lifecycle management system
for the RVOIP stack. It includes:

- Component trait for standard lifecycle methods
- Lifecycle manager for orchestrating component lifecycles
- Dependency resolution for startup/shutdown ordering
*/

pub mod component;
pub mod dependency;
pub mod health;
pub mod manager;

pub use component::{Component, ComponentState};
pub use dependency::{DependencyError, DependencyGraph};
pub use health::{HealthCheck, HealthStatus};
pub use manager::{LifecycleError, LifecycleManager};
