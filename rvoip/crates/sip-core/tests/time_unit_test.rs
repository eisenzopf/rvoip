use rvoip_sip_core::sdp::parser::time_parser::parse_time_with_unit;

#[test]
fn test_time_unit_conversion() {
    // Test plain seconds
    let result = parse_time_with_unit("30").unwrap();
    println!("30 -> {} seconds", result);
    
    // Test minutes
    let result = parse_time_with_unit("5m").unwrap();
    println!("5m -> {} seconds", result);
    
    // Test hours
    let result = parse_time_with_unit("2h").unwrap();
    println!("2h -> {} seconds", result);
    
    // Test days
    let result = parse_time_with_unit("1d").unwrap();
    println!("1d -> {} seconds", result);
} 