/*!
Configuration System

This module provides a standardized configuration system for the RVOIP stack.
It includes:

- Configuration loading from files and environment variables
- Configuration providers for components
- Schema validation for configurations
- Support for dynamic configuration updates
*/

pub mod dynamic;
pub mod loader;
pub mod provider;
pub mod schema;

pub use dynamic::DynamicConfig;
pub use loader::ConfigLoader;
pub use provider::{ConfigProvider, ConfigSource};
pub use schema::SchemaValidator;
