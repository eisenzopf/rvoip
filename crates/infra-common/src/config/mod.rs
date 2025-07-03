/*!
Configuration System

This module provides a standardized configuration system for the RVOIP stack.
It includes:

- Configuration loading from files and environment variables
- Configuration providers for components
- Schema validation for configurations
- Support for dynamic configuration updates
*/

pub mod loader;
pub mod provider;
pub mod schema;
pub mod dynamic;

pub use loader::ConfigLoader;
pub use provider::{ConfigProvider, ConfigSource};
pub use schema::SchemaValidator;
pub use dynamic::DynamicConfig; 