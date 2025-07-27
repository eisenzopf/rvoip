use std::fs::File;
use std::io::{BufRead, BufReader, Write};

// Import from the library instead of implementing custom versions
use g729a_new::encoder::target::target_signal;

fn main() {
    // Read test vectors
    let input_file = File::open("test_inputs.csv").expect("Failed to open test_inputs.csv");
    let reader = BufReader::new(input_file);
    let mut lines = reader.lines();

    // Skip header
    lines.next();

    // Create output file
    let mut output_file = File::create("rust_output.csv").expect("Failed to create rust_output.csv");
    writeln!(output_file, "test_id,x[40]").expect("Failed to write header");

    // Process each test vector
    for line in lines {
        let line = line.expect("Failed to read line");
        let values: Vec<i16> = line
            .split(',')
            .map(|s| s.parse().unwrap_or(0))
            .collect();

        let test_id = values[0];
        
        // Extract arrays from the CSV line:
        // test_id, p[11], f[11], r[40], mem[10]
        let p = &values[1..12];      // LP coefficients: 11 values
        let f = &values[12..23];     // Weighted filter coefficients: 11 values  
        let r = &values[23..63];     // Residual: 40 values
        let mem_input = &values[63..73]; // Memory: 10 values

        // Create arrays for the target_signal function
        let mut x = vec![0i16; 40];
        let mut mem = vec![0i16; 10];
        
        // Copy memory values
        for i in 0..10.min(mem_input.len()) {
            mem[i] = mem_input[i];
        }

        // Call the library target_signal function
        target_signal(
            p,
            f,
            &[],  // exc not needed for basic target signal calculation
            r,
            &mut x,
            &mut mem,
        );

        // Write results
        write!(output_file, "{}", test_id).expect("Failed to write test_id");
        for val in x {
            write!(output_file, ",{}", val).expect("Failed to write value");
        }
        writeln!(output_file).expect("Failed to write newline");
    }
} 