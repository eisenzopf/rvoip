pub mod builder;
pub mod types;
pub mod wiring_manifest;
pub mod yaml_loader;

pub use builder::StateTableBuilder;
pub use types::*;
pub use yaml_loader::YamlTableLoader;

use lazy_static::lazy_static;
use std::sync::Arc;

lazy_static! {
    /// The master state table - single source of truth for all transitions
    pub static ref MASTER_TABLE: Arc<MasterStateTable> = Arc::new(build_master_table());
}

/// Build the complete master state table
fn build_master_table() -> MasterStateTable {
    // Load embedded default YAML (this should always succeed)
    let table = YamlTableLoader::load_embedded_default()
        .expect("Embedded default state table must be valid");

    // Validate the table
    if let Err(errors) = table.validate() {
        panic!("Invalid default state table: {:?}", errors);
    }

    tracing::debug!("Using embedded default state table");
    table
}

/// Load state table with two-tier priority:
/// 1. Config path (if Some)
/// 2. Embedded default
pub fn load_state_table_with_config(config_path: Option<&str>) -> MasterStateTable {
    // 1. Try config path first
    if let Some(path) = config_path {
        tracing::info!("Loading state table from config path: {}", path);
        if let Ok(table) = YamlTableLoader::load_from_file(path) {
            if let Err(errors) = table.validate() {
                tracing::error!("Config state table validation failed: {:?}", errors);
            } else {
                tracing::info!("Successfully loaded state table from config");
                return table;
            }
        } else {
            tracing::warn!("Failed to load state table from config path: {}", path);
        }
    }

    // 2. Load embedded default
    tracing::info!("Using embedded default state table");
    let table = YamlTableLoader::load_embedded_default()
        .expect("Embedded default state table must be valid");

    // Validate the table
    if let Err(errors) = table.validate() {
        panic!("Invalid default state table: {:?}", errors);
    }

    table
}
