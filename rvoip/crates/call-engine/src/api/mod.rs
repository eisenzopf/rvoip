//! Public API module
//!
//! This module provides public APIs for call center applications,
//! including client, supervisor, and administrative interfaces.

pub mod client;
pub mod supervisor;
pub mod admin;

pub use client::CallCenterClient;
pub use supervisor::SupervisorApi;
pub use admin::AdminApi; 