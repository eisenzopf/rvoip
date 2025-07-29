// Test for ACELP fixed codebook search
// This test will compare the Rust implementation against the C reference

use std::fs::File;
use std::io::{BufRead, BufReader};

// Import the actual ACELP implementation
use g729a_new::encoder::acelp_codebook::acelp_code_a;

const L_SUBFR: usize = 40;

#[test]
fn test_acelp_codebook_search_from_csv() {
    let file = File::open("tests/acelp/test_inputs.csv")
        .expect("Failed to open test_inputs.csv");
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    
    // Skip header line
    lines.next();
    
    // Process each test case
    for line in lines {
        let line = line.expect("Failed to read line");
        if line.trim().is_empty() {
            continue;
        }
        
        let values: Vec<&str> = line.split(',').collect();
        let expected_columns = 1 + L_SUBFR + L_SUBFR + 2; // test_id + x + h + T0 + pitch_sharp
        if values.len() < expected_columns {
            continue;
        }
        
        // Parse target vector x[]
        let mut x = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            x[i] = values[i + 1].trim().parse()
                .expect(&format!("Failed to parse x[{}]", i));
        }
        
        // Parse impulse response h[]
        let mut h = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            h[i] = values[L_SUBFR + 1 + i].trim().parse()
                .expect(&format!("Failed to parse h[{}]", i));
        }
        
        // Parse T0 and pitch_sharp
        let t0: i16 = values[2 * L_SUBFR + 1].trim().parse()
            .expect("Failed to parse T0");
        let pitch_sharp: i16 = values[2 * L_SUBFR + 2].trim().parse()
            .expect("Failed to parse pitch_sharp");
        
        // Prepare output arrays
        let mut code = [0i16; L_SUBFR];
        let mut y = [0i16; L_SUBFR];
        let mut sign: i16 = 0;
        
        // Call the ACELP function
        let index = acelp_code_a(
            &x,
            &h,
            t0,
            pitch_sharp,
            &mut code,
            &mut y,
            &mut sign,
        );
        
        // Output results in same format as C test
        print!("{},{}", index, sign);
        
        // Output first 10 code values
        for i in 0..10 {
            print!(",{}", code[i]);
        }
        
        // Output first 10 y values
        for i in 0..10 {
            print!(",{}", y[i]);
        }
        
        println!();
    }
}

// Helper function to run the test directly
fn main() {
    test_acelp_codebook_search_from_csv();
} 