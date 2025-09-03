/// Standalone test for the state table core functionality
/// This tests the state table without requiring the full API layer

#[test]
fn test_state_table_loads() {
    use rvoip_session_core_v2::state_table::MASTER_TABLE;
    
    // This will panic if the state table is invalid
    let _table = &*MASTER_TABLE;
    
    // If we get here, the table loaded successfully
    assert!(true, "State table loaded without panic");
}

#[test]
fn test_state_table_validation() {
    use rvoip_session_core_v2::state_table::MASTER_TABLE;
    
    let table = &*MASTER_TABLE;
    
    // The validate() method checks for consistency
    match table.validate() {
        Ok(_) => assert!(true, "State table validation passed"),
        Err(errors) => {
            for error in errors {
                eprintln!("Validation error: {}", error);
            }
            panic!("State table validation failed");
        }
    }
}