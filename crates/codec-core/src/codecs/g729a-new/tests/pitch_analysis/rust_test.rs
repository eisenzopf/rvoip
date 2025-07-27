use g729a_new::encoder::pitch_ol_fast_g729a::pitch_ol_fast_g729a;
use std::fs::File;
use std::io::{BufRead, BufReader};

const L_FRAME: usize = 80;
const PIT_MAX: usize = 143;
const L_TOTAL: usize = L_FRAME + PIT_MAX;

#[test]
fn test_pitch_analysis_from_csv() {
    let file = File::open("tests/pitch_analysis/test_inputs.csv")
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
        if values.len() < L_TOTAL + 1 {
            continue;
        }
        
        // Parse signal values (skip test index at position 0)
        let mut signal = [0i16; L_TOTAL];
        for i in 0..L_TOTAL {
            signal[i] = values[i + 1].trim().parse()
                .expect(&format!("Failed to parse signal[{}]", i));
        }
        
        // Call the new G.729A compliant open-loop pitch analysis function
        let pitch_lag = pitch_ol_fast_g729a(&signal, PIT_MAX as i32, L_FRAME as i32);
        
        // Output the result
        println!("{}", pitch_lag);
    }
} 