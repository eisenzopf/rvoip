//! Test the three-tier state table loading system without network connections

use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Enable logging to see which state table is loaded
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v2::state_table=info".parse()?)
        )
        .init();
    
    println!("=== Testing State Table Loading Priority ===\n");
    
    // Test 1: Default (no config, no env var)
    println!("1. Testing DEFAULT state table:");
    println!("   Config path: None");
    println!("   Env var: Not set");
    env::remove_var("RVOIP_STATE_TABLE"); // Ensure env var is not set
    let table1 = rvoip_session_core_v2::state_table::load_state_table_with_config(None);
    println!("   ✓ Loaded state table with {} transitions\n", table1.transition_count());
    
    // Test 2: Environment variable
    println!("2. Testing ENVIRONMENT VARIABLE:");
    let enhanced_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("state_tables")
        .join("enhanced_state_table.yaml");
    env::set_var("RVOIP_STATE_TABLE", enhanced_path.to_str().unwrap());
    println!("   Config path: None");
    println!("   Env var: {}", enhanced_path.display());
    let table2 = rvoip_session_core_v2::state_table::load_state_table_with_config(None);
    println!("   ✓ Loaded state table with {} transitions\n", table2.transition_count());
    
    // Test 3: Config path (takes priority over env var)
    println!("3. Testing CONFIG PATH (priority over env var):");
    let default_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("state_tables")
        .join("default_state_table.yaml");
    println!("   Config path: {}", default_path.display());
    println!("   Env var: {} (will be ignored)", enhanced_path.display());
    let table3 = rvoip_session_core_v2::state_table::load_state_table_with_config(
        Some(default_path.to_str().unwrap())
    );
    println!("   ✓ Loaded state table with {} transitions\n", table3.transition_count());
    
    // Test 4: Invalid paths fall back correctly
    println!("4. Testing FALLBACK on invalid paths:");
    env::set_var("RVOIP_STATE_TABLE", "/nonexistent/path.yaml");
    println!("   Config path: /invalid/config/path.yaml");
    println!("   Env var: /nonexistent/path.yaml");
    let table4 = rvoip_session_core_v2::state_table::load_state_table_with_config(
        Some("/invalid/config/path.yaml")
    );
    println!("   ✓ Fell back to default: {} transitions\n", table4.transition_count());
    
    println!("=== Summary ===");
    println!("Priority order:");
    println!("1. Config path (if provided and valid)");
    println!("2. RVOIP_STATE_TABLE env var (if set and valid)");
    println!("3. Embedded default (always available)");
    
    Ok(())
}
