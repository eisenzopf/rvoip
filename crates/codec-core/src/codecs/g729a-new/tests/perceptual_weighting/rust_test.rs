use g729a_new::encoder::perceptual_weighting::perceptual_weighting;
use g729a_new::common::basic_operators::Word16;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[test]
fn test_perceptual_weighting_from_csv() {
    let file = File::open("tests/perceptual_weighting/test_inputs.csv").expect("Failed to open test_inputs.csv");
    let reader = BufReader::new(file);
    let lines = reader.lines();
    
    println!("test_id,p0,p1,p2,p3,p4,p5,p6,p7,p8,p9,p10,f0,f1,f2,f3,f4,f5,f6,f7,f8,f9,f10");
    
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
        
        if values.len() < 12 {  // test_id + 11 coefficients
            continue; // Skip invalid lines
        }
        
        let test_id: i32 = values[0].trim().parse()
            .expect("Failed to parse test_id");
        
        // Parse a coefficients
        let mut a = [0; 11];
        for i in 0..11 {
            a[i] = values[i + 1].trim().parse()
                .expect(&format!("Failed to parse a[{}]", i));
        }
        
        // Run the perceptual weighting calculation
        let mut ap1 = [0; 11];  // For gamma1
        let mut ap2 = [0; 11];  // For gamma2
        
        perceptual_weighting(&a, &mut ap1, &mut ap2);
        
        // Print results in CSV format
        print!("{}", test_id);
        for i in 0..11 {
            print!(",{}", ap1[i]);
        }
        for i in 0..11 {
            print!(",{}", ap2[i]);
        }
        println!();
    }
}
