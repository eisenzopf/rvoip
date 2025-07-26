
use g729a_new::encoder::lsp_quantizer::az_lsp;
use g729a_new::encoder::lspvq::LspQuantizer;
use g729a_new::common::basic_operators::Word16;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[test]
fn test_lsp_quantizer() {
    let file = File::open("tests/lsp_quantizer/test_inputs.csv").expect("Failed to open test_inputs.csv");
    let reader = BufReader::new(file);
    let lines = reader.lines();
    
    println!("test_id,lsp_q0,lsp_q1,lsp_q2,lsp_q3,lsp_q4,lsp_q5,lsp_q6,lsp_q7,lsp_q8,lsp_q9,ana0,ana1");
    
    // Create quantizer once - maintain state across all test cases
    let mut quantizer = LspQuantizer::new();
    
    // Process each test case
    for line in lines {
        let line = line.expect("Failed to read line");
        
        // Skip comment lines and empty lines
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        
        // Skip header line
        if line.starts_with("test_id") {
            continue;
        }
        
        let values: Vec<&str> = line.split(',').collect();
        
        if values.len() < 22 {
            continue; // Skip invalid lines
        }
        
        let test_id: i32 = values[0].trim().parse()
            .expect("Failed to parse test_id");
        
        // Parse old_lsp values
        let mut old_lsp = [0; 10];
        for i in 0..10 {
            old_lsp[i] = values[i + 1].trim().parse()
                .expect(&format!("Failed to parse old_lsp[{}]", i));
        }
        
        // Parse a coefficients
        let mut a = [0; 11];
        for i in 0..11 {
            a[i] = values[i + 11].trim().parse()
                .expect(&format!("Failed to parse a[{}]", i));
        }
        
        // Run the test
        let mut lsp = [0; 10];
        let mut lsp_q = [0; 10];
        let mut ana: [Word16; 2] = [0; 2];
        
        az_lsp(&a, &mut lsp, &old_lsp);
        quantizer.qua_lsp(&lsp, &mut lsp_q, &mut ana);
        
        // Print results in CSV format
        print!("{}", test_id);
        for i in 0..10 {
            print!(",{}", lsp_q[i]);
        }
        println!(",{},{}", ana[0], ana[1]);
    }
}
