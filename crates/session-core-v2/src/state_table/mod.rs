pub mod types;
pub mod builder;
pub mod yaml_loader;

pub use types::*;
pub use builder::StateTableBuilder;
pub use yaml_loader::YamlTableLoader;

use lazy_static::lazy_static;
use std::sync::Arc;

lazy_static! {
    /// The master state table - single source of truth for all transitions
    pub static ref MASTER_TABLE: Arc<MasterStateTable> = Arc::new(build_master_table());
}

/// Build the complete master state table
fn build_master_table() -> MasterStateTable {
    // 1. Try custom YAML from environment variable
    if let Ok(custom_path) = std::env::var("RVOIP_STATE_TABLE") {
        tracing::info!("Loading custom state table from: {}", custom_path);
        if let Ok(table) = YamlTableLoader::load_from_file(&custom_path) {
            if let Err(errors) = table.validate() {
                tracing::error!("Custom state table validation failed: {:?}", errors);
            } else {
                tracing::info!("Successfully loaded custom state table");
                return table;
            }
        } else {
            tracing::warn!("Failed to load custom state table from {}, falling back to default", custom_path);
        }
    }
    
    // 2. Load embedded default YAML (this should always succeed)
    let table = YamlTableLoader::load_embedded_default()
        .expect("Embedded default state table must be valid");
    
    // Validate the table
    if let Err(errors) = table.validate() {
        panic!("Invalid default state table: {:?}", errors);
    }
    
    tracing::debug!("Using embedded default state table");
    table
}