use g729a_new::encoder::perceptual_weighting::perceptual_weighting;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

fn print_coeffs_csv(test_id: i32, p: &[i16], f: &[i16]) {
    print!("{}", test_id);
    for val in p { print!(",{}", val); }
    for val in f { print!(",{}", val); }
    println!();
}

#[test]
fn test_perceptual_weighting_from_csv() {
    let path = Path::new("tests/perceptual_weighting/test_inputs.csv");
    let file = File::open(&path).expect("Failed to open test_inputs.csv");
    let reader = BufReader::new(file);

    println!("test_id,p0,p1,p2,p3,p4,p5,p6,p7,p8,p9,p10,f0,f1,f2,f3,f4,f5,f6,f7,f8,f9,f10");

    for (index, line) in reader.lines().enumerate() {
        if index == 0 { continue; } // Skip header

        let line = line.expect("Failed to read line");
        if line.trim().is_empty() { continue; }

        let values: Vec<i16> = line.split(',')
            .map(|s| s.trim().parse().expect("Failed to parse value"))
            .collect();

        let test_id = values[0] as i32;
        let a: [i16; 11] = values[1..12].try_into().expect("Slice with incorrect length");

        let mut p = [0; 11];
        let mut f = [0; 11];

        perceptual_weighting(&a, &mut p, &mut f);

        print_coeffs_csv(test_id, &p, &f);
    }
}
