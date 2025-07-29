use std::fs::File;
use std::io::{BufRead, BufReader};
use g729a_new::encoder::gain_quantizer::GainQuantizer;

const L_SUBFR: usize = 40;

#[test]
fn test_gain_quantization_from_csv() {
    let file = File::open("tests/gain_quantization/test_inputs.csv")
        .expect("Failed to open test_inputs.csv");
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    
    // Skip header line
    lines.next();
    
    // Create gain quantizer instance
    let mut gain_quantizer = GainQuantizer::new();
    
    // Process each test case
    for line in lines {
        let line = line.expect("Failed to read line");
        if line.trim().is_empty() {
            continue;
        }
        
        let values: Vec<&str> = line.split(',').collect();
        let expected_columns = 1 + L_SUBFR + 5 + 5 + 1; // test_id + code + g_coeff + exp_coeff + tameflag
        if values.len() < expected_columns {
            continue;
        }
        
        // Parse innovative vector code[] (skip test index at position 0)
        let mut code = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            code[i] = values[i + 1].trim().parse()
                .expect(&format!("Failed to parse code[{}]", i));
        }
        
        // Parse g_coeff
        let mut g_coeff = [0i16; 5];
        for i in 0..5 {
            g_coeff[i] = values[L_SUBFR + 1 + i].trim().parse()
                .expect(&format!("Failed to parse g_coeff[{}]", i));
        }
        
        // Parse exp_coeff
        let mut exp_coeff = [0i16; 5];
        for i in 0..5 {
            exp_coeff[i] = values[L_SUBFR + 5 + 1 + i].trim().parse()
                .expect(&format!("Failed to parse exp_coeff[{}]", i));
        }
        
        // Parse tameflag
        let tameflag: i16 = values[L_SUBFR + 5 + 5 + 1].trim().parse()
            .expect("Failed to parse tameflag");
        
        // Call the gain quantization function
        let (index, gain_pit, gain_cod) = gain_quantizer.quantize_gain(
            &code,
            &g_coeff,
            &exp_coeff,
            L_SUBFR as i16,
            tameflag,
        );
        
        // Output the results in same format as C test (index, gain_pit, gain_cod)
        println!("{},{},{}", index, gain_pit, gain_cod);
    }
} 