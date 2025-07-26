use g729a_new::common::basic_operators::*;
use g729a_new::encoder::perceptual_weighting::perceptual_weighting;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

const M: usize = 10;

fn weight_az_with_debug(a: &[Word16], gamma: Word16, ap: &mut [Word16], test_id: i32) {
    ap[0] = a[0];
    let mut fac = gamma;
    if test_id == 0 { println!("Rust fac init: {}", fac); }

    for i in 1..M {
        let l_temp = l_mult(a[i], fac);
        ap[i] = round(l_temp);
        if test_id == 0 { println!("Rust i={}, a[i]={}, fac={}, L_mult={}, ap[i]={}", i, a[i], fac, l_temp, ap[i]); }
        
        let l_temp_fac = l_mult(fac, gamma);
        fac = round(l_temp_fac);
        if test_id == 0 { println!("Rust i={}, new_fac={}", i, fac); }
    }
    let l_temp = l_mult(a[M], fac);
    ap[M] = round(l_temp);
    if test_id == 0 { println!("Rust i={}, a[m]={}, fac={}, L_mult={}, ap[m]={}", M, a[M], fac, l_temp, ap[M]); }
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

        if test_id == 0 {
            const GAMMA1: Word16 = 30802; // 0.94 in Q15
            const GAMMA2: Word16 = 19661; // 0.6 in Q15
            println!("--- Rust DEBUG gamma1 ---");
            weight_az_with_debug(&a, GAMMA1, &mut p, test_id);
            println!("--- Rust DEBUG gamma2 ---");
            weight_az_with_debug(&a, GAMMA2, &mut f, test_id);
        } else {
            perceptual_weighting(&a, &mut p, &mut f);
        }

        print!("{}", test_id);
        for val in p { print!(",{}", val); }
        for val in f { print!(",{}", val); }
        println!();
    }
}
