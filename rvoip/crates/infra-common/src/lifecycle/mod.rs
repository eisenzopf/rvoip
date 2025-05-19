/*!
Lifecycle Management

This module provides a standardized component lifecycle management system
for the RVOIP stack. It includes:

- Component trait for standard lifecycle methods
- Lifecycle manager for orchestrating component lifecycles
- Dependency resolution for startup/shutdown ordering
*/

pub mod component;
pub mod manager;
pub mod dependency;
pub mod health;

pub use component::{Component, ComponentState};
pub use manager::{LifecycleManager, LifecycleError};
pub use dependency::{DependencyGraph, DependencyError};
pub use health::{HealthCheck, HealthStatus}; 