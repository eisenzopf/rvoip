//! New high-performance event system using infra-common
//!
//! This module provides a federated event system that can operate in monolithic
//! or distributed modes while maintaining high performance and compatibility
//! with existing SessionEvent types.

pub mod infra_system;
pub mod compatibility;
pub mod federated_bus;
pub mod task_manager;
pub mod adapter;

pub use infra_system::InfraSessionEventSystem;
pub use compatibility::EventAdapter;
pub use federated_bus::RvoipFederatedBus;
pub use task_manager::{TrackedTaskManager, TaskHandle, TaskPriority, TaskStats};
pub use adapter::SessionEventAdapter;