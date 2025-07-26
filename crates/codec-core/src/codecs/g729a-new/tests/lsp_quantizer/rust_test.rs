
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
    let mut quantizer = LspQuantizer::new_c_compatible();  // Use C-compatible mode
    
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

#[test]
fn test_trace_values() {
    use g729a_new::common::basic_operators::*;
    use g729a_new::common::tab_ld8a::*;
    
    // Test inputs from test_inputs.csv test 1
    let a = [4096,-4174,1,17,12,13,13,13,11,9,103];
    let old_lsp = [0,0,0,0,0,0,0,0,0,0];
    let mut lsp = [0; 10];
    let mut lsp_q = [0; 10];
    let mut ana: [Word16; 2] = [0; 2];
    
    // Run az_lsp
    az_lsp(&a, &mut lsp, &old_lsp);
    
    // Print LSP values
    println!("LSP values after az_lsp:");
    for i in 0..10 {
        println!("lsp[{}] = {}", i, lsp[i]);
    }
    
    // Create quantizer and run quantization
    let mut quantizer = LspQuantizer::new();
    
    // Run quantization with debug output
    quantizer.qua_lsp(&lsp, &mut lsp_q, &mut ana);
    
    println!("\nFinal ana values: ana[0]={}, ana[1]={}", ana[0], ana[1]);
}

#[test]
fn test_arithmetic_operations() {
    use g729a_new::common::basic_operators::*;
    
    // Test the exact calculation that leads to different results
    let tmp2: Word16 = 5841;   // wegt[5]
    let tmp: Word16 = -215;    // difference value
    let l_dist: Word32 = 0;
    
    // Test l_mac behavior
    let result1 = l_mac(l_dist, tmp2, tmp);
    println!("Rust: l_mac(0, {}, {}) = {}", tmp2, tmp, result1);
    
    // Test l_mult directly
    let l_mult_result = l_mult(tmp2, tmp);
    println!("Rust: l_mult({}, {}) = {}", tmp2, tmp, l_mult_result);
    
    // Test with larger accumulator
    let l_dist2 = 300000;
    let result2 = l_mac(l_dist2, tmp2, tmp);
    println!("Rust: l_mac({}, {}, {}) = {}", l_dist2, tmp2, tmp, result2);
    
    // Test mult
    let a: Word16 = 32767;
    let b: Word16 = 2;
    let c = mult(a, b);
    println!("\nRust: mult({}, {}) = {}", a, b, c);
    
    // Test sub with potential overflow
    let x: Word16 = -32768;
    let y: Word16 = 1;
    let z = sub(x, y);
    println!("Rust: sub({}, {}) = {}", x, y, z);
    
    // Check if result1 should be negative
    if result1 as u32 == 4292455666u32 {
        println!("\nResult matches C's unsigned interpretation!");
        println!("As signed: {}", result1);
        println!("As unsigned: {}", result1 as u32);
    }
}
