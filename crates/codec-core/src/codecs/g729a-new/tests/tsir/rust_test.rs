use std::fs::File;
use std::io::{BufRead, BufReader, Write};

type Word16 = i16;

// We'll need to copy the target_signal function here since we can't easily import it as a binary
fn syn_filt(
    a: &[Word16],
    x: &[Word16],
    y: &mut [Word16],
    lg: usize,
    mem: &mut [Word16],
    update: bool,
) {
    // Simplified synthesis filter implementation
    for i in 0..lg {
        let mut s = x[i] as i32;
        for j in 1..a.len().min(i + 1) {
            s -= (a[j] as i32 * y[i - j] as i32) >> 12;
        }
        for j in 1..a.len().min(mem.len() + 1) {
            if i < j {
                s -= (a[j] as i32 * mem[mem.len() - j + i] as i32) >> 12;
            }
        }
        y[i] = s as Word16;
    }
    
    if update && mem.len() > 0 {
        let copy_len = mem.len().min(lg);
        for i in 0..copy_len {
            mem[mem.len() - copy_len + i] = y[lg - copy_len + i];
        }
    }
}

fn target_signal(
    p: &[Word16],
    f: &[Word16],
    _exc: &[Word16],
    r: &[Word16],
    x: &mut [Word16],
    mem: &mut [Word16],
) {
    let mut temp = [0; 40];
    syn_filt(p, r, &mut temp, 40, mem, false);
    syn_filt(f, &temp, x, 40, mem, false);
}

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

        // Call the target_signal function
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