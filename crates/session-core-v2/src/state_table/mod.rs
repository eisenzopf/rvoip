pub mod types;
pub mod builder;
pub mod tables;
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
    // Try to load from YAML first
    if let Ok(table) = YamlTableLoader::load_default() {
        // Validate the table
        if let Err(errors) = table.validate() {
            panic!("Invalid YAML state table: {:?}", errors);
        }
        return table;
    }
    
    // Fallback to hardcoded transitions if YAML fails
    tracing::warn!("Failed to load YAML state table, using hardcoded transitions");
    let mut builder = StateTableBuilder::new();
    
    // Add UAC transitions
    tables::uac::add_uac_transitions(&mut builder);
    
    // Add UAS transitions
    tables::uas::add_uas_transitions(&mut builder);
    
    // Add common transitions
    tables::common::add_common_transitions(&mut builder);
    
    // Add bridge and transfer transitions
    tables::bridge::add_bridge_transitions(&mut builder);
    
    let table = builder.build();
    
    // Validate the table
    if let Err(errors) = table.validate() {
        panic!("Invalid state table: {:?}", errors);
    }
    
    table
}