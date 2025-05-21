// Shared components
pub mod bus;
pub mod registry;
pub mod subscriber;
pub mod types;
pub mod publisher;

// Core interfaces for the new API
pub mod api;
pub mod static_path;
pub mod zero_copy;
pub mod system;
pub mod builder;

// Tests
#[cfg(test)]
mod tests;

