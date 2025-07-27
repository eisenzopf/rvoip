use g729a_new::encoder::pitch::pitch_fr3_fast;
use std::fs::File;
use std::io::{BufRead, BufReader};

const L_SUBFR: usize = 40;
const PIT_MAX: usize = 143;
const EXC_SIZE: usize = L_SUBFR + PIT_MAX;

#[test]
fn test_adaptive_codebook_search_from_csv() {
    let file = File::open("tests/acspc/test_inputs.csv")
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
        let expected_columns = 1 + EXC_SIZE + L_SUBFR + L_SUBFR + 3; // test_id + exc + xn + h + t0_min + t0_max + i_subfr
        if values.len() < expected_columns {
            continue;
        }
        
        // Parse excitation buffer (skip test index at position 0)
        let mut exc = [0i16; EXC_SIZE];
        for i in 0..EXC_SIZE {
            exc[i] = values[i + 1].trim().parse()
                .expect(&format!("Failed to parse exc[{}]", i));
        }
        
        // Parse target vector
        let mut xn = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            xn[i] = values[EXC_SIZE + 1 + i].trim().parse()
                .expect(&format!("Failed to parse xn[{}]", i));
        }
        
        // Parse impulse response
        let mut h = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            h[i] = values[EXC_SIZE + L_SUBFR + 1 + i].trim().parse()
                .expect(&format!("Failed to parse h[{}]", i));
        }
        
        // Parse search parameters
        let t0_min: i16 = values[EXC_SIZE + 2 * L_SUBFR + 1].trim().parse()
            .expect("Failed to parse t0_min");
        let t0_max: i16 = values[EXC_SIZE + 2 * L_SUBFR + 2].trim().parse()
            .expect("Failed to parse t0_max");
        let i_subfr: i16 = values[EXC_SIZE + 2 * L_SUBFR + 3].trim().parse()
            .expect("Failed to parse i_subfr");
        
        // Call the adaptive codebook search function
        let mut pit_frac: i16 = 0;
        let pitch_delay = pitch_fr3_fast(
            &mut exc,
            &xn,
            &h,
            L_SUBFR as i16,
            t0_min,
            t0_max,
            i_subfr,
            &mut pit_frac,
        );
        
        // Output the results in same format as C test (pitch delay and fractional part)
        println!("{},{}", pitch_delay, pit_frac);
    }
} 